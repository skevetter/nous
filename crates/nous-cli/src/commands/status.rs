use std::fs;
use std::path::PathBuf;

use nous_core::config::{config_file_path, Config};

fn pid_file_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("nous")
        .join("nous.pid")
}

fn process_alive(pid: i32) -> bool {
    // SAFETY: kill(2) with signal 0 performs permission checks without sending a signal.
    // Safe to call with any pid; returns error codes for invalid/missing processes.
    unsafe { libc::kill(pid, 0) == 0 }
}

pub async fn run() {
    let pid_path = pid_file_path();

    let pid_str = match fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("nous daemon: stopped");
            return;
        }
        Err(e) => {
            eprintln!("Error reading PID file: {e}");
            std::process::exit(1);
        }
    };

    let pid: i32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error parsing PID file: {e}");
            let _ = fs::remove_file(&pid_path);
            std::process::exit(1);
        }
    };

    if !process_alive(pid) {
        let _ = fs::remove_file(&pid_path);
        println!("nous daemon: stopped (stale PID file removed)");
        return;
    }

    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: failed to load config: {e}");
            Config::default()
        }
    };
    let config_path = config_file_path();

    println!("nous daemon: running");
    println!("  PID:    {pid}");
    println!("  Port:   {}", config.port);
    println!("  Config: {}", config_path.display());
}
