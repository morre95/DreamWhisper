//! Local journal revisions and approval-bound generation. All writes are transactional.
use crate::{db::Database, images::ImageDraft};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn migrate(c: &Connection) -> Result<()> {
    c.execute_batch("BEGIN IMMEDIATE;
      CREATE TABLE IF NOT EXISTS journal_entries(id TEXT PRIMARY KEY, json TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS journal_descriptions(id TEXT PRIMARY KEY, entry_id TEXT NOT NULL REFERENCES journal_entries(id), json TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS journal_drafts(id TEXT PRIMARY KEY, entry_id TEXT NOT NULL REFERENCES journal_entries(id), json TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS drafting_jobs(id TEXT PRIMARY KEY, entry_id TEXT NOT NULL REFERENCES journal_entries(id), status TEXT NOT NULL, json TEXT NOT NULL);
      CREATE INDEX IF NOT EXISTS drafting_queue ON drafting_jobs(status);
      CREATE TABLE IF NOT EXISTS drafting_settings(id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS journal_generations(id TEXT PRIMARY KEY, entry_id TEXT NOT NULL REFERENCES journal_entries(id), json TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS recording_languages(recording_id TEXT PRIMARY KEY REFERENCES recordings(id), language TEXT NOT NULL);
      ")?;
    let has_language: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('jobs') WHERE name='language')",
        [],
        |r| r.get(0),
    )?;
    if !has_language {
        c.execute_batch("ALTER TABLE jobs ADD COLUMN language TEXT NOT NULL DEFAULT 'sv';")?;
    }
    c.execute_batch("PRAGMA user_version=4; COMMIT;")?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub version: u64,
    pub title: String,
    pub date: String,
    pub kind: String,
    pub language: String,
    pub description: String,
    pub reflections: String,
    pub description_revision: String,
    pub confirmed_revision: Option<String>,
    pub recording_id: Option<String>,
    pub run_id: Option<String>,
    pub favourite_image: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntryInput {
    pub id: Option<String>,
    pub version: Option<u64>,
    pub title: String,
    pub date: String,
    pub kind: String,
    pub language: String,
    pub description: String,
    pub reflections: String,
    pub recording_id: Option<String>,
    pub run_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DescriptionRevision {
    pub id: String,
    pub entry_id: String,
    pub description: String,
    pub language: String,
    pub recording_id: Option<String>,
    pub run_id: Option<String>,
    pub created_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Detail {
    pub text: String,
    pub quote: Option<String>,
    pub english: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scene {
    pub title: String,
    pub summary: String,
    pub details: Vec<Detail>,
    pub additions: Vec<Detail>,
    pub questions: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SceneDraft {
    #[serde(default)]
    pub position: u32,
    #[serde(default)]
    pub batch_created_at: String,
    pub id: String,
    pub entry_id: String,
    pub description_revision: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub scene: Scene,
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub manual: bool,
    pub approved: bool,
    pub scene_ids: Vec<String>,
    pub arrangement: String,
    pub provenance: Value,
    pub created_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DraftEdit {
    pub id: String,
    pub scene: Scene,
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub manual: bool,
    pub arrangement: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DraftingSettings {
    pub binary_path: String,
    pub model_path: String,
}
impl Default for DraftingSettings {
    fn default() -> Self {
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        Self {
            binary_path: project
                .join(".local/llama.cpp/build/bin/llama-server")
                .to_string_lossy()
                .into(),
            model_path: project
                .join("models/qwen3-8b/Qwen3-8B-Q4_K_M.gguf")
                .to_string_lossy()
                .into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DraftingJob {
    pub id: String,
    pub entry_id: String,
    pub status: String,
    pub error: Option<String>,
    pub description_revision: String,
    pub description: String,
    pub language: String,
    pub reflections: Option<String>,
    pub scene_ids: Vec<String>,
    pub scenes: Vec<SceneDraft>,
    pub settings: DraftingSettings,
    pub created_at: String,
    pub result_ids: Vec<String>,
}
#[derive(Serialize, Deserialize)]
struct GenerationRecord {
    draft_ids: Vec<String>,
    workflow_id: String,
    job_ids: Vec<String>,
}

#[derive(Serialize)]
pub struct JournalSnapshot {
    pub entries: Vec<JournalEntry>,
    pub drafts: Vec<SceneDraft>,
    pub jobs: Vec<DraftingJob>,
    pub descriptions: Vec<DescriptionRevision>,
    pub settings: DraftingSettings,
    pub setup_ready: bool,
}
fn read<T: serde::de::DeserializeOwned>(c: &Connection, table: &str, key: &str) -> Result<T> {
    let json: String = c
        .query_row(
            &format!("SELECT json FROM {table} WHERE id=?1"),
            [key],
            |r| r.get(0),
        )
        .optional()?
        .context("Posten finns inte")?;
    Ok(serde_json::from_str(&json)?)
}
fn all<T: serde::de::DeserializeOwned>(c: &Connection, table: &str) -> Result<Vec<T>> {
    let mut q = c.prepare(&format!("SELECT json FROM {table} ORDER BY rowid DESC"))?;
    let rows = q
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|json| Ok(serde_json::from_str(&json)?))
        .collect()
}
fn save_entry(c: &Connection, entry: &JournalEntry) -> Result<()> {
    c.execute("INSERT INTO journal_entries VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json", params![entry.id, serde_json::to_string(entry)?])?;
    Ok(())
}
fn ensure_latest(c: &Connection, key: &str) -> Result<()> {
    let superseded: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM journal_drafts WHERE json_extract(json,'$.parent_id')=?1)",
        [key],
        |r| r.get(0),
    )?;
    ensure!(
        !superseded,
        "Utkastet har en nyare version. Granska den senaste versionen."
    );
    Ok(())
}
fn insert_draft(c: &Connection, draft: &SceneDraft) -> Result<()> {
    c.execute(
        "INSERT INTO journal_drafts VALUES(?1,?2,?3)",
        params![draft.id, draft.entry_id, serde_json::to_string(draft)?],
    )?;
    Ok(())
}
pub fn compose(scene: &Scene) -> String {
    scene
        .details
        .iter()
        .chain(&scene.additions)
        .map(|d| d.english.trim().trim_end_matches(['.', ' ']))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(". ")
}
fn validate_scene(scene: &Scene) -> Result<()> {
    ensure!(
        !scene.title.trim().is_empty() && scene.title.len() <= 1000,
        "Scenen behöver en titel"
    );
    ensure!(
        scene.summary.len() <= 20_000
            && scene.details.len() + scene.additions.len() <= 200
            && scene.questions.len() <= 20,
        "Scenen är för stor"
    );
    for d in scene.details.iter().chain(&scene.additions) {
        ensure!(
            !d.english.trim().is_empty()
                && d.english.len() <= 10_000
                && d.text.len() <= 10_000
                && d.quote.as_ref().is_none_or(|q| q.len() <= 20_000),
            "Ogiltig scendetalj"
        );
    }
    ensure!(
        scene.questions.iter().all(|s| s.len() <= 5000),
        "Frågan är för lång"
    );
    Ok(())
}

impl Database {
    pub fn journal_snapshot(&self) -> Result<JournalSnapshot> {
        let c = self.connect()?;
        let settings = self.drafting_settings()?;
        Ok(JournalSnapshot {
            entries: all(&c, "journal_entries")?,
            drafts: all(&c, "journal_drafts")?,
            jobs: all(&c, "drafting_jobs")?,
            descriptions: all(&c, "journal_descriptions")?,
            setup_ready: std::path::Path::new(&settings.binary_path).is_file()
                && std::path::Path::new(&settings.model_path).is_file(),
            settings,
        })
    }
    pub fn journal_entry(&self, key: &str) -> Result<JournalEntry> {
        read(&self.connect()?, "journal_entries", key)
    }
    pub fn save_journal_entry(&self, input: EntryInput) -> Result<JournalEntry> {
        ensure!(
            ["dream", "meditation"].contains(&input.kind.as_str()),
            "Välj dröm eller meditation"
        );
        ensure!(
            ["sv", "en"].contains(&input.language.as_str()),
            "Välj svenska eller engelska"
        );
        ensure!(
            !input.title.trim().is_empty() && input.title.len() <= 1000,
            "Ange en titel (max 1000 byte)"
        );
        ensure!(
            input.description.len() <= 100_000 && input.reflections.len() <= 100_000,
            "Texten är för lång (max 100 000 byte)"
        );
        chrono::NaiveDate::parse_from_str(&input.date, "%Y-%m-%d").context("Ogiltigt datum")?;
        ensure!(
            input.recording_id.is_some() || input.run_id.is_none(),
            "Transkriptet behöver en inspelning"
        );
        if let Some(recording) = &input.recording_id {
            let t = self.transcript(recording, input.run_id.as_deref())?;
            ensure!(
                input.run_id.is_some() && t.run_id == input.run_id,
                "Källtranskriptet finns inte"
            );
        }
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<JournalEntry> = input
            .id
            .as_ref()
            .map(|key| read(&tx, "journal_entries", key))
            .transpose()?;
        if let Some(old) = &previous {
            ensure!(
                input.version == Some(old.version),
                "Posten har ändrats. Läs in den igen innan du sparar."
            );
        }
        let changed = previous.as_ref().is_none_or(|p| {
            p.description != input.description
                || p.language != input.language
                || p.recording_id != input.recording_id
                || p.run_id != input.run_id
        });
        let stamp = now();
        let mut entry = JournalEntry {
            id: previous.as_ref().map_or_else(id, |p| p.id.clone()),
            version: previous.as_ref().map_or(1, |p| p.version + 1),
            title: input.title,
            date: input.date,
            kind: input.kind,
            language: input.language,
            description: input.description,
            reflections: input.reflections,
            description_revision: if changed {
                id()
            } else {
                previous.as_ref().unwrap().description_revision.clone()
            },
            confirmed_revision: previous.as_ref().and_then(|p| p.confirmed_revision.clone()),
            recording_id: input.recording_id,
            run_id: input.run_id,
            favourite_image: previous.as_ref().and_then(|p| p.favourite_image.clone()),
            created_at: previous
                .as_ref()
                .map_or_else(|| stamp.clone(), |p| p.created_at.clone()),
            updated_at: stamp,
        };
        if changed {
            entry.confirmed_revision = if entry.recording_id.is_none() {
                Some(entry.description_revision.clone())
            } else {
                None
            };
        }
        save_entry(&tx, &entry)?;
        if changed {
            let revision = DescriptionRevision {
                id: entry.description_revision.clone(),
                entry_id: entry.id.clone(),
                description: entry.description.clone(),
                language: entry.language.clone(),
                recording_id: entry.recording_id.clone(),
                run_id: entry.run_id.clone(),
                created_at: now(),
            };
            tx.execute(
                "INSERT INTO journal_descriptions VALUES(?1,?2,?3)",
                params![revision.id, entry.id, serde_json::to_string(&revision)?],
            )?;
        }
        tx.commit()?;
        Ok(entry)
    }
    pub fn confirm_description(&self, key: &str, revision: &str) -> Result<JournalEntry> {
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut entry: JournalEntry = read(&tx, "journal_entries", key)?;
        ensure!(
            entry.description_revision == revision,
            "Beskrivningen har ändrats. Granska den igen."
        );
        entry.confirmed_revision = Some(revision.into());
        entry.version += 1;
        entry.updated_at = now();
        save_entry(&tx, &entry)?;
        tx.commit()?;
        Ok(entry)
    }
    pub fn drafting_settings(&self) -> Result<DraftingSettings> {
        let json: Option<String> = self
            .connect()?
            .query_row("SELECT json FROM drafting_settings WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(match json {
            Some(s) => serde_json::from_str(&s)?,
            None => DraftingSettings::default(),
        })
    }
    pub fn save_drafting_settings(&self, settings: DraftingSettings) -> Result<()> {
        ensure!(
            !settings.binary_path.trim().is_empty() && !settings.model_path.trim().is_empty(),
            "Ange språkmodellens program och modellfil"
        );
        self.connect()?.execute("INSERT INTO drafting_settings VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json", [serde_json::to_string(&settings)?])?;
        Ok(())
    }
    pub fn request_drafting(
        &self,
        request_id: &str,
        entry_id: &str,
        revision: &str,
        include_reflections: bool,
        scene_ids: Vec<String>,
    ) -> Result<String> {
        ensure!(
            uuid::Uuid::parse_str(request_id).is_ok(),
            "Ogiltigt begäran-ID"
        );
        ensure!(
            scene_ids.is_empty() || (2..=50).contains(&scene_ids.len()),
            "Välj minst två scener att kombinera"
        );
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT json FROM drafting_jobs WHERE id=?1",
                [request_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let job: DraftingJob = serde_json::from_str(&existing)?;
            ensure!(
                job.entry_id == entry_id
                    && job.description_revision == revision
                    && job.reflections.is_some() == include_reflections
                    && job.scene_ids == scene_ids,
                "Begäran-ID används redan för ett annat utkast"
            );
            return Ok(request_id.into());
        }
        let entry: JournalEntry = read(&tx, "journal_entries", entry_id)?;
        ensure!(
            entry.description_revision == revision
                && entry.confirmed_revision.as_deref() == Some(revision),
            "Granska och bekräfta den aktuella beskrivningen först"
        );
        ensure!(
            !entry.description.trim().is_empty(),
            "Beskriv upplevelsen först"
        );
        let mut scenes = vec![];
        for key in &scene_ids {
            ensure_latest(&tx, key)?;
            let draft: SceneDraft = read(&tx, "journal_drafts", key)?;
            ensure!(
                draft.entry_id == entry_id
                    && draft.description_revision == revision
                    && draft.kind == "scene",
                "Välj scener från den aktuella beskrivningen"
            );
            ensure!(
                !scenes.iter().any(|s: &SceneDraft| s.id == *key),
                "En scen valdes flera gånger"
            );
            scenes.push(draft);
        }
        let job = DraftingJob {
            id: request_id.into(),
            entry_id: entry_id.into(),
            status: "queued".into(),
            error: None,
            description_revision: revision.into(),
            description: entry.description,
            language: entry.language,
            reflections: include_reflections.then_some(entry.reflections),
            scene_ids,
            scenes,
            settings: self.drafting_settings()?,
            created_at: now(),
            result_ids: vec![],
        };
        tx.execute(
            "INSERT INTO drafting_jobs VALUES(?1,?2,?3,?4)",
            params![job.id, entry_id, job.status, serde_json::to_string(&job)?],
        )?;
        tx.commit()?;
        Ok(job.id)
    }
    pub fn claim_drafting(&self) -> Result<Option<DraftingJob>> {
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let json: Option<String> = tx
            .query_row(
                "SELECT json FROM drafting_jobs WHERE status='queued' ORDER BY rowid LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let mut job: Option<DraftingJob> = json.map(|s| serde_json::from_str(&s)).transpose()?;
        if let Some(job) = &mut job {
            job.status = "running".into();
            tx.execute(
                "UPDATE drafting_jobs SET status=?1,json=?2 WHERE id=?3",
                params![job.status, serde_json::to_string(job)?, job.id],
            )?;
        }
        tx.commit()?;
        Ok(job)
    }
    pub fn finish_drafting(
        &self,
        mut job: DraftingJob,
        result: Result<Vec<SceneDraft>>,
    ) -> Result<()> {
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        match result {
            Ok(drafts) => {
                ensure!(
                    !drafts.is_empty() && drafts.len() <= 50,
                    "Språkmodellen returnerade inga eller för många scener"
                );
                for draft in drafts {
                    insert_draft(&tx, &draft)?;
                    job.result_ids.push(draft.id);
                }
                job.status = "completed".into();
                job.error = None;
            }
            Err(e) => {
                job.status = "failed".into();
                job.error = Some(format!("{e:#}"));
            }
        }
        tx.execute(
            "UPDATE drafting_jobs SET status=?1,json=?2 WHERE id=?3",
            params![job.status, serde_json::to_string(&job)?, job.id],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn recover_drafting(&self) -> Result<()> {
        let c = self.connect()?;
        for mut job in all::<DraftingJob>(&c, "drafting_jobs")? {
            if job.status == "running" {
                job.status = "failed".into();
                job.error = Some(
                    "Utkastet avbröts vid omstart. Försök igen eller skriv en prompt själv.".into(),
                );
                c.execute(
                    "UPDATE drafting_jobs SET status=?1,json=?2 WHERE id=?3",
                    params![job.status, serde_json::to_string(&job)?, job.id],
                )?;
            }
        }
        Ok(())
    }
    pub fn save_scene_draft(&self, edit: DraftEdit) -> Result<SceneDraft> {
        validate_scene(&edit.scene)?;
        ensure!(
            !edit.prompt.trim().is_empty()
                && edit.prompt.len() <= 100_000
                && edit.arrangement.len() <= 20_000
                && edit
                    .negative_prompt
                    .as_ref()
                    .is_none_or(|s| s.len() <= 100_000),
            "Ogiltig bildprompt"
        );
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_latest(&tx, &edit.id)?;
        let mut draft: SceneDraft = read(&tx, "journal_drafts", &edit.id)?;
        let revision: DescriptionRevision =
            read(&tx, "journal_descriptions", &draft.description_revision)?;
        draft.parent_id = Some(draft.id);
        draft.id = id();
        draft.scene = edit.scene;
        normalize_details(&mut draft.scene, &revision.description);
        draft.prompt = if edit.manual {
            edit.prompt
        } else {
            compose(&draft.scene)
        };
        ensure!(!draft.prompt.trim().is_empty(), "Bildprompten är tom");
        draft.manual = edit.manual;
        draft.negative_prompt = edit.negative_prompt;
        draft.arrangement = edit.arrangement;
        draft.approved = false;
        draft.created_at = now();
        insert_draft(&tx, &draft)?;
        tx.commit()?;
        Ok(draft)
    }
    pub fn manual_scene(&self, entry_id: &str, revision: &str, prompt: &str) -> Result<SceneDraft> {
        ensure!(
            !prompt.trim().is_empty() && prompt.len() <= 100_000,
            "Skriv en bildprompt (max 100 000 byte)"
        );
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let entry: JournalEntry = read(&tx, "journal_entries", entry_id)?;
        ensure!(
            entry.description_revision == revision,
            "Beskrivningen har ändrats"
        );
        let draft = SceneDraft {
            position: 0,
            batch_created_at: now(),
            id: id(),
            entry_id: entry_id.into(),
            description_revision: revision.into(),
            parent_id: None,
            kind: "scene".into(),
            scene: Scene {
                title: "Egen scen".into(),
                summary: String::new(),
                details: vec![],
                additions: vec![],
                questions: vec![],
            },
            prompt: prompt.into(),
            negative_prompt: None,
            manual: true,
            approved: false,
            scene_ids: vec![],
            arrangement: String::new(),
            provenance: serde_json::json!({"source":"manual"}),
            created_at: now(),
        };
        insert_draft(&tx, &draft)?;
        tx.commit()?;
        Ok(draft)
    }
    pub fn approve_scene(&self, key: &str) -> Result<()> {
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_latest(&tx, key)?;
        let mut draft: SceneDraft = read(&tx, "journal_drafts", key)?;
        let entry: JournalEntry = read(&tx, "journal_entries", &draft.entry_id)?;
        ensure!(
            entry.description_revision == draft.description_revision
                && entry.confirmed_revision.as_deref() == Some(&draft.description_revision),
            "Granska den aktuella beskrivningen först"
        );
        ensure!(!draft.prompt.trim().is_empty(), "Prompten är tom");
        draft.approved = true;
        tx.execute(
            "UPDATE journal_drafts SET json=?1 WHERE id=?2",
            params![serde_json::to_string(&draft)?, key],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn generate_journal_images(
        &self,
        request_id: &str,
        entry_id: &str,
        draft_ids: Vec<String>,
        workflow_id: &str,
    ) -> Result<Vec<String>> {
        ensure!(
            uuid::Uuid::parse_str(request_id).is_ok()
                && !draft_ids.is_empty()
                && draft_ids.len() <= 50,
            "Ogiltig bildbegäran"
        );
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(json) = tx
            .query_row(
                "SELECT json FROM journal_generations WHERE id=?1 AND entry_id=?2",
                params![request_id, entry_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            let saved: GenerationRecord = serde_json::from_str(&json)?;
            ensure!(
                saved.draft_ids == draft_ids && saved.workflow_id == workflow_id,
                "Begäran-ID används redan för en annan bildbegäran"
            );
            return Ok(saved.job_ids);
        }
        let request_drafts = draft_ids.clone();
        let entry: JournalEntry = read(&tx, "journal_entries", entry_id)?;
        let mut jobs = vec![];
        let mut unique = std::collections::HashSet::new();
        for key in draft_ids {
            ensure!(unique.insert(key.clone()), "En scen valdes flera gånger");
            ensure_latest(&tx, &key)?;
            let draft: SceneDraft = read(&tx, "journal_drafts", &key)?;
            ensure!(
                draft.entry_id == entry_id
                    && draft.description_revision == entry.description_revision
                    && draft.approved
                    && entry.confirmed_revision.as_deref() == Some(&draft.description_revision),
                "Granska och godkänn alla valda scener först"
            );
            let revision: DescriptionRevision =
                read(&tx, "journal_descriptions", &draft.description_revision)?;
            let mut job = self.build_image_job(
                ImageDraft {
                    prompt: draft.prompt,
                    negative_prompt: draft.negative_prompt,
                    source_text: revision.description,
                    recording_id: revision.recording_id,
                    run_id: revision.run_id,
                },
                Some(workflow_id),
            )?;
            job.entry_id = Some(entry_id.into());
            job.draft_revision_id = Some(key);
            tx.execute(
                "INSERT INTO image_jobs VALUES(?1,?2)",
                params![job.id, serde_json::to_string(&job)?],
            )?;
            jobs.push(job.id);
        }
        tx.execute(
            "INSERT INTO journal_generations VALUES(?1,?2,?3)",
            params![
                request_id,
                entry_id,
                serde_json::to_string(&GenerationRecord {
                    draft_ids: request_drafts,
                    workflow_id: workflow_id.into(),
                    job_ids: jobs.clone()
                })?
            ],
        )?;
        tx.commit()?;
        Ok(jobs)
    }
    pub fn favourite_image(&self, entry_id: &str, path: Option<String>) -> Result<JournalEntry> {
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut entry: JournalEntry = read(&tx, "journal_entries", entry_id)?;
        if let Some(path) = &path {
            ensure!(
                all::<crate::images::ImageJob>(&tx, "image_jobs")?
                    .iter()
                    .any(|j| j.entry_id.as_deref() == Some(entry_id)
                        && j.images.iter().any(|i| !i.deleted && &i.path == path)),
                "Bilden tillhör inte posten"
            );
        }
        entry.favourite_image = path;
        entry.version += 1;
        save_entry(&tx, &entry)?;
        tx.commit()?;
        Ok(entry)
    }
    pub(crate) fn save_deleted_image(
        &self,
        job: &crate::images::ImageJob,
        path: &str,
    ) -> Result<()> {
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if job.is_emptied() {
            tx.execute("DELETE FROM image_jobs WHERE id=?1", [&job.id])?;
        } else {
            tx.execute(
                "UPDATE image_jobs SET json=?1 WHERE id=?2",
                params![serde_json::to_string(job)?, job.id],
            )?;
        }
        for mut entry in all::<JournalEntry>(&tx, "journal_entries")? {
            if entry.favourite_image.as_deref() == Some(path) {
                entry.favourite_image = None;
                entry.version += 1;
                save_entry(&tx, &entry)?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn set_recording_language(&self, recording: &str, language: &str) -> Result<()> {
        ensure!(
            ["sv", "en"].contains(&language),
            "Välj svenska eller engelska"
        );
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let status: String = tx
            .query_row(
                "SELECT status FROM jobs WHERE recording_id=?1",
                [recording],
                |r| r.get(0),
            )
            .optional()?
            .context("Inspelningen saknas")?;
        ensure!(status != "running", "Vänta tills transkriberingen är klar");
        tx.execute("INSERT INTO recording_languages VALUES(?1,?2) ON CONFLICT(recording_id) DO UPDATE SET language=excluded.language", params![recording, language])?;
        tx.execute(
            "UPDATE jobs SET language=?1 WHERE recording_id=?2 AND status='queued'",
            params![language, recording],
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// Quotations are attribution aids, never a claim that a model understood the text correctly.
pub fn normalize_details(scene: &mut Scene, source: &str) {
    let mut kept = vec![];
    for mut d in scene.details.drain(..) {
        if d.quote
            .as_ref()
            .is_some_and(|q| !q.is_empty() && source.contains(q))
        {
            kept.push(d);
        } else {
            d.quote = None;
            scene.additions.push(d);
        }
    }
    scene.details = kept;
    for d in &mut scene.additions {
        d.quote = None;
    }
}

pub fn scene_from_output(
    job: &DraftingJob,
    mut scene: Scene,
    arrangement: String,
    provenance: Value,
) -> Result<SceneDraft> {
    validate_scene(&scene)?;
    normalize_details(&mut scene, &job.description);
    let prompt = compose(&scene);
    ensure!(
        !prompt.trim().is_empty() && prompt.len() <= 100_000 && arrangement.len() <= 20_000,
        "Språkmodellen gav ingen användbar prompt"
    );
    Ok(SceneDraft {
        position: 0,
        batch_created_at: job.created_at.clone(),
        id: id(),
        entry_id: job.entry_id.clone(),
        description_revision: job.description_revision.clone(),
        parent_id: None,
        kind: if job.scene_ids.is_empty() {
            "scene"
        } else {
            "composition"
        }
        .into(),
        scene,
        prompt,
        negative_prompt: None,
        manual: false,
        approved: false,
        scene_ids: job.scene_ids.clone(),
        arrangement,
        provenance,
        created_at: now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::images::{ComfySettings, GeneratedImage, SavedWorkflow};
    use serde_json::json;

    fn input(entry: Option<&JournalEntry>, description: &str) -> EntryInput {
        EntryInput {
            id: entry.map(|e| e.id.clone()),
            version: entry.map(|e| e.version),
            title: "Min dröm".into(),
            date: "2026-10-06".into(),
            kind: "dream".into(),
            language: "sv".into(),
            description: description.into(),
            reflections: "Bara min privata reflektion".into(),
            recording_id: None,
            run_id: None,
        }
    }
    fn fixture() -> Result<(tempfile::TempDir, Database, JournalEntry, String)> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path())?;
        let entry = db.save_journal_entry(input(None, "En skog som är både inne och ute."))?;
        let workflow = db.save_workflow(SavedWorkflow { id: String::new(), name: "Test".into(), workflow: json!({"6":{"class_type":"CLIPTextEncode","inputs":{"text":"initial"}},"9":{"class_type":"SaveImage","inputs":{}}}), node_id: "6".into(), input_name: "text".into(), negative_field: None })?;
        Ok((temp, db, entry, workflow))
    }
    fn scene(db: &Database, entry: &JournalEntry, prompt: &str) -> Result<SceneDraft> {
        db.manual_scene(&entry.id, &entry.description_revision, prompt)
    }

    #[test]
    fn descriptions_are_versioned_and_concurrent_edits_are_rejected() -> Result<()> {
        let (temp, db, entry, _) = fixture()?;
        let updated = db.save_journal_entry(input(Some(&entry), "En annan skog"))?;
        assert_ne!(updated.description_revision, entry.description_revision);
        assert!(db
            .save_journal_entry(input(Some(&entry), "Stale overwrite"))
            .is_err());
        let snapshot = Database::open(temp.path())?.journal_snapshot()?;
        assert_eq!(snapshot.descriptions.len(), 2);
        assert!(snapshot
            .descriptions
            .iter()
            .any(|r| r.description == entry.description));
        assert_eq!(snapshot.entries[0].reflections, entry.reflections);
        Ok(())
    }
    #[test]
    fn reflections_are_opt_in_and_drafting_inputs_are_snapshots() -> Result<()> {
        let (_temp, db, entry, _) = fixture()?;
        let first = id();
        db.request_drafting(
            &first,
            &entry.id,
            &entry.description_revision,
            false,
            vec![],
        )?;
        assert_eq!(
            db.request_drafting(
                &first,
                &entry.id,
                &entry.description_revision,
                false,
                vec![]
            )?,
            first
        );
        let job = db.claim_drafting()?.unwrap();
        assert!(job.reflections.is_none());
        let second = id();
        db.request_drafting(
            &second,
            &entry.id,
            &entry.description_revision,
            true,
            vec![],
        )?;
        let job2 = db.claim_drafting()?.unwrap();
        assert_eq!(job2.reflections, Some(entry.reflections.clone()));
        let updated = db.save_journal_entry(input(Some(&entry), "Ny text"))?;
        assert_ne!(updated.description, job.description);
        let output = scene_from_output(
            &job,
            Scene {
                title: "Skogen".into(),
                summary: "Omöjlig geometri".into(),
                details: vec![Detail {
                    text: "Skog".into(),
                    quote: Some("En skog".into()),
                    english: "A forest both indoors and outdoors".into(),
                }],
                additions: vec![],
                questions: vec![],
            },
            String::new(),
            json!({}),
        )?;
        db.finish_drafting(job, Ok(vec![output.clone()]))?;
        assert!(db.approve_scene(&output.id).is_err());
        assert!(db
            .journal_snapshot()?
            .drafts
            .iter()
            .any(|d| d.id == output.id));
        Ok(())
    }
    #[test]
    fn copied_transcripts_need_confirmation_and_language_changes_invalidate_it() -> Result<()> {
        let (_temp, db, entry, _) = fixture()?;
        let c = db.connect()?;
        c.execute(
            "INSERT INTO recordings VALUES('rec','name','/file','hash',1,NULL,'today',1)",
            [],
        )?;
        c.execute(
            "INSERT INTO transcription_runs VALUES('run','rec','today','{}')",
            [],
        )?;
        let mut source = input(Some(&entry), "Text från transkript");
        source.recording_id = Some("rec".into());
        source.run_id = Some("run".into());
        let unconfirmed = db.save_journal_entry(source)?;
        assert!(db
            .request_drafting(
                &id(),
                &unconfirmed.id,
                &unconfirmed.description_revision,
                false,
                vec![]
            )
            .is_err());
        let confirmed =
            db.confirm_description(&unconfirmed.id, &unconfirmed.description_revision)?;
        db.request_drafting(
            &id(),
            &confirmed.id,
            &confirmed.description_revision,
            false,
            vec![],
        )?;
        let mut edit = input(Some(&confirmed), &confirmed.description);
        edit.recording_id = confirmed.recording_id;
        edit.run_id = confirmed.run_id;
        edit.language = "en".into();
        let changed = db.save_journal_entry(edit)?;
        assert!(changed.confirmed_revision.is_none());
        assert!(db
            .confirm_description(&changed.id, &entry.description_revision)
            .is_err());
        Ok(())
    }
    #[test]
    fn generation_is_approval_bound_atomic_and_idempotent() -> Result<()> {
        let (_temp, db, entry, workflow) = fixture()?;
        let one = scene(&db, &entry, "A forest")?;
        let two = scene(&db, &entry, "A house")?;
        db.approve_scene(&one.id)?;
        assert!(db
            .generate_journal_images(
                &id(),
                &entry.id,
                vec![one.id.clone(), two.id.clone()],
                &workflow
            )
            .is_err());
        assert!(db.image_jobs()?.is_empty());
        db.approve_scene(&two.id)?;
        let request = id();
        let keys = vec![one.id.clone(), two.id.clone()];
        let jobs = db.generate_journal_images(&request, &entry.id, keys.clone(), &workflow)?;
        assert_eq!(
            db.generate_journal_images(&request, &entry.id, keys, &workflow)?,
            jobs
        );
        assert_eq!(db.image_jobs()?.len(), 2);
        assert!(db
            .image_jobs()?
            .iter()
            .all(|j| j.entry_id.as_deref() == Some(&entry.id)
                && j.draft.source_text == entry.description
                && j.draft_revision_id.is_some()));
        let revised = db.save_scene_draft(DraftEdit {
            id: one.id.clone(),
            scene: one.scene.clone(),
            prompt: "A changed forest".into(),
            negative_prompt: None,
            manual: true,
            arrangement: String::new(),
        })?;
        assert!(!revised.approved);
        assert!(db.approve_scene(&one.id).is_err());
        assert!(db
            .generate_journal_images(&id(), &entry.id, vec![one.id], &workflow)
            .is_err());
        assert!(db
            .generate_journal_images(&id(), &entry.id, vec![revised.id], &workflow)
            .is_err());
        assert_eq!(
            db.image_jobs()?
                .iter()
                .find(|j| j.draft.prompt == "A forest")
                .unwrap()
                .draft
                .prompt,
            "A forest"
        );
        Ok(())
    }
    #[test]
    fn combinations_keep_source_scene_revisions_and_reject_other_entries() -> Result<()> {
        let (_temp, db, entry, _) = fixture()?;
        let one = scene(&db, &entry, "Forest")?;
        let two = scene(&db, &entry, "House")?;
        let other = db.save_journal_entry(input(None, "Annan dröm"))?;
        let foreign = scene(&db, &other, "City")?;
        assert!(db
            .request_drafting(
                &id(),
                &entry.id,
                &entry.description_revision,
                false,
                vec![one.id.clone(), foreign.id]
            )
            .is_err());
        db.request_drafting(
            &id(),
            &entry.id,
            &entry.description_revision,
            false,
            vec![one.id.clone(), two.id.clone()],
        )?;
        let job = db.claim_drafting()?.unwrap();
        assert_eq!(job.scenes.len(), 2);
        assert_eq!(job.scenes[0].prompt, "Forest");
        assert_eq!(db.journal_snapshot()?.drafts.len(), 3);
        Ok(())
    }
    #[test]
    fn deletion_clears_only_the_matching_favourite() -> Result<()> {
        let (temp, db, entry, workflow) = fixture()?;
        let draft = scene(&db, &entry, "Forest")?;
        db.approve_scene(&draft.id)?;
        let jobs = db.generate_journal_images(&id(), &entry.id, vec![draft.id], &workflow)?;
        let mut job = db
            .image_jobs()?
            .into_iter()
            .find(|j| j.id == jobs[0])
            .unwrap();
        let directory = temp.path().join("images").join(&job.id);
        std::fs::create_dir_all(&directory)?;
        let path = directory.join("image.png");
        std::fs::write(&path, b"image")?;
        job.images.push(GeneratedImage {
            deleted: false,
            created_at: None,
            path: path.to_string_lossy().into(),
            node_id: "9".into(),
            filename: "image.png".into(),
            subfolder: String::new(),
            image_type: "output".into(),
        });
        db.save_image_job(&job)?;
        db.favourite_image(&entry.id, Some(path.to_string_lossy().into()))?;
        assert!(db
            .favourite_image(&entry.id, Some("/foreign".into()))
            .is_err());
        db.delete_generated_image(temp.path(), &job.id, &path.to_string_lossy())?;
        assert!(db.journal_entry(&entry.id)?.favourite_image.is_none());
        assert!(!path.exists());
        Ok(())
    }
    #[test]
    fn interrupted_drafting_is_recoverable_without_overwriting_history() -> Result<()> {
        let (temp, db, entry, _) = fixture()?;
        scene(&db, &entry, "Existing prompt")?;
        db.request_drafting(&id(), &entry.id, &entry.description_revision, false, vec![])?;
        db.claim_drafting()?;
        let reopened = Database::open(temp.path())?;
        let snapshot = reopened.journal_snapshot()?;
        assert_eq!(snapshot.jobs[0].status, "failed");
        assert!(snapshot.jobs[0].error.is_some());
        assert_eq!(snapshot.drafts[0].prompt, "Existing prompt");
        Ok(())
    }
    #[test]
    fn attribution_and_removed_additions_change_the_compiled_prompt() {
        let mut scene = Scene {
            title: "Skog".into(),
            summary: "".into(),
            details: vec![
                Detail {
                    text: "Skog".into(),
                    quote: Some("skog".into()),
                    english: "A forest".into(),
                },
                Detail {
                    text: "Sjö".into(),
                    quote: Some("fake quote".into()),
                    english: "A lake".into(),
                },
            ],
            additions: vec![],
            questions: vec![],
        };
        normalize_details(&mut scene, "En skog");
        assert_eq!(scene.details.len(), 1);
        assert_eq!(scene.additions.len(), 1);
        assert!(scene.additions[0].quote.is_none());
        assert_eq!(compose(&scene), "A forest. A lake");
        scene.additions.clear();
        assert_eq!(compose(&scene), "A forest");
    }
    #[test]
    fn version_three_and_legacy_json_upgrade_without_losing_content() -> Result<()> {
        let (temp, db, entry, _) = fixture()?;
        db.connect()?.execute_batch("ALTER TABLE jobs DROP COLUMN language; DROP TABLE recording_languages; PRAGMA user_version=3")?;
        let reopened = Database::open(temp.path())?;
        assert_eq!(
            reopened.journal_entry(&entry.id)?.description,
            entry.description
        );
        let mut legacy = serde_json::to_value(ComfySettings::default())?;
        legacy.as_object_mut().unwrap().remove("negative_field");
        let _: ComfySettings = serde_json::from_value(legacy)?;
        assert_eq!(
            reopened
                .connect()?
                .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))?,
            4
        );
        Ok(())
    }
    #[test]
    fn recording_language_is_captured_at_queue_time() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path())?;
        let audio = temp.path().join("audio.wav");
        std::fs::write(&audio, b"audio")?;
        db.connect()?.execute(
            "INSERT INTO recordings VALUES('rec','name',?1,'hash',1,NULL,'today',1)",
            [audio.to_string_lossy().to_string()],
        )?;
        db.connect()?.execute(
            "INSERT INTO jobs(id,recording_id,status) VALUES('job','rec','queued')",
            [],
        )?;
        db.set_recording_language("rec", "en")?;
        assert_eq!(db.recordings()?[0].language, "en");
        let job = db.claim()?.unwrap();
        assert_eq!(job.language, "en");
        assert!(db.set_recording_language("rec", "sv").is_err());
        db.failed(&job.id, "Test")?;
        db.set_recording_language("rec", "sv")?;
        db.retry("rec")?;
        assert_eq!(db.claim()?.unwrap().language, "sv");
        Ok(())
    }
}
