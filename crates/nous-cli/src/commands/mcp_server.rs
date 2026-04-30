use std::sync::Arc;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::notifications::NotificationRegistry;
use nous_daemon::routes::mcp::{dispatch, get_tool_schemas};
use nous_daemon::state::AppState;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub async fn run(tools_filter: Option<String>) {
    if let Err(e) = execute(tools_filter).await {
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

async fn execute(tools_filter: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;
    config.ensure_dirs()?;

    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations().await?;

    let state = AppState {
        pool: pools.fts.clone(),
        registry: Arc::new(NotificationRegistry::new()),
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
        let method = request
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("");

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
                    .filter(|t| {
                        match &prefixes {
                            None => true,
                            Some(pfs) => pfs.iter().any(|p| t.name.starts_with(p)),
                        }
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
                let tool_name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));

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
