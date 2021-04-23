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
    #[structopt(about = "Record a binary or example")]
    Run {
        #[structopt(name = "OPTIONS", help = "Options to pass to `cargo test`")]
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
    #[structopt(about = "Replay a trace")]
    Replay {
        #[structopt(help = "Leave blank to replay the last trace recorded")]
        trace: Option<String>,
        #[structopt(
            last = true,
            help = "Options to pass to `rr replay`. See `rr replay -h`"
        )]
        rr_opts: Vec<String>,
    },
    #[structopt(about = "List traces")]
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
            let bin = build_and_select(false, &opts, |kind| kind == "bin" || kind == "example")?;
            record(&bin, &rr_opts)?;
        }
        Opt::Test { opts, rr_opts } => {
            let bin = build_and_select(true, &opts, |kind| kind == "test")?;
            record(&bin, &rr_opts)?;
        }
        Opt::Replay { rr_opts, trace } => replay(trace, &rr_opts)?,
        Opt::Ls { rr_opts } => ls(&rr_opts)?,
    }

    Ok(())
}

fn meta() -> anyhow::Result<Metadata> {
    let meta = cargo_metadata::MetadataCommand::new().exec()?;
    Ok(meta)
}

fn build_and_select<F>(
    is_test: bool,
    opts: &[String],
    kind_filter: F,
) -> anyhow::Result<Utf8PathBuf>
where
    F: Fn(&str) -> bool,
{
    use cargo_metadata::Message;

    let meta = meta()?;
    let workspace_members = &meta.workspace_members;

    let mut cmd = Command::new("cargo");

    if is_test {
        cmd.arg("test").arg("--no-run");
    } else {
        cmd.arg("build");
    }

    let mut cmd = cmd
        .args(opts)
        .arg("--message-format=json-render-diagnostics")
        .stdout(Stdio::piped())
        .spawn()?;

    let reader = BufReader::new(cmd.stdout.take().expect("Piped stdout"));

    let mut artifacts = Vec::new();
    for msg in Message::parse_stream(reader) {
        if let Message::CompilerArtifact(artifact) = msg? {
            if artifact.executable.is_some()
                && workspace_members.iter().any(|w| w == &artifact.package_id)
                && artifact.target.kind.iter().any(|s| kind_filter(s))
            {
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
        .context("Artifact has no executable")?;

    debug!("Selected bin {:?}", bin);

    Ok(bin.to_path_buf())
}

fn record(bin: &Utf8Path, args: &[String]) -> anyhow::Result<()> {
    println!();

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

    Ok(())
}

fn replay(trace: Option<String>, args: &[String]) -> anyhow::Result<()> {
    // Suppress so that it goes to rr
    ctrlc::set_handler(|| {})?;

    let mut args = args.to_vec();

    // Tell --quiet to gdb
    if let Some(i) = args.iter().position(|a| a == "--") {
        args.insert(i + 1, "--quiet".to_string());
    } else {
        args.push("--".to_string());
        args.push("--quiet".to_string());
    }

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
