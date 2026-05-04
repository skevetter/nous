use std::path::PathBuf;
use std::sync::{LazyLock, OnceLock};

use regex::Regex;
use serde_json::{json, Value};

use crate::tools::{
    AgentTool, ExecutionPolicy, FileSystemPermission, ToolCategory, ToolContent, ToolContext,
    ToolError, ToolMetadata, ToolOutput, ToolPermissions,
};

// --- CodeGrepTool ---

#[derive(Default)]
pub struct CodeGrepTool {
    meta: OnceLock<ToolMetadata>,
}

impl CodeGrepTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "code_grep".into(),
            description: "Search codebase with regex patterns".into(),
            category: ToolCategory::CodeAnalysis,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex pattern to search" },
                    "path": { "type": "string", "description": "Directory to search in (defaults to workspace)" },
                    "glob": { "type": "string", "description": "File glob filter (e.g. '*.rs')" }
                },
                "required": ["pattern"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                filesystem: Some(FileSystemPermission {
                    read_paths: vec!["**".into()],
                    write_paths: vec![],
                    deny_paths: vec![],
                }),
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 30,
                idempotent: true,
                ..Default::default()
            },
            tags: vec!["code".into(), "grep".into(), "search".into()],
        })
    }
}

impl AgentTool for CodeGrepTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'pattern' required".into()))?;
        let search_path = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .or_else(|| ctx.workspace_dir.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        let glob_filter = args.get("glob").and_then(|v| v.as_str());

        let re = regex::Regex::new(pattern)
            .map_err(|e| ToolError::InvalidArgs(format!("invalid regex: {e}")))?;

        let file_glob = if let Some(g) = glob_filter {
            search_path.join("**").join(g)
        } else {
            search_path.join("**").join("*")
        };

        let entries: Vec<PathBuf> = glob::glob(&file_glob.to_string_lossy())
            .map_err(|e| ToolError::InvalidArgs(format!("invalid glob: {e}")))?
            .filter_map(|r| r.ok())
            .filter(|p| p.is_file())
            .collect();

        let mut results = Vec::new();
        let max_results = 200;

        for entry in entries {
            if results.len() >= max_results {
                break;
            }
            if let Ok(content) = tokio::fs::read_to_string(&entry).await {
                for (i, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        results.push(format!("{}:{}:{}", entry.display(), i + 1, line));
                        if results.len() >= max_results {
                            break;
                        }
                    }
                }
            }
        }

        let text = if results.is_empty() {
            "no matches found".into()
        } else {
            let count = results.len();
            let truncated = if count >= max_results {
                format!("\n\n[Showing first {max_results} matches]")
            } else {
                String::new()
            };
            format!("{}{}", results.join("\n"), truncated)
        };

        Ok(ToolOutput {
            content: vec![ToolContent::Text { text }],
            metadata: None,
        })
    }
}

// --- CodeGlobTool ---

#[derive(Default)]
pub struct CodeGlobTool {
    meta: OnceLock<ToolMetadata>,
}

impl CodeGlobTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "code_glob".into(),
            description: "Find files by glob pattern".into(),
            category: ToolCategory::CodeAnalysis,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern (e.g. '**/*.rs')" },
                    "path": { "type": "string", "description": "Base directory (defaults to workspace)" }
                },
                "required": ["pattern"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                filesystem: Some(FileSystemPermission {
                    read_paths: vec!["**".into()],
                    write_paths: vec![],
                    deny_paths: vec![],
                }),
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                idempotent: true,
                ..Default::default()
            },
            tags: vec!["code".into(), "glob".into(), "find".into()],
        })
    }
}

impl AgentTool for CodeGlobTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'pattern' required".into()))?;
        let base = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .or_else(|| ctx.workspace_dir.clone())
            .unwrap_or_else(|| PathBuf::from("."));

        let full_pattern = base.join(pattern).to_string_lossy().to_string();
        let entries: Vec<String> = glob::glob(&full_pattern)
            .map_err(|e| ToolError::InvalidArgs(format!("invalid glob: {e}")))?
            .filter_map(|r| r.ok())
            .take(1000)
            .map(|p| p.display().to_string())
            .collect();

        let text = if entries.is_empty() {
            "no files found".into()
        } else {
            entries.join("\n")
        };

        Ok(ToolOutput {
            content: vec![ToolContent::Text { text }],
            metadata: None,
        })
    }
}

// --- CodeSymbolsTool ---

struct LangPatterns {
    patterns: &'static [(&'static str, &'static LazyLock<Regex>)],
}

static RE_RS_FN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(pub\s+)?(async\s+)?fn\s+(\w+)").unwrap());
static RE_RS_STRUCT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(pub\s+)?struct\s+(\w+)").unwrap());
static RE_RS_ENUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(pub\s+)?enum\s+(\w+)").unwrap());
static RE_RS_TRAIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(pub\s+)?trait\s+(\w+)").unwrap());
static RE_RS_IMPL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*impl\s+(<[^>]+>\s+)?(\w+)").unwrap());
static RE_RS_USE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*use\s+").unwrap());
static RE_RS_MOD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(pub\s+)?mod\s+(\w+)").unwrap());

static RE_PY_DEF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(async\s+)?def\s+(\w+)").unwrap());
static RE_PY_CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*class\s+(\w+)").unwrap());
static RE_PY_IMPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(import|from)\s+").unwrap());

