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
use anyhow::{Context, Result};
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
pub fn symbol_info(db: &Database, query: &str) -> Result<Vec<SymbolDetail>> {
    let conn = &db.conn;

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
        // This covers methods of a class, members of a namespace, etc.
        let children: Vec<SymbolSummary> = {
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
        };

        details.push(SymbolDetail {
            name,
            qualified_name,
            kind,
            file_path,
            start_line,
            end_line,
            signature,
            doc_comment,
            visibility,
            incoming_edge_count,
            outgoing_edge_count,
            children,
        });
    }

    Ok(details)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn insert_symbol_full(
        db: &Database,
        path: &str,
        name: &str,
        qname: &str,
        kind: &str,
        scope_path: Option<&str>,
        sig: Option<&str>,
        line: u32,
        end_line: u32,
    ) -> i64 {
        let conn = &db.conn;
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'csharp', 0)
             ON CONFLICT(path) DO NOTHING",
            [path],
        ).unwrap();
        let fid: i64 = conn.query_row("SELECT id FROM files WHERE path=?1", [path], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, end_line, col, scope_path, signature, visibility)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, 'public')",
            rusqlite::params![fid, name, qname, kind, line, end_line, scope_path, sig],
        ).unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn symbol_info_basic_lookup() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol_full(&db, "a.cs", "FooService", "App.FooService", "class", None, None, 1, 50);

        let details = symbol_info(&db, "FooService").unwrap();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].name, "FooService");
        assert_eq!(details[0].start_line, 1);
        assert_eq!(details[0].end_line, 50);
        assert_eq!(details[0].kind, "class");
    }

    #[test]
    fn symbol_info_by_qualified_name() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol_full(&db, "a.cs", "GetById", "App.FooService.GetById", "method", Some("App.FooService"), Some("Task<Foo> GetById(int id)"), 10, 20);

        let details = symbol_info(&db, "App.FooService.GetById").unwrap();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].qualified_name, "App.FooService.GetById");
        assert_eq!(details[0].signature.as_deref(), Some("Task<Foo> GetById(int id)"));
    }

    #[test]
    fn symbol_info_edge_counts() {
        let db = Database::open_in_memory().unwrap();
        let s1 = insert_symbol_full(&db, "a.cs", "Caller", "App.Caller", "method", None, None, 1, 5);
        let s2 = insert_symbol_full(&db, "a.cs", "Callee", "App.Callee", "method", None, None, 10, 15);

        db.conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)",
            rusqlite::params![s1, s2],
        ).unwrap();

        // Caller: 0 incoming, 1 outgoing.
        let caller_info = symbol_info(&db, "Caller").unwrap();
        assert_eq!(caller_info[0].incoming_edge_count, 0);
        assert_eq!(caller_info[0].outgoing_edge_count, 1);

        // Callee: 1 incoming, 0 outgoing.
        let callee_info = symbol_info(&db, "Callee").unwrap();
        assert_eq!(callee_info[0].incoming_edge_count, 1);
        assert_eq!(callee_info[0].outgoing_edge_count, 0);
    }

    #[test]
    fn symbol_info_children() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol_full(&db, "a.cs", "MyClass", "App.MyClass", "class", None, None, 1, 100);
        insert_symbol_full(&db, "a.cs", "DoWork", "App.MyClass.DoWork", "method", Some("App.MyClass"), None, 10, 20);
        insert_symbol_full(&db, "a.cs", "Helper", "App.MyClass.Helper", "method", Some("App.MyClass"), None, 25, 35);

        let info = symbol_info(&db, "MyClass").unwrap();
        assert_eq!(info[0].children.len(), 2);
        let child_names: Vec<&str> = info[0].children.iter().map(|c| c.name.as_str()).collect();
        assert!(child_names.contains(&"DoWork"));
        assert!(child_names.contains(&"Helper"));
    }

    #[test]
    fn symbol_info_returns_empty_for_unknown() {
        let db = Database::open_in_memory().unwrap();
        let result = symbol_info(&db, "NoSuchSymbol").unwrap();
        assert!(result.is_empty());
    }
}
