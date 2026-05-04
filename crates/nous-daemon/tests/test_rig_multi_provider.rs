mod common;

use nous_core::agents::processes::{self, UpdateAgentRequest};
use nous_core::agents::{self, RegisterAgentRequest};
use nous_core::error::NousError;
use nous_daemon::llm_client::{LlmClient, DEFAULT_MODEL};
use nous_daemon::process_manager::InvokeParams;
use nous_daemon::state::AppState;
use tempfile::TempDir;

async fn test_state() -> (AppState, TempDir) {
    let (mut state, tmp) = common::test_state().await;
    state.default_model = "test-default-model".to_string();
    (state, tmp)
}

async fn register_agent(state: &AppState, name: &str, process_type: Option<&str>) -> String {
    let agent = agents::register_agent(
        &state.pool,
        RegisterAgentRequest {
            name: name.into(),
            agent_type: None,
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
        processes::update_agent(
            &state.pool,
            UpdateAgentRequest {
                id: &agent.id,
                process_type: Some(pt),
                spawn_command: None,
                working_dir: None,
                auto_restart: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
    }

    agent.id
}

#[tokio::test]
async fn test_default_model_constant_is_set() {
    assert!(
        !DEFAULT_MODEL.is_empty(),
        "DEFAULT_MODEL should be a non-empty string"
    );
    assert!(
        DEFAULT_MODEL.contains("anthropic"),
        "DEFAULT_MODEL should reference an Anthropic model, got: {DEFAULT_MODEL}"
    );
}

#[tokio::test]
async fn test_default_model_used_when_no_override() {
    let (state, _tmp) = test_state().await;
    assert_eq!(state.default_model, "test-default-model");

    let agent_id = register_agent(&state, "model-default-agent", Some("claude")).await;

    let result = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "hello",
            timeout_secs: None,
            metadata: None,
            is_async: false,
        })
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Unavailable(msg) if msg.contains("LLM client not configured")),
        "with no llm_client, should fail with Unavailable error before reaching model selection: {err}"
    );
}

#[tokio::test]
async fn test_model_override_from_metadata() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent(&state, "model-override-agent", Some("claude")).await;

    let metadata = serde_json::json!({
        "model": "us.anthropic.claude-haiku-4-5-20251001-v1:0"
    });

    let result = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "hello",
            timeout_secs: None,
            metadata: Some(metadata),
            is_async: false,
        })
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Unavailable(msg) if msg.contains("LLM client not configured")),
        "with no llm_client, should fail before using model override: {err}"
    );
}

#[tokio::test]
async fn test_preamble_extraction_from_metadata() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent(&state, "preamble-agent", Some("claude")).await;

    let metadata = serde_json::json!({
        "preamble": "You are a helpful assistant specialized in Rust."
    });

    let result = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "hello",
            timeout_secs: None,
            metadata: Some(metadata),
            is_async: false,
        })
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Unavailable(msg) if msg.contains("LLM client not configured")),
        "with no llm_client, preamble extraction should not change the error path: {err}"
    );
}

#[tokio::test]
async fn test_llm_client_none_returns_config_error() {
    let (state, _tmp) = test_state().await;
    assert!(state.llm_client.is_none());

    let agent_id = register_agent(&state, "no-client-agent", Some("claude")).await;

    let result = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "hello",
            timeout_secs: None,
            metadata: None,
            is_async: false,
        })
        .await;

    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Unavailable(msg) if msg.contains("LLM client not configured")),
        "expected Unavailable error about missing LLM client, got: {err}"
    );
}

#[tokio::test]
async fn test_process_type_dispatch_routes_claude() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent(&state, "dispatch-claude", Some("claude")).await;

    let result = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "test",
            timeout_secs: None,
            metadata: None,
            is_async: false,
        })
        .await;

    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Unavailable(msg) if msg.contains("LLM client")),
        "claude dispatch should attempt LLM path and fail on missing client: {err}"
    );
}

#[tokio::test]
async fn test_process_type_dispatch_routes_shell() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent(&state, "dispatch-shell", Some("shell")).await;

    let invocation = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "echo routed",
            timeout_secs: Some(30),
            metadata: None,
            is_async: false,
        })
        .await
        .unwrap();

    assert_eq!(invocation.status, "completed");
    assert!(
        invocation.result.as_deref().unwrap().contains("routed"),
        "shell dispatch should execute the command: {:?}",
        invocation.result
    );
}

#[tokio::test]
async fn test_process_type_dispatch_routes_none_to_shell() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent(&state, "dispatch-none", None).await;

    let invocation = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "echo fallback",
            timeout_secs: Some(30),
            metadata: None,
            is_async: false,
        })
        .await
        .unwrap();

    assert_eq!(invocation.status, "completed");
    assert!(
        invocation.result.as_deref().unwrap().contains("fallback"),
        "None process_type should route to shell: {:?}",
        invocation.result
    );
}

#[tokio::test]
async fn test_process_type_dispatch_unknown_returns_error() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent(&state, "dispatch-unknown", Some("graphql")).await;

    let result = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "test",
            timeout_secs: None,
            metadata: None,
            is_async: false,
        })
        .await;

    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Config(msg) if msg.contains("unsupported process_type 'graphql'")),
        "unknown process_type should return Config error: {err}"
    );
}

#[tokio::test]
async fn test_metadata_with_both_model_and_preamble() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent(&state, "full-metadata-agent", Some("claude")).await;

    let metadata = serde_json::json!({
        "model": "us.anthropic.claude-haiku-4-5-20251001-v1:0",
        "preamble": "You are a code reviewer."
    });

    let result = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "review this code",
            timeout_secs: None,
            metadata: Some(metadata),
            is_async: false,
        })
        .await;

    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Unavailable(msg) if msg.contains("LLM client not configured")),
        "combined model+preamble metadata should still fail on missing client: {err}"
    );
}

#[tokio::test]
async fn test_metadata_with_extra_fields_does_not_break_dispatch() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent(&state, "extra-metadata-agent", Some("claude")).await;

    let metadata = serde_json::json!({
        "model": "us.anthropic.claude-sonnet-4-20250514-v1:0",
        "preamble": "Be concise.",
        "temperature": 0.7,
        "custom_field": "should be ignored"
    });

    let result = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "hello",
            timeout_secs: None,
            metadata: Some(metadata),
            is_async: false,
        })
        .await;

    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Unavailable(msg) if msg.contains("LLM client not configured")),
        "extra metadata fields should not cause a different error path: {err}"
    );
}

#[tokio::test]
async fn test_rig_agent_creation_with_real_credentials() {
    if std::env::var("AWS_ACCESS_KEY_ID").is_err() {
        return;
    }
    use rig::client::completion::CompletionClient;
    use rig::client::ProviderClient;

    let client = LlmClient::from_env().expect("AWS credentials required");
    let _agent = client.agent(DEFAULT_MODEL).build();

    let _agent_with_preamble = client
        .agent(DEFAULT_MODEL)
        .preamble("You are a helpful assistant.")
        .build();

    let custom_model = "us.anthropic.claude-haiku-4-5-20251001-v1:0";
    let _custom_agent = client.agent(custom_model).build();
}
