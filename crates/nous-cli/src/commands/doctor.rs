use std::net::TcpListener;
use std::process;

use nous_core::config::Config;
use nous_core::db::DbPools;

const DAEMON_PORT: u16 = 8377;

pub async fn run() {
    println!("nous doctor v{}", env!("CARGO_PKG_VERSION"));
    println!("==================");

    let mut has_failure = false;

    let data_dir = match check_config() {
        Ok(config) => Some(config.data_dir),
        Err(()) => {
            has_failure = true;
            None
        }
    };

    if let Some(ref dir) = data_dir {
        if !check_storage(dir) {
            has_failure = true;
        }
    } else {
        println!("[FAIL] Storage directory check skipped (config failed)");
        has_failure = true;
    }

    if let Some(ref dir) = data_dir {
        if !check_database(dir).await {
            has_failure = true;
        }
    } else {
        println!("[FAIL] Database check skipped (config failed)");
        has_failure = true;
    }

    check_port(DAEMON_PORT);

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

    let migration_ok = match pools.run_migrations().await {
        Ok(()) => {
            println!("[OK] Migrations up to date");
            true
        }
        Err(e) => {
            println!("[FAIL] Migration failed: {e}");
            false
        }
    };

    pools.close().await;
    migration_ok
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
