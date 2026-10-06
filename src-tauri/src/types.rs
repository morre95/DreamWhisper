use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub python_path: String,
    pub model_path: String,
    #[serde(default)]
    pub english_model_path: String,
    pub batch_size: u32,
    pub auto_import: bool,
    pub transcription_enabled: bool,
    #[serde(default)]
    pub start_at_login: bool,
}

impl Settings {
    pub fn defaults(root: &std::path::Path) -> Self {
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let python = project.join(".venv/bin/python");
        let model = project.join("models/kb-whisper-large");
        Self {
            python_path: if cfg!(debug_assertions) && python.is_file() {
                python.to_string_lossy().into()
            } else {
                "python3.12".into()
            },
            model_path: if cfg!(debug_assertions) && model.join("model.bin").is_file() {
                model.to_string_lossy().into()
            } else {
                root.join("models/kb-whisper-large")
                    .to_string_lossy()
                    .into()
            },
            english_model_path: project
                .join("models/whisper-large-v3")
                .to_string_lossy()
                .into(),
            batch_size: 8,
            auto_import: true,
            transcription_enabled: false,
            start_at_login: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub block_path: String,
    pub vendor: String,
    pub model: String,
    pub serial: String,
    pub uuid: String,
    pub label: String,
    pub mount_path: Option<String>,
    pub registered: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub name: String,
    pub archive_path: String,
    pub sha256: String,
    pub size: u64,
    pub source_modified_at: Option<String>,
    pub imported_at: String,
    pub status: String,
    pub error: Option<String>,
    pub attempts: u32,
    pub duration: Option<f64>,
    pub language: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub word: String,
    pub probability: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Segment {
    #[serde(default)]
    pub id: i64,
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(default)]
    pub edited_text: Option<String>,
    #[serde(default)]
    pub words: Vec<Word>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Transcript {
    pub run_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub segments: Vec<Segment>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportManifest {
    pub id: String,
    pub name: String,
    pub archive_file: String,
    pub sha256: String,
    pub size: u64,
    pub source_path: String,
    pub source_device: Option<String>,
    pub source_modified_at: Option<String>,
    pub imported_at: String,
}

#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub imported: u32,
    pub skipped: u32,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub queue: Vec<String>,
    pub worker_status: String,
    pub worker_error: Option<String>,
    pub recordings: Vec<Recording>,
    pub devices: Vec<Device>,
    pub settings: Settings,
    pub data_dir: String,
    pub activity: String,
    pub device_error: Option<String>,
    pub last_import: Option<ImportReport>,
}
