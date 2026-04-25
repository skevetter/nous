pub trait EmbeddingBackend: Send + Sync {
    fn model_id(&self) -> &str;
    fn dimensions(&self) -> usize;
    fn max_tokens(&self) -> usize;
    fn embed(&self, texts: &[&str]) -> nous_shared::Result<Vec<Vec<f32>>>;

    fn embed_one(&self, text: &str) -> nous_shared::Result<Vec<f32>> {
        Ok(self.embed(&[text])?.remove(0))
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
