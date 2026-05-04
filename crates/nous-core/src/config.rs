use serde::Deserialize;
use std::path::PathBuf;

use crate::error::NousError;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchConfig {}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default = "RateLimitConfig::default_requests_per_minute")]
    pub requests_per_minute: u32,
    #[serde(default = "RateLimitConfig::default_burst_size")]
    pub burst_size: u32,
}

impl RateLimitConfig {
    fn default_requests_per_minute() -> u32 {
        100
    }
    fn default_burst_size() -> u32 {
        25
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: Self::default_requests_per_minute(),
            burst_size: Self::default_burst_size(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SchedulerConfig {
    #[serde(default = "SchedulerConfig::default_max_concurrent")]
    pub max_concurrent: usize,
    #[serde(default = "SchedulerConfig::default_allow_shell")]
    pub allow_shell: bool,
    #[serde(default = "SchedulerConfig::default_timeout_secs")]
    pub default_timeout_secs: u64,
}

impl SchedulerConfig {
    fn default_max_concurrent() -> usize {
        4
    }
    fn default_allow_shell() -> bool {
        false
    }
    fn default_timeout_secs() -> u64 {
        300
    }
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent: Self::default_max_concurrent(),
            allow_shell: false,
            default_timeout_secs: Self::default_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub data_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            host: "127.0.0.1".to_string(),
            port: 8377,
            api_key: None,
            search: SearchConfig::default(),
            rate_limit: RateLimitConfig::default(),
            scheduler: SchedulerConfig::default(),
        }
    }
}

impl Config {
    pub fn resolve_api_key(&self) -> Option<String> {
        std::env::var("NOUS_API_KEY")
            .ok()
            .or_else(|| self.api_key.clone())
    }

    pub fn load() -> Result<Self, NousError> {
        let config_path = config_file_path();
        let config = match std::fs::read_to_string(&config_path) {
            Ok(contents) => toml::from_str(&contents).map_err(|e| {
                NousError::Config(format!("failed to parse {}: {e}", config_path.display()))
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                return Err(NousError::Config(format!(
                    "failed to read {}: {e}",
                    config_path.display()
                )));
            }
        };
        config.validate()?;
        Ok(config)
    }

    pub fn with_data_dir(mut self, data_dir: PathBuf) -> Result<Self, NousError> {
        self.data_dir = data_dir;
        self.validate()?;
        Ok(self)
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        if let Some(parent) = config_file_path().parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), NousError> {
        if self
            .data_dir
            .components()
            .any(|c| c == std::path::Component::ParentDir)
        {
            return Err(NousError::Validation(
                "data_dir must not contain '..' components".to_string(),
            ));
        }
        Ok(())
    }
}

pub fn config_file_path() -> PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        config_dir.join("nous").join("config.toml")
    } else {
        home_fallback().join("config.toml")
    }
}

fn default_data_dir() -> PathBuf {
    if let Some(data_dir) = dirs::data_dir() {
        data_dir.join("nous")
    } else {
        home_fallback()
    }
}

fn home_fallback() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".nous")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_correct() {
        let cfg = Config::default();
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 8377);
        assert!(cfg.data_dir.to_string_lossy().contains("nous"));
    }

    #[test]
    fn with_data_dir_overrides() {
        let cfg = Config::default()
            .with_data_dir(PathBuf::from("/tmp/test-nous"))
            .unwrap();
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/test-nous"));
    }

    #[test]
    fn with_data_dir_rejects_parent_traversal() {
        let result = Config::default().with_data_dir(PathBuf::from("/tmp/../etc/nous"));
        assert!(result.is_err());
    }

    #[test]
    fn load_returns_defaults_when_no_file() {
        let cfg = Config::load().unwrap();
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 8377);
    }

    #[test]
    fn parse_toml_config() {
        let toml_str = r#"
            data_dir = "/custom/data"
            host = "0.0.0.0"
            port = 9000
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.data_dir, PathBuf::from("/custom/data"));
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 9000);
    }

    #[test]
    fn partial_toml_uses_defaults_for_missing() {
        let toml_str = r#"
            port = 9999
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 9999);
    }

    #[test]
    fn malformed_toml_returns_error() {
        let toml_str = "port = not_a_number";
        let result: Result<Config, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }


    #[test]
    fn scheduler_config_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.scheduler.max_concurrent, 4);
        assert!(!cfg.scheduler.allow_shell);
        assert_eq!(cfg.scheduler.default_timeout_secs, 300);
    }

    #[test]
    fn scheduler_config_from_toml() {
        let toml_str = r#"
            [scheduler]
            allow_shell = true
            max_concurrent = 8
            default_timeout_secs = 600
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert!(cfg.scheduler.allow_shell);
        assert_eq!(cfg.scheduler.max_concurrent, 8);
        assert_eq!(cfg.scheduler.default_timeout_secs, 600);
    }

    #[test]
    fn api_key_from_config_file() {
        let toml_str = r#"
            api_key = "my-secret"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.api_key.as_deref(), Some("my-secret"));
    }

    #[test]
    fn api_key_default_is_none() {
        let cfg = Config::default();
        assert!(cfg.api_key.is_none());
    }

    #[test]
    fn resolve_api_key_prefers_env() {
        temp_env::with_var("NOUS_API_KEY", Some("env-key"), || {
            let mut cfg = Config::default();
            cfg.api_key = Some("file-key".into());
            assert_eq!(cfg.resolve_api_key().as_deref(), Some("env-key"));
        });
    }

    #[test]
    fn resolve_api_key_falls_back_to_config() {
        temp_env::with_var("NOUS_API_KEY", None::<&str>, || {
            let mut cfg = Config::default();
            cfg.api_key = Some("file-key".into());
            assert_eq!(cfg.resolve_api_key().as_deref(), Some("file-key"));
        });
    }
}
