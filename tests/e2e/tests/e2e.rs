use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use tempfile::TempDir;

const DB_KEY: &str = "test-e2e-key-do-not-use";

const IMPORT_JSON: &str = r#"{
  "version": 1,
  "memories": [
    {
      "id": "mem_e2etest0001",
      "title": "E2E Test Memory",
      "content": "This memory was created by the end-to-end test script.",
      "memory_type": "fact",
      "source": "e2e-test",
      "importance": "moderate",
      "confidence": "high",
      "session_id": null,
      "trace_id": null,
      "agent_id": null,
      "agent_model": null,
      "valid_from": null,
      "valid_until": null,
      "category_id": null,
      "created_at": "2026-01-01T00:00:00Z",
      "updated_at": "2026-01-01T00:00:00Z",
      "tags": ["e2e", "test"],
      "relationships": []
    }
  ],
  "categories": []
}"#;

fn bin_dir() -> PathBuf {
    let mut path = std::env::current_exe().expect("cannot resolve test binary path");
    path.pop(); // remove binary name
    // cargo test puts test binaries under target/debug/deps/
    if path.ends_with("deps") {
        path.pop();
    }
    path
}

fn nous_mcp_bin() -> PathBuf {
    let p = bin_dir().join("nous-mcp");
    assert!(p.exists(), "nous-mcp binary not found at {}", p.display());
    p
}

fn nous_otlp_bin() -> PathBuf {
    let p = bin_dir().join("nous-otlp");
    assert!(p.exists(), "nous-otlp binary not found at {}", p.display());
    p
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind ephemeral port");
    listener.local_addr().unwrap().port()
}

struct OtlpServer {
    child: std::process::Child,
    port: u16,
}

impl OtlpServer {
    fn start(port: u16, db_path: &std::path::Path) -> Self {
        let child = Command::new(nous_otlp_bin())
            .args(["serve", "--port", &port.to_string(), "--db"])
            .arg(db_path)
            .env("NOUS_DB_KEY", DB_KEY)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to start nous-otlp");

        OtlpServer { child, port }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for OtlpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn wait_for_otlp(server: &OtlpServer) {
    let client = reqwest::Client::new();
    let url = format!("{}/v1/logs", server.base_url());

    for _ in 0..60 {
        let result = client
            .post(&url)
            .header("content-type", "application/x-protobuf")
            .body(vec![])
            .send()
            .await;

        if let Ok(resp) = result {
            let status = resp.status().as_u16();
            if status == 200 || status == 400 {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("nous-otlp did not become ready within 30 seconds");
}

fn run_import(mcp_db: &std::path::Path, key_file: &std::path::Path, import_file: &std::path::Path) {
    let output = Command::new(nous_mcp_bin())
        .args(["import"])
        .arg(import_file)
        .env("NOUS_DB_KEY", DB_KEY)
        .env("NOUS_MEMORY_DB", mcp_db)
        .env("NOUS_DB_KEY_FILE", key_file)
        .output()
        .expect("failed to run nous-mcp import");

    assert!(
        output.status.success(),
        "nous-mcp import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_export(mcp_db: &std::path::Path, key_file: &std::path::Path) -> String {
    let output = Command::new(nous_mcp_bin())
        .args(["export"])
        .env("NOUS_DB_KEY", DB_KEY)
        .env("NOUS_MEMORY_DB", mcp_db)
        .env("NOUS_DB_KEY_FILE", key_file)
        .output()
        .expect("failed to run nous-mcp export");

    assert!(
        output.status.success(),
        "nous-mcp export failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("export output not valid UTF-8")
}

struct TestEnv {
    _tmp: TempDir,
    mcp_db: PathBuf,
    otlp_db: PathBuf,
    key_file: PathBuf,
    import_file: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let tmp = TempDir::new().expect("failed to create temp dir");
        let dir = tmp.path();

        let mcp_db = dir.join("memory.db");
        let otlp_db = dir.join("otlp.db");
        let key_file = dir.join("db.key");
        let import_file = dir.join("import.json");

        std::fs::write(&key_file, DB_KEY).expect("failed to write key file");
        std::fs::write(&import_file, IMPORT_JSON).expect("failed to write import file");

        TestEnv {
            _tmp: tmp,
            mcp_db,
            otlp_db,
            key_file,
            import_file,
        }
    }
}

#[tokio::test]
async fn test_import_and_export_roundtrip() {
    let env = TestEnv::new();

    run_import(&env.mcp_db, &env.key_file, &env.import_file);

    let export_output = run_export(&env.mcp_db, &env.key_file);
    assert!(
        export_output.contains("E2E Test Memory"),
        "export missing title"
    );
    assert!(
        export_output.contains("end-to-end test script"),
        "export missing content"
    );
    assert!(export_output.contains("e2e-test"), "export missing source");
    assert!(
        export_output.contains(r#""e2e""#),
        "export missing tag 'e2e'"
    );
}

#[tokio::test]
async fn test_otlp_server_health() {
    let env = TestEnv::new();
    let port = free_port();

    let server = OtlpServer::start(port, &env.otlp_db);
    wait_for_otlp(&server).await;

    assert!(
        env.otlp_db.exists(),
        "OTLP DB file should exist after server start"
    );
    assert!(
        std::fs::metadata(&env.otlp_db).unwrap().len() > 0,
        "OTLP DB file should be non-empty"
    );
}

#[tokio::test]
async fn test_otlp_endpoint_response() {
    let env = TestEnv::new();
    let port = free_port();

    let server = OtlpServer::start(port, &env.otlp_db);
    wait_for_otlp(&server).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/logs", server.base_url()))
        .header("content-type", "application/x-protobuf")
        .body(vec![])
        .send()
        .await
        .expect("POST to /v1/logs failed");

    let status = resp.status().as_u16();
    assert!(
        status == 200 || status == 400,
        "expected HTTP 200 or 400, got {status}"
    );
}

#[tokio::test]
async fn test_full_e2e_flow() {
    let env = TestEnv::new();

    // 1. Start OTLP server
    let port = free_port();
    let server = OtlpServer::start(port, &env.otlp_db);
    wait_for_otlp(&server).await;

    // 2. Import a memory
    run_import(&env.mcp_db, &env.key_file, &env.import_file);

    // 3. POST to OTLP endpoint
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/logs", server.base_url()))
        .header("content-type", "application/x-protobuf")
        .body(vec![])
        .send()
        .await
        .expect("POST to /v1/logs failed");

    let status = resp.status().as_u16();
    assert!(
        status == 200 || status == 400,
        "OTLP endpoint returned unexpected status {status}"
    );

    // 4. Verify export
    let export_output = run_export(&env.mcp_db, &env.key_file);
    assert!(export_output.contains("E2E Test Memory"));
    assert!(export_output.contains("end-to-end test script"));
    assert!(export_output.contains("e2e-test"));
    assert!(export_output.contains(r#""e2e""#));

    // 5. Verify OTLP DB exists and is non-empty
    assert!(env.otlp_db.exists(), "OTLP DB file should exist");
    assert!(
        std::fs::metadata(&env.otlp_db).unwrap().len() > 0,
        "OTLP DB file should be non-empty"
    );
}
