use std::collections::HashMap;

use nous_shared::Result;
use rusqlite::params;

use crate::db::MemoryDb;
use crate::types::{ContextEntry, Importance, Memory, SearchFilters, SearchMode, SearchResult};

fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    let day_secs = secs % 86400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;

    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let year_days: i64 = if leap { 366 } else { 365 };
        if remaining < year_days {
            break;
        }
        remaining -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days: [i64; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut mo = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md {
            mo = i;
            break;
        }
        remaining -= md;
    }
    let d = remaining + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        y,
        mo + 1,
        d,
        h,
        m,
        s
    )
}

fn importance_weight(importance: &Importance) -> f64 {
    match importance {
        Importance::High => 3.0,
        Importance::Moderate => 2.0,
        Importance::Low => 1.0,
    }
}

fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        title: row.get(1)?,
        content: row.get(2)?,
        memory_type: row.get::<_, String>(3)?.parse().map_err(|e: String| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, e.into())
        })?,
        source: row.get(4)?,
        importance: row.get::<_, String>(5)?.parse().map_err(|e: String| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, e.into())
        })?,
        confidence: row.get::<_, String>(6)?.parse().map_err(|e: String| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, e.into())
        })?,
        workspace_id: row.get(7)?,
        session_id: row.get(8)?,
        trace_id: row.get(9)?,
        agent_id: row.get(10)?,
        agent_model: row.get(11)?,
        valid_from: row.get(12)?,
        valid_until: row.get(13)?,
        archived: row.get::<_, i64>(14)? != 0,
        category_id: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

fn load_tags_for(conn: &rusqlite::Connection, memory_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT t.name FROM tags t
         JOIN memory_tags mt ON mt.tag_id = t.id
         WHERE mt.memory_id = ?1
         ORDER BY t.name",
    )?;
    let tags = stmt
        .query_map(params![memory_id], |row| row.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(tags)
}

