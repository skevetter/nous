use serde::{Deserialize, Serialize};

use crate::error::NousError;

// --- Types ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    Decision,
    Convention,
    Bugfix,
    Architecture,
    Fact,
    Observation,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Convention => "convention",
            Self::Bugfix => "bugfix",
            Self::Architecture => "architecture",
            Self::Fact => "fact",
            Self::Observation => "observation",
        }
    }
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MemoryType {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "decision" => Ok(Self::Decision),
            "convention" => Ok(Self::Convention),
            "bugfix" => Ok(Self::Bugfix),
            "architecture" => Ok(Self::Architecture),
            "fact" => Ok(Self::Fact),
            "observation" => Ok(Self::Observation),
            other => Err(NousError::Validation(format!(
                "invalid memory type: '{other}'. Valid values: decision, convention, bugfix, architecture, fact, observation"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Importance {
    Low,
    Moderate,
    High,
}

impl Importance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
        }
    }
}

impl std::fmt::Display for Importance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Importance {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(Self::Low),
            "moderate" => Ok(Self::Moderate),
            "high" => Ok(Self::High),
            other => Err(NousError::Validation(format!(
                "invalid importance: '{other}'. Valid values: low, moderate, high"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    Supersedes,
    ConflictsWith,
    Related,
    Compatible,
    Scoped,
    NotConflict,
}

impl RelationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Supersedes => "supersedes",
            Self::ConflictsWith => "conflicts_with",
            Self::Related => "related",
            Self::Compatible => "compatible",
            Self::Scoped => "scoped",
            Self::NotConflict => "not_conflict",
        }
    }
}

impl std::fmt::Display for RelationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for RelationType {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "supersedes" => Ok(Self::Supersedes),
            "conflicts_with" => Ok(Self::ConflictsWith),
            "related" => Ok(Self::Related),
            "compatible" => Ok(Self::Compatible),
            "scoped" => Ok(Self::Scoped),
            "not_conflict" => Ok(Self::NotConflict),
            other => Err(NousError::Validation(format!(
                "invalid relation type: '{other}'. Valid values: supersedes, conflicts_with, related, compatible, scoped, not_conflict"
            ))),
        }
    }
}

