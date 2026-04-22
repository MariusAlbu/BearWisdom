// =============================================================================
// search/chunker.rs  —  AST-aware code chunking for embedding
//
// Splits file content into chunks aligned to symbol boundaries extracted from
// the `symbols` table.  Each chunk is at most `max_tokens` tokens (estimated
// as chars / 4).  Gaps between symbols are collected as their own chunks.
// All chunks are SHA-256 hashed and persisted to `code_chunks`.
// =============================================================================

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tracing::{debug, trace};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single code chunk ready for embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChunk {
    pub file_id: i64,
    pub symbol_id: Option<i64>,
    pub content: String,
    pub content_hash: String,
    pub start_line: u32,
    pub end_line: u32,
}

// ---------------------------------------------------------------------------
// Symbol boundary row (query result)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct SymbolBoundary {
    id: i64,
    start_line: u32,
    end_line: u32,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Default token budget per chunk (tokens ≈ chars / 4).
pub const DEFAULT_MAX_TOKENS: usize = 512;

/// Split file content into chunks aligned to symbol boundaries.
///
/// Queries symbol start/end lines from the database, creates one chunk per
/// symbol (splitting oversized symbols at blank lines), then fills inter-symbol
/// gaps.  All chunks are hashed with SHA-256.
pub fn chunk_file(
    conn: &Connection,
    file_id: i64,
    content: &str,
    max_tokens: usize,
) -> Result<Vec<CodeChunk>> {
    let lines: Vec<&str> = content.split('\n').collect();
    let total_lines = lines.len() as u32;

    // Query symbol boundaries for this file, ordered by start line.
    let boundaries = query_symbol_boundaries(conn, file_id)?;

    let mut chunks: Vec<CodeChunk> = Vec::new();
    let mut covered_up_to: u32 = 0; // exclusive upper bound (0-based line index)

    for boundary in &boundaries {
        let sym_start = boundary.start_line;
        let sym_end = boundary.end_line.min(total_lines.saturating_sub(1));

        // Gap between previous symbol and this one.
        if sym_start > covered_up_to {
            let gap_content = extract_lines(&lines, covered_up_to, sym_start.saturating_sub(1));
            if !gap_content.trim().is_empty() {
                let gap_chunks =
                    split_to_max_tokens(None, file_id, gap_content, covered_up_to, max_tokens);
                chunks.extend(gap_chunks);
            }
        }

        let symbol_content = extract_lines(&lines, sym_start, sym_end);
        if !symbol_content.trim().is_empty() {
            let sym_chunks = split_to_max_tokens(
                Some(boundary.id),
                file_id,
                symbol_content,
                sym_start,
                max_tokens,
            );
            chunks.extend(sym_chunks);
        }

        covered_up_to = sym_end + 1;
    }

    // Trailing gap after the last symbol.
    if covered_up_to < total_lines {
        let tail_content = extract_lines(&lines, covered_up_to, total_lines.saturating_sub(1));
        if !tail_content.trim().is_empty() {
            let tail_chunks =
                split_to_max_tokens(None, file_id, tail_content, covered_up_to, max_tokens);
            chunks.extend(tail_chunks);
        }
    }

    trace!(
        file_id,
        chunk_count = chunks.len(),
        "chunked file into code chunks"
    );

    Ok(chunks)
}

/// Chunk a single file's content into `code_chunks` using the caller's
/// transaction. Skips dedup (full-reindex path). Used by the streaming
/// parse pipeline so per-file chunk writes join the main write transaction.
pub fn chunk_one_file_in_tx(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    content: &str,
) -> Result<u32> {
    let chunks = chunk_file(tx, file_id, content, DEFAULT_MAX_TOKENS)?;
    let mut stmt = tx.prepare_cached(
        "INSERT INTO code_chunks (file_id, symbol_id, content_hash, content, start_line, end_line)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for chunk in &chunks {
        stmt.execute(params![
            chunk.file_id,
            chunk.symbol_id,
            chunk.content_hash,
            chunk.content,
            chunk.start_line,
            chunk.end_line,
        ])?;
    }
    Ok(chunks.len() as u32)
}

