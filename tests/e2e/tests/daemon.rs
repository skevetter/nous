use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nous_core::embed::MockEmbedding;
use nous_mcp::config::{Config, DaemonConfig};
use nous_mcp::daemon::Daemon;
use nous_mcp::daemon_api::daemon_router;
use nous_mcp::daemon_client::DaemonClient;
use nous_mcp::server::NousServer;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_id() -> u64 {
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn test_base_dir() -> PathBuf {
    let dir = home_dir().join(".cache").join("nous-test");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
}

fn test_db_path() -> String {
    test_base_dir()
        .join(format!(
            "nous-e2e-daemon-{}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            unique_id(),
        ))
        .to_string_lossy()
        .into_owned()
}

struct DaemonTestEnv {
    dir: PathBuf,
    db_path: String,
    config: DaemonConfig,
}

impl DaemonTestEnv {
    fn new(name: &str) -> Self {
        let seq = unique_id();
        let dir = test_base_dir().join(format!("d-{}-{}-{}", name, std::process::id(), seq));
        let _ = std::fs::remove_dir_all(&dir);

        let db_path = test_db_path();

        let config = DaemonConfig {
            socket_path: dir.join("daemon.sock").to_string_lossy().into_owned(),
            pid_file: dir.join("daemon.pid").to_string_lossy().into_owned(),
            log_file: dir.join("daemon.log").to_string_lossy().into_owned(),
            mcp_transport: "stdio".into(),
            mcp_port: 8377,
            shutdown_timeout_secs: 5,
        };

        Self {
            dir,
            db_path,
            config,
        }
    }

    fn server(&self) -> Arc<NousServer> {
        let cfg = Config::default();
        let embedding = Box::new(MockEmbedding::new(384));
        Arc::new(NousServer::new(cfg, embedding, &self.db_path, None).unwrap())
    }

    fn client(&self) -> DaemonClient {
        DaemonClient::new(&self.config.socket_path)
    }

    fn pid_file_path(&self) -> &Path {
        Path::new(&self.config.pid_file)
    }

    fn socket_path(&self) -> &Path {
        Path::new(&self.config.socket_path)
    }
}

impl Drop for DaemonTestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_file(&self.db_path);
    }
}

async fn wait_ready() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

// ---------------------------------------------------------------------------
// 1. daemon_start_stop_lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn daemon_start_stop_lifecycle() {
    let env = DaemonTestEnv::new("lifecycle");
    let server = env.server();
    let daemon = Daemon::new(&env.config).unwrap();
    let router = daemon_router(daemon.shutdown_sender(), server);
    let handle = tokio::spawn(daemon.run(router));

    wait_ready().await;

    let client = env.client();
    let status = client.status().await.unwrap();
    assert_eq!(status.pid, std::process::id());
    assert!(!status.version.is_empty());
    assert!(status.uptime_secs < 5);

    let resp = client.shutdown().await.unwrap();
    assert!(resp.ok);

    let result = handle.await.unwrap();
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// 2. pid_file_management
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pid_file_management() {
    let env = DaemonTestEnv::new("pid-mgmt");
    let server = env.server();
    let daemon = Daemon::new(&env.config).unwrap();

    let pid_path = env.pid_file_path().to_path_buf();
    assert!(pid_path.exists(), "PID file should exist after Daemon::new");

    let content = std::fs::read_to_string(&pid_path).unwrap();
    assert_eq!(content.trim().parse::<u32>().unwrap(), std::process::id(),);

    let router = daemon_router(daemon.shutdown_sender(), server);
    let handle = tokio::spawn(daemon.run(router));

    wait_ready().await;

    let client = env.client();
    client.shutdown().await.unwrap();
    let _ = handle.await;

    assert!(
        !pid_path.exists(),
        "PID file should be removed after shutdown"
    );
}

// ---------------------------------------------------------------------------
// 3. stale_pid_cleanup
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stale_pid_cleanup() {
    let env = DaemonTestEnv::new("stale-pid");

    let pid_path = env.pid_file_path().to_path_buf();
    if let Some(parent) = pid_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&pid_path, "999999999").unwrap();

    let server = env.server();
    let daemon = Daemon::new(&env.config).unwrap();

    let content = std::fs::read_to_string(&pid_path).unwrap();
    assert_eq!(
        content.trim().parse::<u32>().unwrap(),
        std::process::id(),
        "stale PID should be replaced with current PID"
    );

    let router = daemon_router(daemon.shutdown_sender(), server);
    let handle = tokio::spawn(daemon.run(router));

    wait_ready().await;

    let client = env.client();
    let status = client.status().await.unwrap();
    assert_eq!(status.pid, std::process::id());

    client.shutdown().await.unwrap();
    let _ = handle.await;
}

