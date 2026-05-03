use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use super::{AgentTool, AgentToolDyn, ToolCategory, ToolMetadata};

pub type DynTool = Arc<dyn AgentToolDyn>;

pub struct ToolRegistry {
    tools: RwLock<HashMap<String, DynTool>>,
    categories: RwLock<HashMap<ToolCategory, Vec<String>>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            categories: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register(&self, tool: impl AgentTool) {
        let meta = tool.metadata().clone();
        let name = meta.name.clone();
        let category = meta.category;
        let dyn_tool: DynTool = Arc::new(tool);

        let mut tools = self.tools.write().await;
        tools.insert(name.clone(), dyn_tool);

        let mut cats = self.categories.write().await;
        cats.entry(category).or_default().push(name);
    }

    pub async fn get(&self, name: &str) -> Option<DynTool> {
        self.tools.read().await.get(name).cloned()
    }

    pub async fn list(&self) -> Vec<ToolMetadata> {
        self.tools
            .read()
            .await
            .values()
            .map(|t| t.metadata_dyn().clone())
            .collect()
    }

    pub async fn list_by_category(&self, category: ToolCategory) -> Vec<ToolMetadata> {
        let cats = self.categories.read().await;
        let tools = self.tools.read().await;
        cats.get(&category)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|n| tools.get(n))
                    .map(|t| t.metadata_dyn().clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub async fn deregister(&self, name: &str) -> bool {
        let mut tools = self.tools.write().await;
        if let Some(tool) = tools.remove(name) {
            let category = tool.metadata_dyn().category;
            let mut cats = self.categories.write().await;
            if let Some(names) = cats.get_mut(&category) {
                names.retain(|n| n != name);
            }
            true
        } else {
            false
        }
    }

    pub async fn count(&self) -> usize {
        self.tools.read().await.len()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::tools::{
        ExecutionPolicy, ToolCategory, ToolContext, ToolError, ToolMetadata, ToolOutput,
        ToolPermissions,
    };

    struct FakeTool {
        meta: ToolMetadata,
    }

    impl FakeTool {
        fn new(name: &str, category: ToolCategory) -> Self {
            Self {
                meta: ToolMetadata {
                    name: name.into(),
                    description: format!("Fake {name} tool"),
                    category,
                    version: "0.1.0".into(),
                    input_schema: json!({"type": "object"}),
                    output_schema: None,
                    permissions: ToolPermissions::default(),
                    execution_policy: ExecutionPolicy::default(),
                    tags: vec![],
                },
            }
        }
    }

    impl crate::tools::AgentTool for FakeTool {
        fn metadata(&self) -> &ToolMetadata {
            &self.meta
        }

        async fn call(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput {
                content: vec![],
                metadata: None,
            })
        }
    }

    #[tokio::test]
    async fn register_and_get() {
        let registry = ToolRegistry::new();
        registry
            .register(FakeTool::new("test_tool", ToolCategory::Custom))
            .await;

        let tool = registry.get("test_tool").await;
        assert!(tool.is_some());
        assert_eq!(tool.unwrap().metadata_dyn().name, "test_tool");
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let registry = ToolRegistry::new();
        assert!(registry.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn list_all() {
        let registry = ToolRegistry::new();
        registry
            .register(FakeTool::new("tool_a", ToolCategory::Shell))
            .await;
        registry
            .register(FakeTool::new("tool_b", ToolCategory::Http))
            .await;

        let all = registry.list().await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn list_by_category() {
        let registry = ToolRegistry::new();
        registry
            .register(FakeTool::new("fs_read", ToolCategory::FileSystem))
            .await;
        registry
            .register(FakeTool::new("fs_write", ToolCategory::FileSystem))
            .await;
        registry
            .register(FakeTool::new("shell_exec", ToolCategory::Shell))
            .await;

        let fs_tools = registry.list_by_category(ToolCategory::FileSystem).await;
        assert_eq!(fs_tools.len(), 2);

        let shell_tools = registry.list_by_category(ToolCategory::Shell).await;
        assert_eq!(shell_tools.len(), 1);

        let http_tools = registry.list_by_category(ToolCategory::Http).await;
        assert!(http_tools.is_empty());
    }

    #[tokio::test]
    async fn deregister() {
        let registry = ToolRegistry::new();
        registry
            .register(FakeTool::new("removable", ToolCategory::Custom))
            .await;
        assert_eq!(registry.count().await, 1);

        assert!(registry.deregister("removable").await);
        assert_eq!(registry.count().await, 0);
        assert!(registry.get("removable").await.is_none());
    }

    #[tokio::test]
    async fn deregister_nonexistent_returns_false() {
        let registry = ToolRegistry::new();
        assert!(!registry.deregister("ghost").await);
    }

    #[tokio::test]
    async fn count() {
        let registry = ToolRegistry::new();
        assert_eq!(registry.count().await, 0);

        registry
            .register(FakeTool::new("a", ToolCategory::Memory))
            .await;
        registry
            .register(FakeTool::new("b", ToolCategory::Memory))
            .await;
        assert_eq!(registry.count().await, 2);
    }
}
