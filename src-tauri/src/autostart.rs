//! Opt-in XDG autostart. Only this app's desktop entry is managed.
use anyhow::{bail, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

pub fn configure(config: &Path, executable: &Path, enabled: bool) -> Result<()> {
    let entry = config.join("autostart/se.dreamwhisper.desktop.desktop");
    if !enabled && !entry.exists() {
        return Ok(());
    }
    let program = executable
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Programmets sökväg måste vara UTF-8"))?;
    if program.contains(['\n', '\r', '%', '=']) {
        bail!("Programmets sökväg kan inte användas för autostart");
    }
    let mut quoted = String::new();
    for c in program.chars() {
        match c {
            '\\' => quoted.push_str("\\\\\\\\"),
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(c);
            }
            _ => quoted.push(c),
        }
    }
    let parent = entry.parent().unwrap();
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!("dreamwhisper-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    write!(file,"[Desktop Entry]\nType=Application\nName=DreamWhisper\nComment=Lokalt ljudarkiv och transkribering\nExec=\"{quoted}\" --background\nTerminal=false\nHidden={}\n",!enabled)?;
    file.sync_all()?;
    fs::rename(&temporary, &entry)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