static RE_JS_FUNCTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(export\s+)?(async\s+)?function\s+(\w+)").unwrap());
static RE_JS_CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(export\s+)?class\s+(\w+)").unwrap());
static RE_JS_INTERFACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(export\s+)?interface\s+(\w+)").unwrap());
static RE_JS_TYPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(export\s+)?type\s+(\w+)").unwrap());
static RE_JS_IMPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*import\s+").unwrap());
static RE_JS_CONST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(export\s+)?(const|let)\s+(\w+)").unwrap());

static RE_GO_FUNC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^func\s+(\([^)]*\)\s+)?(\w+)").unwrap());
static RE_GO_TYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^type\s+(\w+)").unwrap());
static RE_GO_IMPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*import\s+").unwrap());

static RE_GENERIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(fn|func|def|function)\s+\w+").unwrap());

static PATTERNS_RS: LangPatterns = LangPatterns {
    patterns: &[
        ("fn", &RE_RS_FN),
        ("struct", &RE_RS_STRUCT),
        ("enum", &RE_RS_ENUM),
        ("trait", &RE_RS_TRAIT),
        ("impl", &RE_RS_IMPL),
        ("use", &RE_RS_USE),
        ("mod", &RE_RS_MOD),
    ],
};

static PATTERNS_PY: LangPatterns = LangPatterns {
    patterns: &[
        ("def", &RE_PY_DEF),
        ("class", &RE_PY_CLASS),
        ("import", &RE_PY_IMPORT),
    ],
};

static PATTERNS_JS: LangPatterns = LangPatterns {
    patterns: &[
        ("function", &RE_JS_FUNCTION),
        ("class", &RE_JS_CLASS),
        ("interface", &RE_JS_INTERFACE),
        ("type", &RE_JS_TYPE),
        ("import", &RE_JS_IMPORT),
        ("const/let", &RE_JS_CONST),
    ],
};

static PATTERNS_GO: LangPatterns = LangPatterns {
    patterns: &[
        ("func", &RE_GO_FUNC),
        ("type", &RE_GO_TYPE),
        ("import", &RE_GO_IMPORT),
    ],
};

static PATTERNS_GENERIC: LangPatterns = LangPatterns {
    patterns: &[("function", &RE_GENERIC)],
};

#[derive(Default)]
pub struct CodeSymbolsTool {
    meta: OnceLock<ToolMetadata>,
}

impl CodeSymbolsTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "code_symbols".into(),
            description: "List functions, types, and imports in a file".into(),
            category: ToolCategory::CodeAnalysis,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to analyze" }
                },
                "required": ["path"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                filesystem: Some(FileSystemPermission {
                    read_paths: vec!["**".into()],
                    write_paths: vec![],
                    deny_paths: vec![],
                }),
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                idempotent: true,
                ..Default::default()
            },
            tags: vec!["code".into(), "symbols".into(), "analysis".into()],
        })
    }
}

impl AgentTool for CodeSymbolsTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'path' required".into()))?;
        let path = PathBuf::from(path);

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("{}: {}", path.display(), e)))?;

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let lang = match ext {
            "rs" => &PATTERNS_RS,
            "py" => &PATTERNS_PY,
            "ts" | "tsx" | "js" | "jsx" => &PATTERNS_JS,
            "go" => &PATTERNS_GO,
            _ => &PATTERNS_GENERIC,
        };

        let mut symbols = Vec::new();
        for (i, line) in content.lines().enumerate() {
            for &(kind, re) in lang.patterns {
                if re.is_match(line) {
                    symbols.push(format!(
                        "{}:{} [{}] {}",
                        i + 1,
                        line.trim(),
                        kind,
                        path.display()
                    ));
                    break;
                }
            }
        }

        let text = if symbols.is_empty() {
            "no symbols found".into()
        } else {
            symbols.join("\n")
        };

        Ok(ToolOutput {
            content: vec![ToolContent::Text { text }],
            metadata: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::tools::{NetworkPolicy, ResolvedPermissions, ToolContext};

    fn test_ctx_with_dir(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            agent_id: "test-agent".into(),
            agent_name: "test".into(),
            namespace: "test".into(),
            workspace_dir: Some(dir.to_path_buf()),
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

    #[tokio::test]
    async fn code_glob_finds_files() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "fn main() {}")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.rs"), "fn test() {}")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("c.txt"), "text")
            .await
            .unwrap();

        let tool = CodeGlobTool::new();
        let ctx = test_ctx_with_dir(dir.path());
        let output = tool.call(json!({"pattern": "*.rs"}), &ctx).await.unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("a.rs"));
            assert!(text.contains("b.rs"));
            assert!(!text.contains("c.txt"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn code_grep_finds_pattern() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("main.rs"), "fn hello() {}\nfn world() {}")
            .await
            .unwrap();

        let tool = CodeGrepTool::new();
        let ctx = test_ctx_with_dir(dir.path());
        let output = tool
            .call(json!({"pattern": "fn hello"}), &ctx)
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("fn hello"));
            assert!(!text.contains("fn world"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn code_symbols_rust() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        tokio::fs::write(
            &file,
            "use std::io;\n\npub struct Foo;\n\nimpl Foo {\n    pub fn bar() {}\n}\n",
        )
        .await
        .unwrap();

        let tool = CodeSymbolsTool::new();
        let ctx = test_ctx_with_dir(dir.path());
        let output = tool
            .call(json!({"path": file.to_str().unwrap()}), &ctx)
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("[use]"));
            assert!(text.contains("[struct]"));
            assert!(text.contains("[fn]"));
        } else {
            panic!("expected text content");
        }
    }
}
