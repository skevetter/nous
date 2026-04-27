use std::path::{Path, PathBuf};

use nous_core::scheduler::ScheduleConfig;
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
    pub rooms: RoomsConfig,
    pub daemon: DaemonConfig,
    pub schedule: ScheduleConfig,
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
    pub dimensions: usize,
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
            variant: "onnx/model_q4.onnx".into(),
            dimensions: 1024,
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

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RoomsConfig {
    pub max_rooms: usize,
    pub max_messages_per_room: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    pub socket_path: String,
    pub pid_file: String,
    pub log_file: String,
    pub mcp_transport: String,
    pub mcp_port: u16,
    pub shutdown_timeout_secs: u64,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: "~/.cache/nous/daemon.sock".into(),
            pid_file: "~/.cache/nous/daemon.pid".into(),
            log_file: "~/.cache/nous/daemon.log".into(),
            mcp_transport: "stdio".into(),
            mcp_port: 8377,
            shutdown_timeout_secs: 30,
        }
    }
}

impl Default for RoomsConfig {
    fn default() -> Self {
        Self {
            max_rooms: 1000,
            max_messages_per_room: 10000,
        }
    }
}

const DEFAULT_CONFIG_TOML: &str = r#"[memory]
db_path = "~/.cache/nous/memory.db"

[embedding]
model = "onnx-community/Qwen3-Embedding-0.6B-ONNX"
variant = "onnx/model_q4.onnx"
dimensions = 1024
chunk_size = 512
chunk_overlap = 64

[otlp]
db_path = "~/.cache/nous/otlp.db"
port = 4318

[classification]
confidence_threshold = 0.3

[encryption]
db_key_file = "~/.config/nous/db.key"

[rooms]
max_rooms = 1000
max_messages_per_room = 10000

[daemon]
socket_path = "~/.cache/nous/daemon.sock"
pid_file = "~/.cache/nous/daemon.pid"
log_file = "~/.cache/nous/daemon.log"
mcp_transport = "stdio"
mcp_port = 8377
shutdown_timeout_secs = 30

[schedule]
enabled = true
allow_shell = false
allow_http = true
max_concurrent = 4
default_timeout_secs = 300
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
        if let Ok(val) = std::env::var("NOUS_ROOMS_MAX")
            && let Ok(n) = val.parse::<usize>()
        {
            cfg.rooms.max_rooms = n;
        }
        if let Ok(val) = std::env::var("NOUS_ROOMS_MAX_MESSAGES")
            && let Ok(n) = val.parse::<usize>()
        {
            cfg.rooms.max_messages_per_room = n;
        }
        if let Ok(val) = std::env::var("NOUS_DAEMON_SOCKET")
            && !val.is_empty()
        {
            cfg.daemon.socket_path = val;
        }
        if let Ok(val) = std::env::var("NOUS_SCHEDULE_ENABLED") {
            cfg.schedule.enabled = val != "0" && val.to_lowercase() != "false";
        }
        if let Ok(val) = std::env::var("NOUS_SCHEDULE_ALLOW_SHELL") {
            cfg.schedule.allow_shell = val == "1" || val.to_lowercase() == "true";
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
variant = "onnx/model_q4.onnx"
dimensions = 1024
chunk_size = 512
chunk_overlap = 64

[otlp]
db_path = "~/.cache/nous/otlp.db"
port = 4318

[classification]
confidence_threshold = 0.3

[encryption]
db_key_file = "~/.config/nous/db.key"

[rooms]
max_rooms = 1000
max_messages_per_room = 10000

[daemon]
socket_path = "~/.cache/nous/daemon.sock"
pid_file = "~/.cache/nous/daemon.pid"
log_file = "~/.cache/nous/daemon.log"
mcp_transport = "stdio"
mcp_port = 8377
shutdown_timeout_secs = 30

[schedule]
enabled = true
allow_shell = false
allow_http = true
max_concurrent = 4
default_timeout_secs = 300
"#;

    #[test]
    fn parse_full_config() {
        let cfg: Config = toml::from_str(FULL_TOML).unwrap();
        assert_eq!(cfg.memory.db_path, "~/.cache/nous/memory.db");
        assert_eq!(
            cfg.embedding.model,
            "onnx-community/Qwen3-Embedding-0.6B-ONNX"
        );
        assert_eq!(cfg.embedding.variant, "onnx/model_q4.onnx");
        assert_eq!(cfg.embedding.dimensions, 1024);
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
        assert_eq!(cfg.embedding.variant, "onnx/model_q4.onnx");
        assert_eq!(cfg.embedding.dimensions, 1024);
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
        assert_eq!(
            cfg.embedding.model,
            "onnx-community/Qwen3-Embedding-0.6B-ONNX"
        );
        assert_eq!(cfg.embedding.variant, "onnx/model_q4.onnx");
        assert_eq!(cfg.embedding.dimensions, 1024);
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
variant = "onnx/model_q4.onnx"
dimensions = 1024
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

    #[test]
    fn default_embedding_variant_includes_onnx_subdirectory() {
        let cfg = EmbeddingConfig::default();
        assert_eq!(cfg.model, "onnx-community/Qwen3-Embedding-0.6B-ONNX");
        assert!(
            cfg.variant.starts_with("onnx/"),
            "variant must include onnx/ subdirectory prefix, got: {}",
            cfg.variant
        );
        assert!(
            cfg.variant.ends_with(".onnx"),
            "variant must end with .onnx extension, got: {}",
            cfg.variant
        );
    }

    #[test]
    fn default_config_toml_matches_struct_defaults() {
        let from_toml = Config::load_from_str(DEFAULT_CONFIG_TOML).unwrap();
        let from_default = EmbeddingConfig::default();
        assert_eq!(from_toml.embedding.model, from_default.model);
        assert_eq!(from_toml.embedding.variant, from_default.variant);
    }
}
