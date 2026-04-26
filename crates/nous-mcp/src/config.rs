use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml parse error: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("config error: {0}")]
    Other(String),
}

type Result<T> = std::result::Result<T, ConfigError>;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub memory: MemoryConfig,
    pub embedding: EmbeddingConfig,
    pub otlp: OtlpConfig,
    pub classification: ClassificationConfig,
    pub encryption: EncryptionConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub db_path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    pub model: String,
    pub variant: String,
    pub chunk_size: usize,
    pub chunk_overlap: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct OtlpConfig {
    pub db_path: String,
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ClassificationConfig {
    pub confidence_threshold: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct EncryptionConfig {
    pub db_key_file: String,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            db_path: "~/.cache/nous/memory.db".into(),
        }
    }
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model: "onnx-community/Qwen3-Embedding-0.6B-ONNX".into(),
            variant: "model_q4f16.onnx".into(),
            chunk_size: 512,
            chunk_overlap: 64,
        }
    }
}

impl Default for OtlpConfig {
    fn default() -> Self {
        Self {
            db_path: "~/.cache/nous/otlp.db".into(),
            port: 4318,
        }
    }
}

impl Default for ClassificationConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.3,
        }
    }
}

impl Default for EncryptionConfig {
    fn default() -> Self {
        Self {
            db_key_file: "~/.config/nous/db.key".into(),
        }
    }
}

const DEFAULT_CONFIG_TOML: &str = r#"[memory]
db_path = "~/.cache/nous/memory.db"

[embedding]
model = "onnx-community/Qwen3-Embedding-0.6B-ONNX"
variant = "model_q4f16.onnx"
chunk_size = 512
chunk_overlap = 64

[otlp]
db_path = "~/.cache/nous/otlp.db"
port = 4318

[classification]
confidence_threshold = 0.3

[encryption]
db_key_file = "~/.config/nous/db.key"
"#;

fn default_config_path() -> std::result::Result<PathBuf, ConfigError> {
    nous_shared::xdg::config_path("config.toml").map_err(|e| ConfigError::Other(e.to_string()))
}

impl Config {
    pub fn load(path: Option<PathBuf>) -> Result<Self> {
        let path = match path {
            Some(p) => p,
            None => default_config_path()?,
        };

        let contents = if path.exists() {
            std::fs::read_to_string(&path)?
        } else {
            write_default_config(&path)?;
            DEFAULT_CONFIG_TOML.to_string()
        };

        Self::load_from_str(&contents)
    }

    pub fn load_from_str(s: &str) -> Result<Self> {
        let mut cfg: Config = toml::from_str(s)?;
        Self::apply_env_overrides(&mut cfg);
        Ok(cfg)
    }

    pub fn resolve_db_key(&self) -> std::result::Result<String, nous_shared::NousError> {
        let key_path = PathBuf::from(&self.encryption.db_key_file);
        nous_shared::sqlite::resolve_key_with_path(&key_path)
    }

    fn apply_env_overrides(cfg: &mut Config) {
        if let Ok(val) = std::env::var("NOUS_MEMORY_DB")
            && !val.is_empty()
        {
            cfg.memory.db_path = val;
        }
        if let Ok(val) = std::env::var("NOUS_OTLP_DB")
            && !val.is_empty()
        {
            cfg.otlp.db_path = val;
        }
        if let Ok(val) = std::env::var("NOUS_DB_KEY_FILE")
            && !val.is_empty()
        {
            cfg.encryption.db_key_file = val;
        }
    }
}

fn write_default_config(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, DEFAULT_CONFIG_TOML)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const FULL_TOML: &str = r#"
[memory]
db_path = "~/.cache/nous/memory.db"

[embedding]
model = "onnx-community/Qwen3-Embedding-0.6B-ONNX"
variant = "model_q4f16.onnx"
chunk_size = 512
chunk_overlap = 64

[otlp]
db_path = "~/.cache/nous/otlp.db"
port = 4318

[classification]
confidence_threshold = 0.3

