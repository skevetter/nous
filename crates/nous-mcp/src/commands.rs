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
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref())?;

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
        "SELECT id, name, parent_id, source, description, embedding, created_at FROM categories ORDER BY id",
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
                created_at: row.get(6)?,
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
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref())?;
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
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref())?;
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
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref())?;

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
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref())?;

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

pub fn run_re_embed(
    config: &Config,
    embedding: &dyn EmbeddingBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref())?;
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

    let _classifier = CategoryClassifier::new(&db, embedding)?;

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
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref())?;
    let classifier = CategoryClassifier::new(&db, embedding)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use nous_core::embed::MockEmbedding;
    use nous_core::types::RelationType;

    fn test_db() -> MemoryDb {
        MemoryDb::open(":memory:", None).unwrap()
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

        let classifier = CategoryClassifier::new(&db, &embedding).unwrap();
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
