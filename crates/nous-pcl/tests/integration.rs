use std::collections::HashMap;
use std::process::Command;

use nous_pcl::{
    CollectorConfig, CollectorRegistry, GitCollector, PclDirectory, PipelineRunner, RunMetadata,
    TransformPipeline,
};
use rusqlite::Connection;

fn init_test_repo(dir: &std::path::Path) {
    Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(dir)
        .output()
        .unwrap();
}

fn make_commit(dir: &std::path::Path, filename: &str, content: &str, message: &str) {
    std::fs::write(dir.join(filename), content).unwrap();
    Command::new("git")
        .args(["add", filename])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(dir)
        .output()
        .unwrap();
}

#[test]
fn end_to_end_git_collector_and_transform() {
    let repo_dir = tempfile::tempdir().unwrap();
    init_test_repo(repo_dir.path());
    make_commit(repo_dir.path(), "hello.txt", "hello world", "first commit");
    make_commit(repo_dir.path(), "readme.md", "# README", "add readme");
    make_commit(
        repo_dir.path(),
        "hello.txt",
        "hello world updated",
        "update hello",
    );

    let pcl_dir_tmp = tempfile::tempdir().unwrap();
    let pcl_dir = PclDirectory::new(pcl_dir_tmp.path().to_path_buf());
    pcl_dir.initialize().unwrap();

    let mut registry = CollectorRegistry::new();
    registry.register(Box::new(GitCollector::new()));

    let config = CollectorConfig {
        name: "git".into(),
        enabled: true,
        settings: HashMap::from([
            (
                "repo_paths".into(),
                serde_json::json!([repo_dir.path().to_str().unwrap()]),
            ),
            ("depth".into(), serde_json::json!(50)),
        ]),
    };

    // Run collector via pipeline runner — produces bronze JSON-lines
    let runner = PipelineRunner::new(&pcl_dir, &registry);
    let count = runner.run_collector("git", &config).unwrap();
    assert!(count > 0, "should collect records");

    // Verify bronze files exist
    let bronze_git = pcl_dir.bronze_dir("git");
    assert!(bronze_git.join("commit.jsonl").exists());
    assert!(bronze_git.join("branch.jsonl").exists());

    let commit_lines = std::fs::read_to_string(bronze_git.join("commit.jsonl"))
        .unwrap()
        .lines()
        .count();
    assert_eq!(commit_lines, 3, "should have 3 commit lines");

    // Transform bronze → silver
    let conn = Connection::open_in_memory().unwrap();
    let transform = TransformPipeline::new(&pcl_dir, &conn);
    transform.initialize_silver_tables().unwrap();

    let result = transform.transform_git().unwrap();
    // We created 3 commits, at least 1 branch (master/main), and 1 diff file (hello.txt changed in last commit)
    // Assert against known test data, not DB state
    assert!(
        result.inserted >= 5,
        "expected at least 3 commits + 1 branch + 1 diff, got {}",
        result.inserted
    );
    assert_eq!(result.skipped, 0);
    assert_eq!(result.errors, 0);

    // Verify SQLite contents against known test data
    let commit_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_commits", [], |r| r.get(0))
        .unwrap();
    assert_eq!(commit_count, 3, "we made exactly 3 commits");

    let branch_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_branches", [], |r| r.get(0))
        .unwrap();
    assert!(branch_count >= 1, "should have at least main/master branch");

    let diff_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_diffs", [], |r| r.get(0))
        .unwrap();
    assert!(diff_count >= 1, "last commit changed hello.txt");

    // Verify commit data
    let message: String = conn
        .query_row(
            "SELECT message FROM git_commits ORDER BY date DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(message, "update hello");

    // Test metadata save/load
    let metadata = RunMetadata {
        collector_name: "git".into(),
        last_run: chrono::Utc::now().to_rfc3339(),
        records_collected: count,
        duration_ms: 100,
    };
    metadata.save(pcl_dir.config_dir().as_path()).unwrap();

    let loaded = RunMetadata::load(pcl_dir.config_dir().as_path(), "git")
        .unwrap()
        .unwrap();
    assert_eq!(loaded.records_collected, count);

    // Run collector AGAIN to test dedup
    runner.run_collector("git", &config).unwrap();

    let result2 = transform.transform_git().unwrap();
    assert!(result2.skipped > 0, "should skip duplicate commits");

    let commit_count_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_commits", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        commit_count_after, 3,
        "commit count should not increase after dedup"
    );

    let branch_count_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_branches", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        branch_count_after, branch_count,
        "branch count should not increase after dedup"
    );

    let diff_count_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_diffs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        diff_count_after, diff_count,
        "diff count should not increase after dedup"
    );
}
