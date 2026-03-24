// =============================================================================
// BearWisdom CLI
//
// Exposes the bearwisdom code intelligence engine as a command-line tool.
// All output is JSON to stdout.  Errors are {"ok":false,"error":"..."}.
// Success payloads include {"ok":true,"data":{...}} or {"ok":true,"data":[...]}.
//
// DB path resolution:
//   ~/.bearwisdom/indexes/<first-16-hex-chars-of-sha256(canonical-path)>/index.db
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::{
    atomic::AtomicBool,
    Arc,
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};

use bearwisdom::db::Database;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "bw",
    version,
    about = "BearWisdom code intelligence engine — tree-sitter + SQLite"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    // ---- Lifecycle ---------------------------------------------------------
    /// Index a project (full re-index) and open the database.
    /// Also discovers and auto-assigns namespace concepts.
    Open {
        /// Absolute path to the project root.
        path: String,
    },

    /// Show index status for a project (state, file count, symbol count, edge count).
    /// Opens the existing DB read-only; does NOT re-index.
    Status {
        /// Absolute path to the project root.
        path: String,
    },

    // ---- Symbol search -----------------------------------------------------
    /// Full-text symbol search (FTS5 BM25).  Supports prefixes with *.
    SearchSymbols {
        /// Absolute path to the project root.
        path: String,
        /// FTS5 query (e.g. "GetById", "Catalog*", "\"get items\"").
        query: String,
        /// Maximum results (default: 20).
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Fuzzy file search (nucleo — Ctrl+P equivalent).
    FuzzyFiles {
        /// Absolute path to the project root.
        path: String,
        /// Pattern to match against file paths.
        pattern: String,
        /// Maximum results (default: 20).
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Fuzzy symbol search (nucleo — Ctrl+T equivalent).
    FuzzySymbols {
        /// Absolute path to the project root.
        path: String,
        /// Pattern to match against qualified symbol names.
        pattern: String,
        /// Maximum results (default: 20).
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    // ---- Content search ----------------------------------------------------
    /// FTS5 trigram content search (file-level matches).  Minimum 3 chars.
    SearchContent {
        /// Absolute path to the project root.
        path: String,
        /// Substring to search for in indexed file content.
        query: String,
        /// Maximum results (default: 20).
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Grep across the project tree (.gitignore-aware).
    Grep {
        /// Absolute path to the project root.
        path: String,
        /// Pattern to search for (literal by default).
        pattern: String,
        /// Treat pattern as a regular expression.
        #[arg(long)]
        regex: bool,
        /// Case-insensitive matching.
        #[arg(long)]
        case_insensitive: bool,
        /// Match whole words only.
        #[arg(long)]
        whole_word: bool,
        /// Restrict to a single language tag (e.g. "typescript", "csharp").
        #[arg(long)]
        lang: Option<String>,
        /// Maximum results (default: 200).
        #[arg(long, default_value = "200")]
        limit: usize,
    },

    /// Hybrid search combining FTS5 trigram and KNN vector results via RRF.
    /// Falls back to FTS5-only when the ONNX model is unavailable.
    Hybrid {
        /// Absolute path to the project root.
        path: String,
        /// Natural-language or code query.
        query: String,
        /// Maximum results (default: 20).
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    // ---- Navigation --------------------------------------------------------
    /// List all symbols defined in a specific file.
    FileSymbols {
        /// Absolute path to the project root.
        path: String,
        /// Relative file path (forward-slash, relative to project root).
        file: String,
    },

    /// Go-to-definition by symbol name or qualified name.
    Definition {
        /// Absolute path to the project root.
        path: String,
        /// Symbol name (simple or fully qualified, e.g. "GetById" or "Catalog.Service.GetById").
        symbol: String,
    },

    /// Find all references to a symbol (by name or qualified name).
    References {
        /// Absolute path to the project root.
        path: String,
        /// Symbol name or qualified name.
        symbol: String,
        /// Maximum results (default: 100).
        #[arg(long, default_value = "100")]
        limit: usize,
    },

    // ---- Architecture ------------------------------------------------------
    /// High-level architecture overview: totals, per-language stats, hotspots, entry points.
    Architecture {
        /// Absolute path to the project root.
        path: String,
    },

    /// Blast radius analysis: which symbols would be affected by changing this one?
    BlastRadius {
        /// Absolute path to the project root.
        path: String,
        /// Symbol name or qualified name to analyze.
        symbol: String,
        /// Maximum graph traversal depth (default: 3).
        #[arg(long, default_value = "3")]
        depth: u32,
    },

    /// Incoming call hierarchy: who calls this symbol?
    CallsIn {
        /// Absolute path to the project root.
        path: String,
        /// Symbol name or qualified name.
        symbol: String,
        /// Maximum results (default: 50, 0 = unlimited).
        #[arg(long, default_value = "50")]
        limit: usize,
    },

    /// Outgoing call hierarchy: what does this symbol call?
    CallsOut {
        /// Absolute path to the project root.
        path: String,
        /// Symbol name or qualified name.
        symbol: String,
        /// Maximum results (default: 50, 0 = unlimited).
        #[arg(long, default_value = "50")]
        limit: usize,
    },

    /// Detailed symbol information: kind, location, signature, doc comment, edge counts, children.
    SymbolInfo {
        /// Absolute path to the project root.
        path: String,
        /// Symbol name or qualified name.
        symbol: String,
    },

    // ---- Concepts ----------------------------------------------------------
    /// List all discovered domain concepts with member counts.
    Concepts {
        /// Absolute path to the project root.
        path: String,
    },

    /// Auto-discover namespace concepts from qualified names.
    DiscoverConcepts {
        /// Absolute path to the project root.
        path: String,
    },

    /// List symbols that belong to a concept.
    ConceptMembers {
        /// Absolute path to the project root.
        path: String,
        /// Concept name (e.g. "Microsoft.eShop" or "App.Catalog").
        concept: String,
        /// Maximum results (default: 100, 0 = unlimited).
        #[arg(long, default_value = "100")]
        limit: usize,
    },

    // ---- Graph export ------------------------------------------------------
    /// Export the symbol graph as nodes + edges (JSON).
    ExportGraph {
        /// Absolute path to the project root.
        path: String,
        /// Optional filter: qualified-name prefix ("App.Catalog") or concept ("@auth").
        #[arg(long)]
        filter: Option<String>,
        /// Maximum nodes to export (default: 500, hard cap: 10 000).
        #[arg(long, default_value = "500")]
        max_nodes: usize,
    },

    // ---- Flow --------------------------------------------------------------
    /// Trace the cross-language flow graph from a file + line.
    TraceFlow {
        /// Absolute path to the project root.
        path: String,
        /// Relative file path (forward-slash).
        file: String,
        /// Source line number (1-based).
        line: u32,
        /// Maximum traversal depth (default: 5).
        #[arg(long, default_value = "5")]
        depth: u32,
    },
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    // Initialise tracing to stderr so stdout stays clean JSON.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();

    let result = run(cli.command);

    match result {
        Ok(json) => println!("{json}"),
        Err(e) => {
            let msg = format!("{e:#}");
            // Ensure we still print valid JSON even on error.
            println!("{}", serde_json::json!({"ok": false, "error": msg}));
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

fn run(command: Commands) -> Result<String> {
    match command {
        Commands::Open { path } => cmd_open(&path),
        Commands::Status { path } => cmd_status(&path),

        Commands::SearchSymbols { path, query, limit } => {
            cmd_search_symbols(&path, &query, limit)
        }
        Commands::FuzzyFiles { path, pattern, limit } => {
            cmd_fuzzy_files(&path, &pattern, limit)
        }
        Commands::FuzzySymbols { path, pattern, limit } => {
            cmd_fuzzy_symbols(&path, &pattern, limit)
        }

        Commands::SearchContent { path, query, limit } => {
            cmd_search_content(&path, &query, limit)
        }
        Commands::Grep {
            path,
            pattern,
            regex,
            case_insensitive,
            whole_word,
            lang,
            limit,
        } => cmd_grep(&path, &pattern, regex, !case_insensitive, whole_word, lang.as_deref(), limit),
        Commands::Hybrid { path, query, limit } => cmd_hybrid(&path, &query, limit),

        Commands::FileSymbols { path, file } => cmd_file_symbols(&path, &file),
        Commands::Definition { path, symbol } => cmd_definition(&path, &symbol),
        Commands::References { path, symbol, limit } => cmd_references(&path, &symbol, limit),

        Commands::Architecture { path } => cmd_architecture(&path),
        Commands::BlastRadius { path, symbol, depth } => cmd_blast_radius(&path, &symbol, depth),
        Commands::CallsIn { path, symbol, limit } => cmd_calls_in(&path, &symbol, limit),
        Commands::CallsOut { path, symbol, limit } => cmd_calls_out(&path, &symbol, limit),
        Commands::SymbolInfo { path, symbol } => cmd_symbol_info(&path, &symbol),

        Commands::Concepts { path } => cmd_concepts(&path),
        Commands::DiscoverConcepts { path } => cmd_discover_concepts(&path),
        Commands::ConceptMembers { path, concept, limit } => {
            cmd_concept_members(&path, &concept, limit)
        }

        Commands::ExportGraph { path, filter, max_nodes } => {
            cmd_export_graph(&path, filter.as_deref(), max_nodes)
        }
        Commands::TraceFlow { path, file, line, depth } => {
            cmd_trace_flow(&path, &file, line, depth)
        }
    }
}

// ---------------------------------------------------------------------------
// Lifecycle helpers
// ---------------------------------------------------------------------------

/// Open and fully index the project, then print stats.
fn cmd_open(project_path: &str) -> Result<String> {
    let root = PathBuf::from(project_path);
    let db_path = resolve_db_path(&root)?;

    eprintln!("Opening database at {}", db_path.display());

    let mut db = Database::open_with_vec(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    eprintln!("Running full index for {} ...", root.display());

    let stats = bearwisdom::full_index(&mut db, &root, None, None)
        .context("Full index failed")?;

    // Auto-discover namespace concepts and assign members.
    if let Err(e) = bearwisdom::query::concepts::discover_concepts(&db) {
        eprintln!("Warning: discover_concepts failed: {e}");
    }
    if let Err(e) = bearwisdom::query::concepts::auto_assign_concepts(&db) {
        eprintln!("Warning: auto_assign_concepts failed: {e}");
    }

    ok_json(serde_json::json!({
        "db_path": db_path.display().to_string(),
        "file_count": stats.file_count,
        "symbol_count": stats.symbol_count,
        "edge_count": stats.edge_count,
        "unresolved_ref_count": stats.unresolved_ref_count,
        "duration_ms": stats.duration_ms,
    }))
}

/// Report the current index status without re-indexing.
fn cmd_status(project_path: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let conn = &db.conn;

    let files: u32 =
        conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap_or(0);
    let symbols: u32 =
        conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .unwrap_or(0);
    let edges: u32 =
        conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap_or(0);
    let unresolved: u32 =
        conn.query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0))
            .unwrap_or(0);

    ok_json(serde_json::json!({
        "state": "ready",
        "file_count": files,
        "symbol_count": symbols,
        "edge_count": edges,
        "unresolved_ref_count": unresolved,
    }))
}

// ---------------------------------------------------------------------------
// Symbol search
// ---------------------------------------------------------------------------

fn cmd_search_symbols(project_path: &str, query: &str, limit: usize) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let results = bearwisdom::query::search::search_symbols(&db, query, limit)
        .context("search_symbols failed")?;
    ok_json(results)
}

fn cmd_fuzzy_files(project_path: &str, pattern: &str, limit: usize) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let idx = bearwisdom::search::fuzzy::FuzzyIndex::from_db(&db)
        .context("Failed to build FuzzyIndex")?;
    let results = idx.match_files(pattern, limit);
    ok_json(results)
}

fn cmd_fuzzy_symbols(project_path: &str, pattern: &str, limit: usize) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let idx = bearwisdom::search::fuzzy::FuzzyIndex::from_db(&db)
        .context("Failed to build FuzzyIndex")?;
    let results = idx.match_symbols(pattern, limit);
    ok_json(results)
}

// ---------------------------------------------------------------------------
// Content search
// ---------------------------------------------------------------------------

fn cmd_search_content(project_path: &str, query: &str, limit: usize) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let scope = bearwisdom::search::scope::SearchScope::default();
    let results = bearwisdom::search::content_search::search_content(&db, query, &scope, limit)
        .context("search_content failed")?;
    ok_json(results)
}

fn cmd_grep(
    project_path: &str,
    pattern: &str,
    regex: bool,
    case_sensitive: bool,
    whole_word: bool,
    lang: Option<&str>,
    limit: usize,
) -> Result<String> {
    let root = PathBuf::from(project_path);
    let cancelled = Arc::new(AtomicBool::new(false));

    let mut scope = bearwisdom::search::scope::SearchScope::default();
    if let Some(l) = lang {
        scope = scope.with_language(l);
    }

    let options = bearwisdom::search::grep::GrepOptions {
        case_sensitive,
        whole_word,
        regex,
        max_results: limit,
        scope,
        context_lines: 0,
    };

    let results =
        bearwisdom::search::grep::grep_search(&root, pattern, &options, &cancelled)
            .context("grep_search failed")?;
    ok_json(results)
}

fn cmd_hybrid(project_path: &str, query: &str, limit: usize) -> Result<String> {
    let db_path = resolve_db_path(&PathBuf::from(project_path))?;
    let db = Database::open_with_vec(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    // Resolve model directory: <project>/models/CodeRankEmbed or ~/.bearwisdom/models/CodeRankEmbed
    let root = PathBuf::from(project_path);
    let model_dir = resolve_model_dir(&root);

    let mut embedder = bearwisdom::search::embedder::Embedder::new(
        model_dir.unwrap_or_else(|| root.join("models").join("CodeRankEmbed")),
    );

    let scope = bearwisdom::search::scope::SearchScope::default();
    let results = bearwisdom::search::hybrid::hybrid_search(&db, &mut embedder, query, &scope, limit)
        .context("hybrid_search failed")?;
    ok_json(results)
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

fn cmd_file_symbols(project_path: &str, file_path: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let conn = &db.conn;

    let mut stmt = conn
        .prepare(
            "SELECT s.name, s.qualified_name, s.kind, s.line, s.col,
                    s.end_line, s.scope_path, s.signature, s.visibility
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path = ?1
             ORDER BY s.line",
        )
        .context("Failed to prepare file_symbols query")?;

    let rows = stmt
        .query_map([file_path], |row| {
            Ok(serde_json::json!({
                "name":          row.get::<_, String>(0)?,
                "qualified_name": row.get::<_, String>(1)?,
                "kind":          row.get::<_, String>(2)?,
                "line":          row.get::<_, u32>(3)?,
                "col":           row.get::<_, u32>(4)?,
                "end_line":      row.get::<_, Option<u32>>(5)?,
                "scope_path":    row.get::<_, Option<String>>(6)?,
                "signature":     row.get::<_, Option<String>>(7)?,
                "visibility":    row.get::<_, Option<String>>(8)?,
            }))
        })
        .context("Failed to execute file_symbols query")?;

    let results: Vec<_> = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect file_symbols rows")?;
    ok_json(results)
}

fn cmd_definition(project_path: &str, symbol: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let results = bearwisdom::query::definitions::goto_definition(&db, symbol)
        .context("goto_definition failed")?;
    ok_json(results)
}

fn cmd_references(project_path: &str, symbol: &str, limit: usize) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let results = bearwisdom::query::references::find_references(&db, symbol, limit)
        .context("find_references failed")?;
    ok_json(results)
}

// ---------------------------------------------------------------------------
// Architecture
// ---------------------------------------------------------------------------

fn cmd_architecture(project_path: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let overview = bearwisdom::query::architecture::get_overview(&db)
        .context("get_overview failed")?;
    ok_json(overview)
}

fn cmd_blast_radius(project_path: &str, symbol: &str, depth: u32) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let result = bearwisdom::query::blast_radius::blast_radius(&db, symbol, depth)
        .context("blast_radius failed")?;
    ok_json(result)
}

fn cmd_calls_in(project_path: &str, symbol: &str, limit: usize) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let results = bearwisdom::query::call_hierarchy::incoming_calls(&db, symbol, limit)
        .context("incoming_calls failed")?;
    ok_json(results)
}

fn cmd_calls_out(project_path: &str, symbol: &str, limit: usize) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let results = bearwisdom::query::call_hierarchy::outgoing_calls(&db, symbol, limit)
        .context("outgoing_calls failed")?;
    ok_json(results)
}

