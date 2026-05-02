use std::sync::Arc;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::memory::OnnxEmbeddingModel;
use nous_core::notifications::NotificationRegistry;
use nous_daemon::process_manager::ProcessRegistry;
use nous_daemon::routes::mcp::{dispatch, get_tool_schemas};
use nous_daemon::state::AppState;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub async fn run(
    tools_filter: Option<String>,
    model: Option<String>,
    region: Option<String>,
    profile: Option<String>,
    port: Option<u16>,
) {
    if let Err(e) = execute(tools_filter, model, region, profile, port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn prefix_to_tool_prefix(prefix: &str) -> &str {
    match prefix.trim() {
        "chat" => "room_",
        "task" => "task_",
        "memory" => "memory_",
        "agent" => "agent_",
        "artifact" => "artifact_",
        "worktree" => "worktree_",
        "schedule" => "schedule_",
        "inventory" => "inventory_",
        other => other,
    }
}

fn build_prefixes(filter: &str) -> Vec<&str> {
    filter.split(',').map(prefix_to_tool_prefix).collect()
}

async fn execute(
    tools_filter: Option<String>,
    model: Option<String>,
    region: Option<String>,
    profile: Option<String>,
    port: Option<u16>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    if let Some(p) = port {
        config.port = p;
    }
    config.ensure_dirs()?;

    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations(&config.search.tokenizer).await?;

    let embedder: Option<Arc<dyn nous_core::memory::Embedder>> =
        match OnnxEmbeddingModel::load(None) {
            Ok(model) => Some(Arc::new(model)),
            Err(e) => {
                tracing::warn!("embedding model not available, vector/hybrid search disabled: {e}");
                None
            }
        };

    use nous_daemon::llm_client::{build_client, LlmConfig};

    let llm_config = LlmConfig::resolve(model, region, profile);

    let has_credentials = std::env::var("AWS_ACCESS_KEY_ID").is_ok()
        || std::env::var("AWS_PROFILE").is_ok()
        || std::env::var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").is_ok();

    let (llm_client, default_model) = if has_credentials {
        let client = build_client(&llm_config).await;
        tracing::info!(region = %llm_config.region, model = %llm_config.model, "LLM client configured for Bedrock");
        (Some(Arc::new(client)), llm_config.model)
    } else {
        tracing::warn!("LLM client not available (no AWS credentials found in environment)");
        (None, llm_config.model)
    };

    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry: Arc::new(NotificationRegistry::new()),
        embedder,
        schedule_notify: Arc::new(Notify::new()),
        shutdown: CancellationToken::new(),
        process_registry: Arc::new(ProcessRegistry::new()),
        llm_client,
        default_model,
        #[cfg(feature = "sandbox")]
        sandbox_manager: None,
    };

    let prefixes: Option<Vec<&str>> = tools_filter.as_deref().map(build_prefixes);

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("parse error: {e}") }
                });
                let out = serde_json::to_string(&err_resp)? + "\n";
                stdout.write_all(out.as_bytes()).await?;
                stdout.flush().await?;
                continue;
            }
        };

        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");

        let response = match method {
            "initialize" => {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": { "listChanged": false }
                        },
                        "serverInfo": {
                            "name": "nous",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                })
            }
            "notifications/initialized" => continue,
            "tools/list" => {
                let schemas = get_tool_schemas();
                let tools: Vec<Value> = schemas
                    .iter()
                    .filter(|t| match &prefixes {
                        None => true,
                        Some(pfs) => pfs.iter().any(|p| t.name.starts_with(p)),
                    })
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name,
                            "description": t.description,
                            "inputSchema": t.input_schema
                        })
                    })
                    .collect();
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tools }
                })
            }
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(Value::Null);
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));

                // Enforce --tools filter: reject calls to tools not in the allowed set
                if let Some(ref pfs) = prefixes {
                    if !pfs.iter().any(|p| tool_name.starts_with(p)) {
                        let out = serde_json::to_string(&serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32601,
                                "message": format!("tool not available: {tool_name}")
                            }
                        }))? + "\n";
                        stdout.write_all(out.as_bytes()).await?;
                        stdout.flush().await?;
                        continue;
                    }
                }

                match dispatch(&state, tool_name, &arguments).await {
                    Ok(result) => {
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": serde_json::to_string(&result).unwrap_or_default()
                                }]
                            }
                        })
                    }
                    Err(e) => {
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": e.to_string()
                                }],
                                "isError": true
                            }
                        })
                    }
                }
            }
            _ => {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32601,
                        "message": format!("method not found: {method}")
                    }
                })
            }
        };

        let out = serde_json::to_string(&response)? + "\n";
        stdout.write_all(out.as_bytes()).await?;
        stdout.flush().await?;
    }

    pools.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nous_daemon::routes::mcp::get_tool_schemas;

    #[test]
    fn prefix_mapping_known_categories() {
        assert_eq!(prefix_to_tool_prefix("chat"), "room_");
        assert_eq!(prefix_to_tool_prefix("task"), "task_");
        assert_eq!(prefix_to_tool_prefix("memory"), "memory_");
        assert_eq!(prefix_to_tool_prefix("agent"), "agent_");
        assert_eq!(prefix_to_tool_prefix("artifact"), "artifact_");
        assert_eq!(prefix_to_tool_prefix("worktree"), "worktree_");
        assert_eq!(prefix_to_tool_prefix("schedule"), "schedule_");
        assert_eq!(prefix_to_tool_prefix("inventory"), "inventory_");
    }

    #[test]
    fn prefix_mapping_unknown_passthrough() {
        assert_eq!(prefix_to_tool_prefix("custom_"), "custom_");
        assert_eq!(prefix_to_tool_prefix("foo"), "foo");
    }

    #[test]
    fn build_prefixes_single() {
        let prefixes = build_prefixes("chat");
        assert_eq!(prefixes, vec!["room_"]);
    }

    #[test]
    fn build_prefixes_multiple() {
        let prefixes = build_prefixes("chat,task");
        assert_eq!(prefixes, vec!["room_", "task_"]);
    }

    #[test]
    fn tools_list_filter_with_chat_task_returns_only_matching() {
        let schemas = get_tool_schemas();
        let prefixes = build_prefixes("chat,task");

        let filtered: Vec<_> = schemas
            .iter()
            .filter(|t| prefixes.iter().any(|p| t.name.starts_with(p)))
            .collect();

        assert!(!filtered.is_empty());
        for tool in &filtered {
            assert!(
                tool.name.starts_with("room_") || tool.name.starts_with("task_"),
                "unexpected tool in filtered set: {}",
                tool.name
            );
        }
    }

    #[test]
    fn tools_list_no_filter_returns_all() {
        let schemas = get_tool_schemas();
        // Without filter, all tools should be present
        assert!(
            schemas.len() >= 50,
            "expected many tools, got {}",
            schemas.len()
        );

        // Verify multiple categories are present
        let has_room = schemas.iter().any(|t| t.name.starts_with("room_"));
        let has_task = schemas.iter().any(|t| t.name.starts_with("task_"));
        let has_memory = schemas.iter().any(|t| t.name.starts_with("memory_"));
        let has_agent = schemas.iter().any(|t| t.name.starts_with("agent_"));
        assert!(has_room, "missing room_ tools");
        assert!(has_task, "missing task_ tools");
        assert!(has_memory, "missing memory_ tools");
        assert!(has_agent, "missing agent_ tools");
    }

    #[test]
    fn tools_call_filter_rejects_non_matching_tool() {
        // Simulate the filter check that happens in the tools/call handler
        let prefixes = build_prefixes("chat,task");
        let tool_name = "memory_save";

        let allowed = prefixes.iter().any(|p| tool_name.starts_with(p));
        assert!(
            !allowed,
            "memory_save should be rejected when filter is chat,task"
        );
    }

    #[test]
    fn tools_call_filter_allows_matching_tool() {
        let prefixes = build_prefixes("chat,task");

        assert!(prefixes.iter().any(|p| "room_create".starts_with(p)));
        assert!(prefixes.iter().any(|p| "task_create".starts_with(p)));
    }

    #[test]
    fn tools_call_no_filter_allows_everything() {
        let prefixes: Option<Vec<&str>> = None;
        let tool_name = "memory_save";

        let allowed = match &prefixes {
            None => true,
            Some(pfs) => pfs.iter().any(|p| tool_name.starts_with(p)),
        };
        assert!(allowed, "with no filter, all tools should be allowed");
    }
}
