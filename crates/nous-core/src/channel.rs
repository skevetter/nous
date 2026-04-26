use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nous_shared::Result;
use nous_shared::ids::MemoryId;
use nous_shared::sqlite::{open_connection, spawn_blocking};
use rusqlite::Connection;
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};

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
            for op in batch {
                match op {
                    WriteOp::Store(memory, resp) => {
                        let _ = resp.send(db.store(&memory));
                    }
                    WriteOp::Update(id, patch, resp) => {
                        let _ = resp.send(db.update(&id, &patch));
                    }
                    WriteOp::Forget(id, hard, resp) => {
                        let _ = resp.send(db.forget(&id, hard));
                    }
                    WriteOp::Relate(src, tgt, rel, resp) => {
                        let _ = resp.send(db.relate(&src, &tgt, rel));
                    }
                }
            }
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
        let result = spawn_blocking(move || {
            let result = f(&conn);
            Ok((conn, result))
        })
        .await;

        match result {
            Ok((conn, result)) => {
                connections.lock().await.push(conn);
                drop(permit);
                result
            }
            Err(e) => {
                drop(permit);
                Err(e)
            }
        }
    }
}
