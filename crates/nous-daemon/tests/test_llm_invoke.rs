mod common;

use nous_core::agents::processes::{self, UpdateAgentRequest};
use nous_core::agents::{self, RegisterAgentRequest};
use nous_core::error::NousError;
use nous_daemon::process_manager::InvokeParams;
use nous_daemon::state::AppState;
use std::sync::Arc;
use tempfile::TempDir;

async fn test_state() -> (AppState, TempDir) {
    common::test_state().await
}

async fn register_agent_with_process_type(
    state: &AppState,
    name: &str,
    process_type: Option<&str>,
) -> String {
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

// --- Sync claude invoke with no LlmClient ---

#[tokio::test]
async fn sync_claude_invoke_no_llm_client_returns_config_error() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent_with_process_type(&state, "claude-sync", Some("claude")).await;

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
        "expected Unavailable error about LLM client, got: {err}"
    );
}

// --- Async claude invoke with no LlmClient ---

#[tokio::test]
async fn async_claude_invoke_no_llm_client_returns_config_error() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent_with_process_type(&state, "claude-async", Some("claude")).await;

    let result = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "hello",
            timeout_secs: None,
            metadata: None,
            is_async: true,
        })
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Unavailable(msg) if msg.contains("LLM client not configured")),
        "expected Unavailable error about LLM client, got: {err}"
    );
}

// --- Unsupported process_type ---

#[tokio::test]
async fn unsupported_process_type_returns_config_error() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent_with_process_type(&state, "http-agent", Some("http")).await;

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
        matches!(&err, NousError::Config(msg) if msg.contains("unsupported process_type")),
        "expected Config error about unsupported process_type, got: {err}"
    );
}

// --- Shell invoke regression ---

#[tokio::test]
async fn shell_invoke_completes_with_output() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent_with_process_type(&state, "shell-agent", Some("shell")).await;

    let invocation = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "echo hello",
            timeout_secs: Some(30),
            metadata: None,
            is_async: false,
        })
        .await
        .unwrap();

    assert_eq!(invocation.status, "completed");
    assert!(
        invocation.result.as_deref().unwrap().contains("hello"),
        "expected 'hello' in result, got: {:?}",
        invocation.result
    );
}

// --- Shell invoke with NULL process_type ---

#[tokio::test]
async fn null_process_type_falls_back_to_shell() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent_with_process_type(&state, "null-pt-agent", None).await;

    let invocation = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "echo test",
            timeout_secs: Some(30),
            metadata: None,
            is_async: false,
        })
        .await
        .unwrap();

    assert_eq!(invocation.status, "completed");
    assert!(
        invocation.result.as_deref().unwrap().contains("test"),
        "expected 'test' in result, got: {:?}",
        invocation.result
    );
}

// --- Invocation status set to 'failed' on dispatch error ---

#[tokio::test]
async fn invocation_status_failed_on_claude_dispatch_error() {
    let (state, _tmp) = test_state().await;
    let agent_id =
        register_agent_with_process_type(&state, "claude-fail-status", Some("claude")).await;

    let _ = state
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

    let invocations = processes::list_invocations(&state.pool, &agent_id, None, Some(1))
        .await
        .unwrap();

    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].status, "failed");
    assert!(invocations[0]
        .error
        .as_deref()
        .unwrap()
        .contains("LLM client not configured"));
}

#[tokio::test]
async fn invocation_status_failed_on_unsupported_process_type() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent_with_process_type(&state, "http-fail-status", Some("http")).await;

    let _ = state
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

    let invocations = processes::list_invocations(&state.pool, &agent_id, None, Some(1))
        .await
        .unwrap();

    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].status, "failed");
    assert!(invocations[0]
        .error
        .as_deref()
        .unwrap()
        .contains("unsupported process_type"));
}

// --- Async shell invoke completes in background ---

#[tokio::test]
async fn async_shell_invoke_returns_running_then_completes() {
    let (state, _tmp) = test_state().await;
    let agent_id =
        register_agent_with_process_type(&state, "async-shell-agent", Some("shell")).await;

    let invocation = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "echo async-ok",
            timeout_secs: Some(30),
            metadata: None,
            is_async: true,
        })
        .await
        .unwrap();

    assert_eq!(invocation.status, "running");

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let final_inv = loop {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let inv = processes::get_invocation(&state.pool, &invocation.id)
            .await
            .unwrap();
        if inv.status != "running" {
            break inv;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for async invocation to complete"
        );
    };
    assert_eq!(final_inv.status, "completed");
    assert!(
        final_inv.result.as_deref().unwrap().contains("async-ok"),
        "expected 'async-ok' in result, got: {:?}",
        final_inv.result
    );
}

// This test makes a live call to AWS Bedrock and cannot be converted to a
// mock-based test because its purpose is to verify actual LLM invocation
// end-to-end. All mock-based LLM dispatch scenarios are covered by the
// tests above. Run with: cargo test -- --ignored real_bedrock_claude_invoke
#[tokio::test]
#[ignore = "requires AWS_ACCESS_KEY_ID credentials"]
async fn real_bedrock_claude_invoke() {
    use nous_daemon::llm_client::LlmClient;
    use rig::client::ProviderClient;

    let (mut state, _tmp) = test_state().await;

    let client = LlmClient::from_env().expect("AWS credentials required");
    state.llm_client = Some(Arc::new(client));

    let agent_id =
        register_agent_with_process_type(&state, "real-claude-agent", Some("claude")).await;

    let invocation = state
        .process_registry
        .invoke(InvokeParams {
            state: &state,
            agent_id: &agent_id,
            prompt: "Respond with exactly: PONG",
            timeout_secs: Some(30),
            metadata: None,
            is_async: false,
        })
        .await
        .unwrap();

    assert_eq!(invocation.status, "completed");
    assert!(
        invocation.result.is_some(),
        "expected non-empty result from Bedrock"
    );
}
