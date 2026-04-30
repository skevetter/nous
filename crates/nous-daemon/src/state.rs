use std::sync::Arc;

use nous_core::notifications::NotificationRegistry;
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub registry: Arc<NotificationRegistry>,
}
