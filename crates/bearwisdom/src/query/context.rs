// =============================================================================
// query/context.rs  —  smart context selection for LLM prompts
//
// 5-step pipeline:
//   1. Semantic seed   — FTS5 search for task-relevant symbols.
//   2. Graph expansion — walk edges outward from seeds (callers + callees).
//   3. Concept enrichment — pull in concept siblings of seed symbols.
//   4. Scoring         — rank candidates by semantic relevance, proximity,
//                         centrality, edge strength, and concept overlap.
//   5. Budget pruning  — walk the ranked list until the token budget is full.
//
// Designed for LLM tool use: given a natural-language task description,
// return the most relevant symbols and files that should be included in
// the context window.
// =============================================================================

use crate::db::Database;
use crate::query::architecture::SymbolSummary;
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A symbol with a relevance score and explanation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedSymbol {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file_path: String,
    pub line: u32,
    /// Composite relevance score (0.0–1.0, higher = more relevant).
    pub score: f64,
    /// Short explanation of why this symbol was included.
    pub reason: String,
}

/// The complete smart context result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartContextResult {
    /// The original task description.
    pub task: String,
    /// Estimated token count of the returned context.
    pub token_estimate: u32,
    /// Ranked symbols (most relevant first).
    pub symbols: Vec<RankedSymbol>,
    /// Unique file paths referenced by the symbols.
    pub files: Vec<String>,
    /// Concepts that overlap with the context.
    pub concepts: Vec<String>,
}

// ---------------------------------------------------------------------------
// Internal candidate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Candidate {
    id: i64,
    name: String,
    qualified_name: String,
    kind: String,
    file_path: String,
    line: u32,
    semantic_score: f64,  // from FTS5 BM25
    min_hop: u32,         // distance from nearest seed (0 = is a seed)
    incoming_edges: u32,  // centrality signal
    concept_overlap: bool,
    avg_confidence: f64,
}

// ---------------------------------------------------------------------------
// Scoring weights
// ---------------------------------------------------------------------------

const W_SEMANTIC: f64 = 0.40;
const W_PROXIMITY: f64 = 0.25;
const W_CENTRALITY: f64 = 0.15;
const W_EDGE_STRENGTH: f64 = 0.10;
const W_CONCEPT: f64 = 0.10;

// ---------------------------------------------------------------------------
// Token estimation per symbol kind
// ---------------------------------------------------------------------------

fn estimate_tokens(kind: &str) -> u32 {
    match kind {
        "class" | "struct" => 200,
        "method" | "function" => 100,
        "interface" | "trait" => 80,
        "enum" => 50,
        "property" | "field" | "constant" => 20,
        _ => 60,
    }
}

// ---------------------------------------------------------------------------
// Seed helpers
// ---------------------------------------------------------------------------

/// Common English stop words that are useless as FTS5 symbol search terms.
fn stop_words() -> HashSet<&'static str> {
    [
        "a", "an", "the", "is", "are", "to", "from", "with", "for", "of",
        "in", "on", "by", "and", "or", "not", "this", "that", "it", "be",
        "do", "if", "at", "as", "no", "has", "have", "was", "were", "will",
        "can", "new", "all", "get", "set", "add", "make", "find", "what",
        "how", "where", "which", "when", "why", "who",
    ]
    .into_iter()
    .collect()
}

