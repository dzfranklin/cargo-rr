#![warn(clippy::all, clippy::pedantic, clippy::cargo)]

use std::{borrow::Cow, sync::Arc};

use anyhow::anyhow;
use clap::{AppSettings, Parser, Subcommand};
use seacan::{bin, test, CompilerMessage, ExecutableArtifact, FeatureSpec, PackageSpec};
#[allow(unused)]
use tracing::{debug, error, info, warn};

use cargo_rr::{list, record, replay, Trace};

#[derive(Parser, Debug)]
#[clap(bin_name = "cargo", about, author)]
enum OptWrapper {
    #[clap(subcommand, name = "rr")]
    Opt(Opt),
}

#[derive(Subcommand, Debug)]
#[clap(about, author)]
enum Opt {
    #[clap(about = "Record a binary or example")]
    Run(RunOpt),
    #[clap(about = "Record a test")]
    Test(TestOpt),
    #[clap(about = "Replay a trace")]
    Replay(ReplayOpt),
    #[clap(about = "List traces")]
    Ls,
}

#[derive(Parser, Debug)]
#[clap(setting(AppSettings::TrailingVarArg))]
#[clap(setting(AppSettings::AllowHyphenValues))]
struct RunOpt {
    #[clap(long)]
    bin: Option<String>,
    #[clap(long)]
    example: Option<String>,
    #[clap(long)]
    all_features: bool,
    #[clap(long)]
    no_default_features: bool,
    #[clap(long)]
    features: Vec<String>,
    #[clap(long)]
    release: bool,
    #[clap(long)]
    package: Option<String>,
    #[clap(
        help = r#"Space-separated options to pass to `rr record` (e.g `"--chaos -M"`). See `rr record -h`"#
    )]
    rr_opts: Option<String>,
    #[clap(last = true)]
    args: Vec<String>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Parser, Debug)]
#[clap(setting(AppSettings::AllowHyphenValues))]
struct TestOpt {
    name: Option<String>,
    #[clap(long, help = "Match name exactly")]
    exact: bool,
    #[clap(long)]
    lib: bool,
    #[clap(long)]
    bin: Option<String>,
    #[clap(long)]
    bins: bool,
    #[clap(
        long,
        help = "Test only the specified integration test (i.e. file in tests/)"
    )]
    test: Option<String>,
    #[clap(long)]
    tests: bool,
    #[clap(long)]
    example: Option<String>,
    #[clap(long)]
    examples: bool,
    #[clap(long)]
    doc: bool,
    #[clap(long)]
    all_features: bool,
    #[clap(long)]
    no_default_features: bool,
    #[clap(long)]
    features: Vec<String>,
    #[clap(long)]
    release: bool,
    #[clap(long)]
    package: Option<String>,
    #[clap(
        help = r#"Space-separated options to pass to `rr record` (e.g `"--chaos -M"`). See `rr record -h`"#
    )]
    rr_opts: Option<String>,
}

#[derive(Parser, Debug)]
#[clap(setting(AppSettings::AllowHyphenValues))]
struct ReplayOpt {
    #[clap(help = "Leave blank to replay the last trace recorded")]
    trace: Option<String>,
    #[clap(
        long,
        require_equals(true),
        help = "Space-separated options to pass to `rr replay`. See `rr replay -h`"
    )]
    rr_opts: Option<String>,
    #[clap(long, require_equals(true), help = "Options to pass to rust-gdb")]
    gdb_opts: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    if let Err(err) = handle_opts() {
        println!(); // to separate is_test_artifactour output from anything cargo outputs
        return Err(err);
    }
    Ok(())
}

fn handle_opts() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let OptWrapper::Opt(opt) = OptWrapper::from_args();

    debug!(?opt, "Parsed options");

    match opt {
        Opt::Run(opt) => {
            handle_run(opt)?;
        }
        Opt::Test(opt) => {
            handle_test(opt)?;
        }
        Opt::Replay(opt) => {
            handle_replay(opt)?;
        }
        Opt::Ls => list()?,
    }

    Ok(())
}

fn handle_run(opt: RunOpt) -> anyhow::Result<()> {
    let package = opt.package.map_or(PackageSpec::Any, PackageSpec::Name);

    let features = parse_features(opt.all_features, opt.no_default_features, opt.features)?;

    let mut compiler = match (opt.bin, opt.example) {
        (Some(bin), None) => bin::Compiler::bin(bin),
        (None, Some(example)) => bin::Compiler::example(example),
        (None, None) => todo!("Run default bin"),
        (Some(_), Some(_)) => return Err(anyhow!("You cannot specify both --bin and --example")),
    };

    eprintln!("Compiling...");
    let artifact = compiler
        .package(package)
        .release(opt.release)
        .features(features)
        .on_compiler_msg(on_compiler_msg)
        .compile()?;

    let trace = record(artifact.executable, opt.rr_opts.as_deref(), &opt.args)?;
    print_replay_howto(&trace);
    Ok(())
}

fn handle_test(opt: TestOpt) -> anyhow::Result<()> {
    let (rr_opts, mut compiler) = configure_test_compiler(opt)?;
    eprintln!("Compiling...");
    let artifacts = compiler.on_compiler_msg(on_compiler_msg).compile()?;

    let mut specs = Vec::new();
    for artifact in artifacts {
        let tests = artifact.tests;
        let artifact = Arc::new(artifact.artifact);
        for test in tests {
            let spec = TestSpec {
                test,
                artifact: artifact.clone(),
            };
            specs.push(spec);
        }
    }

    let selected = select_test_spec(specs)?;

    let trace = record(
        selected.artifact.executable.clone(),
        rr_opts.as_deref(),
        &selected.test.run_args(),
    )?;
    print_replay_howto(&trace);
    Ok(())
}

