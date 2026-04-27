use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use nous_otlp::db::OtlpDb;
use nous_otlp::server::run_server;
use serde::Serialize;

#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
    Csv,
}

#[derive(Debug, Parser)]
#[command(name = "nous-otlp", about = "OTLP HTTP receiver with SQLite storage")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value_t = 4318)]
        port: u16,
        #[arg(long)]
        db: Option<PathBuf>,
    },
    Status {
        #[arg(long)]
        db: Option<PathBuf>,
    },
    Logs {
        session_id: String,
        #[arg(long, default_value_t = 100)]
        limit: i64,
        #[arg(long, default_value_t = 0)]
        offset: i64,
        #[arg(long, default_value = "human")]
        format: OutputFormat,
    },
    Spans {
        trace_id: String,
        #[arg(long, default_value_t = 100)]
        limit: i64,
        #[arg(long, default_value_t = 0)]
        offset: i64,
        #[arg(long, default_value = "human")]
        format: OutputFormat,
    },
}

fn resolve_db_path(db: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    match db {
        Some(p) => Ok(p),
        None => {
            let dir = nous_shared::xdg::cache_dir()?;
            Ok(dir.join("otlp.db"))
        }
    }
}

fn format_timestamp(ts: i64) -> String {
    if ts == 0 {
        return "0".to_string();
    }
    let secs = ts / 1_000_000_000;
    let nanos = ts % 1_000_000_000;

    let days_from_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let (hour, min, sec) = (
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60,
    );

    let mut y = 1970_i64;
    let mut remaining_days = days_from_epoch;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        y += 1;
    }
    let month_days: [i64; 12] = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0;
    for (i, &d) in month_days.iter().enumerate() {
        if remaining_days < d {
            m = i;
            break;
        }
        remaining_days -= d;
    }
    let _ = nanos;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m + 1,
        remaining_days + 1,
        hour,
        min,
        sec
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn format_duration_ms(start: i64, end: i64) -> String {
    let nanos = end.saturating_sub(start);
    let ms = nanos as f64 / 1_000_000.0;
    if ms < 1.0 {
        format!("{ms:.2}ms")
    } else {
        format!("{ms:.0}ms")
    }
}

fn span_kind_str(kind: i32) -> &'static str {
    match kind {
        0 => "UNSPECIFIED",
        1 => "INTERNAL",
        2 => "SERVER",
        3 => "CLIENT",
        4 => "PRODUCER",
        5 => "CONSUMER",
        _ => "UNKNOWN",
    }
}

fn status_code_str(code: i32) -> &'static str {
    match code {
        0 => "UNSET",
        1 => "OK",
        2 => "ERROR",
        _ => "UNKNOWN",
    }
}

fn print_table(headers: &[&str], rows: &[Vec<String>]) {
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

fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    serde_json::to_writer_pretty(std::io::stdout().lock(), value)?;
    println!();
    Ok(())
}

fn print_csv(headers: &[&str], rows: &[Vec<String>]) -> anyhow::Result<()> {
    let mut wtr = csv::Writer::from_writer(std::io::stdout().lock());
    wtr.write_record(headers)?;
    for row in rows {
        wtr.write_record(row)?;
    }
    wtr.flush()?;
    Ok(())
}

fn run_otlp_logs(
    db: &OtlpDb,
    session_id: &str,
    limit: i64,
    offset: i64,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    let logs = db.query_logs(session_id, Some(limit), Some(offset))?;

    match format {
        OutputFormat::Json => {
            #[derive(Serialize)]
            struct LogRow {
                timestamp: String,
                severity: String,
                body: String,
            }
            let items: Vec<LogRow> = logs
                .iter()
                .map(|l| LogRow {
                    timestamp: format_timestamp(l.timestamp),
                    severity: l.severity.clone(),
                    body: l.body.clone(),
                })
                .collect();
            print_json(&items)?;
        }
        OutputFormat::Csv => {
            let rows: Vec<Vec<String>> = logs
                .iter()
                .map(|l| {
                    vec![
                        format_timestamp(l.timestamp),
                        l.severity.clone(),
                        l.body.clone(),
                    ]
                })
                .collect();
            print_csv(&["timestamp", "severity", "body"], &rows)?;
        }
        OutputFormat::Human => {
            let rows: Vec<Vec<String>> = logs
                .iter()
                .map(|l| {
                    vec![
                        format_timestamp(l.timestamp),
                        l.severity.clone(),
                        l.body.clone(),
                    ]
                })
                .collect();
            print_table(&["Timestamp", "Severity", "Body"], &rows);
        }
    }
    Ok(())
}