[encryption]
db_key_file = "~/.config/nous/db.key"
"#;

    #[test]
    fn parse_full_config() {
        let cfg: Config = toml::from_str(FULL_TOML).unwrap();
        assert_eq!(cfg.memory.db_path, "~/.cache/nous/memory.db");
        assert_eq!(
            cfg.embedding.model,
            "onnx-community/Qwen3-Embedding-0.6B-ONNX"
        );
        assert_eq!(cfg.embedding.variant, "model_q4f16.onnx");
        assert_eq!(cfg.embedding.chunk_size, 512);
        assert_eq!(cfg.embedding.chunk_overlap, 64);
        assert_eq!(cfg.otlp.db_path, "~/.cache/nous/otlp.db");
        assert_eq!(cfg.otlp.port, 4318);
        assert_eq!(cfg.classification.confidence_threshold, 0.3);
        assert_eq!(cfg.encryption.db_key_file, "~/.config/nous/db.key");
    }

    #[test]
    fn parse_with_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();
        let cfg = Config::load_from_str("").unwrap();
        assert_eq!(cfg.memory.db_path, "~/.cache/nous/memory.db");
        assert_eq!(
            cfg.embedding.model,
            "onnx-community/Qwen3-Embedding-0.6B-ONNX"
        );
        assert_eq!(cfg.embedding.variant, "model_q4f16.onnx");
        assert_eq!(cfg.embedding.chunk_size, 512);
        assert_eq!(cfg.embedding.chunk_overlap, 64);
        assert_eq!(cfg.otlp.db_path, "~/.cache/nous/otlp.db");
        assert_eq!(cfg.otlp.port, 4318);
        assert_eq!(cfg.classification.confidence_threshold, 0.3);
        assert_eq!(cfg.encryption.db_key_file, "~/.config/nous/db.key");
    }

    #[test]
    fn env_overrides_memory_db_path() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("NOUS_MEMORY_DB", "/custom/memory.db") };
        let cfg = Config::load_from_str(FULL_TOML).unwrap();
        unsafe { std::env::remove_var("NOUS_MEMORY_DB") };
        assert_eq!(cfg.memory.db_path, "/custom/memory.db");
    }

    #[test]
    fn env_overrides_otlp_db_path() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("NOUS_OTLP_DB", "/custom/otlp.db") };
        let cfg = Config::load_from_str(FULL_TOML).unwrap();
        unsafe { std::env::remove_var("NOUS_OTLP_DB") };
        assert_eq!(cfg.otlp.db_path, "/custom/otlp.db");
    }

    #[test]
    fn env_overrides_db_key_file() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("NOUS_DB_KEY_FILE", "/custom/db.key") };
        let cfg = Config::load_from_str(FULL_TOML).unwrap();
        unsafe { std::env::remove_var("NOUS_DB_KEY_FILE") };
        assert_eq!(cfg.encryption.db_key_file, "/custom/db.key");
    }

    #[test]
    fn invalid_toml_returns_error() {
        let result = Config::load_from_str("this is [not valid toml!!!");
        assert!(result.is_err());
    }

    #[test]
    fn missing_config_file_creates_default() {
        let _lock = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("nous-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config_path = dir.join("config.toml");

        assert!(!config_path.exists());
        let cfg = Config::load(Some(config_path.clone())).unwrap();
        assert!(config_path.exists());

        assert_eq!(cfg.memory.db_path, "~/.cache/nous/memory.db");
        assert_eq!(cfg.embedding.chunk_size, 512);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_key_uses_config_db_key_file() {
        let _lock = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("nous-key-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let key_file = dir.join("test.key");
        std::fs::write(&key_file, "test-secret-key").unwrap();

        let toml_str = format!(
            r#"
[memory]
db_path = "~/.cache/nous/memory.db"

[embedding]
model = "onnx-community/Qwen3-Embedding-0.6B-ONNX"
variant = "model_q4f16.onnx"
chunk_size = 512
chunk_overlap = 64

[otlp]
db_path = "~/.cache/nous/otlp.db"
port = 4318

[classification]
confidence_threshold = 0.3

[encryption]
db_key_file = "{}"
"#,
            key_file.display()
        );

        unsafe { std::env::remove_var("NOUS_DB_KEY") };
        let cfg = Config::load_from_str(&toml_str).unwrap();
        let key = cfg.resolve_db_key().unwrap();
        assert_eq!(key, "test-secret-key");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
