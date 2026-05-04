use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::net::validate_url;
use crate::tools::{
    AgentTool, ExecutionPolicy, NetworkPermission, RiskLevel, ToolCategory, ToolContent,
    ToolContext, ToolError, ToolMetadata, ToolOutput, ToolPermissions,
};

fn check_network_permission(url: &str, ctx: &ToolContext) -> Result<(), ToolError> {
    let net_perm = ctx.permissions.network.as_ref().ok_or_else(|| {
        ToolError::PermissionDenied(
            "network access denied: no network permissions configured".into(),
        )
    })?;

    let parsed = reqwest::Url::parse(url)
        .map_err(|e| ToolError::InvalidArgs(format!("invalid URL: {e}")))?;
    let host = parsed.host_str().ok_or_else(|| {
        ToolError::InvalidArgs("URL has no host".into())
    })?;

    if net_perm.denied_hosts.iter().any(|d| d == host) {
        return Err(ToolError::PermissionDenied(format!(
            "network access denied: host '{host}' is in the denied list"
        )));
    }

    if !net_perm.allowed_hosts.iter().any(|a| a == host || a == "*") {
        return Err(ToolError::PermissionDenied(format!(
            "network access denied: host '{host}' is not in the allowed list"
        )));
    }

    Ok(())
}

// --- HttpRequestTool ---

#[derive(Default)]
pub struct HttpRequestTool {
    meta: OnceLock<ToolMetadata>,
}

impl HttpRequestTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "http_request".into(),
            description: "Make an HTTP request (GET/POST/PUT/DELETE)".into(),
            category: ToolCategory::Http,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Request URL" },
                    "method": { "type": "string", "enum": ["GET", "POST", "PUT", "DELETE"], "description": "HTTP method (default: GET)" },
                    "headers": { "type": "object", "description": "Request headers as key-value pairs" },
                    "body": { "type": "string", "description": "Request body (for POST/PUT)" }
                },
                "required": ["url"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                network: Some(NetworkPermission {
                    allowed_hosts: vec![],
                    denied_hosts: vec![],
                    max_request_size_bytes: Some(10_485_760),
                }),
                risk_level: RiskLevel::Medium,
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 30,
                ..Default::default()
            },
            tags: vec!["http".into(), "request".into()],
        })
    }
}

impl AgentTool for HttpRequestTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'url' required".into()))?;

        check_network_permission(url, ctx)?;
        validate_url(url).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;

        let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET");

        let client = reqwest::Client::new();
        let mut request = match method.to_uppercase().as_str() {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "DELETE" => client.delete(url),
            other => {
                return Err(ToolError::InvalidArgs(format!(
                    "unsupported method: {other}"
                )))
            }
        };

        if let Some(headers) = args.get("headers").and_then(|v| v.as_object()) {
            for (k, v) in headers {
                if let Some(val) = v.as_str() {
                    let header_name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                        .map_err(|e| ToolError::InvalidArgs(format!("invalid header name: {e}")))?;
                    let header_val = reqwest::header::HeaderValue::from_str(val).map_err(|e| {
                        ToolError::InvalidArgs(format!("invalid header value: {e}"))
                    })?;
                    request = request.header(header_name, header_val);
                }
            }
        }

        if let Some(body) = args.get("body").and_then(|v| v.as_str()) {
            request = request.body(body.to_string());
        }

        let response = request
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let status = response.status().as_u16();
        let headers: Value = response
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), json!(v.to_str().unwrap_or(""))))
            .collect::<serde_json::Map<String, Value>>()
            .into();

        let body = response
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text { text: body }],
            metadata: Some(json!({
                "status": status,
                "headers": headers,
            })),
        })
    }
}

// --- HttpFetchTool ---

#[derive(Default)]
pub struct HttpFetchTool {
    meta: OnceLock<ToolMetadata>,
}

impl HttpFetchTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "http_fetch".into(),
            description: "Fetch URL content and extract text".into(),
            category: ToolCategory::Http,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch" },
                    "max_bytes": { "type": "integer", "description": "Maximum bytes to fetch (default: 1048576)" }
                },
                "required": ["url"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                network: Some(NetworkPermission {
                    allowed_hosts: vec![],
                    denied_hosts: vec![],
                    max_request_size_bytes: Some(1_048_576),
                }),
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 30,
                idempotent: true,
                ..Default::default()
            },
            tags: vec!["http".into(), "fetch".into()],
        })
    }
}

