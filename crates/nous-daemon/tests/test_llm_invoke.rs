use nous_core::agents::processes;
use nous_core::agents::{self, AgentType, RegisterAgentRequest};
use nous_core::db::DbPools;
use nous_core::error::NousError;
use nous_core::memory::MockEmbedder;
use nous_core::notifications::NotificationRegistry;
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
        llm_client: None,
    };
    (state, tmp)
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

// --- Sync claude invoke with no LlmClient ---

#[tokio::test]
async fn sync_claude_invoke_no_llm_client_returns_config_error() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent_with_process_type(&state, "claude-sync", Some("claude")).await;

    let result = state
        .process_registry
        .invoke(&state, &agent_id, "hello", None, None, false)
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Config(msg) if msg.contains("LLM client not configured")),
        "expected Config error about LLM client, got: {err}"
    );
}

// --- Async claude invoke with no LlmClient ---

#[tokio::test]
async fn async_claude_invoke_no_llm_client_returns_config_error() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent_with_process_type(&state, "claude-async", Some("claude")).await;

    let result = state
        .process_registry
        .invoke(&state, &agent_id, "hello", None, None, true)
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(&err, NousError::Config(msg) if msg.contains("LLM client not configured")),
        "expected Config error about LLM client, got: {err}"
    );
}

// --- Unsupported process_type ---

#[tokio::test]
async fn unsupported_process_type_returns_config_error() {
    let (state, _tmp) = test_state().await;
    let agent_id = register_agent_with_process_type(&state, "http-agent", Some("http")).await;

    let result = state
        .process_registry
        .invoke(&state, &agent_id, "hello", None, None, false)
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
        .invoke(&state, &agent_id, "echo hello", Some(30), None, false)
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
        .invoke(&state, &agent_id, "echo test", Some(30), None, false)
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
        .invoke(&state, &agent_id, "hello", None, None, false)
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
        .invoke(&state, &agent_id, "hello", None, None, false)
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
        .invoke(&state, &agent_id, "echo async-ok", Some(30), None, true)
        .await
        .unwrap();

    assert_eq!(invocation.status, "running");

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let final_inv = processes::get_invocation(&state.pool, &invocation.id)
        .await
        .unwrap();
    assert_eq!(final_inv.status, "completed");
    assert!(
        final_inv.result.as_deref().unwrap().contains("async-ok"),
        "expected 'async-ok' in result, got: {:?}",
        final_inv.result
    );
}

// --- Real Bedrock test (behind #[ignore]) ---

#[tokio::test]
#[ignore]
async fn real_bedrock_claude_invoke() {
    use nous_daemon::llm_client::LlmClient;

    let (mut state, _tmp) = test_state().await;

    let client = LlmClient::from_env().expect("AWS credentials required");
    state.llm_client = Some(Arc::new(client));

    let agent_id =
        register_agent_with_process_type(&state, "real-claude-agent", Some("claude")).await;

    let invocation = state
        .process_registry
        .invoke(
            &state,
            &agent_id,
            "Respond with exactly: PONG",
            Some(30),
            None,
            false,
        )
        .await
        .unwrap();

    assert_eq!(invocation.status, "completed");
    assert!(
        invocation.result.is_some(),
        "expected non-empty result from Bedrock"
    );
}
