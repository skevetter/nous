use std::path::PathBuf;
use std::sync::Mutex;

use crate::error::NousError;

pub trait Embedder: Send + Sync {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, NousError>;
    fn dimension(&self) -> usize;
}

pub struct OnnxEmbeddingModel {
    session: Mutex<ort::session::Session>,
    tokenizer: tokenizers::Tokenizer,
    dimension: usize,
}

const MAX_SEQ_LEN: usize = 512;

impl OnnxEmbeddingModel {
    pub fn load(model_path: Option<&str>) -> Result<Self, NousError> {
        ort::init().with_telemetry(false).commit();

        let path = resolve_model_path(model_path)?;

        if !path.exists() {
            return Err(NousError::Config(format!(
                "ONNX model not found at: {}. Set NOUS_MODEL_PATH or download all-MiniLM-L6-v2.onnx to ~/.nous/models/",
                path.display()
            )));
        }

        let tokenizer_path = path.with_file_name("tokenizer.json");
        if !tokenizer_path.exists() {
            return Err(NousError::Config(format!(
                "tokenizer.json not found at: {}. Download it for all-MiniLM-L6-v2.",
                tokenizer_path.display()
            )));
        }

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            NousError::Internal(format!(
                "failed to load tokenizer from {}: {e}",
                tokenizer_path.display()
            ))
        })?;

        let mut builder = ort::session::Session::builder()
            .map_err(|e| NousError::Internal(format!("failed to create session builder: {e}")))?;

        builder = builder
            .with_intra_threads(1)
            .map_err(|e| NousError::Internal(format!("failed to set intra threads: {e}")))?;

        let session = builder.commit_from_file(&path).map_err(|e| {
            NousError::Internal(format!(
                "failed to load ONNX model from {}: {e}",
                path.display()
            ))
        })?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            dimension: crate::db::EMBEDDING_DIMENSION,
        })
    }

    fn tokenize(&self, text: &str) -> Result<Vec<i64>, NousError> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| NousError::Internal(format!("tokenization failed: {e}")))?;

        let ids: Vec<i64> = encoding
            .get_ids()
            .iter()
            .take(MAX_SEQ_LEN)
            .map(|&id| id as i64)
            .collect();

        Ok(ids)
    }
}

impl Embedder for OnnxEmbeddingModel {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, NousError> {
        let mut session = self
            .session
            .lock()
            .map_err(|e| NousError::Internal(format!("session lock poisoned: {e}")))?;

        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let input_ids = self.tokenize(text)?;
            let seq_len = input_ids.len();
            let attention_mask: Vec<i64> = vec![1; seq_len];
            let token_type_ids: Vec<i64> = vec![0; seq_len];

            let shape = vec![1i64, seq_len as i64];

            let input_ids_tensor =
                ort::value::Tensor::from_array((shape.clone(), input_ids.into_boxed_slice()))
                    .map_err(|e| {
                        NousError::Internal(format!("failed to create input_ids tensor: {e}"))
                    })?;

            let attention_mask_tensor =
                ort::value::Tensor::from_array((shape.clone(), attention_mask.into_boxed_slice()))
                    .map_err(|e| {
                        NousError::Internal(format!("failed to create attention_mask tensor: {e}"))
                    })?;

            let token_type_ids_tensor =
                ort::value::Tensor::from_array((shape, token_type_ids.into_boxed_slice()))
                    .map_err(|e| {
                        NousError::Internal(format!("failed to create token_type_ids tensor: {e}"))
                    })?;

            let inputs = ort::inputs![
                "input_ids" => input_ids_tensor,
                "attention_mask" => attention_mask_tensor,
                "token_type_ids" => token_type_ids_tensor,
            ];

            let outputs = session
                .run(inputs)
                .map_err(|e| NousError::Internal(format!("ONNX inference failed: {e}")))?;

            let (shape, data) = outputs[0].try_extract_tensor::<f32>().map_err(|e| {
                NousError::Internal(format!("failed to extract output tensor: {e}"))
            })?;

