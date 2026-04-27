use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nous_shared::Result;
use nous_shared::ids::MemoryId;
use nous_shared::sqlite::open_connection;
use nous_shared::sqlite::spawn_blocking;
use rusqlite::Connection;
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};

use crate::chunk::Chunk;
use crate::db::MemoryDb;
use crate::types::{MemoryPatch, NewMemory, RelationType};

const CHANNEL_CAPACITY: usize = 256;
const BATCH_LIMIT: usize = 32;

pub enum WriteOp {
    Store(NewMemory, oneshot::Sender<Result<MemoryId>>),
    Update(MemoryId, MemoryPatch, oneshot::Sender<Result<bool>>),
    Forget(MemoryId, bool, oneshot::Sender<Result<bool>>),
    Relate(
        MemoryId,
        MemoryId,
        RelationType,
        oneshot::Sender<Result<()>>,
    ),
    Unrelate(
        MemoryId,
        MemoryId,
        RelationType,
        oneshot::Sender<Result<bool>>,
    ),
    Unarchive(MemoryId, oneshot::Sender<Result<bool>>),
    CategorySuggest {
        name: String,
        description: Option<String>,
        parent_id: Option<i64>,
        memory_id: MemoryId,
        embedding_blob: Option<Vec<u8>>,
        resp: oneshot::Sender<Result<i64>>,
    },
    CategoryDelete(String, oneshot::Sender<Result<()>>),
    CategoryRename(String, String, oneshot::Sender<Result<()>>),
    CategoryUpdate {
        name: String,
        new_name: Option<String>,
        description: Option<String>,
        threshold: Option<f32>,
        embedding_blob: Option<Vec<u8>>,
        resp: oneshot::Sender<Result<()>>,
    },
    StoreChunks(
        MemoryId,
        Vec<Chunk>,
        Vec<Vec<f32>>,
        oneshot::Sender<Result<()>>,
    ),
    DeleteChunks(MemoryId, oneshot::Sender<Result<()>>),
    LogAccess(MemoryId, String, oneshot::Sender<Result<()>>),
}

#[derive(Clone)]
pub struct WriteChannel {
    tx: mpsc::Sender<WriteOp>,
    batch_count: Arc<AtomicUsize>,
}

