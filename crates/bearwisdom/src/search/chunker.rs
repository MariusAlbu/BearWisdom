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
use tracing::trace;

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

/// Chunk and persist all chunks for a file into `code_chunks`.
///
/// Deletes existing chunks for the file first, then inserts all new chunks.
/// Returns the number of chunks inserted.
pub fn chunk_and_store(conn: &Connection, file_id: i64, content: &str) -> Result<u32> {
    let chunks = chunk_file(conn, file_id, content, DEFAULT_MAX_TOKENS)?;

    // Delete existing chunks — ON DELETE CASCADE in the schema handles vector
    // entries if/when we add that constraint; here we delete explicitly first.
    conn.execute("DELETE FROM code_chunks WHERE file_id = ?1", params![file_id])
        .with_context(|| format!("Failed to delete existing chunks for file_id={file_id}"))?;

    let count = chunks.len() as u32;

    let mut stmt = conn.prepare_cached(
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
        ])
        .with_context(|| {
            format!(
                "Failed to insert chunk for file_id={file_id} lines={}-{}",
                chunk.start_line, chunk.end_line
            )
        })?;
    }

    Ok(count)
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
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn make_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::apply_pragmas(&conn, true).unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        conn
    }

    fn insert_file(conn: &Connection, path: &str) -> i64 {
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'rust', 0)",
            params![path],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_symbol(conn: &Connection, file_id: i64, name: &str, start: u32, end: u32) -> i64 {
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, end_line)
             VALUES (?1, ?2, ?2, 'function', ?3, 0, ?4)",
            params![file_id, name, start, end],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        let h1 = sha256_hex("hello");
        let h2 = sha256_hex("hello");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // 32 bytes → 64 hex chars
    }

    #[test]
    fn sha256_hex_differs_for_different_input() {
        assert_ne!(sha256_hex("a"), sha256_hex("b"));
    }

    #[test]
    fn estimate_tokens_rounds_up() {
        assert_eq!(estimate_tokens("abcd"), 1); // 4 chars = 1 token
        assert_eq!(estimate_tokens("abcde"), 2); // 5 chars → 2 tokens
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn extract_lines_basic() {
        let lines = vec!["zero", "one", "two", "three"];
        assert_eq!(extract_lines(&lines, 1, 2), "one\ntwo");
    }

    #[test]
    fn extract_lines_out_of_bounds_clamps() {
        let lines = vec!["a", "b"];
        // end beyond length — should clamp
        let result = extract_lines(&lines, 0, 10);
        assert_eq!(result, "a\nb");
    }

    #[test]
    fn chunk_file_no_symbols_produces_single_chunk() {
        let conn = make_db();
        let file_id = insert_file(&conn, "src/empty.rs");
        let content = "fn main() {\n    println!(\"hello\");\n}\n";

        let chunks = chunk_file(&conn, file_id, content, 512).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].symbol_id, None);
        assert_eq!(chunks[0].start_line, 0);
    }

    #[test]
    fn chunk_file_with_symbols_aligns_to_boundaries() {
        let conn = make_db();
        let file_id = insert_file(&conn, "src/two_fns.rs");

        // Two non-overlapping symbols on lines 0-2 and 4-6
        insert_symbol(&conn, file_id, "foo", 0, 2);
        insert_symbol(&conn, file_id, "bar", 4, 6);

        let content = "fn foo() {\n    1\n}\n\nfn bar() {\n    2\n}\n";
        let chunks = chunk_file(&conn, file_id, content, 512).unwrap();

        // We expect at least 2 chunks (one per symbol).
        assert!(chunks.len() >= 2, "Expected at least 2 chunks, got {}", chunks.len());

        // All chunks belong to this file.
        assert!(chunks.iter().all(|c| c.file_id == file_id));
    }

    #[test]
    fn chunk_file_symbol_gets_symbol_id() {
        let conn = make_db();
        let file_id = insert_file(&conn, "src/fn.rs");
        let sym_id = insert_symbol(&conn, file_id, "my_fn", 0, 2);

        let content = "fn my_fn() {\n    42\n}\n";
        let chunks = chunk_file(&conn, file_id, content, 512).unwrap();

        // At least one chunk should carry the symbol id.
        let with_sym: Vec<_> = chunks.iter().filter(|c| c.symbol_id == Some(sym_id)).collect();
        assert!(!with_sym.is_empty(), "At least one chunk should reference the symbol");
    }

    #[test]
    fn chunk_and_store_deletes_and_reinserts() {
        let conn = make_db();
        let file_id = insert_file(&conn, "src/store.rs");

        let content = "fn a() {}\nfn b() {}\n";

        let n1 = chunk_and_store(&conn, file_id, content).unwrap();
        assert!(n1 > 0);

        // Store again — should replace.
        let n2 = chunk_and_store(&conn, file_id, content).unwrap();
        assert_eq!(n1, n2, "Second store should produce same count");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM code_chunks WHERE file_id = ?1", params![file_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, n2 as i64, "DB should contain exactly the new chunks");
    }

    #[test]
    fn chunk_hash_is_stored() {
        let conn = make_db();
        let file_id = insert_file(&conn, "src/hash.rs");
        chunk_and_store(&conn, file_id, "fn x() {}\n").unwrap();

        let hash: String = conn
            .query_row(
                "SELECT content_hash FROM code_chunks WHERE file_id = ?1 LIMIT 1",
                params![file_id],
                |r| r.get(0),
            )
            .unwrap();
        // SHA-256 hex is exactly 64 chars.
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn oversized_chunk_is_split() {
        let conn = make_db();
        let file_id = insert_file(&conn, "src/big.rs");

        // Create content larger than 512 tokens (> 2048 chars), no symbols.
        let big_line = "x".repeat(200);
        let content = (0..20).map(|_| big_line.clone()).collect::<Vec<_>>().join("\n");
        // 20 lines × 200 chars = 4000 chars → ~1000 tokens > 512 budget

        let chunks = chunk_file(&conn, file_id, &content, 512).unwrap();
        assert!(chunks.len() > 1, "Oversized content should produce multiple chunks");
    }

    #[test]
    fn empty_content_produces_no_chunks() {
        let conn = make_db();
        let file_id = insert_file(&conn, "src/empty.rs");
        let chunks = chunk_file(&conn, file_id, "", 512).unwrap();
        assert!(chunks.is_empty() || chunks.iter().all(|c| c.content.trim().is_empty()),
            "Empty content should produce no meaningful chunks");
    }
}