fn cmd_symbol_info(project_path: &str, symbol: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let results = bearwisdom::query::symbol_info::symbol_info(&db, symbol)
        .context("symbol_info failed")?;
    // Return first match or null.
    let first = results.into_iter().next();
    ok_json(first)
}

// ---------------------------------------------------------------------------
// Concepts
// ---------------------------------------------------------------------------

fn cmd_concepts(project_path: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let concepts = bearwisdom::query::concepts::list_concepts(&db)
        .context("list_concepts failed")?;
    ok_json(concepts)
}

fn cmd_discover_concepts(project_path: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let created = bearwisdom::query::concepts::discover_concepts(&db)
        .context("discover_concepts failed")?;
    if let Err(e) = bearwisdom::query::concepts::auto_assign_concepts(&db) {
        eprintln!("Warning: auto_assign_concepts failed: {e}");
    }
    ok_json(serde_json::json!({ "created": created }))
}

fn cmd_concept_members(project_path: &str, concept: &str, limit: usize) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let members = bearwisdom::query::concepts::concept_members(&db, concept, limit)
        .context("concept_members failed")?;
    ok_json(members)
}

// ---------------------------------------------------------------------------
// Graph / Flow
// ---------------------------------------------------------------------------

fn cmd_export_graph(
    project_path: &str,
    filter: Option<&str>,
    max_nodes: usize,
) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let graph = bearwisdom::query::subgraph::export_graph(&db, filter, max_nodes)
        .context("export_graph failed")?;
    ok_json(graph)
}

