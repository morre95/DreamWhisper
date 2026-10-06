//! GPU diagnostic using an isolated temporary journal; does not modify the user's archive.
use anyhow::{Context, Result};
use dreamwhisper_lib::{
    db::Database,
    drafting,
    journal::{DraftingSettings, EntryInput},
};
use std::{path::PathBuf, sync::atomic::AtomicBool};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 5 || (args.len() == 6 && args[5] == "--combine"),
        "Usage: draft LLAMA_SERVER GGUF_FILE sv|en DESCRIPTION_FILE [--combine]"
    );
    let temp = tempfile::tempdir()?;
    let db = Database::open(temp.path())?;
    db.save_drafting_settings(DraftingSettings {
        binary_path: args[1].clone(),
        model_path: args[2].clone(),
    })?;
    let entry = db.save_journal_entry(EntryInput {
        id: None,
        version: None,
        title: "Validation".into(),
        date: chrono::Local::now().format("%Y-%m-%d").to_string(),
        kind: "dream".into(),
        language: args[3].clone(),
        description: std::fs::read_to_string(PathBuf::from(&args[4]))?,
        reflections: "This reflection must remain excluded".into(),
        recording_id: None,
        run_id: None,
    })?;
    db.request_drafting(
        &uuid::Uuid::new_v4().to_string(),
        &entry.id,
        &entry.description_revision,
        false,
        vec![],
    )?;
    let job = db.claim_drafting()?.context("No drafting job")?;
    let started = std::time::Instant::now();
    let result = drafting::run(&job, temp.path(), &AtomicBool::new(false));
    if result.is_err() {
        eprintln!(
            "Runtime diagnostics: {}",
            std::fs::read_to_string(temp.path().join("logs/drafting.log")).unwrap_or_default()
        );
    }
    db.finish_drafting(job, result)?;
    let mut snapshot = db.journal_snapshot()?;
    snapshot.drafts.sort_by_key(|draft| draft.position);
    anyhow::ensure!(
        snapshot.jobs[0].status == "completed",
        "{}",
        snapshot.jobs[0].error.as_deref().unwrap_or("Draft failed")
    );
    println!(
        "{} scenes saved in {:.1}s; approvals remain pending",
        snapshot.drafts.len(),
        started.elapsed().as_secs_f64()
    );
    println!("{}", serde_json::to_string_pretty(&snapshot.drafts)?);
    if args.len() == 6 {
        db.request_drafting(
            &uuid::Uuid::new_v4().to_string(),
            &entry.id,
            &entry.description_revision,
            false,
            snapshot.drafts.iter().map(|d| d.id.clone()).collect(),
        )?;
        let combination = db.claim_drafting()?.context("No composition job")?;
        let result = drafting::run(&combination, temp.path(), &AtomicBool::new(false));
        db.finish_drafting(combination, result)?;
        let snapshot = db.journal_snapshot()?;
        anyhow::ensure!(
            snapshot.jobs.iter().all(|job| job.status == "completed"),
            "Composition failed"
        );
        println!(
            "Composition: {}",
            serde_json::to_string_pretty(
                &snapshot
                    .drafts
                    .iter()
                    .filter(|draft| !draft.scene_ids.is_empty())
                    .collect::<Vec<_>>()
            )?
        );
    }
    Ok(())
}
