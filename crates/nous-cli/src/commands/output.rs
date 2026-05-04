use comfy_table::{Cell, Table};
use serde_json::Value;

#[derive(Clone, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Json,
    Table,
    Csv,
}

pub fn print_list(values: &Value, format: &OutputFormat, columns: &[&str]) {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(values).unwrap_or_default());
        }
        OutputFormat::Table => print_table(values, columns),
        OutputFormat::Csv => print_csv(values, columns),
    }
}

fn cell_value(row: &Value, col: &str) -> String {
    match row.get(col) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

fn print_table(values: &Value, columns: &[&str]) {
    let rows = match values.as_array() {
        Some(arr) => arr,
        None => {
            println!("{}", serde_json::to_string_pretty(values).unwrap_or_default());
            return;
        }
    };

    let mut table = Table::new();
    table.set_header(columns.iter().map(|c| Cell::new(c)).collect::<Vec<_>>());

    for row in rows {
        table.add_row(columns.iter().map(|c| Cell::new(cell_value(row, c))).collect::<Vec<_>>());
    }

    println!("{table}");
}

fn print_csv(values: &Value, columns: &[&str]) {
    let rows = match values.as_array() {
        Some(arr) => arr,
        None => {
            println!("{}", serde_json::to_string_pretty(values).unwrap_or_default());
            return;
        }
    };

    println!("{}", columns.join(","));
    for row in rows {
        let line = columns
            .iter()
            .map(|c| csv_escape(&cell_value(row, c)))
            .collect::<Vec<_>>()
            .join(",");
        println!("{line}");
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