impl AgentTool for HttpFetchTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'url' required".into()))?;

        check_network_permission(url, ctx)?;
        validate_url(url).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;

        let max_bytes = args
            .get("max_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(1_048_576) as usize;

        let response = reqwest::get(url)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        let bytes = response
            .bytes()
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let text = if bytes.len() > max_bytes {
            let truncated = String::from_utf8_lossy(&bytes[..max_bytes]);
            format!(
                "{truncated}\n\n[Truncated at {max_bytes} bytes. Total: {} bytes]",
                bytes.len()
            )
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };

        Ok(ToolOutput {
            content: vec![ToolContent::Text { text }],
            metadata: Some(json!({
                "status": status,
                "content_type": content_type,
                "size": bytes.len(),
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::tools::{NetworkPolicy, ResolvedPermissions, ToolContext};

    fn test_ctx_no_network() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".into(),
            agent_name: "test".into(),
            namespace: "test".into(),
            workspace_dir: None,
            session_id: None,
            timeout: Duration::from_secs(30),
            permissions: ResolvedPermissions {
                allowed_tools: None,
                denied_tools: None,
                allowed_paths: None,
                network_access: NetworkPolicy::None,
                max_output_bytes: 1_048_576,
                shell: None,
                network: None,
            },
            services: None,
        }
    }

    #[test]
    fn http_request_tool_metadata() {
        let tool = HttpRequestTool::new();
        let meta = tool.metadata();
        assert_eq!(meta.name, "http_request");
        assert_eq!(meta.category, ToolCategory::Http);
        assert_eq!(meta.permissions.risk_level, RiskLevel::Medium);
    }

    #[test]
    fn http_fetch_tool_metadata() {
        let tool = HttpFetchTool::new();
        let meta = tool.metadata();
        assert_eq!(meta.name, "http_fetch");
        assert_eq!(meta.category, ToolCategory::Http);
    }

    #[tokio::test]
    async fn http_denied_by_default_no_permissions() {
        let ctx = test_ctx_no_network();
        let tool = HttpRequestTool::new();
        let result = tool
            .call(json!({"url": "https://example.com"}), &ctx)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied(_)));
    }

    #[tokio::test]
    async fn http_allowed_with_matching_host() {
        let ctx = ToolContext {
            agent_id: "test-agent".into(),
            agent_name: "test".into(),
            namespace: "test".into(),
            workspace_dir: None,
            session_id: None,
            timeout: Duration::from_secs(30),
            permissions: ResolvedPermissions {
                allowed_tools: None,
                denied_tools: None,
                allowed_paths: None,
                network_access: NetworkPolicy::Unrestricted,
                max_output_bytes: 1_048_576,
                shell: None,
                network: Some(NetworkPermission {
                    allowed_hosts: vec!["example.com".into()],
                    denied_hosts: vec![],
                    max_request_size_bytes: None,
                }),
            },
            services: None,
        };
        let tool = HttpRequestTool::new();
        let result = tool
            .call(json!({"url": "https://example.com/path"}), &ctx)
            .await;
        // Permission check passes (request may fail due to network, but that's OK)
        // We verify the permission check itself doesn't block it
        assert!(!matches!(result, Err(ToolError::PermissionDenied(_))));
    }

    #[tokio::test]
    async fn http_denied_host_rejected() {
        let ctx = ToolContext {
            agent_id: "test-agent".into(),
            agent_name: "test".into(),
            namespace: "test".into(),
            workspace_dir: None,
            session_id: None,
            timeout: Duration::from_secs(30),
            permissions: ResolvedPermissions {
                allowed_tools: None,
                denied_tools: None,
                allowed_paths: None,
                network_access: NetworkPolicy::Unrestricted,
                max_output_bytes: 1_048_576,
                shell: None,
                network: Some(NetworkPermission {
                    allowed_hosts: vec!["*".into()],
                    denied_hosts: vec!["evil.com".into()],
                    max_request_size_bytes: None,
                }),
            },
            services: None,
        };
        let tool = HttpRequestTool::new();
        let result = tool
            .call(json!({"url": "https://evil.com/steal"}), &ctx)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied(_)));
    }
}
