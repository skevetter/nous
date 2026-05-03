use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::tools::{
    AgentTool, ExecutionPolicy, FileSystemPermission, RiskLevel, ToolCategory, ToolContent,
    ToolContext, ToolError, ToolMetadata, ToolOutput, ToolPermissions,
};

fn canonicalize_path(path: &std::path::Path) -> Result<PathBuf, ToolError> {
    if path.exists() {
        std::fs::canonicalize(path).map_err(|e| {
            ToolError::InvalidArgs(format!("cannot resolve path '{}': {e}", path.display()))
        })
    } else {
        let mut current = path.to_path_buf();
        let mut tail = Vec::new();
        loop {
            if current.exists() {
                let base = std::fs::canonicalize(&current).map_err(|e| {
                    ToolError::InvalidArgs(format!(
                        "cannot resolve path '{}': {e}",
                        current.display()
                    ))
                })?;
                let mut result = base;
                for component in tail.into_iter().rev() {
                    result = result.join(component);
                }
                return Ok(result);
            }
            match current.file_name() {
                Some(name) => {
                    tail.push(name.to_os_string());
                    current = current
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .ok_or_else(|| {
                            ToolError::InvalidArgs(format!("invalid path '{}'", path.display()))
                        })?
                        .to_path_buf();
                }
                None => {
                    return Err(ToolError::InvalidArgs(format!(
                        "invalid path '{}'",
                        path.display()
                    )));
                }
            }
        }
    }
}

fn check_read_permission(path: &std::path::Path, ctx: &ToolContext) -> Result<PathBuf, ToolError> {
    let canonical = canonicalize_path(path)?;
    if let Some(ref allowed) = ctx.permissions.allowed_paths {
        if !allowed.iter().any(|p| canonical.starts_with(p)) {
            return Err(ToolError::PermissionDenied(format!(
                "read access denied for '{}'",
                canonical.display()
            )));
        }
    }
    Ok(canonical)
}

fn check_write_permission(path: &std::path::Path, ctx: &ToolContext) -> Result<PathBuf, ToolError> {
    let canonical = canonicalize_path(path)?;
    if let Some(ref allowed) = ctx.permissions.allowed_paths {
        if !allowed.iter().any(|p| canonical.starts_with(p)) {
            return Err(ToolError::PermissionDenied(format!(
                "write access denied for '{}'",
                canonical.display()
            )));
        }
    }
    Ok(canonical)
}

// --- FsReadTool ---

#[derive(Default)]
pub struct FsReadTool {
    meta: OnceLock<ToolMetadata>,
}

impl FsReadTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "fs_read".into(),
            description: "Read file contents from the local filesystem".into(),
            category: ToolCategory::FileSystem,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute file path" },
                    "offset": { "type": "integer", "description": "Line offset (0-based)" },
                    "limit": { "type": "integer", "description": "Max lines to read" }
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
            tags: vec!["filesystem".into(), "read".into()],
        })
    }
}

impl AgentTool for FsReadTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'path' required".into()))?;
        let path = PathBuf::from(path);
        let path = check_read_permission(&path, ctx)?;

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("{}: {}", path.display(), e)))?;

        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;
        let lines: String = content
            .lines()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolOutput {
            content: vec![ToolContent::Text { text: lines }],
            metadata: None,
        })
    }
}

// --- FsWriteTool ---

#[derive(Default)]
pub struct FsWriteTool {
    meta: OnceLock<ToolMetadata>,
}

impl FsWriteTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "fs_write".into(),
            description: "Write or overwrite a file on the local filesystem".into(),
            category: ToolCategory::FileSystem,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute file path" },
                    "content": { "type": "string", "description": "File content to write" }
                },
                "required": ["path", "content"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                filesystem: Some(FileSystemPermission {
                    read_paths: vec![],
                    write_paths: vec!["**".into()],
                    deny_paths: vec![],
                }),
                risk_level: RiskLevel::Medium,
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                ..Default::default()
            },
            tags: vec!["filesystem".into(), "write".into()],
        })
    }
}

