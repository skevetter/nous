use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn nous_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nous"))
}

fn run_nous(args: &[&str], config_home: &str, data_home: &str) -> std::process::Output {
    Command::new(nous_bin())
        .args(args)
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_DATA_HOME", data_home)
        .output()
        .expect("failed to execute nous binary")
}

#[test]
fn agent_add_full_definition() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let skills_dir = config_home.join("nous").join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    fs::write(
        skills_dir.join("code-review.md"),
        "# Code Review\nGuidelines.",
    )
    .unwrap();

    let toml_path = tmp.path().join("agent.toml");
    fs::write(
        &toml_path,
        r#"[agent]
name = "test-reviewer"
version = "1.0.0"
namespace = "testns"

[process]
type = "claude"
spawn_command = "claude --model sonnet"
working_dir = "/tmp"
auto_restart = false

[skills]
refs = ["code-review"]

[metadata]
model = "claude-sonnet-4-6"
timeout = 3600
tags = ["test"]
"#,
    )
    .unwrap();

    let output = run_nous(
        &["agent", "add", toml_path.to_str().unwrap()],
        config_home.to_str().unwrap(),
        data_home.to_str().unwrap(),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "agent add failed: stdout={stdout}, stderr={stderr}"
    );

    let agent: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(agent["name"].as_str().unwrap(), "test-reviewer");
    assert_eq!(agent["namespace"].as_str().unwrap(), "testns");
}

#[test]
fn agent_add_minimal_definition() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let toml_path = tmp.path().join("agent.toml");
    fs::write(
        &toml_path,
        r#"[agent]
name = "minimal-agent"
version = "0.1.0"
"#,
    )
    .unwrap();

    let output = run_nous(
        &["agent", "add", toml_path.to_str().unwrap()],
        config_home.to_str().unwrap(),
        data_home.to_str().unwrap(),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "agent add minimal failed: stdout={stdout}, stderr={stderr}"
    );

    let agent: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(agent["name"].as_str().unwrap(), "minimal-agent");
    assert_eq!(agent["status"].as_str().unwrap(), "idle");
}

#[test]
fn agent_add_invalid_file_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let output = run_nous(
        &["agent", "add", "/nonexistent/file.toml"],
        config_home.to_str().unwrap(),
        data_home.to_str().unwrap(),
    );

    assert!(!output.status.success());
}

#[test]
fn agent_remove_by_name() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let config_str = config_home.to_str().unwrap();
    let data_str = data_home.to_str().unwrap();

    // First register an agent
    let output = run_nous(
        &["agent", "register", "--name", "removable"],
        config_str,
        data_str,
    );
    assert!(output.status.success(), "register failed");

    // Remove by name
    let output = run_nous(&["agent", "remove", "removable"], config_str, data_str);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "agent remove failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("\"result\""));
}

#[test]
fn agent_remove_by_uuid() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let config_str = config_home.to_str().unwrap();
    let data_str = data_home.to_str().unwrap();

    // Register an agent to get its ID
    let output = run_nous(
        &["agent", "register", "--name", "removable-uuid"],
        config_str,
        data_str,
    );
    assert!(output.status.success());
    let agent: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let agent_id = agent["id"].as_str().unwrap();

    // Remove by UUID
    let output = run_nous(&["agent", "remove", agent_id], config_str, data_str);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "agent remove by uuid failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("\"result\""));
}

#[test]
fn agent_remove_cascade() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let config_str = config_home.to_str().unwrap();
    let data_str = data_home.to_str().unwrap();

    // Register parent
    let output = run_nous(
        &["agent", "register", "--name", "parent-cascade"],
        config_str,
        data_str,
    );
    assert!(output.status.success());
    let parent: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let parent_id = parent["id"].as_str().unwrap();

    // Register child
    let output = run_nous(
        &[
            "agent",
            "register",
            "--name",
            "child-cascade",
            "--parent",
            parent_id,
        ],
        config_str,
        data_str,
    );
    assert!(output.status.success());

    // Remove parent with cascade
    let output = run_nous(
        &["agent", "remove", "parent-cascade", "--cascade"],
        config_str,
        data_str,
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "agent remove cascade failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("\"result\""));
}

#[test]
fn agent_remove_nonexistent_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let output = run_nous(
        &["agent", "remove", "nonexistent-agent-xyz"],
        config_home.to_str().unwrap(),
        data_home.to_str().unwrap(),
    );

    assert!(!output.status.success());
}

#[test]
fn agent_add_then_remove_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let config_str = config_home.to_str().unwrap();
    let data_str = data_home.to_str().unwrap();

    let toml_path = tmp.path().join("roundtrip.toml");
    fs::write(
        &toml_path,
        r#"[agent]
name = "roundtrip-agent"
version = "1.0.0"
"#,
    )
    .unwrap();

    // Add
    let output = run_nous(
        &["agent", "add", toml_path.to_str().unwrap()],
        config_str,
        data_str,
    );
    assert!(output.status.success());
    let agent: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(agent["name"].as_str().unwrap(), "roundtrip-agent");

    // Remove
    let output = run_nous(
        &["agent", "remove", "roundtrip-agent"],
        config_str,
        data_str,
    );
    assert!(output.status.success());

    // Verify it's gone
    let output = run_nous(
        &["agent", "lookup", "roundtrip-agent"],
        config_str,
        data_str,
    );
    assert!(!output.status.success());
}
