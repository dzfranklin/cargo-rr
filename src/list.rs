use anyhow::anyhow;
use std::fs;

use crate::traces_dir;

pub fn list() -> anyhow::Result<()> {
    let root = traces_dir()?;
    let mut items = Vec::new();

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| anyhow!("Trace name not valid unicode"))?;
            let created = entry.metadata()?.created()?;
            items.push((created, name.to_owned()));
        }
    }

    items.sort_by(|(a, _), (b, _)| a.cmp(&b));

    for (_, name) in items {
        println!("{}", name);
    }

    Ok(())
}
