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
        let reply_to = args
            .get("reply_to")
            .and_then(|v| v.as_str())
            .map(String::from);

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services
            .post_to_room(crate::tools::PostToRoomParams {
                room: room.to_string(),
                sender_id: ctx.agent_id.clone(),
                content: content.to_string(),
                reply_to,
            })
            .await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
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
        let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services.read_room(room.to_string(), limit).await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
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
        let purpose = args
            .get("purpose")
            .and_then(|v| v.as_str())
            .map(String::from);

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services.create_room(name.to_string(), purpose).await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
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

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services.wait_for_message(room.to_string(), timeout).await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
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
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);
        let assignee = args
            .get("assignee")
            .and_then(|v| v.as_str())
            .map(String::from);
        let priority = args
            .get("priority")
            .and_then(|v| v.as_str())
            .map(String::from);

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services
            .create_task(crate::tools::ToolCreateTaskParams {
                title: title.to_string(),
                description,
                assignee,
                priority,
            })
            .await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
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
        let status = args
            .get("status")
            .and_then(|v| v.as_str())
            .map(String::from);
        let note = args.get("note").and_then(|v| v.as_str()).map(String::from);

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services
            .update_task(task_id.to_string(), status, note)
            .await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
            metadata: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::tools::{NetworkPolicy, ResolvedPermissions, ToolContext, ToolServices};

    struct MockToolServices;

    #[async_trait::async_trait]
    impl ToolServices for MockToolServices {
        async fn save_memory(&self, _params: crate::tools::SaveMemoryParams) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        async fn search_memories(
            &self,
            _params: crate::tools::SearchMemoriesParams,
        ) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        async fn search_memories_hybrid(
            &self,
            _params: crate::tools::SearchMemoriesHybridParams,
        ) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        async fn get_memory_context(
            &self,
            _params: crate::tools::GetMemoryContextParams,
        ) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        async fn relate_memories(
            &self,
            _source_id: String,
            _target_id: String,
            _relation_type: String,
        ) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        async fn update_memory(
            &self,
            _memory_id: String,
            _content: Option<String>,
            _importance: Option<String>,
        ) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        async fn post_to_room(
            &self,
            params: crate::tools::PostToRoomParams,
        ) -> Result<Value, ToolError> {
            let crate::tools::PostToRoomParams { room, sender_id, content, reply_to } = params;
            Ok(json!({
                "id": "msg-001",
                "room": room,
                "sender_id": sender_id,
                "content": content,
                "reply_to": reply_to,
            }))
        }

        async fn read_room(&self, room: String, limit: Option<u32>) -> Result<Value, ToolError> {
            Ok(json!({
                "room": room,
                "limit": limit.unwrap_or(10),
                "messages": [],
            }))
        }

        async fn create_room(
            &self,
            name: String,
            purpose: Option<String>,
        ) -> Result<Value, ToolError> {
            Ok(json!({
                "name": name,
                "purpose": purpose,
            }))
        }

        async fn wait_for_message(
            &self,
            room: String,
            timeout_secs: u64,
        ) -> Result<Value, ToolError> {
            Ok(json!({
                "room": room,
                "timeout_secs": timeout_secs,
                "message": null,
            }))
        }

        async fn create_task(
            &self,
            params: crate::tools::ToolCreateTaskParams,
        ) -> Result<Value, ToolError> {
            let crate::tools::ToolCreateTaskParams { title, description, assignee, priority } =
                params;
            Ok(json!({
                "id": "task-001",
                "title": title,
                "description": description,
                "assignee": assignee,
                "priority": priority,
            }))
        }

        async fn update_task(
            &self,
            task_id: String,
            status: Option<String>,
            note: Option<String>,
        ) -> Result<Value, ToolError> {
            Ok(json!({
                "task_id": task_id,
                "status": status,
                "note": note,
            }))
        }
    }

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
                shell: None,
                network: None,
            },
            services: None,
        }
    }

    fn test_ctx_with_services() -> ToolContext {
        ToolContext {
            services: Some(Arc::new(MockToolServices)),
            ..test_ctx()
        }
    }

    #[tokio::test]
    async fn room_post_no_services_returns_error() {
        let tool = RoomPostTool::new();
        let result = tool
            .call(
                json!({"room": "test-room", "content": "hello"}),
                &test_ctx(),
            )
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no services configured"));
    }

    #[tokio::test]
    async fn room_post_delegates_to_services() {
        let tool = RoomPostTool::new();
        let output = tool
            .call(
                json!({"room": "test-room", "content": "hello"}),
                &test_ctx_with_services(),
            )
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["id"], "msg-001");
            assert_eq!(data["room"], "test-room");
            assert_eq!(data["content"], "hello");
            assert_eq!(data["sender_id"], "test-agent");
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn room_read_delegates_to_services() {
        let tool = RoomReadTool::new();
        let output = tool
            .call(
                json!({"room": "test-room", "limit": 5}),
                &test_ctx_with_services(),
            )
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["room"], "test-room");
            assert_eq!(data["limit"], 5);
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn room_create_delegates_to_services() {
        let tool = RoomCreateTool::new();
        let output = tool
            .call(
                json!({"name": "my-room", "purpose": "testing"}),
                &test_ctx_with_services(),
            )
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["name"], "my-room");
            assert_eq!(data["purpose"], "testing");
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn room_wait_delegates_to_services() {
        let tool = RoomWaitTool::new();
        let output = tool
            .call(
                json!({"room": "wait-room", "timeout_secs": 30}),
                &test_ctx_with_services(),
            )
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["room"], "wait-room");
            assert_eq!(data["timeout_secs"], 30);
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn task_create_delegates_to_services() {
        let tool = TaskCreateTool::new();
        let output = tool
            .call(
                json!({"title": "test task", "priority": "high"}),
                &test_ctx_with_services(),
            )
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["id"], "task-001");
            assert_eq!(data["title"], "test task");
            assert_eq!(data["priority"], "high");
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn task_update_delegates_to_services() {
        let tool = TaskUpdateTool::new();
        let output = tool
            .call(
                json!({"task_id": "task-123", "status": "done", "note": "completed"}),
                &test_ctx_with_services(),
            )
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["task_id"], "task-123");
            assert_eq!(data["status"], "done");
            assert_eq!(data["note"], "completed");
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn task_create_no_services_returns_error() {
        let tool = TaskCreateTool::new();
        let result = tool.call(json!({"title": "test task"}), &test_ctx()).await;
        assert!(result.is_err());
    }
}