fn cmd_trace_flow(project_path: &str, file: &str, line: u32, depth: u32) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let steps = bearwisdom::search::flow::trace_flow(&db, file, line, depth)
        .context("trace_flow failed")?;
    ok_json(steps)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the database path for a project root.
///
/// Mirrors the logic in `IndexManager::resolve_db_path` in the Tauri layer:
///   ~/.bearwisdom/indexes/<first-16-hex-chars-of-sha256(canonical-path)>/index.db
fn resolve_db_path(project_root: &Path) -> Result<PathBuf> {
    let canonical =
        std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());

    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let short_hash = &hash[..16];

    let home = dirs::home_dir().context("Cannot resolve home directory")?;
    let dir = home.join(".bearwisdom").join("indexes").join(short_hash);

    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Cannot create index dir {}", dir.display()))?;

    Ok(dir.join("index.db"))
}

/// Open an existing database.  Does NOT re-index.
/// Returns an error if no database exists for this project yet.
fn open_existing_db(project_path: &str) -> Result<Database> {
    let root = PathBuf::from(project_path);
    let db_path = resolve_db_path(&root)?;

    if !db_path.exists() {
        anyhow::bail!(
            "No index found for '{}'. Run `bw open {}` first.",
            project_path,
            project_path
        );
    }

    Database::open_with_vec(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))
}

/// Resolve the CodeRankEmbed model directory: tries project root first, then ~/.bearwisdom.
fn resolve_model_dir(project_root: &Path) -> Option<PathBuf> {
    let workspace_model = project_root.join("models").join("CodeRankEmbed");
    if workspace_model.join("tokenizer.json").exists() {
        return Some(workspace_model);
    }
    if let Some(home) = dirs::home_dir() {
        let home_model = home.join(".bearwisdom").join("models").join("CodeRankEmbed");
        if home_model.join("tokenizer.json").exists() {
            return Some(home_model);
        }
    }
    None
}

/// Serialize a value as `{"ok":true,"data":<value>}`.
fn ok_json<T: serde::Serialize>(value: T) -> Result<String> {
    let inner = serde_json::to_value(value).context("Failed to serialize result")?;
    serde_json::to_string(&serde_json::json!({"ok": true, "data": inner}))
        .context("Failed to serialize JSON envelope")
}
