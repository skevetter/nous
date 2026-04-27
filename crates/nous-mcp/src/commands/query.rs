use nous_core::db::MemoryDb;
use nous_shared::NousError;

use super::{OutputFormat, print_csv, print_json, print_table};
use crate::config::Config;

fn open_db(config: &Config) -> Result<MemoryDb, Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    Ok(MemoryDb::open(
        &config.memory.db_path,
        db_key.as_deref(),
        config.embedding.dimensions,
    )?)
}

pub fn run_context(
    config: &Config,
    workspace: &str,
    summary: Option<&str>,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;

    let ws_id: i64 = db
        .connection()
        .query_row(
            "SELECT id FROM workspaces WHERE path = ?1",
            rusqlite::params![workspace],
            |row| row.get(0),
        )
        .map_err(|_| {
            Box::new(NousError::NotFound(format!(
                "workspace not found: {workspace}"
            ))) as Box<dyn std::error::Error>
        })?;

    let entries = db.context(ws_id, summary.is_some())?;

    match format {
        OutputFormat::Json => {
            let entries_json: Vec<serde_json::Value> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "title": e.title,
                        "content": e.content,
                        "memory_type": e.memory_type,
                        "importance": e.importance,
                        "created_at": e.created_at,
                    })
                })
                .collect();
            print_json(&serde_json::json!({
                "workspace": workspace,
                "entries": entries_json,
                "count": entries.len(),
            }))?;
        }
        OutputFormat::Csv => {
            let rows: Vec<Vec<String>> = entries
                .iter()
                .map(|e| {
                    vec![
                        e.id.clone(),
                        e.title.clone(),
                        e.memory_type.to_string(),
                        e.importance.to_string(),
                        e.created_at.clone(),
                    ]
                })
                .collect();
            print_csv(
                &["id", "title", "memory_type", "importance", "created_at"],
                &rows,
            )?;
        }
        OutputFormat::Human => {
            println!("Workspace: {workspace}");
            println!("Memories: {}", entries.len());
            println!();
            for (i, e) in entries.iter().enumerate() {
                println!("{}. [{}] {}", i + 1, e.id, e.title);
                println!(
                    "   Type: {} | Importance: {} | Created: {}",
                    e.memory_type, e.importance, e.created_at
                );
                println!();
            }
        }
    }
    Ok(())
}

fn is_read_only_sql(sql: &str) -> bool {
    let upper = sql.to_uppercase();
    let trimmed = upper.trim_start();

    let first_keyword = trimmed
        .split(|c: char| c.is_whitespace() || c == '(')
        .next()
        .unwrap_or("");

    match first_keyword {
        "SELECT" | "EXPLAIN" => !contains_write_keyword(&upper),
        "WITH" => !contains_write_keyword(&upper),
        "PRAGMA" => !trimmed
            .strip_prefix("PRAGMA")
            .unwrap_or("")
            .trim_start()
            .contains('='),
        _ => false,
    }
}

fn contains_write_keyword(upper_sql: &str) -> bool {
    let write_keywords = [
        "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE", "ATTACH", "DETACH", "REPLACE",
        "REINDEX", "VACUUM",
    ];
    let tokens: Vec<&str> = upper_sql
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .collect();
    for kw in &write_keywords {
        if tokens.iter().any(|t| t == kw) {
            return true;
        }
    }
    false
}

