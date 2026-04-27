use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub source: String,
    pub timestamp: String,
    pub schema_version: u32,
    pub kind: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectorConfig {
    pub name: String,
    pub enabled: bool,
    #[serde(default)]
    pub settings: HashMap<String, serde_json::Value>,
}

pub trait Collector: Send + Sync {
    fn name(&self) -> &str;
    fn collect(&self, config: &CollectorConfig) -> Result<Vec<Record>, crate::error::PclError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_roundtrip() {
        let record = Record {
            source: "git".to_string(),
            timestamp: "2026-04-26T12:00:00Z".to_string(),
            schema_version: 1,
            kind: "commit".to_string(),
            data: serde_json::json!({"sha": "abc123", "message": "initial commit"}),
        };

        let json = serde_json::to_string(&record).unwrap();
        let deserialized: Record = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.source, "git");
        assert_eq!(deserialized.timestamp, "2026-04-26T12:00:00Z");
        assert_eq!(deserialized.schema_version, 1);
        assert_eq!(deserialized.kind, "commit");
        assert_eq!(deserialized.data["sha"], "abc123");
    }

    #[test]
    fn collector_config_roundtrip() {
        let config = CollectorConfig {
            name: "git".to_string(),
            enabled: true,
            settings: HashMap::from([("repo_path".to_string(), serde_json::json!("/tmp/repo"))]),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: CollectorConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.name, "git");
        assert!(deserialized.enabled);
        assert_eq!(deserialized.settings["repo_path"], "/tmp/repo");
    }

    #[test]
    fn collector_config_default_settings() {
        let json = r#"{"name": "git", "enabled": true}"#;
        let config: CollectorConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.name, "git");
        assert!(config.settings.is_empty());
    }
}
