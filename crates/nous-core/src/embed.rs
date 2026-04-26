use std::borrow::Cow;
use std::path::Path;
use std::sync::Mutex;

use nous_shared::NousError;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;
use tokenizers::{PaddingDirection, PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModelArch {
    Encoder,
    Decoder,
}

#[derive(Debug, Clone)]
struct KvInputMeta {
    name: String,
    num_heads: usize,
    head_dim: usize,
}

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

        let max_tokens = detect_max_tokens(&tokenizer_path)?;

        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| NousError::Embedding(format!("load tokenizer: {e}")))?;

        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: max_tokens,
                ..Default::default()
            }))
            .map_err(|e| NousError::Embedding(format!("set truncation: {e}")))?;

        let session = Session::builder()
            .map_err(|e| NousError::Embedding(format!("session builder: {e}")))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| NousError::Embedding(format!("optimization level: {e}")))?
            .commit_from_file(&model_path)
            .map_err(|e| NousError::Embedding(format!("load ONNX model: {e}")))?;

        let hidden_states_idx = find_hidden_states_output(&session)?;
        let dimensions = detect_dimensions(&session, &model_path, hidden_states_idx)?;

        let needs_token_type_ids = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");

        let kv_inputs: Vec<KvInputMeta> = session
            .inputs()
            .iter()
            .filter(|i| i.name().starts_with("past_key_values"))
            .filter_map(|i| {
                if let ort::value::ValueType::Tensor { shape, .. } = i.dtype()
                    && shape.len() == 4
                {
                    return Some(KvInputMeta {
                        name: i.name().to_string(),
                        num_heads: shape[1].max(0) as usize,
                        head_dim: shape[3].max(0) as usize,
                    });
                }
                None
            })
            .collect();

        let arch = if kv_inputs.is_empty() {
            ModelArch::Encoder
        } else {
            ModelArch::Decoder
        };

        let hidden_states_idx = find_hidden_states_output(&session)?;

        let pad_direction = match arch {
            ModelArch::Encoder => PaddingDirection::Right,
            ModelArch::Decoder => PaddingDirection::Left,
        };
        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            direction: pad_direction,
            ..Default::default()
        }));

        // TODO: register/activate model in db at startup

        Ok(OnnxBackend {
            model_id: model_repo,
            dimensions,
            max_tokens,
            batch_size: self.batch_size,
            needs_token_type_ids,
            arch,
            kv_inputs,
            hidden_states_idx,
            tokenizer,
            session: Mutex::new(session),
        })
    }
}

fn detect_max_tokens(tokenizer_path: &Path) -> nous_shared::Result<usize> {
    let raw = std::fs::read_to_string(tokenizer_path)
        .map_err(|e| NousError::Embedding(format!("read tokenizer.json: {e}")))?;
    let json: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| NousError::Embedding(format!("parse tokenizer.json: {e}")))?;
    Ok(json
        .get("model_max_length")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(8192))
}

