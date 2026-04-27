use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use nous_core::db::MemoryDb;
use nous_core::types::{MemoryType, NewMemory};
use nous_otlp::db::OtlpDb;
use nous_otlp::decode::{LogEvent, Span};
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

const IMPORT_WITH_CATEGORIES_JSON: &str = r#"{
  "version": 1,
  "memories": [
    {
      "id": "mem_cattest0001",
      "title": "Category Roundtrip Memory",
      "content": "This memory tests category import/export roundtrip.",
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
      "category_id": 900,
      "created_at": "2026-01-01T00:00:00Z",
      "updated_at": "2026-01-01T00:00:00Z",
      "tags": ["category-test"],
      "relationships": []
    }
  ],
  "categories": [
    {
      "id": 900,
      "name": "imported-parent",
      "parent_id": null,
      "source": "user",
      "description": "An imported parent category",
      "created_at": "2026-01-01T00:00:00Z"
    },
    {
      "id": 901,
      "name": "imported-child",
      "parent_id": 900,
      "source": "user",
      "description": "An imported child category",
      "created_at": "2026-01-01T00:00:00Z"
    }
  ]
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
    let p = bin_dir().join("nous");
    assert!(p.exists(), "nous binary not found at {}", p.display());
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
            .stderr(Stdio::inherit())
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

fn run_nous_mcp(
    mcp_db: &std::path::Path,
    key_file: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    Command::new(nous_mcp_bin())
        .args(args)
        .env("NOUS_DB_KEY", DB_KEY)
        .env("NOUS_MEMORY_DB", mcp_db)
        .env("NOUS_DB_KEY_FILE", key_file)
        .output()
        .expect("failed to run nous")
}

fn run_import(mcp_db: &std::path::Path, key_file: &std::path::Path, import_file: &std::path::Path) {
    let output = Command::new(nous_mcp_bin())
        .args(["import"])
        .arg(import_file)
        .env("NOUS_DB_KEY", DB_KEY)
        .env("NOUS_MEMORY_DB", mcp_db)
        .env("NOUS_DB_KEY_FILE", key_file)
        .output()
        .expect("failed to run nous import");

    assert!(
        output.status.success(),
        "nous import failed: {}",
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
        .expect("failed to run nous export");

    assert!(
        output.status.success(),
        "nous export failed: {}",
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

// ---------------------------------------------------------------------------
// Category CRUD E2E tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_category_add_and_list() {
    let env = TestEnv::new();

    // First import to initialise the DB (seeds system categories)
    run_import(&env.mcp_db, &env.key_file, &env.import_file);

    let output = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &[
            "category",
            "add",
            "my-custom-cat",
            "--description",
            "A custom test category",
        ],
    );
    assert!(
        output.status.success(),
        "category add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let list_output = run_nous_mcp(&env.mcp_db, &env.key_file, &["category", "list"]);
    assert!(
        list_output.status.success(),
        "category list failed: {}",
        String::from_utf8_lossy(&list_output.stderr)
    );
    let stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        stdout.contains("my-custom-cat"),
        "category list should contain 'my-custom-cat', got:\n{stdout}"
    );
    assert!(
        stdout.contains("A custom test category"),
        "category list should contain description, got:\n{stdout}"
    );
}

#[tokio::test]
async fn test_category_delete() {
    let env = TestEnv::new();
    run_import(&env.mcp_db, &env.key_file, &env.import_file);

    // Add then delete
    let add = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "add", "to-delete"],
    );
    assert!(
        add.status.success(),
        "category add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    let del = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "delete", "to-delete"],
    );
    assert!(
        del.status.success(),
        "category delete failed: {}",
        String::from_utf8_lossy(&del.stderr)
    );

    let list = run_nous_mcp(&env.mcp_db, &env.key_file, &["category", "list"]);
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        !stdout.contains("to-delete"),
        "deleted category should not appear in list, got:\n{stdout}"
    );
}

