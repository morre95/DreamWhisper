use crate::types::Device;
use anyhow::{bail, Context, Result};
use std::{
    collections::HashMap,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};
use zbus::{
    blocking::{Connection, MessageIterator, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue},
    MatchRule,
};

type Properties = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<String, Properties>>;
fn string(p: &Properties, key: &str) -> String {
    p.get(key)
        .and_then(|v| <&str>::try_from(v).ok())
        .unwrap_or("")
        .to_string()
}

/// Sony recorders expose either a root REC_FILE or a nested PRIVATE/SONY layout.
/// Restrict import to recording folders so MUSIC and other device content are excluded.
pub fn recording_directories(mount: &Path) -> Result<Vec<PathBuf>> {
    if !mount.is_dir() {
        bail!(
            "Diktafonens monteringspunkt går inte att öppna: {}",
            mount.display()
        );
    }
    let candidates = [mount.join("REC_FILE"), mount.join("PRIVATE/SONY/REC_FILE")];
    let mut directories = Vec::new();
    for candidate in &candidates {
        match std::fs::metadata(candidate) {
            Ok(metadata) if metadata.is_dir() => directories.push(candidate.clone()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).with_context(|| {
                    format!(
                        "Inspelningsmappen går inte att läsa: {}",
                        candidate.display()
                    )
                })
            }
        }
    }
    if directories.is_empty() {
        bail!("Ingen inspelningsmapp hittades på {}. Kontrollerade REC_FILE och PRIVATE/SONY/REC_FILE",mount.display());
    }
    Ok(directories)
}

pub fn scan() -> Result<Vec<Device>> {
    let conn = Connection::system().context("Kan inte ansluta till systemets D-Bus")?;
    let proxy = Proxy::new(
        &conn,
        "org.freedesktop.UDisks2",
        "/org/freedesktop/UDisks2",
        "org.freedesktop.DBus.ObjectManager",
    )?;
    let objects: Objects = proxy.call("GetManagedObjects", &())?;
    let mut devices = vec![];
    for (path, interfaces) in &objects {
        let (Some(block), Some(fs)) = (
            interfaces.get("org.freedesktop.UDisks2.Block"),
            interfaces.get("org.freedesktop.UDisks2.Filesystem"),
        ) else {
            continue;
        };
        let Some(drive_path) = block
            .get("Drive")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| OwnedObjectPath::try_from(v).ok())
        else {
            continue;
        };
        let Some(drive) = objects
            .get(&drive_path)
            .and_then(|i| i.get("org.freedesktop.UDisks2.Drive"))
        else {
            continue;
        };
        if string(drive, "ConnectionBus") != "usb" {
            continue;
        }
        let vendor = string(drive, "Vendor");
        let model = string(drive, "Model");
        let serial = string(drive, "Serial");
        let uuid = string(block, "IdUUID");
        let label = string(block, "IdLabel");
        let mounts: Vec<Vec<u8>> = fs
            .get("MountPoints")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| Vec::<Vec<u8>>::try_from(v).ok())
            .unwrap_or_default();
        let mount_path = mounts.into_iter().next().map(|mut bytes| {
            if bytes.last() == Some(&0) {
                bytes.pop();
            }
            PathBuf::from(std::ffi::OsString::from_vec(bytes))
                .to_string_lossy()
                .into_owned()
        });
        let sony = vendor.to_lowercase().contains("sony")
            || model.to_lowercase().contains("ux570")
            || label == "IC RECORDER";
        let has_recordings = mount_path
            .as_ref()
            .is_some_and(|m| recording_directories(Path::new(m)).is_ok());
        if !sony && !has_recordings {
            continue;
        }
        devices.push(Device {
            id: format!("serial:{serial}:uuid:{uuid}"),
            block_path: path.to_string(),
            vendor,
            model,
            serial,
            uuid,
            label,
            mount_path,
            registered: false,
        });
    }
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(devices)
}

pub fn mount(block_path: &str) -> Result<String> {
    // Only mount a current recorder candidate, never an arbitrary frontend-supplied block device.
    let devices = scan()?;
    let d = devices
        .iter()
        .find(|d| d.block_path == block_path)
        .context("Diktafonen är inte längre ansluten")?;
    if let Some(path) = &d.mount_path {
        return Ok(path.clone());
    }
    let conn = Connection::system()?;
    let proxy = Proxy::new(
        &conn,
        "org.freedesktop.UDisks2",
        block_path,
        "org.freedesktop.UDisks2.Filesystem",
    )?;
    let options: HashMap<&str, zbus::zvariant::Value<'_>> =
        HashMap::from([("options", zbus::zvariant::Value::from("ro"))]);
    let path: String = proxy
        .call("Mount", &(options,))
        .context("Montering misslyckades. Montera enheten i filhanteraren och försök igen")?;
    Ok(path)
}

/// Signal listener runs separately from import, so slow USB reads cannot block D-Bus.
pub fn watch(tx: std::sync::mpsc::SyncSender<()>) -> Result<()> {
    let conn = Connection::system()?;
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.UDisks2")?
        .path_namespace("/org/freedesktop/UDisks2")?
        .build();
    let iter = MessageIterator::for_match_rule(rule, &conn, Some(64))?;
    let _ = tx.try_send(());
    for message in iter {
        message?;
        let _ = tx.try_send(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{archive, db::Database};
    #[test]
    fn imports_both_sony_layouts_and_excludes_music() -> Result<()> {
        for layout in ["REC_FILE", "PRIVATE/SONY/REC_FILE"] {
            let temp = tempfile::tempdir()?;
            let mount = temp.path().join("recorder");
            let recordings = mount.join(layout).join("FOLDER01");
            std::fs::create_dir_all(&recordings)?;
            std::fs::create_dir_all(mount.join("MUSIC"))?;
            std::fs::write(recordings.join("note.MP3"), b"recorded audio")?;
            std::fs::write(mount.join("MUSIC/song.mp3"), b"music")?;
            let root = temp.path().join("archive-data");
            let db = Database::open(&root)?;
            let directories = recording_directories(&mount)?;
            assert_eq!(directories, vec![mount.join(layout)]);
            let report = archive::import_directory(&db, &root, &directories[0], Some("sony"))?;
            assert_eq!(report.imported, 1);
            assert!(report.errors.is_empty());
            assert_eq!(db.recordings()?[0].name, "note.MP3");
        }
        Ok(())
    }
    #[test]
    fn finds_all_recording_roots_when_both_exist() -> Result<()> {
        let temp = tempfile::tempdir()?;
        for layout in ["REC_FILE", "PRIVATE/SONY/REC_FILE"] {
            std::fs::create_dir_all(temp.path().join(layout))?;
        }
        assert_eq!(recording_directories(temp.path())?.len(), 2);
        Ok(())
    }
    #[test]
    fn missing_recording_folder_reports_checked_paths() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let error = recording_directories(temp.path()).unwrap_err().to_string();
        assert!(error.contains("Ingen inspelningsmapp"));
        assert!(error.contains("REC_FILE"));
        assert!(error.contains("PRIVATE/SONY/REC_FILE"));
        assert!(error.contains(&temp.path().display().to_string()));
        Ok(())
    }
}
