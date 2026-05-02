# Vector DB & Embeddings Support

Extends the existing memory system to support multiple embedding providers and
vector store backends through rig's embedding and vector store abstractions.
CEO directive: "vector db support for additional providers through rig and
embedding models."

---

## 1. Current State

### Memory system: SQLite FTS5 + sqlite-vec

The memory system in `crates/nous-core/src/memory/` provides:

- **Structured memories** (decisions, conventions, bugfixes, architecture, facts,
  observations) stored in a `memories` table with full-text search via FTS5.
- **Vector embeddings** stored in a `memory_embeddings` vec0 virtual table
  (sqlite-vec) in a separate `memory-vec.db` file. KNN queries return top-K
  results sorted by L2 distance in O(log N + K).
- **Hybrid search** (`search_hybrid_filtered`) combining FTS5 BM25 ranking with
  vec0 KNN results via Reciprocal Rank Fusion (RRF).
- **Chunking** (`memory/chunk.rs`) splits long documents into overlapping
  windows; each chunk gets its own embedding.
- **Reranking** (`memory/rerank.rs`) merges FTS and vector results with RRF.

### Embedding generation: local ONNX (all-MiniLM-L6-v2)

`crates/nous-core/src/memory/embed.rs` defines:

```rust
pub trait Embedder: Send + Sync {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, NousError>;
    fn dimension(&self) -> usize;
}
```

Two implementations:

| Impl | Purpose | Dimension |
|------|---------|-----------|
| `OnnxEmbeddingModel` | Production; runs all-MiniLM-L6-v2 via `ort` | 384 |
| `MockEmbedder` | Tests; deterministic hash-based vectors | 384 |

The ONNX model is loaded from `~/.nous/models/all-MiniLM-L6-v2.onnx` and
generates embeddings locally with no network calls.

### Database pools

`crates/nous-core/src/db/pool.rs` defines:

```rust
pub const EMBEDDING_DIMENSION: usize = 384;
pub type VecPool = Arc<Mutex<Connection>>;

pub struct DbPools {
    pub fts: SqlitePool,
    pub vec: VecPool,
}
```

The vec pool uses rusqlite with the sqlite-vec extension. The FTS pool uses SQLx
async. They share no connection and communicate via memory IDs.

### LLM providers (already multi-provider via rig)

The daemon already supports Bedrock, Anthropic, and OpenAI for completions via
`rig-bedrock` 0.4.5 and `rig-core` 0.36. Embedding support has not yet been
wired through rig -- the platform only uses the local ONNX model.

---

## 2. Rig Embedding Providers

### EmbeddingModel trait

rig-core 0.36 defines (`src/embeddings/embedding.rs`):

```rust
pub trait EmbeddingModel: Send + Sync {
    const MAX_DOCUMENTS: usize;
    type Client;

    fn make(client: &Self::Client, model: impl Into<String>, dims: Option<usize>) -> Self;
    fn ndims(&self) -> usize;
    fn embed_texts(
        &self,
        texts: impl IntoIterator<Item = String> + Send,
    ) -> impl Future<Output = Result<Vec<Embedding>, EmbeddingError>> + Send;
    fn embed_text(&self, text: &str) -> impl Future<Output = Result<Embedding, EmbeddingError>> + Send;
}
```

Key details:
- Vectors are `Vec<f64>` (not f32 like our current system).
- `Embedding` struct holds both the source `document: String` and `vec: Vec<f64>`.
- Batch embedding is supported (up to `MAX_DOCUMENTS` per request).

### EmbeddingsClient trait

Provider clients implement `EmbeddingsClient`:

```rust
pub trait EmbeddingsClient {
    type EmbeddingModel: EmbeddingModel;
    fn embedding_model(&self, model: impl Into<String>) -> Self::EmbeddingModel;
    fn embedding_model_with_ndims(&self, model: impl Into<String>, ndims: usize) -> Self::EmbeddingModel;
}
```

### Available embedding providers in rig

