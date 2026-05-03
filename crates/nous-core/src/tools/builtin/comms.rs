use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::tools::{
    AgentTool, ExecutionPolicy, ToolCategory, ToolContent, ToolContext, ToolError, ToolMetadata,
    ToolOutput, ToolPermissions,
};

// --- RoomPostTool ---

#[derive(Default)]
pub struct RoomPostTool {
    meta: OnceLock<ToolMetadata>,
}

impl RoomPostTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "room_post".into(),
            description: "Post a message to a chat room".into(),
            category: ToolCategory::AgentComms,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "room": { "type": "string", "description": "Room name or ID" },
                    "content": { "type": "string", "description": "Message content" },
                    "reply_to": { "type": "string", "description": "Message ID to reply to (optional)" }
                },
                "required": ["room", "content"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                ..Default::default()
            },
            tags: vec!["comms".into(), "room".into(), "post".into()],
        })
    }
}

impl AgentTool for RoomPostTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let room = args
            .get("room")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'room' required".into()))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'content' required".into()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "room_post: would post to room '{}' from agent {}: {}",
                    room, ctx.agent_id, content
                ),
            }],
            metadata: None,
        })
    }
}

// --- RoomReadTool ---

#[derive(Default)]
pub struct RoomReadTool {
    meta: OnceLock<ToolMetadata>,
}

impl RoomReadTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "room_read".into(),
            description: "Read messages from a chat room".into(),
            category: ToolCategory::AgentComms,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "room": { "type": "string", "description": "Room name or ID" },
                    "limit": { "type": "integer", "description": "Max messages to return (default: 10)" }
                },
                "required": ["room"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                idempotent: true,
                ..Default::default()
            },
            tags: vec!["comms".into(), "room".into(), "read".into()],
        })
    }
}

impl AgentTool for RoomReadTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let room = args
            .get("room")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'room' required".into()))?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "room_read: would read {} messages from room '{}' for agent {}",
                    limit, room, ctx.agent_id
                ),
            }],
            metadata: None,
        })
    }
}

// --- RoomCreateTool ---

#[derive(Default)]
pub struct RoomCreateTool {
    meta: OnceLock<ToolMetadata>,
}

impl RoomCreateTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "room_create".into(),
            description: "Create a new chat room".into(),
            category: ToolCategory::AgentComms,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Room name (slug)" },
                    "purpose": { "type": "string", "description": "Room purpose description" }
                },
                "required": ["name"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                ..Default::default()
            },
            tags: vec!["comms".into(), "room".into(), "create".into()],
        })
    }
}

impl AgentTool for RoomCreateTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'name' required".into()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "room_create: would create room '{}' for agent {}",
                    name, ctx.agent_id
                ),
            }],
            metadata: None,
        })
    }
}

// --- RoomWaitTool ---

#[derive(Default)]
pub struct RoomWaitTool {
    meta: OnceLock<ToolMetadata>,
}

impl RoomWaitTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "room_wait".into(),
            description: "Wait for a message in a room (blocking, with timeout)".into(),
            category: ToolCategory::AgentComms,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "room": { "type": "string", "description": "Room name or ID" },
                    "timeout_secs": { "type": "integer", "description": "Timeout in seconds (default: 60)" }
                },
                "required": ["room"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 120,
                ..Default::default()
            },
            tags: vec!["comms".into(), "room".into(), "wait".into()],
        })
    }
}

impl AgentTool for RoomWaitTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let room = args
            .get("room")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'room' required".into()))?;
        let timeout = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(60);

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "room_wait: would wait {}s for message in room '{}' for agent {}",
                    timeout, room, ctx.agent_id
                ),
            }],
            metadata: None,
        })
    }
}

// --- TaskCreateTool ---

#[derive(Default)]
pub struct TaskCreateTool {
    meta: OnceLock<ToolMetadata>,
}

impl TaskCreateTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "task_create".into(),
            description: "Create a task in the task management system".into(),
            category: ToolCategory::AgentComms,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Task title" },
                    "description": { "type": "string", "description": "Task description" },
                    "assignee": { "type": "string", "description": "Agent ID to assign to" },
                    "priority": { "type": "string", "enum": ["low", "medium", "high", "critical"], "description": "Task priority" }
                },
                "required": ["title"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                ..Default::default()
            },
            tags: vec!["comms".into(), "task".into(), "create".into()],
        })
    }
}

impl AgentTool for TaskCreateTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'title' required".into()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "task_create: would create task '{}' by agent {}",
                    title, ctx.agent_id
                ),
            }],
            metadata: None,
        })
    }
}

// --- TaskUpdateTool ---

#[derive(Default)]
pub struct TaskUpdateTool {
    meta: OnceLock<ToolMetadata>,
}

impl TaskUpdateTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "task_update".into(),
            description: "Update a task's status or add a note".into(),
            category: ToolCategory::AgentComms,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task ID" },
                    "status": { "type": "string", "enum": ["open", "in-progress", "blocked", "done", "cancelled"], "description": "New status" },
                    "note": { "type": "string", "description": "Note to add to the task" }
                },
                "required": ["task_id"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                ..Default::default()
            },
            tags: vec!["comms".into(), "task".into(), "update".into()],
        })
    }
}

impl AgentTool for TaskUpdateTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'task_id' required".into()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "task_update: would update task '{}' by agent {}",
                    task_id, ctx.agent_id
                ),
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
        }
    }

    #[tokio::test]
    async fn room_post_stub() {
        let tool = RoomPostTool::new();
        let output = tool
            .call(
                json!({"room": "test-room", "content": "hello"}),
                &test_ctx(),
            )
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("room_post"));
            assert!(text.contains("test-room"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn room_read_stub() {
        let tool = RoomReadTool::new();
        let output = tool
            .call(json!({"room": "test-room"}), &test_ctx())
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("room_read"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn task_create_stub() {
        let tool = TaskCreateTool::new();
        let output = tool
            .call(json!({"title": "test task"}), &test_ctx())
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("task_create"));
            assert!(text.contains("test task"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn task_update_stub() {
        let tool = TaskUpdateTool::new();
        let output = tool
            .call(
                json!({"task_id": "task-123", "status": "done"}),
                &test_ctx(),
            )
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("task_update"));
            assert!(text.contains("task-123"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn room_create_stub() {
        let tool = RoomCreateTool::new();
        let output = tool
            .call(json!({"name": "my-room"}), &test_ctx())
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("room_create"));
            assert!(text.contains("my-room"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn room_wait_stub() {
        let tool = RoomWaitTool::new();
        let output = tool
            .call(json!({"room": "wait-room"}), &test_ctx())
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("room_wait"));
            assert!(text.contains("wait-room"));
        } else {
            panic!("expected text content");
        }
    }
}
