use std::str::FromStr;

use nous_core::db::MemoryDb;
use nous_core::types::{
    Confidence, Importance, MemoryPatch, MemoryType, MemoryWithRelations, NewMemory, RelationType,
    SearchFilters, SearchMode,
};
use nous_shared::NousError;

use nous_shared::ids::MemoryId;

use super::{OutputFormat, print_csv, print_json};
use crate::config::Config;

fn parse_memory_id(id: &str) -> Result<MemoryId, Box<dyn std::error::Error>> {
    id.parse::<MemoryId>().map_err(|e| {
        Box::new(NousError::Validation(format!("invalid id: {e}"))) as Box<dyn std::error::Error>
    })
}

fn open_db(config: &Config) -> Result<MemoryDb, Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    Ok(MemoryDb::open(
        &config.memory.db_path,
        db_key.as_deref(),
        config.embedding.dimensions,
    )?)
}

fn parse_field<T: FromStr>(value: &str, field: &str) -> Result<T, Box<dyn std::error::Error>>
where
    T::Err: std::fmt::Display,
{
    T::from_str(value)
        .map_err(|e| Box::new(NousError::Validation(format!("invalid {field}: {e}"))) as _)
}

fn print_recall_human(r: &MemoryWithRelations) {
    let m = &r.memory;
    println!("ID: {}", m.id);
    println!("Title: {}", m.title);
    println!("Type: {}", m.memory_type);
    println!("Importance: {}", m.importance);
    println!("Confidence: {}", m.confidence);
    if let Some(ref ws) = m.workspace_id {
        println!("Workspace ID: {ws}");
    }
    if !r.tags.is_empty() {
        println!("Tags: {}", r.tags.join(", "));
    }
    println!("Created: {}", m.created_at);
    println!("Updated: {}", m.updated_at);
    if m.archived {
        println!("Archived: true");
    }
    println!();
    println!("Content:");
    println!("{}", m.content);

    if !r.relationships.is_empty() {
        println!();
        println!("Relations:");
        for rel in &r.relationships {
            if rel.source_id == m.id {
                println!("  {} → {}", rel.relation_type, rel.target_id);
            } else {
                println!("  {} ← {}", rel.relation_type, rel.source_id);
            }
        }
    }
}

fn recall_to_json(r: &MemoryWithRelations) -> serde_json::Value {
    serde_json::json!({
        "id": r.memory.id,
        "title": r.memory.title,
        "content": r.memory.content,
        "memory_type": r.memory.memory_type,
        "source": r.memory.source,
        "importance": r.memory.importance,
        "confidence": r.memory.confidence,
        "workspace_id": r.memory.workspace_id,
        "session_id": r.memory.session_id,
        "trace_id": r.memory.trace_id,
        "agent_id": r.memory.agent_id,
        "agent_model": r.memory.agent_model,
        "valid_from": r.memory.valid_from,
        "valid_until": r.memory.valid_until,
        "archived": r.memory.archived,
        "category_id": r.memory.category_id,
        "created_at": r.memory.created_at,
        "updated_at": r.memory.updated_at,
        "tags": r.tags,
        "relationships": r.relationships.iter().map(|rel| serde_json::json!({
            "source_id": rel.source_id,
            "target_id": rel.target_id,
            "relation_type": rel.relation_type,
        })).collect::<Vec<_>>(),
        "category": r.category.as_ref().map(|c| serde_json::json!({
            "id": c.id,
            "name": c.name,
        })),
        "access_count": r.access_count,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_store(
    config: &Config,
    title: &str,
    content: &str,
    memory_type: &str,
    source: Option<&str>,
    importance: Option<&str>,
    confidence: Option<&str>,
    tags: &[String],
    workspace: Option<&str>,
    session_id: Option<&str>,
    trace_id: Option<&str>,
    agent_id: Option<&str>,
    agent_model: Option<&str>,
    valid_from: Option<&str>,
    category_id: Option<i64>,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let mt: MemoryType = parse_field(memory_type, "type")?;
    let imp: Importance = match importance {
        Some(v) => parse_field(v, "importance")?,
        None => Importance::default(),
    };
    let conf: Confidence = match confidence {
        Some(v) => parse_field(v, "confidence")?,
        None => Confidence::default(),
    };

    let workspace_path = match workspace {
        Some(w) => Some(w.to_string()),
        None => std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned()),
    };

    let new_memory = NewMemory {
        title: title.to_string(),
        content: content.to_string(),
        memory_type: mt,
        source: source.map(|s| s.to_string()).or(Some("cli".to_string())),
        importance: imp,
        confidence: conf,
        tags: tags.to_vec(),
        workspace_path,
        session_id: session_id.map(String::from),
        trace_id: trace_id.map(String::from),
        agent_id: agent_id.map(String::from),
        agent_model: agent_model.map(String::from),
        valid_from: valid_from.map(String::from),
        category_id,
    };

    let db = open_db(config)?;
    let id = db.store(&new_memory)?;

    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({
                "memory_id": id.to_string(),
                "created_at": chrono_now(),
            }))?;
        }
        _ => {
            println!("Memory stored: {id}");
        }
    }
    Ok(())
}