| Provider | Crate | Models | Dimensions |
|----------|-------|--------|------------|
| **OpenAI** | `rig-core` (built-in) | `text-embedding-3-large` (3072), `text-embedding-3-small` (1536), `text-embedding-ada-002` (1536) | Configurable |
| **AWS Bedrock** | `rig-bedrock` 0.4.5 | `amazon.titan-embed-text-v2:0`, `amazon.titan-embed-text-v1`, `cohere.embed-english-v3`, `cohere.embed-multilingual-v3` | 256/512/1024 (Titan v2), 1024 (Cohere) |
| **Cohere** | `rig-core` (built-in) | `embed-english-v3.0`, `embed-multilingual-v3.0` | 1024 |
| **Gemini** | `rig-core` (built-in) | `text-embedding-004` | 768 |
| **Mistral** | `rig-core` (built-in) | `mistral-embed` | 1024 |
| **Together** | `rig-core` (built-in) | Various open models | Varies |
| **OpenRouter** | `rig-core` (built-in) | Various models via routing | Varies |
| **FastEmbed (local)** | `rig-fastembed` 0.4.0 | ONNX models (all-MiniLM-L6-v2, etc.) | 384+ |
| **TEI (local)** | `rig-tei` 0.1.5 | HuggingFace Text Embedding Inference server | Varies |

### Embed trait for documents

rig defines a `#[derive(Embed)]` macro that marks which fields of a struct
should be embedded. The `EmbeddingsBuilder` collects documents, generates
embeddings via the model, and returns `Vec<(Doc, OneOrMany<Embedding>)>`.

---

## 3. Rig Vector Store Providers

### VectorStoreIndex trait

rig-core 0.36 defines (`src/vector_store/mod.rs`):

```rust
pub trait VectorStoreIndex: Send + Sync {
    type Filter: SearchFilter + Send + Sync;

    fn top_n<T: for<'a> Deserialize<'a> + Send>(
        &self,
        req: VectorSearchRequest<Self::Filter>,
    ) -> impl Future<Output = Result<Vec<(f64, String, T)>, VectorStoreError>> + Send;

    fn top_n_ids(
        &self,
        req: VectorSearchRequest<Self::Filter>,
    ) -> impl Future<Output = Result<Vec<(f64, String)>, VectorStoreError>> + Send;
}
```

Any `VectorStoreIndex` automatically implements rig's `Tool` trait, making it
directly usable as a dynamic context source for RAG agents.

### InsertDocuments trait

```rust
pub trait InsertDocuments: Send + Sync {
    fn insert_documents<Doc: Serialize + Embed + Send>(
        &self,
        documents: Vec<(Doc, OneOrMany<Embedding>)>,
    ) -> impl Future<Output = Result<(), VectorStoreError>> + Send;
}
```

### VectorSearchRequest

Queries include: `query` (text to embed), `samples` (max results), `threshold`
(minimum similarity), `filter` (backend-specific metadata filtering), and
`additional_params` (backend-specific JSON).

### Available vector store backends

| Backend | Crate | Maturity | Notes |
|---------|-------|----------|-------|
| **In-Memory** | `rig-core` (built-in) | Stable | BruteForce or LSH index; good for testing/small datasets |
| **SQLite (sqlite-vec)** | `rig-sqlite` 0.2.5 | Stable | Uses sqlite-vec extension; zero-dependency local store |
| **Qdrant** | `rig-qdrant` 0.2.5 | Stable | Production-grade vector DB; HNSW index; metadata filtering |
| **LanceDB** | `rig-lancedb` 0.4.5 | Stable | Columnar; embedded or serverless; good for large datasets |
| **PostgreSQL (pgvector)** | `rig-postgres` 0.2.5 | Stable | If you already have Postgres |
| **MongoDB** | `rig-mongodb` 0.4.5 | Stable | Atlas Vector Search |
| **Milvus** | `rig-milvus` 0.2.5 | Stable | Distributed vector DB |
| **Neo4j** | `rig-neo4j` 0.5.5 | Stable | Graph + vector hybrid |
| **SurrealDB** | `rig-surrealdb` 0.2.5 | Stable | Multi-model DB |
| **S3 Vectors** | `rig-s3vectors` 0.2.5 | New | AWS-native, serverless |
| **Cloudflare Vectorize** | `rig-vectorize` 0.2.5 | Stable | Edge-native |
| **HelixDB** | `rig-helixdb` 0.2.5 | Stable | Specialized vector DB |
| **ScyllaDB** | `rig-scylladb` 0.2.5 | New | High-throughput NoSQL |