#[derive(Clone, Debug)]
struct TestSpec {
    test: test::TestFn,
    artifact: Arc<ExecutableArtifact>,
}

impl skim::SkimItem for TestSpec {
    fn text(&self) -> Cow<str> {
        Cow::Owned(format!(
            "{}::{} ({})",
            self.artifact.target.name, self.test.name, self.test.test_type
        ))
    }
}

fn select_test_spec(mut specs: Vec<TestSpec>) -> anyhow::Result<TestSpec> {
    use skim::prelude::*;

    if specs.is_empty() {
        return Err(anyhow!("No matching test or benchmark functions"));
    }
    if specs.len() == 1 {
        return Ok(specs.pop().unwrap());
    }

    let skim_opts = SkimOptionsBuilder::default()
        .height(Some("50%"))
        .build()
        .map_err(|e| anyhow!("skim: {}", e))?;

    let (tx, rx): (SkimItemSender, SkimItemReceiver) = unbounded();
    for spec in specs {
        tx.send(Arc::new(spec))?;
    }
    drop(tx);

    let mut selected = Skim::run_with(&skim_opts, Some(rx))
        .map(|o| {
            if o.is_abort {
                Vec::new()
            } else {
                o.selected_items
            }
        })
        .unwrap_or_default();
    if selected.is_empty() {
        return Err(anyhow!("No test selected"));
    }
    if selected.len() > 1 {
        panic!("Should have been impossible to select more than one test");
    }
    let selected = &*selected.pop().unwrap();
    if let Some(selected) = selected.as_any().downcast_ref::<TestSpec>() {
        Ok(selected.clone())
    } else {
        Err(anyhow!("No test selected"))
    }
}

fn configure_test_compiler(opt: TestOpt) -> anyhow::Result<(Option<String>, test::Compiler)> {
    use test::{NameSpec, TypeSpec};

    let name = match (opt.name, opt.exact) {
        (Some(name), true) => NameSpec::Exact(name),
        (Some(name), false) => NameSpec::Substring(name),
        (None, true) => return Err(anyhow!("Cannot specify --exact without specifying a name")),
        (None, false) => NameSpec::Any,
    };

    let mut test_type = None;
    let require_tt_unset = |tt: &Option<TypeSpec>| {
        if tt.is_some() {
            Err(anyhow!(
                "Only one type of test can be specified (--lib, --bins, etc)"
            ))
        } else {
            Ok(())
        }
    };
    if opt.lib {
        require_tt_unset(&test_type)?;
        test_type = Some(TypeSpec::Lib);
    }
    if let Some(name) = opt.bin {
        require_tt_unset(&test_type)?;
        test_type = Some(TypeSpec::Bin(name));
    }
    if opt.bins {
        require_tt_unset(&test_type)?;
        test_type = Some(TypeSpec::Bins);
    }
    if let Some(name) = opt.test {
        require_tt_unset(&test_type)?;
        test_type = Some(TypeSpec::Integration(name));
    }
    if opt.tests {
        require_tt_unset(&test_type)?;
        test_type = Some(TypeSpec::Integrations);
    }
    if let Some(name) = opt.example {
        require_tt_unset(&test_type)?;
        test_type = Some(TypeSpec::Example(name));
    }
    if opt.examples {
        require_tt_unset(&test_type)?;
        test_type = Some(TypeSpec::Examples);
    }
    if opt.doc {
        require_tt_unset(&test_type)?;
        test_type = Some(TypeSpec::Doc);
    }
    let test_type = test_type.unwrap_or(TypeSpec::Unspecified);

    let package = opt.package.map_or(PackageSpec::Any, PackageSpec::Name);
    let features = parse_features(opt.all_features, opt.no_default_features, opt.features)?;

    let mut compiler = test::Compiler::new(name, test_type);
    compiler
        .package(package)
        .release(opt.release)
        .features(features);
    Ok((opt.rr_opts, compiler))
}

fn parse_features(
    all: bool,
    no_default: bool,
    features: Vec<String>,
) -> anyhow::Result<FeatureSpec> {
    match (all, no_default) {
        (true, false) => {
            if features.is_empty() {
                Ok(FeatureSpec::all())
            } else {
                Err(anyhow!(
                    "You cannot specify both --all-features and --features"
                ))
            }
        }
        (true, true) => Err(anyhow!(
            "You cannot specify both --all-features and --no-default-features"
        )),
        (false, true) => Ok(FeatureSpec::new_no_default(features)),
        (false, false) => Ok(FeatureSpec::new(features)),
    }
}

fn on_compiler_msg(msg: CompilerMessage) {
    if let Some(rendered) = msg.message.rendered {
        eprintln!("{}", rendered);
    }
}

fn handle_replay(opt: ReplayOpt) -> anyhow::Result<()> {
    let trace = opt.trace.map_or_else(Trace::latest, |s| Trace::new(&s))?;
    replay(trace, opt.rr_opts.as_deref(), opt.gdb_opts)?;
    Ok(())
}

fn print_replay_howto(trace: &Trace) {
    eprintln!(
        "\nTrace {} recorded.\nRun `cargo rr replay` to debug the latest trace",
        trace.name()
    );
}
