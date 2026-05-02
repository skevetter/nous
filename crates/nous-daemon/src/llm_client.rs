use rig::client::completion::CompletionClient;
use rig::client::ProviderClient;
use rig::completion::Prompt as _;
use rig_bedrock::client::{Client as BedrockClient, ClientBuilder, DEFAULT_AWS_REGION};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

pub use rig_bedrock::client::Client as RigBedrockClient;

// Default models per provider
pub const DEFAULT_BEDROCK_MODEL: &str = "anthropic.claude-sonnet-4-20250514-v1:0";
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-20250514";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4o";

/// Which LLM provider to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Bedrock,
    Anthropic,
    OpenAI,
}

impl ProviderKind {
    pub fn default_model(&self) -> &'static str {
        match self {
            ProviderKind::Bedrock => DEFAULT_BEDROCK_MODEL,
            ProviderKind::Anthropic => DEFAULT_ANTHROPIC_MODEL,
            ProviderKind::OpenAI => DEFAULT_OPENAI_MODEL,
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderKind::Bedrock => write!(f, "bedrock"),
            ProviderKind::Anthropic => write!(f, "anthropic"),
            ProviderKind::OpenAI => write!(f, "openai"),
        }
    }
}

impl std::str::FromStr for ProviderKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bedrock" | "aws" => Ok(ProviderKind::Bedrock),
            "anthropic" | "claude" => Ok(ProviderKind::Anthropic),
            "openai" | "gpt" => Ok(ProviderKind::OpenAI),
            other => Err(format!(
                "unknown provider '{other}': expected bedrock, anthropic, or openai"
            )),
        }
    }
}

/// Unified LLM provider that wraps different backend clients.
#[derive(Clone)]
pub enum LlmProvider {
    Bedrock(BedrockClient),
    Anthropic(rig::providers::anthropic::Client),
    OpenAI(rig::providers::openai::CompletionsClient),
}

impl LlmProvider {
    /// Get the provider kind for this instance.
    pub fn kind(&self) -> ProviderKind {
        match self {
            LlmProvider::Bedrock(_) => ProviderKind::Bedrock,
            LlmProvider::Anthropic(_) => ProviderKind::Anthropic,
            LlmProvider::OpenAI(_) => ProviderKind::OpenAI,
        }
    }

    /// Send a prompt to the LLM and return the response text.
    pub async fn prompt(
        &self,
        model: &str,
        preamble: &str,
        prompt_text: &str,
        timeout: Duration,
    ) -> Result<String, LlmError> {
        match self {
            LlmProvider::Bedrock(client) => {
                let agent = if preamble.is_empty() {
                    client.agent(model).build()
                } else {
                    client.agent(model).preamble(preamble).build()
                };
                let result = tokio::time::timeout(timeout, agent.prompt(prompt_text)).await;
                match result {
                    Ok(Ok(output)) => Ok(output),
                    Ok(Err(e)) => Err(LlmError::Provider(format!(
                        "Bedrock error (model={model}): {e:?}"
                    ))),
                    Err(_) => Err(LlmError::Timeout),
                }
            }
            LlmProvider::Anthropic(client) => {
                let agent = if preamble.is_empty() {
                    client.agent(model).build()
                } else {
                    client.agent(model).preamble(preamble).build()
                };
                let result = tokio::time::timeout(timeout, agent.prompt(prompt_text)).await;
                match result {
                    Ok(Ok(output)) => Ok(output),
                    Ok(Err(e)) => Err(LlmError::Provider(format!(
                        "Anthropic error (model={model}): {e:?}"
                    ))),
                    Err(_) => Err(LlmError::Timeout),
                }
            }
            LlmProvider::OpenAI(client) => {
                let agent = if preamble.is_empty() {
                    client.agent(model).build()
                } else {
                    client.agent(model).preamble(preamble).build()
                };
                let result = tokio::time::timeout(timeout, agent.prompt(prompt_text)).await;
                match result {
                    Ok(Ok(output)) => Ok(output),
                    Ok(Err(e)) => Err(LlmError::Provider(format!(
                        "OpenAI error (model={model}): {e:?}"
                    ))),
                    Err(_) => Err(LlmError::Timeout),
                }
            }
        }
    }
}