### How RAG works in rig

The pattern from `rig-bedrock/examples/rag_with_bedrock.rs`:

```rust
let embedding_model = client.embedding_model_with_ndims(AMAZON_TITAN_EMBED_TEXT_V2_0, 256);
let embeddings = EmbeddingsBuilder::new(embedding_model.clone())
    .documents(docs)?
    .build()
    .await?;
let vector_store = InMemoryVectorStore::from_documents(embeddings);
let index = vector_store.index(embedding_model);
let rag_agent = client.agent(MODEL)
    .preamble("...")
    .dynamic_context(1, index)  // <-- vector store as dynamic context
    .build();
let response = rag_agent.prompt("query").await?;
```

The agent automatically queries the vector store index before each prompt,
injecting relevant documents into the context window.

---

## 4. Target State

### Embedding providers (initial support)

| Provider | Rationale | Auth |
|----------|-----------|------|
| **Local ONNX** (existing) | Zero-cost, no credentials, works offline | None |
| **AWS Bedrock Titan** | Nous already has Bedrock client; AWS users get cloud embeddings without new credentials | AWS SSO / env creds |
| **OpenAI** | Most popular; high-quality; simple API key | `OPENAI_API_KEY` |

Future additions (Cohere, Gemini, FastEmbed, TEI) follow the same adapter pattern.

### Vector store backends (initial support)

| Backend | Rationale | When to use |
|---------|-----------|-------------|
| **sqlite-vec** (existing) | Zero-dependency local; already integrated | Default for all local/dev use |
| **Qdrant** (new, optional) | Production-grade; horizontal scaling; metadata filtering | Production deployments with large memory corpora |

Future additions (LanceDB, PostgreSQL, S3 Vectors) follow the same adapter pattern.

### Relationship to existing FTS5 search

The existing FTS5 full-text search is **complemented, not replaced**:

- FTS5 remains the primary path for keyword/phrase search.
- Vector search provides semantic similarity for queries where exact terms differ.
- Hybrid search (RRF) already merges both -- this design extends it to use
  cloud embedding providers and external vector stores.
- The existing `search_hybrid_filtered` function becomes the unified entry point.

### Configuration model

Users configure embedding provider and vector store via the same layered
resolution pattern (CLI > env > config.toml > defaults):

```toml
# ~/.config/nous/config.toml

[embedding]
provider = "local"          # "local" | "bedrock" | "openai"
model = "all-MiniLM-L6-v2" # provider-specific model name
dimensions = 384            # embedding dimension (provider-dependent)

[vector_store]
backend = "sqlite-vec"      # "sqlite-vec" | "qdrant"

[vector_store.qdrant]       # only needed if backend = "qdrant"
url = "http://localhost:6334"
collection = "nous_memories"
api_key = ""                # optional; can also use QDRANT_API_KEY env var
```

Default configuration requires zero setup (local ONNX + sqlite-vec).

---

## 5. Architecture

### New types

```rust
// crates/nous-core/src/memory/embed.rs (extend existing file)

/// Embedding provider selection. Drives which backend generates vectors.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingProvider {
    #[default]
    Local,      // OnnxEmbeddingModel (existing)
    Bedrock,    // rig_bedrock::embedding::EmbeddingModel
    OpenAi,     // rig::providers::openai::embedding::EmbeddingModel
}

/// Configuration for the embedding subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    pub provider: EmbeddingProvider,
    pub model: String,
    pub dimensions: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: EmbeddingProvider::Local,
            model: "all-MiniLM-L6-v2".to_string(),
            dimensions: 384,
        }
    }
}
```

