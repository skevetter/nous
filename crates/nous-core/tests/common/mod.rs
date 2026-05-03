use nous_core::db::DbPools;
use tempfile::TempDir;

pub async fn setup_test_db() -> (DbPools, TempDir) {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();
    (pools, tmp)
}
