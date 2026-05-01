// =============================================================================
// query/pattern.rs — tree-sitter AST pattern search
//
// Lets callers express structural questions ("show me all match expressions
// with N+ arms", "show me string literals appearing in match patterns")
// directly as tree-sitter queries against the project source. The tool is
// useful exactly when text-based grep falls short — when the question is
// shape-of-AST, not shape-of-string.
// =============================================================================

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

use crate::db::Database;
use crate::languages::default_registry;
use crate::query::QueryResult;

/// One capture in one query match.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMatch {
    pub file_path: String,
    /// 1-based start line of the captured node.
    pub start_line: u32,
    /// 1-based end line of the captured node.
    pub end_line: u32,
    /// 0-based start column.
    pub start_col: u32,
    /// 0-based end column.
    pub end_col: u32,
    /// Tree-sitter node kind (e.g. `match_expression`, `string_literal`).
    pub node_kind: String,
    /// Capture name from the query (`@fn`, `@target`, ...). Empty when the
    /// pattern has no captures.
    pub capture_name: String,
    /// First ~120 bytes of the captured source text, for context.
    pub snippet: String,
}

/// Run a tree-sitter query over every internal file of `language` known to
/// the index, collecting captured-node matches.
///
/// `query_str` is a tree-sitter S-expression query; see
/// <https://tree-sitter.github.io/tree-sitter/using-parsers/queries/index.html>.
/// Returns at most `max_results` matches across all files.
pub fn pattern_search(
    db: &Database,
    project_root: &Path,
    language: &str,
    query_str: &str,
    max_results: u32,
) -> QueryResult<Vec<PatternMatch>> {
    let _timer = db.timer("pattern_search");

    let plugin = default_registry().get(language);
    let grammar = plugin
        .grammar(language)
        .ok_or_else(|| anyhow::anyhow!("language '{language}' has no registered grammar"))
        .map_err(crate::query::error::QueryError::Internal)?;

    let query = Query::new(&grammar, query_str)
        .with_context(|| format!("invalid tree-sitter query: {query_str}"))
        .map_err(crate::query::error::QueryError::Internal)?;

    let capture_names: Vec<String> = query
        .capture_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let conn = db.conn();
    let mut stmt = conn
        .prepare_cached(
            "SELECT path
             FROM files
             WHERE language = ?1
               AND origin = 'internal'
             ORDER BY path",
        )
        .context("pattern_search: prepare files query")
        .map_err(crate::query::error::QueryError::Internal)?;

    let file_paths: Vec<String> = stmt
        .query_map([language], |r| r.get::<_, String>(0))
        .context("pattern_search: execute files query")
        .map_err(crate::query::error::QueryError::Internal)?
        .filter_map(|r| r.ok())
        .collect();

    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .map_err(|e| crate::query::error::QueryError::Internal(anyhow::anyhow!("set_language: {e}")))?;

    let mut out = Vec::new();
    let cap = max_results as usize;

    for rel_path in file_paths {
        if out.len() >= cap {
            break;
        }
        let abs = PathBuf::from(project_root).join(&rel_path);
        let Ok(source) = std::fs::read_to_string(&abs) else {
            continue;
        };
        let Some(tree) = parser.parse(source.as_bytes(), None) else {
            continue;
        };
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        while let Some(m) = matches.next() {
            for cap in m.captures {
                if out.len() >= max_results as usize {
                    break;
                }
                let node = cap.node;
                let start = node.start_position();
                let end = node.end_position();
                let snippet = node
                    .utf8_text(source.as_bytes())
                    .unwrap_or("")
                    .chars()
                    .take(120)
                    .collect::<String>();
                let capture_name = capture_names
                    .get(cap.index as usize)
                    .cloned()
                    .unwrap_or_default();
                out.push(PatternMatch {
                    file_path: rel_path.clone(),
                    start_line: (start.row + 1) as u32,
                    end_line: (end.row + 1) as u32,
                    start_col: start.column as u32,
                    end_col: end.column as u32,
                    node_kind: node.kind().to_string(),
                    capture_name,
                    snippet,
                });
            }
            if out.len() >= max_results as usize {
                break;
            }
        }
    }

    Ok(out)
}