fn batch_load_tags(
    conn: &rusqlite::Connection,
    memory_ids: &[&str],
) -> Result<HashMap<String, Vec<String>>> {
    if memory_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders: String = (1..=memory_ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT mt.memory_id, t.name FROM tags t
         JOIN memory_tags mt ON mt.tag_id = t.id
         WHERE mt.memory_id IN ({placeholders})
         ORDER BY mt.memory_id, t.name"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = memory_ids
        .iter()
        .map(|id| id as &dyn rusqlite::types::ToSql)
        .collect();
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (memory_id, tag_name) in rows {
        map.entry(memory_id).or_default().push(tag_name);
    }
    Ok(map)
}

impl MemoryDb {
    pub(crate) fn search_fts(
        &self,
        query: &str,
        filters: &SearchFilters,
    ) -> Result<Vec<SearchResult>> {
        let mut conditions = vec!["memories_fts MATCH ?1".to_owned()];
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(query.to_owned())];

        self.apply_filter_conditions(filters, &mut conditions, &mut param_values);

        let limit = filters.limit.unwrap_or(20);
        let sql = format!(
            "SELECT m.id, m.title, m.content, m.memory_type, m.source, m.importance, m.confidence,
                    m.workspace_id, m.session_id, m.trace_id, m.agent_id, m.agent_model,
                    m.valid_from, m.valid_until, m.archived, m.category_id, m.created_at, m.updated_at,
                    bm25(memories_fts) AS rank
             FROM memories_fts
             JOIN memories m ON m.rowid = memories_fts.rowid
             WHERE {}
             ORDER BY rank,
                      CASE m.importance WHEN 'high' THEN 1 WHEN 'moderate' THEN 2 ELSE 3 END,
                      m.created_at DESC
             LIMIT ?{}",
            conditions.join(" AND "),
            param_values.len() + 1,
        );

        param_values.push(Box::new(limit as i64));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut stmt = self.connection().prepare(&sql)?;
        let results = stmt
            .query_map(params_ref.as_slice(), |row| {
                let memory = row_to_memory(row)?;
                let bm25_rank: f64 = row.get(18)?;
                Ok((memory, bm25_rank))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut search_results = Vec::new();
        for (memory, bm25_rank) in results {
            let tags = load_tags_for(self.connection(), &memory.id)?;
            let rank = -bm25_rank * importance_weight(&memory.importance);
            search_results.push(SearchResult { memory, tags, rank });
        }

        Ok(search_results)
    }

    pub(crate) fn search_semantic(
        &self,
        query_embedding: &[f32],
        filters: &SearchFilters,
    ) -> Result<Vec<SearchResult>> {
        let limit = filters.limit.unwrap_or(20);
        let knn_k = (limit * 5) as i64;

        let query_blob: Vec<u8> = query_embedding
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();

        let mut stmt = self.connection().prepare(
            "SELECT chunk_id, distance
             FROM memory_embeddings
             WHERE embedding MATCH ?1 AND k = ?2
             ORDER BY distance",
        )?;

        let chunk_rows = stmt
            .query_map(params![query_blob, knn_k], |row| {
                let chunk_id: String = row.get(0)?;
                let distance: f64 = row.get(1)?;
                Ok((chunk_id, distance))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut best_distances: HashMap<String, f64> = HashMap::new();
        for (chunk_id, distance) in &chunk_rows {
            let memory_id: String = self.connection().query_row(
                "SELECT memory_id FROM memory_chunks WHERE id = ?1",
                params![chunk_id],
                |row| row.get(0),
            )?;
            let entry = best_distances.entry(memory_id).or_insert(f64::INFINITY);
            if *distance < *entry {
                *entry = *distance;
            }
        }

        let mut scored: Vec<(String, f64)> = best_distances.into_iter().collect();
        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let candidate_ids: Vec<&str> = scored.iter().map(|(id, _)| id.as_str()).collect();

        // Batch-load in 2 queries instead of 2N (replaces former N+1 per-row lookups)
        let memory_map = self.batch_load_memories(&candidate_ids)?;
        let tags_map = batch_load_tags(self.connection(), &candidate_ids)?;

        let mut results = Vec::new();
        for (memory_id, distance) in &scored {
            if results.len() >= limit {
                break;
            }

            let memory = match memory_map.get(memory_id) {
                Some(m) => m.clone(),
                None => continue,
            };

            if !self.matches_filters(&memory, filters)? {
                continue;
            }

            let tags = tags_map.get(memory_id).cloned().unwrap_or_default();
            let rank = (1.0 / (1.0 + distance)) * importance_weight(&memory.importance);
            results.push(SearchResult { memory, tags, rank });
        }

        Ok(results)
    }

    pub fn search(
        &self,
        query: &str,
        embedding: &[f32],
        filters: &SearchFilters,
        mode: SearchMode,
    ) -> Result<Vec<SearchResult>> {
        let results = match mode {
            SearchMode::Fts => self.search_fts(query, filters)?,
            SearchMode::Semantic => self.search_semantic(embedding, filters)?,
            SearchMode::Hybrid => {
                let fts_results = self.search_fts(query, filters)?;
                let sem_results = self.search_semantic(embedding, filters)?;
                self.fuse_rrf(fts_results, sem_results, filters)?
            }
        };

        self.log_access_for_results(&results)?;
        Ok(results)
    }

    pub fn context(&self, workspace_id: i64, summary: bool) -> Result<Vec<ContextEntry>> {
        let mut stmt = self.connection().prepare(
            "SELECT id, title, content, memory_type, importance, created_at
             FROM memories
             WHERE workspace_id = ?1
               AND archived = 0
               AND (valid_until IS NULL OR valid_until > strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ORDER BY CASE importance WHEN 'high' THEN 1 WHEN 'moderate' THEN 2 ELSE 3 END,
                      created_at DESC
             LIMIT 50",
        )?;

        let entries = stmt
            .query_map(params![workspace_id], |row| {
                let content: String = row.get(2)?;
                Ok(ContextEntry {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: if summary { None } else { Some(content) },
                    memory_type: row.get::<_, String>(3)?.parse().map_err(|e: String| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            e.into(),
                        )
                    })?,
                    importance: row.get::<_, String>(4)?.parse().map_err(|e: String| {
                        rusqlite::Error::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Text,
                            e.into(),
                        )
                    })?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        for entry in &entries {
            self.connection().execute(
                "INSERT INTO access_log (memory_id, access_type) VALUES (?1, 'context')",
                params![entry.id],
            )?;
        }

        Ok(entries)
    }

    fn apply_filter_conditions(
        &self,
        filters: &SearchFilters,
        conditions: &mut Vec<String>,
        params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    ) {
        let archived = filters.archived.unwrap_or(false);
        conditions.push(format!("m.archived = ?{}", params.len() + 1));
        params.push(Box::new(archived as i64));

        if let Some(ref mt) = filters.memory_type {
            conditions.push(format!("m.memory_type = ?{}", params.len() + 1));
            params.push(Box::new(mt.to_string()));
        }

        if let Some(cat_id) = filters.category_id {
            conditions.push(format!("m.category_id = ?{}", params.len() + 1));
            params.push(Box::new(cat_id));
        }

        if let Some(ws_id) = filters.workspace_id {
            conditions.push(format!("m.workspace_id = ?{}", params.len() + 1));
            params.push(Box::new(ws_id));
        }

        if let Some(ref imp) = filters.importance {
            conditions.push(format!("m.importance = ?{}", params.len() + 1));
            params.push(Box::new(imp.to_string()));
        }

        if let Some(ref conf) = filters.confidence {
            conditions.push(format!("m.confidence = ?{}", params.len() + 1));
            params.push(Box::new(conf.to_string()));
        }

        if let Some(ref since) = filters.since {
            conditions.push(format!("m.created_at >= ?{}", params.len() + 1));
            params.push(Box::new(since.clone()));
        }

        if let Some(ref until) = filters.until {
            conditions.push(format!("m.created_at <= ?{}", params.len() + 1));
            params.push(Box::new(until.clone()));
        }

        if filters.valid_only == Some(true) {
            conditions.push(
                "(m.valid_until IS NULL OR m.valid_until > strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))"
                    .to_owned(),
            );
        }

        if let Some(ref tag_names) = filters.tags
            && !tag_names.is_empty()
        {
            let placeholders: Vec<String> = tag_names
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", params.len() + 1 + i))
                .collect();
            conditions.push(format!(
                "m.id IN (SELECT mt.memory_id FROM memory_tags mt JOIN tags t ON t.id = mt.tag_id WHERE t.name IN ({}))",
                placeholders.join(", ")
            ));
            for tag in tag_names {
                params.push(Box::new(tag.clone()));
            }
        }
    }

    fn batch_load_memories(&self, ids: &[&str]) -> Result<HashMap<String, Memory>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders: String = (1..=ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT id, title, content, memory_type, source, importance, confidence,
                    workspace_id, session_id, trace_id, agent_id, agent_model,
                    valid_from, valid_until, archived, category_id, created_at, updated_at
             FROM memories WHERE id IN ({placeholders})"
        );
        let mut stmt = self.connection().prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt
            .query_map(params.as_slice(), row_to_memory)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut map = HashMap::with_capacity(rows.len());
        for memory in rows {
            map.insert(memory.id.clone(), memory);
        }
        Ok(map)
    }

    fn matches_filters(&self, memory: &Memory, filters: &SearchFilters) -> Result<bool> {
        let archived = filters.archived.unwrap_or(false);
        if memory.archived != archived {
            return Ok(false);
        }

        if let Some(ref mt) = filters.memory_type
            && &memory.memory_type != mt
        {
            return Ok(false);
        }

        if let Some(cat_id) = filters.category_id
            && memory.category_id != Some(cat_id)
        {
            return Ok(false);
        }

        if let Some(ws_id) = filters.workspace_id
            && memory.workspace_id != Some(ws_id)
        {
            return Ok(false);
        }

        if let Some(ref imp) = filters.importance
            && &memory.importance != imp
        {
            return Ok(false);
        }

        if let Some(ref conf) = filters.confidence
            && &memory.confidence != conf
        {
            return Ok(false);
        }

        if let Some(ref since) = filters.since
            && memory.created_at < *since
        {
            return Ok(false);
        }

        if let Some(ref until) = filters.until
            && memory.created_at > *until
        {
            return Ok(false);
        }

        if filters.valid_only == Some(true)
            && let Some(ref valid_until) = memory.valid_until
            && !valid_until.is_empty()
        {
            let now = now_iso8601();
            if *valid_until < now {
                return Ok(false);
            }
        }

        if let Some(ref tag_names) = filters.tags
            && !tag_names.is_empty()
        {
            let tags = load_tags_for(self.connection(), &memory.id)?;
            if !tag_names.iter().any(|t| tags.contains(t)) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn fuse_rrf(
        &self,
        fts_results: Vec<SearchResult>,
        sem_results: Vec<SearchResult>,
        filters: &SearchFilters,
    ) -> Result<Vec<SearchResult>> {
        let k = 60.0_f64;
        let limit = filters.limit.unwrap_or(20);

        let mut rrf_scores: HashMap<String, f64> = HashMap::new();
        let mut memory_map: HashMap<String, SearchResult> = HashMap::new();

        for (rank_pos, result) in fts_results.into_iter().enumerate() {
            let score = 1.0 / (k + rank_pos as f64 + 1.0);
            *rrf_scores.entry(result.memory.id.clone()).or_default() += score;
            memory_map.insert(result.memory.id.clone(), result);
        }

        for (rank_pos, result) in sem_results.into_iter().enumerate() {
            let score = 1.0 / (k + rank_pos as f64 + 1.0);
            *rrf_scores.entry(result.memory.id.clone()).or_default() += score;
            memory_map.entry(result.memory.id.clone()).or_insert(result);
        }

        let mut fused: Vec<SearchResult> = rrf_scores
            .into_iter()
            .filter_map(|(id, rrf_score)| {
                memory_map.remove(&id).map(|mut result| {
                    result.rank = rrf_score * importance_weight(&result.memory.importance);
                    result
                })
            })
            .collect();

        fused.sort_by(|a, b| {
            b.rank
                .partial_cmp(&a.rank)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        fused.truncate(limit);

        Ok(fused)
    }

    fn log_access_for_results(&self, results: &[SearchResult]) -> Result<()> {
        for result in results {
            self.connection().execute(
                "INSERT INTO access_log (memory_id, access_type) VALUES (?1, 'search')",
                params![result.memory.id],
            )?;
        }
        Ok(())
    }
}