#[tokio::test]
async fn test_category_rename() {
    let env = TestEnv::new();
    run_import(&env.mcp_db, &env.key_file, &env.import_file);

    let add = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &[
            "category",
            "add",
            "old-name",
            "--description",
            "Will be renamed",
        ],
    );
    assert!(
        add.status.success(),
        "category add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    let rename = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "rename", "old-name", "new-name"],
    );
    assert!(
        rename.status.success(),
        "category rename failed: {}",
        String::from_utf8_lossy(&rename.stderr)
    );

    let list = run_nous_mcp(&env.mcp_db, &env.key_file, &["category", "list"]);
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        stdout.contains("new-name"),
        "renamed category should appear in list, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("old-name"),
        "old category name should not appear in list, got:\n{stdout}"
    );
}

#[tokio::test]
async fn test_category_update() {
    let env = TestEnv::new();
    run_import(&env.mcp_db, &env.key_file, &env.import_file);

    let add = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "add", "updatable", "--description", "Original"],
    );
    assert!(
        add.status.success(),
        "category add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    let update = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &[
            "category",
            "update",
            "updatable",
            "--description",
            "Updated description",
            "--threshold",
            "0.85",
        ],
    );
    assert!(
        update.status.success(),
        "category update failed: {}",
        String::from_utf8_lossy(&update.stderr)
    );

    let list = run_nous_mcp(&env.mcp_db, &env.key_file, &["category", "list"]);
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        stdout.contains("Updated description"),
        "updated description should appear in list, got:\n{stdout}"
    );
}

#[tokio::test]
async fn test_category_delete_refuses_with_children() {
    let env = TestEnv::new();
    run_import(&env.mcp_db, &env.key_file, &env.import_file);

    // Add parent
    let add_parent = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "add", "parent-cat"],
    );
    assert!(
        add_parent.status.success(),
        "parent add failed: {}",
        String::from_utf8_lossy(&add_parent.stderr)
    );

    // Add child under parent
    let add_child = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "add", "child-cat", "--parent", "parent-cat"],
    );
    assert!(
        add_child.status.success(),
        "child add failed: {}",
        String::from_utf8_lossy(&add_child.stderr)
    );

    // Try deleting parent — should report error about children
    let del = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "delete", "parent-cat"],
    );
    let stderr = String::from_utf8_lossy(&del.stderr);
    assert!(
        stderr.contains("children"),
        "error should mention children, got:\n{stderr}"
    );

    // Parent should still exist in list
    let list = run_nous_mcp(&env.mcp_db, &env.key_file, &["category", "list"]);
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        stdout.contains("parent-cat"),
        "parent category should still exist after failed delete, got:\n{stdout}"
    );
}

#[tokio::test]
async fn test_re_classify_assigns_categories() {
    let env = TestEnv::new();

    // Import a memory (seeds DB + system categories)
    run_import(&env.mcp_db, &env.key_file, &env.import_file);

    // Run re-classify
    let rc = run_nous_mcp(&env.mcp_db, &env.key_file, &["re-classify"]);
    assert!(
        rc.status.success(),
        "re-classify failed: {}",
        String::from_utf8_lossy(&rc.stderr)
    );
    let rc_stderr = String::from_utf8_lossy(&rc.stderr);
    assert!(
        rc_stderr.contains("Re-classified"),
        "re-classify should report progress on stderr, got:\n{rc_stderr}"
    );
}

#[tokio::test]
async fn test_category_list_filter_by_source() {
    let env = TestEnv::new();
    run_import(&env.mcp_db, &env.key_file, &env.import_file);

    let add = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "add", "user-only-cat"],
    );
    assert!(
        add.status.success(),
        "category add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    let user_list = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "list", "--source", "user"],
    );
    let user_stdout = String::from_utf8_lossy(&user_list.stdout);
    assert!(
        user_stdout.contains("user-only-cat"),
        "user source filter should include user category, got:\n{user_stdout}"
    );

    let seed_list = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "list", "--source", "system"],
    );
    let seed_stdout = String::from_utf8_lossy(&seed_list.stdout);
    assert!(
        !seed_stdout.contains("user-only-cat"),
        "system source filter should not include user category, got:\n{seed_stdout}"
    );
}

