use crate::{db::Database, types::*};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path},
};
use walkdir::WalkDir;

pub fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut input = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    let mut size = 0;
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((format!("{:x}", hash.finalize()), size))
}

pub fn import_directory(
    db: &Database,
    root: &Path,
    source: &Path,
    device: Option<&str>,
) -> Result<ImportReport> {
    let source = source
        .canonicalize()
        .context("Importmappen går inte att öppna")?;
    if !source.is_dir() {
        bail!("Välj en mapp med ljudinspelningar");
    }
    let archive = root.join("archive");
    fs::create_dir_all(&archive)?;
    let archive_real = archive.canonicalize()?;
    if source.starts_with(&archive_real) || archive_real.starts_with(&source) {
        bail!("Importmappen får inte innehålla appens ljudarkiv");
    }
    let mut report = ImportReport::default();
    for entry in WalkDir::new(&source).follow_links(false) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                report.errors.push(e.to_string());
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["wav", "mp3"].contains(&ext.as_str()) {
            continue;
        }
        match import_file(db, root, entry.path(), device) {
            Ok(true) => report.imported += 1,
            Ok(false) => report.skipped += 1,
            Err(e) => report
                .errors
                .push(format!("{}: {e:#}", entry.path().display())),
        }
    }
    Ok(report)
}

pub fn import_file(
    db: &Database,
    root: &Path,
    source: &Path,
    device: Option<&str>,
) -> Result<bool> {
    let before = fs::metadata(source)?;
    if !before.is_file() || before.len() == 0 {
        bail!("Tom eller ogiltig ljudfil");
    }
    let archive = root.join("archive");
    fs::create_dir_all(&archive)?;
    let ext = source
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["wav", "mp3"].contains(&ext.as_str()) {
        bail!("Endast WAV och MP3 stöds");
    }
    let temp = archive.join(format!("{}.part", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        let (hash, size) = hash_file(&temp)?;
        let (source_hash, source_size) = hash_file(source)?;
        let after = fs::metadata(source)?;
        if size != before.len()
            || source_size != size
            || source_hash != hash
            || before.len() != after.len()
            || before.modified()? != after.modified()?
        {
            bail!("Källfilen ändrades eller kopian kunde inte verifieras");
        }
        let dest = archive.join(format!("{hash}.{ext}"));
        let manifest_path = archive.join(format!("{hash}.json"));
        let m = ImportManifest {
            id: uuid::Uuid::new_v4().to_string(),
            name: source
                .file_name()
                .context("Filnamn saknas")?
                .to_string_lossy()
                .into(),
            archive_file: format!("{hash}.{ext}"),
            sha256: hash.clone(),
            size,
            source_path: source.to_string_lossy().into(),
            source_device: device.map(str::to_owned),
            source_modified_at: before
                .modified()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
            imported_at: chrono::Utc::now().to_rfc3339(),
        };
        // Write provenance before publishing the audio; recovery ignores a manifest without audio.
        if !manifest_path.exists() {
            let tmp_manifest = archive.join(format!("{}.manifest.part", uuid::Uuid::new_v4()));
            let mut f = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_manifest)?;
            f.write_all(&serde_json::to_vec_pretty(&m)?)?;
            f.sync_all()?;
            fs::rename(&tmp_manifest, &manifest_path)?;
            File::open(&archive)?.sync_all()?;
        }
        if dest.exists() {
            if hash_file(&dest)? != (hash, size) {
                bail!("Befintlig arkivfil är skadad; importen avbröts");
            }
        } else {
            fs::rename(&temp, &dest)?;
            File::open(&archive)?.sync_all()?;
        }
        db.add_manifest(&m, &dest)
    })();
    // Only our own temporary file is removed. Originals are never changed.
    if temp.exists() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn reconcile(db: &Database, root: &Path) -> Result<Vec<String>> {
    let archive = root.join("archive");
    fs::create_dir_all(&archive)?;
    let mut errors = vec![];
    for entry in fs::read_dir(&archive)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let result = (|| -> Result<()> {
            let m: ImportManifest = serde_json::from_slice(&fs::read(&path)?)?;
            let parts: Vec<_> = Path::new(&m.archive_file).components().collect();
            if parts.len() != 1
                || !matches!(parts[0], Component::Normal(_))
                || m.sha256.len() != 64
                || !m.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || !m.archive_file.starts_with(&format!("{}.", m.sha256))
            {
                bail!("Ogiltigt arkivmanifest");
            }
            let audio = archive.join(&m.archive_file);
            if audio.is_file() && !db.has_hash(&m.sha256)? {
                if hash_file(&audio)? != (m.sha256.clone(), m.size) {
                    bail!("Arkivfilens hash stämmer inte");
                }
                db.add_manifest(&m, &audio)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            errors.push(format!("{}: {e:#}", path.display()));
        }
    }
    db.reconcile_missing()?;
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn import_is_verified_idempotent_and_original_is_untouched() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("data");
        let src = temp.path().join("recorder");
        fs::create_dir_all(&src)?;
        let audio = src.join("note.MP3");
        fs::write(&audio, b"some audio content")?;
        let db = Database::open(&root)?;
        assert!(import_file(&db, &root, &audio, Some("sony"))?);
        assert!(!import_file(&db, &root, &audio, Some("sony"))?);
        assert_eq!(db.recordings()?.len(), 1);
        assert_eq!(fs::read(&audio)?, b"some audio content");
        assert_eq!(
            fs::read(&db.recordings()?[0].archive_path)?,
            fs::read(&audio)?
        );
        assert!(db.claim()?.is_some());
        assert!(db.claim()?.is_none());
        Ok(())
    }
    #[test]
    fn manifest_recovers_crash_before_database_commit() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("data");
        let audio = temp.path().join("note.wav");
        fs::write(&audio, b"recover me")?;
        let db = Database::open(&root)?;
        import_file(&db, &root, &audio, None)?;
        let c = db.connect()?;
        c.execute("DELETE FROM jobs", [])?;
        c.execute("DELETE FROM sources", [])?;
        c.execute("DELETE FROM recordings", [])?;
        reconcile(&db, &root)?;
        assert_eq!(db.recordings()?.len(), 1);
        assert_eq!(db.recordings()?[0].status, "queued");
        Ok(())
    }
    #[test]
    fn corrupted_archive_is_not_silently_reused() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("data");
        let audio = temp.path().join("a.wav");
        fs::write(&audio, b"original")?;
        let db = Database::open(&root)?;
        import_file(&db, &root, &audio, None)?;
        fs::write(&db.recordings()?[0].archive_path, b"corrupt")?;
        assert!(import_file(&db, &root, &audio, None).is_err());
        Ok(())
    }
}