fn run_otlp_spans(
    db: &OtlpDb,
    trace_id: &str,
    limit: i64,
    offset: i64,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    let spans = db.query_spans(trace_id, Some(limit), Some(offset))?;

    match format {
        OutputFormat::Json => {
            #[derive(Serialize)]
            struct SpanRow {
                span_id: String,
                name: String,
                kind: String,
                start_time: String,
                duration: String,
                status: String,
            }
            let items: Vec<SpanRow> = spans
                .iter()
                .map(|s| SpanRow {
                    span_id: s.span_id.clone(),
                    name: s.name.clone(),
                    kind: span_kind_str(s.kind).to_string(),
                    start_time: format_timestamp(s.start_time),
                    duration: format_duration_ms(s.start_time, s.end_time),
                    status: status_code_str(s.status_code).to_string(),
                })
                .collect();
            print_json(&items)?;
        }
        OutputFormat::Csv => {
            let rows: Vec<Vec<String>> = spans
                .iter()
                .map(|s| {
                    vec![
                        s.span_id.clone(),
                        s.name.clone(),
                        span_kind_str(s.kind).to_string(),
                        format_timestamp(s.start_time),
                        format_duration_ms(s.start_time, s.end_time),
                        status_code_str(s.status_code).to_string(),
                    ]
                })
                .collect();
            print_csv(
                &[
                    "span_id",
                    "name",
                    "kind",
                    "start_time",
                    "duration",
                    "status",
                ],
                &rows,
            )?;
        }
        OutputFormat::Human => {
            let rows: Vec<Vec<String>> = spans
                .iter()
                .map(|s| {
                    vec![
                        s.span_id.clone(),
                        s.name.clone(),
                        span_kind_str(s.kind).to_string(),
                        format_timestamp(s.start_time),
                        format_duration_ms(s.start_time, s.end_time),
                        status_code_str(s.status_code).to_string(),
                    ]
                })
                .collect();
            print_table(
                &[
                    "Span ID",
                    "Name",
                    "Kind",
                    "Start Time",
                    "Duration",
                    "Status",
                ],
                &rows,
            );
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { port, db } => {
            let db_path = resolve_db_path(db)?;
            let key = nous_shared::sqlite::resolve_key()?;
            let db = OtlpDb::open(db_path.to_str().unwrap(), Some(&key))?;
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            eprintln!("listening on {addr} (db: {})", db_path.display());
            run_server(db, addr).await?;
        }
        Command::Status { db } => {
            let db_path = resolve_db_path(db)?;
            let key = nous_shared::sqlite::resolve_key()?;
            let db = OtlpDb::open(db_path.to_str().unwrap(), Some(&key))?;
            let conn = db.connection();

            let logs: i64 = conn.query_row("SELECT count(*) FROM log_events", [], |r| r.get(0))?;
            let spans: i64 = conn.query_row("SELECT count(*) FROM spans", [], |r| r.get(0))?;
            let metrics: i64 = conn.query_row("SELECT count(*) FROM metrics", [], |r| r.get(0))?;

            println!("db: {}", db_path.display());
            println!("log_events: {logs}");
            println!("spans: {spans}");
            println!("metrics: {metrics}");
        }
        Command::Logs {
            session_id,
            limit,
            offset,
            format,
        } => {
            let db_path = resolve_db_path(None)?;
            let key = nous_shared::sqlite::resolve_key()?;
            let db = OtlpDb::open(db_path.to_str().unwrap(), Some(&key))?;
            run_otlp_logs(&db, &session_id, limit, offset, &format)?;
        }
        Command::Spans {
            trace_id,
            limit,
            offset,
            format,
        } => {
            let db_path = resolve_db_path(None)?;
            let key = nous_shared::sqlite::resolve_key()?;
            let db = OtlpDb::open(db_path.to_str().unwrap(), Some(&key))?;
            run_otlp_spans(&db, &trace_id, limit, offset, &format)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nous_otlp::decode::{LogEvent, Span};

    // --- Clap parsing tests ---

    #[test]
    fn logs_defaults() {
        let cli = Cli::try_parse_from(["nous-otlp", "logs", "sess-123"]).unwrap();
        match cli.command {
            Command::Logs {
                session_id,
                limit,
                offset,
                format,
            } => {
                assert_eq!(session_id, "sess-123");
                assert_eq!(limit, 100);
                assert_eq!(offset, 0);
                assert!(matches!(format, OutputFormat::Human));
            }
            _ => panic!("expected Logs"),
        }
    }

    #[test]
    fn logs_with_all_flags() {
        let cli = Cli::try_parse_from([
            "nous-otlp",
            "logs",
            "sess-abc",
            "--limit",
            "50",
            "--offset",
            "10",
            "--format",
            "json",
        ])
        .unwrap();
        match cli.command {
            Command::Logs {
                session_id,
                limit,
                offset,
                format,
            } => {
                assert_eq!(session_id, "sess-abc");
                assert_eq!(limit, 50);
                assert_eq!(offset, 10);
                assert!(matches!(format, OutputFormat::Json));
            }
            _ => panic!("expected Logs"),
        }
    }

    #[test]
    fn logs_csv_format() {
        let cli = Cli::try_parse_from(["nous-otlp", "logs", "sess-x", "--format", "csv"]).unwrap();
        match cli.command {
            Command::Logs { format, .. } => {
                assert!(matches!(format, OutputFormat::Csv));
            }
            _ => panic!("expected Logs"),
        }
    }

    #[test]
    fn spans_defaults() {
        let cli = Cli::try_parse_from(["nous-otlp", "spans", "trace-456"]).unwrap();
        match cli.command {
            Command::Spans {
                trace_id,
                limit,
                offset,
                format,
            } => {
                assert_eq!(trace_id, "trace-456");
                assert_eq!(limit, 100);
                assert_eq!(offset, 0);
                assert!(matches!(format, OutputFormat::Human));
            }
            _ => panic!("expected Spans"),
        }
    }

    #[test]
    fn spans_with_all_flags() {
        let cli = Cli::try_parse_from([
            "nous-otlp",
            "spans",
            "trace-xyz",
            "--limit",
            "25",
            "--offset",
            "5",
            "--format",
            "json",
        ])
        .unwrap();
        match cli.command {
            Command::Spans {
                trace_id,
                limit,
                offset,
                format,
            } => {
                assert_eq!(trace_id, "trace-xyz");
                assert_eq!(limit, 25);
                assert_eq!(offset, 5);
                assert!(matches!(format, OutputFormat::Json));
            }
            _ => panic!("expected Spans"),
        }
    }

    #[test]
    fn spans_csv_format() {
        let cli =
            Cli::try_parse_from(["nous-otlp", "spans", "trace-z", "--format", "csv"]).unwrap();
        match cli.command {
            Command::Spans { format, .. } => {
                assert!(matches!(format, OutputFormat::Csv));
            }
            _ => panic!("expected Spans"),
        }
    }

    // --- Integration tests ---

    fn make_log(ts: i64, session_id: &str, severity: &str, body: &str) -> LogEvent {
        LogEvent {
            timestamp: ts,
            severity: severity.to_string(),
            body: body.to_string(),
            resource_attrs: "{}".to_string(),
            scope_attrs: "{}".to_string(),
            log_attrs: "{}".to_string(),
            session_id: Some(session_id.to_string()),
            trace_id: None,
            span_id: None,
        }
    }

    fn make_span(start: i64, end: i64, trace_id: &str, name: &str) -> Span {
        Span {
            trace_id: trace_id.to_string(),
            span_id: format!("span-{start}"),
            parent_span_id: None,
            name: name.to_string(),
            kind: 1,
            start_time: start,
            end_time: end,
            status_code: 1,
            status_message: None,
            resource_attrs: "{}".to_string(),
            span_attrs: "{}".to_string(),
            events_json: "[]".to_string(),
        }
    }

    #[test]
    fn integration_otlp_logs_query() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        db.store_logs(&[
            make_log(1_000_000_000_000_000_000, "sess-int", "INFO", "hello"),
            make_log(2_000_000_000_000_000_000, "sess-int", "ERROR", "oops"),
            make_log(3_000_000_000_000_000_000, "sess-other", "DEBUG", "nope"),
        ])
        .unwrap();

        let logs = db.query_logs("sess-int", Some(100), Some(0)).unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].body, "hello");
        assert_eq!(logs[1].body, "oops");
    }

    #[test]
    fn integration_otlp_logs_pagination() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let logs: Vec<LogEvent> = (0..5)
            .map(|i| {
                make_log(
                    (i + 1) * 1_000_000_000_000_000_000,
                    "sess-pg",
                    "INFO",
                    &format!("log-{i}"),
                )
            })
            .collect();
        db.store_logs(&logs).unwrap();

        let page = db.query_logs("sess-pg", Some(2), Some(2)).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].body, "log-2");
        assert_eq!(page[1].body, "log-3");
    }

    #[test]
    fn integration_otlp_spans_query() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        db.store_spans(&[
            make_span(
                1_000_000_000_000_000_000,
                1_000_000_045_000_000_000,
                "trace-int",
                "memory_store",
            ),
            make_span(
                2_000_000_000_000_000_000,
                2_000_000_030_000_000_000,
                "trace-int",
                "embed_text",
            ),
            make_span(
                3_000_000_000_000_000_000,
                3_000_000_010_000_000_000,
                "trace-other",
                "nope",
            ),
        ])
        .unwrap();

        let spans = db.query_spans("trace-int", Some(100), Some(0)).unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].name, "memory_store");
        assert_eq!(spans[1].name, "embed_text");
    }

    #[test]
    fn integration_otlp_spans_pagination() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let spans: Vec<Span> = (0..5)
            .map(|i| {
                let start = (i + 1) * 1_000_000_000_000_000_000;
                make_span(start, start + 100_000_000, "trace-pg", &format!("op-{i}"))
            })
            .collect();
        db.store_spans(&spans).unwrap();

        let page = db.query_spans("trace-pg", Some(2), Some(1)).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].name, "op-1");
        assert_eq!(page[1].name, "op-2");
    }
}
