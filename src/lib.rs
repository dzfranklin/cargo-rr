use camino::{Utf8Path, Utf8PathBuf};
use std::{
    fs::{self, File},
    io::{self, Read},
};

use anyhow::anyhow;

mod list;
mod record;
mod replay;

pub use list::list;
pub use record::record;
pub use replay::replay;

pub struct Trace(Utf8PathBuf);

impl Trace {
    pub fn new(name: &str) -> anyhow::Result<Self> {
        let root = traces_dir()?;
        let dir = root.join(name);
        if !dir.exists() {
            return Err(anyhow!("Trace `{}` does not exist in `{}`", name, root));
        }
        Ok(Self(dir))
    }

    fn name_for_bin(bin: &Utf8Path) -> anyhow::Result<Self> {
        let root = traces_dir()?;
        let bin_name = bin
            .file_name()
            .ok_or_else(|| anyhow!("Can't get file name of bin"))?;
        let mut dir = root.join(bin_name);

        let mut suffix = 0;
        loop {
            if !dir.exists() {
                break Ok(Self(dir));
            }
            suffix += 1;
            dir.set_file_name(format!("{}-{}", bin_name, suffix));
        }
    }

    pub fn name(&self) -> &str {
        &self.0.file_name().expect("Trace dir shouldn't end in ..")
    }

    pub fn dir(&self) -> &Utf8Path {
        self.0.as_path()
    }

    pub fn set_latest(&self) -> anyhow::Result<()> {
        let root = self.0.parent().expect("Trace has parent");
        fs::write(root.join("latest"), self.name())?;
        Ok(())
    }

    pub fn latest() -> anyhow::Result<Self> {
        let root = traces_dir()?;
        let mut file = match File::open(root.join("latest")) {
            Ok(f) => f,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(anyhow!("No trace in `{}`", root));
            }
            Err(err) => return Err(err.into()),
        };
        let mut latest = String::new();
        file.read_to_string(&mut latest)?;
        let latest = latest.trim();
        let latest = fs::canonicalize(root.join(latest))?;
        let latest = Utf8PathBuf::from_path_buf(latest)
            .map_err(|_| anyhow!("Path to target dir must be utf-8"))?;
        Ok(Self(latest))
    }
}

pub fn traces_dir() -> anyhow::Result<Utf8PathBuf> {
    let meta = cargo_metadata::MetadataCommand::new().no_deps().exec()?;
    let dir = meta.target_directory.join("rr");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn split_rr_opts(opts: Option<&str>) -> Vec<&str> {
    opts.map_or_else(Vec::new, |s| s.split(' ').collect())
}
