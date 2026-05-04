use nous_core::memory::{VectorStoreBackend, VectorStoreConfig};
use serde::Deserialize;

pub fn resolve_vector_store_config(cli_backend: Option<String>) -> VectorStoreConfig {
    let file = load_config_file();

    let backend_str = cli_backend
        .or_else(|| std::env::var("NOUS_VECTOR_STORE").ok())
        .or_else(|| file.as_ref().and_then(|f| f.backend.clone()));

    let backend = match backend_str.as_deref() {
        Some("qdrant") => VectorStoreBackend::Qdrant,
        _ => VectorStoreBackend::SqliteVec,
    };

    VectorStoreConfig { backend }
}

#[derive(Deserialize)]
struct ConfigFile {
    vector_store: Option<VectorStoreSection>,
}

#[derive(Deserialize)]
struct VectorStoreSection {
    backend: Option<String>,
}

fn load_config_file() -> Option<VectorStoreSection> {
    let path = dirs::config_dir()?.join("nous/config.toml");
    let content = std::fs::read_to_string(path).ok()?;
    let parsed: ConfigFile = toml::from_str(&content).ok()?;
    parsed.vector_store
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_defaults_to_sqlite_vec() {
        temp_env::with_vars_unset(
            vec!["NOUS_VECTOR_STORE"],
            || {
                let config = resolve_vector_store_config(None);
                assert_eq!(config.backend, VectorStoreBackend::SqliteVec);
            },
        );
    }

    #[test]
    fn resolve_cli_overrides_env() {
        temp_env::with_vars(vec![("NOUS_VECTOR_STORE", Some("sqlite-vec"))], || {
            let config = resolve_vector_store_config(Some("qdrant".to_string()));
            assert_eq!(config.backend, VectorStoreBackend::Qdrant);
        });
    }

    #[test]
    fn resolve_env_overrides_default() {
        temp_env::with_vars(
            vec![("NOUS_VECTOR_STORE", Some("qdrant"))],
            || {
                let config = resolve_vector_store_config(None);
                assert_eq!(config.backend, VectorStoreBackend::Qdrant);
            },
        );
    }

    #[test]
    fn resolve_unknown_backend_defaults_to_sqlite_vec() {
        temp_env::with_vars_unset(
            vec!["NOUS_VECTOR_STORE"],
            || {
                let config =
                    resolve_vector_store_config(Some("unknown-backend".to_string()));
                assert_eq!(config.backend, VectorStoreBackend::SqliteVec);
            },
        );
    }

    #[test]
    fn resolve_qdrant_backend_selects_qdrant() {
        temp_env::with_vars_unset(
            vec!["NOUS_VECTOR_STORE"],
            || {
                let config = resolve_vector_store_config(Some("qdrant".to_string()));
                assert_eq!(config.backend, VectorStoreBackend::Qdrant);
            },
        );
    }
}