impl WriteChannel {
    pub fn new(db: MemoryDb) -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let batch_count = Arc::new(AtomicUsize::new(0));
        let handle = tokio::spawn(write_worker(rx, db, Arc::clone(&batch_count)));
        (Self { tx, batch_count }, handle)
    }

    pub async fn store(&self, memory: NewMemory) -> Result<MemoryId> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::Store(memory, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn update(&self, id: MemoryId, patch: MemoryPatch) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::Update(id, patch, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn forget(&self, id: MemoryId, hard: bool) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::Forget(id, hard, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn relate(&self, src: MemoryId, tgt: MemoryId, rel: RelationType) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::Relate(src, tgt, rel, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn unrelate(&self, src: MemoryId, tgt: MemoryId, rel: RelationType) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::Unrelate(src, tgt, rel, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn category_suggest(
        &self,
        name: String,
        description: Option<String>,
        parent_id: Option<i64>,
        memory_id: MemoryId,
        embedding_blob: Option<Vec<u8>>,
    ) -> Result<i64> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::CategorySuggest {
                name,
                description,
                parent_id,
                memory_id,
                embedding_blob,
                resp: resp_tx,
            })
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn category_delete(&self, name: String) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::CategoryDelete(name, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn category_rename(&self, old_name: String, new_name: String) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::CategoryRename(old_name, new_name, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn category_update(
        &self,
        name: String,
        new_name: Option<String>,
        description: Option<String>,
        threshold: Option<f32>,
        embedding_blob: Option<Vec<u8>>,
    ) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::CategoryUpdate {
                name,
                new_name,
                description,
                threshold,
                embedding_blob,
                resp: resp_tx,
            })
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn unarchive(&self, id: MemoryId) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::Unarchive(id, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn store_chunks(
        &self,
        memory_id: MemoryId,
        chunks: Vec<Chunk>,
        embeddings: Vec<Vec<f32>>,
    ) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::StoreChunks(memory_id, chunks, embeddings, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn delete_chunks(&self, memory_id: MemoryId) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::DeleteChunks(memory_id, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn log_access(&self, memory_id: MemoryId, access_type: String) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::LogAccess(memory_id, access_type, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub fn batch_count(&self) -> usize {
        self.batch_count.load(Ordering::Relaxed)
    }

    pub fn sender(&self) -> &mpsc::Sender<WriteOp> {
        &self.tx
    }
}

async fn write_worker(
    mut rx: mpsc::Receiver<WriteOp>,
    db: MemoryDb,
    batch_count: Arc<AtomicUsize>,
) {
    let db = Arc::new(Mutex::new(db));

    while let Some(first) = rx.recv().await {
        let mut batch = Vec::with_capacity(BATCH_LIMIT);
        batch.push(first);

        while batch.len() < BATCH_LIMIT {
            match rx.try_recv() {
                Ok(op) => batch.push(op),
                Err(_) => break,
            }
        }

        batch_count.fetch_add(1, Ordering::Relaxed);
        let db = Arc::clone(&db);
        let _ = spawn_blocking(move || {
            let db = db.blocking_lock();
            let tx = db.connection().unchecked_transaction()?;
            for op in batch {
                match op {
                    WriteOp::Store(memory, resp) => {
                        let _ = resp.send(MemoryDb::store_on(&tx, &memory));
                    }
                    WriteOp::Update(id, patch, resp) => {
                        let _ = resp.send(MemoryDb::update_on(&tx, &id, &patch));
                    }
                    WriteOp::Forget(id, hard, resp) => {
                        let _ = resp.send(MemoryDb::forget_on(&tx, &id, hard));
                    }
                    WriteOp::Relate(src, tgt, rel, resp) => {
                        let _ = resp.send(MemoryDb::relate_on(&tx, &src, &tgt, rel));
                    }
                    WriteOp::Unrelate(src, tgt, rel, resp) => {
                        let _ = resp.send(MemoryDb::unrelate_on(&tx, &src, &tgt, rel));
                    }
                    WriteOp::CategorySuggest {
                        name,
                        description,
                        parent_id,
                        memory_id,
                        embedding_blob,
                        resp,
                    } => {
                        let result = MemoryDb::category_suggest_on(
                            &tx,
                            &name,
                            description.as_deref(),
                            parent_id,
                            &memory_id,
                        )
                        .and_then(|id| {
                            if let Some(blob) = embedding_blob {
                                tx.execute(
                                    "UPDATE categories SET embedding = ?1 WHERE id = ?2",
                                    rusqlite::params![blob, id],
                                )?;
                            }
                            Ok(id)
                        });
                        let _ = resp.send(result);
                    }
                    WriteOp::CategoryDelete(name, resp) => {
                        let _ = resp.send(MemoryDb::category_delete_on(&tx, &name));
                    }
                    WriteOp::CategoryRename(old_name, new_name, resp) => {
                        let _ = resp.send(MemoryDb::category_rename_on(&tx, &old_name, &new_name));
                    }
                    WriteOp::CategoryUpdate {
                        name,
                        new_name,
                        description,
                        threshold,
                        embedding_blob,
                        resp,
                    } => {
                        let result = MemoryDb::category_update_on(
                            &tx,
                            &name,
                            new_name.as_deref(),
                            description.as_deref(),
                            threshold,
                        )
                        .and_then(|()| {
                            if let Some(blob) = embedding_blob {
                                let effective_name = new_name.as_deref().unwrap_or(&name);
                                tx.execute(
                                    "UPDATE categories SET embedding = ?1 WHERE name = ?2",
                                    rusqlite::params![blob, effective_name],
                                )?;
                            }
                            Ok(())
                        });
                        let _ = resp.send(result);
                    }
                    WriteOp::Unarchive(id, resp) => {
                        let _ = resp.send(MemoryDb::unarchive_on(&tx, &id));
                    }
                    WriteOp::StoreChunks(id, chunks, embeddings, resp) => {
                        let _ =
                            resp.send(MemoryDb::store_chunks_on(&tx, &id, &chunks, &embeddings));
                    }
                    WriteOp::DeleteChunks(id, resp) => {
                        let _ = resp.send(MemoryDb::delete_chunks_on(&tx, &id));
                    }
                    WriteOp::LogAccess(id, access_type, resp) => {
                        let _ = resp.send(MemoryDb::log_access_on(&tx, &id, &access_type));
                    }
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await;
    }
}

pub struct ReadPool {
    connections: Arc<Mutex<Vec<Connection>>>,
    semaphore: Arc<Semaphore>,
}

impl ReadPool {
    pub fn new(path: &str, key: Option<&str>, size: usize) -> Result<Self> {
        let mut connections = Vec::with_capacity(size);
        for _ in 0..size {
            let conn = open_connection(path, key)?;
            conn.execute_batch("PRAGMA query_only = ON")?;
            connections.push(conn);
        }
        Ok(Self {
            connections: Arc::new(Mutex::new(connections)),
            semaphore: Arc::new(Semaphore::new(size)),
        })
    }

    pub async fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| nous_shared::NousError::Internal("semaphore closed".into()))?;

        let conn = {
            let mut conns = self.connections.lock().await;
            conns.pop().ok_or_else(|| {
                nous_shared::NousError::Internal("no connections available".into())
            })?
        };

        let connections = Arc::clone(&self.connections);
        let result = tokio::task::spawn_blocking(move || {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&conn)));
            (conn, res)
        })
        .await;

        match result {
            Ok((conn, res)) => {
                connections.lock().await.push(conn);
                drop(permit);
                match res {
                    Ok(r) => r,
                    Err(_) => Err(nous_shared::NousError::Internal(
                        "closure panicked in ReadPool::with_conn".into(),
                    )),
                }
            }
            Err(e) => {
                drop(permit);
                Err(nous_shared::NousError::Internal(format!(
                    "spawn_blocking failed: {e}"
                )))
            }
        }
    }
}
