//! Persistent, single-GPU ComfyUI jobs. Called only by the worker supervisor.
use crate::db::Database;
use anyhow::{bail, Context, Result};
use reqwest::blocking::{Client, Response};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComfySettings {
    pub url: String,
    pub workflow: Value,
    pub workflow_name: String,
    pub node_id: String,
    pub input_name: String,
    #[serde(default)]
    pub negative_field: Option<PromptField>,
    #[serde(default)]
    pub selected_workflow_id: Option<String>,
    #[serde(default = "default_comfy_directory")]
    pub comfy_directory: String,
    #[serde(default = "default_comfy_python")]
    pub comfy_python_path: String,
}
impl Default for ComfySettings {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:8188".into(),
            workflow: json!({}),
            workflow_name: String::new(),
            node_id: String::new(),
            input_name: String::new(),
            negative_field: None,
            selected_workflow_id: None,
            comfy_directory: default_comfy_directory(),
            comfy_python_path: default_comfy_python(),
        }
    }
}
fn default_comfy_directory() -> String {
    let path = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join("comfy/ComfyUI");
    if path.join("main.py").is_file() {
        path.to_string_lossy().into()
    } else {
        String::new()
    }
}
fn default_comfy_python() -> String {
    let path = std::path::PathBuf::from(default_comfy_directory()).join(".venv/bin/python");
    if path.is_file() {
        path.to_string_lossy().into()
    } else {
        String::new()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptField {
    pub node_id: String,
    pub input_name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedWorkflow {
    pub id: String,
    pub name: String,
    pub workflow: Value,
    pub node_id: String,
    pub input_name: String,
    #[serde(default)]
    pub negative_field: Option<PromptField>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComfyConnection {
    pub url: String,
    pub comfy_directory: String,
    pub comfy_python_path: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ImageDraft {
    pub prompt: String,
    #[serde(default)]
    pub negative_prompt: Option<String>,
    pub source_text: String,
    pub recording_id: Option<String>,
    pub run_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeneratedImage {
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub created_at: Option<String>,
    pub path: String,
    pub node_id: String,
    pub filename: String,
    pub subfolder: String,
    pub image_type: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageJob {
    #[serde(default)]
    pub entry_id: Option<String>,
    #[serde(default)]
    pub draft_revision_id: Option<String>,
    pub id: String,
    pub created_at: String,
    pub draft: ImageDraft,
    pub config: ComfySettings,
    pub status: String,
    pub prompt_id: Option<String>,
    pub error: Option<String>,
    pub images: Vec<GeneratedImage>,
    pub release_pending: bool,
}
#[derive(Serialize)]
pub struct ImageSnapshot {
    pub workflows: Vec<SavedWorkflow>,
    pub settings: ComfySettings,
    pub draft: ImageDraft,
    pub jobs: Vec<ImageJob>,
}

impl Database {
    pub fn delete_generated_image(&self, root: &Path, id: &str, path: &str) -> Result<()> {
        let mut job = self
            .image_jobs()?
            .into_iter()
            .find(|j| j.id == id)
            .context("Bildjobbet finns inte")?;
        let image = job
            .images
            .iter_mut()
            .find(|i| i.path == path)
            .context("Bilden finns inte i bildjobbet")?;
        let file = std::path::PathBuf::from(&image.path);
        let directory = root.join("images").join(&job.id);
        anyhow::ensure!(
            file.parent() == Some(directory.as_path()),
            "Ogiltig bildsökväg"
        );
        if file.exists() {
            anyhow::ensure!(
                file.canonicalize()?
                    .starts_with(root.join("images").canonicalize()?),
                "Bilden ligger utanför bildarkivet"
            );
        }
        // Save the tombstone first so a crash or later follow-up cannot restore the image.
        image.deleted = true;
        self.save_deleted_image(&job, path)?;
        match std::fs::remove_file(&file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn comfy_settings(&self) -> Result<ComfySettings> {
        read_config(&self.connect()?)
    }
    pub fn save_comfy_settings(&self, config: &ComfySettings) -> Result<()> {
        validate_url(&config.url)?;
        if !config.workflow.as_object().is_some_and(|w| w.is_empty()) {
            validate_workflow(&config.workflow)?;
            build_prompt(config, "test")?;
        }
        write_config(&self.connect()?, config)
    }
    pub fn saved_workflows(&self) -> Result<Vec<SavedWorkflow>> {
        let c = self.connect()?;
        let mut q = c.prepare("SELECT json FROM image_workflows ORDER BY rowid")?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }
    pub fn save_workflow(&self, mut workflow: SavedWorkflow) -> Result<String> {
        workflow.name = workflow.name.trim().into();
        if workflow.name.is_empty() || workflow.name.chars().count() > 120 {
            bail!("Ange ett flödesnamn med 1–120 tecken.");
        }
        if serde_json::to_vec(&workflow.workflow)?.len() > 5 * 1024 * 1024 {
            bail!("Workflow är större än 5 MB.");
        }
        let config = ComfySettings {
            workflow: workflow.workflow.clone(),
            node_id: workflow.node_id.clone(),
            input_name: workflow.input_name.clone(),
            negative_field: workflow.negative_field.clone(),
            ..Default::default()
        };
        build_prompt(&config, "test")?;
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if workflow.id.is_empty() {
            workflow.id = uuid::Uuid::new_v4().to_string();
        } else if !workflow_exists(&tx, &workflow.id)? {
            bail!("Flödet finns inte längre.");
        }
        tx.execute("INSERT INTO image_workflows VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json", rusqlite::params![workflow.id,serde_json::to_string(&workflow)?])?;
        tx.commit()?;
        Ok(workflow.id)
    }
    pub fn delete_workflow(&self, id: &str) -> Result<()> {
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ids = tx
            .prepare("SELECT id FROM image_workflows ORDER BY rowid")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let position = ids
            .iter()
            .position(|w| w == id)
            .context("Flödet finns inte längre.")?;
        tx.execute("DELETE FROM image_workflows WHERE id=?1", [id])?;
        let config = read_config(&tx)?;
        if config.selected_workflow_id.as_deref() == Some(id) {
            // Keep only the connection: the old singleton workflow must not be
            // migrated back. The following flow, else the preceding one, takes over.
            let next = ids
                .get(position + 1)
                .or_else(|| ids.get(position.checked_sub(1)?));
            let config = ComfySettings {
                url: config.url,
                comfy_directory: config.comfy_directory,
                comfy_python_path: config.comfy_python_path,
                selected_workflow_id: next.cloned(),
                ..Default::default()
            };
            write_config(&tx, &config)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn select_workflow(&self, id: &str) -> Result<()> {
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        select_workflow_in(&tx, id)?;
        tx.commit()?;
        Ok(())
    }
    pub fn save_comfy_connection(&self, connection: ComfyConnection) -> Result<()> {
        let mut config = self.comfy_settings()?;
        config.url = connection.url.trim().into();
        config.comfy_directory = connection.comfy_directory.trim().into();
        config.comfy_python_path = connection.comfy_python_path.trim().into();
        self.save_comfy_settings(&config)
    }
    pub fn migrate_legacy_workflow(&self) -> Result<()> {
        let mut config = self.comfy_settings()?;
        if config.selected_workflow_id.is_some() || build_prompt(&config, "test").is_err() {
            return Ok(());
        }
        let workflow = SavedWorkflow {
            id: "legacy".into(),
            name: if config.workflow_name.trim().is_empty() {
                "Tidigare flöde".into()
            } else {
                config.workflow_name.clone()
            },
            workflow: config.workflow.clone(),
            node_id: config.node_id.clone(),
            input_name: config.input_name.clone(),
            negative_field: config.negative_field.clone(),
        };
        let mut c = self.connect()?;
        let tx = c.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO image_workflows VALUES(?1,?2)",
            rusqlite::params![workflow.id, serde_json::to_string(&workflow)?],
        )?;
        config.selected_workflow_id = Some(workflow.id);
        write_config(&tx, &config)?;
        tx.commit()?;
        Ok(())
    }
    pub fn image_draft(&self) -> Result<ImageDraft> {
        let value: Option<String> = self
            .connect()?
            .query_row("SELECT json FROM image_draft WHERE id=1", [], |r| r.get(0))
            .optional()?;
        Ok(value
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default())
    }
    pub fn save_image_draft(&self, draft: &ImageDraft) -> Result<()> {
        self.connect()?.execute(
            "INSERT INTO image_draft VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            [serde_json::to_string(draft)?],
        )?;
        Ok(())
    }
    pub fn image_jobs(&self) -> Result<Vec<ImageJob>> {
        let c = self.connect()?;
        let mut query = c.prepare("SELECT json FROM image_jobs ORDER BY rowid DESC")?;
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }
    pub fn save_image_job(&self, job: &ImageJob) -> Result<()> {
        self.connect()?.execute(
            "INSERT INTO image_jobs VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            rusqlite::params![job.id, serde_json::to_string(job)?],
        )?;
        Ok(())
    }
    pub fn enqueue_image(&self, draft: ImageDraft) -> Result<String> {
        self.enqueue_image_with_workflow(draft, None)
    }
    pub fn enqueue_image_with_workflow(
        &self,
        draft: ImageDraft,
        workflow_id: Option<&str>,
    ) -> Result<String> {
        let job = self.build_image_job(draft, workflow_id)?;
        self.connect()?.execute(
            "INSERT INTO image_jobs VALUES(?1,?2)",
            rusqlite::params![job.id, serde_json::to_string(&job)?],
        )?;
        Ok(job.id)
    }
    pub(crate) fn build_image_job(
        &self,
        mut draft: ImageDraft,
        workflow_id: Option<&str>,
    ) -> Result<ImageJob> {
        if draft.prompt.trim().is_empty() {
            bail!("Skriv en bildprompt först.");
        }
        if draft.prompt.len() > 100_000 {
            bail!("Bildprompten är för lång (max 100 000 byte).");
        }
        if draft
            .negative_prompt
            .as_ref()
            .is_some_and(|text| text.len() > 100_000)
        {
            bail!("Negativ prompt är för lång (max 100 000 byte).");
        }
        let mut config = self.comfy_settings()?;
        validate_url(&config.url)?;
        if let Some(id) = workflow_id.or(config.selected_workflow_id.as_deref()) {
            let workflow = self
                .saved_workflows()?
                .into_iter()
                .find(|w| w.id == id)
                .context("Det valda flödet finns inte längre.")?;
            config.selected_workflow_id = Some(workflow.id);
            config.workflow = workflow.workflow;
            config.workflow_name = workflow.name;
            config.node_id = workflow.node_id;
            config.input_name = workflow.input_name;
            config.negative_field = workflow.negative_field;
        }
        config.workflow =
            build_prompt_with_negative(&config, &draft.prompt, draft.negative_prompt.as_deref())?;
        draft.negative_prompt = config.negative_field.as_ref().and_then(|field| {
            config
                .workflow
                .get(&field.node_id)?
                .get("inputs")?
                .get(&field.input_name)?
                .as_str()
                .map(str::to_owned)
        });
        // Resolve source references before committing a durable job.
        if let Some(id) = &draft.recording_id {
            let exists: bool = self.connect()?.query_row(
                "SELECT EXISTS(SELECT 1 FROM recordings WHERE id=?1)",
                [id],
                |r| r.get(0),
            )?;
            if !exists {
                bail!("Källinspelningen finns inte.");
            }
            let transcript = self.transcript(id, draft.run_id.as_deref())?;
            if draft.run_id.is_some() && transcript.run_id != draft.run_id {
                bail!("Transkriptversionen finns inte.");
            }
        }
        let job = ImageJob {
            entry_id: None,
            draft_revision_id: None,
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            draft,
            config,
            status: "queued".into(),
            prompt_id: None,
            error: None,
            images: vec![],
            release_pending: false,
        };
        Ok(job)
    }
    pub fn image_snapshot(&self) -> Result<ImageSnapshot> {
        Ok(ImageSnapshot {
            workflows: self.saved_workflows()?,
            settings: self.comfy_settings()?,
            draft: self.image_draft()?,
            jobs: self.image_jobs()?,
        })
    }
    pub fn next_image_job(&self) -> Result<Option<ImageJob>> {
        // Finish interrupted/accepted submissions before taking another queued job.
        let jobs = self.image_jobs()?;
        Ok(jobs
            .iter()
            .rev()
            .find(|j| j.status != "queued" && (j.release_pending || !terminal(&j.status)))
            .cloned()
            .or_else(|| jobs.into_iter().rev().find(|j| j.status == "queued")))
    }
    pub fn dismiss_image_job(&self, id: &str) -> Result<()> {
        let mut job = self
            .image_jobs()?
            .into_iter()
            .find(|j| j.id == id)
            .context("Bildjobbet saknas")?;
        if job.status == "queued" || (!terminal(&job.status) && job.error.is_none()) {
            bail!("Detta jobb kan inte avslutas manuellt just nu.");
        }
        if !terminal(&job.status) {
            job.status = "uncertain".into();
        }
        job.release_pending = false;
        job.error = Some("Uppföljningen avslutades manuellt. Ingen begäran om att avbryta jobbet skickades till ComfyUI.".into());
        self.save_image_job(&job)
    }
    pub fn retry_image_download(&self, id: &str) -> Result<()> {
        let mut job = self
            .image_jobs()?
            .into_iter()
            .find(|j| j.id == id)
            .context("Bildjobbet saknas")?;
        if job.prompt_id.is_none() || !terminal(&job.status) {
            bail!("Jobbet kan inte följas upp just nu.");
        }
        job.status = "queued_comfy".into();
        job.release_pending = true;
        job.error = None;
        self.save_image_job(&job)
    }
}
fn terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "uncertain")
}

pub fn validate_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value.trim()).context("Ogiltig ComfyUI-adress")?;
    if url.scheme() != "http"
        || !matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        bail!("Ange en lokal HTTP-adress, exempelvis http://127.0.0.1:8188.");
    }
    Ok(())
}
pub fn validate_workflow(workflow: &Value) -> Result<()> {
    let nodes = workflow
        .as_object()
        .context("Workflow måste vara ett JSON-objekt i API-format.")?;
    if nodes.is_empty()
        || nodes
            .values()
            .any(|v| !v["class_type"].is_string() || !v["inputs"].is_object())
    {
        bail!("Workflow är inte i API-format. Exportera med Save (API Format) / Export (API) i ComfyUI.");
    }
    Ok(())
}
pub fn build_prompt(config: &ComfySettings, text: &str) -> Result<Value> {
    build_prompt_with_negative(config, text, None)
}
pub fn build_prompt_with_negative(
    config: &ComfySettings,
    text: &str,
    negative: Option<&str>,
) -> Result<Value> {
    validate_workflow(&config.workflow)?;
    let mut workflow = config.workflow.clone();
    let field = workflow
        .get_mut(&config.node_id)
        .and_then(|n| n.get_mut("inputs"))
        .and_then(|n| n.get_mut(&config.input_name))
        .context("Välj nod och textfält för positiv prompt.")?;
    if !field.is_string() {
        bail!("Det positiva promptfältet måste innehålla text, inte en nodkoppling.");
    }
    *field = Value::String(text.into());
    if let Some(mapping) = &config.negative_field {
        if mapping.node_id == config.node_id && mapping.input_name == config.input_name {
            bail!("Positiv och negativ prompt måste använda olika textfält.");
        }
        let field = workflow
            .get_mut(&mapping.node_id)
            .and_then(|n| n.get_mut("inputs"))
            .and_then(|n| n.get_mut(&mapping.input_name))
            .context("Det negativa promptfältet finns inte i flödet.")?;
        if !field.is_string() {
            bail!("Det negativa promptfältet måste innehålla text, inte en nodkoppling.");
        }
        if let Some(text) = negative {
            *field = Value::String(text.into());
        }
    }
    Ok(workflow)
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()?)
}
fn endpoint(config: &ComfySettings, suffix: &str) -> String {
    format!("{}{}", config.url.trim().trim_end_matches('/'), suffix)
}
fn response_json(response: Response) -> Result<Value> {
    let status = response.status();
    let mut text = String::new();
    response.take(2 * 1024 * 1024).read_to_string(&mut text)?;
    if !status.is_success() {
        bail!("ComfyUI HTTP {status}: {text}");
    }
    serde_json::from_str(&text).context("ComfyUI svarade inte med giltig JSON")
}
pub fn test_connection(url: &str) -> Result<()> {
    validate_url(url)?;
    let config = ComfySettings {
        url: url.into(),
        ..Default::default()
    };
    response_json(
        client()?
            .get(endpoint(&config, "/system_stats"))
            .timeout(Duration::from_secs(3))
            .send()
            .context("ComfyUI kunde inte nås. Starta servern och kontrollera adressen.")?,
    )?;
    Ok(())
}
fn queue_contains(queue: &Value, key: &str, id: &str) -> bool {
    queue[key]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item[1].as_str() == Some(id)))
}
fn queue_empty(queue: &Value) -> bool {
    ["queue_running", "queue_pending"]
        .iter()
        .all(|key| queue[*key].as_array().is_some_and(Vec::is_empty))
}

/// Before loading another local engine, release a reachable, idle ComfyUI instance.
/// Refuse an occupied or unverifiable queue; never interrupt another client's work.
pub fn release_idle_comfy(db: &Database) -> Result<()> {
    let config = db.comfy_settings()?;
    validate_url(&config.url)?;
    let http = Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .build()?;
    let response = match http.get(endpoint(&config, "/queue")).send() {
        Ok(response) => response,
        Err(error) if error.is_connect() => return Ok(()),
        Err(_) => bail!("ComfyUI:s kö kunde inte kontrolleras. Försök igen när servern svarar."),
    };
    let queue = response_json(response)?;
    anyhow::ensure!(
        queue_empty(&queue),
        "ComfyUI använder GPU:n. Vänta tills dess kö är tom och försök igen."
    );
    http.post(endpoint(&config, "/free"))
        .json(&json!({"unload_models":true,"free_memory":true}))
        .send()?
        .error_for_status()?;
    std::thread::sleep(Duration::from_secs(2));
    Ok(())
}

/// One short supervisor tick. Never resubmits a job after a possibly accepted POST.
pub fn step(db: &Database, root: &Path, mut job: ImageJob) -> Result<()> {
    let http = client()?;
    if terminal(&job.status) {
        if job.release_pending {
            let queue = response_json(http.get(endpoint(&job.config, "/queue")).send()?)?;
            if !queue_empty(&queue) {
                return Ok(());
            }
            http.post(endpoint(&job.config, "/free"))
                .json(&json!({"unload_models": true, "free_memory": true}))
                .send()?
                .error_for_status()?;
            // /free wakes the ComfyUI supervisor asynchronously. Give it time to process flags.
            std::thread::sleep(Duration::from_secs(2));
            job.release_pending = false;
            db.save_image_job(&job)?;
        }
        return Ok(());
    }
    if job.status == "queued" {
        job.status = "submitting".into();
        // Current local ComfyUI accepts caller-generated UUIDs. Persist before POST so
        // even an interrupted response can be looked up without creating a duplicate.
        job.prompt_id = Some(job.id.clone());
        job.release_pending = true;
        db.save_image_job(&job)?;
        let response = match http
            .post(endpoint(&job.config, "/prompt"))
            .json(&json!({"prompt": job.config.workflow, "prompt_id": job.id, "client_id": job.id}))
            .send()
        {
            Ok(response) => response,
            Err(e) => {
                job.error = Some(format!("Inskickningen kunde inte bekräftas: {e}. Jobbet följs upp utan att skickas igen."));
                return db.save_image_job(&job);
            }
        };
        let status = response.status();
        let body = match response_json(response) {
            Ok(body) => body,
            Err(e) => {
                job.status = if status.is_client_error() {
                    "failed"
                } else {
                    "submitting"
                }
                .into();
                job.error = Some(format!("{e:#}"));
                return db.save_image_job(&job);
            }
        };
        if let Some(id) = body["prompt_id"].as_str() {
            job.prompt_id = Some(id.into());
        } else {
            job.error = Some(format!("ComfyUI bekräftade inte jobbet: {body}"));
            return db.save_image_job(&job);
        }
        job.status = "queued_comfy".into();
        job.error = None;
        return db.save_image_job(&job);
    }
    let id = job
        .prompt_id
        .as_deref()
        .context("Bildjobbet saknar server-ID")?;
    let history = response_json(
        http.get(endpoint(&job.config, &format!("/history/{id}")))
            .send()?,
    )?;
    if let Some(record) = history.get(id) {
        if record["status"]["status_str"] == "error" {
            job.status = "failed".into();
            job.error = Some(format!(
                "ComfyUI kunde inte skapa bilden: {}",
                record["status"]["messages"]
            ));
            return db.save_image_job(&job);
        }
        if record["status"]["completed"] == true {
            job.status = "downloading".into();
            job.error = None;
            db.save_image_job(&job)?;
            let result = download_images(&http, root, &mut job, &record["outputs"], db);
            match result {
                Ok(()) if !job.images.is_empty() => {
                    job.status = "completed".into();
                    job.error = None;
                }
                Ok(()) => {
                    job.status = "failed".into();
                    job.error = Some("Workflow slutfördes utan bildresultat. Lägg till SaveImage eller PreviewImage i ComfyUI.".into());
                }
                Err(e) => {
                    job.status = "failed".into();
                    job.error = Some(format!("Bilderna kunde inte hämtas: {e:#}. Välj Följ upp för att hämta igen utan ny bildkörning."));
                }
            }
            return db.save_image_job(&job);
        }
    }
    let queue = response_json(http.get(endpoint(&job.config, "/queue")).send()?)?;
    // Check running first: a prompt can move between history and queue requests.
    if queue_contains(&queue, "queue_running", id) {
        job.status = "running".into();
        job.error = None;
    } else if queue_contains(&queue, "queue_pending", id) {
        job.status = "queued_comfy".into();
        job.error = None;
    } else {
        // Recheck history to avoid classifying a just-completed job as missing.
        let latest = response_json(
            http.get(endpoint(&job.config, &format!("/history/{id}")))
                .send()?,
        )?;
        if latest.get(id).is_some() {
            return Ok(());
        }
        job.status = "uncertain".into();
        job.error = Some("Jobbet finns inte i ComfyUI:s kö eller historik. Det skickas inte automatiskt igen. Kontrollera ComfyUI och välj Följ upp eller skapa ett nytt jobb.".into());
    }
    db.save_image_job(&job)
}
fn read_config(c: &Connection) -> Result<ComfySettings> {
    let value: Option<String> = c
        .query_row("SELECT json FROM image_config WHERE id=1", [], |r| r.get(0))
        .optional()?;
    Ok(value
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default())
}
fn write_config(c: &Connection, config: &ComfySettings) -> Result<()> {
    c.execute(
        "INSERT INTO image_config VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
        [serde_json::to_string(config)?],
    )?;
    Ok(())
}
fn workflow_exists(c: &Connection, id: &str) -> Result<bool> {
    Ok(c.query_row(
        "SELECT 1 FROM image_workflows WHERE id=?1",
        [id],
        |_| Ok(()),
    )
    .optional()?
    .is_some())
}
fn select_workflow_in(c: &Connection, id: &str) -> Result<()> {
    if !workflow_exists(c, id)? {
        bail!("Flödet saknas.");
    }
    let mut config = read_config(c)?;
    config.selected_workflow_id = Some(id.into());
    write_config(c, &config)
}
fn download_images(
    http: &Client,
    root: &Path,
    job: &mut ImageJob,
    outputs: &Value,
    db: &Database,
) -> Result<()> {
    let directory = root.join("images").join(&job.id);
    std::fs::create_dir_all(&directory)?;
    for (node_id, output) in outputs.as_object().context("Ogiltiga bildresultat")? {
        for image in output["images"].as_array().into_iter().flatten() {
            let filename = image["filename"]
                .as_str()
                .context("Bildresultatet saknar filnamn")?;
            let subfolder = image["subfolder"].as_str().unwrap_or("");
            let image_type = image["type"].as_str().unwrap_or("output");
            if job.images.iter().any(|i| {
                i.node_id == *node_id
                    && i.filename == filename
                    && i.subfolder == subfolder
                    && i.image_type == image_type
                    && (i.deleted || Path::new(&i.path).is_file())
            }) {
                continue;
            }
            let extension = match Path::new(filename)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase()
                .as_str()
            {
                "png" => "png",
                "jpg" | "jpeg" => "jpg",
                "webp" => "webp",
                _ => bail!("Bildformatet stöds inte: {filename}"),
            };
            let path = directory.join(format!("{}.{}", uuid::Uuid::new_v4(), extension));
            let temporary = path.with_extension("part");
            let mut response = http
                .get(endpoint(&job.config, "/view"))
                .query(&[
                    ("filename", filename),
                    ("subfolder", subfolder),
                    ("type", image_type),
                ])
                .send()?
                .error_for_status()?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            let result = (|| -> Result<()> {
                let size =
                    std::io::copy(&mut (&mut response).take(64 * 1024 * 1024 + 1), &mut file)?;
                if size == 0 || size > 64 * 1024 * 1024 {
                    bail!("Bildfilen är tom eller större än 64 MB.");
                }
                file.flush()?;
                file.sync_all()?;
                std::fs::rename(&temporary, &path)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&temporary);
            }
            result?;
            job.images.push(GeneratedImage {
                deleted: false,
                created_at: Some(chrono::Utc::now().to_rfc3339()),
                path: path.to_string_lossy().into(),
                node_id: node_id.clone(),
                filename: filename.into(),
                subfolder: subfolder.into(),
                image_type: image_type.into(),
            });
            db.save_image_job(job)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::BufRead, net::TcpListener, thread, time::Instant};

    struct MockServer {
        url: String,
        thread: thread::JoinHandle<Result<()>>,
    }
    impl MockServer {
        fn start(responses: Vec<(String, u16, Vec<u8>)>) -> Result<Self> {
            Self::start_with_guard(responses, None)
        }
        fn start_with_guard(
            responses: Vec<(String, u16, Vec<u8>)>,
            whisper_pid: Option<std::path::PathBuf>,
        ) -> Result<Self> {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            listener.set_nonblocking(true)?;
            let url = format!("http://{}", listener.local_addr()?);
            let thread = thread::spawn(move || -> Result<()> {
                for (expected, status, body) in responses {
                    let deadline = Instant::now() + Duration::from_secs(10);
                    let (mut stream, _) = loop {
                        match listener.accept() {
                            Ok(connection) => break connection,
                            Err(e)
                                if e.kind() == std::io::ErrorKind::WouldBlock
                                    && Instant::now() < deadline =>
                            {
                                thread::sleep(Duration::from_millis(5))
                            }
                            Err(e) => return Err(e.into()),
                        }
                    };
                    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                    let mut reader = std::io::BufReader::new(stream.try_clone()?);
                    let mut line = String::new();
                    reader.read_line(&mut line)?;
                    assert!(
                        line.starts_with(&expected),
                        "expected {expected}, got {line}"
                    );
                    let mut length = 0;
                    loop {
                        line.clear();
                        reader.read_line(&mut line)?;
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                            length = value.trim().parse()?;
                        }
                    }
                    let mut request_body = vec![0; length];
                    reader.read_exact(&mut request_body)?;
                    if expected.starts_with("POST /prompt") {
                        if let Some(path) = &whisper_pid {
                            let pid = std::fs::read_to_string(path)?;
                            assert!(
                                !Path::new(&format!("/proc/{}", pid.trim())).exists(),
                                "Whisper must be reaped before ComfyUI receives the job"
                            );
                        }
                        let request: Value = serde_json::from_slice(&request_body)?;
                        assert_eq!(request["prompt"]["6"]["inputs"]["text"], "en svensk skog");
                        assert!(request["prompt_id"].as_str().is_some());
                    }
                    write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n", body.len())?;
                    stream.write_all(&body)?;
                }
                Ok(())
            });
            Ok(Self { url, thread })
        }
        fn finish(self) -> Result<()> {
            self.thread.join().expect("mock API panicked")
        }
    }
    fn reply(path: impl Into<String>, body: Value) -> (String, u16, Vec<u8>) {
        (path.into(), 200, serde_json::to_vec(&body).unwrap())
    }
    fn config() -> ComfySettings {
        ComfySettings {
            workflow: json!({
                "6": {"class_type":"CLIPTextEncode", "inputs":{"text":"positive", "clip":["4",1]}},
                "7": {"class_type":"CLIPTextEncode", "inputs":{"text":"negative"}},
                "3": {"class_type":"KSampler", "inputs":{"seed":123,"steps":20}}
            }),
            workflow_name: "test.json".into(),
            node_id: "6".into(),
            input_name: "text".into(),
            ..Default::default()
        }
    }
    fn fixture() -> Result<(tempfile::TempDir, Database, ImageJob)> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path())?;
        db.save_comfy_settings(&config())?;
        db.enqueue_image(ImageDraft {
            prompt: "en svensk skog".into(),
            ..Default::default()
        })?;
        let job = db.next_image_job()?.unwrap();
        Ok((temp, db, job))
    }
    fn generated(path: &Path) -> GeneratedImage {
        GeneratedImage {
            deleted: false,
            created_at: None,
            path: path.to_string_lossy().into(),
            node_id: "9".into(),
            filename: path.file_name().unwrap().to_string_lossy().into(),
            subfolder: "".into(),
            image_type: "output".into(),
        }
    }
    #[test]
    fn deleting_images_rejects_stored_paths_outside_the_job_directory() -> Result<()> {
        let (temp, db, mut job) = fixture()?;
        let directory = temp.path().join("images").join(&job.id);
        std::fs::create_dir_all(&directory)?;
        let outside = temp.path().join("outside.png");
        std::fs::write(&outside, b"image")?;
        let link = directory.join("link.png");
        std::os::unix::fs::symlink(&outside, &link)?;
        job.images.push(generated(&outside));
        job.images.push(generated(&link));
        db.save_image_job(&job)?;
        let error = db
            .delete_generated_image(temp.path(), &job.id, &job.images[0].path)
            .unwrap_err();
        assert_eq!(error.to_string(), "Ogiltig bildsökväg");
        let error = db
            .delete_generated_image(temp.path(), &job.id, &job.images[1].path)
            .unwrap_err();
        assert_eq!(error.to_string(), "Bilden ligger utanför bildarkivet");
        assert!(outside.exists() && link.exists());
        assert!(db.image_jobs()?[0].images.iter().all(|i| !i.deleted));
        Ok(())
    }
    #[test]
    fn deleted_images_stay_deleted_and_preserve_other_results() -> Result<()> {
        let (temp, db, mut job) = fixture()?;
        let directory = temp.path().join("images").join(&job.id);
        std::fs::create_dir_all(&directory)?;
        for name in ["first.png", "second.png"] {
            let path = directory.join(name);
            std::fs::write(&path, b"image")?;
            job.images.push(generated(&path));
        }
        db.save_image_job(&job)?;
        let first = job.images[0].path.clone();
        assert!(db
            .delete_generated_image(temp.path(), &job.id, "/tmp/unrelated.png")
            .is_err());
        db.delete_generated_image(temp.path(), &job.id, &first)?;
        db.delete_generated_image(temp.path(), &job.id, &first)?;
        assert!(!Path::new(&first).exists());
        assert!(Path::new(&job.images[1].path).exists());
        let reopened = Database::open(temp.path())?;
        let mut stored = reopened.image_jobs()?.remove(0);
        assert!(stored.images[0].deleted);
        assert_eq!(stored.images.len(), 2);
        // Both outputs are already accounted for, including the deleted one.
        // An unusable server URL would fail if the deleted image were fetched again.
        stored.config.url = "http://127.0.0.1:1".into();
        download_images(
            &Client::new(),
            temp.path(),
            &mut stored,
            &json!({"9":{"images":[
                {"filename":"first.png"}, {"filename":"second.png"}
            ]}}),
            &reopened,
        )?;
        assert_eq!(stored.images.len(), 2);
        assert!(!Path::new(&first).exists());
        Ok(())
    }
    fn wait_until(mut condition: impl FnMut() -> bool) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !condition() {
            if Instant::now() > deadline {
                bail!("Timed out waiting for supervisor");
            }
            thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    }
    #[test]
    fn gpu_handoff_finishes_active_transcription_and_respects_manual_pause() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let service = crate::service::Service::open(temp.path().join("data"))?;
        service.db.save_comfy_settings(&ComfySettings {
            url: "http://127.0.0.1:0".into(),
            ..Default::default()
        })?;
        let script = temp.path().join("fake_worker.py");
        let pid_file = script.with_extension("pid");
        std::fs::write(
            &script,
            r#"import json, os, sys, time
from pathlib import Path
Path(__file__).with_suffix('.pid').write_text(str(os.getpid()))
print(json.dumps({'type':'ready'}), flush=True)
for line in sys.stdin:
    job = json.loads(line)
    time.sleep(1)
    print(json.dumps({'type':'completed','job_id':job['job_id'],'metadata':{'duration':1},'segments':[]}), flush=True)
"#,
        )?;
        let mut settings = service.db.settings(&service.root)?;
        settings.python_path = "python3".into();
        settings.model_path = temp.path().join("model").to_string_lossy().into();
        std::fs::create_dir_all(&settings.model_path)?;
        std::fs::write(
            Path::new(&settings.model_path).join("model.bin"),
            b"fixture",
        )?;
        settings.transcription_enabled = true;
        settings.auto_import = false;
        service.db.save_settings(&settings)?;
        let audio = temp.path().join("test.wav");
        std::fs::write(&audio, b"fixture")?;
        crate::archive::import_file(&service.db, &service.root, &audio, None)?;
        service.start(script);
        // Always stop the service, including if an assertion fails.
        struct Stop(std::sync::Arc<crate::service::Service>);
        impl Drop for Stop {
            fn drop(&mut self) {
                self.0.shutdown();
            }
        }
        let _stop = Stop(service.clone());
        wait_until(|| {
            service
                .snapshot()
                .is_ok_and(|s| s.worker_status == "running")
        })?;
        let first_pid = std::fs::read_to_string(&pid_file)?;
        let id = uuid::Uuid::new_v4().to_string();
        let server = MockServer::start_with_guard(
            vec![
                reply("POST /prompt", json!({"prompt_id":id})),
                reply(
                    format!("GET /history/{id}"),
                    json!({id.clone():{"status":{"completed":true,"status_str":"success"},"outputs":{}}}),
                ),
                reply("GET /queue", json!({"queue_running":[],"queue_pending":[]})),
                ("POST /free".into(), 200, vec![]),
            ],
            Some(pid_file.clone()),
        )?;
        let mut config = config();
        config.url = server.url.clone();
        config.workflow = build_prompt(&config, "en svensk skog")?;
        service.db.save_image_job(&ImageJob {
            entry_id: None,
            draft_revision_id: None,
            id,
            created_at: chrono::Utc::now().to_rfc3339(),
            draft: ImageDraft {
                prompt: "en svensk skog".into(),
                ..Default::default()
            },
            config,
            status: "queued".into(),
            prompt_id: None,
            error: None,
            images: vec![],
            release_pending: false,
        })?;
        wait_until(|| {
            service
                .db
                .image_jobs()
                .is_ok_and(|j| j[0].status == "queued_comfy")
        })?;
        assert_eq!(service.db.recordings()?[0].status, "completed");
        service.control_queue(false)?;
        wait_until(|| {
            service.db.next_image_job().is_ok_and(|j| j.is_none())
                && service
                    .snapshot()
                    .is_ok_and(|s| s.worker_status == "paused")
        })?;
        assert_eq!(std::fs::read_to_string(&pid_file)?, first_pid);
        assert!(!service.db.settings(&service.root)?.transcription_enabled);
        // Only an explicit user start should reload Whisper after that pause.
        service.control_queue(true)?;
        wait_until(|| service.snapshot().is_ok_and(|s| s.worker_status == "ready"))?;
        assert_ne!(std::fs::read_to_string(&pid_file)?, first_pid);
        server.finish()?;
        Ok(())
    }
    #[test]
    fn saved_workflows_survive_restart_and_each_job_keeps_its_selected_flow() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path())?;
        let original = config();
        let first = db.save_workflow(SavedWorkflow {
            id: String::new(),
            name: "Foto".into(),
            workflow: original.workflow.clone(),
            node_id: "6".into(),
            input_name: "text".into(),
            negative_field: None,
        })?;
        let mut second_json = original.workflow.clone();
        second_json["3"]["inputs"]["steps"] = json!(50);
        let second = db.save_workflow(SavedWorkflow {
            id: String::new(),
            name: "Illustration".into(),
            workflow: second_json,
            node_id: "6".into(),
            input_name: "text".into(),
            negative_field: None,
        })?;
        db.select_workflow(&first)?;
        let first_job = db.enqueue_image_with_workflow(
            ImageDraft {
                prompt: "första bilden".into(),
                ..Default::default()
            },
            Some(&first),
        )?;
        db.select_workflow(&second)?;
        db.enqueue_image(ImageDraft {
            prompt: "andra bilden".into(),
            ..Default::default()
        })?;
        let reopened = Database::open(temp.path())?;
        assert_eq!(reopened.saved_workflows()?.len(), 2);
        assert_eq!(
            reopened.comfy_settings()?.selected_workflow_id.as_deref(),
            Some(second.as_str())
        );
        let jobs = reopened.image_jobs()?;
        let first_copy = jobs.iter().find(|j| j.id == first_job).unwrap();
        assert_eq!(first_copy.config.workflow_name, "Foto");
        assert_eq!(first_copy.config.workflow["3"]["inputs"]["steps"], 20);
        assert_eq!(jobs[0].config.workflow_name, "Illustration");
        assert_eq!(jobs[0].config.workflow["3"]["inputs"]["steps"], 50);
        let mut edited = reopened
            .saved_workflows()?
            .into_iter()
            .find(|w| w.id == first)
            .unwrap();
        edited.name = "Ändrat foto".into();
        edited.workflow["3"]["inputs"]["steps"] = json!(99);
        reopened.save_workflow(edited)?;
        assert_eq!(
            reopened
                .image_jobs()?
                .iter()
                .find(|j| j.id == first_job)
                .unwrap()
                .config
                .workflow["3"]["inputs"]["steps"],
            20
        );
        assert!(reopened.select_workflow("missing").is_err());
        Ok(())
    }
    #[test]
    fn positive_and_negative_prompts_are_independent_durable_and_allow_explicit_empty() -> Result<()>
    {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path())?;
        let config = config();
        let id = db.save_workflow(SavedWorkflow {
            id: String::new(),
            name: "Två promptar".into(),
            workflow: config.workflow.clone(),
            node_id: "6".into(),
            input_name: "text".into(),
            negative_field: Some(PromptField {
                node_id: "7".into(),
                input_name: "text".into(),
            }),
        })?;
        db.select_workflow(&id)?;
        let draft = ImageDraft {
            prompt: "en skog".into(),
            negative_prompt: Some("suddigt, vattenstämpel".into()),
            ..Default::default()
        };
        db.save_image_draft(&draft)?;
        db.enqueue_image_with_workflow(draft, Some(&id))?;
        let reopened = Database::open(temp.path())?;
        assert_eq!(
            reopened.image_draft()?.negative_prompt.as_deref(),
            Some("suddigt, vattenstämpel")
        );
        let job = reopened.image_jobs()?[0].clone();
        assert_eq!(job.config.workflow["6"]["inputs"]["text"], "en skog");
        assert_eq!(
            job.config.workflow["7"]["inputs"]["text"],
            "suddigt, vattenstämpel"
        );
        assert_eq!(job.config.workflow["3"], config.workflow["3"]);
        assert_eq!(
            job.draft.negative_prompt.as_deref(),
            Some("suddigt, vattenstämpel")
        );
        assert_eq!(
            reopened.saved_workflows()?[0].workflow["7"]["inputs"]["text"],
            "negative"
        );
        reopened.enqueue_image(ImageDraft {
            prompt: "ny bild".into(),
            ..Default::default()
        })?;
        assert_eq!(
            reopened.image_jobs()?[0].config.workflow["7"]["inputs"]["text"],
            "negative"
        );
        reopened.enqueue_image(ImageDraft {
            prompt: "ny bild".into(),
            negative_prompt: Some(String::new()),
            ..Default::default()
        })?;
        assert_eq!(
            reopened.image_jobs()?[0].config.workflow["7"]["inputs"]["text"],
            ""
        );
        let mut invalid = reopened.saved_workflows()?[0].clone();
        invalid.negative_field = Some(PromptField {
            node_id: "6".into(),
            input_name: "text".into(),
        });
        assert!(reopened.save_workflow(invalid.clone()).is_err());
        invalid.negative_field = Some(PromptField {
            node_id: "6".into(),
            input_name: "clip".into(),
        });
        assert!(reopened.save_workflow(invalid.clone()).is_err());
        invalid.negative_field = Some(PromptField {
            node_id: "missing".into(),
            input_name: "text".into(),
        });
        assert!(reopened.save_workflow(invalid).is_err());
        Ok(())
    }
    #[test]
    fn deleting_the_selected_workflow_selects_the_next_and_preserves_jobs() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path())?;
        let save = |name: &str| {
            db.save_workflow(SavedWorkflow {
                id: String::new(),
                name: name.into(),
                workflow: config().workflow,
                node_id: "6".into(),
                input_name: "text".into(),
                negative_field: None,
            })
        };
        let [first, second, third, fourth] = [
            save("Första")?,
            save("Andra")?,
            save("Tredje")?,
            save("Fjärde")?,
        ];
        let selected = |db: &Database| db.comfy_settings().map(|c| c.selected_workflow_id);
        // Saving never changes which flow Images uses for its next image.
        assert_eq!(selected(&db)?, None);
        db.select_workflow(&fourth)?;
        db.enqueue_image_with_workflow(
            ImageDraft {
                prompt: "sparat bildjobb".into(),
                ..Default::default()
            },
            Some(&first),
        )?;
        db.delete_workflow(&first)?;
        assert_eq!(selected(&db)?, Some(fourth.clone()));
        assert_eq!(db.image_jobs()?[0].config.workflow_name, "Första");
        assert_eq!(
            db.image_jobs()?[0].config.workflow["6"]["inputs"]["text"],
            "sparat bildjobb"
        );
        db.select_workflow(&second)?;
        db.delete_workflow(&second)?;
        assert_eq!(selected(&db)?, Some(third.clone()));
        db.enqueue_image(ImageDraft {
            prompt: "nästa flöde".into(),
            ..Default::default()
        })?;
        assert_eq!(db.image_jobs()?[0].config.workflow_name, "Tredje");
        db.select_workflow(&fourth)?;
        db.delete_workflow(&fourth)?;
        assert_eq!(selected(&db)?, Some(third.clone()));
        db.delete_workflow(&third)?;
        let reopened = Database::open(temp.path())?;
        assert!(reopened.saved_workflows()?.is_empty());
        assert_eq!(selected(&reopened)?, None);
        assert!(reopened
            .enqueue_image(ImageDraft {
                prompt: "ny bild".into(),
                ..Default::default()
            })
            .is_err());
        assert_eq!(reopened.image_jobs()?.len(), 2);
        assert!(reopened.delete_workflow(&first).is_err());
        assert_eq!(reopened.comfy_settings()?.url, "http://127.0.0.1:8188");
        Ok(())
    }
    #[test]
    fn older_json_defaults_to_no_negative_prompt_mapping() -> Result<()> {
        let old = json!({"id":"old","name":"Tidigare","workflow":config().workflow,"node_id":"6","input_name":"text"});
        let workflow: SavedWorkflow = serde_json::from_value(old)?;
        assert!(workflow.negative_field.is_none());
        let draft: ImageDraft = serde_json::from_value(
            json!({"prompt":"positiv","source_text":"","recording_id":null,"run_id":null}),
        )?;
        assert!(draft.negative_prompt.is_none());
        Ok(())
    }
    #[test]
    fn legacy_workflow_migrates_once_without_losing_server_settings() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path())?;
        let mut config = config();
        config.workflow_name = "Mitt gamla flöde".into();
        config.url = "http://127.0.0.1:9191".into();
        db.save_comfy_settings(&config)?;
        db.connect()?
            .execute_batch("DROP TABLE image_workflows; PRAGMA user_version=2;")?;
        let db = Database::open(temp.path())?;
        assert_eq!(db.saved_workflows()?.len(), 1);
        assert_eq!(db.saved_workflows()?[0].name, "Mitt gamla flöde");
        assert_eq!(
            db.comfy_settings()?.selected_workflow_id.as_deref(),
            Some("legacy")
        );
        assert_eq!(db.comfy_settings()?.url, "http://127.0.0.1:9191");
        assert_eq!(Database::open(temp.path())?.saved_workflows()?.len(), 1);
        db.save_comfy_connection(ComfyConnection {
            url: "http://127.0.0.1:8188".into(),
            comfy_directory: "/tmp/test".into(),
            comfy_python_path: "/tmp/python".into(),
        })?;
        assert_eq!(
            db.comfy_settings()?.selected_workflow_id.as_deref(),
            Some("legacy")
        );
        assert_eq!(db.saved_workflows()?[0].workflow, config.workflow);
        Ok(())
    }
    #[test]
    fn workflow_changes_only_explicit_text_field_and_rejects_ui_format() -> Result<()> {
        let original = config();
        let prompt = build_prompt(&original, "changed")?;
        assert_eq!(prompt["6"]["inputs"]["text"], "changed");
        assert_eq!(prompt["7"], original.workflow["7"]);
        assert_eq!(prompt["3"], original.workflow["3"]);
        assert_eq!(original.workflow["6"]["inputs"]["text"], "positive");
        assert!(validate_workflow(&json!({"nodes":[],"links":[]})).is_err());
        let mut bad = original;
        bad.input_name = "clip".into();
        assert!(build_prompt(&bad, "changed").is_err());
        assert!(validate_url("https://example.com").is_err());
        assert!(validate_url("http://127.0.0.1:8188/path").is_err());
        validate_url("http://127.0.0.1:8188")?;
        Ok(())
    }
    #[test]
    fn schema_upgrade_preserves_settings_jobs_and_draft() -> Result<()> {
        let (temp, db, job) = fixture()?;
        let settings = crate::types::Settings::defaults(temp.path());
        db.save_settings(&settings)?;
        let draft = ImageDraft {
            prompt: "extra detaljer".into(),
            source_text: "källa".into(),
            ..Default::default()
        };
        db.save_image_draft(&draft)?;
        db.connect()?.execute_batch("PRAGMA user_version=1")?;
        let reopened = Database::open(temp.path())?;
        assert_eq!(reopened.image_draft()?.prompt, draft.prompt);
        assert_eq!(reopened.image_jobs()?[0].id, job.id);
        assert_eq!(
            reopened.settings(temp.path())?.model_path,
            settings.model_path
        );
        assert_eq!(
            reopened
                .connect()?
                .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))?,
            4
        );
        Ok(())
    }
    #[test]
    fn submits_once_downloads_all_images_and_recovers_without_resubmitting() -> Result<()> {
        let (temp, db, mut job) = fixture()?;
        let id = job.id.clone();
        let outputs = json!({"9":{"images":[{"filename":"a.png","subfolder":"test folder","type":"output"},{"filename":"b.png","subfolder":"","type":"temp"}]}});
        let history = json!({id.clone(): {"status":{"completed":true,"status_str":"success"},"outputs":outputs}});
        let server = MockServer::start(vec![
            reply("POST /prompt", json!({"prompt_id":id})),
            reply(format!("GET /history/{id}"), json!({})),
            reply(
                "GET /queue",
                json!({"queue_running":[[0,id]],"queue_pending":[]}),
            ),
            reply(format!("GET /history/{id}"), history.clone()),
            (
                "GET /view?filename=a.png&subfolder=test+folder&type=output".into(),
                200,
                b"\x89PNG\r\n\x1a\nfirst".to_vec(),
            ),
            (
                "GET /view?filename=b.png&subfolder=&type=temp".into(),
                200,
                b"\x89PNG\r\n\x1a\nsecond".to_vec(),
            ),
            // Simulate an app restart after saving results; fetch history but do not POST or redownload.
            reply(format!("GET /history/{id}"), history),
            reply("GET /queue", json!({"queue_running":[],"queue_pending":[]})),
            ("POST /free".into(), 200, vec![]),
        ])?;
        job.config.url = server.url.clone();
        db.save_image_job(&job)?;
        step(&db, temp.path(), job)?;
        assert_eq!(db.next_image_job()?.unwrap().status, "queued_comfy");
        step(&db, temp.path(), db.next_image_job()?.unwrap())?;
        assert_eq!(db.next_image_job()?.unwrap().status, "running");
        step(&db, temp.path(), db.next_image_job()?.unwrap())?;
        let completed = db.image_jobs()?[0].clone();
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.images.len(), 2);
        assert!(completed
            .images
            .iter()
            .all(|i| Path::new(&i.path).is_file()));
        let reopened = Database::open(temp.path())?;
        // Follow-up reuses the existing server ID and downloaded files.
        let mut recovered = completed.clone();
        recovered.status = "downloading".into();
        reopened.save_image_job(&recovered)?;
        step(&reopened, temp.path(), recovered)?;
        assert_eq!(reopened.image_jobs()?[0].images.len(), 2);
        step(&reopened, temp.path(), reopened.next_image_job()?.unwrap())?;
        assert!(reopened.next_image_job()?.is_none());
        assert!(!reopened.settings(temp.path())?.transcription_enabled);
        server.finish()?;
        Ok(())
    }
    #[test]
    fn version_one_migration_preserves_recordings_and_edited_transcripts() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("data");
        let db = Database::open(&root)?;
        let audio = temp.path().join("a.wav");
        std::fs::write(&audio, b"audio fixture")?;
        crate::archive::import_file(&db, &root, &audio, None)?;
        let job = db.claim()?.unwrap();
        db.complete(
            &job,
            &json!({"duration":1}),
            &[crate::types::Segment {
                id: 0,
                start: 0.0,
                end: 1.0,
                text: "ursprunglig".into(),
                edited_text: None,
                words: vec![],
            }],
        )?;
        let original = db.transcript(&job.recording_id, None)?;
        db.edit(original.segments[0].id, "rättad svenska")?;
        db.connect()?.execute_batch("DROP TABLE image_jobs; DROP TABLE image_draft; DROP TABLE image_config; PRAGMA user_version=1;")?;
        let migrated = Database::open(&root)?;
        assert_eq!(migrated.recordings()?.len(), 1);
        assert_eq!(migrated.recordings()?[0].status, "completed");
        let transcript = migrated.transcript(&job.recording_id, None)?;
        assert_eq!(transcript.run_id, original.run_id);
        assert_eq!(
            transcript.segments[0].edited_text.as_deref(),
            Some("rättad svenska")
        );
        assert!(migrated.image_jobs()?.is_empty());
        assert_eq!(migrated.comfy_settings()?.url, "http://127.0.0.1:8188");
        Ok(())
    }
    #[test]
    fn failed_download_is_retried_from_history_without_new_generation() -> Result<()> {
        let (temp, db, mut job) = fixture()?;
        let id = job.id.clone();
        let history = json!({id.clone(): {"status":{"completed":true,"status_str":"success"},"outputs":{"9":{"images":[{"filename":"a.png","subfolder":"","type":"output"}]}}}});
        let server = MockServer::start(vec![
            reply(format!("GET /history/{id}"), history.clone()),
            ("GET /view?".into(), 404, b"missing".to_vec()),
            reply(format!("GET /history/{id}"), history),
            ("GET /view?".into(), 200, b"\x89PNG\r\n\x1a\nsaved".to_vec()),
        ])?;
        job.config.url = server.url.clone();
        job.status = "downloading".into();
        job.prompt_id = Some(id);
        job.release_pending = true;
        db.save_image_job(&job)?;
        step(&db, temp.path(), job)?;
        let failed = db.image_jobs()?[0].clone();
        assert_eq!(failed.status, "failed");
        assert!(failed.images.is_empty());
        assert!(failed.error.as_deref().unwrap().contains("hämta igen"));
        db.retry_image_download(&failed.id)?;
        step(&db, temp.path(), db.next_image_job()?.unwrap())?;
        assert_eq!(db.image_jobs()?[0].status, "completed");
        assert_eq!(db.image_jobs()?[0].images.len(), 1);
        server.finish()?;
        Ok(())
    }
    #[test]
    fn server_error_after_submission_is_followed_up_without_reposting() -> Result<()> {
        let (temp, db, mut job) = fixture()?;
        let id = job.id.clone();
        let server = MockServer::start(vec![
            ("POST /prompt".into(), 500, b"response interrupted".to_vec()),
            reply(format!("GET /history/{id}"), json!({})),
            reply(
                "GET /queue",
                json!({"queue_running":[],"queue_pending":[[0,id]]}),
            ),
        ])?;
        job.config.url = server.url.clone();
        db.save_image_job(&job)?;
        step(&db, temp.path(), job)?;
        let interrupted = db.next_image_job()?.unwrap();
        assert_eq!(interrupted.status, "submitting");
        assert_eq!(interrupted.prompt_id.as_deref(), Some(id.as_str()));
        step(&db, temp.path(), interrupted)?;
        assert_eq!(db.image_jobs()?[0].status, "queued_comfy");
        server.finish()?;
        Ok(())
    }
    #[test]
    fn interrupted_post_is_checked_and_never_replayed() -> Result<()> {
        let (temp, db, mut job) = fixture()?;
        let id = job.id.clone();
        let server = MockServer::start(vec![
            reply(format!("GET /history/{id}"), json!({})),
            reply("GET /queue", json!({"queue_running":[],"queue_pending":[]})),
            reply(format!("GET /history/{id}"), json!({})),
        ])?;
        job.config.url = server.url.clone();
        job.status = "submitting".into();
        job.prompt_id = Some(id);
        job.release_pending = true;
        db.save_image_job(&job)?;
        step(&db, temp.path(), job)?;
        assert_eq!(db.image_jobs()?[0].status, "uncertain");
        server.finish()?;
        Ok(())
    }
    #[test]
    fn execution_error_is_recorded_and_manual_followup_preserves_server_id() -> Result<()> {
        let (temp, db, mut job) = fixture()?;
        let id = job.id.clone();
        let server = MockServer::start(vec![reply(
            format!("GET /history/{id}"),
            json!({id.clone():{"status":{"completed":false,"status_str":"error","messages":[["execution_error",{"exception_message":"out of memory"}]]}}}),
        )])?;
        job.config.url = server.url.clone();
        job.status = "queued_comfy".into();
        job.prompt_id = Some(id);
        job.release_pending = true;
        db.save_image_job(&job)?;
        step(&db, temp.path(), job)?;
        let failed = db.image_jobs()?[0].clone();
        assert_eq!(failed.status, "failed");
        assert!(failed.error.unwrap().contains("out of memory"));
        db.dismiss_image_job(&failed.id)?;
        assert!(db.next_image_job()?.is_none());
        db.retry_image_download(&failed.id)?;
        assert_eq!(db.next_image_job()?.unwrap().status, "queued_comfy");
        server.finish()?;
        Ok(())
    }
}
