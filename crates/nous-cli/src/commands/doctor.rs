use std::net::TcpListener;
use std::process;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_daemon::llm_client::{credential_source, has_credentials, LlmConfig, ProviderKind};

use super::model;

pub async fn run(port: Option<u16>) {
    println!("nous doctor v{}", env!("CARGO_PKG_VERSION"));
    println!("==================");

    let mut has_failure = false;

    let config = match check_config() {
        Ok(mut config) => {
            if let Some(p) = port {
                config.port = p;
            }
            Some(config)
        }
        Err(()) => {
            has_failure = true;
            None
        }
    };

    if let Some(ref cfg) = config {
        if !check_storage(&cfg.data_dir) {
            has_failure = true;
        }
    } else {
        println!("[FAIL] Storage directory check skipped (config failed)");
        has_failure = true;
    }

    if let Some(ref cfg) = config {
        if !check_database(&cfg.data_dir).await {
            has_failure = true;
        }
    } else {
        println!("[FAIL] Database check skipped (config failed)");
        has_failure = true;
    }

    let daemon_port = config.as_ref().map(|c| c.port).unwrap_or(8377);
    check_port(daemon_port);
    check_embedding_model();
    check_llm_providers();

    if has_failure {
        process::exit(1);
    }
}

fn check_config() -> Result<Config, ()> {
    match Config::load() {
        Ok(config) => {
            let config_path = config_file_path();
            println!("[OK] Config loaded from {}", config_path.display());
            println!("     Data directory: {}", config.data_dir.display());
            Ok(config)
        }
        Err(e) => {
            println!("[FAIL] Config load failed: {e}");
            Err(())
        }
    }
}

fn check_storage(data_dir: &std::path::Path) -> bool {
    if let Err(e) = std::fs::create_dir_all(data_dir) {
        println!("[FAIL] Cannot create storage directory: {e}");
        return false;
    }

    let probe = data_dir.join(".nous-doctor-probe");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            println!("[OK] Storage directory is writable");
            true
        }
        Err(e) => {
            println!("[FAIL] Storage directory is not writable: {e}");
            false
        }
    }
}

async fn check_database(data_dir: &std::path::Path) -> bool {
    let pools = match DbPools::connect(data_dir).await {
        Ok(pools) => pools,
        Err(e) => {
            println!("[FAIL] Database connection failed: {e}");
            return false;
        }
    };

    println!("[OK] Database connectivity (memory-fts.db, memory-vec.db)");

    let migration_ok = match pools.run_migrations("porter unicode61").await {
        Ok(()) => {
            println!("[OK] Migrations up to date");
            true
        }
        Err(e) => {
            println!("[FAIL] Migration failed: {e}");
            false
        }
    };

    // Check tasks subsystem
    check_subsystem_table(&pools.fts, "tasks").await;

    // Check inventory subsystem
    check_subsystem_table(&pools.fts, "inventory").await;

    pools.close().await;
    migration_ok
}

async fn check_subsystem_table(pool: &sqlx::SqlitePool, table: &str) {
    match sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
    {
        Ok(count) => {
            println!("[OK] {table} table accessible ({count} rows)");
        }
        Err(e) => {
            println!("[FAIL] {table} table not accessible: {e}");
        }
    }
}

fn check_port(port: u16) {
    if is_port_available(port) {
        println!("[OK] Port {port} is available");
    } else {
        println!("[WARN] Port {port} is in use (daemon may be running)");
    }
}

pub fn is_port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn check_embedding_model() {
    let (model_ok, tokenizer_ok) = model::check_model_files();

    if model_ok && tokenizer_ok {
        println!("[OK] Embedding model files present");
    } else {
        if !model_ok {
            println!("[WARN] Missing: all-MiniLM-L6-v2.onnx");
        }
        if !tokenizer_ok {
            println!("[WARN] Missing: tokenizer.json");
        }
        println!("      Run `nous model download` to install embedding model");
    }
}

fn check_llm_providers() {
    let llm_config = LlmConfig::resolve(None, None, None, None);

    println!();
    println!("LLM Provider Configuration:");
    println!("  Active provider: {}", llm_config.provider);
    println!("  Model: {}", llm_config.model);
    if llm_config.provider == ProviderKind::Bedrock {
        println!("  Region: {}", llm_config.region);
        if let Some(ref profile) = llm_config.profile {
            println!("  Profile: {profile}");
        }
    }

    // Check credentials for active provider
    let active_creds = has_credentials(llm_config.provider);
    if active_creds {
        println!(
            "[OK] {} credentials available ({})",
            llm_config.provider,
            credential_source(llm_config.provider)
        );
    } else {
        println!(
            "[WARN] {} credentials not found (need: {})",
            llm_config.provider,
            credential_source(llm_config.provider)
        );
    }

    // Show status of all providers
    println!();
    println!("  All providers:");
    for kind in [ProviderKind::Bedrock, ProviderKind::Anthropic, ProviderKind::OpenAI] {
        let marker = if kind == llm_config.provider {
            " (active)"
        } else {
            ""
        };
        let status = if has_credentials(kind) {
            "credentials found"
        } else {
            "no credentials"
        };
        println!("    {kind}: {status}{marker}");
    }
}

fn config_file_path() -> std::path::PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        config_dir.join("nous").join("config.toml")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".nous")
            .join("config.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_zero_is_always_available() {
        assert!(is_port_available(0));
    }

    #[test]
    fn bound_port_is_not_available() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(!is_port_available(port));
    }
}
