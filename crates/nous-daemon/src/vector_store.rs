use nous_core::memory::{QdrantConfig, VectorStoreBackend, VectorStoreConfig};
use serde::Deserialize;

pub fn resolve_vector_store_config(
    cli_backend: Option<String>,
    cli_qdrant_url: Option<String>,
    cli_qdrant_collection: Option<String>,
) -> VectorStoreConfig {
    let file = load_config_file();

    let backend_str = cli_backend
        .or_else(|| std::env::var("NOUS_VECTOR_STORE").ok())
        .or_else(|| file.as_ref().and_then(|f| f.backend.clone()));

    let backend = match backend_str.as_deref() {
        Some("qdrant") => VectorStoreBackend::Qdrant,
        _ => VectorStoreBackend::SqliteVec,
    };

    let qdrant_url = cli_qdrant_url
        .or_else(|| std::env::var("NOUS_QDRANT_URL").ok())
        .or_else(|| file.as_ref().and_then(|f| f.qdrant_url.clone()));

    let qdrant_collection = cli_qdrant_collection
        .or_else(|| std::env::var("NOUS_QDRANT_COLLECTION").ok())
        .or_else(|| file.as_ref().and_then(|f| f.qdrant_collection.clone()));

    let qdrant_api_key = std::env::var("NOUS_QDRANT_API_KEY").ok();

    let qdrant = match (&backend, qdrant_url, qdrant_collection) {
        (VectorStoreBackend::Qdrant, Some(url), Some(collection)) => Some(QdrantConfig {
            url,
            collection,
            api_key: qdrant_api_key,
        }),
        _ => None,
    };

    VectorStoreConfig { backend, qdrant }
}

#[derive(Deserialize)]
struct ConfigFile {
    vector_store: Option<VectorStoreSection>,
}

#[derive(Deserialize)]
struct VectorStoreSection {
    backend: Option<String>,
    qdrant_url: Option<String>,
    qdrant_collection: Option<String>,
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
            vec![
                "NOUS_VECTOR_STORE",
                "NOUS_QDRANT_URL",
                "NOUS_QDRANT_COLLECTION",
                "NOUS_QDRANT_API_KEY",
            ],
            || {
                let config = resolve_vector_store_config(None, None, None);
                assert_eq!(config.backend, VectorStoreBackend::SqliteVec);
                assert!(config.qdrant.is_none());
            },
        );
    }

    #[test]
    fn resolve_cli_overrides_env() {
        temp_env::with_vars(vec![("NOUS_VECTOR_STORE", Some("sqlite-vec"))], || {
            let config = resolve_vector_store_config(
                Some("qdrant".to_string()),
                Some("http://localhost:6334".to_string()),
                Some("test-collection".to_string()),
            );
            assert_eq!(config.backend, VectorStoreBackend::Qdrant);
            let qdrant = config.qdrant.unwrap();
            assert_eq!(qdrant.url, "http://localhost:6334");
            assert_eq!(qdrant.collection, "test-collection");
        });
    }

    #[test]
    fn resolve_env_overrides_default() {
        temp_env::with_vars(
            vec![
                ("NOUS_VECTOR_STORE", Some("qdrant")),
                ("NOUS_QDRANT_URL", Some("http://qdrant:6334")),
                ("NOUS_QDRANT_COLLECTION", Some("memories")),
                ("NOUS_QDRANT_API_KEY", Some("secret-key")),
            ],
            || {
                let config = resolve_vector_store_config(None, None, None);
                assert_eq!(config.backend, VectorStoreBackend::Qdrant);
                let qdrant = config.qdrant.unwrap();
                assert_eq!(qdrant.url, "http://qdrant:6334");
                assert_eq!(qdrant.collection, "memories");
                assert_eq!(qdrant.api_key.as_deref(), Some("secret-key"));
            },
        );
    }

    #[test]
    fn resolve_unknown_backend_defaults_to_sqlite_vec() {
        temp_env::with_vars_unset(
            vec![
                "NOUS_VECTOR_STORE",
                "NOUS_QDRANT_URL",
                "NOUS_QDRANT_COLLECTION",
                "NOUS_QDRANT_API_KEY",
            ],
            || {
                let config =
                    resolve_vector_store_config(Some("unknown-backend".to_string()), None, None);
                assert_eq!(config.backend, VectorStoreBackend::SqliteVec);
                assert!(config.qdrant.is_none());
            },
        );
    }

    #[test]
    fn resolve_qdrant_backend_without_url_has_no_qdrant_config() {
        temp_env::with_vars_unset(
            vec![
                "NOUS_VECTOR_STORE",
                "NOUS_QDRANT_URL",
                "NOUS_QDRANT_COLLECTION",
                "NOUS_QDRANT_API_KEY",
            ],
            || {
                let config = resolve_vector_store_config(Some("qdrant".to_string()), None, None);
                assert_eq!(config.backend, VectorStoreBackend::Qdrant);
                assert!(config.qdrant.is_none());
            },
        );
    }
}
