use rig_bedrock::client::{Client, ClientBuilder, DEFAULT_AWS_REGION};
use serde::Deserialize;

pub type LlmClient = Client;

pub const DEFAULT_MODEL: &str = "anthropic.claude-sonnet-4-20250514-v1:0";

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub model: String,
    pub region: String,
    pub profile: Option<String>,
}

#[derive(Deserialize)]
struct ConfigFile {
    llm: Option<LlmSection>,
}

#[derive(Deserialize)]
struct LlmSection {
    model: Option<String>,
    region: Option<String>,
    profile: Option<String>,
}

impl LlmConfig {
    /// Resolve config using: CLI flag > env var > config file > hardcoded default.
    pub fn resolve(
        cli_model: Option<String>,
        cli_region: Option<String>,
        cli_profile: Option<String>,
    ) -> Self {
        let file = Self::load_config_file();

        let model = cli_model
            .or_else(|| std::env::var("NOUS_LLM_MODEL").ok())
            .or_else(|| file.as_ref().and_then(|f| f.model.clone()))
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        let region = cli_region
            .or_else(|| std::env::var("AWS_REGION").ok())
            .or_else(|| file.as_ref().and_then(|f| f.region.clone()))
            .unwrap_or_else(|| DEFAULT_AWS_REGION.to_string());

        let profile = cli_profile
            .or_else(|| std::env::var("AWS_PROFILE").ok())
            .or_else(|| file.as_ref().and_then(|f| f.profile.clone()));

        Self {
            model,
            region,
            profile,
        }
    }

    fn load_config_file() -> Option<LlmSection> {
        let path = dirs::config_dir()?.join("nous/config.toml");
        let content = std::fs::read_to_string(path).ok()?;
        let parsed: ConfigFile = toml::from_str(&content).ok()?;
        parsed.llm
    }
}

pub async fn build_client(config: &LlmConfig) -> Client {
    if let Some(ref profile) = config.profile {
        Client::with_profile_name(profile)
    } else {
        ClientBuilder::default()
            .region(&config.region)
            .build()
            .await
    }
}