pub fn run_sql(
    config: &Config,
    query: &str,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let trimmed = query.trim();

    if !is_read_only_sql(trimmed) {
        return Err(Box::new(NousError::Validation(
            "only SELECT queries are allowed; write operations (INSERT, UPDATE, DELETE, DROP, ALTER, CREATE) are rejected".to_string(),
        )) as Box<dyn std::error::Error>);
    }

    let db = open_db(config)?;
    let conn = db.connection();
    let mut stmt = conn.prepare(trimmed)?;
    let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let col_count = column_names.len();

    let rows: Vec<Vec<serde_json::Value>> = stmt
        .query_map([], |row| {
            let mut values = Vec::with_capacity(col_count);
            for i in 0..col_count {
                let val = match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => serde_json::Value::Number(n.into()),
                    rusqlite::types::ValueRef::Real(f) => serde_json::json!(f),
                    rusqlite::types::ValueRef::Text(t) => {
                        serde_json::Value::String(String::from_utf8_lossy(t).into_owned())
                    }
                    rusqlite::types::ValueRef::Blob(b) => {
                        serde_json::Value::String(format!("<blob:{} bytes>", b.len()))
                    }
                };
                values.push(val);
            }
            Ok(values)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    match format {
        OutputFormat::Json => {
            let json_rows: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    let mut map = serde_json::Map::new();
                    for (i, col) in column_names.iter().enumerate() {
                        if let Some(val) = row.get(i) {
                            map.insert(col.clone(), val.clone());
                        }
                    }
                    serde_json::Value::Object(map)
                })
                .collect();
            print_json(&serde_json::json!({
                "columns": column_names,
                "rows": json_rows,
            }))?;
        }
        OutputFormat::Csv => {
            let string_rows: Vec<Vec<String>> = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|v| match v {
                            serde_json::Value::Null => String::new(),
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect()
                })
                .collect();
            let headers: Vec<&str> = column_names.iter().map(|s| s.as_str()).collect();
            print_csv(&headers, &string_rows)?;
        }
        OutputFormat::Human => {
            let string_rows: Vec<Vec<String>> = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|v| match v {
                            serde_json::Value::Null => "NULL".to_string(),
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect()
                })
                .collect();
            let headers: Vec<&str> = column_names.iter().map(|s| s.as_str()).collect();
            print_table(&headers, &string_rows);
        }
    }
    Ok(())
}

pub fn run_schema(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let schema = MemoryDb::schema_on(db.connection())?;
    println!("{schema}");
    Ok(())
}

pub fn run_workspaces(
    config: &Config,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let workspaces = MemoryDb::workspaces_on(db.connection())?;

    match format {
        OutputFormat::Json => {
            let list: Vec<serde_json::Value> = workspaces
                .iter()
                .map(|(w, count)| {
                    serde_json::json!({
                        "id": w.id,
                        "path": w.path,
                        "memories": count,
                    })
                })
                .collect();
            print_json(&serde_json::json!({"workspaces": list}))?;
        }
        OutputFormat::Csv => {
            let rows: Vec<Vec<String>> = workspaces
                .iter()
                .map(|(w, count)| vec![w.id.to_string(), w.path.clone(), count.to_string()])
                .collect();
            print_csv(&["id", "path", "memories"], &rows)?;
        }
        OutputFormat::Human => {
            let rows: Vec<Vec<String>> = workspaces
                .iter()
                .map(|(w, count)| vec![w.id.to_string(), w.path.clone(), count.to_string()])
                .collect();
            print_table(&["ID", "Path", "Memories"], &rows);
        }
    }
    Ok(())
}

pub fn run_tags(config: &Config, format: &OutputFormat) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let tags = MemoryDb::tags_on(db.connection())?;

    match format {
        OutputFormat::Json => {
            let list: Vec<serde_json::Value> = tags
                .iter()
                .map(|(name, count)| serde_json::json!({"tag": name, "count": count}))
                .collect();
            print_json(&serde_json::json!({"tags": list}))?;
        }
        OutputFormat::Csv => {
            let rows: Vec<Vec<String>> = tags
                .iter()
                .map(|(name, count)| vec![name.clone(), count.to_string()])
                .collect();
            print_csv(&["tag", "count"], &rows)?;
        }
        OutputFormat::Human => {
            let rows: Vec<Vec<String>> = tags
                .iter()
                .map(|(name, count)| vec![name.clone(), count.to_string()])
                .collect();
            print_table(&["Tag", "Count"], &rows);
        }
    }
    Ok(())
}
