use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};

use chrono::Utc;
use rusqlite::{Connection, params};

use crate::collector::Record;
use crate::directory::PclDirectory;
use crate::error::PclError;

#[derive(Debug, Clone, Default)]
pub struct TransformResult {
    pub inserted: usize,
    pub skipped: usize,
    pub errors: usize,
}

pub struct TransformPipeline<'a> {
    directory: &'a PclDirectory,
    conn: &'a Connection,
}

impl<'a> TransformPipeline<'a> {
    pub fn new(directory: &'a PclDirectory, conn: &'a Connection) -> Self {
        Self { directory, conn }
    }

    pub fn initialize_silver_tables(&self) -> Result<(), PclError> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS git_commits (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    sha TEXT NOT NULL UNIQUE,
                    author TEXT NOT NULL,
                    date TEXT NOT NULL,
                    message TEXT NOT NULL,
                    files_changed INTEGER NOT NULL DEFAULT 0,
                    repo_path TEXT NOT NULL,
                    ingested_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_git_commits_sha ON git_commits(sha);
                CREATE INDEX IF NOT EXISTS idx_git_commits_date ON git_commits(date);

                CREATE TABLE IF NOT EXISTS git_branches (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    is_remote INTEGER NOT NULL DEFAULT 0,
                    is_current INTEGER NOT NULL DEFAULT 0,
                    repo_path TEXT NOT NULL,
                    ingested_at TEXT NOT NULL,
                    UNIQUE(name, repo_path)
                );

                CREATE TABLE IF NOT EXISTS git_diffs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    file TEXT NOT NULL,
                    insertions INTEGER NOT NULL DEFAULT 0,
                    deletions INTEGER NOT NULL DEFAULT 0,
                    repo_path TEXT NOT NULL,
                    commit_sha TEXT,
                    ingested_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_git_diffs_file ON git_diffs(file);",
            )
            .map_err(|e| PclError::CollectorFailed {
                name: "transform".into(),
                source: Box::new(e),
            })?;
        Ok(())
    }

    pub fn transform_git(&self) -> Result<TransformResult, PclError> {
        let bronze_dir = self.directory.bronze_dir("git");
        if !bronze_dir.exists() {
            return Ok(TransformResult::default());
        }

        let now = Utc::now().to_rfc3339();
        let mut result = TransformResult::default();

        let entries = fs::read_dir(&bronze_dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            let file = fs::File::open(&path)?;
            let reader = BufReader::new(file);

            for line in reader.lines() {
                let line = line?;
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let record: Record = match serde_json::from_str(line) {
                    Ok(r) => r,
                    Err(e) => {
                        self.log_error(&format!("malformed record: {e}: {line}"))?;
                        result.errors += 1;
                        continue;
                    }
                };

                match record.kind.as_str() {
                    "commit" => match self.insert_commit(&record, &now) {
                        Ok(true) => result.inserted += 1,
                        Ok(false) => result.skipped += 1,
                        Err(e) => {
                            self.log_error(&format!("commit insert error: {e}"))?;
                            result.errors += 1;
                        }
                    },
                    "branch" => match self.insert_branch(&record, &now) {
                        Ok(true) => result.inserted += 1,
                        Ok(false) => result.skipped += 1,
                        Err(e) => {
                            self.log_error(&format!("branch insert error: {e}"))?;
                            result.errors += 1;
                        }
                    },
                    "diff" => match self.insert_diff(&record, &now) {
                        Ok(true) => result.inserted += 1,
                        Ok(false) => result.skipped += 1,
                        Err(e) => {
                            self.log_error(&format!("diff insert error: {e}"))?;
                            result.errors += 1;
                        }
                    },
                    other => {
                        self.log_error(&format!("unknown record kind: {other}"))?;
                        result.errors += 1;
                    }
                }
            }
        }

        Ok(result)
    }

    fn insert_commit(&self, record: &Record, ingested_at: &str) -> Result<bool, PclError> {
        let data = &record.data;
        let sha = data["sha"]
            .as_str()
            .ok_or_else(|| PclError::InvalidConfig("commit missing 'sha'".into()))?;
        let author = data["author"]
            .as_str()
            .ok_or_else(|| PclError::InvalidConfig("commit missing 'author'".into()))?;
        let date = data["date"]
            .as_str()
            .ok_or_else(|| PclError::InvalidConfig("commit missing 'date'".into()))?;
        let message = data["message"]
            .as_str()
            .ok_or_else(|| PclError::InvalidConfig("commit missing 'message'".into()))?;
        let files_changed = data["files_changed"].as_i64().unwrap_or(0);
        let repo_path = data["repo_path"].as_str().unwrap_or("");

        let normalized_date = normalize_timestamp(date);

        let changed = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO git_commits (sha, author, date, message, files_changed, repo_path, ingested_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![sha, author, normalized_date, message, files_changed, repo_path, ingested_at],
            )
            .map_err(|e| PclError::CollectorFailed {
                name: "transform".into(),
                source: Box::new(e),
            })?;

        Ok(changed > 0)
    }

    fn insert_branch(&self, record: &Record, ingested_at: &str) -> Result<bool, PclError> {
        let data = &record.data;
        let name = data["name"]
            .as_str()
            .ok_or_else(|| PclError::InvalidConfig("branch missing 'name'".into()))?;
        let is_remote = data["is_remote"].as_bool().unwrap_or(false) as i32;
        let is_current = data["is_current"].as_bool().unwrap_or(false) as i32;
        let repo_path = data["repo_path"].as_str().unwrap_or("");

        let changed = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO git_branches (name, is_remote, is_current, repo_path, ingested_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![name, is_remote, is_current, repo_path, ingested_at],
            )
            .map_err(|e| PclError::CollectorFailed {
                name: "transform".into(),
                source: Box::new(e),
            })?;

        Ok(changed > 0)
    }

    fn insert_diff(&self, record: &Record, ingested_at: &str) -> Result<bool, PclError> {
        let data = &record.data;
        let file = data["file"]
            .as_str()
            .ok_or_else(|| PclError::InvalidConfig("diff missing 'file'".into()))?;
        let insertions = data["insertions"].as_i64().unwrap_or(0);
        let deletions = data["deletions"].as_i64().unwrap_or(0);
        let repo_path = data["repo_path"].as_str().unwrap_or("");
        let commit_sha = data["commit_sha"].as_str().unwrap_or("");

        let changed = self
            .conn
            .execute(
                "INSERT INTO git_diffs (file, insertions, deletions, repo_path, commit_sha, ingested_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![file, insertions, deletions, repo_path, commit_sha, ingested_at],
            )
            .map_err(|e| PclError::CollectorFailed {
                name: "transform".into(),
                source: Box::new(e),
            })?;

        Ok(changed > 0)
    }

    fn log_error(&self, msg: &str) -> Result<(), PclError> {
        let logs_dir = self.directory.logs_dir();
        fs::create_dir_all(&logs_dir)?;
        let log_path = logs_dir.join("errors.log");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        let now = Utc::now().to_rfc3339();
        writeln!(file, "[{now}] {msg}")?;
        Ok(())
    }
}

