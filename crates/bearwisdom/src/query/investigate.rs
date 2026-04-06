// =============================================================================
// query/investigate.rs  —  composite "deep-dive" query
//
// Combines symbol_info + incoming_calls + outgoing_calls + blast_radius
// into a single round-trip.  Designed for LLM consumption — one tool call
// instead of four.
// =============================================================================

use crate::db::Database;
use crate::query::blast_radius::{self, AffectedSymbol};
use crate::query::call_hierarchy::{self, CallHierarchyItem};
use crate::query::QueryResult;
use anyhow::Context;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Controls the scope of an investigate query.
#[derive(Debug, Clone)]
pub struct InvestigateOptions {
    /// Maximum number of callers to return.
    pub caller_limit: usize,
    /// Maximum number of callees to return.
    pub callee_limit: usize,
    /// Blast radius traversal depth (1 = direct dependents only).
    pub blast_depth: u32,
}

impl Default for InvestigateOptions {
    fn default() -> Self {
        Self {
            caller_limit: 10,
            callee_limit: 10,
            blast_depth: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Slim symbol summary used as the center of an investigate result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlimSymbol {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line: u32,
    pub signature: Option<String>,
}

/// Combined result of an investigate query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigateResult {
    /// The symbol being investigated.
    pub symbol: SlimSymbol,
    /// Symbols that call this symbol (incoming call hierarchy).
    pub callers: Vec<CallHierarchyItem>,
    /// Symbols that this symbol calls (outgoing call hierarchy).
    pub callees: Vec<CallHierarchyItem>,
    /// Blast radius — what breaks if this symbol changes.
    pub blast_radius: Option<BlastRadiusSlim>,
}

/// Slim blast radius — just the count and affected list, no center repeat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlastRadiusSlim {
    pub total_affected: u32,
    pub affected: Vec<AffectedSymbol>,
}

// ---------------------------------------------------------------------------
// Public function
// ---------------------------------------------------------------------------

/// Deep-dive into a symbol: returns info, callers, callees, and blast radius.
///
/// `symbol_name` may be a simple name or fully-qualified name.
/// Returns `Ok(None)` if the symbol is not found.
pub fn investigate(
    db: &Database,
    symbol_name: &str,
    opts: &InvestigateOptions,
) -> QueryResult<Option<InvestigateResult>> {
    let _timer = db.timer("investigate");
    let conn = db.conn();

    // --- Resolve the symbol ---
    let lookup_sql = if symbol_name.contains('.') {
        "SELECT s.id, s.name, s.kind, f.path, s.line, s.signature
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.qualified_name = ?1
         LIMIT 1"
    } else {
        "SELECT s.id, s.name, s.kind, f.path, s.line, s.signature
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1
         ORDER BY s.qualified_name
         LIMIT 1"
    };

    let row = conn
        .query_row(lookup_sql, [symbol_name], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .optional()
        .context("investigate: symbol lookup")?;

    let Some((_id, name, kind, file_path, line, signature)) = row else {
        return Ok(None);
    };

    let symbol = SlimSymbol {
        name: name.clone(),
        kind,
        file_path,
        line,
        signature,
    };

    // --- Callers ---
    let callers = call_hierarchy::incoming_calls(db, &name, opts.caller_limit)
        .unwrap_or_default();

    // --- Callees ---
    let callees = call_hierarchy::outgoing_calls(db, &name, opts.callee_limit)
        .unwrap_or_default();

    // --- Blast radius ---
    let blast_radius = blast_radius::blast_radius(db, symbol_name, opts.blast_depth, 500)
        .ok()
        .flatten()
        .map(|br| BlastRadiusSlim {
            total_affected: br.total_affected,
            affected: br.affected,
        });

    Ok(Some(InvestigateResult {
        symbol,
        callers,
        callees,
        blast_radius,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_investigate_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result = investigate(&db, "nonexistent", &InvestigateOptions::default()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_investigate_found() {
        let db = Database::open_in_memory().unwrap();

        db.conn().execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/a.rs', 'h', 'rust', 0)",
            [],
        ).unwrap();
        let file_id = db.conn().last_insert_rowid();

        db.conn().execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, signature)
             VALUES (?1, 'do_work', 'mod::do_work', 'function', 10, 0, 'fn do_work()')",
            [file_id],
        ).unwrap();

        let result = investigate(&db, "do_work", &InvestigateOptions::default()).unwrap();
        assert!(result.is_some());

        let r = result.unwrap();
        assert_eq!(r.symbol.name, "do_work");
        assert_eq!(r.symbol.signature.as_deref(), Some("fn do_work()"));
        assert!(r.callers.is_empty());
        assert!(r.callees.is_empty());
    }
}
