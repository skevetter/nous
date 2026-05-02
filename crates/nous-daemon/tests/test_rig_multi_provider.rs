use nous_core::agents::processes;
use nous_core::agents::{self, AgentType, RegisterAgentRequest};
use nous_core::db::DbPools;
use nous_core::error::NousError;
use nous_core::memory::MockEmbedder;
use nous_core::notifications::NotificationRegistry;
use nous_daemon::llm_client::{
    LlmConfig, ProviderKind, DEFAULT_ANTHROPIC_MODEL, DEFAULT_BEDROCK_MODEL,
    DEFAULT_OPENAI_MODEL,
};
use nous_daemon::process_manager::ProcessRegistry;
use nous_daemon::state::AppState;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

async fn test_state() -> (AppState, TempDir) {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();
    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry: Arc::new(NotificationRegistry::new()),
        embedder: Some(Arc::new(MockEmbedder::new())),
        schedule_notify: Arc::new(Notify::new()),
        shutdown: CancellationToken::new(),
        process_registry: Arc::new(ProcessRegistry::new()),
        llm_provider: None,
        default_model: "test-default-model".to_string(),
        #[cfg(feature = "sandbox")]
        sandbox_manager: None,
    };
    (state, tmp)
}

async fn register_agent(state: &AppState, name: &str, process_type: Option<&str>) -> String {
    let agent = agents::register_agent(
        &state.pool,
        RegisterAgentRequest {
            name: name.into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    if let Some(pt) = process_type {
        processes::update_agent(&state.pool, &agent.id, Some(pt), None, None, None, None)
            .await
            .unwrap();
    }

    agent.id
}

// --- ProviderKind tests ---

#[test]
fn provider_kind_from_str_bedrock() {
    assert_eq!("bedrock".parse::<ProviderKind>().unwrap(), ProviderKind::Bedrock);
    assert_eq!("aws".parse::<ProviderKind>().unwrap(), ProviderKind::Bedrock);
}

#[test]
fn provider_kind_from_str_anthropic() {
    assert_eq!("anthropic".parse::<ProviderKind>().unwrap(), ProviderKind::Anthropic);
    assert_eq!("claude".parse::<ProviderKind>().unwrap(), ProviderKind::Anthropic);
}

#[test]
fn provider_kind_from_str_openai() {
    assert_eq!("openai".parse::<ProviderKind>().unwrap(), ProviderKind::OpenAI);
    assert_eq!("gpt".parse::<ProviderKind>().unwrap(), ProviderKind::OpenAI);
}

#[test]
fn provider_kind_from_str_unknown() {
    assert!("foobar".parse::<ProviderKind>().is_err());
}

#[test]
fn provider_kind_display() {
    assert_eq!(ProviderKind::Bedrock.to_string(), "bedrock");
    assert_eq!(ProviderKind::Anthropic.to_string(), "anthropic");
    assert_eq!(ProviderKind::OpenAI.to_string(), "openai");
}

#[test]
fn provider_kind_default_models() {
    assert_eq!(ProviderKind::Bedrock.default_model(), DEFAULT_BEDROCK_MODEL);
    assert_eq!(ProviderKind::Anthropic.default_model(), DEFAULT_ANTHROPIC_MODEL);
    assert_eq!(ProviderKind::OpenAI.default_model(), DEFAULT_OPENAI_MODEL);
}

// --- LlmConfig resolution ---

#[test]
fn llm_config_resolve_defaults_to_bedrock() {
    let config = LlmConfig::resolve(None, None, None, None);
    assert_eq!(config.provider, ProviderKind::Bedrock);
    assert_eq!(config.model, DEFAULT_BEDROCK_MODEL);
}

#[test]
fn llm_config_resolve_cli_provider_overrides() {
    let config = LlmConfig::resolve(Some("anthropic".to_string()), None, None, None);
    assert_eq!(config.provider, ProviderKind::Anthropic);
    assert_eq!(config.model, DEFAULT_ANTHROPIC_MODEL);
}

#[test]
fn llm_config_resolve_cli_model_overrides_default() {
    let config = LlmConfig::resolve(
        Some("openai".to_string()),
        Some("gpt-4-turbo".to_string()),
        None,
        None,
    );
    assert_eq!(config.provider, ProviderKind::OpenAI);
    assert_eq!(config.model, "gpt-4-turbo");
}

#[test]
fn llm_config_resolve_env_provider() {
    temp_env::with_vars(
        [("NOUS_LLM_PROVIDER", Some("openai"))],
        || {
            let config = LlmConfig::resolve(None, None, None, None);
            assert_eq!(config.provider, ProviderKind::OpenAI);
            assert_eq!(config.model, DEFAULT_OPENAI_MODEL);
        },
    );
}

#[test]
fn llm_config_resolve_cli_takes_priority_over_env() {
    temp_env::with_vars(
        [("NOUS_LLM_PROVIDER", Some("openai"))],
        || {
            let config = LlmConfig::resolve(Some("anthropic".to_string()), None, None, None);
            assert_eq!(config.provider, ProviderKind::Anthropic);
        },
    );
}

// --- AppState with no provider ---

#[tokio::test]
async fn test_llm_provider_none_returns_config_error() {
    let (state, _tmp) = test_state().await;
    assert!(state.llm_provider.is_none());

    let agent_id = register_agent(&state, "no-provider-agent", Some("claude")).await;

    let result = state
        .process_registry
        .invoke(&state, &agent_id, "hello", None, None, false)
        .await;

    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Config(msg) if msg.contains("LLM provider not configured")),
        "expected Config error about missing LLM provider, got: {err}"
    );
}