impl AgentTool for FsWriteTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'path' required".into()))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'content' required".into()))?;
        let path = PathBuf::from(path);
        let path = check_write_permission(&path, ctx)?;

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        }

        tokio::fs::write(&path, content)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!("wrote {} bytes to {}", content.len(), path.display()),
            }],
            metadata: None,
        })
    }
}

// --- FsEditTool ---

#[derive(Default)]
pub struct FsEditTool {
    meta: OnceLock<ToolMetadata>,
}

impl FsEditTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "fs_edit".into(),
            description: "Apply targeted edits to a file (old_string → new_string)".into(),
            category: ToolCategory::FileSystem,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute file path" },
                    "old_string": { "type": "string", "description": "Text to find and replace" },
                    "new_string": { "type": "string", "description": "Replacement text" },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences (default: false)" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                filesystem: Some(FileSystemPermission {
                    read_paths: vec!["**".into()],
                    write_paths: vec!["**".into()],
                    deny_paths: vec![],
                }),
                risk_level: RiskLevel::Medium,
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                ..Default::default()
            },
            tags: vec!["filesystem".into(), "edit".into()],
        })
    }
}

impl AgentTool for FsEditTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'path' required".into()))?;
        let old_string = args
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'old_string' required".into()))?;
        let new_string = args
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'new_string' required".into()))?;
        let replace_all = args
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let path = PathBuf::from(path);
        let path = check_write_permission(&path, ctx)?;

        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("{}: {}", path.display(), e)))?;
        let content = String::from_utf8(bytes)
            .map_err(|_| ToolError::InvalidArgs("file is not valid UTF-8".into()))?;

        if !content.contains(old_string) {
            return Err(ToolError::InvalidArgs(
                "old_string not found in file".into(),
            ));
        }

        let (new_content, count) = if replace_all {
            let count = content.matches(old_string).count();
            (content.replace(old_string, new_string), count)
        } else {
            (content.replacen(old_string, new_string, 1), 1)
        };

        tokio::fs::write(&path, &new_content)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!("replaced {} occurrence(s) in {}", count, path.display()),
            }],
            metadata: None,
        })
    }
}

// --- FsListTool ---

#[derive(Default)]
pub struct FsListTool {
    meta: OnceLock<ToolMetadata>,
}

impl FsListTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "fs_list".into(),
            description: "List directory contents with optional glob pattern".into(),
            category: ToolCategory::FileSystem,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path" },
                    "pattern": { "type": "string", "description": "Glob pattern to filter entries" }
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
            tags: vec!["filesystem".into(), "list".into()],
        })
    }
}

impl AgentTool for FsListTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'path' required".into()))?;
        let pattern = args.get("pattern").and_then(|v| v.as_str());
        let path = PathBuf::from(path);
        let path = check_read_permission(&path, ctx)?;

        if let Some(pat) = pattern {
            let full_pattern = path.join(pat).to_string_lossy().to_string();
            let entries: Vec<String> = glob::glob(&full_pattern)
                .map_err(|e| ToolError::InvalidArgs(format!("invalid glob: {e}")))?
                .filter_map(|r| r.ok())
                .map(|p| p.display().to_string())
                .collect();
            Ok(ToolOutput {
                content: vec![ToolContent::Text {
                    text: entries.join("\n"),
                }],
                metadata: None,
            })
        } else {
            let mut entries = Vec::new();
            let mut read_dir = tokio::fs::read_dir(&path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("{}: {}", path.display(), e)))?;
            while let Some(entry) = read_dir
                .next_entry()
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
            {
                let ft = entry
                    .file_type()
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
                let suffix = if ft.is_dir() { "/" } else { "" };
                entries.push(format!("{}{}", entry.file_name().to_string_lossy(), suffix));
            }
            entries.sort();
            Ok(ToolOutput {
                content: vec![ToolContent::Text {
                    text: entries.join("\n"),
                }],
                metadata: None,
            })
        }
    }
}