```rust
// crates/nous-core/src/memory/vector_store.rs (new file)

/// Vector store backend selection.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum VectorStoreBackend {
    #[default]
    SqliteVec,  // existing sqlite-vec via rusqlite
    Qdrant,     // external Qdrant instance
}

/// Configuration for the vector store subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorStoreConfig {
    pub backend: VectorStoreBackend,
    pub qdrant: Option<QdrantConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantConfig {
    pub url: String,
    pub collection: String,
    pub api_key: Option<String>,
}

impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            backend: VectorStoreBackend::SqliteVec,
            qdrant: None,
        }
    }
}
```

### Unified embedder abstraction (bridging local + rig)

The existing `Embedder` trait uses `Vec<f32>`. Rig uses `Vec<f64>`. We need an
adapter layer:

```rust
// crates/nous-core/src/memory/embed.rs (extend)

/// Wraps rig's async EmbeddingModel trait into our synchronous Embedder trait.
/// Converts f64 rig vectors to f32 for sqlite-vec storage.
pub struct RigEmbedderAdapter<M: rig::embeddings::EmbeddingModel> {
    model: M,
    rt: tokio::runtime::Handle,
}

impl<M: rig::embeddings::EmbeddingModel> Embedder for RigEmbedderAdapter<M> {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, NousError> {
        let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
        let embeddings = self.rt.block_on(self.model.embed_texts(owned))
            .map_err(|e| NousError::Internal(format!("embedding failed: {e}")))?;
        Ok(embeddings.into_iter()
            .map(|e| e.vec.into_iter().map(|v| v as f32).collect())
            .collect())
    }

    fn dimension(&self) -> usize {
        self.model.ndims()
    }
}
```

For async callers (daemon invocations), we provide an async variant:

```rust
pub trait AsyncEmbedder: Send + Sync {
    async fn embed_async(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, NousError>;
    fn dimension(&self) -> usize;
}
```

### Unified vector store abstraction (bridging sqlite-vec + rig)

For the Qdrant backend, we use `rig-qdrant` directly:

```rust
// crates/nous-daemon/src/vector_store.rs (new file)

use rig::vector_store::VectorStoreIndex;

pub enum VectorStore {
    SqliteVec(crate::db::VecPool),
    Qdrant(rig_qdrant::QdrantVectorStore),
}
```

The sqlite-vec path continues to use the existing direct rusqlite queries.
The Qdrant path delegates to rig-qdrant's `VectorStoreIndex` implementation.

### Crate structure

```
crates/nous-core/src/memory/
  mod.rs           -- existing: Memory types, FTS operations, hybrid search
  embed.rs         -- extended: EmbeddingProvider, EmbeddingConfig, RigEmbedderAdapter
  chunk.rs         -- existing: Chunker (unchanged)
  rerank.rs        -- existing: RRF (unchanged)
  vector_store.rs  -- NEW: VectorStoreBackend, VectorStoreConfig, QdrantConfig
  analytics.rs     -- existing (unchanged)

crates/nous-daemon/src/
  llm_client.rs    -- existing: LlmProvider enum
  embedding.rs     -- NEW: build_embedder() factory, EmbeddingProvider resolution
  vector_store.rs  -- NEW: VectorStore enum, build_vector_store() factory
  state.rs         -- extended: add embedding_provider + vector_store to AppState
```

### AppState integration

```rust
// crates/nous-daemon/src/state.rs (extend)

pub struct AppState {
    // ... existing fields ...
    pub embedder: Arc<dyn Embedder>,            // Unified embedder (local or rig-backed)
    pub vector_store_config: VectorStoreConfig, // Which backend to use
}
```

The embedder is initialized at startup based on `EmbeddingConfig`. When the
config specifies `bedrock`, the existing Bedrock client is reused to construct
a `rig_bedrock::embedding::EmbeddingModel`. When `openai`, a new OpenAI client
is built from `OPENAI_API_KEY`.

### CLI flags

```
nous start --embedding-provider <local|bedrock|openai>
           --embedding-model <model-name>
           --embedding-dimensions <N>
           --vector-store <sqlite-vec|qdrant>
           --qdrant-url <url>
           --qdrant-collection <name>
```