// --- LlmProvider kind accessor ---

#[tokio::test]
#[ignore]
async fn provider_kind_bedrock_from_real_credentials() {
    use nous_daemon::llm_client::build_provider;

    let config = LlmConfig::resolve(Some("bedrock".to_string()), None, None, None);
    let provider = build_provider(&config).await;
    if let Some(p) = provider {
        assert_eq!(p.kind(), ProviderKind::Bedrock);
    }
}

#[tokio::test]
#[ignore]
async fn provider_kind_anthropic_from_real_credentials() {
    use nous_daemon::llm_client::build_provider;

    let config = LlmConfig::resolve(Some("anthropic".to_string()), None, None, None);
    let provider = build_provider(&config).await;
    if let Some(p) = provider {
        assert_eq!(p.kind(), ProviderKind::Anthropic);
    }
}

#[tokio::test]
#[ignore]
async fn provider_kind_openai_from_real_credentials() {
    use nous_daemon::llm_client::build_provider;

    let config = LlmConfig::resolve(Some("openai".to_string()), None, None, None);
    let provider = build_provider(&config).await;
    if let Some(p) = provider {
        assert_eq!(p.kind(), ProviderKind::OpenAI);
    }
}

// --- Credential check helpers ---

#[test]
fn has_credentials_bedrock_no_env() {
    temp_env::with_vars(
        [
            ("AWS_ACCESS_KEY_ID", None::<&str>),
            ("AWS_PROFILE", None::<&str>),
            ("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", None::<&str>),
        ],
        || {
            assert!(!nous_daemon::llm_client::has_credentials(ProviderKind::Bedrock));
        },
    );
}

#[test]
fn has_credentials_anthropic_with_key() {
    temp_env::with_vars(
        [("ANTHROPIC_API_KEY", Some("sk-ant-test"))],
        || {
            assert!(nous_daemon::llm_client::has_credentials(ProviderKind::Anthropic));
        },
    );
}

#[test]
fn has_credentials_openai_with_key() {
    temp_env::with_vars(
        [("OPENAI_API_KEY", Some("sk-test"))],
        || {
            assert!(nous_daemon::llm_client::has_credentials(ProviderKind::OpenAI));
        },
    );
}

#[test]
fn credential_source_descriptions() {
    assert!(nous_daemon::llm_client::credential_source(ProviderKind::Bedrock).contains("AWS"));
    assert!(nous_daemon::llm_client::credential_source(ProviderKind::Anthropic).contains("ANTHROPIC_API_KEY"));
    assert!(nous_daemon::llm_client::credential_source(ProviderKind::OpenAI).contains("OPENAI_API_KEY"));
}
