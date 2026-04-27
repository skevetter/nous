use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::PclError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMetadata {
    pub collector_name: String,
    pub last_run: String,
    pub records_collected: usize,
    pub duration_ms: u64,
}

const METADATA_FILE: &str = "last_run.json";

impl RunMetadata {
    pub fn save(&self, config_dir: &Path) -> Result<(), PclError> {
        std::fs::create_dir_all(config_dir)?;
        let path = config_dir.join(METADATA_FILE);

        let mut all = Self::load_all(config_dir)?;
        all.insert(self.collector_name.clone(), self.clone());

        let json = serde_json::to_string_pretty(&all)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(config_dir: &Path, collector: &str) -> Result<Option<RunMetadata>, PclError> {
        let all = Self::load_all(config_dir)?;
        Ok(all.get(collector).cloned())
    }

    fn load_all(config_dir: &Path) -> Result<HashMap<String, RunMetadata>, PclError> {
        let path = config_dir.join(METADATA_FILE);
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let contents = std::fs::read_to_string(path)?;
        let map: HashMap<String, RunMetadata> = serde_json::from_str(&contents)?;
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let meta = RunMetadata {
            collector_name: "git".into(),
            last_run: "2026-04-26T12:00:00+00:00".into(),
            records_collected: 42,
            duration_ms: 1500,
        };

        meta.save(dir.path()).unwrap();
        let loaded = RunMetadata::load(dir.path(), "git").unwrap().unwrap();

        assert_eq!(loaded.collector_name, "git");
        assert_eq!(loaded.records_collected, 42);
        assert_eq!(loaded.duration_ms, 1500);
        assert_eq!(loaded.last_run, "2026-04-26T12:00:00+00:00");
    }

    #[test]
    fn load_missing_collector() {
        let dir = tempfile::tempdir().unwrap();
        let result = RunMetadata::load(dir.path(), "nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn save_multiple_collectors() {
        let dir = tempfile::tempdir().unwrap();

        let git_meta = RunMetadata {
            collector_name: "git".into(),
            last_run: "2026-04-26T12:00:00+00:00".into(),
            records_collected: 10,
            duration_ms: 500,
        };
        git_meta.save(dir.path()).unwrap();

        let jira_meta = RunMetadata {
            collector_name: "jira".into(),
            last_run: "2026-04-26T13:00:00+00:00".into(),
            records_collected: 20,
            duration_ms: 800,
        };
        jira_meta.save(dir.path()).unwrap();

        let git = RunMetadata::load(dir.path(), "git").unwrap().unwrap();
        assert_eq!(git.records_collected, 10);

        let jira = RunMetadata::load(dir.path(), "jira").unwrap().unwrap();
        assert_eq!(jira.records_collected, 20);
    }

    #[test]
    fn save_overwrites_same_collector() {
        let dir = tempfile::tempdir().unwrap();

        let meta1 = RunMetadata {
            collector_name: "git".into(),
            last_run: "2026-04-26T12:00:00+00:00".into(),
            records_collected: 10,
            duration_ms: 500,
        };
        meta1.save(dir.path()).unwrap();

        let meta2 = RunMetadata {
            collector_name: "git".into(),
            last_run: "2026-04-26T14:00:00+00:00".into(),
            records_collected: 25,
            duration_ms: 700,
        };
        meta2.save(dir.path()).unwrap();

        let loaded = RunMetadata::load(dir.path(), "git").unwrap().unwrap();
        assert_eq!(loaded.records_collected, 25);
        assert_eq!(loaded.last_run, "2026-04-26T14:00:00+00:00");
    }
}