Environment variable equivalents:
- `NOUS_EMBEDDING_PROVIDER`
- `NOUS_EMBEDDING_MODEL`
- `NOUS_EMBEDDING_DIMENSIONS`
- `NOUS_VECTOR_STORE`
- `NOUS_QDRANT_URL`
- `NOUS_QDRANT_COLLECTION`
- `NOUS_QDRANT_API_KEY`

---

## 6. Dependency Impact

### New workspace dependencies

```toml
# Cargo.toml [workspace.dependencies] additions

rig-core = "0.36"          # already present in nous-daemon; promote to workspace
```

### New crate dependencies for nous-daemon

```toml
# crates/nous-daemon/Cargo.toml additions (all behind feature flags)

[features]
default = []
qdrant = ["dep:rig-qdrant"]

[dependencies]
rig-qdrant = { version = "0.2.5", optional = true }
```

### Existing dependencies leveraged

| Dependency | Already in workspace | Used for |
|-----------|---------------------|----------|
| `rig-bedrock` 0.4.5 | Yes (nous-daemon) | Bedrock embedding via `EmbeddingsClient` |
| `rig-core` 0.36 | Yes (nous-daemon) | OpenAI embedding via built-in provider |
| `rusqlite` 0.32 + `sqlite-vec` 0.1.9 | Yes (workspace) | Existing sqlite-vec backend |
| `ort` 2.0.0-rc.12 + `tokenizers` 0.21 | Yes (nous-core) | Local ONNX embedding |

### Why not rig-sqlite?

`rig-sqlite` 0.2.5 is a rig-compatible wrapper around sqlite-vec. However, nous
already has a working sqlite-vec integration with custom schema (memory_embeddings
table, chunk tables, dual-pool architecture). Adopting `rig-sqlite` would require
migrating this schema and losing the existing integration with the FTS pool.

Instead, we keep the existing sqlite-vec integration as-is and only add rig
vector store backends (Qdrant) as an alternative. A future workstream could
implement rig's `VectorStoreIndex` trait on top of our existing `VecPool` if
needed for RAG agent interop.

### Binary size impact

- `rig-qdrant` (optional feature): adds ~1-2 MB (gRPC client for Qdrant).
- No impact when `qdrant` feature is disabled.
- No new mandatory dependencies.

---

## 7. Migration Plan

### Phase 1: Embedding provider abstraction (non-breaking)

1. Extend `EmbeddingConfig` and `EmbeddingProvider` enum in nous-core.
2. Add `RigEmbedderAdapter` to bridge rig's `EmbeddingModel` to our `Embedder` trait.
3. Add `build_embedder()` factory in nous-daemon that returns `Arc<dyn Embedder>`
   based on resolved config.
4. Wire into `AppState` -- replace the current direct `OnnxEmbeddingModel`
   construction with the factory.
5. Default remains `Local` (OnnxEmbeddingModel). No behavior change unless
   user sets `embedding.provider`.

### Phase 2: Rig embedding providers (additive)

1. Implement `Bedrock` variant: reuse existing `rig_bedrock::client::Client` from
   LLM provider to call `embedding_model_with_ndims()`.
2. Implement `OpenAI` variant: construct `rig::providers::openai::Client` from
   `OPENAI_API_KEY`, call `embedding_model()`.
3. Handle dimension mismatch: if the user changes embedding provider, the
   existing sqlite-vec table has a fixed dimension (384). Either:
   - Validate at startup that config dimensions match `EMBEDDING_DIMENSION`.
   - Or add a migration that recreates `memory_embeddings` with the new dimension
     (requires re-embedding all stored memories).

### Phase 3: Qdrant vector store backend (optional, feature-gated)

1. Add `rig-qdrant` dependency behind `qdrant` feature flag.
2. Implement `VectorStore::Qdrant` variant in nous-daemon.
3. When backend is Qdrant:
   - `store_embedding` writes to Qdrant instead of (or in addition to) sqlite-vec.
   - `search_similar` queries Qdrant instead of sqlite-vec.
   - Hybrid search still merges with FTS5 results via RRF.