/// Errors from LLM operations.
#[derive(Debug)]
pub enum LlmError {
    Provider(String),
    Timeout,
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::Provider(msg) => write!(f, "{msg}"),
            LlmError::Timeout => write!(f, "invocation timed out"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: ProviderKind,
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
    provider: Option<String>,
    model: Option<String>,
    region: Option<String>,
    profile: Option<String>,
}

impl LlmConfig {
    /// Resolve config using: CLI flag > env var > config file > hardcoded default.
    pub fn resolve(
        cli_provider: Option<String>,
        cli_model: Option<String>,
        cli_region: Option<String>,
        cli_profile: Option<String>,
    ) -> Self {
        let file = Self::load_config_file();

        let provider = cli_provider
            .or_else(|| std::env::var("NOUS_LLM_PROVIDER").ok())
            .or_else(|| file.as_ref().and_then(|f| f.provider.clone()))
            .and_then(|s| s.parse::<ProviderKind>().ok())
            .unwrap_or(ProviderKind::Bedrock);

        let model = cli_model
            .or_else(|| std::env::var("NOUS_LLM_MODEL").ok())
            .or_else(|| file.as_ref().and_then(|f| f.model.clone()))
            .unwrap_or_else(|| provider.default_model().to_string());

        let region = cli_region
            .or_else(|| std::env::var("AWS_REGION").ok())
            .or_else(|| file.as_ref().and_then(|f| f.region.clone()))
            .unwrap_or_else(|| DEFAULT_AWS_REGION.to_string());

        let profile = cli_profile
            .or_else(|| std::env::var("AWS_PROFILE").ok())
            .or_else(|| file.as_ref().and_then(|f| f.profile.clone()));

        Self {
            provider,
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

/// Build an LlmProvider from the resolved configuration.
/// Returns None if credentials are not available.
pub async fn build_provider(config: &LlmConfig) -> Option<LlmProvider> {
    match config.provider {
        ProviderKind::Bedrock => {
            let has_credentials = std::env::var("AWS_ACCESS_KEY_ID").is_ok()
                || std::env::var("AWS_PROFILE").is_ok()
                || std::env::var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").is_ok()
                || config.profile.is_some();

            if !has_credentials {
                return None;
            }

            let client = if let Some(ref profile) = config.profile {
                BedrockClient::with_profile_name(profile)
            } else {
                ClientBuilder::default()
                    .region(&config.region)
                    .build()
                    .await
            };
            Some(LlmProvider::Bedrock(client))
        }
        ProviderKind::Anthropic => {
            if std::env::var("ANTHROPIC_API_KEY").is_err() {
                return None;
            }
            match rig::providers::anthropic::Client::from_env() {
                Ok(client) => Some(LlmProvider::Anthropic(client)),
                Err(e) => {
                    tracing::warn!("Failed to create Anthropic client: {e}");
                    None
                }
            }
        }
        ProviderKind::OpenAI => {
            if std::env::var("OPENAI_API_KEY").is_err() {
                return None;
            }
            match rig::providers::openai::CompletionsClient::from_env() {
                Ok(client) => Some(LlmProvider::OpenAI(client)),
                Err(e) => {
                    tracing::warn!("Failed to create OpenAI client: {e}");
                    None
                }
            }
        }
    }
}

/// Check if credentials are available for a given provider kind.
pub fn has_credentials(provider: ProviderKind) -> bool {
    match provider {
        ProviderKind::Bedrock => {
            std::env::var("AWS_ACCESS_KEY_ID").is_ok()
                || std::env::var("AWS_PROFILE").is_ok()
                || std::env::var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").is_ok()
        }
        ProviderKind::Anthropic => std::env::var("ANTHROPIC_API_KEY").is_ok(),
        ProviderKind::OpenAI => std::env::var("OPENAI_API_KEY").is_ok(),
    }
}

/// Return a human-readable description of the credential source for a provider.
pub fn credential_source(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Bedrock => "AWS SSO / AWS_ACCESS_KEY_ID / AWS_PROFILE",
        ProviderKind::Anthropic => "ANTHROPIC_API_KEY",
        ProviderKind::OpenAI => "OPENAI_API_KEY",
    }
}
