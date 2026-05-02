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
fn skill_list_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let skills_dir = config_home.join("nous").join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    let output = run_nous(
        &["skill", "list"],
        config_home.to_str().unwrap(),
        data_home.to_str().unwrap(),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "skill list failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("No skills found"),
        "expected 'No skills found' message, got: {stdout}"
    );
}

#[test]
fn skill_list_no_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let output = run_nous(
        &["skill", "list"],
        config_home.to_str().unwrap(),
        data_home.to_str().unwrap(),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "skill list (no dir) failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("No skills directory found"),
        "expected 'No skills directory found' message, got: {stdout}"
    );
}

#[test]
fn skill_list_with_skills() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let skills_dir = config_home.join("nous").join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    fs::write(skills_dir.join("code-review.md"), "# Code Review").unwrap();
    fs::write(skills_dir.join("debugging.md"), "# Debugging").unwrap();

    let output = run_nous(
        &["skill", "list"],
        config_home.to_str().unwrap(),
        data_home.to_str().unwrap(),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "skill list failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("code-review"),
        "expected 'code-review' in output, got: {stdout}"
    );
    assert!(
        stdout.contains("debugging"),
        "expected 'debugging' in output, got: {stdout}"
    );
    assert!(
        stdout.contains("2 skill(s) found"),
        "expected '2 skill(s) found' in output, got: {stdout}"
    );
}

#[test]
fn skill_list_ignores_non_md() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&data_home).unwrap();

    let skills_dir = config_home.join("nous").join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    fs::write(skills_dir.join("valid-skill.md"), "# Valid").unwrap();
    fs::write(skills_dir.join("not-a-skill.txt"), "not a skill").unwrap();
    fs::write(skills_dir.join("also-not.toml"), "key = 'val'").unwrap();

    let output = run_nous(
        &["skill", "list"],
        config_home.to_str().unwrap(),
        data_home.to_str().unwrap(),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "skill list failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("valid-skill"),
        "expected 'valid-skill' in output, got: {stdout}"
    );
    assert!(
        !stdout.contains("not-a-skill"),
        "should not list .txt files, got: {stdout}"
    );
    assert!(
        !stdout.contains("also-not"),
        "should not list .toml files, got: {stdout}"
    );
    assert!(
        stdout.contains("1 skill(s) found"),
        "expected '1 skill(s) found' in output, got: {stdout}"
    );
}
