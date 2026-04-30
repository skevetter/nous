use thiserror::Error;

#[derive(Debug, Error)]
pub enum NousError {
    #[error("validation error: {0}")]
    Validation(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] sqlx::Error),

    #[error("internal error: {0}")]
    Internal(String),
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
    }

    #[test]
    fn sqlite_non_busy_is_not_retryable() {
        let err = NousError::Sqlite(sqlx::Error::RowNotFound);
        assert!(!err.is_retryable());
    }
}
