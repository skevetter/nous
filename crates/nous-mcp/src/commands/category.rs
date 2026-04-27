use nous_core::db::MemoryDb;
use nous_core::embed::EmbeddingBackend;
use nous_core::types::{CategorySource, CategoryTree};
use nous_shared::NousError;
use nous_shared::ids::MemoryId;
use serde::Serialize;

use super::OutputFormat;
use crate::config::Config;

use super::{print_csv, print_json};

pub fn run_category_list(
    config: &Config,
    source: Option<&str>,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(
        &config.memory.db_path,
        db_key.as_deref(),
        config.embedding.dimensions,
    )?;

    let source_filter = match source {
        Some(s) => {
            let parsed: CategorySource = s.parse().map_err(|e: String| e)?;
            Some(parsed)
        }
        None => None,
    };

    let trees = db.category_list(source_filter)?;

    match format {
        OutputFormat::Json => {
            #[derive(Serialize)]
            struct CatJson {
                id: i64,
                name: String,
                source: String,
                description: Option<String>,
                children: Vec<CatJson>,
            }
            fn tree_to_json(tree: &CategoryTree) -> CatJson {
                CatJson {
                    id: tree.category.id,
                    name: tree.category.name.clone(),
                    source: tree.category.source.to_string(),
                    description: tree.category.description.clone(),
                    children: tree.children.iter().map(tree_to_json).collect(),
                }
            }
            let items: Vec<CatJson> = trees.iter().map(tree_to_json).collect();
            print_json(&items)?;
        }
        OutputFormat::Csv => {
            let mut rows = Vec::new();
            fn flatten(tree: &CategoryTree, rows: &mut Vec<Vec<String>>, depth: usize) {
                rows.push(vec![
                    tree.category.id.to_string(),
                    tree.category.name.clone(),
                    tree.category.source.to_string(),
                    tree.category.description.clone().unwrap_or_default(),
                    depth.to_string(),
                ]);
                for child in &tree.children {
                    flatten(child, rows, depth + 1);
                }
            }
            for tree in &trees {
                flatten(tree, &mut rows, 0);
            }
            print_csv(&["id", "name", "source", "description", "depth"], &rows)?;
        }
        OutputFormat::Human => {
            for tree in &trees {
                print_category_tree(tree, 0);
            }
        }
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
    let db = MemoryDb::open(
        &config.memory.db_path,
        db_key.as_deref(),
        config.embedding.dimensions,
    )?;

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
    let db = MemoryDb::open(
        &config.memory.db_path,
        db_key.as_deref(),
        config.embedding.dimensions,
    )?;
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
    let db = MemoryDb::open(
        &config.memory.db_path,
        db_key.as_deref(),
        config.embedding.dimensions,
    )?;

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
    let db = MemoryDb::open(
        &config.memory.db_path,
        db_key.as_deref(),
        config.embedding.dimensions,
    )?;

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

#[allow(clippy::too_many_arguments)]
pub fn run_category_suggest(
    config: &Config,
    memory_id: &str,
    name: &str,
    description: Option<&str>,
    parent: Option<&str>,
    embedding: &dyn EmbeddingBackend,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let mid = memory_id.parse::<MemoryId>().map_err(|e| {
        Box::new(NousError::Validation(format!("invalid id: {e}"))) as Box<dyn std::error::Error>
    })?;

    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(
        &config.memory.db_path,
        db_key.as_deref(),
        config.embedding.dimensions,
    )?;

    let exists: bool = db
        .connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM memories WHERE id = ?1)",
            rusqlite::params![mid.to_string()],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if !exists {
        return Err(Box::new(NousError::NotFound(format!(
            "memory not found: {memory_id}"
        ))));
    }

    let parent_id = match parent {
        Some(parent_str) => {
            if let Ok(id) = parent_str.parse::<i64>() {
                Some(id)
            } else {
                let id: i64 = db
                    .connection()
                    .query_row(
                        "SELECT id FROM categories WHERE name = ?1",
                        rusqlite::params![parent_str],
                        |row| row.get(0),
                    )
                    .map_err(|_| format!("parent category '{parent_str}' not found"))?;
                Some(id)
            }
        }
        None => None,
    };

    let cat_id = db.category_suggest(name, description, parent_id, &mid)?;

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

    match format {
        OutputFormat::Json => {
            #[derive(Serialize)]
            struct SuggestResult {
                category_id: i64,
                category_name: String,
                memory_id: String,
            }
            print_json(&SuggestResult {
                category_id: cat_id,
                category_name: name.to_string(),
                memory_id: memory_id.to_string(),
            })?;
        }
        OutputFormat::Csv => {
            print_csv(
                &["category_id", "category_name", "memory_id"],
                &[vec![
                    cat_id.to_string(),
                    name.to_string(),
                    memory_id.to_string(),
                ]],
            )?;
        }
        OutputFormat::Human => {
            println!("Category created: {cat_id} ({name})");
            println!("Memory updated: {memory_id}");
        }
    }
    Ok(())
}
