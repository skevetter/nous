use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

fn nous_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nous"))
}

fn pid_file_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("nous")
        .join("nous.pid")
}

fn cleanup_daemon(pid_path: &Path) {
    if let Ok(content) = fs::read_to_string(pid_path) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            // SAFETY: kill(2) is safe to call with any pid
            let ret = unsafe { libc::kill(pid, libc::SIGTERM) };
            if ret == 0 {
                for _ in 0..20 {
                    thread::sleep(Duration::from_millis(100));
                    // SAFETY: kill with signal 0 checks process existence
                    if unsafe { libc::kill(pid, 0) } != 0 {
                        break;
                    }
                }
                // Fallback: SIGKILL if still alive
                if unsafe { libc::kill(pid, 0) } == 0 {
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                    thread::sleep(Duration::from_millis(500));
                }
            }
        }
    }
    let _ = fs::remove_file(pid_path);
}

struct DaemonGuard;

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        cleanup_daemon(&pid_file_path());
    }
}

#[test]
fn test_full_daemon_lifecycle() {
    if std::env::var("NOUS_TEST_DAEMON").is_err() {
        return;
    }
    cleanup_daemon(&pid_file_path());
    let _guard = DaemonGuard;

    // 1. Start daemon
    let output = Command::new(nous_bin())
        .args(["serve", "--daemon"])
        .output()
        .expect("failed to execute nous serve --daemon");
    assert!(
        output.status.success(),
        "serve --daemon failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    thread::sleep(Duration::from_secs(3));

    // 2. PID file created
    let pid_path = pid_file_path();
    assert!(
        pid_path.exists(),
        "PID file should exist after daemon start"
    );

    let pid_str = fs::read_to_string(&pid_path).expect("should read PID file");
    let pid: i32 = pid_str
        .trim()
        .parse()
        .expect("PID file should contain a number");
    assert!(pid > 0, "PID should be positive");

    // 3. Status shows running
    let output = Command::new(nous_bin())
        .arg("status")
        .output()
        .expect("failed to execute nous status");
    assert!(output.status.success(), "status command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("running"),
        "status should show 'running', got: {stdout}"
    );

    // 4. Reload succeeds
    let output = Command::new(nous_bin())
        .arg("reload")
        .output()
        .expect("failed to execute nous reload");
    assert!(
        output.status.success(),
        "reload should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify daemon still running after reload
    thread::sleep(Duration::from_millis(500));
    let output = Command::new(nous_bin())
        .arg("status")
        .output()
        .expect("failed to execute nous status");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("running"),
        "daemon should still be running after reload, got: {stdout}"
    );

    // 5. Stop daemon
    let output = Command::new(nous_bin())
        .arg("stop")
        .output()
        .expect("failed to execute nous stop");
    assert!(
        output.status.success(),
        "stop should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    thread::sleep(Duration::from_secs(2));

    // 6. PID file removed
    assert!(
        !pid_file_path().exists(),
        "PID file should be removed after stop"
    );

    // 7. Status shows stopped
    let output = Command::new(nous_bin())
        .arg("status")
        .output()
        .expect("failed to execute nous status");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("stopped"),
        "status should show 'stopped' after stop, got: {stdout}"
    );
}

#[test]
fn test_stop_when_not_running() {
    if std::env::var("NOUS_TEST_DAEMON").is_err() {
        return;
    }
    cleanup_daemon(&pid_file_path());

    let output = Command::new(nous_bin())
        .arg("stop")
        .output()
        .expect("failed to execute nous stop");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not running"),
        "stop when not running should report 'not running', got stderr: {stderr}"
    );
}

#[test]
fn test_reload_when_not_running() {
    if std::env::var("NOUS_TEST_DAEMON").is_err() {
        return;
    }
    cleanup_daemon(&pid_file_path());

    let output = Command::new(nous_bin())
        .arg("reload")
        .output()
        .expect("failed to execute nous reload");

    assert!(
        !output.status.success(),
        "reload when not running should fail"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not running"),
        "reload when not running should report 'not running', got stderr: {stderr}"
    );
}

#[test]
fn test_start_alias() {
    if std::env::var("NOUS_TEST_DAEMON").is_err() {
        return;
    }
    cleanup_daemon(&pid_file_path());
    let _guard = DaemonGuard;

    // `nous start` should work same as `nous serve --daemon`
    let output = Command::new(nous_bin())
        .arg("start")
        .output()
        .expect("failed to execute nous start");
    assert!(
        output.status.success(),
        "start should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    thread::sleep(Duration::from_secs(3));

    // PID file should exist
    let pid_path = pid_file_path();
    assert!(
        pid_path.exists(),
        "PID file should exist after `nous start`"
    );

    // Status should show running
    let output = Command::new(nous_bin())
        .arg("status")
        .output()
        .expect("failed to execute nous status");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("running"),
        "status should show 'running' after start alias, got: {stdout}"
    );
}
