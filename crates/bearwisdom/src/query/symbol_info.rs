// =============================================================================
// query/symbol_info.rs  —  detailed symbol inspection
//
// Returns a rich `SymbolDetail` record for a single symbol: its core
// attributes (name, kind, location, signature, doc comment), edge counts
// (how many symbols reference it and how many it references), and its
// immediate children (methods of a class, etc.).
//
// This is the backing query for a hover-card or "show symbol details" panel
// in the editor.
// =============================================================================

use crate::db::Database;
use crate::query::QueryResult;
use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::query::architecture::SymbolSummary;

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Full detail for a single symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolDetail {
    pub name: String,
    pub qualified_name: String,
    /// Symbol kind string, e.g. "class", "method", "interface".
    pub kind: String,
    pub file_path: String,
    /// 1-based start line.
    pub start_line: u32,
    /// 1-based end line (same as start_line if the extractor did not record it).
    pub end_line: u32,
    pub signature: Option<String>,
    /// XML doc comment (C#) or JSDoc (TS), if present.
    pub doc_comment: Option<String>,
    pub visibility: Option<String>,
    /// Number of edges whose target is this symbol (how many things depend on it).
    pub incoming_edge_count: u32,
    /// Number of edges whose source is this symbol (how many things it depends on).
    pub outgoing_edge_count: u32,
    /// Direct children of this symbol in the scope hierarchy.
    /// For a class: its methods, fields, properties.
    /// For a namespace: its classes.
    pub children: Vec<SymbolSummary>,
}

// ---------------------------------------------------------------------------
// Public function
// ---------------------------------------------------------------------------

/// Look up detailed information for a symbol by name or qualified name.
///
/// `query` may be a simple name (may return multiple matches) or a fully
/// qualified name (returns at most one match).
///
/// Returns an empty vec if nothing is found.
pub fn symbol_info(db: &Database, query: &str, opts: &super::QueryOptions) -> QueryResult<Vec<SymbolDetail>> {
    let _timer = db.timer("symbol_info");

    // Check cache first.
    if let Some(ref cache) = db.query_cache {
        if let Some(cached) = cache.get_symbol_info(query) {
            if let Ok(result) = serde_json::from_str::<Vec<SymbolDetail>>(&cached) {
                return Ok(result);
            }
        }
    }

    let conn = db.conn();

    // --- Step 1: Resolve to symbol rows ---
    // We resolve a list of IDs, then fetch full detail for each.
    let symbol_rows: Vec<(i64, String, String, String, String, u32, u32, Option<String>, Option<String>, Option<String>)> = {
        let (sql, param) = if query.contains('.') {
            (
                "SELECT s.id, s.name, s.qualified_name, s.kind, f.path,
                        s.line, COALESCE(s.end_line, s.line),
                        s.signature, s.doc_comment, s.visibility
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.qualified_name = ?1
                 ORDER BY s.line",
                query,
            )
        } else {
            (
                "SELECT s.id, s.name, s.qualified_name, s.kind, f.path,
                        s.line, COALESCE(s.end_line, s.line),
                        s.signature, s.doc_comment, s.visibility
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.name = ?1
                 ORDER BY s.qualified_name",
                query,
            )
        };

        let mut stmt = conn.prepare(sql)
            .context("Failed to prepare symbol_info lookup")?;

        let rows = stmt.query_map([param], |row| {
            Ok((
                row.get::<_, i64>(0)?,         // id
                row.get::<_, String>(1)?,        // name
                row.get::<_, String>(2)?,        // qualified_name
                row.get::<_, String>(3)?,        // kind
                row.get::<_, String>(4)?,        // file_path
                row.get::<_, u32>(5)?,           // start_line
                row.get::<_, u32>(6)?,           // end_line
                row.get::<_, Option<String>>(7)?,// signature
                row.get::<_, Option<String>>(8)?,// doc_comment
                row.get::<_, Option<String>>(9)?,// visibility
            ))
        }).context("Failed to execute symbol_info lookup")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect symbol_info rows")?
    };

    if symbol_rows.is_empty() {
        return Ok(vec![]);
    }

    // --- Step 2: For each symbol, fetch edge counts + children ---
    let mut details = Vec::with_capacity(symbol_rows.len());

    for (id, name, qualified_name, kind, file_path, start_line, end_line, signature, doc_comment, visibility) in symbol_rows {
        // Incoming edge count: edges pointing at this symbol.
        let incoming_edge_count: u32 = conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE target_id = ?1",
            [id],
            |r| r.get(0),
        ).context("Failed to count incoming edges")?;

        // Outgoing edge count: edges originating from this symbol.
        let outgoing_edge_count: u32 = conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE source_id = ?1",
            [id],
            |r| r.get(0),
        ).context("Failed to count outgoing edges")?;

        // Children: symbols whose scope_path equals our qualified_name.
        // Skipped unless opts.include_children is set.
        let children: Vec<SymbolSummary> = if opts.include_children {
            let mut stmt = conn.prepare(
                "SELECT s.name, s.qualified_name, s.kind, f.path, s.line
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.scope_path = ?1
                 ORDER BY s.line",
            ).context("Failed to prepare children query")?;

            let rows = stmt.query_map([&qualified_name], |row| {
                Ok(SymbolSummary {
                    name:           row.get(0)?,
                    qualified_name: row.get(1)?,
                    kind:           row.get(2)?,
                    file_path:      row.get(3)?,
                    line:           row.get(4)?,
                })
            }).context("Failed to execute children query")?;

            rows.collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect children")?
        } else {
            vec![]
        };

        details.push(SymbolDetail {
            name,
            qualified_name,
            kind,
            file_path,
            start_line,
            end_line,
            signature: if opts.include_signature { signature } else { None },
            doc_comment: if opts.include_doc { doc_comment } else { None },
            visibility,
            incoming_edge_count,
            outgoing_edge_count,
            children,
        });
    }

    // Store in cache.
    if let Some(ref cache) = db.query_cache {
        if let Ok(json) = serde_json::to_string(&details) {
            cache.put_symbol_info(query.to_string(), json);
        }
    }

    Ok(details)
}

