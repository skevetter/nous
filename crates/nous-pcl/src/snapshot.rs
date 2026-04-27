use std::fmt;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::directory::PclDirectory;
use crate::error::PclError;

const STALE_MINUTES: i64 = 15;
const INDEX_FILE: &str = "_index.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotLevel {
    Synopsis,
    Standard,
    Detailed,
}

impl SnapshotLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Synopsis => "synopsis",
            Self::Standard => "standard",
            Self::Detailed => "detailed",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "synopsis" => Self::Synopsis,
            "detailed" => Self::Detailed,
            _ => Self::Standard,
        }
    }
}

impl fmt::Display for SnapshotLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotIndex {
    pub entries: Vec<SnapshotMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    pub subject: String,
    pub generated_at: String,
    pub stale_after: String,
    pub level: String,
    pub path: String,
}

impl SnapshotMeta {
    pub fn is_stale(&self) -> bool {
        let stale_after = match DateTime::parse_from_rfc3339(&self.stale_after) {
            Ok(dt) => dt.to_utc(),
            Err(_) => return true,
        };
        Utc::now() > stale_after
    }
}

pub struct SnapshotGenerator<'a> {
    directory: &'a PclDirectory,
    conn: &'a Connection,
}

struct GitStats {
    commit_count: i64,
    branch_count: i64,
    total_insertions: i64,
    total_deletions: i64,
    files_changed_count: i64,
}

struct CommitRow {
    sha: String,
    author: String,
    date: String,
    message: String,
}

struct BranchRow {
    name: String,
    is_remote: bool,
    is_current: bool,
}

struct DiffRow {
    file: String,
    insertions: i64,
    deletions: i64,
}

impl<'a> SnapshotGenerator<'a> {
    pub fn new(directory: &'a PclDirectory, conn: &'a Connection) -> Self {
        Self { directory, conn }
    }

