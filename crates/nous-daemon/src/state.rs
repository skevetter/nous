use std::sync::Arc;

use nous_core::db::VecPool;
use nous_core::memory::Embedder;
use nous_core::notifications::NotificationRegistry;
use sqlx::SqlitePool;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::process_manager::ProcessRegistry;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub vec_pool: VecPool,
    pub registry: Arc<NotificationRegistry>,
    pub embedder: Option<Arc<dyn Embedder>>,
    pub schedule_notify: Arc<Notify>,
    pub shutdown: CancellationToken,
    pub process_registry: Arc<ProcessRegistry>,
}