fn detect_dimensions(
    session: &Session,
    model_path: &Path,
    output_idx: usize,
) -> nous_shared::Result<usize> {
    let outputs = session.outputs();
    if let Some(output) = outputs.get(output_idx)
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

fn build_position_ids(attention_mask: &ndarray::ArrayView2<i64>) -> ndarray::Array2<i64> {
    let (batch, seq_len) = attention_mask.dim();
    let mut positions = ndarray::Array2::<i64>::zeros((batch, seq_len));
    for b in 0..batch {
        let mut pos = 0i64;
        for s in 0..seq_len {
            if attention_mask[[b, s]] == 1 {
                positions[[b, s]] = pos;
                pos += 1;
            }
        }
    }
    positions
}

fn find_hidden_states_output(session: &Session) -> nous_shared::Result<usize> {
    let outputs = session.outputs();
    let candidates: Vec<_> = outputs
        .iter()
        .enumerate()
        .filter(|(_, o)| !o.name().starts_with("present"))
        .collect();
    if candidates.len() == 1 {
        return Ok(candidates[0].0);
    }
    // Fallback: 3D tensor = hidden states, 4D = KV cache
    outputs
        .iter()
        .enumerate()
        .find(|(_, o)| {
            matches!(
                o.dtype(),
                ort::value::ValueType::Tensor { shape, .. } if shape.len() == 3
            )
        })
        .map(|(i, _)| i)
        .ok_or_else(|| NousError::Embedding("no hidden-states output found".into()))
}

pub struct OnnxBackend {
    model_id: String,
    dimensions: usize,
    max_tokens: usize,
    batch_size: usize,
    needs_token_type_ids: bool,
    arch: ModelArch,
    kv_inputs: Vec<KvInputMeta>,
    hidden_states_idx: usize,
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

        let mut named_inputs = ort::inputs! {
            "input_ids" => ids_tensor,
            "attention_mask" => mask_tensor,
        };

        let token_type_array;
        if self.needs_token_type_ids {
            let token_type_ids = vec![0i64; batch_size * seq_len];
            token_type_array =
                ndarray::Array2::from_shape_vec((batch_size, seq_len), token_type_ids)
                    .map_err(|e| NousError::Embedding(format!("shape token_type_ids: {e}")))?;
            let token_type_tensor = TensorRef::from_array_view(token_type_array.view())
                .map_err(|e| NousError::Embedding(format!("token_type tensor: {e}")))?;
            named_inputs.push((Cow::from("token_type_ids"), token_type_tensor.into()));
        }

        let position_ids_array;
        let mut kv_tensors: Vec<ndarray::Array4<f32>> = Vec::new();
        if self.arch == ModelArch::Decoder {
            position_ids_array = build_position_ids(&mask_array.view());
            let pos_tensor = TensorRef::from_array_view(position_ids_array.view())
                .map_err(|e| NousError::Embedding(format!("position_ids tensor: {e}")))?;
            named_inputs.push((Cow::from("position_ids"), pos_tensor.into()));

            for kv in &self.kv_inputs {
                let t = ndarray::Array4::<f32>::zeros((batch_size, kv.num_heads, 0, kv.head_dim));
                kv_tensors.push(t);
            }
            for (i, kv) in self.kv_inputs.iter().enumerate() {
                let tensor = TensorRef::from_array_view(kv_tensors[i].view())
                    .map_err(|e| NousError::Embedding(format!("kv tensor: {e}")))?;
                named_inputs.push((Cow::from(kv.name.clone()), tensor.into()));
            }
        }

        let mut session = self
            .session
            .lock()
            .map_err(|e| NousError::Embedding(format!("session lock: {e}")))?;
        let outputs = session
            .run(named_inputs)
            .map_err(|e| NousError::Embedding(format!("inference: {e}")))?;

        let output = &outputs[self.hidden_states_idx];
        let (shape, data) = output
            .try_extract_tensor::<f32>()
            .map_err(|e| NousError::Embedding(format!("extract tensor: {e}")))?;

        let dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();

        let results = if dims.len() == 3 && self.arch == ModelArch::Decoder {
            let hidden = dims[2];
            (0..batch_size)
                .map(|i| {
                    let last_tok_idx = last_real_token(&encodings[i]);
                    let offset = i * dims[1] * hidden + last_tok_idx * hidden;
                    let vec = data[offset..offset + hidden].to_vec();
                    l2_normalize(vec)
                })
                .collect()
        } else if dims.len() == 3 {
            mean_pool(&encodings, data, &dims)
                .into_iter()
                .map(l2_normalize)
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

fn mean_pool(encodings: &[tokenizers::Encoding], data: &[f32], dims: &[usize]) -> Vec<Vec<f32>> {
    let seq_len = dims[1];
    let hidden = dims[2];
    encodings
        .iter()
        .enumerate()
        .map(|(i, enc)| {
            let mask = enc.get_attention_mask();
            let mut sum = vec![0.0f32; hidden];
            let mut count = 0usize;
            for t in 0..seq_len {
                if mask.get(t).copied().unwrap_or(0) == 1 {
                    let off = i * seq_len * hidden + t * hidden;
                    for d in 0..hidden {
                        sum[d] += data[off + d];
                    }
                    count += 1;
                }
            }
            if count > 0 {
                let c = count as f32;
                for v in &mut sum {
                    *v /= c;
                }
            }
            sum
        })
        .collect()
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
