use bearwisdom::db::DbPool;
use bearwisdom::query::QueryOptions;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{schemars, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

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

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchParams {
    /// Search keywords (symbol names, words from signatures or doc comments)
    pub query: String,
    /// Maximum results (default: 10)
    pub limit: Option<usize>,
    /// Include function/method signatures in results (default: false)
    pub include_signature: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
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
    /// Maximum results (default: 20)
    pub limit: Option<usize>,
    /// Truncate lines longer than this (default: 120, 0 = unlimited)
    pub max_line_length: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SymbolInfoParams {
    /// Symbol name or qualified name (e.g. 'DoWork', 'App.Svc.DoWork')
    pub name: String,
    /// Include function/method signatures (default: false)
    pub include_signature: Option<bool>,
    /// Include doc comments (default: false)
    pub include_doc: Option<bool>,
    /// Include child symbols — methods of a class, etc. (default: false)
    pub include_children: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FindReferencesParams {
    /// Symbol name to find references for
    pub name: String,
    /// Maximum results (default: 20)
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CallHierarchyParams {
    /// Function or method name
    pub name: String,
    /// Direction: "in" for callers, "out" for callees (default: "in")
    pub direction: Option<String>,
    /// Maximum results (default: 20)
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FileSymbolsParams {
    /// Relative file path within the project
    pub file_path: String,
    /// Output mode: "names" (minimal), "outline" (default), "full" (all fields)
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BlastRadiusParams {
    /// Symbol name or qualified name to analyze impact for
    pub symbol: String,
    /// Max traversal depth (default: 2, max: 10)
    pub depth: Option<u32>,
    /// Maximum number of affected symbols to return (default: 500, max: 5000).
    /// When the cap is hit the response includes ``truncated: true``.
    pub max_results: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SmartContextParams {
    /// Natural-language task description (e.g. "add pagination to the catalog API")
    pub task: String,
    /// Token budget for the context (default: 8000)
    pub budget: Option<u32>,
    /// Graph expansion depth (default: 2)
    pub depth: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArchitectureParams {}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PackagesParams {}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceOverviewParams {}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct InvestigateParams {
    /// Symbol name or qualified name to investigate
    pub symbol: String,
    /// Max callers to return (default: 10)
    pub caller_limit: Option<usize>,
    /// Max callees to return (default: 10)
    pub callee_limit: Option<usize>,
    /// Blast radius depth (default: 1)
    pub blast_depth: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DiagnosticsParams {
    /// Relative file path to check for issues
    pub file_path: String,
    /// Confidence threshold for flagging edges (default: 0.80, lower = more strict)
    pub confidence_threshold: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CompleteAtParams {
    /// Relative file path
    pub file_path: String,
    /// 1-based line number of cursor position
    pub line: u32,
    /// 0-based column of cursor position
    pub col: u32,
    /// Prefix text typed so far (can be empty)
    pub prefix: String,
    /// Include signatures in results (default: false)
    pub include_signature: Option<bool>,
}

// =============================================================================
// MCP Server handler
// =============================================================================

#[derive(Clone)]
pub struct BearWisdomServer {
    pool: DbPool,
    project_root: PathBuf,
    session_id: Arc<str>,
    tool_router: ToolRouter<Self>,
}

impl BearWisdomServer {
    pub fn new(pool: DbPool, project_root: PathBuf) -> Self {
        let session_id: Arc<str> = uuid::Uuid::new_v4().to_string().into();
        tracing::info!("MCP audit session: {session_id}");
        Self {
            pool,
            project_root,
            session_id,
            tool_router: Self::tool_router(),
        }
    }

    /// Acquire a database connection from the pool, returning a structured error on failure.
    fn get_db(&self) -> Result<bearwisdom::PoolGuard, String> {
        self.pool
            .get()
            .map_err(|e| error_response("INTERNAL_ERROR", &format!("Pool error: {e}")))
    }

    /// Write one audit record.  Best-effort — a write failure must not propagate.
    fn audit_call(&self, tool: &str, params_json: &str, result: &str, duration_ms: u64) {
        if let Ok(db) = self.pool.get() {
            let token_estimate = (result.len() / 4) as i64;
            let _ = db.write_audit_record(
                &self.session_id,
                tool,
                params_json,
                result,
                duration_ms,
                token_estimate,
            );
        }
    }

    /// Shared dispatch helper for tools that acquire a db connection.
    ///
    /// Handles timing, audit, and pool errors. The closure receives a `PoolGuard`
    /// and returns `Ok(json_string)` on success or `Err(error_response_string)` on failure.
    fn run_tool<P, F>(&self, tool_name: &str, params: &P, f: F) -> String
    where
        P: Serialize,
        F: FnOnce(&bearwisdom::PoolGuard) -> Result<String, String>,
    {
        let t0 = std::time::Instant::now();
        let params_json = serde_json::to_string(params).unwrap_or_default();
        let db = match self.get_db() {
            Ok(d) => d,
            Err(e) => {
                self.audit_call(tool_name, &params_json, &e, t0.elapsed().as_millis() as u64);
                return e;
            }
        };
        let result = f(&db).unwrap_or_else(|e| e);
        self.audit_call(tool_name, &params_json, &result, t0.elapsed().as_millis() as u64);
        result
    }

    /// Shared dispatch helper for tools that do NOT need a db connection (e.g. bw_grep).
    ///
    /// Handles timing and audit only. The closure returns `Ok(json_string)` or
    /// `Err(error_response_string)`.
    fn run_tool_no_db<P, F>(&self, tool_name: &str, params: &P, f: F) -> String
    where
        P: Serialize,
        F: FnOnce() -> Result<String, String>,
    {
        let t0 = std::time::Instant::now();
        let params_json = serde_json::to_string(params).unwrap_or_default();
        let result = f().unwrap_or_else(|e| e);
        self.audit_call(tool_name, &params_json, &result, t0.elapsed().as_millis() as u64);
        result
    }

    /// Return an `Err(error_response)` for a missing/empty required input field.
    /// Intended for use as an early-return inside a `run_tool` / `run_tool_no_db` closure.
    fn invalid_input(message: &str) -> Result<String, String> {
        Err(error_response("INVALID_INPUT", message))
    }

    /// Serialize a query result to JSON, mapping serialization failures to an error response.
    fn to_json<T: Serialize>(value: &T) -> Result<String, String> {
        serde_json::to_string(value)
            .map_err(|e| error_response("SERIALIZATION_ERROR", &format!("{e}")))
    }

    /// Map a `QueryError` to a structured error response string.
    ///
    /// Variant-specific codes let callers distinguish retryable and actionable errors:
    ///   - `NOT_INDEXED`    — no index exists yet; caller should index first.
    ///   - `NOT_FOUND`      — the requested symbol/file does not exist in the index.
    ///   - `DATABASE_BUSY`  — SQLite lock contention; caller may retry after a short delay.
    ///   - `QUERY_ERROR`    — internal error (schema mismatch, I/O, etc.).
    fn query_err(e: bearwisdom::QueryError) -> String {
        match e {
            bearwisdom::QueryError::NotIndexed => error_response(
                "NOT_INDEXED",
                "The project has not been indexed yet. Run `bw open <path>` first.",
            ),
            bearwisdom::QueryError::NotFound(ref name) => {
                error_response("NOT_FOUND", &format!("Not found: {name}"))
            }
            bearwisdom::QueryError::DatabaseBusy => error_response(
                "DATABASE_BUSY",
                "Database is busy (another writer holds the lock). Retry after a short delay.",
            ),
            bearwisdom::QueryError::Internal(ref inner) => {
                error_response("QUERY_ERROR", &format!("{inner}"))
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for BearWisdomServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "BearWisdom code intelligence: search symbols, grep source, inspect call hierarchies, \
             find references, analyze blast radius, and get architecture overviews for the indexed project. \
             Use bw_investigate for a combined deep-dive into any symbol.",
        )
    }
}

#[tool_router]
impl BearWisdomServer {
    /// Search code symbols by keyword. Returns ~10 results with name, kind, file, line.
    /// Pass include_signature: true for full signatures. Use bw_grep for raw text search.
    #[tool(name = "bw_search")]
    fn search(&self, Parameters(params): Parameters<SearchParams>) -> String {
        self.run_tool("bw_search", &params, |db| {
            if params.query.trim().is_empty() {
                return Self::invalid_input("Query cannot be empty");
            }
            let limit = params.limit.unwrap_or(10);
            let opts = QueryOptions {
                include_signature: params.include_signature.unwrap_or(false),
                ..QueryOptions::default()
            };
            bearwisdom::query::search::search_symbols(db, &params.query, limit, &opts)
                .map_err(Self::query_err)
                .and_then(|r| Self::to_json(&r))
        })
    }

    /// Fast substring or regex search across source files. Returns ~20 matching lines.
    /// Lines truncated to 120 chars by default (pass max_line_length: 0 for full lines).
    /// Use bw_search for semantic symbol lookup.
    #[tool(name = "bw_grep")]
    fn grep(&self, Parameters(params): Parameters<GrepParams>) -> String {
        let root = self.project_root.clone();
        self.run_tool_no_db("bw_grep", &params, || {
            if params.pattern.is_empty() {
                return Self::invalid_input("Pattern cannot be empty");
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
                max_results: params.limit.unwrap_or(20),
                scope,
                context_lines: 0,
            };
            let mut results =
                bearwisdom::search::grep::grep_search(&root, &params.pattern, &options, &cancelled)
                    .map_err(|e| error_response("QUERY_ERROR", &format!("{e}")))?;
            let max_len = params.max_line_length.unwrap_or(120);
            bearwisdom::search::grep::truncate_matches(&mut results, max_len);
            Self::to_json(&results)
        })
    }

    /// Get symbol details: location, edge counts, visibility. Returns slim output by default.
    /// Pass include_signature/include_doc/include_children: true for richer data.
    #[tool(name = "bw_symbol_info")]
    fn symbol_info(&self, Parameters(params): Parameters<SymbolInfoParams>) -> String {
        self.run_tool("bw_symbol_info", &params, |db| {
            if params.name.is_empty() {
                return Self::invalid_input("Symbol name cannot be empty");
            }
            let opts = QueryOptions {
                include_signature: params.include_signature.unwrap_or(false),
                include_doc: params.include_doc.unwrap_or(false),
                include_children: params.include_children.unwrap_or(false),
                ..QueryOptions::default()
            };
            bearwisdom::query::symbol_info::symbol_info_json(db, &params.name, &opts)
                .map_err(Self::query_err)
        })
    }

    /// Find all references to a symbol. Returns ~20 results with file, line, edge kind.
    #[tool(name = "bw_find_references")]
    fn find_references(&self, Parameters(params): Parameters<FindReferencesParams>) -> String {
        self.run_tool("bw_find_references", &params, |db| {
            if params.name.is_empty() {
                return Self::invalid_input("Symbol name cannot be empty");
            }
            let limit = params.limit.unwrap_or(20);
            bearwisdom::query::references::find_references_json(db, &params.name, limit)
                .map_err(Self::query_err)
        })
    }

    /// Show call hierarchy: "in" = who calls this, "out" = what does this call. ~20 results.
    #[tool(name = "bw_call_hierarchy")]
    fn call_hierarchy(&self, Parameters(params): Parameters<CallHierarchyParams>) -> String {
        self.run_tool("bw_call_hierarchy", &params, |db| {
            if params.name.is_empty() {
                return Self::invalid_input("Symbol name cannot be empty");
            }
            let limit = params.limit.unwrap_or(20);
            let query_result = match params.direction.as_deref() {
                Some("out") => {
                    bearwisdom::query::call_hierarchy::outgoing_calls(db, &params.name, limit)
                }
                _ => bearwisdom::query::call_hierarchy::incoming_calls(db, &params.name, limit),
            };
            query_result
                .map_err(Self::query_err)
                .and_then(|r| Self::to_json(&r))
        })
    }

    /// List symbols in a file. Modes: "names" (minimal), "outline" (default), "full".
    #[tool(name = "bw_file_symbols")]
    fn file_symbols(&self, Parameters(params): Parameters<FileSymbolsParams>) -> String {
        self.run_tool("bw_file_symbols", &params, |db| {
            if params.file_path.is_empty() {
                return Self::invalid_input("file_path is required");
            }
            let mode = bearwisdom::query::symbol_info::FileSymbolsMode::from_str(
                params.mode.as_deref().unwrap_or("outline"),
            );
            bearwisdom::query::symbol_info::file_symbols(db, &params.file_path, mode)
                .map_err(Self::query_err)
                .and_then(|r| Self::to_json(&r))
        })
    }

    /// Blast radius: what breaks if this symbol changes? Default depth 2.
    #[tool(name = "bw_blast_radius")]
    fn blast_radius(&self, Parameters(params): Parameters<BlastRadiusParams>) -> String {
        self.run_tool("bw_blast_radius", &params, |db| {
            if params.symbol.is_empty() {
                return Self::invalid_input("Symbol name cannot be empty");
            }
            let depth = params.depth.unwrap_or(2).min(10).max(1);
            bearwisdom::query::blast_radius::blast_radius(db, &params.symbol, depth, 500)
                .map_err(Self::query_err)
                .and_then(|r| Self::to_json(&r))
        })
    }

    /// High-level project summary: languages, file/symbol counts, top hotspots, entry points.
    #[tool(name = "bw_architecture_overview")]
    fn architecture_overview(
        &self,
        Parameters(params): Parameters<ArchitectureParams>,
    ) -> String {
        self.run_tool("bw_architecture_overview", &params, |db| {
            bearwisdom::query::architecture::get_overview(db)
                .map_err(Self::query_err)
                .and_then(|r| Self::to_json(&r))
        })
    }

    /// List detected packages with file/symbol/edge counts.
    /// Returns an empty array for single-project repos — no error.
    #[tool(name = "bw_packages")]
    fn packages(&self, Parameters(params): Parameters<PackagesParams>) -> String {
        self.run_tool("bw_packages", &params, |db| {
            bearwisdom::list_packages(db)
                .map_err(Self::query_err)
                .and_then(|r| Self::to_json(&r))
        })
    }

    /// Workspace overview: per-package breakdown + cross-package edge count + shared hotspots.
    /// Returns empty/zero fields for single-project repos — no error.
    #[tool(name = "bw_workspace_overview")]
    fn workspace_overview(&self, Parameters(params): Parameters<WorkspaceOverviewParams>) -> String {
        self.run_tool("bw_workspace_overview", &params, |db| {
            bearwisdom::workspace_overview(db)
                .map_err(Self::query_err)
                .and_then(|r| Self::to_json(&r))
        })
    }

    /// Get diagnostics for a file: unresolved symbols + low-confidence edges.
    #[tool(name = "bw_diagnostics")]
    fn diagnostics(&self, Parameters(params): Parameters<DiagnosticsParams>) -> String {
        self.run_tool("bw_diagnostics", &params, |db| {
            if params.file_path.is_empty() {
                return Self::invalid_input("file_path is required");
            }
            let threshold = params.confidence_threshold.unwrap_or(
                bearwisdom::query::diagnostics::LOW_CONFIDENCE_THRESHOLD,
            );
            bearwisdom::query::diagnostics::get_diagnostics(db, &params.file_path, threshold)
                .map_err(Self::query_err)
                .and_then(|r| Self::to_json(&r))
        })
    }

    /// Auto-complete symbols at a cursor position. Returns scope-aware candidates ranked by distance and relevance.
    #[tool(name = "bw_complete")]
    fn complete_at(&self, Parameters(params): Parameters<CompleteAtParams>) -> String {
        self.run_tool("bw_complete", &params, |db| {
            bearwisdom::query::completion::complete_at(
                db,
                &params.file_path,
                params.line,
                params.col,
                &params.prefix,
                params.include_signature.unwrap_or(false),
            )
            .map_err(Self::query_err)
            .and_then(|r| Self::to_json(&r))
        })
    }

    /// Build smart context for a task: returns the most relevant symbols, files, and concepts
    /// to include in the LLM context window. Uses semantic search + graph expansion + scoring.
    #[tool(name = "bw_context")]
    fn smart_context(&self, Parameters(params): Parameters<SmartContextParams>) -> String {
        self.run_tool("bw_context", &params, |db| {
            if params.task.trim().is_empty() {
                return Self::invalid_input("Task description cannot be empty");
            }
            let budget = params.budget.unwrap_or(8000);
            let depth = params.depth.unwrap_or(2);
            bearwisdom::query::context::smart_context(db, &params.task, budget, depth)
                .map_err(Self::query_err)
                .and_then(|r| Self::to_json(&r))
        })
    }

    /// Deep-dive a symbol in one call: info + callers + callees + blast radius.
    /// Use this instead of calling bw_symbol_info + bw_call_hierarchy + bw_blast_radius separately.
    #[tool(name = "bw_investigate")]
    fn investigate(&self, Parameters(params): Parameters<InvestigateParams>) -> String {
        self.run_tool("bw_investigate", &params, |db| {
            if params.symbol.is_empty() {
                return Self::invalid_input("Symbol name cannot be empty");
            }
            let opts = bearwisdom::query::investigate::InvestigateOptions {
                caller_limit: params.caller_limit.unwrap_or(10),
                callee_limit: params.callee_limit.unwrap_or(10),
                blast_depth: params.blast_depth.unwrap_or(1),
            };
            bearwisdom::query::investigate::investigate(db, &params.symbol, &opts)
                .map_err(Self::query_err)
                .and_then(|r| Self::to_json(&r))
        })
    }
}
