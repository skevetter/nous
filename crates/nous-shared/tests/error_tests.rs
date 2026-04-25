use nous_shared::{NousError, Result};

#[test]
fn sqlite_variant_display() {
    let inner = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
        Some("test".to_string()),
    );
    let err = NousError::from(inner);
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("sqlite"),
        "expected 'sqlite' in: {msg}"
    );
}

#[test]
fn io_variant_display() {
    let inner = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
    let err = NousError::from(inner);
    let msg = err.to_string();
    assert!(msg.to_lowercase().contains("io"), "expected 'io' in: {msg}");
}

#[test]
fn config_variant_display() {
    let err = NousError::Config("bad toml".into());
    let msg = err.to_string();
    assert!(msg.contains("bad toml"), "expected 'bad toml' in: {msg}");
}

#[test]
fn encryption_variant_display() {
    let err = NousError::Encryption("key expired".into());
    let msg = err.to_string();
    assert!(
        msg.contains("key expired"),
        "expected 'key expired' in: {msg}"
    );
}

#[test]
fn embedding_variant_display() {
    let err = NousError::Embedding("model not found".into());
    let msg = err.to_string();
    assert!(
        msg.contains("model not found"),
        "expected 'model not found' in: {msg}"
    );
}

#[test]
fn from_rusqlite_error() {
    let inner = rusqlite::Error::InvalidQuery;
    let err: NousError = inner.into();
    assert!(matches!(err, NousError::Sqlite(_)));
}

#[test]
fn from_io_error() {
    let inner = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    let err: NousError = inner.into();
    assert!(matches!(err, NousError::Io(_)));
}

#[test]
fn result_type_alias_compiles() {
    fn returns_ok() -> Result<u32> {
        Ok(42)
    }
    fn returns_err() -> Result<u32> {
        Err(NousError::Config("oops".into()))
    }
    assert_eq!(returns_ok().unwrap(), 42);
    assert!(returns_err().is_err());
}

#[test]
fn send_sync_static() {
    fn assert_send_sync_static<T: Send + Sync + 'static>() {}
    assert_send_sync_static::<NousError>();
}