            // Shape is (1, seq_len, hidden_dim) for sentence-transformers models
            if shape.len() == 3 {
                let seq_len_out = shape[1] as usize;
                let hidden_dim = shape[2] as usize;
                let mut embedding = vec![0.0f32; hidden_dim];

                // Mean pooling over sequence dimension
                for s in 0..seq_len_out {
                    let offset = s * hidden_dim;
                    for d in 0..hidden_dim {
                        embedding[d] += data[offset + d];
                    }
                }
                for val in &mut embedding {
                    *val /= seq_len_out as f32;
                }

                // L2 normalize
                let norm: f32 = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for val in &mut embedding {
                        *val /= norm;
                    }
                }
                results.push(embedding);
            } else {
                return Err(NousError::Internal(format!(
                    "unexpected output tensor shape: {shape:?}"
                )));
            }
        }

        Ok(results)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

fn resolve_model_path(override_path: Option<&str>) -> Result<PathBuf, NousError> {
    if let Some(p) = override_path {
        return Ok(PathBuf::from(p));
    }

    if let Ok(env_path) = std::env::var("NOUS_MODEL_PATH") {
        return Ok(PathBuf::from(env_path));
    }

    let home = dirs::home_dir()
        .ok_or_else(|| NousError::Config("cannot determine home directory".into()))?;
    Ok(home.join(".nous/models/all-MiniLM-L6-v2.onnx"))
}

/// Mock embedder for testing — produces deterministic vectors based on text content.
pub struct MockEmbedder {
    pub dimension: usize,
}

impl MockEmbedder {
    pub fn new() -> Self {
        Self {
            dimension: crate::db::EMBEDDING_DIMENSION,
        }
    }
}

impl Default for MockEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

impl Embedder for MockEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, NousError> {
        Ok(texts
            .iter()
            .map(|text| {
                let mut embedding = vec![0.0f32; self.dimension];
                for (i, byte) in text.bytes().enumerate() {
                    embedding[i % self.dimension] += byte as f32 / 255.0;
                }
                // L2 normalize
                let norm: f32 = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for val in &mut embedding {
                        *val /= norm;
                    }
                }
                embedding
            })
            .collect())
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_embedder_produces_correct_dimension() {
        let embedder = MockEmbedder::new();
        let results = embedder.embed(&["hello world"]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].len(), crate::db::EMBEDDING_DIMENSION);
    }

    #[test]
    fn mock_embedder_produces_normalized_vectors() {
        let embedder = MockEmbedder::new();
        let results = embedder.embed(&["test text"]).unwrap();
        let norm: f32 = results[0].iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn mock_embedder_deterministic() {
        let embedder = MockEmbedder::new();
        let r1 = embedder.embed(&["same text"]).unwrap();
        let r2 = embedder.embed(&["same text"]).unwrap();
        assert_eq!(r1, r2);
    }

    #[test]
    fn mock_embedder_different_texts_different_vectors() {
        let embedder = MockEmbedder::new();
        let r1 = embedder.embed(&["hello"]).unwrap();
        let r2 = embedder.embed(&["world"]).unwrap();
        assert_ne!(r1[0], r2[0]);
    }

    #[test]
    fn mock_embedder_batch() {
        let embedder = MockEmbedder::new();
        let results = embedder.embed(&["one", "two", "three"]).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn resolve_model_path_with_override() {
        let path = resolve_model_path(Some("/custom/path/model.onnx")).unwrap();
        assert_eq!(path, PathBuf::from("/custom/path/model.onnx"));
    }

    #[test]
    fn resolve_model_path_defaults_to_home() {
        std::env::remove_var("NOUS_MODEL_PATH");
        let path = resolve_model_path(None).unwrap();
        assert!(path
            .to_string_lossy()
            .contains(".nous/models/all-MiniLM-L6-v2.onnx"));
    }

    #[test]
    fn tokenizer_ids_within_vocab_range() {
        let tokenizer_path = dirs::home_dir()
            .map(|h| h.join(".nous/models/tokenizer.json"))
            .unwrap_or_default();

        if !tokenizer_path.exists() {
            eprintln!("skipping tokenizer test: tokenizer.json not found");
            return;
        }

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path).unwrap();
        let encoding = tokenizer
            .encode(
                "Hello world, this is a test sentence for tokenization.",
                true,
            )
            .unwrap();

        for &id in encoding.get_ids() {
            assert!(id < 30522, "token ID {id} exceeds vocab size 30522");
        }
    }
}