    pub fn generate_task_snapshot(
        &self,
        subject: &str,
        level: SnapshotLevel,
    ) -> Result<String, PclError> {
        let now = Utc::now();
        let stale_after = now + Duration::minutes(STALE_MINUTES);

        let stats = self.query_stats(subject)?;
        let latest_commit = self.query_latest_commit(subject)?;
        let branches = self.query_branches(subject)?;

        let mut md = String::new();

        // YAML frontmatter
        md.push_str("---\n");
        md.push_str(&format!("generated_at: {}\n", now.to_rfc3339()));
        md.push_str(&format!("stale_after: {}\n", stale_after.to_rfc3339()));
        md.push_str("scope: task\n");
        md.push_str(&format!("subject: {subject}\n"));
        md.push_str(&format!("level: {level}\n"));
        md.push_str("---\n\n");

        // Header
        md.push_str(&format!("# {subject} — Snapshot\n\n"));

        // Status line
        md.push_str(&format!(
            "**Status:** {} commits, {} branches, {} files changed\n",
            stats.commit_count, stats.branch_count, stats.files_changed_count,
        ));
        md.push_str(&format!("**Updated:** {}\n\n", now.to_rfc3339()));

        // Key Facts
        md.push_str("## Key Facts\n");
        if let Some(ref c) = latest_commit {
            md.push_str(&format!(
                "- Latest commit: {} — {}\n",
                &c.sha[..7.min(c.sha.len())],
                c.message
            ));
        } else {
            md.push_str("- No commits found\n");
        }
        md.push_str(&format!(
            "- {} branch(es); active: {}\n",
            stats.branch_count,
            branches
                .iter()
                .find(|b| b.is_current)
                .map(|b| b.name.as_str())
                .unwrap_or("unknown")
        ));
        md.push_str(&format!(
            "- Total: +{} / -{}\n",
            stats.total_insertions, stats.total_deletions
        ));

        if level == SnapshotLevel::Synopsis {
            return Ok(md);
        }

        // Details section (standard + detailed)
        md.push('\n');
        md.push_str("## Details\n\n");

        // Recent commits table
        let commits = self.query_recent_commits(subject, 10)?;
        md.push_str("### Recent Commits\n");
        md.push_str("| SHA | Author | Date | Message |\n");
        md.push_str("|-----|--------|------|---------|\n");
        for c in &commits {
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                &c.sha[..7.min(c.sha.len())],
                c.author,
                &c.date[..10.min(c.date.len())],
                c.message
            ));
        }
        md.push('\n');

        // Branches
        md.push_str("### Branches\n");
        for b in &branches {
            let suffix = if b.is_current {
                " (current)"
            } else if b.is_remote {
                " (remote)"
            } else {
                ""
            };
            md.push_str(&format!("- {}{suffix}\n", b.name));
        }

        if level == SnapshotLevel::Standard {
            return Ok(md);
        }

        // File Changes (detailed only)
        md.push('\n');
        let diffs = self.query_diffs(subject)?;
        md.push_str("### File Changes\n");
        md.push_str("| File | +/- |\n");
        md.push_str("|------|-----|\n");
        for d in &diffs {
            md.push_str(&format!(
                "| {} | +{}/-{} |\n",
                d.file, d.insertions, d.deletions
            ));
        }

        // Not Applicable section (detailed only)
        md.push_str("\n## Not Applicable\n");
        md.push_str("- CI pipeline data (not yet collected)\n");
        md.push_str("- Ticket/issue data (not yet collected)\n");

        Ok(md)
    }

    pub fn write_snapshot(
        &self,
        subject: &str,
        content: &str,
        level: SnapshotLevel,
    ) -> Result<PathBuf, PclError> {
        let gold_dir = self.directory.gold_dir();
        fs::create_dir_all(&gold_dir)?;

        let filename = sanitize_filename(subject);
        let path = gold_dir.join(format!("{filename}.md"));
        fs::write(&path, content)?;

        let now = Utc::now();
        let stale_after = now + Duration::minutes(STALE_MINUTES);

        let meta = SnapshotMeta {
            subject: subject.to_string(),
            generated_at: now.to_rfc3339(),
            stale_after: stale_after.to_rfc3339(),
            level: level.to_string(),
            path: format!("{filename}.md"),
        };

        self.update_index(&meta)?;

        Ok(path)
    }

    pub fn load_index(&self) -> Result<SnapshotIndex, PclError> {
        let index_path = self.directory.gold_dir().join(INDEX_FILE);
        if !index_path.exists() {
            return Ok(SnapshotIndex {
                entries: Vec::new(),
            });
        }
        let content = fs::read_to_string(index_path)?;
        let index: SnapshotIndex = serde_json::from_str(&content)?;
        Ok(index)
    }

    fn update_index(&self, meta: &SnapshotMeta) -> Result<(), PclError> {
        let mut index = self.load_index()?;
        index.entries.retain(|e| e.subject != meta.subject);
        index.entries.push(meta.clone());

        let index_path = self.directory.gold_dir().join(INDEX_FILE);
        let json = serde_json::to_string_pretty(&index)?;
        fs::write(index_path, json)?;
        Ok(())
    }

    fn query_stats(&self, subject: &str) -> Result<GitStats, PclError> {
        let where_clause = subject_where_clause(subject);

        let commit_count: i64 = self
            .conn
            .query_row(
                &format!("SELECT COUNT(*) FROM git_commits {where_clause}"),
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        let branch_count: i64 = self
            .conn
            .query_row(
                &format!("SELECT COUNT(*) FROM git_branches {where_clause}"),
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        let (total_insertions, total_deletions): (i64, i64) = self
            .conn
            .query_row(
                &format!(
                    "SELECT COALESCE(SUM(insertions), 0), COALESCE(SUM(deletions), 0) FROM git_diffs {where_clause}"
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0));

        let files_changed_count: i64 = self
            .conn
            .query_row(
                &format!("SELECT COUNT(DISTINCT file) FROM git_diffs {where_clause}"),
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        Ok(GitStats {
            commit_count,
            branch_count,
            total_insertions,
            total_deletions,
            files_changed_count,
        })
    }

    fn query_latest_commit(&self, subject: &str) -> Result<Option<CommitRow>, PclError> {
        let where_clause = subject_where_clause(subject);
        let sql = format!(
            "SELECT sha, author, date, message FROM git_commits {where_clause} ORDER BY date DESC LIMIT 1"
        );
        let result = self.conn.query_row(&sql, [], |r| {
            Ok(CommitRow {
                sha: r.get(0)?,
                author: r.get(1)?,
                date: r.get(2)?,
                message: r.get(3)?,
            })
        });
        match result {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(PclError::CollectorFailed {
                name: "snapshot".into(),
                source: Box::new(e),
            }),
        }
    }

    fn query_recent_commits(
        &self,
        subject: &str,
        limit: usize,
    ) -> Result<Vec<CommitRow>, PclError> {
        let where_clause = subject_where_clause(subject);
        let sql = format!(
            "SELECT sha, author, date, message FROM git_commits {where_clause} ORDER BY date DESC LIMIT ?1"
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| PclError::CollectorFailed {
                name: "snapshot".into(),
                source: Box::new(e),
            })?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(CommitRow {
                    sha: r.get(0)?,
                    author: r.get(1)?,
                    date: r.get(2)?,
                    message: r.get(3)?,
                })
            })
            .map_err(|e| PclError::CollectorFailed {
                name: "snapshot".into(),
                source: Box::new(e),
            })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| PclError::CollectorFailed {
                name: "snapshot".into(),
                source: Box::new(e),
            })?);
        }
        Ok(result)
    }

    fn query_branches(&self, subject: &str) -> Result<Vec<BranchRow>, PclError> {
        let where_clause = subject_where_clause(subject);
        let sql = format!(
            "SELECT name, is_remote, is_current FROM git_branches {where_clause} ORDER BY is_current DESC, name ASC"
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| PclError::CollectorFailed {
                name: "snapshot".into(),
                source: Box::new(e),
            })?;
        let rows = stmt
            .query_map([], |r| {
                Ok(BranchRow {
                    name: r.get(0)?,
                    is_remote: r.get::<_, i32>(1)? != 0,
                    is_current: r.get::<_, i32>(2)? != 0,
                })
            })
            .map_err(|e| PclError::CollectorFailed {
                name: "snapshot".into(),
                source: Box::new(e),
            })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| PclError::CollectorFailed {
                name: "snapshot".into(),
                source: Box::new(e),
            })?);
        }
        Ok(result)
    }

    fn query_diffs(&self, subject: &str) -> Result<Vec<DiffRow>, PclError> {
        let where_clause = subject_where_clause(subject);
        let sql = format!(
            "SELECT file, COALESCE(SUM(insertions), 0), COALESCE(SUM(deletions), 0) FROM git_diffs {where_clause} GROUP BY file ORDER BY (COALESCE(SUM(insertions), 0) + COALESCE(SUM(deletions), 0)) DESC"
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| PclError::CollectorFailed {
                name: "snapshot".into(),
                source: Box::new(e),
            })?;
        let rows = stmt
            .query_map([], |r| {
                Ok(DiffRow {
                    file: r.get(0)?,
                    insertions: r.get(1)?,
                    deletions: r.get(2)?,
                })
            })
            .map_err(|e| PclError::CollectorFailed {
                name: "snapshot".into(),
                source: Box::new(e),
            })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| PclError::CollectorFailed {
                name: "snapshot".into(),
                source: Box::new(e),
            })?);
        }
        Ok(result)
    }
}