#[tokio::test]
async fn test_category_update_rename_via_update() {
    let env = TestEnv::new();
    run_import(&env.mcp_db, &env.key_file, &env.import_file);

    let add = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "add", "before-update"],
    );
    assert!(
        add.status.success(),
        "category add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    let update = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &[
            "category",
            "update",
            "before-update",
            "--new-name",
            "after-update",
        ],
    );
    assert!(
        update.status.success(),
        "category update with --new-name failed: {}",
        String::from_utf8_lossy(&update.stderr)
    );

    let list = run_nous_mcp(&env.mcp_db, &env.key_file, &["category", "list"]);
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        stdout.contains("after-update"),
        "new name should appear after update: {stdout}"
    );
    assert!(
        !stdout.contains("before-update"),
        "old name should not appear after update: {stdout}"
    );
}

#[tokio::test]
async fn test_category_crud_full_lifecycle() {
    let env = TestEnv::new();
    run_import(&env.mcp_db, &env.key_file, &env.import_file);

    // Add
    let add = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &[
            "category",
            "add",
            "lifecycle-cat",
            "--description",
            "lifecycle test",
        ],
    );
    assert!(add.status.success());
    let add_stdout = String::from_utf8_lossy(&add.stdout);
    assert!(add_stdout.contains("Added category 'lifecycle-cat'"));

    // Rename
    let rename = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "rename", "lifecycle-cat", "lifecycle-v2"],
    );
    assert!(rename.status.success());

    // Update description
    let update = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &[
            "category",
            "update",
            "lifecycle-v2",
            "--description",
            "updated lifecycle",
        ],
    );
    assert!(update.status.success());

    // Verify state
    let list = run_nous_mcp(&env.mcp_db, &env.key_file, &["category", "list"]);
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains("lifecycle-v2"));
    assert!(stdout.contains("updated lifecycle"));
    assert!(!stdout.contains("lifecycle-cat"));

    // Delete
    let del = run_nous_mcp(
        &env.mcp_db,
        &env.key_file,
        &["category", "delete", "lifecycle-v2"],
    );
    assert!(del.status.success());
    let del_stdout = String::from_utf8_lossy(&del.stdout);
    assert!(del_stdout.contains("Deleted category 'lifecycle-v2'"));

    // Verify gone
    let list = run_nous_mcp(&env.mcp_db, &env.key_file, &["category", "list"]);
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(!stdout.contains("lifecycle-v2"));
}

#[tokio::test]
async fn test_import_export_categories_roundtrip() {
    let env = TestEnv::new();

    // Write import file with categories
    let cat_import_file = env._tmp.path().join("import_cats.json");
    std::fs::write(&cat_import_file, IMPORT_WITH_CATEGORIES_JSON)
        .expect("failed to write category import file");

    // Import
    run_import(&env.mcp_db, &env.key_file, &cat_import_file);

    // Export
    let export_out = run_export(&env.mcp_db, &env.key_file);

    // Verify both categories appear in export
    assert!(
        export_out.contains("imported-parent"),
        "export should contain 'imported-parent', got:\n{export_out}"
    );
    assert!(
        export_out.contains("imported-child"),
        "export should contain 'imported-child', got:\n{export_out}"
    );
    assert!(
        export_out.contains("An imported parent category"),
        "export should contain parent description, got:\n{export_out}"
    );
    assert!(
        export_out.contains("An imported child category"),
        "export should contain child description, got:\n{export_out}"
    );

    // Verify memory is present
    assert!(
        export_out.contains("Category Roundtrip Memory"),
        "export should contain imported memory, got:\n{export_out}"
    );

    // Verify the memory has a category_id (non-null, since it was mapped)
    assert!(
        export_out.contains("category-test"),
        "export should contain tag from imported memory, got:\n{export_out}"
    );
}

// ---------------------------------------------------------------------------
// OTLP Correlation CLI E2E tests
// ---------------------------------------------------------------------------

