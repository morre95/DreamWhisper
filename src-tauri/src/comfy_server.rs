use crate::images::{test_connection, validate_url, ComfySettings};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    fs::OpenOptions,
    net::{IpAddr, SocketAddr, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, Serialize)]
pub struct ServerStatus {
    pub status: String,
    pub error: Option<String>,
    pub url: String,
    pub log_path: String,
}
#[derive(Default)]
pub struct ComfyServer {
    child: Option<Child>,
    status: ServerStatus,
}
impl ComfyServer {
    pub fn snapshot(&mut self) -> ServerStatus {
        if let Some(child) = self.child.as_mut() {
            if let Ok(Some(exit)) = child.try_wait() {
                self.child = None;
                self.status.status = "failed".into();
                self.status.error = Some(format!(
                    "ComfyUI avslutades ({exit}). Se {}",
                    self.status.log_path
                ));
            }
        }
        self.status.clone()
    }
    pub fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.status.status = "stopped".into();
    }
    pub fn start(
        manager: &Arc<Mutex<Self>>,
        config: &ComfySettings,
        root: &Path,
    ) -> Result<ServerStatus> {
        validate_url(&config.url)?;
        let url = reqwest::Url::parse(config.url.trim())?;
        let host = match url.host_str().unwrap_or("127.0.0.1") {
            "localhost" => "127.0.0.1",
            "[::1]" => "::1",
            other => other,
        };
        let port = url.port_or_known_default().unwrap_or(8188);
        let mut server = manager.lock().unwrap();
        server.snapshot();
        if server.child.is_some() {
            if server.status.url.trim_end_matches('/') != config.url.trim().trim_end_matches('/') {
                bail!(
                    "En server startad av DreamWhisper kör redan på {}.",
                    server.status.url
                );
            }
            return Ok(server.status.clone());
        }
        if test_connection(&config.url).is_ok() {
            server.status = ServerStatus {
                status: "external".into(),
                url: config.url.clone(),
                ..Default::default()
            };
            return Ok(server.status.clone());
        }
        if TcpStream::connect_timeout(
            &SocketAddr::new(host.parse::<IpAddr>()?, port),
            Duration::from_millis(500),
        )
        .is_ok()
        {
            bail!("Port {port} används redan, men ComfyUI svarar inte. Kontrollera den befintliga servern.");
        }
        let directory = Path::new(&config.comfy_directory);
        let python = Path::new(&config.comfy_python_path);
        if !directory.is_absolute() || !directory.join("main.py").is_file() {
            bail!("Ange den fullständiga sökvägen till ComfyUI-mappen med main.py.");
        }
        if !python.is_absolute() || !python.is_file() {
            bail!("Ange ComfyUI:s Pythonprogram, exempelvis dess .venv/bin/python.");
        }
        let logs = root.join("logs");
        std::fs::create_dir_all(&logs)?;
        let log_path = logs.join("comfyui.log");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        let mut command = Command::new(python);
        command
            .current_dir(directory)
            .args([
                "-u",
                "main.py",
                "--listen",
                host,
                "--port",
                &port.to_string(),
                "--disable-auto-launch",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            let parent = std::process::id();
            unsafe {
                command.pre_exec(move || {
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
        server.child = Some(command.spawn().context("ComfyUI kunde inte startas")?);
        server.status = ServerStatus {
            status: "starting".into(),
            error: None,
            url: config.url.clone(),
            log_path: log_path.to_string_lossy().into(),
        };
        let initial = server.status.clone();
        drop(server);
        let weak = Arc::downgrade(manager);
        let address = config.url.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            loop {
                let Some(manager) = weak.upgrade() else {
                    return;
                };
                {
                    let mut server = manager.lock().unwrap();
                    server.snapshot();
                    if server.child.is_none() {
                        return;
                    }
                }
                let ready = test_connection(&address).is_ok();
                let mut server = manager.lock().unwrap();
                if server.child.is_none() {
                    return;
                }
                if ready {
                    server.status.status = "running".into();
                    server.status.error = None;
                    return;
                }
                if started.elapsed() > Duration::from_secs(120) {
                    server.shutdown();
                    server.status.status = "failed".into();
                    server.status.error = Some(format!(
                        "ComfyUI svarade inte inom två minuter. Se {}",
                        server.status.log_path
                    ));
                    return;
                }
                drop(server);
                drop(manager);
                std::thread::sleep(Duration::from_secs(1));
            }
        });
        Ok(initial)
    }
}
impl Drop for ComfyServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    fn free_url() -> Result<String> {
        let socket = TcpListener::bind("127.0.0.1:0")?;
        Ok(format!("http://{}", socket.local_addr()?))
    }
    fn wait(manager: &Arc<Mutex<ComfyServer>>, expected: &str) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while manager.lock().unwrap().snapshot().status != expected {
            if Instant::now() > deadline {
                bail!("Timed out waiting for {expected}");
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    }
    #[test]
    fn starts_once_checks_readiness_and_never_stops_an_external_server() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let directory = temp.path().join("fake_comfy");
        std::fs::create_dir_all(&directory)?;
        std::fs::write(
            directory.join("main.py"),
            r#"import argparse, json, sys
from pathlib import Path
from http.server import HTTPServer, BaseHTTPRequestHandler
parser=argparse.ArgumentParser()
parser.add_argument('--listen'); parser.add_argument('--port',type=int); parser.add_argument('--disable-auto-launch',action='store_true')
args=parser.parse_args()
Path('launch.json').write_text(json.dumps(sys.argv))
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'{"system":{},"devices":[]}')
HTTPServer((args.listen,args.port),Handler).serve_forever()
"#,
        )?;
        let config = ComfySettings {
            url: free_url()?,
            comfy_directory: directory.to_string_lossy().into(),
            comfy_python_path: "/usr/bin/python3".into(),
            ..Default::default()
        };
        let owned = Arc::new(Mutex::new(ComfyServer::default()));
        assert_eq!(
            ComfyServer::start(&owned, &config, temp.path())?.status,
            "starting"
        );
        wait(&owned, "running")?;
        let pid = owned.lock().unwrap().child.as_ref().unwrap().id();
        assert_eq!(
            ComfyServer::start(&owned, &config, temp.path())?.status,
            "running"
        );
        assert_eq!(owned.lock().unwrap().child.as_ref().unwrap().id(), pid);
        let args: Vec<String> =
            serde_json::from_str(&std::fs::read_to_string(directory.join("launch.json"))?)?;
        assert!(args.iter().any(|a| a == "--disable-auto-launch"));
        let external = Arc::new(Mutex::new(ComfyServer::default()));
        assert_eq!(
            ComfyServer::start(&external, &config, temp.path())?.status,
            "external"
        );
        external.lock().unwrap().shutdown();
        test_connection(&config.url)?;
        owned.lock().unwrap().shutdown();
        assert!(test_connection(&config.url).is_err());
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        Ok(())
    }
    #[test]
    fn startup_failure_is_reported_with_a_log_path() -> Result<()> {
        let temp = tempfile::tempdir()?;
        std::fs::write(
            temp.path().join("main.py"),
            "raise RuntimeError('fake missing dependency')",
        )?;
        let config = ComfySettings {
            url: free_url()?,
            comfy_directory: temp.path().to_string_lossy().into(),
            comfy_python_path: "/usr/bin/python3".into(),
            ..Default::default()
        };
        let server = Arc::new(Mutex::new(ComfyServer::default()));
        ComfyServer::start(&server, &config, temp.path())?;
        wait(&server, "failed")?;
        let status = server.lock().unwrap().snapshot();
        assert!(status.error.as_deref().unwrap().contains("comfyui.log"));
        assert!(std::fs::read_to_string(status.log_path)?.contains("fake missing dependency"));
        Ok(())
    }
}
