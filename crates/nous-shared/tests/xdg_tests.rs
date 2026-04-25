use std::sync::Mutex;

use nous_shared::xdg;

static ENV_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn cache_dir_respects_nous_cache_dir() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let expected = tmp.path().join("custom-cache");

    unsafe {
        std::env::set_var("NOUS_CACHE_DIR", &expected);
        std::env::remove_var("XDG_CACHE_HOME");
    }

    let result = xdg::cache_dir().unwrap();
    assert_eq!(result, expected);
    assert!(expected.is_dir());

    unsafe {
        std::env::remove_var("NOUS_CACHE_DIR");
    }
}

#[test]
fn config_dir_respects_nous_config_dir() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let expected = tmp.path().join("custom-config");

    unsafe {
        std::env::set_var("NOUS_CONFIG_DIR", &expected);
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    let result = xdg::config_dir().unwrap();
    assert_eq!(result, expected);
    assert!(expected.is_dir());

    unsafe {
        std::env::remove_var("NOUS_CONFIG_DIR");
    }
}

#[test]
fn cache_dir_falls_back_to_xdg() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let xdg_cache = tmp.path().join("xdg-cache");

    unsafe {
        std::env::remove_var("NOUS_CACHE_DIR");
        std::env::set_var("XDG_CACHE_HOME", &xdg_cache);
    }

    let result = xdg::cache_dir().unwrap();
    assert_eq!(result, xdg_cache.join("nous"));
    assert!(xdg_cache.join("nous").is_dir());

    unsafe {
        std::env::remove_var("XDG_CACHE_HOME");
    }
}

#[test]
fn config_dir_falls_back_to_xdg() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let xdg_config = tmp.path().join("xdg-config");

    unsafe {
        std::env::remove_var("NOUS_CONFIG_DIR");
        std::env::set_var("XDG_CONFIG_HOME", &xdg_config);
    }

    let result = xdg::config_dir().unwrap();
    assert_eq!(result, xdg_config.join("nous"));
    assert!(xdg_config.join("nous").is_dir());

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
fn cache_dir_falls_back_to_home() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("fakehome");
    std::fs::create_dir_all(&home).unwrap();

    unsafe {
        std::env::remove_var("NOUS_CACHE_DIR");
        std::env::remove_var("XDG_CACHE_HOME");
        std::env::set_var("HOME", &home);
    }

    let result = xdg::cache_dir().unwrap();
    assert_eq!(result, home.join(".cache").join("nous"));
    assert!(home.join(".cache").join("nous").is_dir());

    unsafe {
        std::env::remove_var("HOME");
    }
}

#[test]
fn config_dir_falls_back_to_home() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("fakehome");
    std::fs::create_dir_all(&home).unwrap();

    unsafe {
        std::env::remove_var("NOUS_CONFIG_DIR");
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::set_var("HOME", &home);
    }

    let result = xdg::config_dir().unwrap();
    assert_eq!(result, home.join(".config").join("nous"));
    assert!(home.join(".config").join("nous").is_dir());

    unsafe {
        std::env::remove_var("HOME");
    }
}

#[test]
fn db_path_joins_on_cache_dir() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("db-cache");

    unsafe {
        std::env::set_var("NOUS_CACHE_DIR", &cache);
    }

    let result = xdg::db_path("test.db").unwrap();
    assert_eq!(result, cache.join("test.db"));

    unsafe {
        std::env::remove_var("NOUS_CACHE_DIR");
    }
}

#[test]
fn config_path_joins_on_config_dir() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("cfg-path");

    unsafe {
        std::env::set_var("NOUS_CONFIG_DIR", &config);
    }

    let result = xdg::config_path("config.toml").unwrap();
    assert_eq!(result, config.join("config.toml"));

    unsafe {
        std::env::remove_var("NOUS_CONFIG_DIR");
    }
}

#[test]
fn returns_error_when_no_home() {
    let _lock = ENV_MUTEX.lock().unwrap();

    unsafe {
        std::env::remove_var("NOUS_CACHE_DIR");
        std::env::remove_var("XDG_CACHE_HOME");
        std::env::remove_var("NOUS_CONFIG_DIR");
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HOME");
    }

    let cache_result = xdg::cache_dir();
    assert!(cache_result.is_err());
    let err = format!("{}", cache_result.unwrap_err());
    assert!(err.contains("HOME"), "error should mention HOME: {err}");

    let config_result = xdg::config_dir();
    assert!(config_result.is_err());

    unsafe {
        std::env::set_var("HOME", std::env::temp_dir());
    }
}
