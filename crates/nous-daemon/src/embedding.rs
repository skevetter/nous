use std::sync::Arc;

use nous_core::error::NousError;
use nous_core::memory::{
    Embedder, EmbeddingConfig, EmbeddingProvider, OnnxEmbeddingModel, RigEmbedderAdapter,
};
use rig::client::{EmbeddingsClient, ProviderClient};
use serde::Deserialize;

pub fn build_embedder(config: &EmbeddingConfig) -> Result<Arc<dyn Embedder>, NousError> {
    match config.provider {
        EmbeddingProvider::Local => {
            let model = OnnxEmbeddingModel::load(None)?;
            Ok(Arc::new(model))
        }
        EmbeddingProvider::Bedrock => {
            let rt = tokio::runtime::Handle::current();
            let client = rig_bedrock::client::Client::from_env()
                .map_err(|e| NousError::Config(format!("failed to create Bedrock client: {e}")))?;
            let model = client.embedding_model_with_ndims(&config.model, config.dimensions);
            Ok(Arc::new(RigEmbedderAdapter::new(model, rt)))
        }
        EmbeddingProvider::OpenAi => {
            let rt = tokio::runtime::Handle::current();
            let client = rig::providers::openai::Client::from_env()
                .map_err(|e| NousError::Config(format!("failed to create OpenAI client: {e}")))?;
            let model = client.embedding_model_with_ndims(&config.model, config.dimensions);
            Ok(Arc::new(RigEmbedderAdapter::new(model, rt)))
        }
    }
}

#[derive(Deserialize)]
struct ConfigFile {
    embedding: Option<EmbeddingSection>,
}

#[derive(Deserialize)]
struct EmbeddingSection {
    provider: Option<String>,
    model: Option<String>,
    dimensions: Option<usize>,
}

pub fn resolve_embedding_config(
    cli_provider: Option<String>,
    cli_model: Option<String>,
    cli_dimensions: Option<usize>,
) -> EmbeddingConfig {
    let file = load_config_file();

    let provider_str = cli_provider
        .or_else(|| std::env::var("NOUS_EMBEDDING_PROVIDER").ok())
        .or_else(|| file.as_ref().and_then(|f| f.provider.clone()));

    let provider = match provider_str.as_deref() {
        Some("bedrock") => EmbeddingProvider::Bedrock,
        Some("openai") => EmbeddingProvider::OpenAi,
        _ => EmbeddingProvider::Local,
    };

    let default = EmbeddingConfig::default();

    let model = cli_model
        .or_else(|| std::env::var("NOUS_EMBEDDING_MODEL").ok())
        .or_else(|| file.as_ref().and_then(|f| f.model.clone()))
        .unwrap_or(default.model);

    let dimensions = cli_dimensions
        .or_else(|| {
            std::env::var("NOUS_EMBEDDING_DIMENSIONS")
                .ok()
                .and_then(|s| s.parse().ok())
        })
        .or_else(|| file.as_ref().and_then(|f| f.dimensions))
        .unwrap_or(default.dimensions);

    EmbeddingConfig {
        provider,
        model,
        dimensions,
    }
}

fn load_config_file() -> Option<EmbeddingSection> {
    let path = dirs::config_dir()?.join("nous/config.toml");
    let content = std::fs::read_to_string(path).ok()?;
    let parsed: ConfigFile = toml::from_str(&content).ok()?;
    parsed.embedding
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_defaults_to_local() {
        temp_env::with_vars_unset(
            vec![
                "NOUS_EMBEDDING_PROVIDER",
                "NOUS_EMBEDDING_MODEL",
                "NOUS_EMBEDDING_DIMENSIONS",
            ],
            || {
                let config = resolve_embedding_config(None, None, None);
                assert_eq!(config.provider, EmbeddingProvider::Local);
                assert_eq!(config.model, "all-MiniLM-L6-v2");
                assert_eq!(config.dimensions, 384);
            },
        );
    }

    #[test]
    fn resolve_cli_overrides_all() {
        let config = resolve_embedding_config(
            Some("bedrock".to_string()),
            Some("amazon.titan-embed-text-v2:0".to_string()),
            Some(1024),
        );
        assert_eq!(config.provider, EmbeddingProvider::Bedrock);
        assert_eq!(config.model, "amazon.titan-embed-text-v2:0");
        assert_eq!(config.dimensions, 1024);
    }

    #[test]
    fn resolve_env_overrides_default() {
        temp_env::with_vars(
            vec![
                ("NOUS_EMBEDDING_PROVIDER", Some("openai")),
                ("NOUS_EMBEDDING_MODEL", Some("text-embedding-3-small")),
                ("NOUS_EMBEDDING_DIMENSIONS", Some("1536")),
            ],
            || {
                let config = resolve_embedding_config(None, None, None);
                assert_eq!(config.provider, EmbeddingProvider::OpenAi);
                assert_eq!(config.model, "text-embedding-3-small");
                assert_eq!(config.dimensions, 1536);
            },
        );
    }

    #[test]
    fn resolve_cli_beats_env() {
        temp_env::with_vars(vec![("NOUS_EMBEDDING_PROVIDER", Some("openai"))], || {
            let config = resolve_embedding_config(Some("bedrock".to_string()), None, None);
            assert_eq!(config.provider, EmbeddingProvider::Bedrock);
        });
    }

    #[test]
    fn resolve_unknown_provider_defaults_to_local() {
        let config = resolve_embedding_config(Some("unknown".to_string()), None, None);
        assert_eq!(config.provider, EmbeddingProvider::Local);
    }

    #[test]
    fn resolve_invalid_dimensions_env_uses_default() {
        temp_env::with_vars(
            vec![("NOUS_EMBEDDING_DIMENSIONS", Some("not-a-number"))],
            || {
                let config = resolve_embedding_config(None, None, None);
                assert_eq!(config.dimensions, 384);
            },
        );
    }
}
