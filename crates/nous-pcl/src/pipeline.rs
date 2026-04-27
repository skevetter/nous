use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;

use crate::collector::CollectorConfig;
use crate::directory::PclDirectory;
use crate::error::PclError;
use crate::registry::CollectorRegistry;

pub struct PipelineRunner<'a> {
    directory: &'a PclDirectory,
    registry: &'a CollectorRegistry,
}

impl<'a> PipelineRunner<'a> {
    pub fn new(directory: &'a PclDirectory, registry: &'a CollectorRegistry) -> Self {
        Self {
            directory,
            registry,
        }
    }

    pub fn run_collector(&self, name: &str, config: &CollectorConfig) -> Result<usize, PclError> {
        let collector = self
            .registry
            .get(name)
            .ok_or_else(|| PclError::CollectorNotFound(name.to_string()))?;

        let records = collector
            .collect(config)
            .map_err(|e| PclError::CollectorFailed {
                name: name.to_string(),
                source: Box::new(e),
            })?;

        let count = records.len();
        for record in &records {
            let dir = self.directory.bronze_dir(&record.source);
            fs::create_dir_all(&dir)?;

            let path = dir.join(format!("{}.jsonl", record.kind));
            let mut file = OpenOptions::new().create(true).append(true).open(&path)?;

            let line = serde_json::to_string(record)?;
            writeln!(file, "{line}")?;
        }

        Ok(count)
    }

    pub fn run_all(&self, configs: &[CollectorConfig]) -> Result<HashMap<String, usize>, PclError> {
        let mut results = HashMap::new();
        for config in configs {
            if !config.enabled {
                continue;
            }
            let count = self.run_collector(&config.name, config)?;
            results.insert(config.name.clone(), count);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::{Collector, CollectorConfig, Record};

    struct MockCollector;

    impl Collector for MockCollector {
        fn name(&self) -> &str {
            "mock"
        }

        fn collect(&self, _config: &CollectorConfig) -> Result<Vec<Record>, PclError> {
            Ok(vec![
                Record {
                    source: "mock".to_string(),
                    timestamp: "2026-04-26T12:00:00Z".to_string(),
                    schema_version: 1,
                    kind: "event".to_string(),
                    data: serde_json::json!({"id": 1}),
                },
                Record {
                    source: "mock".to_string(),
                    timestamp: "2026-04-26T12:01:00Z".to_string(),
                    schema_version: 1,
                    kind: "event".to_string(),
                    data: serde_json::json!({"id": 2}),
                },
                Record {
                    source: "mock".to_string(),
                    timestamp: "2026-04-26T12:02:00Z".to_string(),
                    schema_version: 1,
                    kind: "metric".to_string(),
                    data: serde_json::json!({"value": 42}),
                },
            ])
        }
    }

    fn setup() -> (tempfile::TempDir, PclDirectory, CollectorRegistry) {
        let dir = tempfile::tempdir().unwrap();
        let pcl_dir = PclDirectory::new(dir.path().to_path_buf());
        pcl_dir.initialize().unwrap();

        let mut registry = CollectorRegistry::new();
        registry.register(Box::new(MockCollector));

        (dir, pcl_dir, registry)
    }

    fn mock_config(enabled: bool) -> CollectorConfig {
        CollectorConfig {
            name: "mock".to_string(),
            enabled,
            settings: HashMap::new(),
        }
    }

    #[test]
    fn run_collector_writes_jsonl() {
        let (_dir, pcl_dir, registry) = setup();
        let runner = PipelineRunner::new(&pcl_dir, &registry);
        let config = mock_config(true);

        let count = runner.run_collector("mock", &config).unwrap();
        assert_eq!(count, 3);

        let event_path = pcl_dir.bronze_dir("mock").join("event.jsonl");
        let contents = std::fs::read_to_string(&event_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: Record = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first.data["id"], 1);

        let metric_path = pcl_dir.bronze_dir("mock").join("metric.jsonl");
        let metric_contents = std::fs::read_to_string(&metric_path).unwrap();
        assert_eq!(metric_contents.lines().count(), 1);
    }

    #[test]
    fn run_collector_appends() {
        let (_dir, pcl_dir, registry) = setup();
        let runner = PipelineRunner::new(&pcl_dir, &registry);
        let config = mock_config(true);

        runner.run_collector("mock", &config).unwrap();
        runner.run_collector("mock", &config).unwrap();

        let event_path = pcl_dir.bronze_dir("mock").join("event.jsonl");
        let contents = std::fs::read_to_string(&event_path).unwrap();
        assert_eq!(contents.lines().count(), 4);
    }

    #[test]
    fn run_collector_not_found() {
        let (_dir, pcl_dir, registry) = setup();
        let runner = PipelineRunner::new(&pcl_dir, &registry);
        let config = mock_config(true);

        let err = runner.run_collector("nonexistent", &config).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn run_all_skips_disabled() {
        let (_dir, pcl_dir, registry) = setup();
        let runner = PipelineRunner::new(&pcl_dir, &registry);

        let configs = vec![mock_config(true), mock_config(false)];
        let results = runner.run_all(&configs).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results["mock"], 3);
    }

    #[test]
    fn run_all_empty_configs() {
        let (_dir, pcl_dir, registry) = setup();
        let runner = PipelineRunner::new(&pcl_dir, &registry);

        let results = runner.run_all(&[]).unwrap();
        assert!(results.is_empty());
    }
}