pub fn run_recall(
    config: &Config,
    id: &str,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let memory_id = parse_memory_id(id)?;

    let db = open_db(config)?;
    let result = db.recall(&memory_id)?;

    match result {
        Some(r) => match format {
            OutputFormat::Json => {
                print_json(&recall_to_json(&r))?;
            }
            _ => {
                print_recall_human(&r);
            }
        },
        None => {
            return Err(Box::new(NousError::NotFound(format!(
                "memory not found: {id}"
            ))));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn run_update(
    config: &Config,
    id: &str,
    title: Option<&str>,
    content: Option<&str>,
    importance: Option<&str>,
    confidence: Option<&str>,
    tags: Option<&[String]>,
    valid_until: Option<&str>,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let memory_id = parse_memory_id(id)?;

    let imp = match importance {
        Some(v) => Some(parse_field::<Importance>(v, "importance")?),
        None => None,
    };
    let conf = match confidence {
        Some(v) => Some(parse_field::<Confidence>(v, "confidence")?),
        None => None,
    };

    let patch = MemoryPatch {
        title: title.map(String::from),
        content: content.map(String::from),
        tags: tags.map(|t| t.to_vec()),
        importance: imp,
        confidence: conf,
        valid_until: valid_until.map(String::from),
    };

    let db = open_db(config)?;
    let updated = db.update(&memory_id, &patch)?;

    if !updated {
        return Err(Box::new(NousError::NotFound(format!(
            "memory not found: {id}"
        ))));
    }

    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({"status": "updated", "id": id}))?;
        }
        _ => {
            println!("Memory updated: {id}");
        }
    }
    Ok(())
}

pub fn run_forget(
    config: &Config,
    id: &str,
    hard: bool,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let memory_id = parse_memory_id(id)?;

    let db = open_db(config)?;
    let found = db.forget(&memory_id, hard)?;

    if !found {
        return Err(Box::new(NousError::NotFound(format!(
            "memory not found: {id}"
        ))));
    }

    let action = if hard { "deleted" } else { "archived" };
    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({"status": action, "id": id}))?;
        }
        _ => {
            println!("Memory {action}: {id}");
        }
    }
    Ok(())
}

pub fn run_unarchive(
    config: &Config,
    id: &str,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let memory_id = parse_memory_id(id)?;

    let db = open_db(config)?;
    let found = db.unarchive(&memory_id)?;

    if !found {
        return Err(Box::new(NousError::NotFound(format!(
            "memory not found or not archived: {id}"
        ))));
    }

    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({"status": "restored", "id": id}))?;
        }
        _ => {
            println!("Memory restored: {id}");
        }
    }
    Ok(())
}

pub fn run_relate(
    config: &Config,
    source: &str,
    target: &str,
    relation: &str,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let source_id = parse_memory_id(source)?;
    let target_id = parse_memory_id(target)?;
    let rel_type: RelationType = parse_field(relation, "relation type")?;

    let db = open_db(config)?;
    db.relate(&source_id, &target_id, rel_type)?;

    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({
                "status": "related",
                "source": source,
                "target": target,
                "type": relation,
            }))?;
        }
        _ => {
            println!("Relationship created: {source} → {relation} → {target}");
        }
    }
    Ok(())
}

