use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

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
            eprintln!("daemon not running (no PID file)");
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

    // SAFETY: kill(2) is safe to call with any pid; returns error codes for invalid pids.
    // We send SIGTERM directly and interpret the result to avoid a TOCTOU race.
    let ret = unsafe { libc::kill(pid, libc::SIGTERM) };
    if ret != 0 {
        let errno = unsafe { *libc::__errno_location() };
        if errno == libc::ESRCH {
            let _ = fs::remove_file(&pid_path);
            eprintln!("daemon not running (stale PID file removed)");
            return;
        }
        let err = std::io::Error::from_raw_os_error(errno);
        eprintln!("Error sending SIGTERM to pid {pid}: {err}");
        std::process::exit(1);
    }

    let timeout = Duration::from_secs(5);
    let poll_interval = Duration::from_millis(100);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if !process_alive(pid) {
            let _ = fs::remove_file(&pid_path);
            println!("daemon stopped");
            return;
        }
        thread::sleep(poll_interval);
    }

    eprintln!("daemon did not stop within timeout");
    std::process::exit(1);
}
