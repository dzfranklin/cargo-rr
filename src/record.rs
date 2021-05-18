use std::process::Command;

use anyhow::Context;
use camino::Utf8PathBuf;
use tracing::debug;

use crate::{split_rr_opts, Trace};

pub fn record(bin: Utf8PathBuf, rr_opts: Option<&str>, args: &[String]) -> anyhow::Result<Trace> {
    debug!(?bin, ?args, "Recording");

    let trace = Trace::name_for_bin(&bin)?;

    let mut cmd = Command::new("rr")
        .arg("record")
        .args(&split_rr_opts(rr_opts))
        .args(&["--output-trace-dir", trace.0.as_str()])
        .arg(bin)
        .arg("--")
        .args(args)
        .spawn()
        .context("Failed to run rr")?;

    trace.set_latest()?;

    let status = cmd.wait()?;
    if !status.success() {
        // Not an error as this might just mean the recorded program failed
        println!("cargo-rr: `rr record` exited with status {}", status);
    }

    Ok(trace)
}
