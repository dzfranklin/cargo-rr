#![warn(clippy::all, clippy::pedantic, clippy::cargo)]

use std::{
    io::BufReader,
    process::{Command, Stdio},
};

use anyhow::{anyhow, Context};
use camino::Utf8PathBuf;
use cargo_metadata::{camino::Utf8Path, Artifact, Metadata};
use clap::AppSettings;
use dialoguer::Select;
use structopt::StructOpt;
#[allow(unused)]
use tracing::{debug, error, info, warn};

#[derive(StructOpt, Debug)]
#[structopt(bin_name = "cargo", about, author)]
enum OptWrapper {
    #[structopt(name = "rr")]
    Opt(Opt),
}

#[derive(StructOpt, Debug)]
#[structopt(about, author)]
enum Opt {
    #[structopt(
        about = "Record a binary or example",
        setting(AppSettings::TrailingVarArg),
        setting(AppSettings::AllowLeadingHyphen)
    )]
    Run {
        #[structopt(name = "OPTIONS", help = "Options to pass to `cargo run`")]
        opts: Vec<String>,
        #[structopt(
            last = true,
            help = "Options to pass to `rr record`. See `rr record -h`"
        )]
        rr_opts: Vec<String>,
    },

    #[structopt(
        about = "Record a test",
        setting(AppSettings::TrailingVarArg),
        setting(AppSettings::AllowLeadingHyphen)
    )]
    Test {
        #[structopt(name = "OPTIONS", help = "Options to pass to `cargo test`")]
        opts: Vec<String>,
        #[structopt(
            last = true,
            help = "Options to pass to `rr record`. See `rr record -h`"
        )]
        rr_opts: Vec<String>,
    },

    #[structopt(
        about = "Replay a trace",
        setting(AppSettings::TrailingVarArg),
        setting(AppSettings::AllowLeadingHyphen)
    )]
    Replay {
        #[structopt(help = "Leave blank to replay the last trace recorded")]
        trace: Option<String>,
        #[structopt(
            last = true,
            help = "Options to pass to `rr replay`. See `rr replay -h`"
        )]
        rr_opts: Vec<String>,
    },

    #[structopt(about = "List traces", setting(AppSettings::AllowLeadingHyphen))]
    Ls {
        #[structopt(last = true, help = "Options to pass to `rr ls`. See `rr ls -h`")]
        rr_opts: Vec<String>,
    },
}

fn main() -> anyhow::Result<()> {
    if let Err(err) = run() {
        println!(); // to separate is_test_artifactour output from anything cargo outputs
        return Err(err);
    }
    Ok(())
}

fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let OptWrapper::Opt(opt) = OptWrapper::from_args();

    debug!(?opt, "Parsed options");

    match opt {
        Opt::Run { opts, rr_opts } => {
            let bin = build_and_select(false, &opts)?;
            record(&bin, rr_opts, Vec::new())?;
        }
        Opt::Test { mut opts, rr_opts } => {
            let bin_args = if let Some(last) = opts.last() {
                if last.starts_with('-') {
                    Vec::new()
                } else {
                    let test_name = opts.pop().unwrap();
                    vec![test_name]
                }
            } else {
                Vec::new()
            };

            let bin = build_and_select(true, &opts)?;

            record(&bin, rr_opts, bin_args)?;
        }
        Opt::Replay {
            mut rr_opts,
            mut trace,
        } => {
            // fix ambiguous parse
            if let Some(trace_val) = trace.as_ref() {
                if trace_val.starts_with('-') {
                    rr_opts.insert(0, trace.take().unwrap());
                    trace = None;
                    debug!(?rr_opts, ?trace, "Moved mis-parsed opt into opts, now")
                }
            }

            replay(trace, rr_opts)?
        }
        Opt::Ls { rr_opts } => ls(&rr_opts)?,
    }

    Ok(())
}

fn meta() -> anyhow::Result<Metadata> {
    let meta = cargo_metadata::MetadataCommand::new().exec()?;
    Ok(meta)
}

