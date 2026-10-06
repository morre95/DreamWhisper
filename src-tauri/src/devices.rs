use crate::types::Device;
use anyhow::{Context, Result};
use std::{collections::HashMap, os::unix::ffi::OsStringExt, path::PathBuf};
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
            .is_some_and(|m| PathBuf::from(m).join("PRIVATE/SONY/REC_FILE").is_dir());
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
