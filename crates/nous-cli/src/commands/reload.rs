use std::fs;
use std::path::PathBuf;

fn pid_file_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("nous")
        .join("nous.pid")
}

pub async fn run() {
    let pid_path = pid_file_path();

    let pid_str = match fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("daemon not running (no PID file)");
            std::process::exit(1);
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
    let ret = unsafe { libc::kill(pid, libc::SIGHUP) };
    if ret != 0 {
        let errno = unsafe { *libc::__errno_location() };
        if errno == libc::ESRCH {
            let _ = fs::remove_file(&pid_path);
            eprintln!("daemon not running (stale PID file removed)");
            std::process::exit(1);
        }
        let err = std::io::Error::from_raw_os_error(errno);
        eprintln!("Error sending SIGHUP to pid {pid}: {err}");
        std::process::exit(1);
    }

    println!("reload signal sent to daemon (pid: {pid})");
}
