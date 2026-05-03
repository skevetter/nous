use thiserror::Error;

#[derive(Debug, Error)]
pub enum NousError {
    #[error("validation error: {0}")]
    Validation(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] sqlx::Error),

    #[error("sea-orm error: {0}")]
    SeaOrm(#[from] sea_orm::DbErr),

    #[error("cyclic link detected: {0}")]
    CyclicLink(String),

    #[error("no linked room: {0}")]
    NoLinkedRoom(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("service unavailable: {0}")]
    Unavailable(String),
}

impl NousError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            NousError::Sqlite(sqlx::Error::Database(db_err))
                if db_err.code().as_deref() == Some("5")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::error::DatabaseError;

    #[derive(Debug)]
    struct FakeDbError {
        code: Option<String>,
    }

    impl std::fmt::Display for FakeDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "fake db error")
        }
    }

    impl std::error::Error for FakeDbError {}

    impl DatabaseError for FakeDbError {
        fn message(&self) -> &str {
            "fake db error"
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            self.code.as_deref().map(std::borrow::Cow::Borrowed)
        }
    }

    #[test]
    fn sqlite_busy_is_retryable() {
        let db_err = Box::new(FakeDbError {
            code: Some("5".to_string()),
        });
        let err = NousError::Sqlite(sqlx::Error::Database(db_err));
        assert!(err.is_retryable());
    }

    #[test]
    fn sqlite_other_code_is_not_retryable() {
        let db_err = Box::new(FakeDbError {
            code: Some("19".to_string()),
        });
        let err = NousError::Sqlite(sqlx::Error::Database(db_err));
        assert!(!err.is_retryable());
    }

    #[test]
    fn non_sqlite_errors_are_not_retryable() {
        let err = NousError::Validation("bad input".into());
        assert!(!err.is_retryable());

        let err = NousError::NotFound("missing".into());
        assert!(!err.is_retryable());

        let err = NousError::Conflict("duplicate".into());
        assert!(!err.is_retryable());

        let err = NousError::Internal("boom".into());
        assert!(!err.is_retryable());

        let err = NousError::Config("bad toml".into());
        assert!(!err.is_retryable());

        let err = NousError::Unavailable("LLM not configured".into());
        assert!(!err.is_retryable());
    }

    #[test]
    fn sqlite_non_busy_is_not_retryable() {
        let err = NousError::Sqlite(sqlx::Error::RowNotFound);
        assert!(!err.is_retryable());
    }
}