/// Multi-strategy seed collection.
///
/// 1. Try FTS5 with the raw task string — works when the task contains symbol names.
/// 2. If empty, extract keywords (strip stop words) and try each individually.
/// 3. Also try LIKE-based fallback on `symbols.name` for words ≥ 4 chars.
///
/// Results are deduplicated by `qualified_name`, keeping the highest score.
/// The returned list is capped at `limit`.
fn seed_symbols(db: &Database, task: &str, limit: usize) -> Vec<super::search::SearchResult> {
    use super::search::SearchResult;

    let opts = super::QueryOptions { include_signature: false, ..Default::default() };
    let mut by_qn: HashMap<String, SearchResult> = HashMap::new();

    // --- Strategy 1: raw FTS5 ---
    if let Ok(results) = super::search::search_symbols(db, task, limit, &opts) {
        for r in results {
            by_qn.insert(r.qualified_name.clone(), r);
        }
    }

    // --- Strategy 2: per-keyword FTS5 ---
    if by_qn.is_empty() {
        let stops = stop_words();
        let keywords: Vec<&str> = task
            .split_whitespace()
            .filter(|w| {
                let lower = w.to_lowercase();
                !stops.contains(lower.as_str())
            })
            .collect();

        for kw in &keywords {
            if let Ok(results) = super::search::search_symbols(db, kw, limit, &opts) {
                for r in results {
                    let entry = by_qn.entry(r.qualified_name.clone());
                    entry
                        .and_modify(|existing| {
                            if r.score > existing.score {
                                *existing = r.clone();
                            }
                        })
                        .or_insert(r);
                }
            }
        }
    }

    // --- Strategy 3: LIKE-based fallback ---
    {
        let words: Vec<&str> = task
            .split_whitespace()
            .filter(|w| w.len() >= 4)
            .collect();

        for word in &words {
            let pattern = format!("%{}%", word.to_lowercase());
            let sql = "SELECT s.name, s.qualified_name, s.kind, f.path, s.line
                       FROM symbols s JOIN files f ON f.id = s.file_id
                       WHERE lower(s.name) LIKE ?1
                       LIMIT 20";
            if let Ok(rows) = db.conn.prepare(sql).and_then(|mut stmt| {
                stmt.query_map([&pattern], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, u32>(4)?,
                    ))
                })
                .map(|mapped| mapped.filter_map(|r| r.ok()).collect::<Vec<_>>())
            }) {
                for (name, qn, kind, fp, line) in rows {
                    by_qn.entry(qn.clone()).or_insert(SearchResult {
                        name,
                        qualified_name: qn,
                        kind,
                        file_path: fp,
                        start_line: line,
                        signature: None,
                        score: 0.5,
                    });
                }
            }
        }
    }

    // Sort by score descending, cap at limit.
    let mut results: Vec<SearchResult> = by_qn.into_values().collect();
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    results
}

// ---------------------------------------------------------------------------
// Public function
// ---------------------------------------------------------------------------

