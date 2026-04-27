use std::path::Path;
use std::process::Command;

use chrono::Utc;

use crate::collector::{Collector, CollectorConfig, Record};
use crate::error::PclError;

pub struct GitCollector;

impl GitCollector {
    pub fn new() -> Self {
        Self
    }

    fn repo_paths(config: &CollectorConfig) -> Result<Vec<String>, PclError> {
        let val = config
            .settings
            .get("repo_paths")
            .ok_or_else(|| PclError::InvalidConfig("missing 'repo_paths' setting".into()))?;
        let paths: Vec<String> = serde_json::from_value(val.clone())
            .map_err(|e| PclError::InvalidConfig(format!("invalid 'repo_paths': {e}")))?;
        if paths.is_empty() {
            return Err(PclError::InvalidConfig("'repo_paths' is empty".into()));
        }
        Ok(paths)
    }

    fn depth(config: &CollectorConfig) -> u32 {
        config
            .settings
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as u32
    }

    fn run_git(repo: &Path, args: &[&str]) -> Result<String, PclError> {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .map_err(|e| PclError::CollectorFailed {
                name: "git".into(),
                source: Box::new(e),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PclError::CollectorFailed {
                name: "git".into(),
                source: stderr.to_string().into(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn collect_commits(repo: &Path, depth: u32, now: &str) -> Result<Vec<Record>, PclError> {
        let format = "--format=%H%n%an%n%aI%n%s";
        let max = format!("-{depth}");
        let output = Self::run_git(repo, &["log", format, &max])?;
        let mut records = Vec::new();
        let lines: Vec<&str> = output.lines().collect();
        for chunk in lines.chunks(4) {
            if chunk.len() < 4 {
                continue;
            }
            let sha = chunk[0];
            let author = chunk[1];
            let date = chunk[2];
            let message = chunk[3];

            let numstat_output =
                Self::run_git(repo, &["show", "--stat", "--format=", sha]).unwrap_or_default();
            let files_changed = numstat_output
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count()
                .saturating_sub(1); // last line is summary

            records.push(Record {
                source: "git".into(),
                timestamp: now.into(),
                schema_version: 1,
                kind: "commit".into(),
                data: serde_json::json!({
                    "sha": sha,
                    "author": author,
                    "date": date,
                    "message": message,
                    "files_changed": files_changed,
                    "repo_path": repo.to_string_lossy(),
                }),
            });
        }
        Ok(records)
    }

    fn collect_branches(repo: &Path, now: &str) -> Result<Vec<Record>, PclError> {
        let output = Self::run_git(repo, &["branch", "-a", "--format=%(refname:short) %(HEAD)"])?;
        let mut records = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let is_current = line.ends_with('*');
            let name = line.trim_end_matches(" *").trim_end().to_string();
            let is_remote = name.starts_with("remotes/");

            records.push(Record {
                source: "git".into(),
                timestamp: now.into(),
                schema_version: 1,
                kind: "branch".into(),
                data: serde_json::json!({
                    "name": name,
                    "is_remote": is_remote,
                    "is_current": is_current,
                    "repo_path": repo.to_string_lossy(),
                }),
            });
        }
        Ok(records)
    }

    fn collect_diffs(repo: &Path, now: &str) -> Result<Vec<Record>, PclError> {
        let head_sha = Self::run_git(repo, &["rev-parse", "HEAD"])
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let output = match Self::run_git(repo, &["diff", "HEAD~1", "--numstat"]) {
            Ok(o) => o,
            Err(_) => return Ok(Vec::new()),
        };

        let mut records = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 3 {
                continue;
            }
            let insertions = parts[0].parse::<i64>().unwrap_or(0);
            let deletions = parts[1].parse::<i64>().unwrap_or(0);
            let file = parts[2];

            records.push(Record {
                source: "git".into(),
                timestamp: now.into(),
                schema_version: 1,
                kind: "diff".into(),
                data: serde_json::json!({
                    "file": file,
                    "insertions": insertions,
                    "deletions": deletions,
                    "repo_path": repo.to_string_lossy(),
                    "commit_sha": head_sha,
                }),
            });
        }
        Ok(records)
    }
}

impl Default for GitCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for GitCollector {
    fn name(&self) -> &str {
        "git"
    }

    fn collect(&self, config: &CollectorConfig) -> Result<Vec<Record>, PclError> {
        let repo_paths = Self::repo_paths(config)?;
        let depth = Self::depth(config);
        let now = Utc::now().to_rfc3339();
        let mut all_records = Vec::new();

        for repo_str in &repo_paths {
            let repo = Path::new(repo_str);
            if !repo.exists() {
                return Err(PclError::InvalidConfig(format!(
                    "repo path does not exist: {repo_str}"
                )));
            }
            if !repo.join(".git").exists()
                && Self::run_git(repo, &["rev-parse", "--git-dir"]).is_err()
            {
                return Err(PclError::InvalidConfig(format!(
                    "not a git repository: {repo_str}"
                )));
            }

            let mut commits = Self::collect_commits(repo, depth, &now)?;
            let mut branches = Self::collect_branches(repo, &now)?;
            let mut diffs = Self::collect_diffs(repo, &now)?;

            all_records.append(&mut commits);
            all_records.append(&mut branches);
            all_records.append(&mut diffs);
        }

        Ok(all_records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;

    fn init_test_repo(dir: &Path) {
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

    fn make_commit(dir: &Path, filename: &str, content: &str, message: &str) {
        fs::write(dir.join(filename), content).unwrap();
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

    fn test_config(repo_path: &str) -> CollectorConfig {
        CollectorConfig {
            name: "git".into(),
            enabled: true,
            settings: HashMap::from([
                ("repo_paths".into(), serde_json::json!([repo_path])),
                ("depth".into(), serde_json::json!(10)),
            ]),
        }
    }

    #[test]
    fn collects_commits_from_temp_repo() {
        let dir = tempfile::tempdir().unwrap();
        init_test_repo(dir.path());
        make_commit(dir.path(), "a.txt", "hello", "first commit");
        make_commit(dir.path(), "b.txt", "world", "second commit");

        let collector = GitCollector::new();
        let config = test_config(dir.path().to_str().unwrap());
        let records = collector.collect(&config).unwrap();

        let commits: Vec<_> = records.iter().filter(|r| r.kind == "commit").collect();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].data["message"], "second commit");
        assert_eq!(commits[1].data["message"], "first commit");
        assert_eq!(commits[0].source, "git");
        assert_eq!(commits[0].schema_version, 1);
    }

    #[test]
    fn collects_branches() {
        let dir = tempfile::tempdir().unwrap();
        init_test_repo(dir.path());
        make_commit(dir.path(), "a.txt", "hello", "init");

        Command::new("git")
            .args(["branch", "feature-x"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let collector = GitCollector::new();
        let config = test_config(dir.path().to_str().unwrap());
        let records = collector.collect(&config).unwrap();

        let branches: Vec<_> = records.iter().filter(|r| r.kind == "branch").collect();
        assert!(branches.len() >= 2);
        let names: Vec<&str> = branches
            .iter()
            .map(|r| r.data["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"feature-x"));
    }

    #[test]
    fn collects_diffs() {
        let dir = tempfile::tempdir().unwrap();
        init_test_repo(dir.path());
        make_commit(dir.path(), "a.txt", "hello", "first");
        make_commit(dir.path(), "b.txt", "world", "second");

        let collector = GitCollector::new();
        let config = test_config(dir.path().to_str().unwrap());
        let records = collector.collect(&config).unwrap();

        let diffs: Vec<_> = records.iter().filter(|r| r.kind == "diff").collect();
        assert!(!diffs.is_empty());
        assert!(diffs[0].data["file"].as_str().unwrap().contains("b.txt"));
    }

    #[test]
    fn errors_on_missing_repo_path() {
        let collector = GitCollector::new();
        let config = CollectorConfig {
            name: "git".into(),
            enabled: true,
            settings: HashMap::from([(
                "repo_paths".into(),
                serde_json::json!(["/nonexistent/path/xyzzy"]),
            )]),
        };
        let err = collector.collect(&config).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn errors_on_missing_config() {
        let collector = GitCollector::new();
        let config = CollectorConfig {
            name: "git".into(),
            enabled: true,
            settings: HashMap::new(),
        };
        let err = collector.collect(&config).unwrap_err();
        assert!(err.to_string().contains("repo_paths"));
    }

    #[test]
    fn default_depth_is_50() {
        let config = CollectorConfig {
            name: "git".into(),
            enabled: true,
            settings: HashMap::from([("repo_paths".into(), serde_json::json!(["/tmp"]))]),
        };
        assert_eq!(GitCollector::depth(&config), 50);
    }
}
