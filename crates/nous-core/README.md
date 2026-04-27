# nous-core

Core library implementing the Nous memory engine. Handles persistent storage of
structured memories in SQLite, local embedding inference via ONNX Runtime,
full-text and semantic search with hybrid fusion, hierarchical category
classification, and a concurrency model built around batched writes and pooled
reads.

## Key Types and APIs

### Memory Database (`db`)

`MemoryDb` wraps a `rusqlite::Connection` and provides the complete memory
lifecycle: `store`, `recall`, `update`, `forget` (soft and hard delete),
`unarchive`, `relate`, and `unrelate`. It also manages categories (`category_add`,
`category_list`, `category_suggest`, `category_delete`, `category_rename`,
`category_update`), chunk storage with vec0 embeddings, access logging, and
database statistics. On first open it loads the sqlite-vec extension, runs
schema migrations, seeds 15 default category hierarchies, and registers
embedding models.

### Embedding (`embed`)

The `EmbeddingBackend` trait defines `embed(&[&str])` and `embed_one(&str)` for
producing normalized float vectors. `OnnxBackend` downloads HuggingFace ONNX models
at runtime, detects encoder vs decoder architecture, applies mean pooling or
last-token pooling with KV-cache support, and L2-normalizes output vectors.
`OnnxBackendBuilder` provides a builder pattern to configure model ID, variant,
and batch size. `FixtureEmbedding` and `MockEmbedding` supply deterministic
vectors for testing.

### Search (`search`)

Three search modes are supported. `search_fts` uses SQLite FTS5 with BM25 ranking
and importance weighting. `search_semantic` performs KNN queries against the vec0
virtual table with batch-loaded memory hydration to avoid N+1 queries. `search`
dispatches to FTS, semantic, or hybrid mode, where hybrid applies Reciprocal Rank
Fusion (`fuse_rrf` with k=60) to merge both result sets. `context` returns
workspace-scoped, importance-ordered, validity-filtered memories.

### Classification (`classify`)

`CategoryClassifier` caches category embeddings and performs two-level hierarchical
classification: it first scores against top-level categories, then refines among
children of the best match. Per-category confidence thresholds control assignment.

### Concurrency (`channel`)

`WriteChannel` wraps an mpsc sender accepting `WriteOp` enums (14 variants covering
all mutation operations). A background worker batches up to 32 operations per
SQLite transaction. Each write op carries a oneshot channel for the caller to
await its result. `ReadPool` manages a semaphore-guarded pool of read-only
connections for concurrent query access.

### Chunking (`chunk`)

`Chunker` splits text into overlapping word-based chunks with configurable size and
overlap parameters. Short trailing chunks are merged into the previous chunk to
avoid fragments.

### Additional Modules

`types` defines the full domain model: `Memory`, `NewMemory`, `MemoryPatch`,
`SearchResult`, `SearchFilters`, `Category`, `Tag`, `Relationship`, `Workspace`,
and enums for `MemoryType`, `Importance`, `Confidence`, and `SearchMode`.
`model` provides embedding model CRUD. `sqlite_vec` loads the vec0 extension
via FFI.

## Dependencies

Depends on `nous-shared` for error types and SQLite helpers, `rusqlite` with the
`bundled-sqlcipher` feature, `ort` for ONNX Runtime, `tokenizers` and `hf-hub` for
HuggingFace model management, `ndarray` for tensor operations, and `tokio` for
async runtime support.
