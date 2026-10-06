//! Persistent, single-GPU ComfyUI jobs. Called only by the worker supervisor.
use crate::db::Database;
use anyhow::{bail, Context, Result};
use reqwest::blocking::{Client, Response};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs::OpenOptions, io::{Read, Write}, path::Path, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComfySettings {
    pub url: String,
    pub workflow: Value,
    pub workflow_name: String,
    pub node_id: String,
    pub input_name: String,
}
impl Default for ComfySettings {
    fn default() -> Self {
        Self { url: "http://127.0.0.1:8188".into(), workflow: json!({}), workflow_name: String::new(), node_id: String::new(), input_name: String::new() }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ImageDraft {
    pub prompt: String,
    pub source_text: String,
    pub recording_id: Option<String>,
    pub run_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeneratedImage {
    pub path: String,
    pub node_id: String,
    pub filename: String,
    pub subfolder: String,
    pub image_type: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageJob {
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
    pub settings: ComfySettings,
    pub draft: ImageDraft,
    pub jobs: Vec<ImageJob>,
}

impl Database {
    pub fn comfy_settings(&self) -> Result<ComfySettings> {
        let value: Option<String> = self.connect()?.query_row("SELECT json FROM image_config WHERE id=1", [], |r| r.get(0)).optional()?;
        Ok(value.map(|s| serde_json::from_str(&s)).transpose()?.unwrap_or_default())
    }
    pub fn save_comfy_settings(&self, config: &ComfySettings) -> Result<()> {
        validate_url(&config.url)?;
        validate_workflow(&config.workflow)?;
        build_prompt(config, "test")?;
        self.connect()?.execute("INSERT INTO image_config VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json", [serde_json::to_string(config)?])?;
        Ok(())
    }
    pub fn image_draft(&self) -> Result<ImageDraft> {
        let value: Option<String> = self.connect()?.query_row("SELECT json FROM image_draft WHERE id=1", [], |r| r.get(0)).optional()?;
        Ok(value.map(|s| serde_json::from_str(&s)).transpose()?.unwrap_or_default())
    }
    pub fn save_image_draft(&self, draft: &ImageDraft) -> Result<()> {
        self.connect()?.execute("INSERT INTO image_draft VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json", [serde_json::to_string(draft)?])?;
        Ok(())
    }
    pub fn image_jobs(&self) -> Result<Vec<ImageJob>> {
        let c = self.connect()?;
        let mut query = c.prepare("SELECT json FROM image_jobs ORDER BY rowid DESC")?;
        let rows = query.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }
    pub fn save_image_job(&self, job: &ImageJob) -> Result<()> {
        self.connect()?.execute("INSERT INTO image_jobs VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json", rusqlite::params![job.id, serde_json::to_string(job)?])?;
        Ok(())
    }
    pub fn enqueue_image(&self, draft: ImageDraft) -> Result<String> {
        if draft.prompt.trim().is_empty() { bail!("Skriv en bildprompt först."); }
        if draft.prompt.len() > 100_000 { bail!("Bildprompten är för lång (max 100 000 byte)."); }
        let mut config = self.comfy_settings()?;
        validate_url(&config.url)?;
        config.workflow = build_prompt(&config, &draft.prompt)?;
        // Resolve source references before committing a durable job.
        if let Some(id) = &draft.recording_id {
            let transcript = self.transcript(id, draft.run_id.as_deref())?;
            if draft.run_id.is_some() && transcript.run_id != draft.run_id { bail!("Transkriptversionen finns inte."); }
        }
        let job = ImageJob { id: uuid::Uuid::new_v4().to_string(), created_at: chrono::Utc::now().to_rfc3339(), draft, config, status: "queued".into(), prompt_id: None, error: None, images: vec![], release_pending: false };
        self.save_image_job(&job)?;
        Ok(job.id)
    }
    pub fn image_snapshot(&self) -> Result<ImageSnapshot> {
        Ok(ImageSnapshot { settings: self.comfy_settings()?, draft: self.image_draft()?, jobs: self.image_jobs()? })
    }
    pub fn next_image_job(&self) -> Result<Option<ImageJob>> {
        // Finish interrupted/accepted submissions before taking another queued job.
        let jobs = self.image_jobs()?;
        Ok(jobs.iter().rev().find(|j| j.status != "queued" && (j.release_pending || !terminal(&j.status))).cloned()
            .or_else(|| jobs.into_iter().rev().find(|j| j.status == "queued")))
    }
    pub fn retry_image_download(&self, id: &str) -> Result<()> {
        let mut job = self.image_jobs()?.into_iter().find(|j| j.id == id).context("Bildjobbet saknas")?;
        if job.prompt_id.is_none() || !terminal(&job.status) { bail!("Jobbet kan inte följas upp just nu."); }
        job.status = "queued_comfy".into();
        job.release_pending = true;
        job.error = None;
        self.save_image_job(&job)
    }
}
fn terminal(status: &str) -> bool { matches!(status, "completed" | "failed" | "uncertain") }

pub fn validate_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value.trim()).context("Ogiltig ComfyUI-adress")?;
    if url.scheme() != "http" || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]" | "::1")) || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() || url.path() != "/" {
        bail!("Ange en lokal HTTP-adress, exempelvis http://127.0.0.1:8188.");
    }
    Ok(())
}
pub fn validate_workflow(workflow: &Value) -> Result<()> {
    let nodes = workflow.as_object().context("Workflow måste vara ett JSON-objekt i API-format.")?;
    if nodes.is_empty() || nodes.values().any(|v| !v["class_type"].is_string() || !v["inputs"].is_object()) {
        bail!("Workflow är inte i API-format. Exportera med Save (API Format) / Export (API) i ComfyUI.");
    }
    Ok(())
}
pub fn build_prompt(config: &ComfySettings, text: &str) -> Result<Value> {
    validate_workflow(&config.workflow)?;
    let mut workflow = config.workflow.clone();
    let field = workflow.get_mut(&config.node_id).and_then(|n| n.get_mut("inputs")).and_then(|n| n.get_mut(&config.input_name)).context("Välj nod och textfält för bildprompten.")?;
    if !field.is_string() { bail!("Det valda promptfältet måste innehålla text, inte en nodkoppling."); }
    *field = Value::String(text.into());
    Ok(workflow)
}
fn client() -> Result<Client> {
    Ok(Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(30)).build()?)
}
fn endpoint(config: &ComfySettings, suffix: &str) -> String { format!("{}{}", config.url.trim().trim_end_matches('/'), suffix) }
fn response_json(response: Response) -> Result<Value> {
    let status = response.status();
    let mut text = String::new();
    response.take(2 * 1024 * 1024).read_to_string(&mut text)?;
    if !status.is_success() { bail!("ComfyUI HTTP {status}: {text}"); }
    Ok(serde_json::from_str(&text).context("ComfyUI svarade inte med giltig JSON")?)
}
pub fn test_connection(url: &str) -> Result<()> {
    validate_url(url)?;
    let config = ComfySettings { url: url.into(), ..Default::default() };
    response_json(client()?.get(endpoint(&config, "/system_stats")).send().context("ComfyUI kunde inte nås. Starta servern och kontrollera adressen.")?)?;
    Ok(())
}
fn queue_contains(queue: &Value, key: &str, id: &str) -> bool {
    queue[key].as_array().is_some_and(|items| items.iter().any(|item| item[1].as_str() == Some(id)))
}
fn queue_empty(queue: &Value) -> bool {
    ["queue_running", "queue_pending"].iter().all(|key| queue[*key].as_array().is_some_and(Vec::is_empty))
}

