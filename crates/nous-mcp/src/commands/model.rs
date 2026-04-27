use nous_core::db::MemoryDb;
use nous_shared::NousError;

use super::{OutputFormat, confirm, print_csv, print_json, print_table};
use crate::config::Config;

fn open_db(config: &Config) -> Result<MemoryDb, Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    Ok(MemoryDb::open(
        &config.memory.db_path,
        db_key.as_deref(),
        384,
    )?)
}

pub fn run_model_list(
    config: &Config,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let models = db.list_models()?;
    let vec0_dims = db.vec0_dimensions()?;

    match format {
        OutputFormat::Json => {
            let list: Vec<serde_json::Value> = models
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "name": m.name,
                        "variant": m.variant,
                        "dimensions": m.dimensions,
                        "max_tokens": m.max_tokens,
                        "active": m.active,
                        "created_at": m.created_at,
                    })
                })
                .collect();
            let mut result = serde_json::json!({ "models": list });
            if let Some(d) = vec0_dims {
                result["vec0_dimensions"] = serde_json::json!(d);
            }
            print_json(&result)?;
        }
        OutputFormat::Csv => {
            let rows: Vec<Vec<String>> = models
                .iter()
                .map(|m| {
                    vec![
                        m.id.to_string(),
                        m.name.clone(),
                        m.variant.clone().unwrap_or_default(),
                        m.dimensions.to_string(),
                        m.max_tokens.to_string(),
                        m.active.to_string(),
                        m.created_at.clone(),
                    ]
                })
                .collect();
            print_csv(
                &[
                    "id",
                    "name",
                    "variant",
                    "dimensions",
                    "max_tokens",
                    "active",
                    "created",
                ],
                &rows,
            )?;
        }
        OutputFormat::Human => {
            let rows: Vec<Vec<String>> = models
                .iter()
                .map(|m| {
                    vec![
                        m.id.to_string(),
                        m.name.clone(),
                        m.variant.clone().unwrap_or_default(),
                        m.dimensions.to_string(),
                        m.max_tokens.to_string(),
                        if m.active { "*".into() } else { String::new() },
                        m.created_at.chars().take(10).collect(),
                    ]
                })
                .collect();
            print_table(
                &[
                    "ID",
                    "Name",
                    "Variant",
                    "Dims",
                    "Max Tokens",
                    "Active",
                    "Created",
                ],
                &rows,
            );
        }
    }
    Ok(())
}

pub fn run_model_info(
    config: &Config,
    id: i64,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let model = db.get_model(id)?;
    let embed_count = db.embedding_count().unwrap_or(0);
    let vec0_dims = db.vec0_dimensions()?;

    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({
                "id": model.id,
                "name": model.name,
                "variant": model.variant,
                "dimensions": model.dimensions,
                "max_tokens": model.max_tokens,
                "chunk_size": model.chunk_size,
                "chunk_overlap": model.chunk_overlap,
                "active": model.active,
                "created_at": model.created_at,
                "embeddings": embed_count,
                "vec0_dimensions": vec0_dims,
            }))?;
        }
        OutputFormat::Csv | OutputFormat::Human => {
            println!("Model ID: {}", model.id);
            println!("Name: {}", model.name);
            println!("Variant: {}", model.variant.as_deref().unwrap_or("(none)"));
            println!("Dimensions: {}", model.dimensions);
            println!("Max Tokens: {}", model.max_tokens);
            println!("Chunk Size: {}", model.chunk_size);
            println!("Chunk Overlap: {}", model.chunk_overlap);
            println!("Active: {}", if model.active { "yes" } else { "no" });
            println!("Created: {}", model.created_at);
            println!();
            if let Some(d) = vec0_dims {
                println!("Embeddings: {embed_count} chunks in vec0 table (dimensions: {d})");
            } else {
                println!("Embeddings: {embed_count} chunks in vec0 table");
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn run_model_register(
    config: &Config,
    name: &str,
    variant: &str,
    dimensions: i64,
    max_tokens: i64,
    chunk_size: i64,
    chunk_overlap: i64,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let id = db.register_model(
        name,
        Some(variant),
        dimensions,
        max_tokens,
        chunk_size,
        chunk_overlap,
    )?;

    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({
                "id": id,
                "name": name,
                "message": format!("Model registered: ID {id}"),
            }))?;
        }
        OutputFormat::Csv | OutputFormat::Human => {
            println!("Model registered: ID {id}");
        }
    }
    Ok(())
}

pub fn run_model_activate(
    config: &Config,
    id: i64,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let model = db.get_model(id)?;
    db.activate_model(id)?;

    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({
                "id": model.id,
                "name": model.name,
                "active": true,
                "message": format!("Model activated: {} ({})", model.id, model.name),
            }))?;
        }
        OutputFormat::Csv | OutputFormat::Human => {
            println!("Model activated: {} ({})", model.id, model.name);
        }
    }
    Ok(())
}

pub fn run_model_deactivate(
    config: &Config,
    id: i64,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let model = db.get_model(id)?;
    db.deactivate_model(id)?;

    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({
                "id": model.id,
                "name": model.name,
                "active": false,
                "message": format!("Model deactivated: {} ({})", model.id, model.name),
            }))?;
        }
        OutputFormat::Csv | OutputFormat::Human => {
            println!("Model deactivated: {} ({})", model.id, model.name);
        }
    }
    Ok(())
}

