use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum VectorStoreBackend {
    #[default]
    SqliteVec,
    Qdrant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorStoreConfig {
    pub backend: VectorStoreBackend,
    pub qdrant: Option<QdrantConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantConfig {
    pub url: String,
    pub collection: String,
    pub api_key: Option<String>,
}

impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            backend: VectorStoreBackend::SqliteVec,
            qdrant: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_store_config_default() {
        let config = VectorStoreConfig::default();
        assert_eq!(config.backend, VectorStoreBackend::SqliteVec);
        assert!(config.qdrant.is_none());
    }
}