/// Build a smart context for the given task description.
///
/// `budget` — maximum token estimate (default 8000).
/// `depth`  — graph expansion depth (default 2).
pub fn smart_context(
    db: &Database,
    task: &str,
    budget: u32,
    depth: u32,
) -> Result<SmartContextResult> {
    let conn = &db.conn;

    if task.trim().is_empty() {
        return Ok(SmartContextResult {
            task: task.to_string(),
            token_estimate: 0,
            symbols: vec![],
            files: vec![],
            concepts: vec![],
        });
    }

    // =====================================================================
    // Step 1: Semantic seed — multi-strategy symbol search
    // =====================================================================
    let seeds = seed_symbols(db, task, 30);

    if seeds.is_empty() {
        return Ok(SmartContextResult {
            task: task.to_string(),
            token_estimate: 0,
            symbols: vec![],
            files: vec![],
            concepts: vec![],
        });
    }

    // Normalise BM25 scores to 0.0–1.0.
    let max_score = seeds.iter().map(|s| s.score).fold(0.0f64, f64::max).max(1.0);

    // Build candidate map: qualified_name → Candidate.
    let mut candidates: HashMap<String, Candidate> = HashMap::new();

    // Resolve seed symbol IDs and populate initial candidates.
    let mut seed_ids: Vec<i64> = Vec::new();
    let mut seed_concepts: HashSet<String> = HashSet::new();

    for seed in &seeds {
        let row: Option<(i64, u32)> = conn
            .query_row(
                "SELECT s.id, (SELECT COUNT(*) FROM edges WHERE target_id = s.id)
                 FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE s.qualified_name = ?1 LIMIT 1",
                [&seed.qualified_name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();

        let Some((sym_id, incoming)) = row else { continue };
        seed_ids.push(sym_id);

        candidates.insert(
            seed.qualified_name.clone(),
            Candidate {
                id: sym_id,
                name: seed.name.clone(),
                qualified_name: seed.qualified_name.clone(),
                kind: seed.kind.clone(),
                file_path: seed.file_path.clone(),
                line: seed.start_line,
                semantic_score: seed.score / max_score,
                min_hop: 0,
                incoming_edges: incoming,
                concept_overlap: true, // seeds always overlap
                avg_confidence: 1.0,
            },
        );

        // Collect concept memberships for seeds.
        let mut stmt = conn.prepare_cached(
            "SELECT c.name FROM concepts c
             JOIN concept_members cm ON cm.concept_id = c.id
             WHERE cm.symbol_id = ?1",
        )?;
        let concepts: Vec<String> = stmt
            .query_map([sym_id], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        seed_concepts.extend(concepts);
    }

    // =====================================================================
    // Step 2: Graph expansion — follow edges from seeds
    // =====================================================================
    let mut frontier: Vec<i64> = seed_ids.clone();
    let mut visited: HashSet<i64> = seed_ids.iter().copied().collect();

    for hop in 1..=depth {
        if frontier.is_empty() {
            break;
        }

        let mut next_frontier: Vec<i64> = Vec::new();

        for &source_id in &frontier {
            // Outgoing edges (callees / dependencies).
            let mut stmt = conn.prepare_cached(
                "SELECT e.target_id, s.name, s.qualified_name, s.kind, f.path, s.line,
                        e.confidence,
                        (SELECT COUNT(*) FROM edges WHERE target_id = e.target_id)
                 FROM edges e
                 JOIN symbols s ON s.id = e.target_id
                 JOIN files f ON f.id = s.file_id
                 WHERE e.source_id = ?1
                 LIMIT 20",
            )?;

            let rows: Vec<(i64, String, String, String, String, u32, f64, u32)> = stmt
                .query_map([source_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?,
                        r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            for (id, name, qn, kind, fp, line, conf, incoming) in rows {
                if !visited.insert(id) {
                    // Already visited — update min_hop if closer.
                    if let Some(c) = candidates.get_mut(&qn) {
                        c.min_hop = c.min_hop.min(hop);
                    }
                    continue;
                }

                next_frontier.push(id);
                candidates.insert(qn.clone(), Candidate {
                    id,
                    name,
                    qualified_name: qn,
                    kind,
                    file_path: fp,
                    line,
                    semantic_score: 0.0,
                    min_hop: hop,
                    incoming_edges: incoming,
                    concept_overlap: false, // updated in step 3
                    avg_confidence: conf,
                });
            }

            // Incoming edges (callers).
            let mut stmt = conn.prepare_cached(
                "SELECT e.source_id, s.name, s.qualified_name, s.kind, f.path, s.line,
                        e.confidence,
                        (SELECT COUNT(*) FROM edges WHERE target_id = e.source_id)
                 FROM edges e
                 JOIN symbols s ON s.id = e.source_id
                 JOIN files f ON f.id = s.file_id
                 WHERE e.target_id = ?1
                 LIMIT 20",
            )?;

            let rows: Vec<(i64, String, String, String, String, u32, f64, u32)> = stmt
                .query_map([source_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?,
                        r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            for (id, name, qn, kind, fp, line, conf, incoming) in rows {
                if !visited.insert(id) {
                    if let Some(c) = candidates.get_mut(&qn) {
                        c.min_hop = c.min_hop.min(hop);
                    }
                    continue;
                }

                next_frontier.push(id);
                candidates.insert(qn.clone(), Candidate {
                    id,
                    name,
                    qualified_name: qn,
                    kind,
                    file_path: fp,
                    line,
                    semantic_score: 0.0,
                    min_hop: hop,
                    incoming_edges: incoming,
                    concept_overlap: false,
                    avg_confidence: conf,
                });
            }
        }

        frontier = next_frontier;
    }

    // =====================================================================
    // Step 3: Concept enrichment — mark candidates in seed concepts
    // =====================================================================
    if !seed_concepts.is_empty() {
        // Collect all symbol IDs that belong to any seed concept.
        let concept_member_ids: HashSet<i64> = {
            let placeholders = seed_concepts.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT cm.symbol_id FROM concept_members cm
                 JOIN concepts c ON c.id = cm.concept_id
                 WHERE c.name IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql)?;
            let concepts_vec: Vec<&str> = seed_concepts.iter().map(|s| s.as_str()).collect();
            let params: Vec<&dyn rusqlite::types::ToSql> = concepts_vec
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows: Vec<i64> = stmt
                .query_map(params.as_slice(), |r| r.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            rows.into_iter().collect()
        };

        for candidate in candidates.values_mut() {
            if concept_member_ids.contains(&candidate.id) {
                candidate.concept_overlap = true;
            }
        }
    }

    // =====================================================================
    // Step 4: Scoring
    // =====================================================================
    let max_incoming = candidates
        .values()
        .map(|c| c.incoming_edges)
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    let mut scored: Vec<(String, f64, &Candidate)> = candidates
        .values()
        .map(|c| {
            let proximity = 1.0 / (1.0 + c.min_hop as f64);
            let centrality = c.incoming_edges as f64 / max_incoming;
            let concept = if c.concept_overlap { 1.0 } else { 0.0 };

            let score = W_SEMANTIC * c.semantic_score
                + W_PROXIMITY * proximity
                + W_CENTRALITY * centrality
                + W_EDGE_STRENGTH * c.avg_confidence
                + W_CONCEPT * concept;

            (c.qualified_name.clone(), score, c)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // =====================================================================
    // Step 5: Budget pruning
    // =====================================================================
    let mut result_symbols: Vec<RankedSymbol> = Vec::new();
    let mut result_files: HashSet<String> = HashSet::new();
    let mut token_total: u32 = 0;

    for (_, score, candidate) in &scored {
        let tokens = estimate_tokens(&candidate.kind);
        if token_total + tokens > budget && !result_symbols.is_empty() {
            break;
        }

        let reason = if candidate.min_hop == 0 {
            "semantic match".to_string()
        } else if candidate.concept_overlap {
            format!("concept sibling ({}hop)", candidate.min_hop)
        } else {
            format!("graph neighbor ({}hop)", candidate.min_hop)
        };

        result_symbols.push(RankedSymbol {
            name: candidate.name.clone(),
            qualified_name: candidate.qualified_name.clone(),
            kind: candidate.kind.clone(),
            file_path: candidate.file_path.clone(),
            line: candidate.line,
            score: *score,
            reason,
        });

        result_files.insert(candidate.file_path.clone());
        token_total += tokens;
    }

    let mut files: Vec<String> = result_files.into_iter().collect();
    files.sort();

    let concepts: Vec<String> = seed_concepts.into_iter().collect();

    Ok(SmartContextResult {
        task: task.to_string(),
        token_estimate: token_total,
        symbols: result_symbols,
        files,
        concepts,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_empty_task() {
        let db = Database::open_in_memory().unwrap();
        let result = smart_context(&db, "", 8000, 2).unwrap();
        assert!(result.symbols.is_empty());
        assert_eq!(result.token_estimate, 0);
    }

    #[test]
    fn test_no_matches() {
        let db = Database::open_in_memory().unwrap();
        let result = smart_context(&db, "xyznonexistent", 8000, 2).unwrap();
        assert!(result.symbols.is_empty());
    }

    #[test]
    fn test_basic_context() {
        let db = Database::open_in_memory().unwrap();

        db.conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/svc.rs', 'h', 'rust', 0)",
            [],
        ).unwrap();
        let fid = db.conn.last_insert_rowid();

        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, signature)
             VALUES (?1, 'CatalogService', 'app.CatalogService', 'class', 1, 0, 'class CatalogService')",
            [fid],
        ).unwrap();
        let sid = db.conn.last_insert_rowid();

        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, scope_path)
             VALUES (?1, 'get_item', 'app.CatalogService.get_item', 'method', 5, 0, 'app.CatalogService')",
            [fid],
        ).unwrap();
        let mid = db.conn.last_insert_rowid();

        // Edge: get_item calls CatalogService (so CatalogService has incoming)
        db.conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 0.9)",
            rusqlite::params![mid, sid],
        ).unwrap();

        let result = smart_context(&db, "CatalogService", 8000, 2).unwrap();
        assert!(!result.symbols.is_empty());
        assert!(result.token_estimate > 0);
        assert!(!result.files.is_empty());
        // First result should be the direct match.
        assert_eq!(result.symbols[0].name, "CatalogService");
    }
}