pub fn run_model_switch(
    config: &Config,
    id: i64,
    force: bool,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let target = db.get_model(id)?;
    let current = db.active_model()?;
    let embed_count = db.embedding_count().unwrap_or(0);

    let dims_differ = current
        .as_ref()
        .is_some_and(|c| c.dimensions != target.dimensions);

    if dims_differ {
        let cur = current.as_ref().unwrap();
        let warning = format!(
            "Warning: Switching from model {} ({} dims) to model {} ({} dims) will reset all embeddings.\n\
             This will delete {} chunks from the vec0 table.",
            cur.id, cur.dimensions, target.id, target.dimensions, embed_count
        );
        eprintln!("{warning}");

        if !force && !confirm("\nProceed?") {
            eprintln!("Aborted.");
            return Ok(());
        }
    }

    if dims_differ {
        db.reset_embeddings(target.dimensions as usize)?;
    }

    db.activate_model(id)?;

    let mut messages = vec![format!("Model switched: {} ({})", target.id, target.name)];

    if dims_differ {
        messages.push(format!(
            "Embeddings reset: vec0 table recreated with {} dimensions",
            target.dimensions
        ));
    }

    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({
                "id": target.id,
                "name": target.name,
                "dimensions": target.dimensions,
                "reset": dims_differ,
                "messages": messages,
            }))?;
        }
        OutputFormat::Csv | OutputFormat::Human => {
            for msg in &messages {
                println!("{msg}");
            }
        }
    }
    Ok(())
}

pub fn run_embedding_inspect(
    config: &Config,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = open_db(config)?;
    let active = db.active_model()?;
    let embed_count = db.embedding_count().unwrap_or(0);
    let vec0_dims = db.vec0_dimensions()?;

    match format {
        OutputFormat::Json => {
            let mut result = serde_json::json!({
                "vec0_dimensions": vec0_dims,
                "embeddings": embed_count,
            });
            if let Some(ref m) = active {
                result["active_model"] = serde_json::json!({
                    "id": m.id,
                    "name": m.name,
                    "dimensions": m.dimensions,
                    "chunk_size": m.chunk_size,
                    "chunk_overlap": m.chunk_overlap,
                });
            }
            let mismatch = match (&active, vec0_dims) {
                (Some(m), Some(d)) => m.dimensions != d,
                _ => false,
            };
            result["dimension_mismatch"] = serde_json::json!(mismatch);
            print_json(&result)?;
        }
        OutputFormat::Csv | OutputFormat::Human => {
            if let Some(ref m) = active {
                println!("Active Model: {} ({})", m.id, m.name);
                println!("Model Dimensions: {}", m.dimensions);
                println!("Chunk Size: {}", m.chunk_size);
                println!("Chunk Overlap: {}", m.chunk_overlap);
            } else {
                println!("Active Model: (none)");
            }
            println!();
            println!("vec0 Table:");
            if let Some(d) = vec0_dims {
                println!("  Dimensions: {d}");
            } else {
                println!("  Dimensions: (unknown)");
            }
            println!("  Embeddings: {embed_count} chunks");

            match (&active, vec0_dims) {
                (Some(m), Some(d)) if m.dimensions != d => {
                    println!();
                    println!("Status: ERROR \u{2014} dimension mismatch!");
                    println!(
                        "The active model produces {}-dimensional embeddings, but vec0 table expects {} dimensions.",
                        m.dimensions, d
                    );
                    println!(
                        "Run 'nous embedding reset' to recreate the vec0 table, or switch back to a {d}-dim model."
                    );
                }
                (Some(_), Some(_)) => {
                    println!();
                    println!("Status: OK (dimensions match)");
                }
                _ => {
                    println!();
                    println!("Status: OK");
                }
            }
        }
    }
    Ok(())
}

pub fn run_embedding_reset(
    config: &Config,
    force: bool,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    if !force {
        return Err(Box::new(NousError::Validation(
            "embedding reset requires --force flag".to_string(),
        )) as Box<dyn std::error::Error>);
    }

    let db = open_db(config)?;
    let embed_count = db.embedding_count().unwrap_or(0);
    let active = db.active_model()?;

    let new_dims = active.as_ref().map(|m| m.dimensions).unwrap_or(384);
    db.reset_embeddings(new_dims as usize)?;

    match format {
        OutputFormat::Json => {
            let mut result = serde_json::json!({
                "deleted": embed_count,
                "new_dimensions": new_dims,
            });
            if let Some(ref m) = active {
                result["active_model"] = serde_json::json!({
                    "id": m.id,
                    "name": m.name,
                });
            }
            print_json(&result)?;
        }
        OutputFormat::Csv | OutputFormat::Human => {
            println!("vec0 table reset: {embed_count} embeddings deleted");
            if let Some(ref m) = active {
                println!(
                    "New dimensions: {} (matching active model {}: {})",
                    new_dims, m.id, m.name
                );
            } else {
                println!("New dimensions: {new_dims}");
            }
        }
    }
    Ok(())
}
