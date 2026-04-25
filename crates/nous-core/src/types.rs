use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

// --- Enums ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Decision,
    Convention,
    Bugfix,
    Architecture,
    Fact,
    Observation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    Low,
    #[default]
    Moderate,
    High,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    #[default]
    Moderate,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    Related,
    Supersedes,
    Contradicts,
    DependsOn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategorySource {
    System,
    User,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Fts,
    Semantic,
    Hybrid,
}

macro_rules! impl_display_fromstr {
    ($ty:ty, $( $variant:ident => $str:literal ),+ $(,)?) => {
        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let s = match self {
                    $( Self::$variant => $str, )+
                };
                f.write_str(s)
            }
        }

        impl FromStr for $ty {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $str => Ok(Self::$variant), )+
                    _ => Err(format!("invalid value: {s}")),
                }
            }
        }
    };
}

impl_display_fromstr!(MemoryType,
    Decision => "decision",
    Convention => "convention",
    Bugfix => "bugfix",
    Architecture => "architecture",
    Fact => "fact",
    Observation => "observation",
);

impl_display_fromstr!(Importance,
    Low => "low",
    Moderate => "moderate",
    High => "high",
);

impl_display_fromstr!(Confidence,
    Low => "low",
    Moderate => "moderate",
    High => "high",
);

impl_display_fromstr!(RelationType,
    Related => "related",
    Supersedes => "supersedes",
    Contradicts => "contradicts",
    DependsOn => "depends_on",
);

impl_display_fromstr!(CategorySource,
    System => "system",
    User => "user",
    Agent => "agent",
);

impl_display_fromstr!(SearchMode,
    Fts => "fts",
    Semantic => "semantic",
    Hybrid => "hybrid",
);

// --- Structs ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub title: String,
    pub content: String,
    pub memory_type: MemoryType,
    pub source: Option<String>,
    pub importance: Importance,
    pub confidence: Confidence,
    pub workspace_id: Option<i64>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_model: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub archived: bool,
    pub category_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryTag {
    pub memory_id: String,
    pub tag_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: i64,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: RelationType,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: i64,
    pub path: String,
    pub name: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub source: CategorySource,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessLogEntry {
    pub id: i64,
    pub memory_id: String,
    pub accessed_at: String,
    pub access_type: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: i64,
    pub name: String,
    pub dimensions: i64,
    pub max_tokens: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryChunk {
    pub id: String,
    pub memory_id: String,
    pub chunk_index: i64,
    pub content: String,
    pub token_count: i64,
    pub model_id: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMemory {
    pub title: String,
    pub content: String,
    pub memory_type: MemoryType,
    pub source: Option<String>,
    #[serde(default)]
    pub importance: Importance,
    #[serde(default)]
    pub confidence: Confidence,
    #[serde(default)]
    pub tags: Vec<String>,
    pub workspace_path: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_model: Option<String>,
    pub valid_from: Option<String>,
    pub category_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPatch {
    pub title: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub importance: Option<Importance>,
    pub confidence: Option<Confidence>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWithRelations {
    pub memory: Memory,
    pub tags: Vec<String>,
    pub relationships: Vec<Relationship>,
    pub category: Option<Category>,
    pub access_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub memory: Memory,
    pub tags: Vec<String>,
    pub rank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchFilters {
    pub memory_type: Option<MemoryType>,
    pub category_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub importance: Option<Importance>,
    pub confidence: Option<Confidence>,
    pub tags: Option<Vec<String>>,
    pub archived: Option<bool>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub valid_only: Option<bool>,
    pub limit: Option<usize>,
}

impl Default for SearchFilters {
    fn default() -> Self {
        Self {
            memory_type: None,
            category_id: None,
            workspace_id: None,
            importance: None,
            confidence: None,
            tags: None,
            archived: Some(false),
            since: None,
            until: None,
            valid_only: None,
            limit: Some(20),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEntry {
    pub id: String,
    pub title: String,
    pub content: Option<String>,
    pub memory_type: MemoryType,
    pub importance: Importance,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryTree {
    pub category: Category,
    pub children: Vec<CategoryTree>,
}
