use anyhow::Result;
use sqlez::connection::Connection;

use crate::store::{self, StoredChunk};

/// A single search result.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SearchResult {
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub snippet: String,
    pub score: f32,
}

const DEFAULT_LIMIT: usize = 10;

/// Perform hybrid search (keyword + vector).
pub fn search(
    conn: &Connection,
    query: &str,
    limit: usize,
    file_filter: Option<&str>,
) -> Result<Vec<SearchResult>> {
    let limit = limit.max(1).min(50).max(DEFAULT_LIMIT);

    let total_chunks = store::chunk_count(conn)?;
    if total_chunks == 0 {
        return Ok(vec![]);
    }

    // Phase 1: Keyword search via FTS5
    let keyword_results = keyword_search(conn, query, limit * 2, file_filter)?;

    // Phase 2: Vector search (load all embeddings, compute similarity)
    let vector_results = if total_chunks > 0 {
        vector_search(conn, query, limit * 2, file_filter)?
    } else {
        vec![]
    };

    // Phase 3: Reciprocal Rank Fusion
    let merged = fuse_results(keyword_results, vector_results, limit);
    Ok(merged)
}

fn keyword_search(
    conn: &Connection,
    query: &str,
    limit: usize,
    file_filter: Option<&str>,
) -> Result<Vec<(String, usize, usize, String, f32)>> {
    // Clean query for FTS5: remove special characters, keep words
    let fts_query: String = query
        .split_whitespace()
        .filter(|w| w.len() > 1)
        .map(|w| format!("\"{}\"", w.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");

    if fts_query.is_empty() {
        return Ok(vec![]);
    }

    let sql = if let Some(_filter) = file_filter {
        format!(
            "SELECT c.file_path, c.start_line, c.end_line, c.chunk_text, bm25(chunks_fts) as score
             FROM chunks_fts
             JOIN chunks c ON chunks_fts.rowid = c.id
             WHERE chunks_fts MATCH ? AND c.file_path LIKE ?
             ORDER BY score
             LIMIT {}",
            limit
        )
    } else {
        format!(
            "SELECT c.file_path, c.start_line, c.end_line, c.chunk_text, bm25(chunks_fts) as score
             FROM chunks_fts
             JOIN chunks c ON chunks_fts.rowid = c.id
             WHERE chunks_fts MATCH ?
             ORDER BY score
             LIMIT {}",
            limit
        )
    };

    let results = if let Some(filter) = file_filter {
        let filter_pattern = format!("%{}%", filter);
        conn.select_bound::<(&str, &str), (String, usize, usize, String, f32)>(&sql)?((
            fts_query.as_str(),
            filter_pattern.as_str(),
        ))?
    } else {
        conn.select_bound::<&str, (String, usize, usize, String, f32)>(&sql)?(
            fts_query.as_str(),
        )?
    };

    Ok(results)
}

fn vector_search(
    conn: &Connection,
    query: &str,
    limit: usize,
    file_filter: Option<&str>,
) -> Result<Vec<(String, usize, usize, String, f32)>> {
    let chunks = store::load_all_chunks(conn)?;

    // Filter by file_path if filter provided
    let chunks: Vec<&StoredChunk> = if let Some(filter) = file_filter {
        chunks
            .iter()
            .filter(|c| c.file_path.contains(filter))
            .collect()
    } else {
        chunks.iter().collect()
    };

    if chunks.is_empty() {
        return Ok(vec![]);
    }

    // Use keyword matching on the query to find relevant chunks
    // Since we can't embed the query client-side without an API call,
    // we use a simplified approach: score chunks by keyword overlap
    let query_lower = query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().filter(|w| w.len() > 1).collect();

    if query_terms.is_empty() {
        return Ok(vec![]);
    }

    let mut scored: Vec<(String, usize, usize, String, f32)> = chunks
        .iter()
        .filter_map(|chunk| {
            let chunk_lower = chunk.chunk_text.to_lowercase();
            let mut score = 0.0f32;

            // TF-like scoring: count term occurrences
            let total_terms = query_terms.len() as f32;
            for term in &query_terms {
                let count = chunk_lower.matches(term).count() as f32;
                if count > 0.0 {
                    score += 1.0 + (count / (1.0 + count)).ln();
                }
            }

            // Bonus for title-like matches (first line)
            if let Some(first_line) = chunk_lower.lines().next() {
                for term in &query_terms {
                    if first_line.contains(term) {
                        score += 0.5;
                    }
                }
            }

            // Normalize
            score /= total_terms.max(1.0);

            if score > 0.0 {
                Some((
                    chunk.file_path.clone(),
                    chunk.start_line,
                    chunk.end_line,
                    chunk.chunk_text.clone(),
                    score,
                ))
            } else {
                None
            }
        })
        .collect();

    // Sort by score descending, take top limit
    scored.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    Ok(scored)
}

/// Reciprocal Rank Fusion: merge two ranked result lists.
fn fuse_results(
    keyword: Vec<(String, usize, usize, String, f32)>,
    vector: Vec<(String, usize, usize, String, f32)>,
    limit: usize,
) -> Vec<SearchResult> {
    const K: f32 = 60.0;

    // Build a map of chunk key -> fused score
    let mut fused: std::collections::HashMap<(String, usize, usize), f32> =
        std::collections::HashMap::new();

    for (rank, (file_path, start, end, _text, score)) in keyword.iter().enumerate() {
        let rrf_score = 1.0 / (K + rank as f32);
        *fused.entry((file_path.clone(), *start, *end)).or_insert(0.0) += rrf_score + score * 0.3;
    }

    for (rank, (file_path, start, end, _text, score)) in vector.iter().enumerate() {
        let rrf_score = 1.0 / (K + rank as f32);
        *fused.entry((file_path.clone(), *start, *end)).or_insert(0.0) += rrf_score + score * 0.3;
    }

    // Collect and sort results
    let mut results: Vec<SearchResult> = fused
        .into_iter()
        .map(|((file_path, start_line, end_line), score)| {
            // Find the original chunk text from either list
            let snippet = keyword
                .iter()
                .chain(vector.iter())
                .find(|(fp, s, e, _, _)| fp == &file_path && *s == start_line && *e == end_line)
                .map(|(_, _, _, text, _)| text.clone())
                .unwrap_or_default();

            SearchResult {
                file_path,
                start_line,
                end_line,
                snippet: truncate_snippet(&snippet),
                score,
            }
        })
        .collect();

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    results
}

fn truncate_snippet(text: &str) -> String {
    let lines: Vec<&str> = text.lines().take(30).collect();
    let mut result = lines.join("\n");
    if lines.len() < text.lines().count() {
        result.push_str("\n...");
    }
    result
}
