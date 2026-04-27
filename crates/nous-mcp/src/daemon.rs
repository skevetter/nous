use std::fs;
use std::path::{Path, PathBuf};

use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;

use crate::config::DaemonConfig;

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("daemon already running (PID {0})")]
    AlreadyRunning(u32),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, DaemonError>;

#[derive(Debug)]
pub struct Daemon {
    pid_file: PathBuf,
    socket_path: PathBuf,
    listener: UnixListener,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    shutdown_timeout_secs: u64,
}

impl Daemon {
    pub fn new(config: &DaemonConfig) -> Result<Self> {
        let pid_file = PathBuf::from(&config.pid_file);
        let socket_path = PathBuf::from(&config.socket_path);

        Self::check_stale_pid(&pid_file)?;

        if let Some(parent) = pid_file.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let pid = std::process::id();
        fs::write(&pid_file, pid.to_string())?;

        if socket_path.exists() {
            fs::remove_file(&socket_path)?;
        }
        let listener = UnixListener::bind(&socket_path)?;

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Ok(Self {
            pid_file,
            socket_path,
            listener,
            shutdown_tx,
            shutdown_rx,
            shutdown_timeout_secs: config.shutdown_timeout_secs,
        })
    }

    fn check_stale_pid(pid_file: &Path) -> Result<()> {
        let content = match fs::read_to_string(pid_file) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        let pid: u32 = match content.trim().parse() {
            Ok(p) => p,
            Err(_) => {
                fs::remove_file(pid_file)?;
                return Ok(());
            }
        };

        if process_alive(pid) {
            return Err(DaemonError::AlreadyRunning(pid));
        }

        fs::remove_file(pid_file)?;
        Ok(())
    }

    pub fn listener(&self) -> &UnixListener {
        &self.listener
    }

    pub fn shutdown_receiver(&self) -> watch::Receiver<bool> {
        self.shutdown_rx.clone()
    }

    pub fn shutdown_timeout_secs(&self) -> u64 {
        self.shutdown_timeout_secs
    }

    pub async fn install_signal_handlers(&self) {
        let tx = self.shutdown_tx.clone();

        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
        let mut sighup = signal(SignalKind::hangup()).expect("failed to install SIGHUP handler");

        tokio::spawn(async move {
            tokio::select! {
                _ = sigterm.recv() => {
                    let _ = tx.send(true);
                }
                _ = sigint.recv() => {
                    let _ = tx.send(true);
                }
                _ = sighup.recv() => {
                    // no-op placeholder for future reload
                }
            }
        });
    }

    pub fn shutdown(&self) -> Result<()> {
        let _ = self.shutdown_tx.send(true);

        if self.pid_file.exists() {
            fs::remove_file(&self.pid_file)?;
        }

        if self.socket_path.exists() {
            fs::remove_file(&self.socket_path)?;
        }

        Ok(())
    }

    pub fn pid_file_path(&self) -> &Path {
        &self.pid_file
    }

    pub fn socket_file_path(&self) -> &Path {
        &self.socket_path
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.pid_file);
        let _ = fs::remove_file(&self.socket_path);
    }
}

fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nous-daemon-test-{}-{}", name, std::process::id()))
    }

    fn test_config(dir: &Path) -> DaemonConfig {
        DaemonConfig {
            socket_path: dir.join("daemon.sock").to_string_lossy().into_owned(),
            pid_file: dir.join("daemon.pid").to_string_lossy().into_owned(),
            log_file: dir.join("daemon.log").to_string_lossy().into_owned(),
            mcp_transport: "stdio".into(),
            mcp_port: 8377,
            shutdown_timeout_secs: 30,
        }
    }

    #[test]
    fn daemon_config_defaults() {
        let cfg = DaemonConfig::default();
        assert_eq!(cfg.socket_path, "~/.cache/nous/daemon.sock");
        assert_eq!(cfg.pid_file, "~/.cache/nous/daemon.pid");
        assert_eq!(cfg.log_file, "~/.cache/nous/daemon.log");
        assert_eq!(cfg.mcp_transport, "stdio");
        assert_eq!(cfg.mcp_port, 8377);
        assert_eq!(cfg.shutdown_timeout_secs, 30);
    }

    #[tokio::test]
    async fn pid_file_lifecycle() {
        let dir = temp_dir("pid-lifecycle");
        let _ = fs::remove_dir_all(&dir);
        let cfg = test_config(&dir);

        let daemon = Daemon::new(&cfg).unwrap();
        let pid_path = daemon.pid_file_path().to_path_buf();

        assert!(pid_path.exists());
        let content = fs::read_to_string(&pid_path).unwrap();
        assert_eq!(content.trim().parse::<u32>().unwrap(), std::process::id());

        daemon.shutdown().unwrap();
        assert!(!pid_path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn stale_pid_cleanup() {
        let dir = temp_dir("stale-pid");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let pid_path = dir.join("daemon.pid");
        fs::write(&pid_path, "999999999").unwrap();

        let cfg = test_config(&dir);
        let daemon = Daemon::new(&cfg).unwrap();

        let content = fs::read_to_string(&pid_path).unwrap();
        assert_eq!(content.trim().parse::<u32>().unwrap(), std::process::id());

        drop(daemon);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn socket_bind_unbind() {
        let dir = temp_dir("socket-lifecycle");
        let _ = fs::remove_dir_all(&dir);
        let cfg = test_config(&dir);

        let daemon = Daemon::new(&cfg).unwrap();
        let sock_path = daemon.socket_file_path().to_path_buf();

        assert!(sock_path.exists());

        daemon.shutdown().unwrap();
        assert!(!sock_path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn double_start_rejected() {
        let dir = temp_dir("double-start");
        let _ = fs::remove_dir_all(&dir);
        let cfg = test_config(&dir);

        let _daemon = Daemon::new(&cfg).unwrap();

        let cfg2 = test_config(&dir);
        let result = Daemon::new(&cfg2);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("daemon already running"),
            "unexpected error: {err}"
        );

        drop(_daemon);
        let _ = fs::remove_dir_all(&dir);
    }
}
