use std::path::PathBuf;

use crate::error::{NousError, Result};

pub fn cache_dir() -> Result<PathBuf> {
    resolve_dir("NOUS_CACHE_DIR", "XDG_CACHE_HOME", ".cache")
}

pub fn config_dir() -> Result<PathBuf> {
    resolve_dir("NOUS_CONFIG_DIR", "XDG_CONFIG_HOME", ".config")
}

pub fn db_path(name: &str) -> Result<PathBuf> {
    Ok(cache_dir()?.join(name))
}

pub fn config_path(name: &str) -> Result<PathBuf> {
    Ok(config_dir()?.join(name))
}

fn resolve_dir(app_var: &str, xdg_var: &str, default_subdir: &str) -> Result<PathBuf> {
    let path = if let Ok(v) = std::env::var(app_var) {
        PathBuf::from(v)
    } else if let Ok(v) = std::env::var(xdg_var) {
        PathBuf::from(v).join("nous")
    } else {
        let home = std::env::var("HOME").map_err(|_| {
            NousError::Config("HOME is not set and no override env vars provided".into())
        })?;
        PathBuf::from(home).join(default_subdir).join("nous")
    };

    std::fs::create_dir_all(&path)?;
    Ok(path)
}
