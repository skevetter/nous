pub use crate::embed::{
    Embedder, EmbeddingConfig, EmbeddingProvider, OnnxEmbeddingModel, RigEmbedderAdapter,
};
#[cfg(any(test, feature = "test-utils"))]
pub use crate::embed::MockEmbedder;
