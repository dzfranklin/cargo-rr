use std::process::Command;

use anyhow::{anyhow, Context};

use crate::{split_opts, Trace};

pub fn replay(trace: Trace, rr_opts: Option<&str>, gdb_opts: Option<&str>) -> anyhow::Result<()> {
    // Ignore, as gdb handles
    ctrlc::set_handler(|| {})?;

    let mut gdb_opts = split_opts(gdb_opts);
    gdb_opts.push("--quiet");

    let mut cmd = Command::new("rr")
        .arg("replay")
        .args(split_opts(rr_opts))
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