// ---------------------------------------------------------------------------
// 4. socket_ipc_roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn socket_ipc_roundtrip() {
    let env = DaemonTestEnv::new("ipc-roundtrip");
    let server = env.server();
    let daemon = Daemon::new(&env.config).unwrap();
    let router = daemon_router(daemon.shutdown_sender(), server);
    let handle = tokio::spawn(daemon.run(router));

    wait_ready().await;

    let client = env.client();

    // /status
    let status = client.status().await.unwrap();
    assert_eq!(status.pid, std::process::id());

    // POST /rooms — create
    let create_resp: serde_json::Value = client
        .post_json(
            "/rooms",
            &serde_json::json!({"name": "ipc-room", "purpose": "e2e ipc test"}),
        )
        .await
        .unwrap();
    let room_id = create_resp["id"].as_str().unwrap().to_string();
    assert_eq!(create_resp["name"], "ipc-room");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // GET /rooms — list
    let list_resp: serde_json::Value = client.get_json("/rooms").await.unwrap();
    let rooms = list_resp["rooms"].as_array().unwrap();
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0]["name"], "ipc-room");

    // GET /rooms/{id} — get by id
    let get_resp: serde_json::Value = client.get_json(&format!("/rooms/{room_id}")).await.unwrap();
    assert_eq!(get_resp["name"], "ipc-room");
    assert_eq!(get_resp["purpose"], "e2e ipc test");

    client.shutdown().await.unwrap();
    let _ = handle.await;
}

// ---------------------------------------------------------------------------
// 5. mcp_tool_dispatch_through_daemon
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mcp_tool_dispatch_through_daemon() {
    let env = DaemonTestEnv::new("tool-dispatch");
    let server = env.server();
    let daemon = Daemon::new(&env.config).unwrap();
    let router = daemon_router(daemon.shutdown_sender(), server);
    let handle = tokio::spawn(daemon.run(router));

    wait_ready().await;

    let client = env.client();

    // Create a room
    let create_resp: serde_json::Value = client
        .post_json("/rooms", &serde_json::json!({"name": "dispatch-room"}))
        .await
        .unwrap();
    let room_id = create_resp["id"].as_str().unwrap().to_string();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Post a message
    let msg_resp: serde_json::Value = client
        .post_json(
            &format!("/rooms/{room_id}/messages"),
            &serde_json::json!({
                "content": "hello from e2e test",
                "sender": "test-agent"
            }),
        )
        .await
        .unwrap();
    assert!(msg_resp["id"].as_str().is_some());
    assert_eq!(msg_resp["sender_id"], "test-agent");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Read messages back
    let read_resp: serde_json::Value = client
        .get_json(&format!("/rooms/{room_id}/messages"))
        .await
        .unwrap();
    let messages = read_resp["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "hello from e2e test");
    assert_eq!(messages[0]["sender_id"], "test-agent");

    client.shutdown().await.unwrap();
    let _ = handle.await;
}

// ---------------------------------------------------------------------------
// 6. concurrent_clients
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_clients() {
    let env = DaemonTestEnv::new("concurrent");
    let server = env.server();
    let daemon = Daemon::new(&env.config).unwrap();
    let router = daemon_router(daemon.shutdown_sender(), server);
    let handle = tokio::spawn(daemon.run(router));

    wait_ready().await;

    let socket_path = PathBuf::from(&env.config.socket_path);

    let mut tasks = Vec::new();
    for i in 0..5u32 {
        let sp = socket_path.clone();
        tasks.push(tokio::spawn(async move {
            let client = DaemonClient::new(&sp);
            let status = client.status().await.unwrap();
            assert_eq!(status.pid, std::process::id());
            assert!(!status.version.is_empty());
            i
        }));
    }

    let mut completed = Vec::new();
    for task in tasks {
        completed.push(task.await.unwrap());
    }
    completed.sort();
    assert_eq!(completed, vec![0, 1, 2, 3, 4]);

    let client = env.client();
    client.shutdown().await.unwrap();
    let _ = handle.await;
}

// ---------------------------------------------------------------------------
// 7. signal_shutdown
// ---------------------------------------------------------------------------

#[tokio::test]
async fn signal_shutdown() {
    let env = DaemonTestEnv::new("signal");
    let server = env.server();
    let daemon = Daemon::new(&env.config).unwrap();
    let router = daemon_router(daemon.shutdown_sender(), server);
    let handle = tokio::spawn(daemon.run(router));

    wait_ready().await;

    let client = env.client();
    let resp = client.shutdown().await.unwrap();
    assert!(resp.ok);

    let result = handle.await.unwrap();
    assert!(result.is_ok(), "daemon should exit cleanly after shutdown");
}

// ---------------------------------------------------------------------------
// 8. restart_lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn restart_lifecycle() {
    let env = DaemonTestEnv::new("restart");

    // First start
    let server1 = env.server();
    let daemon1 = Daemon::new(&env.config).unwrap();
    let router1 = daemon_router(daemon1.shutdown_sender(), server1);
    let handle1 = tokio::spawn(daemon1.run(router1));

    wait_ready().await;

    let client1 = env.client();
    let status1 = client1.status().await.unwrap();
    assert_eq!(status1.pid, std::process::id());

    client1.shutdown().await.unwrap();
    let result1 = handle1.await.unwrap();
    assert!(result1.is_ok());

    assert!(
        !env.socket_path().exists(),
        "socket should be removed after shutdown"
    );

    // Second start on same paths
    let server2 = env.server();
    let daemon2 = Daemon::new(&env.config).unwrap();
    let router2 = daemon_router(daemon2.shutdown_sender(), server2);
    let handle2 = tokio::spawn(daemon2.run(router2));

    wait_ready().await;

    let client2 = env.client();
    let status2 = client2.status().await.unwrap();
    assert_eq!(status2.pid, std::process::id());
    assert!(!status2.version.is_empty());

    client2.shutdown().await.unwrap();
    let result2 = handle2.await.unwrap();
    assert!(result2.is_ok());
}
