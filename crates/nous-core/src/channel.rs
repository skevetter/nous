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
use crate::schedule_db::ScheduleDb;
use crate::types::{
    MemoryPatch, Message, NewMemory, RelationType, Room, RunPatch, Schedule, SchedulePatch,
    ScheduleRun,
};

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
    CreateRoom {
        id: String,
        name: String,
        purpose: Option<String>,
        metadata: Option<String>,
        resp: oneshot::Sender<Result<String>>,
    },
    PostMessage {
        id: String,
        room_id: String,
        sender_id: String,
        content: String,
        reply_to: Option<String>,
        metadata: Option<String>,
        resp: oneshot::Sender<Result<String>>,
    },
    DeleteRoom(String, bool, oneshot::Sender<Result<bool>>),
    ArchiveRoom(String, oneshot::Sender<Result<bool>>),
    JoinRoom {
        room_id: String,
        agent_id: String,
        role: String,
        resp: oneshot::Sender<Result<()>>,
    },
    CreateSchedule(Schedule, oneshot::Sender<Result<String>>),
    UpdateSchedule(String, SchedulePatch, oneshot::Sender<Result<bool>>),
    DeleteSchedule(String, oneshot::Sender<Result<bool>>),
    RecordRun(ScheduleRun, oneshot::Sender<Result<String>>),
    UpdateRun(String, RunPatch, oneshot::Sender<Result<bool>>),
    ComputeNextRun(String, oneshot::Sender<Result<()>>),
    ForceNextRunAt(String, i64, oneshot::Sender<Result<()>>),
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

    pub async fn create_room(
        &self,
        id: String,
        name: String,
        purpose: Option<String>,
        metadata: Option<String>,
    ) -> Result<String> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::CreateRoom {
                id,
                name,
                purpose,
                metadata,
                resp: resp_tx,
            })
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn post_message(
        &self,
        id: String,
        room_id: String,
        sender_id: String,
        content: String,
        reply_to: Option<String>,
        metadata: Option<String>,
    ) -> Result<String> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::PostMessage {
                id,
                room_id,
                sender_id,
                content,
                reply_to,
                metadata,
                resp: resp_tx,
            })
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn delete_room(&self, id: String, hard: bool) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::DeleteRoom(id, hard, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn archive_room(&self, id: String) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::ArchiveRoom(id, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn join_room(&self, room_id: String, agent_id: String, role: String) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::JoinRoom {
                room_id,
                agent_id,
                role,
                resp: resp_tx,
            })
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn create_schedule(&self, schedule: Schedule) -> Result<String> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::CreateSchedule(schedule, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn update_schedule(&self, id: String, patch: SchedulePatch) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::UpdateSchedule(id, patch, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn delete_schedule(&self, id: String) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::DeleteSchedule(id, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn record_run(&self, run: ScheduleRun) -> Result<String> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::RecordRun(run, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn update_run(&self, id: String, patch: RunPatch) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::UpdateRun(id, patch, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn compute_next_run(&self, id: String) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::ComputeNextRun(id, resp_tx))
            .await
            .map_err(|_| nous_shared::NousError::Internal("write channel closed".into()))?;
        resp_rx
            .await
            .map_err(|_| nous_shared::NousError::Internal("response channel dropped".into()))?
    }

    pub async fn force_next_run_at(&self, id: String, ts: i64) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(WriteOp::ForceNextRunAt(id, ts, resp_tx))
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
    let has_table: bool = db
        .connection()
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='schedule_runs'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if has_table {
        let _ = db.connection().execute(
            "UPDATE schedule_runs SET status = 'failed', error = 'process restarted' WHERE status = 'running'",
            [],
        );
    }
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
                    WriteOp::CreateRoom {
                        id,
                        name,
                        purpose,
                        metadata,
                        resp,
                    } => {
                        let result = MemoryDb::create_room_on(
                            &tx,
                            &id,
                            &name,
                            purpose.as_deref(),
                            metadata.as_deref(),
                        );
                        let _ = resp.send(result.map(|_| id));
                    }
                    WriteOp::PostMessage {
                        id,
                        room_id,
                        sender_id,
                        content,
                        reply_to,
                        metadata,
                        resp,
                    } => {
                        let result = MemoryDb::post_message_on(
                            &tx,
                            &id,
                            &room_id,
                            &sender_id,
                            &content,
                            reply_to.as_deref(),
                            metadata.as_deref(),
                        );
                        let _ = resp.send(result.map(|_| id));
                    }
                    WriteOp::DeleteRoom(id, hard, resp) => {
                        let result = if hard {
                            MemoryDb::hard_delete_room_on(&tx, &id)
                        } else {
                            MemoryDb::archive_room_on(&tx, &id)
                        };
                        let _ = resp.send(result);
                    }
                    WriteOp::ArchiveRoom(id, resp) => {
                        let result = MemoryDb::archive_room_on(&tx, &id);
                        let _ = resp.send(result);
                    }
                    WriteOp::JoinRoom {
                        room_id,
                        agent_id,
                        role,
                        resp,
                    } => {
                        let result = MemoryDb::join_room_on(&tx, &room_id, &agent_id, &role);
                        let _ = resp.send(result);
                    }
                    WriteOp::CreateSchedule(schedule, resp) => {
                        let _ = resp.send(ScheduleDb::create_on(&tx, &schedule));
                    }
                    WriteOp::UpdateSchedule(id, patch, resp) => {
                        let _ = resp.send(ScheduleDb::update_on(&tx, &id, &patch));
                    }
                    WriteOp::DeleteSchedule(id, resp) => {
                        let _ = resp.send(ScheduleDb::delete_on(&tx, &id));
                    }
                    WriteOp::RecordRun(run, resp) => {
                        let _ = resp.send(ScheduleDb::record_run_on(&tx, &run));
                    }
                    WriteOp::UpdateRun(id, patch, resp) => {
                        let _ = resp.send(ScheduleDb::update_run_on(&tx, &id, &patch));
                    }
                    WriteOp::ComputeNextRun(id, resp) => {
                        let _ = resp.send(ScheduleDb::compute_next_run_on(&tx, &id));
                    }
                    WriteOp::ForceNextRunAt(id, ts, resp) => {
                        let result = tx
                            .execute(
                                "UPDATE schedules SET next_run_at = ?1 WHERE id = ?2",
                                rusqlite::params![ts, id],
                            )
                            .map(|_| ())
                            .map_err(Into::into);
                        let _ = resp.send(result);
                    }
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await;
    }
}

#[derive(Clone)]
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

    pub async fn list_rooms(&self, archived: bool, limit: Option<usize>) -> Result<Vec<Room>> {
        let limit = limit.unwrap_or(100);
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, purpose, metadata, archived, created_at, updated_at
                 FROM rooms
                 WHERE archived = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![archived as i64, limit as i64], |row| {
                Ok(Room {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    purpose: row.get(2)?,
                    metadata: row.get(3)?,
                    archived: row.get::<_, i64>(4)? != 0,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
        .await
    }

    pub async fn get_room(&self, id: &str) -> Result<Option<Room>> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            match conn.query_row(
                "SELECT id, name, purpose, metadata, archived, created_at, updated_at
                 FROM rooms WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(Room {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        purpose: row.get(2)?,
                        metadata: row.get(3)?,
                        archived: row.get::<_, i64>(4)? != 0,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            ) {
                Ok(r) => Ok(Some(r)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
        .await
    }

    pub async fn get_room_by_name(&self, name: &str) -> Result<Option<Room>> {
        let name = name.to_string();
        self.with_conn(move |conn| {
            match conn.query_row(
                "SELECT id, name, purpose, metadata, archived, created_at, updated_at
                 FROM rooms WHERE name = ?1 AND archived = 0",
                rusqlite::params![name],
                |row| {
                    Ok(Room {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        purpose: row.get(2)?,
                        metadata: row.get(3)?,
                        archived: row.get::<_, i64>(4)? != 0,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            ) {
                Ok(r) => Ok(Some(r)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
        .await
    }

    pub async fn list_messages(
        &self,
        room_id: &str,
        limit: Option<usize>,
        before: Option<String>,
        since: Option<String>,
    ) -> Result<Vec<Message>> {
        let room_id = room_id.to_string();
        let limit = limit.unwrap_or(100);
        self.with_conn(move |conn| {
            let mut sql = "SELECT id, room_id, sender_id, content, reply_to, metadata, created_at
                           FROM room_messages
                           WHERE room_id = ?1"
                .to_string();
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(room_id)];

            if let Some(b) = before {
                sql.push_str(" AND created_at < ?");
                params.push(Box::new(b));
            }
            if let Some(s) = since {
                sql.push_str(" AND created_at > ?");
                params.push(Box::new(s));
            }

            sql.push_str(" ORDER BY created_at DESC LIMIT ?");
            params.push(Box::new(limit as i64));

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                |row| {
                    Ok(Message {
                        id: row.get(0)?,
                        room_id: row.get(1)?,
                        sender_id: row.get(2)?,
                        content: row.get(3)?,
                        reply_to: row.get(4)?,
                        metadata: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                },
            )?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
        .await
    }

    pub async fn search_messages(
        &self,
        room_id: &str,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<Message>> {
        let room_id = room_id.to_string();
        let query = crate::search::sanitize_fts_query(query);
        let limit = limit.unwrap_or(50);
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT m.id, m.room_id, m.sender_id, m.content, m.reply_to, m.metadata, m.created_at
                 FROM room_messages m
                 JOIN room_messages_fts ON m.rowid = room_messages_fts.rowid
                 WHERE room_messages_fts MATCH ?1 AND m.room_id = ?2
                 ORDER BY bm25(room_messages_fts), m.created_at DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(rusqlite::params![query, room_id, limit as i64], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    room_id: row.get(1)?,
                    sender_id: row.get(2)?,
                    content: row.get(3)?,
                    reply_to: row.get(4)?,
                    metadata: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
        })
        .await
    }

    pub async fn room_info(&self, id: &str) -> Result<Option<serde_json::Value>> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            let room = match conn.query_row(
                "SELECT id, name, purpose, metadata, archived, created_at, updated_at
                 FROM rooms WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(Room {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        purpose: row.get(2)?,
                        metadata: row.get(3)?,
                        archived: row.get::<_, i64>(4)? != 0,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            ) {
                Ok(r) => r,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
                Err(e) => return Err(e.into()),
            };

            let mut stmt = conn.prepare(
                "SELECT agent_id, role, joined_at FROM room_participants WHERE room_id = ?1",
            )?;
            let participants: Vec<serde_json::Value> = stmt
                .query_map(rusqlite::params![id], |row| {
                    Ok(serde_json::json!({
                        "agent_id": row.get::<_, String>(0)?,
                        "role": row.get::<_, String>(1)?,
                        "joined_at": row.get::<_, String>(2)?,
                    }))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            let message_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM room_messages WHERE room_id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )?;

            Ok(Some(serde_json::json!({
                "room": room,
                "participants": participants,
                "message_count": message_count,
            })))
        })
        .await
    }
}
