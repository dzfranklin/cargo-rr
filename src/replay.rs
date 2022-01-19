use std::process::Command;

use anyhow::{anyhow, Context};

use crate::{split_rr_opts, Trace};

pub fn replay(
    trace: Trace,
    rr_opts: Option<&str>,
    mut gdb_opts: Vec<String>,
) -> anyhow::Result<()> {
    // Ignore, as gdb handles
    ctrlc::set_handler(|| {})?;

    gdb_opts.push("--quiet".into());

    let mut cmd = Command::new("rr")
        .arg("replay")
        .args(&split_rr_opts(rr_opts))
        .args(&["-d", "rust-gdb"])
        .arg(trace.dir())
        .arg("--")
        .args(gdb_opts)
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
