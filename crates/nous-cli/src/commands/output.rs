use comfy_table::{Cell, Table};
use serde_json::Value;

#[derive(Clone, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Json,
    Table,
    Csv,
}

/// Parse a comma-separated fields string into a Vec of owned strings.
pub fn parse_fields(fields: &str) -> Vec<String> {
    fields.split(',').map(|f| f.trim().to_owned()).filter(|f| !f.is_empty()).collect()
}

/// Print a list value in the given format.
///
/// `fields_override` — if Some, use those column names; otherwise use `default_columns`.
pub fn print_list(values: &Value, format: &OutputFormat, default_columns: &[&str], fields_override: Option<&[String]>) {
    let owned: Vec<String>;
    let columns: Vec<&str> = if let Some(f) = fields_override {
        f.iter().map(String::as_str).collect()
    } else {
        owned = default_columns.iter().map(|s| s.to_string()).collect();
        owned.iter().map(String::as_str).collect()
    };

    match format {
        OutputFormat::Json => print_json(values, &columns),
        OutputFormat::Table => print_table(values, &columns),
        OutputFormat::Csv => print_csv(values, &columns),
    }
}

fn print_json(values: &Value, columns: &[&str]) {
    // When columns cover all fields we just pretty-print as-is.
    // When a fields filter is active we project each object to only the requested keys.
    let projected = project(values, columns);
    println!("{}", serde_json::to_string_pretty(&projected).unwrap_or_default());
}

fn project(values: &Value, columns: &[&str]) -> Value {
    match values {
        Value::Array(rows) => Value::Array(rows.iter().map(|r| project_row(r, columns)).collect()),
        other => other.clone(),
    }
}

fn project_row(row: &Value, columns: &[&str]) -> Value {
    if let Value::Object(map) = row {
        let projected: serde_json::Map<String, Value> = columns
            .iter()
            .filter_map(|c| map.get(*c).map(|v| (c.to_string(), v.clone())))
            .collect();
        Value::Object(projected)
    } else {
        row.clone()
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
    let Some(rows) = values.as_array() else {
        println!("{}", serde_json::to_string_pretty(values).unwrap_or_default());
        return;
    };

    let mut table = Table::new();
    table.set_header(columns.iter().map(Cell::new).collect::<Vec<_>>());

    for row in rows {
        table.add_row(columns.iter().map(|c| Cell::new(cell_value(row, c))).collect::<Vec<_>>());
    }

    println!("{table}");
}

fn print_csv(values: &Value, columns: &[&str]) {
    let Some(rows) = values.as_array() else {
        println!("{}", serde_json::to_string_pretty(values).unwrap_or_default());
        return;
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
