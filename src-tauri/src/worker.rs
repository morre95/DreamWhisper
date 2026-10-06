use crate::{
    db::Job,
    types::{Segment, Settings},
};
use anyhow::{bail, Context, Result};
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

pub struct Worker {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Result<serde_json::Value, String>>,
}
impl Worker {
    pub fn spawn(
        settings: &Settings,
        script: &Path,
        root: &Path,
        stop: &AtomicBool,
    ) -> Result<Self> {
        let logs = root.join("logs");
        std::fs::create_dir_all(&logs)?;
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs.join("worker.log"))?;
        let mut cmd = Command::new(&settings.python_path);
        cmd.arg("-u")
            .arg(script)
            .arg("--model")
            .arg(&settings.model_path)
            .arg("--batch-size")
            .arg(settings.batch_size.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log));
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            let parent = std::process::id();
            // Kill the Python worker even when the desktop process crashes.
            unsafe {
                cmd.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::getppid() as u32 != parent {
                        return Err(std::io::Error::other("Parent exited"));
                    }
                    Ok(())
                });
            }
        }
        let mut child = cmd
            .spawn()
            .context("Python kunde inte startas. Kontrollera sökvägen till .venv/bin/python")?;
        let stdin = child.stdin.take().context("Worker stdin saknas")?;
        let stdout = child.stdout.take().context("Worker stdout saknas")?;
        let (tx, messages) = mpsc::sync_channel(128);
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let parsed = match line {
                    Ok(line) if line.len() <= 64 * 1024 * 1024 => serde_json::from_str(&line)
                        .map_err(|e| format!("Ogiltigt worker-meddelande: {e}")),
                    Ok(_) => Err("Worker-meddelandet är för stort".into()),
                    Err(e) => Err(e.to_string()),
                };
                if tx.send(parsed).is_err() {
                    break;
                }
            }
        });
        let worker = Self {
            child,
            stdin,
            messages,
        };
        let started = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                bail!("Appen stängs");
            }
            if started.elapsed() > Duration::from_secs(180) {
                bail!("Modellen kunde inte laddas inom tre minuter; se logs/worker.log");
            }
            match worker.messages.recv_timeout(Duration::from_secs(1)) {
                Ok(Ok(value)) if value["type"] == "ready" => return Ok(worker),
                Ok(Ok(value)) => bail!(
                    "{}",
                    value["error"]
                        .as_str()
                        .unwrap_or("Workern kunde inte initieras")
                ),
                Ok(Err(e)) => bail!("{e}"),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => bail!("Python avslutades vid modellinläsning; se logs/worker.log"),
            }
        }
    }
    pub fn transcribe(
        &mut self,
        job: &Job,
        stop: &AtomicBool,
        mut progress: impl FnMut(String),
    ) -> Result<(serde_json::Value, Vec<Segment>)> {
        writeln!(
            self.stdin,
            "{}",
            serde_json::json!({"type":"transcribe","job_id":job.id,"path":job.path})
        )?;
        self.stdin.flush()?;
        let started = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                bail!("Avbruten vid avstängning");
            }
            if started.elapsed() > Duration::from_secs(12 * 3600) {
                bail!("Transkriberingen överskred tolv timmar");
            }
            let value = match self.messages.recv_timeout(Duration::from_secs(1)) {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => bail!("{e}"),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => bail!("Python avslutades oväntat; se logs/worker.log"),
            };
            if value["job_id"].as_str() != Some(&job.id) {
                bail!("Workern svarade med fel jobb-ID");
            }
            match value["type"].as_str() {
                Some("progress") => {
                    let msg = value["message"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            format!(
                                "Transkriberar: {:.0} / {:.0} sekunder",
                                value["seconds"].as_f64().unwrap_or(0.0),
                                value["duration"].as_f64().unwrap_or(0.0)
                            )
                        });
                    progress(msg);
                }
                Some("completed") => {
                    return Ok((
                        value["metadata"].clone(),
                        serde_json::from_value(value["segments"].clone())?,
                    ))
                }
                Some("error") => bail!(
                    "{}",
                    value["error"]
                        .as_str()
                        .unwrap_or("Transkribering misslyckades")
                ),
                _ => bail!("Okänd typ av worker-meddelande"),
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
