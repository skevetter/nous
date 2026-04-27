use std::path::{Path, PathBuf};

use crate::error::PclError;

const SUBDIRS: &[&str] = &["bronze", "silver", "gold", "config", "logs"];

pub struct PclDirectory {
    root: PathBuf,
}

impl PclDirectory {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn initialize(&self) -> Result<(), PclError> {
        for subdir in SUBDIRS {
            std::fs::create_dir_all(self.root.join(subdir))?;
        }
        Ok(())
    }

    pub fn bronze_dir(&self, source: &str) -> PathBuf {
        self.root.join("bronze").join(source)
    }

    pub fn silver_dir(&self) -> PathBuf {
        self.root.join("silver")
    }

    pub fn gold_dir(&self) -> PathBuf {
        self.root.join("gold")
    }

    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
}

impl Default for PclDirectory {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        Self::new(PathBuf::from(home).join(".pcl"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_creates_all_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let pcl = PclDirectory::new(dir.path().to_path_buf());
        pcl.initialize().unwrap();

        for subdir in SUBDIRS {
            assert!(dir.path().join(subdir).is_dir(), "{subdir} should exist");
        }
    }

    #[test]
    fn initialize_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let pcl = PclDirectory::new(dir.path().to_path_buf());
        pcl.initialize().unwrap();
        pcl.initialize().unwrap();

        for subdir in SUBDIRS {
            assert!(dir.path().join(subdir).is_dir());
        }
    }

    #[test]
    fn bronze_dir_includes_source() {
        let pcl = PclDirectory::new(PathBuf::from("/tmp/pcl"));
        assert_eq!(pcl.bronze_dir("git"), PathBuf::from("/tmp/pcl/bronze/git"));
    }

    #[test]
    fn path_helpers() {
        let pcl = PclDirectory::new(PathBuf::from("/data/pcl"));
        assert_eq!(pcl.silver_dir(), PathBuf::from("/data/pcl/silver"));
        assert_eq!(pcl.gold_dir(), PathBuf::from("/data/pcl/gold"));
        assert_eq!(pcl.config_dir(), PathBuf::from("/data/pcl/config"));
        assert_eq!(pcl.logs_dir(), PathBuf::from("/data/pcl/logs"));
    }

    #[test]
    fn root_accessor() {
        let pcl = PclDirectory::new(PathBuf::from("/tmp/pcl"));
        assert_eq!(pcl.root(), Path::new("/tmp/pcl"));
    }
}