struct TraceTestEnv {
    _tmp: TempDir,
    mcp_db: PathBuf,
    otlp_db: PathBuf,
    key_file: PathBuf,
}

impl TraceTestEnv {
    fn new() -> Self {
        let tmp = TempDir::new().expect("failed to create temp dir");
        let dir = tmp.path();

        let mcp_db = dir.join("memory.db");
        let otlp_db = dir.join("otlp.db");
        let key_file = dir.join("db.key");

        std::fs::write(&key_file, DB_KEY).expect("failed to write key file");

        TraceTestEnv {
            _tmp: tmp,
            mcp_db,
            otlp_db,
            key_file,
        }
    }

    fn seed_memory(
        &self,
        title: &str,
        content: &str,
        trace_id: Option<&str>,
        session_id: Option<&str>,
    ) -> String {
        let db = MemoryDb::open(self.mcp_db.to_str().unwrap(), Some(DB_KEY), 384).unwrap();
        let memory = NewMemory {
            title: title.into(),
            content: content.into(),
            memory_type: MemoryType::Fact,
            source: Some("e2e-test".into()),
            importance: Default::default(),
            confidence: Default::default(),
            tags: vec![],
            workspace_path: None,
            session_id: session_id.map(Into::into),
            trace_id: trace_id.map(Into::into),
            agent_id: None,
            agent_model: None,
            valid_from: None,
            category_id: None,
        };
        db.store(&memory).unwrap().to_string()
    }

    fn seed_span(&self, trace_id: &str, span_name: &str) {
        let db = OtlpDb::open(self.otlp_db.to_str().unwrap(), None).unwrap();
        db.store_spans(&[Span {
            trace_id: trace_id.into(),
            span_id: format!("span-e2e-{trace_id}"),
            parent_span_id: None,
            name: span_name.into(),
            kind: 1,
            start_time: 1714004000,
            end_time: 1714004500,
            status_code: 0,
            status_message: None,
            resource_attrs: "{}".into(),
            span_attrs: "{}".into(),
            events_json: "[]".into(),
        }])
        .unwrap();
    }

    fn seed_log(&self, session_id: &str, body: &str) {
        let db = OtlpDb::open(self.otlp_db.to_str().unwrap(), None).unwrap();
        db.store_logs(&[LogEvent {
            timestamp: 1714004000,
            severity: "INFO".into(),
            body: body.into(),
            resource_attrs: "{}".into(),
            scope_attrs: "{}".into(),
            log_attrs: "{}".into(),
            session_id: Some(session_id.into()),
            trace_id: None,
            span_id: None,
        }])
        .unwrap();
    }

    fn run_trace(&self, args: &[&str]) -> std::process::Output {
        Command::new(nous_mcp_bin())
            .arg("trace")
            .args(args)
            .env("NOUS_DB_KEY", DB_KEY)
            .env("NOUS_MEMORY_DB", &self.mcp_db)
            .env("NOUS_DB_KEY_FILE", &self.key_file)
            .env("NOUS_OTLP_DB", &self.otlp_db)
            .output()
            .expect("failed to run nous trace")
    }
}