/// One short supervisor tick. Never resubmits a job after a possibly accepted POST.
pub fn step(db: &Database, root: &Path, mut job: ImageJob) -> Result<()> {
    let http = client()?;
    if terminal(&job.status) {
        if job.release_pending {
            let queue = response_json(http.get(endpoint(&job.config, "/queue")).send()?)?;
            if !queue_empty(&queue) { return Ok(()); }
            http.post(endpoint(&job.config, "/free")).json(&json!({"unload_models": true, "free_memory": true})).send()?.error_for_status()?;
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
        let response = match http.post(endpoint(&job.config, "/prompt")).json(&json!({"prompt": job.config.workflow, "prompt_id": job.id, "client_id": job.id})).send() {
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
                job.status = if status.is_client_error() { "failed" } else { "submitting" }.into();
                job.error = Some(format!("{e:#}"));
                return db.save_image_job(&job);
            }
        };
        if let Some(id) = body["prompt_id"].as_str() { job.prompt_id = Some(id.into()); }
        else { job.error = Some(format!("ComfyUI bekräftade inte jobbet: {body}")); return db.save_image_job(&job); }
        job.status = "queued_comfy".into();
        job.error = None;
        return db.save_image_job(&job);
    }
    let id = job.prompt_id.as_deref().context("Bildjobbet saknar server-ID")?;
    let history = response_json(http.get(endpoint(&job.config, &format!("/history/{id}"))).send()?)?;
    if let Some(record) = history.get(id) {
        if record["status"]["status_str"] == "error" {
            job.status = "failed".into();
            job.error = Some(format!("ComfyUI kunde inte skapa bilden: {}", record["status"]["messages"]));
            return db.save_image_job(&job);
        }
        if record["status"]["completed"] == true {
            job.status = "downloading".into();
            job.error = None;
            db.save_image_job(&job)?;
            let result = download_images(&http, root, &mut job, &record["outputs"], db);
            match result {
                Ok(()) if !job.images.is_empty() => { job.status = "completed".into(); job.error = None; }
                Ok(()) => { job.status = "failed".into(); job.error = Some("Workflow slutfördes utan bildresultat. Lägg till SaveImage eller PreviewImage i ComfyUI.".into()); }
                Err(e) => { job.status = "failed".into(); job.error = Some(format!("Bilderna kunde inte hämtas: {e:#}. Välj Följ upp för att hämta igen utan ny bildkörning.")); }
            }
            return db.save_image_job(&job);
        }
    }
    let queue = response_json(http.get(endpoint(&job.config, "/queue")).send()?)?;
    // Check running first: a prompt can move between history and queue requests.
    if queue_contains(&queue, "queue_running", id) { job.status = "running".into(); job.error = None; }
    else if queue_contains(&queue, "queue_pending", id) { job.status = "queued_comfy".into(); job.error = None; }
    else {
        // Recheck history to avoid classifying a just-completed job as missing.
        let latest = response_json(http.get(endpoint(&job.config, &format!("/history/{id}"))).send()?)?;
        if latest.get(id).is_some() { return Ok(()); }
        job.status = "uncertain".into();
        job.error = Some("Jobbet finns inte i ComfyUI:s kö eller historik. Det skickas inte automatiskt igen. Kontrollera ComfyUI och välj Följ upp eller skapa ett nytt jobb.".into());
    }
    db.save_image_job(&job)
}
fn download_images(http: &Client, root: &Path, job: &mut ImageJob, outputs: &Value, db: &Database) -> Result<()> {
    let directory = root.join("images").join(&job.id);
    std::fs::create_dir_all(&directory)?;
    for (node_id, output) in outputs.as_object().context("Ogiltiga bildresultat")? {
        for image in output["images"].as_array().into_iter().flatten() {
            let filename = image["filename"].as_str().context("Bildresultatet saknar filnamn")?;
            let subfolder = image["subfolder"].as_str().unwrap_or("");
            let image_type = image["type"].as_str().unwrap_or("output");
            if job.images.iter().any(|i| i.node_id == *node_id && i.filename == filename && i.subfolder == subfolder && i.image_type == image_type && Path::new(&i.path).is_file()) { continue; }
            let extension = match Path::new(filename).extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase().as_str() {
                "png" => "png", "jpg" | "jpeg" => "jpg", "webp" => "webp", _ => bail!("Bildformatet stöds inte: {filename}"),
            };
            let path = directory.join(format!("{}.{}", uuid::Uuid::new_v4(), extension));
            let temporary = path.with_extension("part");
            let mut response = http.get(endpoint(&job.config, "/view")).query(&[("filename", filename), ("subfolder", subfolder), ("type", image_type)]).send()?.error_for_status()?;
            let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
            let result = (|| -> Result<()> {
                let size = std::io::copy(&mut (&mut response).take(64 * 1024 * 1024 + 1), &mut file)?;
                if size == 0 || size > 64 * 1024 * 1024 { bail!("Bildfilen är tom eller större än 64 MB."); }
                file.flush()?;
                file.sync_all()?;
                std::fs::rename(&temporary, &path)?;
                Ok(())
            })();
            if result.is_err() { let _ = std::fs::remove_file(&temporary); }
            result?;
            job.images.push(GeneratedImage { path: path.to_string_lossy().into(), node_id: node_id.clone(), filename: filename.into(), subfolder: subfolder.into(), image_type: image_type.into() });
            db.save_image_job(job)?;
        }
    }
    Ok(())
}