fn normalize_timestamp(input: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(input) {
        return dt.to_utc().to_rfc3339();
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S") {
        return dt.and_utc().to_rfc3339();
    }
    input.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::PclDirectory;
    use std::fs;

    fn setup() -> (tempfile::TempDir, PclDirectory, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let pcl_dir = PclDirectory::new(dir.path().to_path_buf());
        pcl_dir.initialize().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        (dir, pcl_dir, conn)
    }

    fn write_bronze_jsonl(pcl_dir: &PclDirectory, kind: &str, lines: &[&str]) {
        let bronze = pcl_dir.bronze_dir("git");
        fs::create_dir_all(&bronze).unwrap();
        let path = bronze.join(format!("{kind}.jsonl"));
        let content = lines.join("\n") + "\n";
        fs::write(path, content).unwrap();
    }

    #[test]
    fn transform_commits() {
        let (_dir, pcl_dir, conn) = setup();
        let pipeline = TransformPipeline::new(&pcl_dir, &conn);
        pipeline.initialize_silver_tables().unwrap();

        let record = serde_json::json!({
            "source": "git",
            "timestamp": "2026-04-26T12:00:00+00:00",
            "schema_version": 1,
            "kind": "commit",
            "data": {
                "sha": "abc123",
                "author": "Alice",
                "date": "2026-04-26T12:00:00+00:00",
                "message": "initial commit",
                "files_changed": 3,
                "repo_path": "/tmp/repo"
            }
        });
        write_bronze_jsonl(&pcl_dir, "commit", &[&record.to_string()]);

        let result = pipeline.transform_git().unwrap();
        assert_eq!(result.inserted, 1);
        assert_eq!(result.skipped, 0);
        assert_eq!(result.errors, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM git_commits", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn dedup_commits() {
        let (_dir, pcl_dir, conn) = setup();
        let pipeline = TransformPipeline::new(&pcl_dir, &conn);
        pipeline.initialize_silver_tables().unwrap();

        let record = serde_json::json!({
            "source": "git",
            "timestamp": "2026-04-26T12:00:00+00:00",
            "schema_version": 1,
            "kind": "commit",
            "data": {
                "sha": "abc123",
                "author": "Alice",
                "date": "2026-04-26T12:00:00+00:00",
                "message": "initial commit",
                "files_changed": 3,
                "repo_path": "/tmp/repo"
            }
        });
        write_bronze_jsonl(
            &pcl_dir,
            "commit",
            &[&record.to_string(), &record.to_string()],
        );

        let result = pipeline.transform_git().unwrap();
        assert_eq!(result.inserted, 1);
        assert_eq!(result.skipped, 1);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM git_commits", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn dedup_branches() {
        let (_dir, pcl_dir, conn) = setup();
        let pipeline = TransformPipeline::new(&pcl_dir, &conn);
        pipeline.initialize_silver_tables().unwrap();

        let record = serde_json::json!({
            "source": "git",
            "timestamp": "2026-04-26T12:00:00+00:00",
            "schema_version": 1,
            "kind": "branch",
            "data": {
                "name": "main",
                "is_remote": false,
                "is_current": true,
                "repo_path": "/tmp/repo"
            }
        });
        write_bronze_jsonl(
            &pcl_dir,
            "branch",
            &[&record.to_string(), &record.to_string()],
        );

        let result = pipeline.transform_git().unwrap();
        assert_eq!(result.inserted, 1);
        assert_eq!(result.skipped, 1);
    }

    #[test]
    fn handles_malformed_records() {
        let (_dir, pcl_dir, conn) = setup();
        let pipeline = TransformPipeline::new(&pcl_dir, &conn);
        pipeline.initialize_silver_tables().unwrap();

        write_bronze_jsonl(&pcl_dir, "commit", &["not valid json at all"]);

        let result = pipeline.transform_git().unwrap();
        assert_eq!(result.errors, 1);
        assert_eq!(result.inserted, 0);

        let log_path = pcl_dir.logs_dir().join("errors.log");
        assert!(log_path.exists());
        let log_content = fs::read_to_string(log_path).unwrap();
        assert!(log_content.contains("malformed record"));
    }

    #[test]
    fn transform_diffs() {
        let (_dir, pcl_dir, conn) = setup();
        let pipeline = TransformPipeline::new(&pcl_dir, &conn);
        pipeline.initialize_silver_tables().unwrap();

        let record = serde_json::json!({
            "source": "git",
            "timestamp": "2026-04-26T12:00:00+00:00",
            "schema_version": 1,
            "kind": "diff",
            "data": {
                "file": "src/main.rs",
                "insertions": 10,
                "deletions": 5,
                "repo_path": "/tmp/repo",
                "commit_sha": "abc123"
            }
        });
        write_bronze_jsonl(&pcl_dir, "diff", &[&record.to_string()]);

        let result = pipeline.transform_git().unwrap();
        assert_eq!(result.inserted, 1);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM git_diffs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn empty_bronze_dir() {
        let (_dir, pcl_dir, conn) = setup();
        let pipeline = TransformPipeline::new(&pcl_dir, &conn);
        pipeline.initialize_silver_tables().unwrap();

        let result = pipeline.transform_git().unwrap();
        assert_eq!(result.inserted, 0);
        assert_eq!(result.skipped, 0);
        assert_eq!(result.errors, 0);
    }

    #[test]
    fn normalize_rfc3339() {
        let result = normalize_timestamp("2026-04-26T12:00:00+05:30");
        assert!(result.contains("06:30:00"));
    }

    #[test]
    fn normalize_passthrough() {
        let result = normalize_timestamp("not a date");
        assert_eq!(result, "not a date");
    }
}
