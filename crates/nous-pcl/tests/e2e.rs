use std::collections::HashMap;
use std::process::Command;

use nous_pcl::snapshot::{SnapshotIndex, SnapshotMeta};
use nous_pcl::{
    CollectorConfig, CollectorRegistry, GitCollector, PclDirectory, PipelineRunner,
    SnapshotGenerator, SnapshotLevel, TransformPipeline,
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
fn e2e_git_to_gold_snapshot() {
    // 1. Create temp dirs
    let repo_dir = tempfile::tempdir().unwrap();
    let pcl_dir_tmp = tempfile::tempdir().unwrap();

    // 2. Initialize git repo with known commits
    init_test_repo(repo_dir.path());
    make_commit(repo_dir.path(), "main.rs", "fn main() {}", "initial commit");
    make_commit(
        repo_dir.path(),
        "main.rs",
        "fn main() { println!(\"hello\"); }",
        "second commit",
    );
    Command::new("git")
        .args(["branch", "feature-test"])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();

    // 3. Initialize PclDirectory
    let pcl_dir = PclDirectory::new(pcl_dir_tmp.path().to_path_buf());
    pcl_dir.initialize().unwrap();

    // 4. Create CollectorConfig
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

    // 5. Run GitCollector → verify bronze files
    let mut registry = CollectorRegistry::new();
    registry.register(Box::new(GitCollector::new()));
    let runner = PipelineRunner::new(&pcl_dir, &registry);
    let count = runner.run_collector("git", &config).unwrap();
    assert!(count > 0, "should collect records");

    let bronze_git = pcl_dir.bronze_dir("git");
    assert!(
        bronze_git.join("commit.jsonl").exists(),
        "bronze commits.jsonl should exist"
    );
    assert!(
        bronze_git.join("branch.jsonl").exists(),
        "bronze branches.jsonl should exist"
    );
    assert!(
        bronze_git.join("diff.jsonl").exists(),
        "bronze diffs.jsonl should exist"
    );

    // 6. Open silver SQLite, run TransformPipeline → verify tables populated
    let conn = Connection::open_in_memory().unwrap();
    let transform = TransformPipeline::new(&pcl_dir, &conn);
    transform.initialize_silver_tables().unwrap();

    let result = transform.transform_git().unwrap();
    assert!(result.inserted > 0, "should insert records into silver");
    assert_eq!(result.errors, 0, "should have no errors");

    let commit_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_commits", [], |r| r.get(0))
        .unwrap();
    assert_eq!(commit_count, 2, "we made exactly 2 commits");

    let branch_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM git_branches", [], |r| r.get(0))
        .unwrap();
    assert!(
        branch_count >= 2,
        "should have at least main + feature-test"
    );

    // 7. Run SnapshotGenerator for each level
    let snap = SnapshotGenerator::new(&pcl_dir, &conn);

    let synopsis = snap
        .generate_task_snapshot("all", SnapshotLevel::Synopsis)
        .unwrap();
    let standard = snap
        .generate_task_snapshot("all", SnapshotLevel::Standard)
        .unwrap();
    let detailed = snap
        .generate_task_snapshot("all", SnapshotLevel::Detailed)
        .unwrap();

    // 8a. Has YAML frontmatter with required fields
    for (name, md) in [
        ("synopsis", &synopsis),
        ("standard", &standard),
        ("detailed", &detailed),
    ] {
        assert!(
            md.starts_with("---\n"),
            "{name} should start with frontmatter"
        );
        assert!(md.contains("generated_at:"), "{name} missing generated_at");
        assert!(md.contains("stale_after:"), "{name} missing stale_after");
        assert!(md.contains("scope: task"), "{name} missing scope");
        assert!(md.contains("subject: all"), "{name} missing subject");
        assert!(
            md.contains(&format!("level: {name}")),
            "{name} missing level"
        );
    }

    // 8b. Synopsis is shorter than standard, standard shorter than detailed
    assert!(
        synopsis.len() < standard.len(),
        "synopsis ({}) should be shorter than standard ({})",
        synopsis.len(),
        standard.len()
    );
    assert!(
        standard.len() < detailed.len(),
        "standard ({}) should be shorter than detailed ({})",
        standard.len(),
        detailed.len()
    );

    // 8c. Contains expected commit info
    assert!(
        standard.contains("second commit"),
        "standard snapshot should contain commit message"
    );
    assert!(
        detailed.contains("main.rs"),
        "detailed snapshot should contain changed file"
    );

    // 8d. Write gold output and verify _index.json
    snap.write_snapshot("all", &synopsis, SnapshotLevel::Synopsis)
        .unwrap();
    snap.write_snapshot("feature-test", &standard, SnapshotLevel::Standard)
        .unwrap();
    snap.write_snapshot("detailed-all", &detailed, SnapshotLevel::Detailed)
        .unwrap();

    let gold_dir = pcl_dir.gold_dir();
    assert!(gold_dir.join("all.md").exists(), "gold/all.md should exist");
    assert!(
        gold_dir.join("feature-test.md").exists(),
        "gold/feature-test.md should exist"
    );
    assert!(
        gold_dir.join("detailed-all.md").exists(),
        "gold/detailed-all.md should exist"
    );
    assert!(
        gold_dir.join("_index.json").exists(),
        "gold/_index.json should exist"
    );

    let index: SnapshotIndex =
        serde_json::from_str(&std::fs::read_to_string(gold_dir.join("_index.json")).unwrap())
            .unwrap();
    assert_eq!(index.entries.len(), 3);

    let all_entry = index.entries.iter().find(|e| e.subject == "all").unwrap();
    assert_eq!(all_entry.level, "synopsis");
    assert_eq!(all_entry.path, "all.md");
    assert!(!all_entry.generated_at.is_empty());
    assert!(!all_entry.stale_after.is_empty());

    // 9. Verify stale detection
    let stale_meta = SnapshotMeta {
        subject: "stale-test".into(),
        generated_at: "2020-01-01T00:00:00+00:00".into(),
        stale_after: "2020-01-01T00:15:00+00:00".into(),
        level: "standard".into(),
        path: "stale-test.md".into(),
    };
    assert!(
        stale_meta.is_stale(),
        "meta with past stale_after should be stale"
    );

    let fresh_entry = index.entries.iter().find(|e| e.subject == "all").unwrap();
    assert!(
        !fresh_entry.is_stale(),
        "just-generated snapshot should not be stale"
    );
}

#[test]
fn e2e_snapshot_level_content_boundaries() {
    let repo_dir = tempfile::tempdir().unwrap();
    let pcl_dir_tmp = tempfile::tempdir().unwrap();

    init_test_repo(repo_dir.path());
    make_commit(repo_dir.path(), "lib.rs", "pub fn hello() {}", "add lib");

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

    let runner = PipelineRunner::new(&pcl_dir, &registry);
    runner.run_collector("git", &config).unwrap();

    let conn = Connection::open_in_memory().unwrap();
    let transform = TransformPipeline::new(&pcl_dir, &conn);
    transform.initialize_silver_tables().unwrap();
    transform.transform_git().unwrap();

    let snap = SnapshotGenerator::new(&pcl_dir, &conn);

    let synopsis = snap
        .generate_task_snapshot("all", SnapshotLevel::Synopsis)
        .unwrap();
    let standard = snap
        .generate_task_snapshot("all", SnapshotLevel::Standard)
        .unwrap();
    let detailed = snap
        .generate_task_snapshot("all", SnapshotLevel::Detailed)
        .unwrap();

    // Synopsis: has key facts but NOT details section
    assert!(synopsis.contains("## Key Facts"));
    assert!(!synopsis.contains("## Details"));
    assert!(!synopsis.contains("### File Changes"));

    // Standard: has details but NOT file changes
    assert!(standard.contains("## Details"));
    assert!(standard.contains("### Recent Commits"));
    assert!(standard.contains("### Branches"));
    assert!(!standard.contains("### File Changes"));

    // Detailed: has everything
    assert!(detailed.contains("## Details"));
    assert!(detailed.contains("### File Changes"));
    assert!(detailed.contains("## Not Applicable"));
}
