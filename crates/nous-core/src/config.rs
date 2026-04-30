use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub data_dir: PathBuf,
    pub host: String,
    pub port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            host: "127.0.0.1".to_string(),
            port: 8377,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let config_path = config_file_path();
        match std::fs::read_to_string(&config_path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn with_data_dir(mut self, data_dir: PathBuf) -> Self {
        self.data_dir = data_dir;
        self
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        if let Some(parent) = config_file_path().parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}

fn config_file_path() -> PathBuf {
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
        let cfg = Config::default().with_data_dir(PathBuf::from("/tmp/test-nous"));
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/test-nous"));
    }

    #[test]
    fn load_returns_defaults_when_no_file() {
        let cfg = Config::load();
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
}
