use crate::{archive, db::Database, devices, types::*, worker::Worker};
use anyhow::{Context, Result};
use fs2::FileExt;
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Default)]
struct Runtime {
    devices: Vec<Device>,
    device_error: Option<String>,
    activity: String,
    last_import: Option<ImportReport>,
}
pub struct Service {
    pub db: Database,
    pub root: PathBuf,
    runtime: Mutex<Runtime>,
    importing: Mutex<()>,
    pub stop: AtomicBool,
    _lock: File,
}
impl Service {
    pub fn open(root: PathBuf) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&root)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("app.lock"))?;
        lock.try_lock_exclusive()
            .context("DreamWhisper körs redan med detta arkiv")?;
        let db = Database::open(&root)?;
        let recovery = archive::reconcile(&db, &root)?;
        Ok(Arc::new(Self {
            db,
            root,
            runtime: Mutex::new(Runtime {
                activity: if recovery.is_empty() {
                    "Redo".into()
                } else {
                    format!("Arkivkontroll: {}", recovery.join("; "))
                },
                ..Runtime::default()
            }),
            importing: Mutex::new(()),
            stop: AtomicBool::new(false),
            _lock: lock,
        }))
    }
    pub fn activity(&self, msg: impl Into<String>) {
        self.runtime.lock().unwrap().activity = msg.into();
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let runtime = self.runtime.lock().unwrap();
        Ok(Snapshot {
            recordings: self.db.recordings()?,
            devices: runtime.devices.clone(),
            settings: self.db.settings(&self.root)?,
            data_dir: self.root.to_string_lossy().into(),
            activity: runtime.activity.clone(),
            device_error: runtime.device_error.clone(),
            last_import: runtime.last_import.as_ref().map(|r| ImportReport {
                imported: r.imported,
                skipped: r.skipped,
                errors: r.errors.clone(),
            }),
        })
    }
    pub fn import(&self, path: &Path, device: Option<&str>) -> Result<ImportReport> {
        let _guard = self.importing.lock().unwrap();
        let report = archive::import_directory(&self.db, &self.root, path, device)?;
        self.runtime.lock().unwrap().last_import = Some(ImportReport {
            imported: report.imported,
            skipped: report.skipped,
            errors: report.errors.clone(),
        });
        Ok(report)
    }
    pub fn register(&self, id: &str) -> Result<()> {
        let d = devices::scan()?
            .into_iter()
            .find(|d| d.id == id)
            .context("Diktafonen är inte längre ansluten")?;
        self.db.register(&d)?;
        for current in &mut self.runtime.lock().unwrap().devices {
            current.registered = self.db.registered(current)?;
        }
        Ok(())
    }
    pub fn unregister(&self, id: &str) -> Result<()> {
        let d = self
            .runtime
            .lock()
            .unwrap()
            .devices
            .iter()
            .find(|d| d.id == id)
            .cloned()
            .context("Diktafonen saknas")?;
        self.db.unregister(&d)?;
        for current in &mut self.runtime.lock().unwrap().devices {
            current.registered = self.db.registered(current)?;
        }
        Ok(())
    }
    pub fn import_device(&self, id: &str) -> Result<ImportReport> {
        let d = devices::scan()?
            .into_iter()
            .find(|d| d.id == id)
            .context("Diktafonen är inte längre ansluten")?;
        let mount = devices::mount(&d.block_path)?;
        self.import(
            &PathBuf::from(mount).join("PRIVATE/SONY/REC_FILE"),
            Some(&d.id),
        )
    }
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn start(self: &Arc<Self>, script: PathBuf) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let signal_service = Arc::downgrade(self);
        std::thread::spawn(move || loop {
            if signal_service
                .upgrade()
                .is_none_or(|s| s.stop.load(Ordering::Relaxed))
            {
                break;
            }
            if let Err(e) = devices::watch(tx.clone()) {
                eprintln!("UDisks2 listener: {e:#}");
            }
            std::thread::sleep(Duration::from_secs(5));
        });
        let service = self.clone();
        std::thread::spawn(move || {
            let mut imported = HashSet::new();
            while !service.stop.load(Ordering::Relaxed) {
                match devices::scan() {
                    Ok(mut list) => {
                        for d in &mut list {
                            d.registered = service.db.registered(d).unwrap_or(false);
                        }
                        let connected: HashSet<_> = list
                            .iter()
                            .filter(|d| d.registered)
                            .map(|d| d.id.clone())
                            .collect();
                        imported.retain(|id| connected.contains(id));
                        {
                            let mut r = service.runtime.lock().unwrap();
                            r.devices = list.clone();
                            r.device_error = None;
                            if let Ok(saved) = service.db.saved_devices() {
                                for mut d in saved {
                                    if !list.iter().any(|current| current.id == d.id) {
                                        d.mount_path = None;
                                        d.block_path.clear();
                                        d.registered = true;
                                        r.devices.push(d);
                                    }
                                }
                            }
                        }
                        if service
                            .db
                            .settings(&service.root)
                            .is_ok_and(|s| s.auto_import)
                        {
                            for d in list
                                .iter()
                                .filter(|d| d.registered && !imported.contains(&d.id))
                                .cloned()
                                .collect::<Vec<_>>()
                            {
                                match service.import_device(&d.id) {
                                    Ok(r) => {
                                        if r.errors.is_empty() {
                                            imported.insert(d.id);
                                        }
                                    }
                                    Err(e) => {
                                        service.runtime.lock().unwrap().device_error =
                                            Some(format!("Import: {e:#}"))
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        service.runtime.lock().unwrap().device_error =
                            Some(format!("UDisks2: {e:#}"))
                    }
                }
                // Signals trigger immediate rescan; timeout repairs missed signals and service restarts.
                let _ = rx.recv_timeout(Duration::from_secs(5));
            }
        });
        let service = self.clone();
        std::thread::spawn(move || {
            let mut worker: Option<Worker> = None;
            let mut configuration = String::new();
            let mut startup_failed = false;
            while !service.stop.load(Ordering::Relaxed) {
                let result = (|| -> Result<()> {
                    let settings = service.db.settings(&service.root)?;
                    let config = serde_json::to_string(&settings)?;
                    if config != configuration {
                        worker = None;
                        startup_failed = false;
                        configuration = config;
                    }
                    if !settings.transcription_enabled {
                        std::thread::sleep(Duration::from_millis(500));
                        return Ok(());
                    }
                    if startup_failed {
                        std::thread::sleep(Duration::from_millis(500));
                        return Ok(());
                    }
                    if worker.is_none() {
                        service.activity("Laddar KB-Whisper på GPU …");
                        match Worker::spawn(&settings, &script, &service.root, &service.stop) {
                            Ok(w) => {
                                worker = Some(w);
                                service.activity("Redo för transkribering");
                            }
                            Err(e) => {
                                startup_failed = true;
                                service.activity(format!("Worker: {e:#}. Stäng av och slå på transkribering efter att felet åtgärdats."));
                                return Ok(());
                            }
                        }
                    }
                    let Some(job) = service.db.claim()? else {
                        std::thread::sleep(Duration::from_millis(500));
                        return Ok(());
                    };
                    let result = worker
                        .as_mut()
                        .unwrap()
                        .transcribe(&job, &service.stop, |msg| service.activity(msg));
                    match result {
                        Ok((metadata, segments)) => {
                            if let Err(e) = service.db.complete(&job, &metadata, &segments) {
                                service.db.failed(
                                    &job.id,
                                    &format!("Resultatet kunde inte sparas: {e:#}"),
                                )?;
                                service.activity(format!("Databasfel: {e:#}"));
                            } else {
                                service.activity("Transkribering klar");
                            }
                        }
                        Err(e) => {
                            if !service.stop.load(Ordering::Relaxed) {
                                service.db.failed(&job.id, &format!("{e:#}"))?;
                            }
                            worker = None;
                            service.activity(format!("Transkribering misslyckades: {e:#}"));
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    service.activity(format!("Köfel: {e:#}"));
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
            // Worker::drop terminates and reaps the child. Running jobs recover on next startup.
            drop(worker);
        });
    }
}

pub fn export_text(transcript: &Transcript, format: &str) -> Result<String> {
    let segments = &transcript.segments;
    let text = match format {
        "txt" => segments
            .iter()
            .map(|s| s.edited_text.as_deref().unwrap_or(&s.text))
            .collect::<Vec<_>>()
            .join("\n"),
        "md" => segments
            .iter()
            .map(|s| {
                format!(
                    "**{}** {}",
                    timestamp(s.start, false),
                    s.edited_text.as_deref().unwrap_or(&s.text)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        "srt" => segments
            .iter()
            .enumerate()
            .map(|(i, s)| {
                format!(
                    "{}\n{} --> {}\n{}\n",
                    i + 1,
                    timestamp(s.start, true),
                    timestamp(s.end, true),
                    s.edited_text.as_deref().unwrap_or(&s.text)
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => anyhow::bail!("Okänt exportformat"),
    };
    Ok(format!("{text}\n"))
}
fn timestamp(seconds: f64, millis: bool) -> String {
    let ms = (seconds * 1000.0).round().max(0.0) as u64;
    let (h, m, s) = (ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60);
    if millis {
        format!("{h:02}:{m:02}:{s:02},{:03}", ms % 1000)
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subtitle_export_uses_corrections_and_rounds_across_hour_boundary() -> Result<()> {
        let t = Transcript {
            run_id: Some("run".into()),
            metadata: None,
            segments: vec![Segment {
                id: 1,
                start: 3599.9996,
                end: 3601.23,
                text: "Original".into(),
                edited_text: Some("Rättat".into()),
                words: vec![],
            }],
        };
        assert_eq!(
            export_text(&t, "srt")?,
            "1\n01:00:00,000 --> 01:00:01,230\nRättat\n\n"
        );
        assert_eq!(export_text(&t, "txt")?, "Rättat\n");
        assert!(export_text(&t, "../../unsafe").is_err());
        Ok(())
    }
    #[test]
    fn same_archive_cannot_be_opened_by_two_apps() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let first = Service::open(temp.path().to_path_buf())?;
        assert!(Service::open(temp.path().to_path_buf()).is_err());
        drop(first);
        assert!(Service::open(temp.path().to_path_buf()).is_ok());
        Ok(())
    }
}
