use std::path::{Path, PathBuf};

use http_body_util::BodyExt;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::daemon_api::{ShutdownResponse, StatusResponse};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("connection refused — is the daemon running?")]
    ConnectionRefused,

    #[error("request timed out")]
    Timeout,

    #[error("server error ({0}): {1}")]
    ServerError(u16, String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("http error: {0}")]
    Http(String),
}

pub struct DaemonClient {
    socket_path: PathBuf,
}

impl DaemonClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub async fn status(&self) -> Result<StatusResponse, ClientError> {
        self.get("/status").await
    }

    pub async fn shutdown(&self) -> Result<ShutdownResponse, ClientError> {
        self.post("/shutdown").await
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        self.get(path).await
    }

    pub async fn post_json<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        let json = serde_json::to_vec(body).map_err(|e| ClientError::Http(e.to_string()))?;
        let req = hyper::Request::builder()
            .method("POST")
            .uri(format!("http://localhost{path}"))
            .header("content-type", "application/json")
            .body(http_body_util::Full::new(bytes::Bytes::from(json)))
            .map_err(|e: hyper::http::Error| ClientError::Http(e.to_string()))?;

        self.send_full(req).await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let req = hyper::Request::builder()
            .method("GET")
            .uri(format!("http://localhost{path}"))
            .body(http_body_util::Empty::<bytes::Bytes>::new())
            .map_err(|e: hyper::http::Error| ClientError::Http(e.to_string()))?;

        self.send(req).await
    }

    async fn post<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let req = hyper::Request::builder()
            .method("POST")
            .uri(format!("http://localhost{path}"))
            .body(http_body_util::Empty::<bytes::Bytes>::new())
            .map_err(|e: hyper::http::Error| ClientError::Http(e.to_string()))?;

        self.send(req).await
    }

    pub(crate) async fn send<T: DeserializeOwned>(
        &self,
        req: hyper::Request<http_body_util::Empty<bytes::Bytes>>,
    ) -> Result<T, ClientError> {
        let stream = tokio::net::UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::ConnectionRefused
                    || e.kind() == std::io::ErrorKind::NotFound
                {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Io(e)
                }
            })?;

        let io = hyper_util::rt::TokioIo::new(stream);

        let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
            .map_err(|e| ClientError::Http(e.to_string()))?;

        tokio::spawn(conn);

        let resp = sender
            .send_request(req)
            .await
            .map_err(|e| ClientError::Http(e.to_string()))?;

        let status = resp.status().as_u16();

        let body_bytes = resp
            .into_body()
            .collect()
            .await
            .map_err(|e| ClientError::Http(e.to_string()))?
            .to_bytes();

        if status >= 400 {
            let msg = String::from_utf8_lossy(&body_bytes).into_owned();
            return Err(ClientError::ServerError(status, msg));
        }

        serde_json::from_slice(&body_bytes).map_err(|e| ClientError::Http(e.to_string()))
    }

    async fn send_full<T: DeserializeOwned>(
        &self,
        req: hyper::Request<http_body_util::Full<bytes::Bytes>>,
    ) -> Result<T, ClientError> {
        let stream = tokio::net::UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::ConnectionRefused
                    || e.kind() == std::io::ErrorKind::NotFound
                {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Io(e)
                }
            })?;

        let io = hyper_util::rt::TokioIo::new(stream);

        let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
            .map_err(|e| ClientError::Http(e.to_string()))?;

        tokio::spawn(conn);

        let resp = sender
            .send_request(req)
            .await
            .map_err(|e| ClientError::Http(e.to_string()))?;

        let status = resp.status().as_u16();

        let body_bytes = resp
            .into_body()
            .collect()
            .await
            .map_err(|e| ClientError::Http(e.to_string()))?
            .to_bytes();

        if status >= 400 {
            let msg = String::from_utf8_lossy(&body_bytes).into_owned();
            return Err(ClientError::ServerError(status, msg));
        }

        serde_json::from_slice(&body_bytes).map_err(|e| ClientError::Http(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;
    use crate::daemon::Daemon;
    use crate::daemon_api::daemon_router;
    use crate::server::NousServer;
    use nous_core::embed::MockEmbedding;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nous-client-test-{}-{}", name, std::process::id()))
    }

    fn test_db_path() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!(
            "/tmp/nous-client-test-{}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            seq,
        )
    }

    fn test_server(db_path: &str) -> std::sync::Arc<NousServer> {
        let mut cfg = crate::config::Config::default();
        cfg.encryption.db_key_file = format!("{db_path}.key");
        let embedding = Box::new(MockEmbedding::new(384));
        std::sync::Arc::new(NousServer::new(cfg, embedding, db_path).unwrap())
    }

    fn test_config(dir: &std::path::Path) -> DaemonConfig {
        DaemonConfig {
            socket_path: dir.join("daemon.sock").to_string_lossy().into_owned(),
            pid_file: dir.join("daemon.pid").to_string_lossy().into_owned(),
            log_file: dir.join("daemon.log").to_string_lossy().into_owned(),
            mcp_transport: "stdio".into(),
            mcp_port: 8377,
            shutdown_timeout_secs: 5,
        }
    }

    #[tokio::test]
    async fn status_returns_pid_and_version() {
        let dir = temp_dir("status");
        let _ = fs::remove_dir_all(&dir);
        let db_path = test_db_path();
        let cfg = test_config(&dir);
        let socket_path = cfg.socket_path.clone();

        let daemon = Daemon::new(&cfg).unwrap();
        let server = test_server(&db_path);
        let router = daemon_router(daemon.shutdown_sender(), server);
        let handle = tokio::spawn(daemon.run(router));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let client = DaemonClient::new(&socket_path);
        let status = client.status().await.unwrap();

        assert_eq!(status.pid, std::process::id());
        assert!(!status.version.is_empty());

        client.shutdown().await.unwrap();
        let _ = handle.await;
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn shutdown_stops_daemon() {
        let dir = temp_dir("shutdown");
        let _ = fs::remove_dir_all(&dir);
        let db_path = test_db_path();
        let cfg = test_config(&dir);
        let socket_path = cfg.socket_path.clone();

        let daemon = Daemon::new(&cfg).unwrap();
        let server = test_server(&db_path);
        let router = daemon_router(daemon.shutdown_sender(), server);
        let handle = tokio::spawn(daemon.run(router));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let client = DaemonClient::new(&socket_path);
        let resp = client.shutdown().await.unwrap();
        assert!(resp.ok);

        let result = handle.await.unwrap();
        assert!(result.is_ok());

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn connection_refused_on_missing_socket() {
        let client = DaemonClient::new("/tmp/nous-nonexistent-socket.sock");
        let err = client.status().await.unwrap_err();
        assert!(
            matches!(err, ClientError::ConnectionRefused),
            "expected ConnectionRefused, got: {err}"
        );
    }

    #[tokio::test]
    async fn rooms_endpoint_rejects_empty_body() {
        let dir = temp_dir("rooms-empty");
        let _ = fs::remove_dir_all(&dir);
        let db_path = test_db_path();
        let cfg = test_config(&dir);
        let socket_path = cfg.socket_path.clone();

        let daemon = Daemon::new(&cfg).unwrap();
        let server = test_server(&db_path);
        let router = daemon_router(daemon.shutdown_sender(), server);
        let handle = tokio::spawn(daemon.run(router));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let client = DaemonClient::new(&socket_path);

        let req = hyper::Request::builder()
            .method("POST")
            .uri("http://localhost/rooms")
            .body(http_body_util::Empty::<bytes::Bytes>::new())
            .unwrap();

        let err = client.send::<serde_json::Value>(req).await.unwrap_err();

        assert!(
            matches!(err, ClientError::ServerError(status, _) if status >= 400),
            "expected client/server error, got: {err}"
        );

        client.shutdown().await.unwrap();
        let _ = handle.await;
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_file(&db_path);
    }
}
