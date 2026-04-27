mod admin;
mod category;
pub mod memory;
mod model;
mod query;
mod room;
mod schedule;
mod trace;

pub use admin::{
    build_export_data, import_data, run_export, run_import, run_re_classify, run_re_embed,
    run_rotate_key, run_status,
};
pub use category::{
    run_category_add, run_category_delete, run_category_list, run_category_rename,
    run_category_suggest, run_category_update,
};
pub use memory::{
    run_forget, run_recall, run_relate, run_search, run_store, run_unarchive, run_unrelate,
    run_update,
};
pub use model::{
    run_embedding_inspect, run_embedding_reset, run_model_activate, run_model_deactivate,
    run_model_info, run_model_list, run_model_register, run_model_switch,
};
pub use query::{run_context, run_schema, run_sql, run_tags, run_workspaces};
pub use room::{
    run_room_create, run_room_delete, run_room_get, run_room_list, run_room_post, run_room_read,
    run_room_search,
};
pub use schedule::{
    run_schedule_create, run_schedule_delete, run_schedule_get, run_schedule_list,
    run_schedule_pause, run_schedule_resume,
};
pub use trace::run_trace;

use std::io::{self, BufRead, Write};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
    Csv,
}

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

pub fn print_json<T: Serialize>(value: &T) -> Result<(), Box<dyn std::error::Error>> {
    serde_json::to_writer_pretty(io::stdout().lock(), value)?;
    println!();
    Ok(())
}

pub fn print_csv(headers: &[&str], rows: &[Vec<String>]) -> Result<(), Box<dyn std::error::Error>> {
    let mut wtr = csv::Writer::from_writer(io::stdout().lock());
    wtr.write_record(headers)?;
    for row in rows {
        wtr.write_record(row)?;
    }
    wtr.flush()?;
    Ok(())
}

#[allow(dead_code)]
pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let col_count = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_count && cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }

    for (i, header) in headers.iter().enumerate() {
        if i > 0 {
            print!(" | ");
        }
        print!("{:width$}", header, width = widths[i]);
    }
    println!();

    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            print!("-+-");
        }
        print!("{}", "-".repeat(*w));
    }
    println!();

    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i >= col_count {
                break;
            }
            if i > 0 {
                print!(" | ");
            }
            print!("{:width$}", cell, width = widths[i]);
        }
        println!();
    }
}

#[allow(dead_code)]
pub fn confirm(prompt: &str) -> bool {
    eprint!("{prompt} [y/N] ");
    io::stderr().flush().ok();
    let mut input = String::new();
    if io::stdin().lock().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

pub fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return format!("{}/{rest}", home.to_string_lossy());
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_replaces_home_prefix() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_tilde("~/foo/bar.db"), format!("{home}/foo/bar.db"),);
    }

    #[test]
    fn expand_tilde_leaves_absolute_path_unchanged() {
        assert_eq!(expand_tilde("/tmp/memory.db"), "/tmp/memory.db");
    }

    #[test]
    fn expand_tilde_leaves_relative_path_unchanged() {
        assert_eq!(expand_tilde("data/memory.db"), "data/memory.db");
    }

    #[test]
    fn expand_tilde_leaves_bare_tilde_unchanged() {
        assert_eq!(expand_tilde("~"), "~");
    }
}
