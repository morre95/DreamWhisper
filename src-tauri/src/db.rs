use crate::types::*;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
pub struct Database {
    pub path: PathBuf,
}
#[derive(Debug)]
pub struct Job {
    pub id: String,
    pub recording_id: String,
    pub path: String,
}

impl Database {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let db = Self {
            path: root.join("dreamwhisper.sqlite3"),
        };
        let c = db.connect()?;
        let version: u32 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 2 {
            bail!("Databasen tillhör en nyare version av DreamWhisper");
        }
        c.execute_batch("PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS devices (id TEXT PRIMARY KEY, serial TEXT NOT NULL, uuid TEXT NOT NULL, json TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS recordings (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, archive_path TEXT NOT NULL,
            sha256 TEXT NOT NULL UNIQUE, size INTEGER NOT NULL, source_modified_at TEXT,
            imported_at TEXT NOT NULL, available INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE IF NOT EXISTS sources (
            recording_id TEXT NOT NULL REFERENCES recordings(id), path TEXT NOT NULL,
            device_id TEXT NOT NULL, modified_at TEXT, UNIQUE(recording_id,path,device_id)
        );
        CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY, recording_id TEXT NOT NULL UNIQUE REFERENCES recordings(id),
            status TEXT NOT NULL CHECK(status IN ('queued','running','completed','failed')),
            attempts INTEGER NOT NULL DEFAULT 0, error TEXT, started_at TEXT
        );
        CREATE TABLE IF NOT EXISTS transcription_runs (
            id TEXT PRIMARY KEY, recording_id TEXT NOT NULL REFERENCES recordings(id),
            created_at TEXT NOT NULL, metadata TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS segments (
            id INTEGER PRIMARY KEY, run_id TEXT NOT NULL REFERENCES transcription_runs(id),
            position INTEGER NOT NULL, start REAL NOT NULL, end REAL NOT NULL,
            text TEXT NOT NULL, edited_text TEXT, words TEXT NOT NULL,
            UNIQUE(run_id,position)
        );
        CREATE INDEX IF NOT EXISTS queue_status ON jobs(status);
        CREATE INDEX IF NOT EXISTS runs_recording ON transcription_runs(recording_id);
        CREATE TABLE IF NOT EXISTS image_config (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS image_draft (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS image_jobs (id TEXT PRIMARY KEY, json TEXT NOT NULL);
        PRAGMA user_version=2;")?;
        c.execute("UPDATE jobs SET status='queued', error='Återställd efter avbruten körning', started_at=NULL WHERE status='running'", [])?;
        Ok(db)
    }
    pub fn connect(&self) -> Result<Connection> {
        let c = Connection::open(&self.path)?;
        c.busy_timeout(Duration::from_secs(10))?;
        c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
        Ok(c)
    }
    pub fn settings(&self, root: &Path) -> Result<Settings> {
        let c = self.connect()?;
        let json: Option<String> = c
            .query_row("SELECT json FROM settings WHERE id=1", [], |r| r.get(0))
            .optional()?;
        match json {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Settings::defaults(root)),
        }
    }
    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        if ![1, 2, 4, 8, 16].contains(&s.batch_size) {
            bail!("Batchstorlek måste vara 1, 2, 4, 8 eller 16");
        }
        if s.python_path.trim().is_empty() || s.model_path.trim().is_empty() {
            bail!("Ange Python och modellmapp");
        }
        self.connect()?.execute(
            "INSERT INTO settings VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            [serde_json::to_string(s)?],
        )?;
        Ok(())
    }
    pub fn register(&self, d: &Device) -> Result<()> {
        if d.serial.is_empty() && d.uuid.is_empty() {
            bail!("Enheten saknar serienummer och volym-ID; använd manuell import");
        }
        self.connect()?.execute("INSERT INTO devices VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET json=excluded.json", params![d.id,d.serial,d.uuid,serde_json::to_string(d)?])?;
        Ok(())
    }
    pub fn unregister(&self, d: &Device) -> Result<()> {
        self.connect()?.execute(
            "DELETE FROM devices WHERE id=?1 OR (?2<>'' AND serial=?2)",
            params![d.id, d.serial],
        )?;
        Ok(())
    }
    pub fn registered(&self, d: &Device) -> Result<bool> {
        Ok(self.connect()?.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE (?1<>'' AND serial=?1) OR (?1='' AND ?2<>'' AND uuid=?2))", params![d.serial,d.uuid], |r| r.get(0))?)
    }
    pub fn saved_devices(&self) -> Result<Vec<Device>> {
        let c = self.connect()?;
        let mut q = c.prepare("SELECT json FROM devices")?;
        let json = q
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        json.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub fn has_hash(&self, hash: &str) -> Result<bool> {
        Ok(self.connect()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM recordings WHERE sha256=?1)",
            [hash],
            |r| r.get(0),
        )?)
    }
    pub fn add_manifest(&self, m: &ImportManifest, archive: &Path) -> Result<bool> {
        let mut c = self.connect()?;
        let tx = c.transaction()?;
        let inserted = tx.execute("INSERT OR IGNORE INTO recordings(id,name,archive_path,sha256,size,source_modified_at,imported_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![m.id,m.name,archive.to_string_lossy(),m.sha256,m.size,m.source_modified_at,m.imported_at])?;
        let id: String = tx.query_row(
            "SELECT id FROM recordings WHERE sha256=?1",
            [&m.sha256],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO sources VALUES(?1,?2,?3,?4)",
            params![
                id,
                m.source_path,
                m.source_device.as_deref().unwrap_or(""),
                m.source_modified_at
            ],
        )?;
        if inserted > 0 {
            tx.execute(
                "INSERT INTO jobs(id,recording_id,status) VALUES(?1,?2,'queued')",
                params![uuid::Uuid::new_v4().to_string(), id],
            )?;
        }
        tx.execute(
            "UPDATE recordings SET available=1,archive_path=?1 WHERE sha256=?2",
            params![archive.to_string_lossy(), m.sha256],
        )?;
        tx.commit()?;
        Ok(inserted > 0)
    }
    pub fn reconcile_missing(&self) -> Result<()> {
        let c = self.connect()?;
        let mut q = c.prepare("SELECT id,archive_path FROM recordings")?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, path) in rows {
            let available = Path::new(&path).is_file();
            c.execute(
                "UPDATE recordings SET available=?1 WHERE id=?2",
                params![available, id],
            )?;
        }
        Ok(())
    }
    pub fn recordings(&self) -> Result<Vec<Recording>> {
        let c = self.connect()?;
        let mut q = c.prepare("SELECT r.id,r.name,r.archive_path,r.sha256,r.size,r.source_modified_at,r.imported_at,CASE WHEN r.available=0 THEN 'failed' ELSE j.status END,CASE WHEN r.available=0 THEN 'Arkivfilen saknas' ELSE j.error END,j.attempts,(SELECT json_extract(metadata,'$.duration') FROM transcription_runs WHERE recording_id=r.id ORDER BY rowid DESC LIMIT 1) FROM recordings r JOIN jobs j ON j.recording_id=r.id ORDER BY r.imported_at DESC,r.id")?;
        let recordings = q
            .query_map([], |r| {
                Ok(Recording {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    archive_path: r.get(2)?,
                    sha256: r.get(3)?,
                    size: r.get(4)?,
                    source_modified_at: r.get(5)?,
                    imported_at: r.get(6)?,
                    status: r.get(7)?,
                    error: r.get(8)?,
                    attempts: r.get(9)?,
                    duration: r.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(recordings)
    }
    pub fn queue(&self) -> Result<Vec<String>> {
        let c = self.connect()?;
        let mut q = c.prepare("SELECT r.id FROM jobs j JOIN recordings r ON r.id=j.recording_id WHERE j.status IN ('queued','running') AND r.available=1 ORDER BY CASE WHEN j.status='running' THEN 0 ELSE 1 END,r.imported_at,j.rowid")?;
        let rows = q
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub fn claim(&self) -> Result<Option<Job>> {
        let mut c = self.connect()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let job = tx.query_row("SELECT j.id,r.id,r.archive_path FROM jobs j JOIN recordings r ON r.id=j.recording_id WHERE j.status='queued' AND r.available=1 ORDER BY r.imported_at,j.rowid LIMIT 1", [], |r| Ok(Job { id:r.get(0)?,recording_id:r.get(1)?,path:r.get(2)? })).optional()?;
        if let Some(j) = &job {
            tx.execute("UPDATE jobs SET status='running',attempts=attempts+1,error=NULL,started_at=?1 WHERE id=?2", params![chrono::Utc::now().to_rfc3339(),j.id])?;
        }
        tx.commit()?;
        Ok(job)
    }
    pub fn failed(&self, id: &str, error: &str) -> Result<()> {
        self.connect()?.execute(
            "UPDATE jobs SET status='failed',error=?1 WHERE id=?2",
            params![error, id],
        )?;
        Ok(())
    }
    pub fn retry(&self, recording: &str) -> Result<()> {
        let n = self.connect()?.execute("UPDATE jobs SET status='queued',error=NULL WHERE recording_id=?1 AND status<>'running' AND EXISTS(SELECT 1 FROM recordings WHERE id=?1 AND available=1)", [recording])?;
        if n == 0 {
            bail!("Inspelningen saknas, körs redan eller saknar arkivfil");
        }
        Ok(())
    }
    pub fn complete(
        &self,
        job: &Job,
        metadata: &serde_json::Value,
        segments: &[Segment],
    ) -> Result<()> {
        for s in segments {
            if !s.start.is_finite() || !s.end.is_finite() || s.start < 0.0 || s.end < s.start {
                bail!("Ogiltiga tidsstämplar från workern");
            }
        }
        let mut c = self.connect()?;
        let tx = c.transaction()?;
        let run = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO transcription_runs VALUES(?1,?2,?3,?4)",
            params![
                run,
                job.recording_id,
                chrono::Utc::now().to_rfc3339(),
                serde_json::to_string(metadata)?
            ],
        )?;
        for (i, s) in segments.iter().enumerate() {
            tx.execute("INSERT INTO segments(run_id,position,start,end,text,words) VALUES(?1,?2,?3,?4,?5,?6)", params![run,i,s.start,s.end,s.text,serde_json::to_string(&s.words)?])?;
        }
        tx.execute(
            "UPDATE jobs SET status='completed',error=NULL WHERE id=?1 AND status='running'",
            [&job.id],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn transcript(&self, recording: &str, requested_run: Option<&str>) -> Result<Transcript> {
        let c = self.connect()?;
        let run: Option<(String,String)> = match requested_run {
            Some(id) => c.query_row("SELECT id,metadata FROM transcription_runs WHERE id=?1 AND recording_id=?2", params![id,recording], |r| Ok((r.get(0)?,r.get(1)?))).optional()?,
            None => c.query_row("SELECT id,metadata FROM transcription_runs WHERE recording_id=?1 ORDER BY rowid DESC LIMIT 1", [recording], |r| Ok((r.get(0)?,r.get(1)?))).optional()?,
        };
        let Some((id, meta)) = run else {
            return Ok(Transcript {
                run_id: None,
                metadata: None,
                segments: vec![],
            });
        };
        let mut q = c.prepare("SELECT id,start,end,text,edited_text,words FROM segments WHERE run_id=?1 ORDER BY position")?;
        let segments = q
            .query_map([&id], |r| {
                Ok(Segment {
                    id: r.get(0)?,
                    start: r.get(1)?,
                    end: r.get(2)?,
                    text: r.get(3)?,
                    edited_text: r.get(4)?,
                    words: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Transcript {
            run_id: Some(id),
            metadata: Some(serde_json::from_str(&meta)?),
            segments,
        })
    }
    pub fn runs(&self, recording: &str) -> Result<Vec<serde_json::Value>> {
        let c = self.connect()?;
        let mut q = c.prepare("SELECT id,created_at,metadata FROM transcription_runs WHERE recording_id=?1 ORDER BY rowid DESC")?;
        let rows = q
            .query_map([recording], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|(id,date,meta)| Ok(serde_json::json!({"id":id,"created_at":date,"metadata":serde_json::from_str::<serde_json::Value>(&meta)?}))).collect()
    }
    pub fn edit(&self, id: i64, text: &str) -> Result<()> {
        if text.len() > 100_000 {
            bail!("Textsegmentet är för långt");
        }
        let n = self.connect()?.execute(
            "UPDATE segments SET edited_text=?1 WHERE id=?2",
            params![text, id],
        )?;
        if n == 0 {
            bail!("Textsegmentet saknas");
        }
        Ok(())
    }
    pub fn recording_path(&self, id: &str) -> Result<PathBuf> {
        let path: String = self
            .connect()?
            .query_row(
                "SELECT archive_path FROM recordings WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .context("Inspelningen saknas")?;
        Ok(PathBuf::from(path))
    }
    pub fn search(&self, query: &str) -> Result<Vec<String>> {
        if query.len() > 1000 {
            bail!("Sökningen är för lång");
        }
        let query = query.to_lowercase();
        let c = self.connect()?;
        let mut q=c.prepare("SELECT r.id,r.name,COALESCE(s.edited_text,s.text,'') FROM recordings r LEFT JOIN segments s ON s.run_id=(SELECT id FROM transcription_runs WHERE recording_id=r.id ORDER BY rowid DESC LIMIT 1)")?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut ids = std::collections::BTreeSet::new();
        for (id, name, text) in rows {
            if name.to_lowercase().contains(&query) || text.to_lowercase().contains(&query) {
                ids.insert(id);
            }
        }
        Ok(ids.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::import_file;
    fn fixture() -> Result<(tempfile::TempDir, Database, String)> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("data");
        let db = Database::open(&root)?;
        let audio = temp.path().join("a.wav");
        std::fs::write(&audio, b"test audio")?;
        import_file(&db, &root, &audio, None)?;
        let id = db.recordings()?[0].id.clone();
        Ok((temp, db, id))
    }
    #[test]
    fn visible_queue_matches_claim_order_and_retry_preserves_pause() -> Result<()> {
        let (temp, db, first) = fixture()?;
        let root = temp.path().join("data");
        let audio = temp.path().join("b.wav");
        std::fs::write(&audio, b"different audio")?;
        import_file(&db, &root, &audio, None)?;
        let queue = db.queue()?;
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0], first);
        let job = db.claim()?.unwrap();
        assert_eq!(job.recording_id, queue[0]);
        assert_eq!(db.queue()?, queue);
        db.failed(&job.id, "test error")?;
        assert_eq!(db.queue()?, vec![queue[1].clone()]);
        db.retry(&first)?;
        db.retry(&first)?;
        assert_eq!(db.queue()?, queue);
        assert!(!db.settings(&root)?.transcription_enabled);
        assert_eq!(db.claim()?.unwrap().recording_id, first);
        Ok(())
    }
    #[test]
    fn crashed_job_requeues_and_successful_result_is_atomic() -> Result<()> {
        let (temp, db, id) = fixture()?;
        assert!(db.claim()?.is_some());
        let db = Database::open(&temp.path().join("data"))?;
        assert_eq!(db.recordings()?[0].status, "queued");
        let job = db.claim()?.unwrap();
        let invalid = Segment {
            id: 0,
            start: 3.0,
            end: 1.0,
            text: "bad".into(),
            edited_text: None,
            words: vec![],
        };
        assert!(db
            .complete(&job, &serde_json::json!({}), &[invalid])
            .is_err());
        assert_eq!(db.transcript(&id, None)?.run_id, None);
        assert_eq!(db.recordings()?[0].status, "running");
        db.failed(&job.id, "GPU error")?;
        assert_eq!(db.recordings()?[0].status, "failed");
        db.retry(&id)?;
        assert!(db.claim()?.is_some());
        assert_eq!(db.recordings()?[0].attempts, 3);
        Ok(())
    }
    #[test]
    fn corrected_text_survives_new_transcription_and_search_is_swedish() -> Result<()> {
        let (_temp, db, id) = fixture()?;
        let segment = Segment {
            id: 0,
            start: 0.0,
            end: 2.5,
            text: "Jag tänker".into(),
            edited_text: None,
            words: vec![Word {
                start: 0.0,
                end: 0.5,
                word: "Jag".into(),
                probability: 0.9,
            }],
        };
        let job = db.claim()?.unwrap();
        db.complete(
            &job,
            &serde_json::json!({"duration":2.5}),
            std::slice::from_ref(&segment),
        )?;
        let original = db.transcript(&id, None)?;
        db.edit(original.segments[0].id, "Önskar åka till Göteborg")?;
        assert_eq!(db.search("önskar")?, vec![id.clone()]);
        db.retry(&id)?;
        let job = db.claim()?.unwrap();
        db.complete(&job, &serde_json::json!({}), &[segment])?;
        let old = db.transcript(&id, original.run_id.as_deref())?;
        assert_eq!(
            old.segments[0].edited_text.as_deref(),
            Some("Önskar åka till Göteborg")
        );
        assert_eq!(old.segments[0].text, "Jag tänker");
        assert_eq!(old.segments[0].words[0].word, "Jag");
        assert_ne!(db.transcript(&id, None)?.run_id, old.run_id);
        assert_eq!(db.runs(&id)?.len(), 2);
        Ok(())
    }
    #[test]
    fn missing_archive_cannot_be_claimed() -> Result<()> {
        let (temp, db, id) = fixture()?;
        std::fs::remove_file(db.recording_path(&id)?)?;
        db.reconcile_missing()?;
        assert_eq!(db.recordings()?[0].status, "failed");
        assert!(db.claim()?.is_none());
        assert!(db.retry(&id).is_err());
        import_file(
            &db,
            &temp.path().join("data"),
            &temp.path().join("a.wav"),
            None,
        )?;
        assert_eq!(db.recordings()?[0].status, "queued");
        assert!(db.claim()?.is_some());
        Ok(())
    }
}