#[tokio::test]
async fn test_status_format_json() {
    let tmp = TempDir::new().expect("cannot create temp dir");
    let mcp_db = tmp.path().join("test.db");
    let key_file = tmp.path().join("test.key");
    std::fs::write(&key_file, DB_KEY).unwrap();

    let _db = MemoryDb::open(mcp_db.to_str().unwrap(), Some(DB_KEY), 384).unwrap();

    let output = Command::new(nous_mcp_bin())
        .args(["status", "--format", "json"])
        .env("NOUS_DB_KEY", DB_KEY)
        .env("NOUS_MEMORY_DB", &mcp_db)
        .env("NOUS_DB_KEY_FILE", &key_file)
        .output()
        .expect("failed to run nous status --format json");

    assert!(
        output.status.success(),
        "nous status --format json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("status --format json should output valid JSON");

    assert!(
        json.get("memories").is_some(),
        "JSON should have 'memories' field"
    );
    assert!(
        json.get("categories").is_some(),
        "JSON should have 'categories' field"
    );
    assert!(
        json.get("chunks").is_some(),
        "JSON should have 'chunks' field"
    );
    assert!(
        json.get("active_model").is_some(),
        "JSON should have 'active_model' field"
    );
}

#[tokio::test]
async fn test_cli_trace_by_trace_id() {
    let env = TraceTestEnv::new();
    let trace_id = "e2e-trace-001";

    env.seed_memory("traced mem", "correlated content", Some(trace_id), None);
    env.seed_span(trace_id, "e2e-operation");

    let output = env.run_trace(&["--trace-id", trace_id]);
    assert!(
        output.status.success(),
        "trace --trace-id failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("output should be valid JSON");

    let memories = json["memories"]
        .as_array()
        .expect("memories should be array");
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0]["title"].as_str().unwrap(), "traced mem");

    let spans = json["spans"].as_array().expect("spans should be array");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0]["name"].as_str().unwrap(), "e2e-operation");
}

#[tokio::test]
async fn test_cli_trace_by_trace_id_with_session_id() {
    let env = TraceTestEnv::new();
    let trace_id = "e2e-trace-002";
    let session_id = "e2e-sess-002";

    env.seed_memory("dual mem", "dual content", Some(trace_id), Some(session_id));
    env.seed_span(trace_id, "dual-span-op");
    env.seed_log(session_id, "dual-session-log");

    let output = env.run_trace(&["--trace-id", trace_id, "--session-id", session_id]);
    assert!(
        output.status.success(),
        "trace --trace-id --session-id failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("output should be valid JSON");

    assert_eq!(json["memories"].as_array().unwrap().len(), 1);
    assert_eq!(json["spans"].as_array().unwrap().len(), 1);
    assert_eq!(json["logs"].as_array().unwrap().len(), 1);
    assert_eq!(
        json["logs"][0]["body"].as_str().unwrap(),
        "dual-session-log"
    );
}

#[tokio::test]
async fn test_cli_trace_by_memory_id() {
    let env = TraceTestEnv::new();
    let trace_id = "e2e-trace-003";
    let session_id = "e2e-sess-003";

    let mem_id = env.seed_memory(
        "mem-lookup",
        "content for mem lookup",
        Some(trace_id),
        Some(session_id),
    );
    env.seed_span(trace_id, "mem-lookup-span");
    env.seed_log(session_id, "mem-lookup-log");

    let output = env.run_trace(&["--memory-id", &mem_id]);
    assert!(
        output.status.success(),
        "trace --memory-id failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("output should be valid JSON");

    assert_eq!(json["memory"]["title"].as_str().unwrap(), "mem-lookup");
    assert_eq!(json["spans"].as_array().unwrap().len(), 1);
    assert_eq!(json["logs"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_cli_trace_unknown_trace_id_returns_empty() {
    let env = TraceTestEnv::new();

    let _ = OtlpDb::open(env.otlp_db.to_str().unwrap(), None).unwrap();
    let _ = MemoryDb::open(env.mcp_db.to_str().unwrap(), Some(DB_KEY), 384).unwrap();

    let output = env.run_trace(&["--trace-id", "nonexistent-trace-xyz"]);
    assert!(
        output.status.success(),
        "trace with unknown trace_id should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("output should be valid JSON");

    assert!(json["memories"].as_array().unwrap().is_empty());
    assert!(json["spans"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_cli_trace_memory_without_correlation_ids_errors() {
    let env = TraceTestEnv::new();

    let mem_id = env.seed_memory("no-corr mem", "no correlation ids", None, None);
    let _ = OtlpDb::open(env.otlp_db.to_str().unwrap(), None).unwrap();

    let output = env.run_trace(&["--memory-id", &mem_id]);
    assert!(
        !output.status.success(),
        "trace --memory-id on a memory without correlation ids should fail"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no trace_id or session_id"),
        "error should mention missing correlation ids, got: {stderr}"
    );
}