4. Add collection auto-creation on first use (idempotent).
5. Support both modes:
   - **Qdrant-only**: for production deployments with dedicated vector infra.
   - **Dual-write**: sqlite-vec + Qdrant for migration/testing periods.

### Phase 4: Dimension flexibility

1. Make `EMBEDDING_DIMENSION` configurable rather than a compile-time constant.
2. Store the active dimension in `vec_schema_version` metadata.
3. On dimension change: drop and recreate `memory_embeddings` table, trigger
   background re-embedding of all memories.

### Backwards compatibility guarantees

- Default config (`local` + `sqlite-vec`) produces identical behavior to current.
- FTS5 search is always available regardless of vector store backend.
- The `embedding BLOB` column on `memories` is unchanged.
- No migration runs unless the user explicitly changes config.

---

## 8. Testing Strategy

### Unit tests

| Test | Location | What it verifies |
|------|----------|-----------------|
| `EmbeddingConfig::default()` | nous-core | Default is local/384 |
| `RigEmbedderAdapter` with mock rig model | nous-core | f64-to-f32 conversion, error mapping |
| Config resolution (CLI > env > file) | nous-daemon | Layered config for embedding/vector_store |
| Dimension validation | nous-daemon | Startup rejects mismatched dimensions |

### Integration tests (with real providers)

| Test | Provider | Gate |
|------|----------|------|
| Bedrock Titan embed | AWS credentials | `#[ignore]` unless `AWS_ACCESS_KEY_ID` set |
| OpenAI embed | API key | `#[ignore]` unless `OPENAI_API_KEY` set |
| Qdrant insert + search | Qdrant instance | `#[ignore]` unless `QDRANT_URL` set; CI uses testcontainers |

### End-to-end tests

| Test | Scenario |
|------|----------|
| Save memory + embed + hybrid search (local) | Existing test coverage, unchanged |
| Save memory + embed + hybrid search (Bedrock) | New; validates full pipeline with cloud embeddings |
| RAG agent with dynamic context | New; validates rig agent can use our vector store as context |

### CI considerations

- Default CI runs only local/sqlite-vec tests (no credentials needed).
- A separate "integration" CI job (nightly or manual trigger) runs cloud
  provider tests with stored credentials.
- Qdrant integration tests use `testcontainers` to spin up a Qdrant instance.

---

## 9. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| f64 vs f32 storage | (a) Convert rig's f64 to f32 for sqlite-vec; (b) Store f64 natively | (a) -- sqlite-vec uses f32; f32 is sufficient for similarity search |
| Embedding dimension migration | (a) Error on mismatch; (b) Auto-migrate | (a) for now -- explicit re-embed command avoids silent data loss |
| Qdrant as default for production | (a) sqlite-vec everywhere; (b) Qdrant default when URL configured | (b) -- zero-config default stays local |
| rig `VectorStoreIndex` impl for sqlite-vec | (a) Skip; (b) Implement trait on VecPool | (a) for Phase 1 -- can add later for RAG agent tool use |
| Async vs sync embedder | (a) Keep sync `Embedder` trait; (b) Add async variant | Both -- sync for compatibility, async for daemon hot path |
| Batch re-embedding on provider change | (a) Manual `nous re-embed` command; (b) Automatic background task | (a) -- explicit is safer; background task in future workstream |

---

## 10. Future Work

- **Additional embedding providers**: Cohere, Gemini, Mistral, local FastEmbed.
- **Additional vector stores**: LanceDB (embedded, columnar), PostgreSQL (pgvector),
  S3 Vectors (AWS-native serverless).
- **RAG agent tool integration**: Implement rig's `VectorStoreIndex` on our
  vector store abstraction so agents can use memories as dynamic context.
- **Multi-vector per memory**: Store multiple embedding vectors per memory
  (e.g., title embedding + content embedding) for improved retrieval.
- **Embedding cache**: Cache embeddings to avoid re-generating when content
  hasn't changed.
- **Streaming embeddings**: For very large documents, stream chunks through
  the embedding pipeline rather than loading all into memory.
