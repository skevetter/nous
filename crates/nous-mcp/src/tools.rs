#![allow(dead_code)]

use rmcp::schemars;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryStoreParams {
    pub title: String,
    pub content: String,
    pub memory_type: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source: Option<String>,
    pub importance: Option<String>,
    pub confidence: Option<String>,
    pub workspace_path: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_model: Option<String>,
    pub valid_from: Option<String>,
    pub category_id: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryRecallParams {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemorySearchParams {
    pub query: String,
    pub mode: Option<String>,
    pub memory_type: Option<String>,
    pub category_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub importance: Option<String>,
    pub confidence: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    pub archived: Option<bool>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub valid_only: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryContextParams {
    pub workspace_path: String,
    #[serde(default)]
    pub summary: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryForgetParams {
    pub id: String,
    #[serde(default)]
    pub hard: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryUnarchiveParams {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryUpdateParams {
    pub id: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub importance: Option<String>,
    pub confidence: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryRelateParams {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryUnrelateParams {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryCategorySuggestParams {
    pub memory_id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryWorkspacesParams {
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryTagsParams {
    pub prefix: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemorySqlParams {
    pub query: String,
}
