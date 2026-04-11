extern crate sqlite_vec;

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
    /// Enable verbose output (signatures, doc comments, children). Default: slim.
    #[arg(long, global = true)]
    full: bool,
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
        /// Skip embedding computation (faster indexing for quality checks).
        #[arg(long)]
        no_embed: bool,
    },

    /// Show index status for a project (state, file count, symbol count, edge count).
    /// Opens the existing DB read-only; does NOT re-index.
    Status {
        /// Absolute path to the project root.
        path: String,
    },

    /// Watch project for file changes and re-index incrementally.
    /// Runs until Ctrl+C. Outputs JSON events to stdout on each re-index.
    Watch {
        /// Absolute path to the project root.
        path: String,
        /// Debounce delay in milliseconds (default: 100).
        #[arg(long, default_value = "100")]
        debounce_ms: u64,
    },

    // ---- Symbol search -----------------------------------------------------
    /// Full-text symbol search (FTS5 BM25).  Supports prefixes with *.
    SearchSymbols {
        /// Absolute path to the project root.
        path: String,
        /// FTS5 query (e.g. "GetById", "Catalog*", "\"get items\"").
        query: String,
        /// Maximum results (default: 10).
        #[arg(long, default_value = "10")]
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
        /// Maximum results (default: 20).
        #[arg(long, default_value = "20")]
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
        /// Output mode: "names", "outline" (default), "full"
        #[arg(long, default_value = "outline")]
        mode: String,
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
        /// Maximum results (default: 20).
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Show diagnostics for a file: unresolved symbols and low-confidence edges.
    Diagnostics {
        /// Absolute path to the project root.
        path: String,
        /// Relative file path to check.
        file: String,
        /// Confidence threshold (default: 0.80).
        #[arg(long, default_value = "0.80")]
        threshold: f64,
    },

    /// Find dead code candidates: symbols with zero incoming edges that are not entry points.
    DeadCode {
        /// Absolute path to the project root.
        path: String,
        /// Restrict to file path or directory prefix.
        #[arg(long)]
        scope: Option<String>,
        /// Visibility filter: all, private, public (default: all).
        #[arg(long, default_value = "all")]
        visibility: String,
        /// Include test file symbols (default: false).
        #[arg(long, default_value = "false")]
        include_tests: bool,
        /// Maximum results (default: 100).
        #[arg(long, default_value = "100")]
        limit: usize,
    },

    /// List entry points: main functions, route handlers, test functions, event handlers.
    EntryPoints {
        /// Absolute path to the project root.
        path: String,
    },

    // ---- Architecture ------------------------------------------------------
    /// High-level architecture overview: totals, per-language stats, hotspots, entry points.
    Architecture {
        /// Absolute path to the project root.
        path: String,
    },

    /// Smart context: select the most relevant symbols for a task (LLM context optimization).
    SmartContext {
        /// Absolute path to the project root.
        path: String,
        /// Natural-language task description.
        task: String,
        /// Token budget (default: 8000).
        #[arg(long, default_value = "8000")]
        budget: u32,
        /// Graph expansion depth (default: 2).
        #[arg(long, default_value = "2")]
        depth: u32,
    },

    /// Blast radius analysis: which symbols would be affected by changing this one?
    BlastRadius {
        /// Absolute path to the project root.
        path: String,
        /// Symbol name or qualified name to analyze.
        symbol: String,
        /// Maximum graph traversal depth (default: 2).
        #[arg(long, default_value = "2")]
        depth: u32,
    },

    /// Incoming call hierarchy: who calls this symbol?
    CallsIn {
        /// Absolute path to the project root.
        path: String,
        /// Symbol name or qualified name.
        symbol: String,
        /// Maximum results (default: 20, 0 = unlimited).
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Outgoing call hierarchy: what does this symbol call?
    CallsOut {
        /// Absolute path to the project root.
        path: String,
        /// Symbol name or qualified name.
        symbol: String,
        /// Maximum results (default: 20, 0 = unlimited).
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Detailed symbol information: kind, location, signature, doc comment, edge counts, children.
    SymbolInfo {
        /// Absolute path to the project root.
        path: String,
        /// Symbol name or qualified name.
        symbol: String,
    },

    /// Deep-dive: symbol info + callers + callees + blast radius in one call.
    Investigate {
        /// Absolute path to the project root.
        path: String,
        /// Symbol name or qualified name.
        symbol: String,
        /// Max callers (default: 10).
        #[arg(long, default_value = "10")]
        caller_limit: usize,
        /// Max callees (default: 10).
        #[arg(long, default_value = "10")]
        callee_limit: usize,
        /// Blast radius depth (default: 1).
        #[arg(long, default_value = "1")]
        blast_depth: u32,
    },

    /// Auto-complete symbols at a cursor position (scope-aware).
    CompleteAt {
        /// Absolute path to the project root.
        path: String,
        /// Relative file path.
        file: String,
        /// 1-based line number.
        line: u32,
        /// 0-based column number.
        col: u32,
        /// Prefix text (partial symbol name).
        #[arg(default_value = "")]
        prefix: String,
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

    // ---- Enrichment --------------------------------------------------------
    /// Compute embeddings for all un-embedded code chunks.
    Embed {
        /// Absolute path to the project root.
        path: String,
        /// Batch size for ONNX inference (default: 4, higher = more RAM).
        #[arg(long, default_value = "4")]
        batch_size: usize,
    },


    /// Import a SCIP index to upgrade edge confidence (from rust-analyzer, scip-typescript, etc.).
    ImportScip {
        /// Absolute path to the project root.
        path: String,
        /// Path to the SCIP index file (e.g. index.scip).
        #[arg(long)]
        scip: String,
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
        /// Direction: "forward" (default), "backward" / "reverse", or "both".
        #[arg(long, default_value = "forward")]
        direction: String,
    },

    /// Full execution trace: walk call graph + flow edges from a symbol or all entry points.
    FullTrace {
        /// Absolute path to the project root.
        path: String,
        /// Symbol to trace from (optional — if omitted, traces from entry points).
        symbol: Option<String>,
        /// Maximum traversal depth (default: 5).
        #[arg(long, default_value = "5")]
        depth: u32,
        /// Maximum traces from entry points (default: 10).
        #[arg(long, default_value = "10")]
        max_traces: usize,
    },

    // ---- Quality ---------------------------------------------------------------
    /// Re-index a project without running the embedder.
    /// Equivalent to `open --no-embed` but outputs machine-readable JSON stats.
    Reindex {
        /// Path to the project root.
        path: String,
    },

    /// Analyze tree-sitter extraction coverage for a project.
    /// Shows which node kinds appear in real code and how many symbols/refs
    /// the extractor produces per language.
    Coverage {
        /// Project root path.
        #[arg(long)]
        project: String,
        /// Only show this language (e.g., "typescript").
        #[arg(long)]
        lang: Option<String>,
        /// Show top N node kinds per language (default: 30).
        #[arg(long, default_value = "30")]
        top: usize,
    },

    /// Run quality checks against baseline. Indexes each project, compares
    /// against quality-baseline.json, and reports regressions/improvements.
    QualityCheck {
        /// Path to the quality-baseline.json file.
        #[arg(long, default_value = "quality-baseline.json")]
        baseline: String,
        /// Re-index projects (don't use cached). Slower but catches indexing regressions.
        #[arg(long)]
        reindex: bool,
        /// Re-capture the baseline from current index state. Walks the project
        /// list in the existing baseline, re-indexes each one, and writes the
        /// new metrics back to the baseline file. Use after intentional
        /// quality improvements to avoid noisy regression reports on the next
        /// run. Ghost projects (source missing) are preserved in place with
        /// their existing values and a marker comment.
        #[arg(long, conflicts_with = "reindex")]
        recapture: bool,
    },

    // ---- Hierarchy ---------------------------------------------------------
    /// Hierarchical graph at four zoom levels: services → packages → files → symbols.
    ///
    /// Use --level to select the zoom level and --scope to drill into a
    /// specific package (for "files") or file (for "symbols").
    Hierarchy {
        /// Absolute path to the project root.
        path: String,
        /// Zoom level: "services", "packages", "files", or "symbols".
        #[arg(long, default_value = "packages")]
        level: String,
        /// Package path (for --level files) or file path (for --level symbols).
        #[arg(long)]
        scope: Option<String>,
        /// Maximum nodes to return (default: 500).
        #[arg(long, default_value = "500")]
        max_nodes: usize,
    },

    // ---- Workspace ---------------------------------------------------------
    /// List all detected packages with file/symbol/edge counts.
    /// Returns an empty array for single-project repos.
    Packages {
        /// Absolute path to the project root.
        path: String,
    },

    /// Workspace overview: per-package breakdown + cross-package coupling.
    /// Returns zero/empty fields for single-project repos.
    Workspace {
        /// Absolute path to the project root.
        path: String,
    },

    /// Inter-package dependency graph inferred from cross-package edges.
    /// Returns an empty array for single-project repos.
    Dependencies {
        /// Absolute path to the project root.
        path: String,
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

    let result = run(cli.command, cli.full);

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

fn run(command: Commands, full: bool) -> Result<String> {
    match command {
        Commands::Open { path, no_embed } => cmd_open(&path, no_embed),
        Commands::Status { path } => cmd_status(&path),
        Commands::Watch { path, debounce_ms } => cmd_watch(&path, debounce_ms),

        Commands::SearchSymbols { path, query, limit } => {
            cmd_search_symbols(&path, &query, limit, full)
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
        } => cmd_grep(&path, &pattern, regex, !case_insensitive, whole_word, lang.as_deref(), limit, full),
        Commands::Hybrid { path, query, limit } => cmd_hybrid(&path, &query, limit),

        Commands::FileSymbols { path, file, mode } => {
            let effective_mode = if full { "full" } else { &mode };
            cmd_file_symbols(&path, &file, effective_mode)
        }
        Commands::Definition { path, symbol } => cmd_definition(&path, &symbol),
        Commands::References { path, symbol, limit } => cmd_references(&path, &symbol, limit),

        Commands::Diagnostics { path, file, threshold } => cmd_diagnostics(&path, &file, threshold),
        Commands::DeadCode { path, scope, visibility, include_tests, limit } => {
            cmd_dead_code(&path, scope.as_deref(), &visibility, include_tests, limit)
        }
        Commands::EntryPoints { path } => cmd_entry_points(&path),
        Commands::CompleteAt { path, file, line, col, prefix } => {
            cmd_complete_at(&path, &file, line, col, &prefix, full)
        }

        Commands::Architecture { path } => cmd_architecture(&path),
        Commands::SmartContext { path, task, budget, depth } => {
            cmd_smart_context(&path, &task, budget, depth)
        }
        Commands::BlastRadius { path, symbol, depth } => cmd_blast_radius(&path, &symbol, depth),
        Commands::CallsIn { path, symbol, limit } => cmd_calls_in(&path, &symbol, limit),
        Commands::CallsOut { path, symbol, limit } => cmd_calls_out(&path, &symbol, limit),
        Commands::SymbolInfo { path, symbol } => cmd_symbol_info(&path, &symbol, full),
        Commands::Investigate { path, symbol, caller_limit, callee_limit, blast_depth } => {
            cmd_investigate(&path, &symbol, caller_limit, callee_limit, blast_depth)
        }

        Commands::Concepts { path } => cmd_concepts(&path),
        Commands::DiscoverConcepts { path } => cmd_discover_concepts(&path),
        Commands::ConceptMembers { path, concept, limit } => {
            cmd_concept_members(&path, &concept, limit)
        }

        Commands::Embed { path, batch_size } => cmd_embed(&path, batch_size),
        Commands::ImportScip { path, scip } => cmd_import_scip(&path, &scip),

        Commands::ExportGraph { path, filter, max_nodes } => {
            cmd_export_graph(&path, filter.as_deref(), max_nodes)
        }
        Commands::TraceFlow { path, file, line, depth, direction } => {
            cmd_trace_flow(&path, &file, line, depth, &direction)
        }
        Commands::FullTrace { path, symbol, depth, max_traces } => {
            cmd_full_trace(&path, symbol.as_deref(), depth, max_traces)
        }
        Commands::Reindex { path } => cmd_reindex(&path),
        Commands::Coverage { project, lang, top } => cmd_coverage(&project, lang.as_deref(), top),
        Commands::QualityCheck { baseline, reindex, recapture } => {
            if recapture {
                cmd_quality_recapture(&baseline)
            } else {
                cmd_quality_check(&baseline, reindex)
            }
        }

        Commands::Hierarchy { path, level, scope, max_nodes } => {
            cmd_hierarchy(&path, &level, scope.as_deref(), max_nodes)
        }

        Commands::Packages { path } => cmd_packages(&path),
        Commands::Workspace { path } => cmd_workspace(&path),
        Commands::Dependencies { path } => cmd_dependencies(&path),
    }
}

// ---------------------------------------------------------------------------
// Lifecycle helpers
// ---------------------------------------------------------------------------

/// Open and fully index the project, then print stats.
fn cmd_open(project_path: &str, no_embed: bool) -> Result<String> {
    let root = PathBuf::from(project_path);
    let db_path = resolve_db_path(&root)?;

    eprintln!("Opening database at {}", db_path.display());

    let mut db = Database::open(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    eprintln!("Running full index for {} ...", root.display());

    let stats = bearwisdom::full_index(&mut db, &root, None, None, None)
        .context("Full index failed")?;

    // Auto-discover namespace concepts and assign members.
    if let Err(e) = bearwisdom::query::concepts::discover_concepts(&db) {
        eprintln!("Warning: discover_concepts failed: {e}");
    }
    if let Err(e) = bearwisdom::query::concepts::auto_assign_concepts(&db) {
        eprintln!("Warning: auto_assign_concepts failed: {e}");
    }

    // Post-index: compute embeddings for code chunks.
    let mut chunks_embedded = 0u32;
    if no_embed {
        eprintln!("Skipping embeddings (--no-embed)");
    } else {
        let model_dir = resolve_model_dir(&root);
        if let Some(ref dir) = model_dir {
            eprintln!("Computing embeddings ...");
            let mut embedder = bearwisdom::search::embedder::Embedder::new(dir.clone());
            match bearwisdom::embed_chunks(&db, &mut embedder, 4) {
                Ok((n, _)) => {
                    chunks_embedded = n;
                    eprintln!("Embedded {n} chunks");
                }
                Err(e) => eprintln!("Warning: embedding failed: {e}"),
            }
            embedder.unload();
        } else {
            eprintln!("No CodeRankEmbed model found, skipping embeddings");
        }
    }

    ok_json(serde_json::json!({
        "db_path": db_path.display().to_string(),
        "file_count": stats.file_count,
        "symbol_count": stats.symbol_count,
        "edge_count": stats.edge_count,
        "unresolved_ref_count": stats.unresolved_ref_count,
        "external_ref_count": stats.external_ref_count,
        "chunks_embedded": chunks_embedded,
        "duration_ms": stats.duration_ms,
    }))
}

/// Compute embeddings for all un-embedded code chunks.
fn cmd_embed(project_path: &str, batch_size: usize) -> Result<String> {
    let root = PathBuf::from(project_path);
    let db_path = resolve_db_path(&root)?;
    let db = Database::open(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    let model_dir = resolve_model_dir(&root)
        .ok_or_else(|| anyhow::anyhow!("No CodeRankEmbed model found. Run scripts/download-model.py first."))?;

    eprintln!("Loading model from {} ...", model_dir.display());
    let mut embedder = bearwisdom::search::embedder::Embedder::new(model_dir);

    match bearwisdom::embed_chunks(&db, &mut embedder, batch_size) {
        Ok((n, _)) => {
            embedder.unload();
            eprintln!("Embedded {n} chunks");
            ok_json(serde_json::json!({ "chunks_embedded": n }))
        }
        Err(e) => {
            embedder.unload();
            Err(e).context("Embedding failed")
        }
    }
}

/// Import a SCIP index to upgrade edge confidence.
fn cmd_import_scip(project_path: &str, scip_path: &str) -> Result<String> {
    let root = PathBuf::from(project_path);
    let db_path = resolve_db_path(&root)?;
    let db = Database::open(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    let scip = PathBuf::from(scip_path);
    let stats = bearwisdom::import_scip(&db, &scip, &root)
        .context("SCIP import failed")?;

    eprintln!(
        "SCIP import: {} docs, {} matched, {} edges created, {} upgraded, {} unmatched",
        stats.documents_processed,
        stats.symbols_matched,
        stats.edges_created,
        stats.edges_upgraded,
        stats.symbols_unmatched,
    );

    ok_json(stats)
}

/// Report the current index status without re-indexing.
fn cmd_status(project_path: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let stats = bearwisdom::index_stats(&db)?;

    ok_json(serde_json::json!({
        "state": "ready",
        "file_count": stats.file_count,
        "symbol_count": stats.symbol_count,
        "edge_count": stats.edge_count,
        "unresolved_ref_count": stats.unresolved_ref_count,
        "external_ref_count": stats.external_ref_count,
        "package_count": stats.package_count,
    }))
}

// ---------------------------------------------------------------------------
// Watch mode
// ---------------------------------------------------------------------------

fn cmd_watch(project_path: &str, debounce_ms: u64) -> Result<String> {
    use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher, Event, EventKind};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    let root = PathBuf::from(project_path);
    let db_path = resolve_db_path(&root)?;
    let mut db = Database::open(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    let debounce = Duration::from_millis(debounce_ms);
    let (tx, rx) = mpsc::channel::<Event>();

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
        Config::default(),
    ).context("Failed to create file watcher")?;

    watcher
        .watch(root.as_ref(), RecursiveMode::Recursive)
        .with_context(|| format!("Failed to watch {}", root.display()))?;

    eprintln!("Watching {} for changes (debounce={}ms, Ctrl+C to stop)", root.display(), debounce_ms);

    // Gitignore-based filtering reuses the walker's language detection.
    let source_extensions: std::collections::HashSet<&str> = [
        "cs", "ts", "tsx", "js", "jsx", "rs", "py", "go", "java", "rb", "php",
        "kt", "swift", "scala", "dart", "ex", "exs", "c", "h", "cpp", "hpp",
        "sh", "bash", "html", "css", "scss", "json", "yaml", "yml", "xml",
        "sql", "toml", "md", "lua", "r", "hs", "proto",
    ].into_iter().collect();

    loop {
        // Drain events with debounce.
        let first = match rx.recv() {
            Ok(e) => e,
            Err(_) => break, // channel closed
        };

        let mut events = vec![first];
        let deadline = Instant::now() + debounce;
        while Instant::now() < deadline {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(e) => events.push(e),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(String::new()),
            }
        }

        // Convert notify events to FileChangeEvents, deduplicating by path.
        let mut seen = std::collections::HashSet::new();
        let mut changes: Vec<bearwisdom::FileChangeEvent> = Vec::new();

        for event in &events {
            let change_kind = match event.kind {
                EventKind::Create(_) => bearwisdom::ChangeKind::Created,
                EventKind::Modify(_) => bearwisdom::ChangeKind::Modified,
                EventKind::Remove(_) => bearwisdom::ChangeKind::Deleted,
                _ => continue,
            };

            for path in &event.paths {
                // Filter to source files only.
                let ext = path.extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !source_extensions.contains(ext) {
                    continue;
                }

                // Convert to relative path.
                let rel = match path.strip_prefix(&root) {
                    Ok(r) => r.to_string_lossy().replace('\\', "/"),
                    Err(_) => continue,
                };

                if seen.insert(rel.clone()) {
                    changes.push(bearwisdom::FileChangeEvent {
                        relative_path: rel,
                        change_kind,
                    });
                }
            }
        }

        if changes.is_empty() {
            continue;
        }

        eprintln!("Detected {} file change(s), re-indexing...", changes.len());
        match bearwisdom::reindex_files(&mut db, &root, &changes, None) {
            Ok(stats) => {
                let json = serde_json::json!({
                    "event": "reindex",
                    "files_added": stats.files_added,
                    "files_modified": stats.files_modified,
                    "files_deleted": stats.files_deleted,
                    "symbols_written": stats.symbols_written,
                    "edges_written": stats.edges_written,
                    "duration_ms": stats.duration_ms,
                });
                println!("{json}");
            }
            Err(e) => {
                eprintln!("Re-index error: {e:#}");
            }
        }
    }

    Ok(String::new())
}

// ---------------------------------------------------------------------------
// Symbol search
// ---------------------------------------------------------------------------

fn cmd_search_symbols(project_path: &str, query: &str, limit: usize, full: bool) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let opts = if full { bearwisdom::query::QueryOptions::full() } else { bearwisdom::query::QueryOptions::default() };
    let results = bearwisdom::query::search::search_symbols(&db, query, limit, &opts)
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
    full: bool,
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

    let mut results =
        bearwisdom::search::grep::grep_search(&root, pattern, &options, &cancelled)
            .context("grep_search failed")?;
    if !full {
        bearwisdom::search::grep::truncate_matches(&mut results, 120);
    }
    ok_json(results)
}

fn cmd_hybrid(project_path: &str, query: &str, limit: usize) -> Result<String> {
    let db_path = resolve_db_path(&PathBuf::from(project_path))?;
    let db = Database::open(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

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

fn cmd_file_symbols(project_path: &str, file_path: &str, mode: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let mode = bearwisdom::query::symbol_info::FileSymbolsMode::from_str(mode);
    let results = bearwisdom::query::symbol_info::file_symbols(&db, file_path, mode)?;
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

fn cmd_diagnostics(project_path: &str, file_path: &str, threshold: f64) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let result = bearwisdom::query::diagnostics::get_diagnostics(&db, file_path, threshold)
        .context("diagnostics failed")?;
    ok_json(result)
}

fn cmd_dead_code(
    project_path: &str,
    scope: Option<&str>,
    visibility: &str,
    include_tests: bool,
    limit: usize,
) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let vis = match visibility {
        "private" => bearwisdom::query::dead_code::VisibilityFilter::PrivateOnly,
        "public" => bearwisdom::query::dead_code::VisibilityFilter::PublicOnly,
        _ => bearwisdom::query::dead_code::VisibilityFilter::All,
    };
    let options = bearwisdom::query::dead_code::DeadCodeOptions {
        scope: scope.map(|s| s.to_string()),
        visibility_filter: vis,
        include_tests,
        max_results: limit,
        ..Default::default()
    };
    let result = bearwisdom::query::dead_code::find_dead_code(&db, &options)
        .context("dead code analysis failed")?;
    ok_json(result)
}

fn cmd_entry_points(project_path: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let result = bearwisdom::query::dead_code::find_entry_points(&db)
        .context("entry points analysis failed")?;
    ok_json(result)
}

fn cmd_complete_at(project_path: &str, file_path: &str, line: u32, col: u32, prefix: &str, full: bool) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let results = bearwisdom::query::completion::complete_at(&db, file_path, line, col, prefix, full)
        .context("completion failed")?;
    ok_json(results)
}

// ---------------------------------------------------------------------------
// Architecture
// ---------------------------------------------------------------------------

fn cmd_smart_context(project_path: &str, task: &str, budget: u32, depth: u32) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let result = bearwisdom::query::context::smart_context(&db, task, budget, depth)
        .context("smart context failed")?;
    ok_json(result)
}

fn cmd_architecture(project_path: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let overview = bearwisdom::query::architecture::get_overview(&db)
        .context("get_overview failed")?;
    ok_json(overview)
}

fn cmd_blast_radius(project_path: &str, symbol: &str, depth: u32) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let result = bearwisdom::query::blast_radius::blast_radius(&db, symbol, depth, 500)
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

fn cmd_symbol_info(project_path: &str, symbol: &str, full: bool) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let opts = if full { bearwisdom::query::QueryOptions::full() } else { bearwisdom::query::QueryOptions::default() };
    let results = bearwisdom::query::symbol_info::symbol_info(&db, symbol, &opts)
        .context("symbol_info failed")?;
    // Return first match or null.
    let first = results.into_iter().next();
    ok_json(first)
}

fn cmd_investigate(
    project_path: &str,
    symbol: &str,
    caller_limit: usize,
    callee_limit: usize,
    blast_depth: u32,
) -> Result<String> {
    let root = PathBuf::from(project_path);
    let db_path = resolve_db_path(&root)?;
    let db = Database::open(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    let opts = bearwisdom::query::investigate::InvestigateOptions {
        caller_limit,
        callee_limit,
        blast_depth,
    };
    let result = bearwisdom::query::investigate::investigate(&db, symbol, &opts)?;
    ok_json(result)
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

fn cmd_trace_flow(project_path: &str, file: &str, line: u32, depth: u32, direction: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    match direction {
        "backward" | "reverse" => {
            let steps = bearwisdom::search::flow::trace_flow_reverse(&db, file, line, depth)
                .context("trace_flow_reverse failed")?;
            ok_json(steps)
        }
        "both" | "bidirectional" => {
            let result = bearwisdom::search::flow::trace_flow_bidirectional(&db, file, line, depth)
                .context("trace_flow_bidirectional failed")?;
            ok_json(result)
        }
        _ => {
            let steps = bearwisdom::search::flow::trace_flow(&db, file, line, depth)
                .context("trace_flow failed")?;
            ok_json(steps)
        }
    }
}

fn cmd_full_trace(project_path: &str, symbol: Option<&str>, depth: u32, max_traces: usize) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let result = match symbol {
        Some(sym) => bearwisdom::query::full_trace::trace_from_symbol(&db, sym, depth)
            .context("full_trace from symbol failed")?,
        None => bearwisdom::query::full_trace::trace_from_entry_points(&db, depth, max_traces)
            .context("full_trace from entry points failed")?,
    };
    ok_json(result)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the database path for a project root: `<project>/.bearwisdom/index.db`.
fn resolve_db_path(project_root: &Path) -> Result<PathBuf> {
    bearwisdom::resolve_db_path(project_root)
}

/// Detect projects whose source tree has been deleted, leaving only a cached
/// `.bearwisdom/` index (and possibly `.git/`) behind. Used by `quality-check`
/// to skip them instead of re-indexing an empty directory and producing a
/// full set of false-regression zeroes.
///
/// Heuristic: if the project root contains no non-hidden entries (everything
/// starts with `.`), or contains only entries whose names match a small
/// allowlist of known cache/metadata dirs, it's a ghost.
fn is_ghost_project(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return true;
    };
    let mut has_source = false;
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') {
            // Hidden dir/file — ignore for the ghost check.
            continue;
        }
        has_source = true;
        break;
    }
    !has_source
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

    Database::open(&db_path)
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

// ---------------------------------------------------------------------------
// Reindex
// ---------------------------------------------------------------------------

fn cmd_reindex(project_path: &str) -> Result<String> {
    let root = PathBuf::from(project_path);
    let db_path = resolve_db_path(&root)?;
    let start = std::time::Instant::now();

    eprintln!("Reindexing {} ...", root.display());

    let mut db = Database::open(&db_path)
        .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;

    bearwisdom::full_index(&mut db, &root, None, None, None)
        .with_context(|| format!("Index failed for {}", root.display()))?;

    let stats = bearwisdom::index_stats(&db)?;
    let flow_breakdown = bearwisdom::flow_edge_breakdown(&db)?;
    let flow_edge_types: std::collections::BTreeMap<String, u32> = flow_breakdown
        .into_iter()
        .map(|b| (b.edge_type, b.count))
        .collect();

    let elapsed_ms = start.elapsed().as_millis() as u64;
    eprintln!("Done in {:.2}s: {} files, {} symbols, {} edges, {} routes, {} flow_edges",
        elapsed_ms as f64 / 1000.0,
        stats.file_count, stats.symbol_count, stats.edge_count,
        stats.route_count, stats.flow_edge_count);

    ok_json(serde_json::json!({
        "project": root.display().to_string(),
        "duration_ms": elapsed_ms,
        "files": stats.file_count,
        "symbols": stats.symbol_count,
        "edges": stats.edge_count,
        "routes": stats.route_count,
        "flow_edges": stats.flow_edge_count,
        "unresolved_refs": stats.unresolved_ref_count,
        "flow_edge_types": flow_edge_types,
    }))
}


// ---------------------------------------------------------------------------
// Quality check
// ---------------------------------------------------------------------------

fn cmd_quality_check(baseline_path: &str, reindex: bool) -> Result<String> {
    let baseline_file = PathBuf::from(baseline_path);
    let content = std::fs::read_to_string(&baseline_file)
        .with_context(|| format!("Failed to read baseline: {}", baseline_file.display()))?;
    let baseline: serde_json::Value =
        serde_json::from_str(&content).context("Failed to parse baseline JSON")?;

    let projects = baseline["projects"]
        .as_array()
        .context("baseline.projects is not an array")?;

    let mut regressions = 0u32;
    let mut improvements = 0u32;
    let mut project_results: Vec<serde_json::Value> = Vec::new();

    for proj in projects {
        let name = proj["project"].as_str().unwrap_or("?");
        let proj_path = proj["path"].as_str().unwrap_or("");
        let root = PathBuf::from(proj_path);

        eprint!("--- {name} ---\n  ");
        if !root.exists() {
            eprintln!("SKIP (path not found: {proj_path})");
            continue;
        }
        // Ghost-project guard: the source tree was deleted but `.bearwisdom/`
        // (and nothing else) remains, so `exists()` returns true but there is
        // nothing to index. Re-indexing would wipe the old DB and produce
        // zeroes across every metric, surfacing a sea of false regressions.
        // Detect by checking for ANY non-dotfile entry in the root; if the
        // project has only hidden dirs (.bearwisdom, .git), treat it as a
        // ghost and skip.
        if is_ghost_project(&root) {
            eprintln!("SKIP (ghost project: source missing, only .bearwisdom/ present)");
            continue;
        }

        let db_path = resolve_db_path(&root)?;

        // Optionally re-index.
        if reindex || !db_path.exists() {
            eprintln!("Indexing...");
            let mut db = Database::open(&db_path)
                .with_context(|| format!("Failed to open DB for {name}"))?;
            bearwisdom::full_index(&mut db, &root, None, None, None)
                .with_context(|| format!("Index failed for {name}"))?;
        }

        let db = Database::open(&db_path)
            .with_context(|| format!("Failed to open DB for {name}"))?;

        // Read current counts via core library.
        let stats = bearwisdom::index_stats(&db)?;
        let files = stats.file_count as i64;
        let symbols = stats.symbol_count as i64;
        let edges = stats.edge_count as i64;
        let routes = stats.route_count as i64;
        let flow_edges = stats.flow_edge_count as i64;
        let unresolved = stats.unresolved_ref_count as i64;
        let unresolved_flows: i64 =
            bearwisdom::unresolved_flow_count(&db)? as i64;

        // Per-type flow edge counts.
        let flow_breakdown = bearwisdom::flow_edge_breakdown(&db)?;
        let flow_edge_types: std::collections::BTreeMap<String, i64> = flow_breakdown
            .into_iter()
            .map(|b| (b.edge_type, b.count as i64))
            .collect();

        // Compare against assertions.
        let assertions = &proj["assertions"];
        let mut proj_regressions: Vec<String> = Vec::new();
        let mut proj_improvements: Vec<String> = Vec::new();

        let check = |_field: &str, current: i64, baseline_val: i64| -> (bool, bool) {
            // regression if current < baseline, improvement if current > baseline
            (current < baseline_val, current > baseline_val)
        };

        // Check key metrics against baseline values.
        let baseline_flow = proj["flow_edges"].as_i64().unwrap_or(0);
        let baseline_routes = proj["routes"].as_i64().unwrap_or(0);
        let baseline_symbols = proj["symbols"].as_i64().unwrap_or(0);
        let baseline_edges = proj["edges"].as_i64().unwrap_or(0);

        for (label, current, baseline_val) in [
            ("symbols", symbols, baseline_symbols),
            ("edges", edges, baseline_edges),
            ("routes", routes, baseline_routes),
            ("flow_edges", flow_edges, baseline_flow),
        ] {
            let (reg, imp) = check(label, current, baseline_val);
            if reg {
                let msg = format!(
                    "{label}: {baseline_val} \u{2192} {current} ({diff})",
                    diff = current - baseline_val
                );
                proj_regressions.push(msg);
            } else if imp {
                let msg = format!(
                    "{label}: {baseline_val} \u{2192} {current} (+{diff})",
                    diff = current - baseline_val
                );
                proj_improvements.push(msg);
            }
        }

        // Check min_* assertions.
        if let Some(obj) = assertions.as_object() {
            for (key, val) in obj {
                if let Some(min_val) = val.as_i64() {
                    let current_val = match key.as_str() {
                        "min_routes" => routes,
                        "min_flow_edges" => flow_edges,
                        k if k.starts_with("min_") && k.ends_with("_edges") => {
                            let edge_type = &k[4..k.len() - 6]; // strip min_ and _edges
                            bearwisdom::flow_edge_count_by_type(&db, edge_type)
                                .unwrap_or(0) as i64
                        }
                        _ => continue,
                    };
                    if current_val < min_val {
                        proj_regressions.push(format!(
                            "{key}: expected >={min_val}, got {current_val}"
                        ));
                    }
                }
            }
        }

        let status = if proj_regressions.is_empty() {
            "pass"
        } else {
            "fail"
        };

        if !proj_regressions.is_empty() {
            regressions += 1;
            for r in &proj_regressions {
                eprintln!("  REGRESSION: {r}");
            }
        } else if !proj_improvements.is_empty() {
            improvements += 1;
            for i in &proj_improvements {
                eprintln!("  improvement: {i}");
            }
        } else {
            eprintln!("  OK (no changes)");
        }

        project_results.push(serde_json::json!({
            "project": name,
            "status": status,
            "current": {
                "files": files,
                "symbols": symbols,
                "edges": edges,
                "routes": routes,
                "flow_edges": flow_edges,
                "unresolved_refs": unresolved,
                "unresolved_flow_starts": unresolved_flows,
                "flow_edge_types": flow_edge_types,
            },
            "regressions": proj_regressions,
            "improvements": proj_improvements,
        }));
    }

    let passed = regressions == 0;
    eprintln!(
        "\n=== SUMMARY: {regressions} regressions, {improvements} improvements ===\n\
         QUALITY CHECK {}",
        if passed { "PASSED" } else { "FAILED" }
    );

    ok_json(serde_json::json!({
        "passed": passed,
        "regressions": regressions,
        "improvements": improvements,
        "projects": project_results,
    }))
}

/// Recapture the quality baseline: re-index every project that still has
/// source on disk, snapshot its current metrics, and write a new
/// `quality-baseline.json`. Preserves the `project`, `path`, `languages`,
/// and `assertions` fields from the existing baseline, and updates the
/// numeric metrics (`files`, `symbols`, `edges`, `routes`, `flow_edges`,
/// `unresolved_refs`, `flow_edge_types`) in place.
///
/// Ghost projects (source directory empty / only `.bearwisdom/` remains)
/// and path-not-found entries are **kept as-is** with their old values —
/// the rationale is that the user may intend to restore the source later,
/// and wiping entries silently would hide that intent. Assertion thresholds
/// are auto-updated to the new captured values so future runs compare
/// apples-to-apples.
fn cmd_quality_recapture(baseline_path: &str) -> Result<String> {
    let baseline_file = PathBuf::from(baseline_path);
    let content = std::fs::read_to_string(&baseline_file)
        .with_context(|| format!("Failed to read baseline: {}", baseline_file.display()))?;
    let mut baseline: serde_json::Value =
        serde_json::from_str(&content).context("Failed to parse baseline JSON")?;

    let Some(projects) = baseline["projects"].as_array().cloned() else {
        anyhow::bail!("baseline.projects is not an array");
    };

    let mut new_projects: Vec<serde_json::Value> = Vec::with_capacity(projects.len());
    let mut recaptured = 0u32;
    let mut skipped_missing = 0u32;
    let mut skipped_ghost = 0u32;

    for proj in projects {
        let name = proj["project"].as_str().unwrap_or("?").to_string();
        let proj_path = proj["path"].as_str().unwrap_or("").to_string();
        let root = PathBuf::from(&proj_path);

        eprint!("--- {name} ---\n  ");

        if !root.exists() {
            eprintln!("SKIP (path not found — keeping old values)");
            skipped_missing += 1;
            new_projects.push(proj);
            continue;
        }
        if is_ghost_project(&root) {
            eprintln!("SKIP (ghost project — keeping old values)");
            skipped_ghost += 1;
            new_projects.push(proj);
            continue;
        }

        eprintln!("Reindexing...");
        let db_path = resolve_db_path(&root)?;
        let mut db = Database::open(&db_path)
            .with_context(|| format!("Failed to open DB for {name}"))?;
        bearwisdom::full_index(&mut db, &root, None, None, None)
            .with_context(|| format!("Index failed for {name}"))?;

        let stats = bearwisdom::index_stats(&db)?;
        let flow_breakdown = bearwisdom::flow_edge_breakdown(&db)?;
        let flow_edge_types: std::collections::BTreeMap<String, u32> = flow_breakdown
            .into_iter()
            .map(|b| (b.edge_type, b.count))
            .collect();

        let mut updated = proj.clone();
        updated["files"] = serde_json::json!(stats.file_count);
        updated["symbols"] = serde_json::json!(stats.symbol_count);
        updated["edges"] = serde_json::json!(stats.edge_count);
        updated["routes"] = serde_json::json!(stats.route_count);
        updated["flow_edges"] = serde_json::json!(stats.flow_edge_count);
        updated["unresolved_refs"] = serde_json::json!(stats.unresolved_ref_count);
        updated["flow_edge_types"] = serde_json::json!(flow_edge_types);

        // Rebuild assertions using the newly captured values so future
        // quality-check runs compare to the NEW floor, not the old one.
        // Only rewrite fields the existing assertions block already had —
        // don't invent new thresholds.
        if let Some(assertions) = updated["assertions"].as_object_mut() {
            let old_keys: Vec<String> = assertions.keys().cloned().collect();
            for key in old_keys {
                if let Some(new_value) = match key.as_str() {
                    "min_routes" => Some(serde_json::json!(stats.route_count)),
                    "min_flow_edges" => Some(serde_json::json!(stats.flow_edge_count)),
                    "min_edges" => Some(serde_json::json!(stats.edge_count)),
                    "min_symbols" => Some(serde_json::json!(stats.symbol_count)),
                    "min_files" => Some(serde_json::json!(stats.file_count)),
                    _ => {
                        // Handle flow-edge-type thresholds: min_{type}_edges.
                        if let Some(ty) = key
                            .strip_prefix("min_")
                            .and_then(|s| s.strip_suffix("_edges"))
                        {
                            flow_edge_types.get(ty).map(|v| serde_json::json!(*v))
                        } else {
                            None
                        }
                    }
                } {
                    assertions.insert(key, new_value);
                }
            }
        }

        new_projects.push(updated);
        recaptured += 1;
        eprintln!(
            "  OK ({} files, {} symbols, {} edges)",
            stats.file_count, stats.symbol_count, stats.edge_count
        );
    }

    baseline["projects"] = serde_json::Value::Array(new_projects);
    // Update captured_at to the current UTC date (time precision not
    // needed — baselines are recaptured by hand, not continuously).
    // Format YYYY-MM-DD manually from the UNIX epoch to avoid a chrono
    // dependency for a single use.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days since 1970-01-01.
    let days = (now / 86_400) as i64;
    // Convert to civil date using Howard Hinnant's date algorithms.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    baseline["captured_at"] = serde_json::json!(format!(
        "{:04}-{:02}-{:02}T00:00:00Z",
        year, m, d
    ));

    let serialized = serde_json::to_string_pretty(&baseline)
        .context("Failed to serialize baseline JSON")?;
    std::fs::write(&baseline_file, serialized)
        .with_context(|| format!("Failed to write baseline: {}", baseline_file.display()))?;

    eprintln!(
        "\n=== RECAPTURE: {recaptured} re-indexed, {skipped_missing} missing, {skipped_ghost} ghost ===\n\
         Wrote {} ({} total projects)",
        baseline_file.display(),
        baseline["projects"].as_array().map(|a| a.len()).unwrap_or(0)
    );

    ok_json(serde_json::json!({
        "recaptured": recaptured,
        "skipped_missing": skipped_missing,
        "skipped_ghost": skipped_ghost,
        "baseline": baseline_file.display().to_string(),
    }))
}

// ---------------------------------------------------------------------------
// Coverage analysis
// ---------------------------------------------------------------------------

fn cmd_coverage(project: &str, lang_filter: Option<&str>, _top: usize) -> Result<String> {
    let project_root = std::path::Path::new(project);
    if !project_root.is_dir() {
        anyhow::bail!("Project path does not exist: {project}");
    }

    let results = bearwisdom::query::coverage::analyze_coverage(project_root);

    let filtered: Vec<_> = if let Some(lang) = lang_filter {
        results.into_iter().filter(|r| r.language == lang).collect()
    } else {
        results
    };

    if filtered.is_empty() {
        return ok_json(serde_json::json!({"languages": [], "message": "No languages with grammars found"}));
    }

    let mut summaries = Vec::new();
    for cov in &filtered {
        let has_rules = cov.symbol_coverage.percent >= 0.0;

        summaries.push(serde_json::json!({
            "language": cov.language,
            "files": cov.file_count,
            "symbols_extracted": cov.symbols_extracted,
            "refs_extracted": cov.refs_extracted,
            "has_rules": has_rules,
            "symbol_coverage": {
                "percent": format!("{:.1}%", cov.symbol_coverage.percent.max(0.0)),
                "matched": cov.symbol_coverage.matched_nodes,
                "expected": cov.symbol_coverage.expected_nodes,
                "kinds_seen": format!("{}/{}", cov.symbol_coverage.declared_kinds_seen, cov.symbol_coverage.declared_kinds_total),
            },
            "ref_coverage": {
                "percent": format!("{:.1}%", cov.ref_coverage.percent.max(0.0)),
                "matched": cov.ref_coverage.matched_nodes,
                "expected": cov.ref_coverage.expected_nodes,
                "kinds_seen": format!("{}/{}", cov.ref_coverage.declared_kinds_seen, cov.ref_coverage.declared_kinds_total),
            },
            "symbol_kinds": cov.symbol_kinds,
            "ref_kinds": cov.ref_kinds,
            "structural_top": cov.structural_top,
        }));
    }

    ok_json(serde_json::json!({"languages": summaries}))
}

// ---------------------------------------------------------------------------
// Hierarchy helper
// ---------------------------------------------------------------------------

/// Hierarchical graph at four zoom levels.
fn cmd_hierarchy(
    project_path: &str,
    level: &str,
    scope: Option<&str>,
    max_nodes: usize,
) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let result = bearwisdom::hierarchical_graph(&db, level, scope, max_nodes)
        .context("hierarchical_graph failed")?;
    ok_json(result)
}

// ---------------------------------------------------------------------------
// Workspace helpers
// ---------------------------------------------------------------------------

/// List detected packages with file/symbol/edge counts.
fn cmd_packages(project_path: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let packages = bearwisdom::list_packages(&db).context("list_packages failed")?;
    ok_json(packages)
}

/// Workspace overview: per-package breakdown + cross-package coupling.
fn cmd_workspace(project_path: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let overview = bearwisdom::workspace_overview(&db).context("workspace_overview failed")?;
    ok_json(overview)
}

/// Inter-package dependency graph inferred from cross-package edges.
fn cmd_dependencies(project_path: &str) -> Result<String> {
    let db = open_existing_db(project_path)?;
    let deps = bearwisdom::package_dependencies(&db).context("package_dependencies failed")?;
    ok_json(deps)
}

/// Serialize a value as `{"ok":true,"data":<value>}`.
fn ok_json<T: serde::Serialize>(value: T) -> Result<String> {
    let inner = serde_json::to_value(value).context("Failed to serialize result")?;
    serde_json::to_string(&serde_json::json!({"ok": true, "data": inner}))
        .context("Failed to serialize JSON envelope")
}