// --- FsSearchTool ---

#[derive(Default)]
pub struct FsSearchTool {
    meta: OnceLock<ToolMetadata>,
}

impl FsSearchTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "fs_search".into(),
            description: "Search file contents via regex pattern".into(),
            category: ToolCategory::FileSystem,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to search" },
                    "pattern": { "type": "string", "description": "Regex pattern" }
                },
                "required": ["path", "pattern"]
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
            tags: vec!["filesystem".into(), "search".into()],
        })
    }
}

impl AgentTool for FsSearchTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'path' required".into()))?;
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'pattern' required".into()))?;
        let path = PathBuf::from(path);
        let path = check_read_permission(&path, ctx)?;

        let re = regex::Regex::new(pattern)
            .map_err(|e| ToolError::InvalidArgs(format!("invalid regex: {e}")))?;

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("{}: {}", path.display(), e)))?;

        let mut matches = Vec::new();
        for (i, line) in content.lines().enumerate() {
            if re.is_match(line) {
                matches.push(format!("{}:{}", i + 1, line));
            }
        }

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: if matches.is_empty() {
                    "no matches found".into()
                } else {
                    matches.join("\n")
                },
            }],
            metadata: None,
        })
    }
}

// --- FsStatTool ---

#[derive(Default)]
pub struct FsStatTool {
    meta: OnceLock<ToolMetadata>,
}

impl FsStatTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "fs_stat".into(),
            description: "Get file metadata (size, modified, permissions)".into(),
            category: ToolCategory::FileSystem,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute file path" }
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
                timeout_secs: 5,
                idempotent: true,
                ..Default::default()
            },
            tags: vec!["filesystem".into(), "stat".into()],
        })
    }
}

impl AgentTool for FsStatTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'path' required".into()))?;
        let path = PathBuf::from(path);
        let path = check_read_permission(&path, ctx)?;

        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("{}: {}", path.display(), e)))?;

        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let file_type = if metadata.is_dir() {
            "directory"
        } else if metadata.is_symlink() {
            "symlink"
        } else {
            "file"
        };

        let info = json!({
            "path": path.display().to_string(),
            "size": metadata.len(),
            "type": file_type,
            "readonly": metadata.permissions().readonly(),
            "modified_epoch": modified,
        });

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: info }],
            metadata: None,
        })
    }
}

// --- FsMkdirTool ---

#[derive(Default)]
pub struct FsMkdirTool {
    meta: OnceLock<ToolMetadata>,
}

impl FsMkdirTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "fs_mkdir".into(),
            description: "Create directories (including parents)".into(),
            category: ToolCategory::FileSystem,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path to create" }
                },
                "required": ["path"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                filesystem: Some(FileSystemPermission {
                    read_paths: vec![],
                    write_paths: vec!["**".into()],
                    deny_paths: vec![],
                }),
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 5,
                ..Default::default()
            },
            tags: vec!["filesystem".into(), "mkdir".into()],
        })
    }
}

impl AgentTool for FsMkdirTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'path' required".into()))?;
        let path = PathBuf::from(path);
        let path = check_write_permission(&path, ctx)?;

        tokio::fs::create_dir_all(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!("created directory {}", path.display()),
            }],
            metadata: None,
        })
    }
}

// --- FsDeleteTool ---

#[derive(Default)]
pub struct FsDeleteTool {
    meta: OnceLock<ToolMetadata>,
}

impl FsDeleteTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "fs_delete".into(),
            description: "Delete files or directories".into(),
            category: ToolCategory::FileSystem,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to delete" }
                },
                "required": ["path"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                filesystem: Some(FileSystemPermission {
                    read_paths: vec![],
                    write_paths: vec!["**".into()],
                    deny_paths: vec![],
                }),
                requires_confirmation: true,
                risk_level: RiskLevel::High,
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                ..Default::default()
            },
            tags: vec!["filesystem".into(), "delete".into()],
        })
    }
}

