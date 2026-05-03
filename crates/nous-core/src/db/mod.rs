mod pool;

pub use pool::{create_vec_pool, DbPools, VecPool, EMBEDDING_DIMENSION};
pub use sea_orm::DatabaseConnection;

pub fn now_utc() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}
