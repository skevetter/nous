use std::collections::HashMap;
use std::path::Path;

use crate::collector::{Collector, CollectorConfig};
use crate::error::PclError;

pub struct CollectorRegistry {
    collectors: HashMap<String, Box<dyn Collector>>,
}

impl CollectorRegistry {
    pub fn new() -> Self {
        Self {
            collectors: HashMap::new(),
        }
    }

    pub fn register(&mut self, collector: Box<dyn Collector>) {
        self.collectors
            .insert(collector.name().to_string(), collector);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Collector> {
        self.collectors.get(name).map(|c| c.as_ref())
    }

    pub fn load_config(path: &Path) -> Result<Vec<CollectorConfig>, PclError> {
        let contents = std::fs::read_to_string(path)?;
        let configs: Vec<CollectorConfig> = serde_json::from_str(&contents)?;
        Ok(configs)
    }

    pub fn list_collectors(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.collectors.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }
}

impl Default for CollectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::Record;

    struct StubCollector {
        stub_name: String,
    }

    impl Collector for StubCollector {
        fn name(&self) -> &str {
            &self.stub_name
        }

        fn collect(&self, _config: &CollectorConfig) -> Result<Vec<Record>, PclError> {
            Ok(vec![])
        }
    }

    #[test]
    fn register_and_get() {
        let mut registry = CollectorRegistry::new();
        registry.register(Box::new(StubCollector {
            stub_name: "git".to_string(),
        }));

        assert!(registry.get("git").is_some());
        assert_eq!(registry.get("git").unwrap().name(), "git");
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn list_collectors_sorted() {
        let mut registry = CollectorRegistry::new();
        registry.register(Box::new(StubCollector {
            stub_name: "zeta".to_string(),
        }));
        registry.register(Box::new(StubCollector {
            stub_name: "alpha".to_string(),
        }));
        registry.register(Box::new(StubCollector {
            stub_name: "mid".to_string(),
        }));

        assert_eq!(registry.list_collectors(), vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn load_config_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("collectors.json");
        std::fs::write(
            &config_path,
            r#"[{"name": "git", "enabled": true}, {"name": "jira", "enabled": false}]"#,
        )
        .unwrap();

        let configs = CollectorRegistry::load_config(&config_path).unwrap();
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].name, "git");
        assert!(configs[0].enabled);
        assert_eq!(configs[1].name, "jira");
        assert!(!configs[1].enabled);
    }

    #[test]
    fn load_config_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("collectors.json");
        std::fs::write(&config_path, "not json").unwrap();

        assert!(CollectorRegistry::load_config(&config_path).is_err());
    }

    #[test]
    fn default_creates_empty_registry() {
        let registry = CollectorRegistry::default();
        assert!(registry.list_collectors().is_empty());
    }
}
