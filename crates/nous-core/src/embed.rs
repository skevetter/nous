use std::path::Path;
use std::sync::Mutex;

use nous_shared::NousError;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;
use tokenizers::{PaddingDirection, PaddingParams, PaddingStrategy, Tokenizer};

pub trait EmbeddingBackend: Send + Sync {
    fn model_id(&self) -> &str;
    fn dimensions(&self) -> usize;
    fn max_tokens(&self) -> usize;
    fn embed(&self, texts: &[&str]) -> nous_shared::Result<Vec<Vec<f32>>>;

    fn embed_one(&self, text: &str) -> nous_shared::Result<Vec<f32>> {
        Ok(self.embed(&[text])?.remove(0))
    }
}

pub struct OnnxBackendBuilder {
    model: Option<String>,
    variant: Option<String>,
    batch_size: usize,
}

impl OnnxBackendBuilder {
    pub fn model(mut self, model: &str) -> Self {
        self.model = Some(model.to_string());
        self
    }

    pub fn variant(mut self, variant: &str) -> Self {
        self.variant = Some(variant.to_string());
        self
    }

    pub fn batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    pub fn build(self) -> nous_shared::Result<OnnxBackend> {
        let model_repo = self
            .model
            .ok_or_else(|| NousError::Validation("model is required".into()))?;
        let variant = self
            .variant
            .ok_or_else(|| NousError::Validation("variant is required".into()))?;

        let api = hf_hub::api::sync::Api::new()
            .map_err(|e| NousError::Embedding(format!("hf-hub API init: {e}")))?;
        let repo = api.model(model_repo.clone());

        let model_path = repo
            .get(&variant)
            .map_err(|e| NousError::Embedding(format!("download model: {e}")))?;
        let tokenizer_path = repo
            .get("tokenizer.json")
            .map_err(|e| NousError::Embedding(format!("download tokenizer: {e}")))?;

        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| NousError::Embedding(format!("load tokenizer: {e}")))?;

        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            direction: PaddingDirection::Left,
            ..Default::default()
        }));

        let session = Session::builder()
            .map_err(|e| NousError::Embedding(format!("session builder: {e}")))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| NousError::Embedding(format!("optimization level: {e}")))?
            .commit_from_file(&model_path)
            .map_err(|e| NousError::Embedding(format!("load ONNX model: {e}")))?;

        let dimensions = detect_dimensions(&session, &model_path)?;

        let needs_token_type_ids = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");

        Ok(OnnxBackend {
            model_id: model_repo,
            dimensions,
            max_tokens: 8192,
            batch_size: self.batch_size,
            needs_token_type_ids,
            tokenizer,
            session: Mutex::new(session),
        })
    }
}

fn detect_dimensions(session: &Session, model_path: &Path) -> nous_shared::Result<usize> {
    let outputs = session.outputs();
    if let Some(output) = outputs.first()
        && let ort::value::ValueType::Tensor { shape, .. } = output.dtype()
        && let Some(&dim) = shape.last()
        && dim > 0
    {
        return Ok(dim as usize);
    }
    Err(NousError::Embedding(format!(
        "cannot detect embedding dimensions from model: {}",
        model_path.display()
    )))
}

pub struct OnnxBackend {
    model_id: String,
    dimensions: usize,
    max_tokens: usize,
    batch_size: usize,
    needs_token_type_ids: bool,
    tokenizer: Tokenizer,
    session: Mutex<Session>,
}

impl OnnxBackend {
    pub fn builder() -> OnnxBackendBuilder {
        OnnxBackendBuilder {
            model: None,
            variant: None,
            batch_size: 32,
        }
    }

    fn embed_batch_inner(&self, texts: &[&str]) -> nous_shared::Result<Vec<Vec<f32>>> {
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| NousError::Embedding(format!("tokenize: {e}")))?;

        let batch_size = encodings.len();
        let seq_len = encodings.first().map(|e| e.get_ids().len()).unwrap_or(0);

        let input_ids: Vec<i64> = encodings
            .iter()
            .flat_map(|e| e.get_ids().iter().map(|&id| id as i64))
            .collect();
        let attention_mask: Vec<i64> = encodings
            .iter()
            .flat_map(|e| e.get_attention_mask().iter().map(|&m| m as i64))
            .collect();

        let ids_array = ndarray::Array2::from_shape_vec((batch_size, seq_len), input_ids)
            .map_err(|e| NousError::Embedding(format!("shape input_ids: {e}")))?;
        let mask_array = ndarray::Array2::from_shape_vec((batch_size, seq_len), attention_mask)
            .map_err(|e| NousError::Embedding(format!("shape attention_mask: {e}")))?;

        let ids_tensor = TensorRef::from_array_view(ids_array.view())
            .map_err(|e| NousError::Embedding(format!("ids tensor: {e}")))?;
        let mask_tensor = TensorRef::from_array_view(mask_array.view())
            .map_err(|e| NousError::Embedding(format!("mask tensor: {e}")))?;

        let token_type_ids = vec![0i64; batch_size * seq_len];
        let token_type_array =
            ndarray::Array2::from_shape_vec((batch_size, seq_len), token_type_ids)
                .map_err(|e| NousError::Embedding(format!("shape token_type_ids: {e}")))?;
        let token_type_tensor = TensorRef::from_array_view(token_type_array.view())
            .map_err(|e| NousError::Embedding(format!("token_type tensor: {e}")))?;