/// Bulk-insert chunks for multiple files in a single transaction.
///
/// Skips all dedup logic — intended for full index after DROP+CREATE when the
/// `code_chunks` table is empty.  Computes chunks in memory, then batch-inserts.
///
/// Returns the total number of chunks inserted.
pub fn bulk_chunk_and_store(
    conn: &Connection,
    files: &[(i64, &str)],  // (file_id, content)
) -> Result<u32> {
    let tx = conn.unchecked_transaction()
        .context("Failed to begin chunk transaction")?;

    let mut total = 0u32;
    let mut stmt = tx.prepare_cached(
        "INSERT INTO code_chunks (file_id, symbol_id, content_hash, content, start_line, end_line)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;

    for &(file_id, content) in files {
        let chunks = chunk_file(&tx, file_id, content, DEFAULT_MAX_TOKENS)?;
        for chunk in &chunks {
            stmt.execute(params![
                chunk.file_id,
                chunk.symbol_id,
                chunk.content_hash,
                chunk.content,
                chunk.start_line,
                chunk.end_line,
            ])?;
        }
        total += chunks.len() as u32;
    }

    drop(stmt);
    tx.commit().context("Failed to commit bulk chunks")?;
    Ok(total)
}

/// Chunk and persist all chunks for a file into `code_chunks`.
///
/// Uses hash-based dedup: chunks whose `content_hash` matches an existing
/// chunk are preserved (keeping their vector in `vec_chunks`).  Only chunks
/// with new content are inserted; only chunks with stale content are deleted.
/// This avoids re-embedding unchanged code on incremental re-index.
///
/// Returns the number of chunks in the final set (preserved + inserted).
pub fn chunk_and_store(conn: &Connection, file_id: i64, content: &str) -> Result<u32> {
    let new_chunks = chunk_file(conn, file_id, content, DEFAULT_MAX_TOKENS)?;

    // Build a multiset of new content hashes (same hash can appear multiple times).
    let mut new_hash_budget: HashMap<&str, u32> = HashMap::new();
    for chunk in &new_chunks {
        *new_hash_budget.entry(chunk.content_hash.as_str()).or_default() += 1;
    }

    // Query existing chunks for this file.
    let existing: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, content_hash FROM code_chunks WHERE file_id = ?1 ORDER BY id",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([file_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };

    // Decide which existing chunks to keep: consume from the budget.
    let mut to_delete: Vec<i64> = Vec::new();
    let mut preserved = 0u32;
    for (id, hash) in &existing {
        if let Some(budget) = new_hash_budget.get_mut(hash.as_str()) {
            if *budget > 0 {
                *budget -= 1;
                preserved += 1;
                continue; // keep this chunk — its vector survives
            }
        }
        to_delete.push(*id);
    }

    // Delete stale chunks (and their vectors).
    for id in &to_delete {
        let _ = conn.execute("DELETE FROM vec_chunks WHERE chunk_id = ?1", [*id]);
        conn.execute("DELETE FROM code_chunks WHERE id = ?1", [*id])?;
    }

    // Insert chunks whose hash still has remaining budget (not covered by preserved chunks).
    let mut ins_stmt = conn.prepare_cached(
        "INSERT INTO code_chunks (file_id, symbol_id, content_hash, content, start_line, end_line)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;

    let mut inserted = 0u32;
    for chunk in &new_chunks {
        // If budget for this hash is > 0, it means we still need to insert one.
        if let Some(budget) = new_hash_budget.get_mut(chunk.content_hash.as_str()) {
            if *budget > 0 {
                ins_stmt.execute(params![
                    chunk.file_id,
                    chunk.symbol_id,
                    chunk.content_hash,
                    chunk.content,
                    chunk.start_line,
                    chunk.end_line,
                ])?;
                *budget -= 1;
                inserted += 1;
            }
        }
    }

    if preserved > 0 {
        debug!(
            file_id,
            preserved,
            inserted,
            deleted = to_delete.len(),
            "chunk_and_store: hash-based dedup preserved {preserved} chunks"
        );
    }

    Ok(preserved + inserted)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn query_symbol_boundaries(conn: &Connection, file_id: i64) -> Result<Vec<SymbolBoundary>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, line, COALESCE(end_line, line) AS end_line
         FROM symbols
         WHERE file_id = ?1
         ORDER BY line",
    )?;

    let rows = stmt
        .query_map(params![file_id], |row| {
            Ok(SymbolBoundary {
                id: row.get(0)?,
                start_line: row.get::<_, u32>(1)?,
                end_line: row.get::<_, u32>(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to query symbol boundaries")?;

    Ok(rows)
}

/// Extract a slice of lines (0-based, inclusive on both ends) as a single
/// string joined by newlines.
fn extract_lines(lines: &[&str], start: u32, end: u32) -> String {
    let start = start as usize;
    let end = (end as usize).min(lines.len().saturating_sub(1));
    if start > end || start >= lines.len() {
        return String::new();
    }
    lines[start..=end].join("\n")
}

/// Estimate token count from char count (chars / 4, rounding up).
fn estimate_tokens(s: &str) -> usize {
    (s.chars().count() + 3) / 4
}

/// Compute SHA-256 hex digest of `text`.
fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Split `content` (starting at `base_line`) into chunks of at most
/// `max_tokens` tokens.  Tries to split at blank lines first; falls back to
/// hard-splitting every `max_tokens * 4` chars.
fn split_to_max_tokens(
    symbol_id: Option<i64>,
    file_id: i64,
    content: String,
    base_line: u32,
    max_tokens: usize,
) -> Vec<CodeChunk> {
    if estimate_tokens(&content) <= max_tokens {
        let hash = sha256_hex(&content);
        let line_count = content.lines().count() as u32;
        return vec![CodeChunk {
            file_id,
            symbol_id,
            content_hash: hash,
            content,
            start_line: base_line,
            end_line: base_line + line_count.saturating_sub(1),
        }];
    }

    // Collect lines as owned strings to avoid borrow conflicts with `content`.
    let owned_lines: Vec<String> = content.lines().map(|l| l.to_owned()).collect();
    let line_count = owned_lines.len() as u32;

    let mut result: Vec<CodeChunk> = Vec::new();
    // Indices into `owned_lines` for the current accumulation window.
    let mut window_start: usize = 0;

    let mut i = 0usize;
    while i <= owned_lines.len() {
        let is_end = i == owned_lines.len();
        let is_blank = !is_end && owned_lines[i].trim().is_empty();
        let flush = is_end || (is_blank && i > window_start);

        if flush && window_start < i {
            let slice = &owned_lines[window_start..i];
            let joined = slice.join("\n");

            if estimate_tokens(&joined) > max_tokens {
                let hard = hard_split_by_chars(
                    &joined,
                    file_id,
                    symbol_id,
                    base_line + window_start as u32,
                    max_tokens,
                );
                result.extend(hard);
            } else if !joined.trim().is_empty() {
                let hash = sha256_hex(&joined);
                result.push(CodeChunk {
                    file_id,
                    symbol_id,
                    content_hash: hash,
                    content: joined,
                    start_line: base_line + window_start as u32,
                    end_line: base_line + i as u32 - 1,
                });
            }
            window_start = i + 1; // skip the blank line
        }

        i += 1;
    }

    if result.is_empty() {
        // Fallback: the whole content as one chunk (all blank-line splits
        // yielded nothing meaningful).
        let hash = sha256_hex(&content);
        result.push(CodeChunk {
            file_id,
            symbol_id,
            content_hash: hash,
            content,
            start_line: base_line,
            end_line: base_line + line_count.saturating_sub(1),
        });
    }

    result
}

/// Hard split by character budget when blank-line splitting is insufficient.
/// Lines within the text are re-counted from `base_line`.
fn hard_split_by_chars(
    text: &str,
    file_id: i64,
    symbol_id: Option<i64>,
    base_line: u32,
    max_tokens: usize,
) -> Vec<CodeChunk> {
    let char_budget = max_tokens * 4;
    let mut result: Vec<CodeChunk> = Vec::new();
    let mut offset = 0usize;
    let mut line_offset = base_line;

    while offset < text.len() {
        // Find the end of this chunk (at a char boundary).
        let end_byte = find_char_boundary(text, offset + char_budget);
        let slice = &text[offset..end_byte];
        let line_count = slice.lines().count() as u32;
        let hash = sha256_hex(slice);

        if !slice.trim().is_empty() {
            result.push(CodeChunk {
                file_id,
                symbol_id,
                content_hash: hash,
                content: slice.to_owned(),
                start_line: line_offset,
                end_line: line_offset + line_count.saturating_sub(1),
            });
        }

        line_offset += line_count;

        // Safety valve: if the boundary didn't advance (e.g. zero budget), step one byte.
        if end_byte <= offset {
            offset += 1;
        } else {
            offset = end_byte;
        }
    }

    result
}

/// Find the largest byte index ≤ `pos` that is a valid UTF-8 char boundary.
fn find_char_boundary(s: &str, pos: usize) -> usize {
    let clamped = pos.min(s.len());
    // Walk backwards from `clamped` to the nearest char boundary.
    let mut i = clamped;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "chunker_tests.rs"]
mod tests;
