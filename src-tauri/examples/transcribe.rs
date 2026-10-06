//! End-to-end diagnostic: imports one local audio file into an isolated temporary archive.
use anyhow::{Context, Result};
use dreamwhisper_lib::{archive, service::Service, worker::Worker};
use std::{path::PathBuf, sync::atomic::AtomicBool};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 4 && args.len() != 5 {
        anyhow::bail!("Usage: transcribe PYTHON MODEL_DIRECTORY AUDIO_FILE [sv|en]");
    }
    let temp = tempfile::tempdir()?;
    let service = Service::open(temp.path().join("data"))?;
    let input = PathBuf::from(&args[3]);
    archive::import_file(&service.db, &service.root, &input, None)?;
    if let Some(language) = args.get(4) {
        let recording = service.db.recordings()?.remove(0);
        service
            .db
            .set_recording_language(&recording.id, &language.to_string_lossy())?;
    }
    let job = service.db.claim()?.context("Inget jobb skapades")?;
    let mut settings = service.db.settings(&service.root)?;
    settings.python_path = args[1].to_string_lossy().into();
    settings.model_path = args[2].to_string_lossy().into();
    let stop = AtomicBool::new(false);
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/worker.py");
    let result = (|| -> Result<()> {
        let mut worker = Worker::spawn(&settings, &script, &service.root, &stop)?;
        let (metadata, segments) = worker.transcribe(&job, &stop, |msg| println!("{msg}"))?;
        service.db.complete(&job, &metadata, &segments)?;
        let t = service.db.transcript(&job.recording_id, None)?;
        assert_eq!(service.db.recordings()?[0].status, "completed");
        assert_eq!(t.segments.len(), segments.len());
        assert_eq!(metadata["device"], "cuda");
        assert_eq!(metadata["compute_type"], "float16");
        assert!(!archive::import_file(
            &service.db,
            &service.root,
            &input,
            None
        )?);
        println!(
            "Godkänt: verifierad import → beständig kö → CUDA FP16 → {} sparade textsegment",
            segments.len()
        );
        println!(
            "Modellrevision: {} · batch: {} · ljud: {} sekunder",
            metadata["model"]["revision"], metadata["batch_size"], metadata["duration"]
        );
        Ok(())
    })();
    if result.is_err() {
        if let Ok(log) = std::fs::read_to_string(service.root.join("logs/worker.log")) {
            eprintln!("{log}");
        }
    }
    result
}