fn subject_where_clause(subject: &str) -> String {
    if subject == "all" {
        String::new()
    } else {
        // Empty where clause — we don't filter by branch in the silver tables
        // since silver stores repo-wide data. Subject is used for labeling only.
        String::new()
    }
}

fn sanitize_filename(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_silver_db(conn: &Connection) {
        conn.execute_batch(
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
                ingested_at TEXT NOT NULL,
                UNIQUE(repo_path, commit_sha, file)
            );",
        )
        .unwrap();
    }

    fn seed_test_data(conn: &Connection) {
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO git_commits (sha, author, date, message, files_changed, repo_path, ingested_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params!["abc1234", "Alice", "2026-04-26T10:00:00+00:00", "initial commit", 2, "/tmp/repo", now],
        ).unwrap();
        conn.execute(
            "INSERT INTO git_commits (sha, author, date, message, files_changed, repo_path, ingested_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params!["def5678", "Bob", "2026-04-26T11:00:00+00:00", "add feature", 3, "/tmp/repo", now],
        ).unwrap();

        conn.execute(
            "INSERT INTO git_branches (name, is_remote, is_current, repo_path, ingested_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["main", 0, 1, "/tmp/repo", now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO git_branches (name, is_remote, is_current, repo_path, ingested_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["feature-x", 0, 0, "/tmp/repo", now],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO git_diffs (file, insertions, deletions, repo_path, commit_sha, ingested_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["src/main.rs", 10, 3, "/tmp/repo", "def5678", now],
        ).unwrap();
        conn.execute(
            "INSERT INTO git_diffs (file, insertions, deletions, repo_path, commit_sha, ingested_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["README.md", 5, 0, "/tmp/repo", "def5678", now],
        ).unwrap();
    }

    #[test]
    fn synopsis_has_frontmatter_and_key_facts() {
        let dir = tempfile::tempdir().unwrap();
        let pcl = PclDirectory::new(dir.path().to_path_buf());
        pcl.initialize().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        setup_silver_db(&conn);
        seed_test_data(&conn);

        let snap = SnapshotGenerator::new(&pcl, &conn);
        let md = snap
            .generate_task_snapshot("all", SnapshotLevel::Synopsis)
            .unwrap();

        assert!(md.starts_with("---\n"));
        assert!(md.contains("generated_at:"));
        assert!(md.contains("stale_after:"));
        assert!(md.contains("scope: task"));
        assert!(md.contains("subject: all"));
        assert!(md.contains("level: synopsis"));
        assert!(md.contains("## Key Facts"));
        assert!(md.contains("2 commits"));
        assert!(md.contains("2 branches"));
        // Synopsis should NOT contain Details section
        assert!(!md.contains("## Details"));
    }

    #[test]
    fn standard_includes_details() {
        let dir = tempfile::tempdir().unwrap();
        let pcl = PclDirectory::new(dir.path().to_path_buf());
        pcl.initialize().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        setup_silver_db(&conn);
        seed_test_data(&conn);

        let snap = SnapshotGenerator::new(&pcl, &conn);
        let md = snap
            .generate_task_snapshot("all", SnapshotLevel::Standard)
            .unwrap();

        assert!(md.contains("## Details"));
        assert!(md.contains("### Recent Commits"));
        assert!(md.contains("### Branches"));
        assert!(md.contains("def5678"));
        assert!(md.contains("main (current)"));
        // Standard should NOT contain File Changes
        assert!(!md.contains("### File Changes"));
    }

    #[test]
    fn detailed_includes_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let pcl = PclDirectory::new(dir.path().to_path_buf());
        pcl.initialize().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        setup_silver_db(&conn);
        seed_test_data(&conn);

        let snap = SnapshotGenerator::new(&pcl, &conn);
        let md = snap
            .generate_task_snapshot("all", SnapshotLevel::Detailed)
            .unwrap();

        assert!(md.contains("### File Changes"));
        assert!(md.contains("src/main.rs"));
        assert!(md.contains("## Not Applicable"));
        assert!(md.contains("CI pipeline data"));
    }

    #[test]
    fn synopsis_shorter_than_standard_shorter_than_detailed() {
        let dir = tempfile::tempdir().unwrap();
        let pcl = PclDirectory::new(dir.path().to_path_buf());
        pcl.initialize().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        setup_silver_db(&conn);
        seed_test_data(&conn);

        let snap = SnapshotGenerator::new(&pcl, &conn);
        let synopsis = snap
            .generate_task_snapshot("all", SnapshotLevel::Synopsis)
            .unwrap();
        let standard = snap
            .generate_task_snapshot("all", SnapshotLevel::Standard)
            .unwrap();
        let detailed = snap
            .generate_task_snapshot("all", SnapshotLevel::Detailed)
            .unwrap();

        assert!(synopsis.len() < standard.len());
        assert!(standard.len() < detailed.len());
    }

    #[test]
    fn write_snapshot_creates_gold_file() {
        let dir = tempfile::tempdir().unwrap();
        let pcl = PclDirectory::new(dir.path().to_path_buf());
        pcl.initialize().unwrap();
        let conn = Connection::open_in_memory().unwrap();

        let snap = SnapshotGenerator::new(&pcl, &conn);
        let path = snap
            .write_snapshot("test-branch", "# content", SnapshotLevel::Standard)
            .unwrap();

        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "# content");
        assert_eq!(path.file_name().unwrap(), "test-branch.md");
    }

    #[test]
    fn write_snapshot_creates_index() {
        let dir = tempfile::tempdir().unwrap();
        let pcl = PclDirectory::new(dir.path().to_path_buf());
        pcl.initialize().unwrap();
        let conn = Connection::open_in_memory().unwrap();

        let snap = SnapshotGenerator::new(&pcl, &conn);
        snap.write_snapshot("branch-a", "content a", SnapshotLevel::Synopsis)
            .unwrap();
        snap.write_snapshot("branch-b", "content b", SnapshotLevel::Standard)
            .unwrap();

        let index = snap.load_index().unwrap();
        assert_eq!(index.entries.len(), 2);
        assert_eq!(index.entries[0].subject, "branch-a");
        assert_eq!(index.entries[0].level, "synopsis");
        assert_eq!(index.entries[1].subject, "branch-b");
        assert_eq!(index.entries[1].level, "standard");

        let index_path = pcl.gold_dir().join(INDEX_FILE);
        assert!(index_path.exists());
    }

    #[test]
    fn write_snapshot_updates_existing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let pcl = PclDirectory::new(dir.path().to_path_buf());
        pcl.initialize().unwrap();
        let conn = Connection::open_in_memory().unwrap();

        let snap = SnapshotGenerator::new(&pcl, &conn);
        snap.write_snapshot("branch-a", "v1", SnapshotLevel::Synopsis)
            .unwrap();
        snap.write_snapshot("branch-a", "v2", SnapshotLevel::Detailed)
            .unwrap();

        let index = snap.load_index().unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].level, "detailed");
    }

    #[test]
    fn stale_detection() {
        let past = (Utc::now() - Duration::minutes(20)).to_rfc3339();
        let future = (Utc::now() + Duration::minutes(20)).to_rfc3339();

        let stale_meta = SnapshotMeta {
            subject: "test".into(),
            generated_at: past.clone(),
            stale_after: past,
            level: "standard".into(),
            path: "test.md".into(),
        };
        assert!(stale_meta.is_stale());

        let fresh_meta = SnapshotMeta {
            subject: "test".into(),
            generated_at: Utc::now().to_rfc3339(),
            stale_after: future,
            level: "standard".into(),
            path: "test.md".into(),
        };
        assert!(!fresh_meta.is_stale());
    }

    #[test]
    fn sanitize_filename_strips_special_chars() {
        assert_eq!(sanitize_filename("feature/my-branch"), "feature_my-branch");
        assert_eq!(sanitize_filename("all"), "all");
        assert_eq!(sanitize_filename("a b c"), "a_b_c");
        assert_eq!(sanitize_filename("test@#$%"), "test____");
    }

    #[test]
    fn snapshot_level_round_trip() {
        assert_eq!(
            SnapshotLevel::from_str_loose("synopsis"),
            SnapshotLevel::Synopsis
        );
        assert_eq!(
            SnapshotLevel::from_str_loose("standard"),
            SnapshotLevel::Standard
        );
        assert_eq!(
            SnapshotLevel::from_str_loose("detailed"),
            SnapshotLevel::Detailed
        );
        assert_eq!(
            SnapshotLevel::from_str_loose("DETAILED"),
            SnapshotLevel::Detailed
        );
        assert_eq!(
            SnapshotLevel::from_str_loose("unknown"),
            SnapshotLevel::Standard
        );
    }

    #[test]
    fn empty_db_produces_valid_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let pcl = PclDirectory::new(dir.path().to_path_buf());
        pcl.initialize().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        setup_silver_db(&conn);

        let snap = SnapshotGenerator::new(&pcl, &conn);
        let md = snap
            .generate_task_snapshot("all", SnapshotLevel::Detailed)
            .unwrap();

        assert!(md.contains("0 commits"));
        assert!(md.contains("0 branches"));
        assert!(md.contains("No commits found"));
    }
}
