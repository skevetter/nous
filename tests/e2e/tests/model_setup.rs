use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

const DB_KEY: &str = "test-e2e-key-do-not-use";

fn bin_dir() -> PathBuf {
    let mut path = std::env::current_exe().expect("cannot resolve test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path
}

fn nous_cli_bin() -> PathBuf {
    let p = bin_dir().join("nous");
    assert!(p.exists(), "nous binary not found at {}", p.display());
    p
}

struct TestEnv {
    _tmp: TempDir,
    mcp_db: PathBuf,
    key_file: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let tmp = TempDir::new().expect("failed to create temp dir");
        let dir = tmp.path();

        let mcp_db = dir.join("memory.db");
        let key_file = dir.join("db.key");

        std::fs::write(&key_file, DB_KEY).expect("failed to write key file");

        TestEnv {
            _tmp: tmp,
            mcp_db,
            key_file,
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(nous_cli_bin())
            .args(args)
            .env("NOUS_DB_KEY", DB_KEY)
            .env("NOUS_MEMORY_DB", &self.mcp_db)
            .env("NOUS_DB_KEY_FILE", &self.key_file)
            .output()
            .expect("failed to run nous")
    }
}

#[test]
fn test_model_setup_list_presets() {
    let env = TestEnv::new();

    let output = env.run(&["model", "setup"]);
    assert!(
        output.status.success(),
        "model setup (no args) failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("full") && stdout.contains("mini"),
        "should list both presets, got:\n{stdout}"
    );
    assert!(
        stdout.contains("Qwen3") || stdout.contains("1024"),
        "should show full preset details, got:\n{stdout}"
    );
    assert!(
        stdout.contains("MiniLM") || stdout.contains("384"),
        "should show mini preset details, got:\n{stdout}"
    );
}

#[test]
fn test_model_setup_list_presets_json() {
    let env = TestEnv::new();

    let output = env.run(&["--format", "json", "model", "setup"]);
    assert!(
        output.status.success(),
        "model setup --format json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("output should be valid JSON");

    let presets = json["presets"]
        .as_array()
        .expect("should have presets array");
    assert_eq!(presets.len(), 2, "should have 2 presets");

    let names: Vec<&str> = presets.iter().filter_map(|p| p["name"].as_str()).collect();
    assert!(names.contains(&"full"), "should contain 'full' preset");
    assert!(names.contains(&"mini"), "should contain 'mini' preset");
}

#[test]
fn test_model_setup_unknown_preset() {
    let env = TestEnv::new();

    let output = env.run(&["model", "setup", "nonexistent"]);
    assert!(
        !output.status.success(),
        "model setup with unknown preset should fail"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Unknown preset") || stderr.contains("nonexistent"),
        "error should mention unknown preset, got:\n{stderr}"
    );
}

#[test]
#[ignore] // requires network access to download model from HuggingFace
fn test_model_setup_mini_downloads_and_activates() {
    let env = TestEnv::new();

    let output = env.run(&["model", "setup", "mini"]);
    assert!(
        output.status.success(),
        "model setup mini failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("downloaded and activated"),
        "should confirm model activation, got:\n{stdout}"
    );

    let status = env.run(&["status", "--format", "json"]);
    assert!(
        status.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let status_stdout = String::from_utf8(status.stdout).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&status_stdout).expect("status should be valid JSON");

    let model_name = json["active_model"]
        .as_str()
        .or_else(|| json["active_model"]["name"].as_str())
        .unwrap_or("");
    assert!(
        model_name.contains("all-MiniLM-L6-v2") || model_name.contains("MiniLM"),
        "active model should be MiniLM after setup, got: {model_name}"
    );
}

#[test]
fn test_no_model_serve_gives_helpful_error() {
    let env = TestEnv::new();

    let output = Command::new(nous_cli_bin())
        .args([
            "serve",
            "--model",
            "nonexistent/model",
            "--variant",
            "onnx/model.onnx",
        ])
        .env("NOUS_DB_KEY", DB_KEY)
        .env("NOUS_MEMORY_DB", &env.mcp_db)
        .env("NOUS_DB_KEY_FILE", &env.key_file)
        .output()
        .expect("failed to run nous serve");

    assert!(
        !output.status.success(),
        "serve with invalid model should fail"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("model setup"),
        "error should mention 'model setup' hint, got:\n{stderr}"
    );
}
