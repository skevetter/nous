use std::collections::HashMap;

use crate::memory::SimilarMemory;

const DEFAULT_K: f32 = 60.0;

/// Reciprocal Rank Fusion: merges results from FTS and vector search.
/// `rrf_score` = sum(1 / (k + `rank_i`)) for each result list containing the item.
pub fn rerank_rrf(
    fts_results: &[SimilarMemory],
    vec_results: &[SimilarMemory],
    k: Option<f32>,
) -> Vec<SimilarMemory> {
    let k = k.unwrap_or(DEFAULT_K);
    let mut scores: HashMap<String, (f32, SimilarMemory)> = HashMap::new();

    for (rank, result) in fts_results.iter().enumerate() {
        // rank fits in u16 (result lists are never 65535+ items); u16→f32 is lossless
        let rank_f = f32::from(u16::try_from(rank).unwrap_or(u16::MAX));
        let rrf_score = 1.0 / (k + rank_f + 1.0);
        scores
            .entry(result.memory.id.clone())
            .and_modify(|(score, _)| *score += rrf_score)
            .or_insert((rrf_score, result.clone()));
    }

    for (rank, result) in vec_results.iter().enumerate() {
        let rank_f = f32::from(u16::try_from(rank).unwrap_or(u16::MAX));
        let rrf_score = 1.0 / (k + rank_f + 1.0);
        scores
            .entry(result.memory.id.clone())
            .and_modify(|(score, _)| *score += rrf_score)
            .or_insert((rrf_score, result.clone()));
    }

    let mut merged: Vec<SimilarMemory> = scores
        .into_values()
        .map(|(score, mut mem)| {
            mem.score = score;
            mem
        })
        .collect();

    merged.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Memory;

    fn make_similar_memory(id: &str, score: f32) -> SimilarMemory {
        SimilarMemory {
            memory: Memory {
                id: id.to_string(),
                workspace_id: "default".to_string(),
                agent_id: None,
                title: format!("Memory {id}"),
                content: format!("Content for {id}"),
                memory_type: "fact".to_string(),
                importance: "moderate".to_string(),
                topic_key: None,
                valid_from: None,
                valid_until: None,
                archived: false,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
            },
            score,
        }
    }

    #[test]
    fn empty_inputs_returns_empty() {
        let result = rerank_rrf(&[], &[], None);
        assert!(result.is_empty());
    }

    #[test]
    fn fts_only_returns_ranked() {
        let fts = vec![make_similar_memory("a", 0.9), make_similar_memory("b", 0.8)];
        let result = rerank_rrf(&fts, &[], None);
        assert_eq!(result.len(), 2);
        assert!(result[0].score > result[1].score);
    }

    #[test]
    fn vec_only_returns_ranked() {
        let vec_results = vec![
            make_similar_memory("x", 0.95),
            make_similar_memory("y", 0.85),
        ];
        let result = rerank_rrf(&[], &vec_results, None);
        assert_eq!(result.len(), 2);
        assert!(result[0].score > result[1].score);
    }

    #[test]
    fn duplicate_gets_boosted() {
        let fts = vec![
            make_similar_memory("shared", 0.9),
            make_similar_memory("fts-only", 0.8),
        ];
        let vec_results = vec![
            make_similar_memory("shared", 0.95),
            make_similar_memory("vec-only", 0.85),
        ];
        let result = rerank_rrf(&fts, &vec_results, None);

        // "shared" appears in both lists so should have the highest RRF score
        assert_eq!(result[0].memory.id, "shared");
        assert!(result[0].score > result[1].score);
    }

    #[test]
    fn custom_k_parameter() {
        let fts = vec![make_similar_memory("a", 0.9)];
        let vec_results = vec![make_similar_memory("a", 0.95)];

        let result_default = rerank_rrf(&fts, &vec_results, None);
        let result_custom = rerank_rrf(&fts, &vec_results, Some(10.0));

        // With smaller k, individual rank contributions are larger
        assert!(result_custom[0].score > result_default[0].score);
    }

    #[test]
    fn deduplication_works() {
        let fts = vec![make_similar_memory("a", 0.9), make_similar_memory("b", 0.8)];
        let vec_results = vec![
            make_similar_memory("a", 0.95),
            make_similar_memory("c", 0.85),
        ];
        let result = rerank_rrf(&fts, &vec_results, None);
        assert_eq!(result.len(), 3); // a, b, c — no duplicates
    }
}
