use std::collections::HashMap;

use nous_shared::Result;
use rusqlite::params;

use crate::db::MemoryDb;
use crate::embed::EmbeddingBackend;
use crate::types::Category;

pub struct CategoryClassifier {
    cache: HashMap<i64, (Category, Vec<f32>)>,
    default_threshold: f32,
}

impl CategoryClassifier {
    pub fn new(db: &MemoryDb, embedder: &dyn EmbeddingBackend, threshold: f32) -> Result<Self> {
        let mut classifier = Self {
            cache: HashMap::new(),
            default_threshold: threshold,
        };
        classifier.load_and_embed(db, embedder)?;
        Ok(classifier)
    }

    pub fn refresh(&mut self, db: &MemoryDb, embedder: &dyn EmbeddingBackend) -> Result<()> {
        self.cache.clear();
        self.load_and_embed(db, embedder)
    }

    pub fn classify(&self, memory_embedding: &[f32]) -> Option<i64> {
        let top_level: Vec<_> = self
            .cache
            .values()
            .filter(|(cat, _)| cat.parent_id.is_none())
            .collect();

        let best = self.best_match(&top_level, memory_embedding)?;

        let children: Vec<_> = self
            .cache
            .values()
            .filter(|(cat, _)| cat.parent_id == Some(best))
            .collect();

        if children.is_empty() {
            return Some(best);
        }

        match self.best_match(&children, memory_embedding) {
            Some(child_id) => Some(child_id),
            None => Some(best),
        }
    }

    pub fn cache(&self) -> &HashMap<i64, (Category, Vec<f32>)> {
        &self.cache
    }

    fn best_match(&self, candidates: &[&(Category, Vec<f32>)], query: &[f32]) -> Option<i64> {
        let mut best_id = None;
        let mut best_score = self.default_threshold;

        for (cat, emb) in candidates {
            let threshold = cat.threshold.unwrap_or(self.default_threshold);
            let score = cosine_similarity(emb, query);
            if score > best_score && score > threshold {
                best_score = score;
                best_id = Some(cat.id);
            }
        }

        best_id
    }

    fn load_and_embed(&mut self, db: &MemoryDb, embedder: &dyn EmbeddingBackend) -> Result<()> {
        let conn = db.connection();
        let mut stmt = conn.prepare(
            "SELECT id, name, parent_id, source, description, embedding, threshold, created_at FROM categories",
        )?;

        let categories: Vec<Category> = stmt
            .query_map([], |row| {
                Ok(Category {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    parent_id: row.get(2)?,
                    source: row.get::<_, String>(3)?.parse().map_err(|e: String| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            e.into(),
                        )
                    })?,
                    description: row.get(4)?,
                    embedding: row.get(5)?,
                    threshold: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut needs_embedding: Vec<(i64, String)> = Vec::new();
        for cat in &categories {
            if cat.embedding.is_none() {
                let text = match &cat.description {
                    Some(desc) if !desc.is_empty() => format!("{} {}", cat.name, desc),
                    _ => cat.name.clone(),
                };
                needs_embedding.push((cat.id, text));
            }
        }

        let mut new_embeddings: HashMap<i64, Vec<f32>> = HashMap::new();
        if !needs_embedding.is_empty() {
            let texts: Vec<&str> = needs_embedding.iter().map(|(_, t)| t.as_str()).collect();
            let embeddings = embedder.embed(&texts)?;
            for ((id, _), emb) in needs_embedding.iter().zip(embeddings) {
                let blob = embedding_to_blob(&emb);
                conn.execute(
                    "UPDATE categories SET embedding = ?1 WHERE id = ?2",
                    params![blob, id],
                )?;
                new_embeddings.insert(*id, emb);
            }
        }

        for cat in categories {
            let emb = if let Some(ref blob) = cat.embedding {
                blob_to_embedding(blob)
            } else {
                new_embeddings.remove(&cat.id).unwrap_or_default()
            };
            self.cache.insert(cat.id, (cat, emb));
        }

        Ok(())
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}
