pub mod code;
pub mod comms;
pub mod filesystem;
pub mod http;
pub mod memory;
pub mod shell;

pub use code::{CodeGlobTool, CodeGrepTool, CodeSymbolsTool};
pub use comms::{
    RoomCreateTool, RoomPostTool, RoomReadTool, RoomWaitTool, TaskCreateTool, TaskUpdateTool,
};
pub use filesystem::{
    FsDeleteTool, FsEditTool, FsListTool, FsMkdirTool, FsReadTool, FsSearchTool, FsStatTool,
    FsWriteTool,
};
pub use http::{HttpFetchTool, HttpRequestTool};
pub use memory::{
    MemoryGetContextTool, MemoryRelateTool, MemorySaveTool, MemorySearchHybridTool,
    MemorySearchTool,
};
pub use shell::{ShellExecBackgroundTool, ShellExecTool, ShellKillTool, ShellReadOutputTool};

use super::registry::ToolRegistry;

pub async fn register_builtin_tools(registry: &ToolRegistry) {
    // Filesystem (8)
    registry.register(FsReadTool::new()).await;
    registry.register(FsWriteTool::new()).await;
    registry.register(FsEditTool::new()).await;
    registry.register(FsListTool::new()).await;
    registry.register(FsSearchTool::new()).await;
    registry.register(FsStatTool::new()).await;
    registry.register(FsMkdirTool::new()).await;
    registry.register(FsDeleteTool::new()).await;

    // Shell (4)
    registry.register(ShellExecTool::new()).await;
    registry.register(ShellExecBackgroundTool::new()).await;
    registry.register(ShellReadOutputTool::new()).await;
    registry.register(ShellKillTool::new()).await;

    // HTTP (2)
    registry.register(HttpRequestTool::new()).await;
    registry.register(HttpFetchTool::new()).await;

    // Memory (5)
    registry.register(MemorySaveTool::new()).await;
    registry.register(MemorySearchTool::new()).await;
    registry.register(MemorySearchHybridTool::new()).await;
    registry.register(MemoryGetContextTool::new()).await;
    registry.register(MemoryRelateTool::new()).await;

    // Agent comms (6)
    registry.register(RoomPostTool::new()).await;
    registry.register(RoomReadTool::new()).await;
    registry.register(RoomCreateTool::new()).await;
    registry.register(RoomWaitTool::new()).await;
    registry.register(TaskCreateTool::new()).await;
    registry.register(TaskUpdateTool::new()).await;

    // Code analysis (3)
    registry.register(CodeGrepTool::new()).await;
    registry.register(CodeGlobTool::new()).await;
    registry.register(CodeSymbolsTool::new()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_builtin_tools_registers_28() {
        let registry = ToolRegistry::new();
        register_builtin_tools(&registry).await;
        assert_eq!(registry.count().await, 28);
    }

    #[tokio::test]
    async fn all_tool_names_unique() {
        let registry = ToolRegistry::new();
        register_builtin_tools(&registry).await;

        let tools = registry.list().await;
        let mut names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        let original_count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), original_count, "duplicate tool names detected");
    }

    #[tokio::test]
    async fn categories_are_populated() {
        use crate::tools::ToolCategory;

        let registry = ToolRegistry::new();
        register_builtin_tools(&registry).await;

        let fs = registry.list_by_category(ToolCategory::FileSystem).await;
        assert_eq!(fs.len(), 8, "expected 8 filesystem tools");

        let shell = registry.list_by_category(ToolCategory::Shell).await;
        assert_eq!(shell.len(), 4, "expected 4 shell tools");

        let http = registry.list_by_category(ToolCategory::Http).await;
        assert_eq!(http.len(), 2, "expected 2 http tools");

        let memory = registry.list_by_category(ToolCategory::Memory).await;
        assert_eq!(memory.len(), 5, "expected 5 memory tools");

        let comms = registry.list_by_category(ToolCategory::AgentComms).await;
        assert_eq!(comms.len(), 6, "expected 6 agent comms tools");

        let code = registry.list_by_category(ToolCategory::CodeAnalysis).await;
        assert_eq!(code.len(), 3, "expected 3 code analysis tools");
    }
}
