use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use nous_otlp::db::OtlpDb;
use nous_otlp::server::run_server;

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
    }

    Ok(())
}