        let mut session = self
            .session
            .lock()
            .map_err(|e| NousError::Embedding(format!("session lock: {e}")))?;
        let outputs = if self.needs_token_type_ids {
            session
                .run(ort::inputs![ids_tensor, mask_tensor, token_type_tensor])
                .map_err(|e| NousError::Embedding(format!("inference: {e}")))?
        } else {
            session
                .run(ort::inputs![ids_tensor, mask_tensor])
                .map_err(|e| NousError::Embedding(format!("inference: {e}")))?
        };

        let output = &outputs[0];
        let (shape, data) = output
            .try_extract_tensor::<f32>()
            .map_err(|e| NousError::Embedding(format!("extract tensor: {e}")))?;

        let dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();

        let results = if dims.len() == 3 {
            // [batch, seq_len, hidden] — last-token pooling
            let hidden = dims[2];
            (0..batch_size)
                .map(|i| {
                    let last_tok_idx = last_real_token(&encodings[i]);
                    let offset = i * dims[1] * hidden + last_tok_idx * hidden;
                    let vec = data[offset..offset + hidden].to_vec();
                    l2_normalize(vec)
                })
                .collect()
        } else if dims.len() == 2 {
            // [batch, hidden] — already pooled
            let hidden = dims[1];
            (0..batch_size)
                .map(|i| {
                    let offset = i * hidden;
                    let vec = data[offset..offset + hidden].to_vec();
                    l2_normalize(vec)
                })
                .collect()
        } else {
            return Err(NousError::Embedding(format!(
                "unexpected output shape: {dims:?}"
            )));
        };

        Ok(results)
    }
}

fn last_real_token(encoding: &tokenizers::Encoding) -> usize {
    let mask = encoding.get_attention_mask();
    mask.iter()
        .rposition(|&m| m == 1)
        .unwrap_or(mask.len().saturating_sub(1))
}

fn l2_normalize(mut vec: Vec<f32>) -> Vec<f32> {
    let norm = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

impl EmbeddingBackend for OnnxBackend {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn max_tokens(&self) -> usize {
        self.max_tokens
    }

    fn embed(&self, texts: &[&str]) -> nous_shared::Result<Vec<Vec<f32>>> {
        let mut all_results = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(self.batch_size) {
            let mut batch = self.embed_batch_inner(chunk)?;
            all_results.append(&mut batch);
        }
        Ok(all_results)
    }
}

pub struct FixtureEmbedding {
    model: String,
    dimension: usize,
    vectors: std::collections::HashMap<String, Vec<f32>>,
}

#[derive(serde::Deserialize)]
struct FixtureData {
    model: String,
    dimension: usize,
    vectors: std::collections::HashMap<String, Vec<f32>>,
}

impl FixtureEmbedding {
    pub fn load(path: &Path) -> nous_shared::Result<Self> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| NousError::Embedding(format!("read fixture: {e}")))?;
        let fixture: FixtureData = serde_json::from_str(&data)
            .map_err(|e| NousError::Embedding(format!("parse fixture: {e}")))?;
        Ok(Self {
            model: fixture.model,
            dimension: fixture.dimension,
            vectors: fixture.vectors,
        })
    }
}

impl EmbeddingBackend for FixtureEmbedding {
    fn model_id(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dimension
    }

    fn max_tokens(&self) -> usize {
        8192
    }

    fn embed(&self, texts: &[&str]) -> nous_shared::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                self.vectors
                    .get(*t)
                    .cloned()
                    .unwrap_or_else(|| self.hash_fallback(t))
            })
            .collect())
    }
}

impl FixtureEmbedding {
    fn hash_fallback(&self, text: &str) -> Vec<f32> {
        let mut raw = vec![0.0f32; self.dimension];
        for (i, byte) in text.bytes().enumerate() {
            let idx = i % self.dimension;
            raw[idx] += (byte as f32).sin() * ((i + 1) as f32).cos();
        }
        if text.is_empty() {
            for (i, val) in raw.iter_mut().enumerate() {
                *val = ((i + 1) as f32).sin();
            }
        }
        let norm = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for val in &mut raw {
                *val /= norm;
            }
        }
        raw
    }
}

pub struct MockEmbedding {
    dimensions: usize,
}

impl MockEmbedding {
    pub fn new(dimensions: usize) -> Self {
        Self { dimensions }
    }

    fn hash_text(&self, text: &str) -> Vec<f32> {
        let mut raw = vec![0.0f32; self.dimensions];
        for (i, byte) in text.bytes().enumerate() {
            let idx = i % self.dimensions;
            raw[idx] += (byte as f32).sin() * ((i + 1) as f32).cos();
        }
        if text.is_empty() {
            for (i, val) in raw.iter_mut().enumerate() {
                *val = ((i + 1) as f32).sin();
            }
        }
        let norm = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for val in &mut raw {
                *val /= norm;
            }
        }
        raw
    }
}

impl EmbeddingBackend for MockEmbedding {
    fn model_id(&self) -> &str {
        "mock"
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn max_tokens(&self) -> usize {
        8192
    }

    fn embed(&self, texts: &[&str]) -> nous_shared::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.hash_text(t)).collect())
    }
}