fn build_and_select(is_test: bool, opts: &[String]) -> anyhow::Result<Utf8PathBuf> {
    use cargo_metadata::Message;

    debug!(?is_test, ?opts, "Building");

    let meta = meta()?;
    let workspace_members = &meta.workspace_members;

    let mut cmd = Command::new("cargo");

    if is_test {
        cmd.arg("test").arg("--no-run");
    } else {
        cmd.arg("build");
    }

    let mut cmd = cmd
        .arg("--message-format=json-render-diagnostics")
        .args(opts)
        .stdout(Stdio::piped())
        .spawn()?;

    let reader = BufReader::new(cmd.stdout.take().expect("Piped stdout"));

    let mut artifacts = Vec::new();
    for msg in Message::parse_stream(reader) {
        debug!(?msg);
        if let Message::CompilerArtifact(artifact) = msg? {
            if artifact.executable.is_some() {
                debug!(?artifact, "Artifact passed filters");
                artifacts.push(artifact);
            }
        }
    }

    artifacts.sort_by(|a, b| a.target.src_path.cmp(&b.target.src_path));
    artifacts.dedup_by(|a, b| a.target.src_path == b.target.src_path);

    let artifact = match artifacts.len() {
        0 => return Err(anyhow!("cargo-rr: No test artifacts built.")),
        1 => &artifacts[0],
        _ => select_artifact(&meta, &artifacts)?,
    };

    let bin = artifact
        .executable
        .as_ref()
        .expect("We know artifact has executable");

    debug!("Selected bin {:?}", bin);

    Ok(bin.to_path_buf())
}

fn record(bin: &Utf8Path, mut args: Vec<String>, bin_args: Vec<String>) -> anyhow::Result<()> {
    debug!(?bin, ?args, ?bin_args, "Recording");

    println!();

    insert_trailing_args(&mut args, bin_args);

    let mut cmd = Command::new("rr")
        .arg("record")
        .arg(bin)
        .args(args)
        .spawn()
        .context("Failed to run rr")?;

    let status = cmd.wait()?;
    if !status.success() {
        // Not an error as this might just mean the recorded program failed
        println!("cargo-rr: `rr record` exited with status {}", status);
    }

    println!("cargo-rr: Run `cargo rr replay` to replay your trace");

    Ok(())
}

fn replay(trace: Option<String>, mut args: Vec<String>) -> anyhow::Result<()> {
    // Suppress so that it goes to rr
    ctrlc::set_handler(|| {})?;

    insert_trailing_args(&mut args, vec!["--quiet".to_string()]);

    if let Some(trace) = trace {
        args.push(trace);
    }

    let mut cmd = Command::new("rr")
        .arg("replay")
        .args(&["-d", "rust-gdb"])
        .args(args)
        .spawn()
        .context("Failed to run rr")?;

    let status = cmd.wait()?;
    if !status.success() {
        return Err(anyhow!(
            "cargo-rr: `rr replay` exited with status {}",
            status
        ));
    }

    Ok(())
}

fn ls(args: &[String]) -> anyhow::Result<()> {
    let mut cmd = Command::new("rr")
        .arg("ls")
        .arg("-t") // sort chronologically
        .args(args)
        .spawn()
        .context("Failed to run rr")?;

    let status = cmd.wait()?;
    if !status.success() {
        return Err(anyhow!("cargo-rr: `rr ls` exited with status {}", status));
    }

    Ok(())
}

fn insert_trailing_args(args: &mut Vec<String>, trailing: Vec<String>) {
    if let Some(i) = args.iter().position(|a| a == "--") {
        for (n, arg) in trailing.into_iter().enumerate() {
            args.insert(i + 1 + n, arg);
        }
    } else {
        args.push("--".to_string());
        for arg in trailing {
            args.push(arg);
        }
    }
}

fn select_artifact<'a>(meta: &Metadata, artifacts: &'a [Artifact]) -> anyhow::Result<&'a Artifact> {
    let names: Vec<_> = artifacts.iter().map(|a| artifact_name(meta, a)).collect();

    let selected = Select::new()
        .with_prompt("Pick an artifact to run")
        .items(&names)
        .default(0)
        .interact()?;

    Ok(&artifacts[selected])
}

fn artifact_name(meta: &Metadata, artifact: &Artifact) -> String {
    let src = &artifact.target.src_path;
    let src = src.strip_prefix(&meta.workspace_root).unwrap_or(src);
    src.to_string()
}