impl AgentTool for FsDeleteTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'path' required".into()))?;
        let path = PathBuf::from(path);
        let path = check_write_permission(&path, ctx)?;

        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("{}: {}", path.display(), e)))?;

        if metadata.is_dir() {
            tokio::fs::remove_dir_all(&path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        } else {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        }

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!("deleted {}", path.display()),
            }],
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

    fn test_ctx() -> ToolContext {
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
            },
            services: None,
        }
    }

    #[tokio::test]
    async fn fs_read_write_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        let ctx = test_ctx();

        let write_tool = FsWriteTool::new();
        write_tool
            .call(
                json!({"path": file.to_str().unwrap(), "content": "hello\nworld\n"}),
                &ctx,
            )
            .await
            .unwrap();

        let read_tool = FsReadTool::new();
        let output = read_tool
            .call(json!({"path": file.to_str().unwrap()}), &ctx)
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("hello"));
            assert!(text.contains("world"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn fs_read_with_offset_limit() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lines.txt");
        let ctx = test_ctx();

        let content = (0..10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(&file, &content).await.unwrap();

        let tool = FsReadTool::new();
        let output = tool
            .call(
                json!({"path": file.to_str().unwrap(), "offset": 2, "limit": 3}),
                &ctx,
            )
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert_eq!(text, "line 2\nline 3\nline 4");
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn fs_edit_replaces_text() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("edit.txt");
        let ctx = test_ctx();

        tokio::fs::write(&file, "foo bar baz foo").await.unwrap();

        let tool = FsEditTool::new();
        tool.call(
            json!({"path": file.to_str().unwrap(), "old_string": "foo", "new_string": "qux"}),
            &ctx,
        )
        .await
        .unwrap();

        let content = tokio::fs::read_to_string(&file).await.unwrap();
        assert_eq!(content, "qux bar baz foo");
    }

    #[tokio::test]
    async fn fs_stat_returns_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("stat.txt");
        let ctx = test_ctx();

        tokio::fs::write(&file, "data").await.unwrap();

        let tool = FsStatTool::new();
        let output = tool
            .call(json!({"path": file.to_str().unwrap()}), &ctx)
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["size"], 4);
            assert_eq!(data["type"], "file");
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn fs_mkdir_creates_nested() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        let ctx = test_ctx();

        let tool = FsMkdirTool::new();
        tool.call(json!({"path": nested.to_str().unwrap()}), &ctx)
            .await
            .unwrap();

        assert!(nested.exists());
    }

    #[tokio::test]
    async fn fs_list_directory() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.txt"), "")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.txt"), "")
            .await
            .unwrap();
        let ctx = test_ctx();

        let tool = FsListTool::new();
        let output = tool
            .call(json!({"path": dir.path().to_str().unwrap()}), &ctx)
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("a.txt"));
            assert!(text.contains("b.txt"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn fs_search_with_regex() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("search.txt");
        tokio::fs::write(&file, "hello world\nfoo bar\nhello rust")
            .await
            .unwrap();
        let ctx = test_ctx();

        let tool = FsSearchTool::new();
        let output = tool
            .call(
                json!({"path": file.to_str().unwrap(), "pattern": "hello"}),
                &ctx,
            )
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("1:hello world"));
            assert!(text.contains("3:hello rust"));
            assert!(!text.contains("foo bar"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn fs_delete_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("del.txt");
        tokio::fs::write(&file, "gone").await.unwrap();
        let ctx = test_ctx();

        let tool = FsDeleteTool::new();
        tool.call(json!({"path": file.to_str().unwrap()}), &ctx)
            .await
            .unwrap();

        assert!(!file.exists());
    }
}