/// JSON-returning variant of [`symbol_info`] for use in MCP and CLI paths.
///
/// Checks the cache for a raw JSON hit first and returns it directly, avoiding
/// the deserialize → struct → reserialize roundtrip that occurs when the
/// caller would otherwise call `symbol_info` and then `serde_json::to_string`.
///
/// On a cache miss the function delegates to [`symbol_info`] and serializes
/// the result exactly once before returning it.
pub fn symbol_info_json(
    db: &Database,
    query: &str,
    opts: &super::QueryOptions,
) -> super::QueryResult<String> {
    // Raw cache hit: return JSON directly without deserializing.
    if let Some(ref cache) = db.query_cache {
        if let Some(raw) = cache.get_symbol_info_raw(query) {
            return Ok(raw);
        }
    }
    let result = symbol_info(db, query, opts)?;
    serde_json::to_string(&result)
        .map_err(|e| super::QueryError::Internal(anyhow::anyhow!("serialization error: {e}")))
}

// ---------------------------------------------------------------------------
// File symbols — structured outline of a single file
// ---------------------------------------------------------------------------

/// Controls how much detail `file_symbols` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSymbolsMode {
    /// name, kind, line only.
    Names,
    /// name, kind, line, end_line, signature.  Default.
    Outline,
    /// All fields including scope_path and visibility.
    Full,
}

impl Default for FileSymbolsMode {
    fn default() -> Self {
        Self::Outline
    }
}

impl FileSymbolsMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "names" => Self::Names,
            "full" => Self::Full,
            _ => Self::Outline,
        }
    }
}

/// One symbol in a file outline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSymbol {
    pub name: String,
    pub kind: String,
    pub line: u32,
    /// Column offset (0-based).  Always populated in Full mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_path: Option<String>,
}

/// Return symbols defined in `file_path`, filtered by `mode`.
pub fn file_symbols(
    db: &Database,
    file_path: &str,
    mode: FileSymbolsMode,
) -> QueryResult<Vec<FileSymbol>> {
    let _timer = db.timer("file_symbols");
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT s.name, s.kind, s.line, s.col, s.end_line,
                s.signature, s.qualified_name, s.visibility, s.scope_path
         FROM symbols s JOIN files f ON s.file_id = f.id
         WHERE f.path = ?1
         ORDER BY s.line",
    ).context("file_symbols: prepare")?;

    let rows = stmt.query_map([file_path], |row| {
        let name: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let line: u32 = row.get(2)?;
        let col: u32 = row.get(3)?;
        let end_line: Option<u32> = row.get(4)?;
        let signature: Option<String> = row.get(5)?;
        let qualified_name: Option<String> = row.get(6)?;
        let visibility: Option<String> = row.get(7)?;
        let scope_path: Option<String> = row.get(8)?;

        Ok(match mode {
            FileSymbolsMode::Names => FileSymbol {
                name, kind, line,
                col: None, end_line: None, signature: None,
                qualified_name: None, visibility: None, scope_path: None,
            },
            FileSymbolsMode::Outline => FileSymbol {
                name, kind, line,
                col: None, end_line, signature,
                qualified_name: None, visibility: None, scope_path: None,
            },
            FileSymbolsMode::Full => FileSymbol {
                name, kind, line, col: Some(col), end_line, signature,
                qualified_name, visibility, scope_path,
            },
        })
    }).context("file_symbols: query")?;

    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("file_symbols: collect")?)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "symbol_info_tests.rs"]
mod tests;
