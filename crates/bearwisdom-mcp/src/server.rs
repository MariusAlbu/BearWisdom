use bearwisdom::db::Database;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{schemars, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Format a structured JSON error response for MCP tool calls.
fn error_response(code: &str, message: &str) -> String {
    serde_json::json!({
        "error": {
            "code": code,
            "message": message,
        }
    })
    .to_string()
}

// =============================================================================
// MCP tool parameter types (schemars generates JSON Schema for Claude Code)
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// Search keywords (symbol names, words from signatures or doc comments)
    pub query: String,
    /// Maximum results to return (default: 15)
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GrepParams {
    /// Literal substring or regex pattern to search for in source files
    pub pattern: String,
    /// Use regex matching (default: false, literal substring)
    pub regex: Option<bool>,
    /// Case-insensitive search (default: false)
    pub case_insensitive: Option<bool>,
    /// Whole-word matching only (default: false)
    pub whole_word: Option<bool>,
    /// Filter by language tag (e.g. "rust", "typescript")
    pub language: Option<String>,
    /// Maximum results to return (default: 30)
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SymbolInfoParams {
    /// Symbol name or qualified name (e.g. 'DoWork', 'App.Svc.DoWork')
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindReferencesParams {
    /// Symbol name to find references for
    pub name: String,
    /// Maximum results to return (default: 100)
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CallHierarchyParams {
    /// Function or method name
    pub name: String,
    /// Direction: "in" for callers, "out" for callees (default: "in")
    pub direction: Option<String>,
    /// Maximum results to return (default: 50)
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileSymbolsParams {
    /// Relative file path within the project
    pub file_path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlastRadiusParams {
    /// Symbol name or qualified name to analyze impact for
    pub symbol: String,
    /// Max traversal depth (default: 3)
    pub depth: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArchitectureParams {}

// =============================================================================
// MCP Server handler
// =============================================================================

#[derive(Clone)]
pub struct BearWisdomServer {
    db: Arc<Mutex<Database>>,
    project_root: PathBuf,
    tool_router: ToolRouter<Self>,
}

impl BearWisdomServer {
    pub fn new(db: Arc<Mutex<Database>>, project_root: PathBuf) -> Self {
        Self {
            db,
            project_root,
            tool_router: Self::tool_router(),
        }
    }

    /// Acquire the database, returning a structured error if the mutex is poisoned.
    fn lock_db(&self) -> Result<std::sync::MutexGuard<'_, Database>, String> {
        self.db
            .lock()
            .map_err(|e| error_response("INTERNAL_ERROR", &format!("Database lock poisoned: {e}")))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for BearWisdomServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "BearWisdom code intelligence: search symbols, grep source, inspect call hierarchies, \
             find references, analyze blast radius, and get architecture overviews for the indexed project.",
        )
    }
}

#[tool_router]
impl BearWisdomServer {
    /// Search code symbols (functions, classes, methods, types) by keyword.
    /// Returns ranked results with file location, kind, and signature.
    /// Use for finding symbols by name or partial name.
    /// Do NOT use for substring search in raw source — use bw_grep instead.
    #[tool(name = "bw_search")]
    fn search(&self, Parameters(params): Parameters<SearchParams>) -> String {
        if params.query.trim().is_empty() {
            return error_response("INVALID_INPUT", "Query cannot be empty");
        }
        let db = match self.lock_db() {
            Ok(d) => d,
            Err(e) => return e,
        };
        let limit = params.limit.unwrap_or(15);
        match bearwisdom::query::search::search_symbols(&db, &params.query, limit) {
            Ok(results) => serde_json::to_string(&results)
                .unwrap_or_else(|e| error_response("SERIALIZATION_ERROR", &format!("{e}"))),
            Err(e) => error_response("QUERY_ERROR", &format!("{e}")),
        }
    }

    /// Fast substring or regex search across all source files.
    /// Returns matching lines with file path and line number.
    /// Respects .gitignore. Use for finding exact text patterns or code snippets.
    /// Do NOT use for semantic queries — use bw_search instead.
    #[tool(name = "bw_grep")]
    fn grep(&self, Parameters(params): Parameters<GrepParams>) -> String {
        if params.pattern.is_empty() {
            return error_response("INVALID_INPUT", "Pattern cannot be empty");
        }
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut scope = bearwisdom::search::scope::SearchScope::default();
        if let Some(lang) = &params.language {
            scope = scope.with_language(lang);
        }
        let options = bearwisdom::search::grep::GrepOptions {
            regex: params.regex.unwrap_or(false),
            case_sensitive: !params.case_insensitive.unwrap_or(false),
            whole_word: params.whole_word.unwrap_or(false),
            max_results: params.limit.unwrap_or(30),
            scope,
            context_lines: 0,
        };
        match bearwisdom::search::grep::grep_search(
            &self.project_root,
            &params.pattern,
            &options,
            &cancelled,
        ) {
            Ok(results) => serde_json::to_string(&results)
                .unwrap_or_else(|e| error_response("SERIALIZATION_ERROR", &format!("{e}"))),
            Err(e) => error_response("QUERY_ERROR", &format!("{e}")),
        }
    }

    /// Get detailed information about a specific code symbol: signature, doc comment,
    /// location, visibility, and edge counts. Use after bw_search to drill into a result.
    #[tool(name = "bw_symbol_info")]
    fn symbol_info(&self, Parameters(params): Parameters<SymbolInfoParams>) -> String {
        if params.name.is_empty() {
            return error_response("INVALID_INPUT", "Symbol name cannot be empty");
        }
        let db = match self.lock_db() {
            Ok(d) => d,
            Err(e) => return e,
        };
        match bearwisdom::query::symbol_info::symbol_info(&db, &params.name) {
            Ok(results) => serde_json::to_string(&results)
                .unwrap_or_else(|e| error_response("SERIALIZATION_ERROR", &format!("{e}"))),
            Err(e) => error_response("QUERY_ERROR", &format!("{e}")),
        }
    }

    /// Find all locations where a symbol is referenced across the codebase.
    /// Returns each reference with file path, line number, referencing symbol, and edge kind.
    /// Use to understand impact before changing a symbol.
    #[tool(name = "bw_find_references")]
    fn find_references(&self, Parameters(params): Parameters<FindReferencesParams>) -> String {
        if params.name.is_empty() {
            return error_response("INVALID_INPUT", "Symbol name cannot be empty");
        }
        let db = match self.lock_db() {
            Ok(d) => d,
            Err(e) => return e,
        };
        let limit = params.limit.unwrap_or(100);
        match bearwisdom::query::references::find_references(&db, &params.name, limit) {
            Ok(results) => serde_json::to_string(&results)
                .unwrap_or_else(|e| error_response("SERIALIZATION_ERROR", &format!("{e}"))),
            Err(e) => error_response("QUERY_ERROR", &format!("{e}")),
        }
    }

    /// Show the call hierarchy for a function or method.
    /// Direction "in" shows callers (who calls this?), "out" shows callees (what does this call?).
    /// Use to trace execution flow or understand coupling.
    #[tool(name = "bw_call_hierarchy")]
    fn call_hierarchy(&self, Parameters(params): Parameters<CallHierarchyParams>) -> String {
        if params.name.is_empty() {
            return error_response("INVALID_INPUT", "Symbol name cannot be empty");
        }
        let db = match self.lock_db() {
            Ok(d) => d,
            Err(e) => return e,
        };
        let limit = params.limit.unwrap_or(50);
        let result = match params.direction.as_deref() {
            Some("out") => {
                bearwisdom::query::call_hierarchy::outgoing_calls(&db, &params.name, limit)
            }
            _ => bearwisdom::query::call_hierarchy::incoming_calls(&db, &params.name, limit),
        };
        match result {
            Ok(items) => serde_json::to_string(&items)
                .unwrap_or_else(|e| error_response("SERIALIZATION_ERROR", &format!("{e}"))),
            Err(e) => error_response("QUERY_ERROR", &format!("{e}")),
        }
    }

    /// List all symbols defined in a file as a flat list with kind, line range, and signature.
    /// Use to understand a file's structure before reading it.
    /// Equivalent to a document outline / table of contents.
    #[tool(name = "bw_file_symbols")]
    fn file_symbols(&self, Parameters(params): Parameters<FileSymbolsParams>) -> String {
        if params.file_path.is_empty() {
            return error_response("INVALID_INPUT", "file_path is required");
        }
        let db = match self.lock_db() {
            Ok(d) => d,
            Err(e) => return e,
        };
        // Query symbols for this file from the DB directly.
        let sql = "SELECT s.name, s.qualified_name, s.kind, s.line, s.end_line, \
                          s.signature, s.visibility, s.scope_path \
                   FROM symbols s JOIN files f ON s.file_id = f.id \
                   WHERE f.path = ?1 \
                   ORDER BY s.line";
        let result: Result<Vec<serde_json::Value>, _> = (|| {
            let mut stmt = db.conn.prepare(sql)?;
            let rows = stmt.query_map([&params.file_path], |row| {
                Ok(serde_json::json!({
                    "name": row.get::<_, String>(0)?,
                    "qualified_name": row.get::<_, String>(1)?,
                    "kind": row.get::<_, String>(2)?,
                    "line": row.get::<_, u32>(3)?,
                    "end_line": row.get::<_, Option<u32>>(4)?,
                    "signature": row.get::<_, Option<String>>(5)?,
                    "visibility": row.get::<_, Option<String>>(6)?,
                    "scope_path": row.get::<_, Option<String>>(7)?,
                }))
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(anyhow::Error::from)
        })();
        match result {
            Ok(symbols) => serde_json::to_string(&symbols)
                .unwrap_or_else(|e| error_response("SERIALIZATION_ERROR", &format!("{e}"))),
            Err(e) => error_response("QUERY_ERROR", &format!("{e}")),
        }
    }

    /// Analyze the blast radius of changing a specific symbol.
    /// Shows which symbols are transitively affected (callers, implementors, type users).
    /// Use before modifying a symbol to understand what might break.
    #[tool(name = "bw_blast_radius")]
    fn blast_radius(&self, Parameters(params): Parameters<BlastRadiusParams>) -> String {
        if params.symbol.is_empty() {
            return error_response("INVALID_INPUT", "Symbol name cannot be empty");
        }
        let db = match self.lock_db() {
            Ok(d) => d,
            Err(e) => return e,
        };
        let depth = params.depth.unwrap_or(3).min(10).max(1);
        match bearwisdom::query::blast_radius::blast_radius(&db, &params.symbol, depth) {
            Ok(result) => serde_json::to_string(&result)
                .unwrap_or_else(|e| error_response("SERIALIZATION_ERROR", &format!("{e}"))),
            Err(e) => error_response("QUERY_ERROR", &format!("{e}")),
        }
    }

    /// Get a high-level summary of the indexed project: languages, file/symbol counts,
    /// hotspots (most-referenced symbols), and entry points.
    /// Use at the start of a session to understand the codebase.
    /// No parameters needed.
    #[tool(name = "bw_architecture_overview")]
    fn architecture_overview(
        &self,
        Parameters(_params): Parameters<ArchitectureParams>,
    ) -> String {
        let db = match self.lock_db() {
            Ok(d) => d,
            Err(e) => return e,
        };
        match bearwisdom::query::architecture::get_overview(&db) {
            Ok(report) => serde_json::to_string(&report)
                .unwrap_or_else(|e| error_response("SERIALIZATION_ERROR", &format!("{e}"))),
            Err(e) => error_response("QUERY_ERROR", &format!("{e}")),
        }
    }
}
