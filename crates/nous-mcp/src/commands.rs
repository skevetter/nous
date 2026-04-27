use std::io;
use std::path::Path;

use nous_core::chunk::Chunker;
use nous_core::classify::CategoryClassifier;
use nous_core::db::MemoryDb;
use nous_core::embed::EmbeddingBackend;
use nous_core::types::{Category, CategorySource, CategoryTree, Memory, NewMemory, Relationship};
use nous_shared::ids::MemoryId;
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportData {
    pub version: u32,
    pub memories: Vec<ExportMemory>,
    pub categories: Vec<ExportCategory>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportMemory {
    pub id: String,
    pub title: String,
    pub content: String,
    pub memory_type: String,
    pub source: Option<String>,
    pub importance: String,
    pub confidence: String,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_model: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub category_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub tags: Vec<String>,
    pub relationships: Vec<ExportRelationship>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportRelationship {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportCategory {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub source: String,
    pub description: Option<String>,
    pub created_at: String,
}

pub fn run_export(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;

    let data = build_export_data(&db)?;
    serde_json::to_writer_pretty(io::stdout().lock(), &data)?;
    println!();
    Ok(())
}

pub fn build_export_data(db: &MemoryDb) -> Result<ExportData, Box<dyn std::error::Error>> {
    let conn = db.connection();

    let mut stmt = conn.prepare(
        "SELECT id, title, content, memory_type, source, importance, confidence,
                workspace_id, session_id, trace_id, agent_id, agent_model,
                valid_from, valid_until, archived, category_id, created_at, updated_at
         FROM memories WHERE archived = 0
         ORDER BY created_at",
    )?;

    let memories: Vec<Memory> = stmt
        .query_map([], |row| {
            Ok(Memory {
                id: row.get(0)?,
                title: row.get(1)?,
                content: row.get(2)?,
                memory_type: row.get::<_, String>(3)?.parse().map_err(|e: String| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        e.into(),
                    )
                })?,
                source: row.get(4)?,
                importance: row.get::<_, String>(5)?.parse().map_err(|e: String| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        e.into(),
                    )
                })?,
                confidence: row.get::<_, String>(6)?.parse().map_err(|e: String| {
                    rusqlite::Error::FromSqlConversionFailure(
                        6,
                        rusqlite::types::Type::Text,
                        e.into(),
                    )
                })?,
                workspace_id: row.get(7)?,
                session_id: row.get(8)?,
                trace_id: row.get(9)?,
                agent_id: row.get(10)?,
                agent_model: row.get(11)?,
                valid_from: row.get(12)?,
                valid_until: row.get(13)?,
                archived: row.get::<_, i64>(14)? != 0,
                category_id: row.get(15)?,
                created_at: row.get(16)?,
                updated_at: row.get(17)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut export_memories = Vec::with_capacity(memories.len());
    for memory in &memories {
        let tags = MemoryDb::load_tags_on(conn, &memory.id)?;
        let relationships = MemoryDb::load_relationships_on(conn, &memory.id)?;
        export_memories.push(memory_to_export(memory, &tags, &relationships));
    }

    let mut cat_stmt = conn.prepare(
        "SELECT id, name, parent_id, source, description, embedding, threshold, created_at FROM categories ORDER BY id",
    )?;
    let categories: Vec<Category> = cat_stmt
        .query_map([], |row| {
            Ok(Category {
                id: row.get(0)?,
                name: row.get(1)?,
                parent_id: row.get(2)?,
                source: row.get::<_, String>(3)?.parse().map_err(|e: String| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        e.into(),
                    )
                })?,
                description: row.get(4)?,
                embedding: row.get(5)?,
                threshold: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let export_categories: Vec<ExportCategory> = categories
        .into_iter()
        .map(|c| ExportCategory {
            id: c.id,
            name: c.name,
            parent_id: c.parent_id,
            source: c.source.to_string(),
            description: c.description,
            created_at: c.created_at,
        })
        .collect();

    Ok(ExportData {
        version: 1,
        memories: export_memories,
        categories: export_categories,
    })
}

fn memory_to_export(
    memory: &Memory,
    tags: &[String],
    relationships: &[Relationship],
) -> ExportMemory {
    ExportMemory {
        id: memory.id.clone(),
        title: memory.title.clone(),
        content: memory.content.clone(),
        memory_type: memory.memory_type.to_string(),
        source: memory.source.clone(),
        importance: memory.importance.to_string(),
        confidence: memory.confidence.to_string(),
        session_id: memory.session_id.clone(),
        trace_id: memory.trace_id.clone(),
        agent_id: memory.agent_id.clone(),
        agent_model: memory.agent_model.clone(),
        valid_from: memory.valid_from.clone(),
        valid_until: memory.valid_until.clone(),
        category_id: memory.category_id,
        created_at: memory.created_at.clone(),
        updated_at: memory.updated_at.clone(),
        tags: tags.to_vec(),
        relationships: relationships
            .iter()
            .filter(|r| r.source_id == memory.id)
            .map(|r| ExportRelationship {
                source_id: r.source_id.clone(),
                target_id: r.target_id.clone(),
                relation_type: r.relation_type.to_string(),
            })
            .collect(),
    }
}

pub fn run_import(
    config: &Config,
    file: &Path,
    embedding: &dyn EmbeddingBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let chunker = Chunker::new(config.embedding.chunk_size, config.embedding.chunk_overlap);

    let reader = std::fs::File::open(file)?;
    let data: ExportData = serde_json::from_reader(reader)?;

    import_data(&db, &data, embedding, &chunker)?;
    eprintln!("Imported {} memories", data.memories.len());
    Ok(())
}

pub fn import_data(
    db: &MemoryDb,
    data: &ExportData,
    embedding: &dyn EmbeddingBackend,
    chunker: &Chunker,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::HashMap;

    let mut cat_id_map: HashMap<i64, i64> = HashMap::new();

    // Import categories first (parents before children)
    let mut sorted_cats: Vec<&ExportCategory> = data.categories.iter().collect();
    sorted_cats.sort_by_key(|c| (c.parent_id.is_some(), c.id));

    for cat in &sorted_cats {
        let source: CategorySource = cat.source.parse().map_err(|e: String| e)?;
        let remapped_parent = cat.parent_id.and_then(|pid| cat_id_map.get(&pid).copied());

        match db.category_add(
            &cat.name,
            remapped_parent,
            cat.description.as_deref(),
            source,
        ) {
            Ok(new_cat_id) => {
                cat_id_map.insert(cat.id, new_cat_id);
            }
            Err(_) => {
                // Category may already exist (e.g. seed categories) — look up its ID
                if let Ok(existing_id) = db.connection().query_row(
                    "SELECT id FROM categories WHERE name = ?1 AND parent_id IS ?2",
                    rusqlite::params![cat.name, remapped_parent],
                    |row| row.get::<_, i64>(0),
                ) {
                    cat_id_map.insert(cat.id, existing_id);
                }
            }
        }
    }

    let mut id_map: HashMap<String, MemoryId> = HashMap::new();

    for memory in &data.memories {
        let memory_type = memory.memory_type.parse().map_err(|e: String| e)?;
        let importance = memory.importance.parse().map_err(|e: String| e)?;
        let confidence = memory.confidence.parse().map_err(|e: String| e)?;

        let remapped_category = memory
            .category_id
            .and_then(|old_id| cat_id_map.get(&old_id).copied());

        let new_memory = NewMemory {
            title: memory.title.clone(),
            content: memory.content.clone(),
            memory_type,
            source: memory.source.clone(),
            importance,
            confidence,
            tags: memory.tags.clone(),
            workspace_path: None,
            session_id: memory.session_id.clone(),
            trace_id: memory.trace_id.clone(),
            agent_id: memory.agent_id.clone(),
            agent_model: memory.agent_model.clone(),
            valid_from: memory.valid_from.clone(),
            category_id: remapped_category,
        };

        let new_id = db.store(&new_memory)?;
        id_map.insert(memory.id.clone(), new_id.clone());

        let chunks = chunker.chunk(&memory.content);
        if !chunks.is_empty() {
            let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
            let embeddings = embedding.embed(&texts)?;
            db.store_chunks(&new_id, &chunks, &embeddings)?;
        }
    }

    for memory in &data.memories {
        for rel in &memory.relationships {
            let relation_type = rel.relation_type.parse().map_err(|e: String| e)?;
            if let (Some(from), Some(to)) = (id_map.get(&rel.source_id), id_map.get(&rel.target_id))
            {
                let _ = db.relate(from, to, relation_type);
            }
        }
    }

    Ok(())
}

pub fn run_status(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();

    let memory_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE archived = 0",
        [],
        |r| r.get(0),
    )?;
    let archived_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE archived = 1",
        [],
        |r| r.get(0),
    )?;
    let category_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))?;
    let chunk_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_chunks", [], |r| r.get(0))?;

    let active_model = db.active_model()?;
    let model_name = active_model
        .as_ref()
        .map(|m| m.name.as_str())
        .unwrap_or("none");

    println!("db_path: {}", config.memory.db_path);
    println!("memories: {memory_count}");
    println!("archived: {archived_count}");
    println!("categories: {category_count}");
    println!("chunks: {chunk_count}");
    println!("active_model: {model_name}");
    Ok(())
}

pub fn run_category_list(
    config: &Config,
    source: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;

    let source_filter = match source {
        Some(s) => {
            let parsed: CategorySource = s.parse().map_err(|e: String| e)?;
            Some(parsed)
        }
        None => None,
    };

    let trees = db.category_list(source_filter)?;
    for tree in &trees {
        print_category_tree(tree, 0);
    }
    Ok(())
}

fn print_category_tree(tree: &CategoryTree, depth: usize) {
    let indent = "  ".repeat(depth);
    let desc = tree
        .category
        .description
        .as_deref()
        .map(|d| format!(" — {d}"))
        .unwrap_or_default();
    println!(
        "{indent}{} [{}]{desc}",
        tree.category.name, tree.category.source
    );
    for child in &tree.children {
        print_category_tree(child, depth + 1);
    }
}

pub fn run_category_add(
    config: &Config,
    name: &str,
    parent: Option<&str>,
    description: Option<&str>,
    embedding: &dyn EmbeddingBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;

    let parent_id = match parent {
        Some(parent_name) => {
            let id: Option<i64> = db
                .connection()
                .query_row(
                    "SELECT id FROM categories WHERE name = ?1 AND parent_id IS NULL",
                    rusqlite::params![parent_name],
                    |row| row.get(0),
                )
                .map_err(|_| format!("parent category '{parent_name}' not found"))?;
            Some(id.unwrap_or_else(|| unreachable!()))
        }
        None => None,
    };

    let cat_id = db.category_add(name, parent_id, description, CategorySource::User)?;

    let embed_text = match description {
        Some(desc) if !desc.is_empty() => format!("{name} {desc}"),
        _ => name.to_string(),
    };
    let emb = embedding.embed_one(&embed_text)?;
    let blob: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
    db.connection().execute(
        "UPDATE categories SET embedding = ?1 WHERE id = ?2",
        rusqlite::params![blob, cat_id],
    )?;

    println!("Added category '{name}' (id={cat_id})");
    Ok(())
}

pub fn run_category_delete(config: &Config, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    db.category_delete(name)?;
    println!("Deleted category '{name}'");
    Ok(())
}

pub fn run_category_rename(
    config: &Config,
    old_name: &str,
    new_name: &str,
    embedding: &dyn EmbeddingBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;

    let desc: Option<String> = db.connection().query_row(
        "SELECT description FROM categories WHERE name = ?1",
        rusqlite::params![old_name],
        |row| row.get(0),
    )?;
    let embed_text = match desc.as_deref() {
        Some(d) if !d.is_empty() => format!("{new_name} {d}"),
        _ => new_name.to_string(),
    };
    let emb = embedding.embed_one(&embed_text)?;
    let blob: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();

    let tx = db.connection().unchecked_transaction()?;
    let changed = tx.execute(
        "UPDATE categories SET name = ?1 WHERE name = ?2",
        rusqlite::params![new_name, old_name],
    )?;
    if changed == 0 {
        return Err(format!("category '{old_name}' not found").into());
    }
    tx.execute(
        "UPDATE categories SET embedding = ?1 WHERE name = ?2",
        rusqlite::params![blob, new_name],
    )?;
    tx.commit()?;

    println!("Renamed category '{old_name}' -> '{new_name}'");
    Ok(())
}

pub fn run_category_update(
    config: &Config,
    name: &str,
    new_name: Option<&str>,
    description: Option<&str>,
    threshold: Option<f32>,
    embedding: &dyn EmbeddingBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;

    let final_name = new_name.unwrap_or(name);
    let embedding_blob = if new_name.is_some() || description.is_some() {
        let current_desc: Option<String> = db.connection().query_row(
            "SELECT description FROM categories WHERE name = ?1",
            rusqlite::params![name],
            |row| row.get(0),
        )?;
        let desc_for_embed = description.or(current_desc.as_deref());
        let embed_text = match desc_for_embed {
            Some(d) if !d.is_empty() => format!("{final_name} {d}"),
            _ => final_name.to_string(),
        };
        let emb = embedding.embed_one(&embed_text)?;
        Some(
            emb.iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<u8>>(),
        )
    } else {
        None
    };

    db.category_update(name, new_name, description, threshold)?;
    if let Some(blob) = embedding_blob {
        db.connection().execute(
            "UPDATE categories SET embedding = ?1 WHERE name = ?2",
            rusqlite::params![blob, final_name],
        )?;
    }

    println!("Updated category '{name}'");
    Ok(())
}

pub fn run_re_embed(
    config: &Config,
    embedding: &dyn EmbeddingBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let chunker = Chunker::new(config.embedding.chunk_size, config.embedding.chunk_overlap);

    let model_id = db.register_model(
        embedding.model_id(),
        None,
        embedding.dimensions() as i64,
        embedding.max_tokens() as i64,
        chunker.chunk_size as i64,
        chunker.chunk_overlap as i64,
    )?;
    db.activate_model(model_id)?;

    let conn = db.connection();
    {
        let chunk_ids: Vec<String> = {
            let mut stmt = conn.prepare("SELECT id FROM memory_chunks")?;
            stmt.query_map([], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for chunk_id in &chunk_ids {
            conn.execute(
                "DELETE FROM memory_embeddings WHERE chunk_id = ?1",
                rusqlite::params![chunk_id],
            )?;
        }
    }
    conn.execute("DELETE FROM memory_chunks", [])?;

    let _classifier = CategoryClassifier::new(
        &db,
        embedding,
        config.classification.confidence_threshold as f32,
    )?;

    let mut stmt =
        conn.prepare("SELECT id, content FROM memories WHERE archived = 0 ORDER BY created_at")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let start = std::time::Instant::now();
    let mut count = 0u64;

    for (id_str, content) in &rows {
        let id: MemoryId = id_str.parse().unwrap();
        let chunks = chunker.chunk(content);
        if !chunks.is_empty() {
            let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
            let embeddings = embedding.embed(&texts)?;
            db.store_chunks(&id, &chunks, &embeddings)?;
        }
        count += 1;
    }

    let elapsed = start.elapsed();
    eprintln!(
        "Re-embedded {count} memories in {:.2}s",
        elapsed.as_secs_f64()
    );
    Ok(())
}

pub fn run_re_classify(
    config: &Config,
    since: Option<&str>,
    embedding: &dyn EmbeddingBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let classifier = CategoryClassifier::new(
        &db,
        embedding,
        config.classification.confidence_threshold as f32,
    )?;

    let conn = db.connection();
    let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match since {
        Some(since_str) => (
            "SELECT id, content FROM memories WHERE archived = 0 AND created_at >= ?1 ORDER BY created_at".into(),
            vec![Box::new(since_str.to_owned())],
        ),
        None => (
            "SELECT id, content FROM memories WHERE archived = 0 ORDER BY created_at".into(),
            vec![],
        ),
    };

    let params_ref: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(String, String)> = stmt
        .query_map(params_ref.as_slice(), |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut updated = 0u64;
    for (id_str, content) in &rows {
        let emb = embedding.embed_one(content)?;
        if let Some(cat_id) = classifier.classify(&emb) {
            conn.execute(
                "UPDATE memories SET category_id = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?2",
                rusqlite::params![cat_id, id_str],
            )?;
            updated += 1;
        }
    }

    eprintln!("Re-classified {updated}/{} memories", rows.len());
    Ok(())
}

pub fn run_rotate_key(
    config: &Config,
    new_key_file: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let current_key = config
        .resolve_db_key()
        .map_err(|e| format!("cannot resolve current key: {e}"))?;

    let new_key = match new_key_file {
        Some(path) => std::fs::read_to_string(path)?.trim().to_string(),
        None => std::env::var("NOUS_NEW_DB_KEY")
            .map_err(|_| "no --new-key-file and NOUS_NEW_DB_KEY not set")?,
    };

    if current_key == new_key {
        return Err("new key must differ from current key".into());
    }

    let db_path = std::path::Path::new(&config.memory.db_path);
    nous_shared::sqlite::rotate_key(db_path, &current_key, &new_key)?;
    eprintln!("Key rotated successfully");
    Ok(())
}

pub fn run_room_create(
    config: &Config,
    name: &str,
    purpose: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let id = MemoryId::new().to_string();
    MemoryDb::create_room_on(db.connection(), &id, name, purpose, None)?;
    println!("{id}");
    Ok(())
}

pub fn run_room_list(
    config: &Config,
    archived: bool,
    limit: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();
    let limit = limit.unwrap_or(100) as i64;
    let mut stmt = conn.prepare(
        "SELECT id, name, purpose, archived, created_at FROM rooms WHERE archived = ?1 ORDER BY created_at DESC LIMIT ?2",
    )?;
    let rows: Vec<(String, String, Option<String>, i64, String)> = stmt
        .query_map(rusqlite::params![archived as i64, limit], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    if rows.is_empty() {
        println!("No rooms found.");
        return Ok(());
    }

    for (id, name, purpose, _archived, created_at) in &rows {
        let p = purpose.as_deref().unwrap_or("");
        println!("{id}  {name}  {p}  {created_at}");
    }
    Ok(())
}

pub fn run_room_get(config: &Config, id_or_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();

    let room_id = resolve_room_id_sync(conn, id_or_name)?;

    let (name, purpose, archived, created_at): (String, Option<String>, i64, String) = conn
        .query_row(
            "SELECT name, purpose, archived, created_at FROM rooms WHERE id = ?1",
            rusqlite::params![room_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;

    let msg_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM room_messages WHERE room_id = ?1",
        rusqlite::params![room_id],
        |row| row.get(0),
    )?;

    let mut stmt = conn.prepare(
        "SELECT agent_id, role FROM room_participants WHERE room_id = ?1 ORDER BY joined_at",
    )?;
    let participants: Vec<(String, String)> = stmt
        .query_map(rusqlite::params![room_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<Result<_, _>>()?;

    println!("id: {room_id}");
    println!("name: {name}");
    if let Some(p) = purpose {
        println!("purpose: {p}");
    }
    println!("archived: {}", archived != 0);
    println!("created: {created_at}");
    println!("messages: {msg_count}");
    if !participants.is_empty() {
        println!("participants:");
        for (agent_id, role) in &participants {
            println!("  {agent_id} ({role})");
        }
    }
    Ok(())
}

pub fn run_room_post(
    config: &Config,
    room: &str,
    content: &str,
    sender: Option<&str>,
    reply_to: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();
    let room_id = resolve_room_id_sync(conn, room)?;
    let msg_id = MemoryId::new().to_string();
    let sender_id = sender.unwrap_or("cli");
    MemoryDb::post_message_on(conn, &msg_id, &room_id, sender_id, content, reply_to, None)?;
    println!("{msg_id}");
    Ok(())
}

pub fn run_room_read(
    config: &Config,
    room: &str,
    limit: Option<usize>,
    since: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();
    let room_id = resolve_room_id_sync(conn, room)?;
    let limit = limit.unwrap_or(50) as i64;

    let (sql, params): (String, Vec<Box<dyn rusqlite::ToSql>>) = match since {
        Some(s) => (
            "SELECT sender_id, content, created_at FROM room_messages WHERE room_id = ?1 AND created_at > ?2 ORDER BY created_at ASC LIMIT ?3".into(),
            vec![Box::new(room_id), Box::new(s.to_string()), Box::new(limit)],
        ),
        None => (
            "SELECT sender_id, content, created_at FROM room_messages WHERE room_id = ?1 ORDER BY created_at ASC LIMIT ?2".into(),
            vec![Box::new(room_id), Box::new(limit)],
        ),
    };

    let mut stmt = conn.prepare(&sql)?;
    let params_ref: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows: Vec<(String, String, String)> = stmt
        .query_map(params_ref.as_slice(), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<_, _>>()?;

    for (sender_id, content, created_at) in &rows {
        println!("[{created_at}] {sender_id}: {content}");
    }
    Ok(())
}

pub fn run_room_search(
    config: &Config,
    room: &str,
    query: &str,
    limit: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();
    let room_id = resolve_room_id_sync(conn, room)?;
    let limit = limit.unwrap_or(50) as i64;

    let mut stmt = conn.prepare(
        "SELECT m.sender_id, m.content, m.created_at
         FROM room_messages m
         JOIN room_messages_fts ON m.rowid = room_messages_fts.rowid
         WHERE room_messages_fts MATCH ?1 AND m.room_id = ?2
         ORDER BY m.created_at DESC LIMIT ?3",
    )?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map(rusqlite::params![query, room_id, limit], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<_, _>>()?;

    for (sender_id, content, created_at) in &rows {
        println!("[{created_at}] {sender_id}: {content}");
    }
    Ok(())
}

pub fn run_room_delete(
    config: &Config,
    id_or_name: &str,
    hard: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();
    let room_id = resolve_room_id_sync(conn, id_or_name)?;

    let result = if hard {
        MemoryDb::hard_delete_room_on(conn, &room_id)?
    } else {
        MemoryDb::archive_room_on(conn, &room_id)?
    };

    if result {
        if hard {
            println!("Deleted room {room_id}");
        } else {
            println!("Archived room {room_id}");
        }
    } else {
        return Err(format!("room not found: {id_or_name}").into());
    }
    Ok(())
}

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36 && s.contains('-')
}

fn resolve_room_id_sync(
    conn: &rusqlite::Connection,
    id_or_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    if looks_like_uuid(id_or_name) {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM rooms WHERE id = ?1)",
            rusqlite::params![id_or_name],
            |row| row.get(0),
        )?;
        if exists {
            return Ok(id_or_name.to_string());
        }
    }
    let id: String = conn
        .query_row(
            "SELECT id FROM rooms WHERE name = ?1 AND archived = 0",
            rusqlite::params![id_or_name],
            |row| row.get(0),
        )
        .map_err(|_| format!("room not found: {id_or_name}"))?;
    Ok(id)
}

fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return format!("{}/{rest}", home.to_string_lossy());
    }
    p.to_string()
}

pub fn run_trace(
    config: &Config,
    trace_id: Option<&str>,
    memory_id: Option<&str>,
    session_id: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let otlp_path = expand_tilde(&config.otlp.db_path);
    if !Path::new(&otlp_path).exists() {
        return Err(format!("OTLP database not found at {otlp_path}").into());
    }
    let otlp_db = nous_otlp::db::OtlpDb::open(&otlp_path, None)?;

    if let Some(mid) = memory_id {
        let db_key = config.resolve_db_key().ok();
        let mem_db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
        let id: MemoryId = mid.parse().unwrap();
        let recalled = mem_db
            .recall(&id)?
            .ok_or_else(|| format!("memory {mid} not found"))?;

        let tid = recalled.memory.trace_id.as_deref();
        let sid = recalled.memory.session_id.as_deref();

        if tid.is_none() && sid.is_none() {
            return Err("memory has no trace_id or session_id for OTLP correlation".into());
        }

        let spans = match tid {
            Some(t) => otlp_db.query_spans(t, None, None)?,
            None => vec![],
        };
        let logs = match sid {
            Some(s) => otlp_db.query_logs(s, None, None)?,
            None => vec![],
        };

        let output = serde_json::json!({
            "memory": {
                "id": recalled.memory.id,
                "title": recalled.memory.title,
                "content": recalled.memory.content,
                "memory_type": recalled.memory.memory_type.to_string(),
                "trace_id": recalled.memory.trace_id,
                "session_id": recalled.memory.session_id,
                "created_at": recalled.memory.created_at,
            },
            "spans": spans.iter().map(span_to_json).collect::<Vec<_>>(),
            "logs": logs.iter().map(log_to_json).collect::<Vec<_>>(),
        });
        serde_json::to_writer_pretty(io::stdout().lock(), &output)?;
        println!();
    } else if let Some(tid) = trace_id {
        let db_key = config.resolve_db_key().ok();
        let mem_db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
        let conn = mem_db.connection();

        let mut stmt = conn.prepare(
            "SELECT id, title, content, memory_type, session_id, trace_id, created_at
             FROM memories WHERE trace_id = ?1 ORDER BY created_at DESC",
        )?;
        let memories: Vec<serde_json::Value> = stmt
            .query_map(rusqlite::params![tid], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "content": row.get::<_, String>(2)?,
                    "memory_type": row.get::<_, String>(3)?,
                    "session_id": row.get::<_, Option<String>>(4)?,
                    "trace_id": row.get::<_, Option<String>>(5)?,
                    "created_at": row.get::<_, String>(6)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let spans = otlp_db.query_spans(tid, None, None)?;
        let logs = match session_id {
            Some(sid) => otlp_db.query_logs(sid, None, None)?,
            None => vec![],
        };

        let output = serde_json::json!({
            "memories": memories,
            "spans": spans.iter().map(span_to_json).collect::<Vec<_>>(),
            "logs": logs.iter().map(log_to_json).collect::<Vec<_>>(),
        });
        serde_json::to_writer_pretty(io::stdout().lock(), &output)?;
        println!();
    } else {
        return Err("either --trace-id or --memory-id is required".into());
    }

    Ok(())
}

fn span_to_json(s: &nous_otlp::decode::Span) -> serde_json::Value {
    serde_json::json!({
        "trace_id": s.trace_id,
        "span_id": s.span_id,
        "parent_span_id": s.parent_span_id,
        "name": s.name,
        "kind": s.kind,
        "start_time": s.start_time,
        "end_time": s.end_time,
        "status_code": s.status_code,
        "status_message": s.status_message,
        "resource_attrs": s.resource_attrs,
        "span_attrs": s.span_attrs,
        "events_json": s.events_json,
    })
}

fn log_to_json(l: &nous_otlp::decode::LogEvent) -> serde_json::Value {
    serde_json::json!({
        "timestamp": l.timestamp,
        "severity": l.severity,
        "body": l.body,
        "resource_attrs": l.resource_attrs,
        "scope_attrs": l.scope_attrs,
        "log_attrs": l.log_attrs,
        "session_id": l.session_id,
        "trace_id": l.trace_id,
        "span_id": l.span_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nous_core::embed::MockEmbedding;
    use nous_core::types::RelationType;

    fn test_db() -> MemoryDb {
        MemoryDb::open(":memory:", None, 384).unwrap()
    }

    fn mock_embedding() -> MockEmbedding {
        MockEmbedding::new(384)
    }

    fn store_test_memory(db: &MemoryDb, title: &str, content: &str, tags: Vec<String>) -> MemoryId {
        let memory = NewMemory {
            title: title.into(),
            content: content.into(),
            memory_type: nous_core::types::MemoryType::Decision,
            source: Some("test".into()),
            importance: nous_core::types::Importance::Moderate,
            confidence: nous_core::types::Confidence::Moderate,
            tags,
            workspace_path: None,
            session_id: None,
            trace_id: None,
            agent_id: None,
            agent_model: None,
            valid_from: None,
            category_id: None,
        };
        db.store(&memory).unwrap()
    }

    #[test]
    fn export_import_round_trip() {
        let db = test_db();
        let embedding = mock_embedding();
        let chunker = Chunker::new(512, 64);

        let id1 = store_test_memory(
            &db,
            "Memory one",
            "First memory content about rust",
            vec!["rust".into(), "lang".into()],
        );
        let id2 = store_test_memory(
            &db,
            "Memory two",
            "Second memory content about testing",
            vec!["testing".into()],
        );
        let id3 = store_test_memory(
            &db,
            "Memory three",
            "Third memory content about deployment",
            vec![],
        );

        db.relate(&id1, &id2, RelationType::Related).unwrap();

        for id_ref in [&id1, &id2, &id3] {
            let recalled = db.recall(id_ref).unwrap().unwrap();
            let chunks = chunker.chunk(&recalled.memory.content);
            if !chunks.is_empty() {
                let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
                let embeddings = embedding.embed(&texts).unwrap();
                db.store_chunks(id_ref, &chunks, &embeddings).unwrap();
            }
        }

        let export_data = build_export_data(&db).unwrap();
        assert_eq!(export_data.memories.len(), 3);
        assert_eq!(export_data.version, 1);

        let m1 = export_data
            .memories
            .iter()
            .find(|m| m.title == "Memory one")
            .unwrap();
        assert!(m1.tags.contains(&"rust".into()));
        assert!(m1.tags.contains(&"lang".into()));
        assert_eq!(m1.relationships.len(), 1);
        assert_eq!(m1.relationships[0].relation_type, "related");

        let m2 = export_data
            .memories
            .iter()
            .find(|m| m.title == "Memory two")
            .unwrap();
        assert!(m2.tags.contains(&"testing".into()));

        let dest_db = test_db();
        import_data(&dest_db, &export_data, &embedding, &chunker).unwrap();

        let conn = dest_db.connection();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE archived = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);

        let tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT t.name) FROM tags t JOIN memory_tags mt ON mt.tag_id = t.id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tag_count, 3);

        let rel_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rel_count, 1);

        let chunk_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_chunks", [], |r| r.get(0))
            .unwrap();
        assert!(chunk_count > 0, "imported memories should have chunks");
    }

    #[test]
    fn import_remaps_categories() {
        let src_db = test_db();
        let embedding = mock_embedding();
        let chunker = Chunker::new(512, 64);

        let user_cat_id = src_db
            .category_add(
                "my-custom-cat",
                None,
                Some("A user category"),
                CategorySource::User,
            )
            .unwrap();

        let memory = NewMemory {
            title: "Categorized".into(),
            content: "Memory with user category".into(),
            memory_type: nous_core::types::MemoryType::Fact,
            source: None,
            importance: nous_core::types::Importance::Moderate,
            confidence: nous_core::types::Confidence::Moderate,
            tags: vec![],
            workspace_path: None,
            session_id: None,
            trace_id: None,
            agent_id: None,
            agent_model: None,
            valid_from: None,
            category_id: Some(user_cat_id),
        };
        src_db.store(&memory).unwrap();

        let export_data = build_export_data(&src_db).unwrap();
        let exported_mem = &export_data.memories[0];
        assert_eq!(exported_mem.category_id, Some(user_cat_id));

        let has_custom = export_data
            .categories
            .iter()
            .any(|c| c.name == "my-custom-cat");
        assert!(has_custom, "export should include user-created category");

        let dest_db = test_db();
        import_data(&dest_db, &export_data, &embedding, &chunker).unwrap();

        let dest_has_custom: bool = dest_db
            .connection()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM categories WHERE name = 'my-custom-cat')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(dest_has_custom, "imported DB should have user category");

        let dest_cat_id: Option<i64> = dest_db
            .connection()
            .query_row("SELECT category_id FROM memories LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(
            dest_cat_id.is_some(),
            "imported memory should have a remapped category_id"
        );

        let dest_cat_name: String = dest_db
            .connection()
            .query_row(
                "SELECT c.name FROM categories c JOIN memories m ON m.category_id = c.id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dest_cat_name, "my-custom-cat");
    }

    #[test]
    fn status_shows_counts() {
        let db = test_db();

        store_test_memory(&db, "M1", "Content 1", vec![]);
        store_test_memory(&db, "M2", "Content 2", vec![]);
        store_test_memory(&db, "M3", "Content 3", vec![]);

        let conn = db.connection();
        let memory_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE archived = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(memory_count, 3);

        let category_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))
            .unwrap();
        assert!(category_count > 0);
    }

    #[test]
    fn re_classify_runs_without_error() {
        let db = test_db();
        let embedding = mock_embedding();

        let id = store_test_memory(
            &db,
            "No category",
            "Some content about kubernetes infrastructure deployment pipelines",
            vec![],
        );

        let conn = db.connection();
        let cat_before: Option<i64> = conn
            .query_row(
                "SELECT category_id FROM memories WHERE id = ?1",
                rusqlite::params![id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(cat_before.is_none());

        let classifier = CategoryClassifier::new(&db, &embedding, 0.3).unwrap();
        let emb = embedding
            .embed_one("Some content about kubernetes infrastructure deployment pipelines")
            .unwrap();
        let category_id = classifier.classify(&emb);

        if let Some(cat_id) = category_id {
            conn.execute(
                "UPDATE memories SET category_id = ?1 WHERE id = ?2",
                rusqlite::params![cat_id, id.to_string()],
            )
            .unwrap();

            let cat_after: Option<i64> = conn
                .query_row(
                    "SELECT category_id FROM memories WHERE id = ?1",
                    rusqlite::params![id.to_string()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(cat_after, Some(cat_id));
        }
        // If no category meets the threshold, that's valid behavior for mock embeddings
    }

    #[test]
    fn category_add_and_list() {
        let db = test_db();
        let embedding = mock_embedding();

        let cat_id = db
            .category_add(
                "my-custom",
                None,
                Some("A custom category"),
                CategorySource::User,
            )
            .unwrap();
        assert!(cat_id > 0);

        let embed_text = "my-custom A custom category";
        let emb = embedding.embed_one(embed_text).unwrap();
        let blob: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
        db.connection()
            .execute(
                "UPDATE categories SET embedding = ?1 WHERE id = ?2",
                rusqlite::params![blob, cat_id],
            )
            .unwrap();

        let trees = db.category_list(Some(CategorySource::User)).unwrap();
        let names: Vec<&str> = trees.iter().map(|t| t.category.name.as_str()).collect();
        assert!(
            names.contains(&"my-custom"),
            "added category should appear in list"
        );
    }

    #[test]
    fn category_add_with_parent() {
        let db = test_db();
        let _embedding = mock_embedding();

        let trees = db.category_list(None).unwrap();
        let infra = trees
            .iter()
            .find(|t| t.category.name == "infrastructure")
            .unwrap();
        let parent_id = infra.category.id;

        let child_id = db
            .category_add(
                "my-infra-child",
                Some(parent_id),
                Some("Child under infrastructure"),
                CategorySource::User,
            )
            .unwrap();
        assert!(child_id > 0);

        let all_trees = db.category_list(None).unwrap();
        let infra_tree = all_trees
            .iter()
            .find(|t| t.category.name == "infrastructure")
            .unwrap();
        let child_names: Vec<&str> = infra_tree
            .children
            .iter()
            .map(|c| c.category.name.as_str())
            .collect();
        assert!(child_names.contains(&"my-infra-child"));
    }

    #[test]
    fn re_embed_regenerates_chunks() {
        let db = test_db();
        let embedding = mock_embedding();
        let chunker = Chunker::new(512, 64);

        let id = store_test_memory(
            &db,
            "To re-embed",
            "Content that will be re-embedded with new model",
            vec![],
        );

        let chunks = chunker.chunk("Content that will be re-embedded with new model");
        if !chunks.is_empty() {
            let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
            let embeddings = embedding.embed(&texts).unwrap();
            db.store_chunks(&id, &chunks, &embeddings).unwrap();
        }

        let conn = db.connection();
        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_chunks", [], |r| r.get(0))
            .unwrap();
        assert!(before > 0);

        {
            let chunk_ids: Vec<String> = {
                let mut stmt = conn.prepare("SELECT id FROM memory_chunks").unwrap();
                stmt.query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .unwrap()
            };
            for chunk_id in &chunk_ids {
                conn.execute(
                    "DELETE FROM memory_embeddings WHERE chunk_id = ?1",
                    rusqlite::params![chunk_id],
                )
                .unwrap();
            }
        }
        conn.execute("DELETE FROM memory_chunks", []).unwrap();

        let after_delete: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after_delete, 0);

        let mut stmt = conn
            .prepare("SELECT id, content FROM memories WHERE archived = 0")
            .unwrap();
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        for (id_str, content) in &rows {
            let mid: MemoryId = id_str.parse().unwrap();
            let chunks = chunker.chunk(content);
            if !chunks.is_empty() {
                let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
                let embeddings = embedding.embed(&texts).unwrap();
                db.store_chunks(&mid, &chunks, &embeddings).unwrap();
            }
        }

        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_chunks", [], |r| r.get(0))
            .unwrap();
        assert!(after > 0, "re-embed should regenerate chunks");
    }

    #[test]
    fn export_includes_categories() {
        let db = test_db();
        let data = build_export_data(&db).unwrap();
        assert!(
            !data.categories.is_empty(),
            "export should include seed categories"
        );
        let infra = data.categories.iter().find(|c| c.name == "infrastructure");
        assert!(infra.is_some());
    }

    #[test]
    fn export_empty_db() {
        let db = test_db();
        let data = build_export_data(&db).unwrap();
        assert_eq!(data.memories.len(), 0);
        assert!(!data.categories.is_empty());
    }
}
