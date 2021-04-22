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

#[derive(StructOpt, Debug)]
#[structopt(bin_name = "cargo")]
enum OptWrapper {
    #[structopt(name = "rr")]
    Opt(Opt),
}

#[derive(StructOpt, Debug)]
#[structopt(about, author)]
enum Opt {
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
        println!(); // to separate our output from anything cargo outputs
        return Err(err);
    }
    Ok(())
}

fn run() -> anyhow::Result<()> {
    let OptWrapper::Opt(opt) = OptWrapper::from_args();

    match opt {
        Opt::Test { opts, rr_opts } => {
            let bin = build_and_select(&opts)?;
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

fn build_and_select(opts: &[String]) -> anyhow::Result<Utf8PathBuf> {
    use cargo_metadata::Message;

    let meta = meta()?;

    let mut cmd = Command::new("cargo")
        .arg("test")
        .args(opts)
        .arg("--no-run")
        .arg("--message-format=json-render-diagnostics")
        .stdout(Stdio::piped())
        .spawn()?;

    let reader = BufReader::new(cmd.stdout.take().expect("Piped stdout"));

    let mut artifacts = Vec::new();
    for msg in Message::parse_stream(reader) {
        if let Message::CompilerArtifact(artifact) = msg? {
            if is_test_artifact(&meta, &artifact) {
                artifacts.push(artifact);
            }
        }
    }

    let artifact = match artifacts.len() {
        0 => return Err(anyhow!("cargo-rr: No test artifacts built.")),
        1 => &artifacts[0],
        _ => select_artifact(&meta, &artifacts)?,
    };

    let bin = artifact
        .executable
        .as_ref()
        .context("Artifact has no executable")?;

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

fn is_test_artifact(meta: &Metadata, artifact: &Artifact) -> bool {
    let members = &meta.workspace_members;
    artifact.executable.is_some()
        && artifact.target.kind.iter().any(|k| k == "test")
        && members.iter().any(|w| w == &artifact.package_id)
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