// --- Domain objects ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub workspace_id: String,
    pub agent_id: Option<String>,
    pub title: String,
    pub content: String,
    pub memory_type: String,
    pub importance: String,
    pub topic_key: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl Memory {
    pub(crate) fn from_model(m: crate::entities::memories::Model) -> Self {
        Self {
            id: m.id,
            workspace_id: m.workspace_id,
            agent_id: m.agent_id,
            title: m.title,
            content: m.content,
            memory_type: m.memory_type,
            importance: m.importance,
            topic_key: m.topic_key,
            valid_from: m.valid_from,
            valid_until: m.valid_until,
            archived: m.archived,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }

    pub(crate) fn from_query_result(row: &sea_orm::QueryResult) -> Result<Self, sea_orm::DbErr> {
        Ok(Self {
            id: row.try_get_by("id")?,
            workspace_id: row.try_get_by("workspace_id")?,
            agent_id: row.try_get_by("agent_id")?,
            title: row.try_get_by("title")?,
            content: row.try_get_by("content")?,
            memory_type: row.try_get_by("memory_type")?,
            importance: row.try_get_by("importance")?,
            topic_key: row.try_get_by("topic_key")?,
            valid_from: row.try_get_by("valid_from")?,
            valid_until: row.try_get_by("valid_until")?,
            archived: row.try_get_by("archived")?,
            created_at: row.try_get_by("created_at")?,
            updated_at: row.try_get_by("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRelation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
    pub created_at: String,
}

impl MemoryRelation {
    pub(crate) fn from_model(m: crate::entities::memory_relations::Model) -> Self {
        Self {
            id: m.id,
            source_id: m.source_id,
            target_id: m.target_id,
            relation_type: m.relation_type,
            created_at: m.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarMemory {
    #[serde(flatten)]
    pub memory: Memory,
    pub score: f32,
}

// --- Request types ---

#[derive(Debug, Clone)]
pub struct SaveMemoryRequest {
    pub workspace_id: Option<String>,
    pub agent_id: Option<String>,
    pub title: String,
    pub content: String,
    pub memory_type: MemoryType,
    pub importance: Option<Importance>,
    pub topic_key: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UpdateMemoryRequest {
    pub id: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub importance: Option<Importance>,
    pub topic_key: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchMemoryRequest {
    pub query: String,
    pub workspace_id: Option<String>,
    pub agent_id: Option<String>,
    pub memory_type: Option<MemoryType>,
    pub importance: Option<Importance>,
    pub include_archived: bool,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct ContextRequest {
    pub workspace_id: Option<String>,
    pub agent_id: Option<String>,
    pub topic_key: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct RelateRequest {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: RelationType,
}

// --- Session types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySession {
    pub id: String,
    pub agent_id: Option<String>,
    pub project: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub summary: Option<String>,
}

impl MemorySession {
    pub(crate) fn from_model(m: crate::entities::memory_sessions::Model) -> Self {
        Self {
            id: m.id,
            agent_id: m.agent_id,
            project: m.project,
            started_at: m.started_at,
            ended_at: m.ended_at,
            summary: m.summary,
        }
    }
}

// --- Project detection ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedProject {
    pub name: String,
    pub project_type: String,
    pub path: String,
}

pub fn detect_current_project(cwd: &str) -> Option<DetectedProject> {
    let path = std::path::Path::new(cwd);

    let markers = [
        ("Cargo.toml", "rust"),
        ("package.json", "node"),
        ("go.mod", "go"),
        ("pyproject.toml", "python"),
        ("setup.py", "python"),
        ("Gemfile", "ruby"),
        ("pom.xml", "java"),
        ("build.gradle", "java"),
        ("mix.exs", "elixir"),
        ("CMakeLists.txt", "cpp"),
    ];

    let mut current = Some(path);
    while let Some(dir) = current {
        for (marker, project_type) in &markers {
            if dir.join(marker).exists() {
                let name = dir
                    .file_name().map_or_else(|| "unknown".into(), |n| n.to_string_lossy().to_string());
                return Some(DetectedProject {
                    name,
                    project_type: String::from(*project_type),
                    path: dir.to_string_lossy().into_owned(),
                });
            }
        }
        current = dir.parent();
    }
    None
}

pub(crate) fn truncate_title(s: &str) -> String {
    let first_line = s.lines().next().unwrap_or(s);
    let char_count = first_line.chars().count();
    if char_count > 80 {
        let end = first_line
            .char_indices()
            .nth(77)
            .map_or(first_line.len(), |(i, _)| i);
        format!("{}...", &first_line[..end])
    } else {
        first_line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_memory_type() {
        assert_eq!(
            "decision".parse::<MemoryType>().unwrap(),
            MemoryType::Decision
        );
        assert_eq!("bugfix".parse::<MemoryType>().unwrap(), MemoryType::Bugfix);
        assert!("invalid".parse::<MemoryType>().is_err());
    }

    #[test]
    fn parse_importance() {
        assert_eq!("high".parse::<Importance>().unwrap(), Importance::High);
        assert_eq!("low".parse::<Importance>().unwrap(), Importance::Low);
        assert!("critical".parse::<Importance>().is_err());
    }

    #[test]
    fn parse_relation_type() {
        assert_eq!(
            "supersedes".parse::<RelationType>().unwrap(),
            RelationType::Supersedes
        );
        assert_eq!(
            "conflicts_with".parse::<RelationType>().unwrap(),
            RelationType::ConflictsWith
        );
        assert!("unknown".parse::<RelationType>().is_err());
    }

    #[test]
    fn detect_current_project_finds_cargo_toml() {
        let project = detect_current_project(env!("CARGO_MANIFEST_DIR"));
        assert!(project.is_some());
        let project = project.unwrap();
        assert_eq!(project.project_type, "rust");
        assert!(!project.name.is_empty());
    }

    #[test]
    fn detect_current_project_returns_none_for_no_markers() {
        let project = detect_current_project("/tmp");
        let _ = project;
    }

    #[test]
    fn truncate_title_ascii_short() {
        assert_eq!(truncate_title("hello world"), "hello world");
    }

    #[test]
    fn truncate_title_empty() {
        assert_eq!(truncate_title(""), "");
    }

    #[test]
    fn truncate_title_ascii_exactly_80() {
        let s = "a".repeat(80);
        assert_eq!(truncate_title(&s), s);
    }

    #[test]
    fn truncate_title_ascii_over_80() {
        let s = "a".repeat(100);
        let expected = format!("{}...", "a".repeat(77));
        assert_eq!(truncate_title(&s), expected);
    }

    #[test]
    fn truncate_title_emoji_short() {
        assert_eq!(truncate_title("🦀🎉🚀"), "🦀🎉🚀");
    }

    #[test]
    fn truncate_title_emoji_over_80() {
        let s = "🦀".repeat(81);
        let expected = format!("{}...", "🦀".repeat(77));
        assert_eq!(truncate_title(&s), expected);
    }

    #[test]
    fn truncate_title_cjk_over_80() {
        let s = "漢".repeat(81);
        let expected = format!("{}...", "漢".repeat(77));
        assert_eq!(truncate_title(&s), expected);
    }

    #[test]
    fn truncate_title_mixed_multibyte() {
        let s = format!("{}{}{}", "a".repeat(40), "🦀".repeat(20), "漢".repeat(21));
        let result = truncate_title(&s);
        assert!(result.ends_with("..."));
        let without_dots = &result[..result.len() - 3];
        assert_eq!(without_dots.chars().count(), 77);
    }

    #[test]
    fn truncate_title_multiline() {
        assert_eq!(truncate_title("first line\nsecond line"), "first line");
    }
}
