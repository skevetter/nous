use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use nous_core::error::NousError;
use serde::Serialize;

pub struct AppError(NousError);

impl From<NousError> for AppError {
    fn from(err: NousError) -> Self {
        Self(err)
    }
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

impl AppError {
    fn classify_sqlite(err: &sqlx::Error) -> (StatusCode, &'static str, String) {
        match err {
            sqlx::Error::Database(db_err) => {
                let code_cow = db_err.code();
                let code = code_cow.as_deref().unwrap_or("");
                match code {
                    "19" | "2067" | "1555" => (
                        StatusCode::CONFLICT,
                        "constraint_violation",
                        format!("constraint violation: {}", db_err.message()),
                    ),
                    "5" | "6" => (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "database_busy",
                        "database is busy, please retry".to_string(),
                    ),
                    _ => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "database_error",
                        "internal database error".to_string(),
                    ),
                }
            }
            sqlx::Error::RowNotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "record not found".to_string(),
            ),
            sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => (
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
                "database connection unavailable".to_string(),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "database_error",
                "internal database error".to_string(),
            ),
        }
    }

    fn classify_sea_orm(err: &sea_orm::DbErr) -> (StatusCode, &'static str, String) {
        match err {
            sea_orm::DbErr::RecordNotFound(entity) => (
                StatusCode::NOT_FOUND,
                "not_found",
                format!("{entity} not found"),
            ),
            sea_orm::DbErr::RecordNotInserted | sea_orm::DbErr::RecordNotUpdated => (
                StatusCode::CONFLICT,
                "constraint_violation",
                "operation failed due to a constraint".to_string(),
            ),
            sea_orm::DbErr::Exec(runtime_err) | sea_orm::DbErr::Query(runtime_err) => {
                let msg = runtime_err.to_string();
                if msg.contains("UNIQUE constraint failed")
                    || msg.contains("FOREIGN KEY constraint")
                    || msg.contains("NOT NULL constraint")
                {
                    (
                        StatusCode::CONFLICT,
                        "constraint_violation",
                        format!("constraint violation: {msg}"),
                    )
                } else if msg.contains("database is locked") || msg.contains("SQLITE_BUSY") {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "database_busy",
                        "database is busy, please retry".to_string(),
                    )
                } else {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "database_error",
                        "internal database error".to_string(),
                    )
                }
            }
            sea_orm::DbErr::Conn(runtime_err) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
                format!("database connection error: {runtime_err}"),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "database_error",
                "internal database error".to_string(),
            ),
        }
    }

    fn code_and_status(&self) -> (StatusCode, &'static str, String) {
        match &self.0 {
            NousError::Validation(msg) => {
                (StatusCode::BAD_REQUEST, "validation_error", msg.clone())
            }
            NousError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.clone()),
            NousError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.clone()),
            NousError::Config(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "config_error", msg.clone())
            }
            NousError::Sqlite(err) => Self::classify_sqlite(err),
            NousError::SeaOrm(err) => Self::classify_sea_orm(err),
            NousError::CyclicLink(msg) => (StatusCode::CONFLICT, "cyclic_link", msg.clone()),
            NousError::NoLinkedRoom(msg) => {
                (StatusCode::BAD_REQUEST, "no_linked_room", msg.clone())
            }
            NousError::Internal(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg.clone())
            }
            NousError::Unavailable(msg) => {
                (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable", msg.clone())
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.code_and_status();
        let body = ErrorEnvelope {
            error: ErrorBody { code, message },
        };
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use sqlx::error::DatabaseError;

    #[derive(Debug)]
    struct FakeDbError {
        code: Option<String>,
        message: String,
    }

    impl std::fmt::Display for FakeDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.message)
        }
    }

    impl std::error::Error for FakeDbError {}

    impl DatabaseError for FakeDbError {
        fn message(&self) -> &str {
            &self.message
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

    fn make_app_error(err: NousError) -> AppError {
        AppError(err)
    }

    #[test]
    fn sqlite_constraint_violation_returns_conflict() {
        let db_err = Box::new(FakeDbError {
            code: Some("19".to_string()),
            message: "UNIQUE constraint failed: rooms.name".to_string(),
        });
        let app_err = make_app_error(NousError::Sqlite(sqlx::Error::Database(db_err)));
        let (status, code, _) = app_err.code_and_status();
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(code, "constraint_violation");
    }

    #[test]
    fn sqlite_unique_constraint_extended_code_returns_conflict() {
        let db_err = Box::new(FakeDbError {
            code: Some("2067".to_string()),
            message: "UNIQUE constraint failed".to_string(),
        });
        let app_err = make_app_error(NousError::Sqlite(sqlx::Error::Database(db_err)));
        let (status, code, _) = app_err.code_and_status();
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(code, "constraint_violation");
    }

    #[test]
    fn sqlite_busy_returns_service_unavailable() {
        let db_err = Box::new(FakeDbError {
            code: Some("5".to_string()),
            message: "database is locked".to_string(),
        });
        let app_err = make_app_error(NousError::Sqlite(sqlx::Error::Database(db_err)));
        let (status, code, msg) = app_err.code_and_status();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(code, "database_busy");
        assert!(msg.contains("retry"));
    }

    #[test]
    fn sqlite_locked_returns_service_unavailable() {
        let db_err = Box::new(FakeDbError {
            code: Some("6".to_string()),
            message: "database table is locked".to_string(),
        });
        let app_err = make_app_error(NousError::Sqlite(sqlx::Error::Database(db_err)));
        let (status, code, _) = app_err.code_and_status();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(code, "database_busy");
    }

    #[test]
    fn sqlite_row_not_found_returns_not_found() {
        let app_err = make_app_error(NousError::Sqlite(sqlx::Error::RowNotFound));
        let (status, code, _) = app_err.code_and_status();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(code, "not_found");
    }

    #[test]
    fn sqlite_pool_timeout_returns_unavailable() {
        let app_err = make_app_error(NousError::Sqlite(sqlx::Error::PoolTimedOut));
        let (status, code, _) = app_err.code_and_status();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(code, "database_unavailable");
    }

    #[test]
    fn sqlite_unknown_code_returns_internal_error() {
        let db_err = Box::new(FakeDbError {
            code: Some("99".to_string()),
            message: "something unexpected".to_string(),
        });
        let app_err = make_app_error(NousError::Sqlite(sqlx::Error::Database(db_err)));
        let (status, code, _) = app_err.code_and_status();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(code, "database_error");
    }

    #[test]
    fn sea_orm_record_not_found_returns_not_found() {
        let app_err =
            make_app_error(NousError::SeaOrm(sea_orm::DbErr::RecordNotFound("agent".into())));
        let (status, code, msg) = app_err.code_and_status();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(code, "not_found");
        assert!(msg.contains("agent"));
    }

    #[test]
    fn sea_orm_record_not_inserted_returns_conflict() {
        let app_err = make_app_error(NousError::SeaOrm(sea_orm::DbErr::RecordNotInserted));
        let (status, code, _) = app_err.code_and_status();
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(code, "constraint_violation");
    }

    #[test]
    fn sea_orm_exec_unique_constraint_returns_conflict() {
        let app_err = make_app_error(NousError::SeaOrm(sea_orm::DbErr::Exec(
            sea_orm::RuntimeErr::Internal("UNIQUE constraint failed: rooms.name".into()),
        )));
        let (status, code, msg) = app_err.code_and_status();
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(code, "constraint_violation");
        assert!(msg.contains("UNIQUE constraint"));
    }

    #[test]
    fn sea_orm_exec_busy_returns_service_unavailable() {
        let app_err = make_app_error(NousError::SeaOrm(sea_orm::DbErr::Exec(
            sea_orm::RuntimeErr::Internal("database is locked".into()),
        )));
        let (status, code, msg) = app_err.code_and_status();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(code, "database_busy");
        assert!(msg.contains("retry"));
    }

    #[test]
    fn sea_orm_conn_error_returns_unavailable() {
        let app_err = make_app_error(NousError::SeaOrm(sea_orm::DbErr::Conn(
            sea_orm::RuntimeErr::Internal("connection refused".into()),
        )));
        let (status, code, _) = app_err.code_and_status();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(code, "database_unavailable");
    }
}