pub fn run_unrelate(
    config: &Config,
    source: &str,
    target: &str,
    relation: &str,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let source_id = parse_memory_id(source)?;
    let target_id = parse_memory_id(target)?;
    let rel_type: RelationType = parse_field(relation, "relation type")?;

    let db = open_db(config)?;
    let found = db.unrelate(&source_id, &target_id, rel_type)?;

    if !found {
        return Err(Box::new(NousError::NotFound(
            "relationship not found".to_string(),
        )));
    }

    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({
                "status": "removed",
                "source": source,
                "target": target,
                "type": relation,
            }))?;
        }
        _ => {
            println!("Relationship removed: {source} → {relation} → {target}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn run_search(
    config: &Config,
    query: &str,
    mode: &str,
    memory_type: Option<&str>,
    importance: Option<&str>,
    confidence: Option<&str>,
    workspace: Option<&str>,
    tags: Option<&[String]>,
    archived: bool,
    since: Option<&str>,
    until: Option<&str>,
    valid_only: bool,
    limit: usize,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let search_mode: SearchMode = parse_field(mode, "mode")?;

    let mt = match memory_type {
        Some(v) => Some(parse_field::<MemoryType>(v, "type")?),
        None => None,
    };
    let imp = match importance {
        Some(v) => Some(parse_field::<Importance>(v, "importance")?),
        None => None,
    };
    let conf = match confidence {
        Some(v) => Some(parse_field::<Confidence>(v, "confidence")?),
        None => None,
    };

    let db = open_db(config)?;

    let workspace_id = match workspace {
        Some(w) => {
            let id: Option<i64> = db
                .connection()
                .query_row(
                    "SELECT id FROM workspaces WHERE path = ?1",
                    rusqlite::params![w],
                    |row| row.get(0),
                )
                .ok();
            id
        }
        None => None,
    };

    let query_embedding = match search_mode {
        SearchMode::Semantic | SearchMode::Hybrid => {
            vec![0.0f32; config.embedding.dimensions]
        }
        SearchMode::Fts => vec![],
    };

    let filters = SearchFilters {
        memory_type: mt,
        category_id: None,
        workspace_id,
        trace_id: None,
        session_id: None,
        importance: imp,
        confidence: conf,
        tags: tags.map(|t| t.to_vec()),
        archived: Some(archived),
        since: since.map(String::from),
        until: until.map(String::from),
        valid_only: Some(valid_only),
        limit: Some(limit),
    };

    let results = db.search(query, &query_embedding, &filters, search_mode)?;

    match format {
        OutputFormat::Json => {
            let results_json: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "memory_id": r.memory.id,
                        "title": r.memory.title,
                        "memory_type": r.memory.memory_type,
                        "importance": r.memory.importance,
                        "rank": r.rank,
                        "tags": r.tags,
                        "created_at": r.memory.created_at,
                    })
                })
                .collect();
            print_json(&serde_json::json!({
                "query": query,
                "mode": mode,
                "results": results_json,
                "count": results.len(),
                "limit": limit,
            }))?;
        }
        OutputFormat::Csv => {
            let rows: Vec<Vec<String>> = results
                .iter()
                .map(|r| {
                    vec![
                        r.memory.id.clone(),
                        r.memory.title.clone(),
                        r.memory.memory_type.to_string(),
                        r.memory.importance.to_string(),
                        format!("{:.2}", r.rank),
                        r.tags.join(","),
                        r.memory.created_at.clone(),
                    ]
                })
                .collect();
            print_csv(
                &[
                    "memory_id",
                    "title",
                    "memory_type",
                    "importance",
                    "rank",
                    "tags",
                    "created_at",
                ],
                &rows,
            )?;
        }
        OutputFormat::Human => {
            if results.is_empty() {
                println!("No results found.");
            } else {
                for (i, r) in results.iter().enumerate() {
                    println!(
                        "{}. [{}] {} (rank: {:.2})",
                        i + 1,
                        r.memory.id,
                        r.memory.title,
                        r.rank
                    );
                    println!(
                        "   Type: {} | Importance: {} | Created: {}",
                        r.memory.memory_type, r.memory.importance, r.memory.created_at
                    );
                    if !r.tags.is_empty() {
                        println!("   Tags: {}", r.tags.join(", "));
                    }
                    println!();
                }
                println!("Found {} results (limit: {limit})", results.len());
            }
        }
    }
    Ok(())
}

fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    let day_secs = secs % 86400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;

    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let year_days: i64 = if leap { 366 } else { 365 };
        if remaining < year_days {
            break;
        }
        remaining -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days: [i64; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut mo = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md {
            mo = i;
            break;
        }
        remaining -= md;
    }
    let d = remaining + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        mo + 1,
        d,
        h,
        m,
        s
    )
}
