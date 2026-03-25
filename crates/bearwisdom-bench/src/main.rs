// =============================================================================
// bearwisdom-bench  —  benchmark CLI
//
// USAGE
// -----
//   bearwisdom-bench index <path> [--db <path>]
//       Walk and index <path>.  Print timing + symbol/edge counts.
//
//   bearwisdom-bench references <symbol> --db <path> [--limit N]
//       Find all references to <symbol> in an existing index.
//
//   bearwisdom-bench definition <symbol> --db <path>
//       Go to definition for <symbol> in an existing index.
//
//   bearwisdom-bench routes --db <path>
//       List all extracted HTTP routes.
//
//   bearwisdom-bench db-mappings --db <path>
//       List all EF Core entity/table mappings.
//
//   bearwisdom-bench report <path> [--db <path>]
//       Full benchmark report: index, run predefined queries, print p50/p95/p99.
//
//   bearwisdom-bench architecture --db <path>
//       Print a codebase overview: totals, language stats, hotspots, entry points.
//
//   bearwisdom-bench blast-radius <symbol> --db <path> [--depth N]
//       Show all symbols affected when <symbol> changes (N-hop subgraph).
//
//   bearwisdom-bench calls-in <symbol> --db <path> [--limit N]
//       Show all symbols that call <symbol> (incoming call hierarchy).
//
//   bearwisdom-bench calls-out <symbol> --db <path> [--limit N]
//       Show all symbols that <symbol> calls (outgoing call hierarchy).
//
//   bearwisdom-bench symbol-info <symbol> --db <path>
//       Print detailed information about a symbol.
//
//   bearwisdom-bench search <query> --db <path> [--limit N]
//       Full-text search across symbol names, signatures, and doc comments.
//
//   bearwisdom-bench discover-concepts --db <path>
//       Auto-discover concepts from namespace structure and print them.
//
//   bearwisdom-bench concept <name> --db <path> [--limit N]
//       List all symbols belonging to a concept.
//
//   bearwisdom-bench export-graph --db <path> [--filter <pattern>] [--format json]
//       Export the symbol graph as text or JSON.
//
// The --db option defaults to a temp file that is deleted after the run.
// =============================================================================

extern crate sqlite_vec;

use bearwisdom::{
    bridge::{enricher::BackgroundEnricher, graph_bridge::GraphBridge},
    connectors::{ef_core, http_api},
    db::Database,
    full_index,
    lsp::manager::LspManager,
    query::{
        architecture, blast_radius as blast_radius_mod, call_hierarchy, concepts as concepts_mod,
        definitions, references, search as search_mod, subgraph as subgraph_mod, symbol_info,
    },
};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "bearwisdom-bench", about = "BearWisdom benchmark CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Index a directory and print timing.
    Index {
        /// Path to the project root.
        path: PathBuf,
        /// Path to the SQLite database file (default: temp file).
        #[arg(long)]
        db: Option<PathBuf>,
    },

    /// Incremental re-index: only process changed files.
    IncrementalIndex {
        /// Path to the project root.
        path: PathBuf,
        /// Path to the SQLite database file.
        #[arg(long)]
        db: PathBuf,
    },

    /// Find all references to a symbol.
    References {
        /// Symbol name (simple or qualified).
        symbol: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum number of results (0 = unlimited).
        #[arg(long, default_value = "50")]
        limit: usize,
    },

    /// Go to definition for a symbol.
    Definition {
        /// Symbol name (simple or qualified).
        symbol: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
    },

    /// List all HTTP routes in an existing index.
    Routes {
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
    },

    /// List all EF Core db_mapping records.
    DbMappings {
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
    },

    /// Full benchmark report: index + predefined queries + statistics.
    Report {
        /// Path to the project root.
        path: PathBuf,
        /// Optional path for the SQLite database (default: temp file).
        #[arg(long)]
        db: Option<PathBuf>,
    },

    /// Print a codebase architecture overview.
    Architecture {
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
    },

    /// Show the blast radius of a symbol (N-hop impact analysis).
    BlastRadius {
        /// Symbol name (simple or qualified).
        symbol: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum number of hops to traverse.
        #[arg(long, default_value = "3")]
        depth: u32,
    },

    /// Show all callers of a symbol (incoming call hierarchy).
    CallsIn {
        /// Symbol name (simple or qualified).
        symbol: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum number of results (0 = unlimited).
        #[arg(long, default_value = "50")]
        limit: usize,
    },

    /// Show all callees of a symbol (outgoing call hierarchy).
    CallsOut {
        /// Symbol name (simple or qualified).
        symbol: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum number of results (0 = unlimited).
        #[arg(long, default_value = "50")]
        limit: usize,
    },

    /// Print detailed information about a symbol.
    SymbolInfo {
        /// Symbol name (simple or qualified).
        symbol: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
    },

    /// Full-text search across symbol names, signatures, and doc comments.
    Search {
        /// FTS5 query string (words, prefix with *, phrases in quotes).
        query: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum number of results.
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Auto-discover concepts from namespace structure.
    DiscoverConcepts {
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
    },

    /// List all symbols belonging to a concept.
    Concept {
        /// Concept name to query.
        name: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum number of results (0 = unlimited).
        #[arg(long, default_value = "50")]
        limit: usize,
    },

    /// Run LSP enrichment on an existing index database.
    Enrich {
        /// Path to the project root (needed for LSP workspace root and file reading).
        path: PathBuf,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum number of unresolved refs to process.
        #[arg(long, default_value = "200")]
        batch_size: usize,
        /// Confidence threshold — upgrade edges below this value.
        #[arg(long, default_value = "0.90")]
        threshold: f64,
    },

    /// Grep: on-demand text/regex search across project files.
    Grep {
        /// Search pattern (literal or regex).
        pattern: String,
        /// Path to the project root.
        #[arg(long)]
        project: PathBuf,
        /// Treat pattern as a regex.
        #[arg(long)]
        regex: bool,
        /// Case-sensitive search (default: true).
        #[arg(long, default_value = "true")]
        case_sensitive: bool,
        /// Restrict to a language (e.g. "csharp", "typescript").
        #[arg(long)]
        lang: Option<String>,
        /// Maximum number of results.
        #[arg(long, default_value = "100")]
        limit: usize,
    },

    /// FTS5 trigram content search across all indexed file content.
    ContentSearch {
        /// Substring to search for (minimum 3 chars for trigram).
        query: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum number of results.
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Fuzzy file path finder (Ctrl+P equivalent).
    FuzzyFile {
        /// Fuzzy pattern to match against file paths.
        pattern: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum number of results.
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Fuzzy symbol finder (Ctrl+T equivalent).
    FuzzySymbol {
        /// Fuzzy pattern to match against symbol qualified names.
        pattern: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum number of results.
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Hybrid FTS5 + vector search (requires model for vector leg).
    HybridSearch {
        /// Natural language or keyword query.
        query: String,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Path to the CodeRankEmbed model directory.
        #[arg(long)]
        model: Option<PathBuf>,
        /// Maximum number of results.
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Trace cross-language execution flow from a file:line.
    TraceFlow {
        /// Relative file path (e.g. "src/UserList.tsx").
        #[arg(long)]
        file: String,
        /// Line number to start tracing from.
        #[arg(long)]
        line: u32,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum traversal depth.
        #[arg(long, default_value = "5")]
        depth: u32,
    },

    /// Import a SCIP index file (compiler-accurate references).
    ImportScip {
        /// Path to the .scip index file.
        #[arg(long)]
        scip: PathBuf,
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Path to the project root.
        #[arg(long)]
        project: PathBuf,
    },

    /// Show recent search history.
    History {
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Maximum number of entries.
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Export the symbol graph as text or JSON.
    ExportGraph {
        /// Path to an existing index database.
        #[arg(long)]
        db: PathBuf,
        /// Optional filter: a namespace prefix (e.g. "App.Catalog") or a
        /// concept name prefixed with @ (e.g. "@authentication").
        #[arg(long)]
        filter: Option<String>,
        /// Output format: "text" (default) or "json".
        #[arg(long, default_value = "text")]
        format: String,
        /// Maximum number of nodes to export.
        #[arg(long, default_value = "500")]
        max_nodes: usize,
    },
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Index { path, db }                    => cmd_index(&path, db.as_deref()),
        Command::IncrementalIndex { path, db }            => cmd_incremental_index(&path, &db),
        Command::References { symbol, db, limit }      => cmd_references(&symbol, &db, limit),
        Command::Definition { symbol, db }             => cmd_definition(&symbol, &db),
        Command::Routes { db }                         => cmd_routes(&db),
        Command::DbMappings { db }                     => cmd_db_mappings(&db),
        Command::Report { path, db }                   => cmd_report(&path, db.as_deref()),
        Command::Architecture { db }                   => cmd_architecture(&db),
        Command::BlastRadius { symbol, db, depth }     => cmd_blast_radius(&symbol, &db, depth),
        Command::CallsIn { symbol, db, limit }         => cmd_calls_in(&symbol, &db, limit),
        Command::CallsOut { symbol, db, limit }        => cmd_calls_out(&symbol, &db, limit),
        Command::SymbolInfo { symbol, db }             => cmd_symbol_info(&symbol, &db),
        Command::Search { query, db, limit }           => cmd_search(&query, &db, limit),
        Command::DiscoverConcepts { db }               => cmd_discover_concepts(&db),
        Command::Concept { name, db, limit }           => cmd_concept(&name, &db, limit),
        Command::Enrich { path, db, batch_size, threshold } =>
            cmd_enrich(&path, &db, batch_size, threshold),
        Command::Grep { pattern, project, regex, case_sensitive, lang, limit } =>
            cmd_grep(&pattern, &project, regex, case_sensitive, lang.as_deref(), limit),
        Command::ContentSearch { query, db, limit } =>
            cmd_content_search(&query, &db, limit),
        Command::FuzzyFile { pattern, db, limit } =>
            cmd_fuzzy_file(&pattern, &db, limit),
        Command::FuzzySymbol { pattern, db, limit } =>
            cmd_fuzzy_symbol(&pattern, &db, limit),
        Command::HybridSearch { query, db, model, limit } =>
            cmd_hybrid_search(&query, &db, model.as_deref(), limit),
        Command::TraceFlow { file, line, db, depth } =>
            cmd_trace_flow(&file, line, &db, depth),
        Command::ImportScip { scip, db, project } =>
            cmd_import_scip(&scip, &db, &project),
        Command::History { db, limit } =>
            cmd_history(&db, limit),
        Command::ExportGraph { db, filter, format, max_nodes } =>
            cmd_export_graph(&db, filter.as_deref(), &format, max_nodes),
    }
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

fn cmd_index(path: &Path, db_path: Option<&Path>) -> Result<()> {
    let _temp;
    let db_path = match db_path {
        Some(p) => p.to_path_buf(),
        None => {
            _temp = tempfile_path();
            _temp.clone()
        }
    };

    let mut db = Database::open(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    let start = Instant::now();
    let stats = full_index(&mut db, path, None, None)
        .with_context(|| format!("Failed to index {}", path.display()))?;
    let elapsed = start.elapsed();

    println!("=== Index complete ===");
    println!("  Database:     {}", db_path.display());
    println!("  Files:        {}", stats.file_count);
    println!("  Symbols:      {}", stats.symbol_count);
    println!("  Edges:        {}", stats.edge_count);
    println!("  Unresolved:   {}", stats.unresolved_ref_count);
    println!("  Routes:       {}", stats.route_count);
    println!("  DB mappings:  {}", stats.db_mapping_count);
    println!("  Parse errors: {}", stats.files_with_errors);
    println!("  Elapsed:      {:.2}s", elapsed.as_secs_f64());

    Ok(())
}

fn cmd_incremental_index(path: &Path, db_path: &Path) -> Result<()> {
    let mut db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let stats = bearwisdom::incremental_index(&mut db, path)?;

    println!("=== Incremental Index ===");
    println!("  Added:      {}", stats.files_added);
    println!("  Modified:   {}", stats.files_modified);
    println!("  Deleted:    {}", stats.files_deleted);
    println!("  Unchanged:  {}", stats.files_unchanged);
    println!("  Symbols:    {}", stats.symbols_written);
    println!("  Edges:      {}", stats.edges_written);
    println!("  Elapsed:    {:.2}ms", stats.duration_ms);

    Ok(())
}

fn cmd_references(symbol: &str, db_path: &Path, limit: usize) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let refs = references::find_references(&db, symbol, limit)?;
    let elapsed = start.elapsed();

    println!("=== References to '{}' ({} results in {:.1}ms) ===",
        symbol, refs.len(), elapsed.as_secs_f64() * 1000.0);

    for r in &refs {
        println!("  {} {} [{}:{}]  ({}, confidence={:.2})",
            r.referencing_kind, r.referencing_symbol,
            r.file_path, r.line,
            r.edge_kind, r.confidence);
    }

    if refs.is_empty() {
        println!("  (no references found)");
    }
    Ok(())
}

fn cmd_definition(symbol: &str, db_path: &Path) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let defs = definitions::goto_definition(&db, symbol)?;
    let elapsed = start.elapsed();

    println!("=== Definition of '{}' ({} results in {:.1}ms) ===",
        symbol, defs.len(), elapsed.as_secs_f64() * 1000.0);

    for d in &defs {
        let sig = d.signature.as_deref().unwrap_or("");
        println!("  {} {} [{}:{}]  {sig}  (confidence={:.2})",
            d.kind, d.qualified_name, d.file_path, d.line, d.confidence);
    }

    if defs.is_empty() {
        println!("  (not found)");
    }
    Ok(())
}

fn cmd_routes(db_path: &Path) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let routes = http_api::list_routes(&db)?;
    println!("=== HTTP Routes ({}) ===", routes.len());
    for r in &routes {
        let handler = r.handler_name.as_deref().unwrap_or("(unknown)");
        println!("  {:6}  {}  ->  {}  [{}:{}]",
            r.http_method, r.route_template, handler, r.file_path, r.line);
    }
    Ok(())
}

fn cmd_db_mappings(db_path: &Path) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let mappings = ef_core::list_mappings(&db)?;
    println!("=== EF Core DB Mappings ({}) ===", mappings.len());
    for m in &mappings {
        println!("  {}  ->  table={}  [{}]  ({})",
            m.entity_type, m.table_name, m.file_path, m.source);
    }
    Ok(())
}

fn cmd_report(path: &Path, db_path: Option<&Path>) -> Result<()> {
    let _temp;
    let db_path = match db_path {
        Some(p) => p.to_path_buf(),
        None => {
            _temp = tempfile_path();
            _temp.clone()
        }
    };

    println!("=== BearWisdom — Benchmark Report ===");
    println!("Project: {}", path.display());
    println!();

    println!("--- Phase 1: Full Index ---");
    let mut db = Database::open(&db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let t0 = Instant::now();
    let stats = full_index(&mut db, path, None, None)
        .with_context(|| format!("Failed to index {}", path.display()))?;
    let index_duration = t0.elapsed();

    println!("  Files:        {}", stats.file_count);
    println!("  Symbols:      {}", stats.symbol_count);
    println!("  Edges:        {}", stats.edge_count);
    println!("  Unresolved:   {}", stats.unresolved_ref_count);
    println!("  Routes:       {}", stats.route_count);
    println!("  DB mappings:  {}", stats.db_mapping_count);
    println!("  Parse errors: {}", stats.files_with_errors);
    println!("  Index time:   {:.2}s", index_duration.as_secs_f64());
    println!();

    println!("--- Phase 2: Query Benchmarks ---");

    let reference_targets = [
        "CatalogItem", "CatalogDbContext", "ICatalogRepository",
        "MapCatalogApiV1", "GetCatalogItems", "OrderStatus", "BasketItem",
    ];
    let definition_targets = [
        "CatalogService", "Program", "OrderingContext", "BasketCheckoutEvent",
    ];

    let mut ref_times: Vec<Duration> = Vec::new();
    let mut def_times: Vec<Duration> = Vec::new();

    println!();
    println!("  References:");
    for sym in &reference_targets {
        let t = Instant::now();
        let refs = references::find_references(&db, sym, 100)?;
        let elapsed = t.elapsed();
        ref_times.push(elapsed);
        println!("    {:40} {:4} refs   {:.1}ms",
            sym, refs.len(), elapsed.as_secs_f64() * 1000.0);
    }

    println!();
    println!("  Definitions:");
    for sym in &definition_targets {
        let t = Instant::now();
        let defs = definitions::goto_definition(&db, sym)?;
        let elapsed = t.elapsed();
        def_times.push(elapsed);
        println!("    {:40} {:4} defs   {:.1}ms",
            sym, defs.len(), elapsed.as_secs_f64() * 1000.0);
    }

    println!();
    println!("--- Phase 3: Query Timing Summary ---");
    let all_times: Vec<Duration> = ref_times.iter().chain(def_times.iter()).copied().collect();
    print_percentiles("  All queries", &all_times);
    print_percentiles("  References ", &ref_times);
    print_percentiles("  Definitions", &def_times);

    println!();
    println!("=== Done ===");
    Ok(())
}

fn cmd_architecture(db_path: &Path) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let ov = architecture::get_overview(&db)?;
    let elapsed = start.elapsed();

    println!("=== Architecture Overview ({:.1}ms) ===", elapsed.as_secs_f64() * 1000.0);
    println!("  Total files:   {}", ov.total_files);
    println!("  Total symbols: {}", ov.total_symbols);
    println!("  Total edges:   {}", ov.total_edges);

    println!();
    println!("  Languages:");
    for lang in &ov.languages {
        println!("    {:15}  {:5} files   {:7} symbols",
            lang.language, lang.file_count, lang.symbol_count);
    }

    println!();
    println!("  Hotspots (top {} by incoming refs):", ov.hotspots.len());
    for (i, h) in ov.hotspots.iter().enumerate() {
        println!("    {:2}. {:40}  {:5} refs   [{}]",
            i + 1, h.qualified_name, h.incoming_refs, h.file_path);
    }

    println!();
    println!("  Entry points ({} public symbols):", ov.entry_points.len());
    for ep in &ov.entry_points {
        println!("    {} {} [{}:{}]", ep.kind, ep.qualified_name, ep.file_path, ep.line);
    }

    Ok(())
}

fn cmd_blast_radius(symbol: &str, db_path: &Path, depth: u32) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let result = blast_radius_mod::blast_radius(&db, symbol, depth)?;
    let elapsed = start.elapsed();

    match result {
        None => {
            println!("Symbol '{}' not found in the index.", symbol);
        }
        Some(br) => {
            println!("=== Blast Radius of '{}' (depth {}, {:.1}ms) ===",
                br.center.qualified_name, depth, elapsed.as_secs_f64() * 1000.0);
            println!("  {} affected symbol(s)", br.total_affected);

            if br.affected.is_empty() {
                println!("  (no dependents found)");
            } else {
                for a in &br.affected {
                    println!("  depth={} {} {} [{}]  (via {})",
                        a.depth, a.kind, a.qualified_name, a.file_path, a.edge_kind);
                }
            }
        }
    }
    Ok(())
}

fn cmd_calls_in(symbol: &str, db_path: &Path, limit: usize) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let items = call_hierarchy::incoming_calls(&db, symbol, limit)?;
    let elapsed = start.elapsed();

    println!("=== Callers of '{}' ({} results in {:.1}ms) ===",
        symbol, items.len(), elapsed.as_secs_f64() * 1000.0);

    for item in &items {
        println!("  {} {} [{}:{}]",
            item.kind, item.qualified_name, item.file_path, item.line);
    }
    if items.is_empty() {
        println!("  (no callers found)");
    }
    Ok(())
}

fn cmd_calls_out(symbol: &str, db_path: &Path, limit: usize) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let items = call_hierarchy::outgoing_calls(&db, symbol, limit)?;
    let elapsed = start.elapsed();

    println!("=== Callees of '{}' ({} results in {:.1}ms) ===",
        symbol, items.len(), elapsed.as_secs_f64() * 1000.0);

    for item in &items {
        println!("  {} {} [{}:{}]",
            item.kind, item.qualified_name, item.file_path, item.line);
    }
    if items.is_empty() {
        println!("  (no callees found)");
    }
    Ok(())
}

fn cmd_symbol_info(symbol: &str, db_path: &Path) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let details = symbol_info::symbol_info(&db, symbol)?;
    let elapsed = start.elapsed();

    println!("=== Symbol Info for '{}' ({} matches in {:.1}ms) ===",
        symbol, details.len(), elapsed.as_secs_f64() * 1000.0);

    if details.is_empty() {
        println!("  (not found)");
        return Ok(());
    }

    for d in &details {
        println!();
        println!("  {} {}", d.kind, d.qualified_name);
        println!("  File:       {}:{}-{}", d.file_path, d.start_line, d.end_line);
        println!("  Visibility: {}", d.visibility.as_deref().unwrap_or("(none)"));
        if let Some(sig) = &d.signature {
            println!("  Signature:  {sig}");
        }
        if let Some(doc) = &d.doc_comment {
            let first_line = doc.lines().next().unwrap_or(doc);
            println!("  Doc:        {first_line}");
        }
        println!("  Incoming edges: {}  Outgoing edges: {}",
            d.incoming_edge_count, d.outgoing_edge_count);
        if !d.children.is_empty() {
            println!("  Children ({}):", d.children.len());
            for c in &d.children {
                println!("    {} {} [{}:{}]",
                    c.kind, c.qualified_name, c.file_path, c.line);
            }
        }
    }
    Ok(())
}

fn cmd_search(query: &str, db_path: &Path, limit: usize) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let results = search_mod::search_symbols(&db, query, limit)?;
    let elapsed = start.elapsed();

    println!("=== Search '{}' ({} results in {:.1}ms) ===",
        query, results.len(), elapsed.as_secs_f64() * 1000.0);

    for r in &results {
        let sig = r.signature.as_deref().unwrap_or("");
        println!("  [{:.3}] {} {} [{}:{}]  {sig}",
            r.score, r.kind, r.qualified_name, r.file_path, r.start_line);
    }
    if results.is_empty() {
        println!("  (no results)");
    }
    Ok(())
}

fn cmd_discover_concepts(db_path: &Path) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let created = concepts_mod::discover_concepts(&db)?;
    let assigned = concepts_mod::auto_assign_concepts(&db)?;
    let elapsed = start.elapsed();

    println!("=== Concept Discovery ({:.1}ms) ===", elapsed.as_secs_f64() * 1000.0);
    println!("  {} new concept(s) discovered, {} symbol assignments", created.len(), assigned);

    for name in &created {
        println!("  {name}");
    }

    println!();
    let all = concepts_mod::list_concepts(&db)?;
    println!("  All concepts ({}):", all.len());
    for c in &all {
        println!("    {:30}  {:5} members  pattern={:?}",
            c.name, c.member_count, c.auto_pattern.as_deref().unwrap_or("(none)"));
    }

    Ok(())
}

fn cmd_concept(name: &str, db_path: &Path, limit: usize) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let members = concepts_mod::concept_members(&db, name, limit)?;
    let elapsed = start.elapsed();

    println!("=== Concept '{}' ({} members in {:.1}ms) ===",
        name, members.len(), elapsed.as_secs_f64() * 1000.0);

    for m in &members {
        println!("  {} {} [{}:{}]",
            m.kind, m.qualified_name, m.file_path, m.line);
    }
    if members.is_empty() {
        println!("  (concept not found or has no members)");
    }
    Ok(())
}

fn cmd_export_graph(
    db_path: &Path,
    filter: Option<&str>,
    format: &str,
    max_nodes: usize,
) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();

    if format == "json" {
        let json = subgraph_mod::export_graph_json(&db, filter, max_nodes)?;
        let elapsed = start.elapsed();
        println!("/* {:.1}ms */", elapsed.as_secs_f64() * 1000.0);
        println!("{json}");
    } else {
        let graph = subgraph_mod::export_graph(&db, filter, max_nodes)?;
        let elapsed = start.elapsed();
        println!("=== Graph Export ({} nodes, {} edges, {:.1}ms) ===",
            graph.nodes.len(), graph.edges.len(), elapsed.as_secs_f64() * 1000.0);

        println!();
        println!("  Nodes:");
        for n in &graph.nodes {
            let concept = n.concept.as_deref().unwrap_or("");
            println!("    {:5}  {} {}  [{}]{}",
                n.id, n.kind, n.qualified_name, n.file_path,
                if concept.is_empty() { String::new() } else { format!("  @{concept}") });
        }

        println!();
        println!("  Edges ({}):", graph.edges.len());
        for e in graph.edges.iter().take(100) {
            println!("    {} --[{}]--> {}  (conf={:.2})",
                e.source_id, e.kind, e.target_id, e.confidence);
        }
        if graph.edges.len() > 100 {
            println!("    ... ({} more edges omitted)", graph.edges.len() - 100);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn print_percentiles(label: &str, times: &[Duration]) {
    if times.is_empty() {
        println!("{label}: (no data)");
        return;
    }
    let mut sorted: Vec<f64> = times.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    sorted.sort_by(f64::total_cmp);

    let p = |pct: f64| -> f64 {
        let idx = ((pct / 100.0) * (sorted.len() - 1) as f64) as usize;
        sorted[idx.min(sorted.len() - 1)]
    };

    println!("{label}:  p50={:.1}ms  p95={:.1}ms  p99={:.1}ms  max={:.1}ms",
        p(50.0), p(95.0), p(99.0), sorted.last().copied().unwrap_or(0.0));
}

/// Return a path inside the system temp directory for a scratch database.
fn tempfile_path() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("bearwisdom-bench-{}.db", std::process::id()));
    p
}

// ---------------------------------------------------------------------------
// Enrich
// ---------------------------------------------------------------------------

fn cmd_enrich(project_root: &Path, db_path: &Path, batch_size: usize, threshold: f64) -> Result<()> {
    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    // Pre-enrichment stats
    let pre_edges: u32 = db.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
    let pre_unresolved: u32 = db.conn.query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0))?;
    let pre_lsp: u32 = db.conn.query_row("SELECT COUNT(*) FROM lsp_edge_meta", [], |r| r.get(0))?;

    println!("=== Pre-Enrichment ===");
    println!("  Edges:       {pre_edges}");
    println!("  Unresolved:  {pre_unresolved}");
    println!("  LSP meta:    {pre_lsp}");
    println!("  Rate:        {:.1}%", pre_edges as f64 / (pre_edges + pre_unresolved) as f64 * 100.0);
    println!();

    // Create LSP manager + bridge + enricher
    let db_arc = std::sync::Arc::new(std::sync::Mutex::new(db));
    let lsp = std::sync::Arc::new(LspManager::new(project_root));
    let bridge = std::sync::Arc::new(GraphBridge::new(
        db_arc.clone(),
        lsp.clone(),
        project_root,
    ));
    let enricher = BackgroundEnricher::new(bridge);

    // Run enrichment using tokio runtime
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to build tokio runtime")?;

    println!("--- Phase 1: Resolving unresolved refs via LSP (batch={batch_size}) ---");
    let progress = rt.block_on(enricher.enrich_unresolved(batch_size))?;
    println!("  Total unresolved:   {}", progress.total_unresolved);
    println!("  Resolved this pass: {}", progress.resolved_this_pass);
    println!("  Still unresolved:   {}", progress.still_unresolved);
    println!("  Elapsed:            {}ms", progress.elapsed_ms);
    println!();

    println!("--- Phase 2: Upgrading low-confidence edges (threshold={threshold}) ---");
    let progress2 = rt.block_on(enricher.enrich_low_confidence(threshold, batch_size))?;
    println!("  Upgraded this pass: {}", progress2.upgraded_this_pass);
    println!("  Elapsed:            {}ms", progress2.elapsed_ms);
    println!();

    // Shutdown LSP servers
    rt.block_on(lsp.shutdown_all())?;

    // Post-enrichment stats
    let db_guard = db_arc.lock().unwrap();
    let post_edges: u32 = db_guard.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
    let post_unresolved: u32 = db_guard.conn.query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0))?;
    let post_lsp: u32 = db_guard.conn.query_row("SELECT COUNT(*) FROM lsp_edge_meta", [], |r| r.get(0))?;

    println!("=== Post-Enrichment ===");
    println!("  Edges:       {post_edges}  (was {pre_edges}, +{})", post_edges.saturating_sub(pre_edges));
    println!("  Unresolved:  {post_unresolved}  (was {pre_unresolved}, -{})", pre_unresolved.saturating_sub(post_unresolved));
    println!("  LSP meta:    {post_lsp}  (was {pre_lsp}, +{})", post_lsp.saturating_sub(pre_lsp));
    println!("  Rate:        {:.1}%  (was {:.1}%)",
        post_edges as f64 / (post_edges + post_unresolved) as f64 * 100.0,
        pre_edges as f64 / (pre_edges + pre_unresolved) as f64 * 100.0);

    // Confidence distribution
    println!();
    println!("=== Confidence Distribution ===");
    let mut stmt = db_guard.conn.prepare(
        "SELECT CASE
            WHEN confidence = 1.0 THEN '1.00 (LSP)'
            WHEN confidence >= 0.95 THEN '0.95'
            WHEN confidence >= 0.92 THEN '0.92'
            WHEN confidence >= 0.90 THEN '0.90'
            WHEN confidence >= 0.85 THEN '0.85'
            WHEN confidence >= 0.80 THEN '0.80'
            WHEN confidence >= 0.50 THEN '0.50'
            ELSE '<0.50' END as bucket,
            COUNT(*) FROM edges GROUP BY bucket ORDER BY bucket DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
    })?;
    for row in rows {
        let (bucket, count) = row?;
        println!("  {bucket:12}  {count:>6}");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Grep
// ---------------------------------------------------------------------------

fn cmd_grep(
    pattern: &str,
    project: &Path,
    regex: bool,
    case_sensitive: bool,
    lang: Option<&str>,
    limit: usize,
) -> Result<()> {
    use bearwisdom::search::{grep, scope::SearchScope};
    use std::sync::{atomic::AtomicBool, Arc};

    let mut scope = SearchScope::default();
    if let Some(l) = lang {
        scope = scope.with_language(l);
    }

    let options = grep::GrepOptions {
        case_sensitive,
        whole_word: false,
        regex,
        max_results: limit,
        scope,
        context_lines: 0,
    };

    let cancelled = Arc::new(AtomicBool::new(false));
    let start = Instant::now();
    let matches = grep::grep_search(project, pattern, &options, &cancelled)?;
    let elapsed = start.elapsed();

    println!("=== Grep '{}' ({} results in {:.1}ms) ===",
        pattern, matches.len(), elapsed.as_secs_f64() * 1000.0);

    for m in &matches {
        println!("  {}:{}: {}", m.file_path, m.line_number, m.line_content);
    }
    if matches.is_empty() {
        println!("  (no results)");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Content search
// ---------------------------------------------------------------------------

fn cmd_content_search(query: &str, db_path: &Path, limit: usize) -> Result<()> {
    use bearwisdom::search::{content_search, scope::SearchScope};

    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let results = content_search::search_content(&db, query, &SearchScope::default(), limit)?;
    let elapsed = start.elapsed();

    println!("=== Content search '{}' ({} files in {:.1}ms) ===",
        query, results.len(), elapsed.as_secs_f64() * 1000.0);

    for r in &results {
        println!("  [{:.3}] {} ({})", r.score, r.file_path, r.language);
    }
    if results.is_empty() {
        println!("  (no results — query must be >= 3 chars for trigram)");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Fuzzy file
// ---------------------------------------------------------------------------

fn cmd_fuzzy_file(pattern: &str, db_path: &Path, limit: usize) -> Result<()> {
    use bearwisdom::search::fuzzy::FuzzyIndex;

    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let index = FuzzyIndex::from_db(&db)?;
    let matches = index.match_files(pattern, limit);
    let elapsed = start.elapsed();

    println!("=== Fuzzy file '{}' ({} results in {:.1}ms) ===",
        pattern, matches.len(), elapsed.as_secs_f64() * 1000.0);

    for m in &matches {
        println!("  [{:5}] {}", m.score, m.text);
    }
    if matches.is_empty() {
        println!("  (no results)");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Fuzzy symbol
// ---------------------------------------------------------------------------

fn cmd_fuzzy_symbol(pattern: &str, db_path: &Path, limit: usize) -> Result<()> {
    use bearwisdom::search::fuzzy::FuzzyIndex;

    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let index = FuzzyIndex::from_db(&db)?;
    let matches = index.match_symbols(pattern, limit);
    let elapsed = start.elapsed();

    println!("=== Fuzzy symbol '{}' ({} results in {:.1}ms) ===",
        pattern, matches.len(), elapsed.as_secs_f64() * 1000.0);

    for m in &matches {
        let meta = match &m.metadata {
            bearwisdom::search::fuzzy::FuzzyMetadata::Symbol { kind, file_path, line } =>
                format!("{kind} [{file_path}:{line}]"),
            _ => String::new(),
        };
        println!("  [{:5}] {} {}", m.score, m.text, meta);
    }
    if matches.is_empty() {
        println!("  (no results)");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Hybrid search
// ---------------------------------------------------------------------------

fn cmd_hybrid_search(query: &str, db_path: &Path, model_dir: Option<&Path>, limit: usize) -> Result<()> {
    use bearwisdom::search::{embedder::Embedder, hybrid, scope::SearchScope};

    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let model_path = model_dir
        .map(|p| p.to_path_buf())
        .or_else(|| bearwisdom::search::embedder::Embedder::resolve_model_dir(&PathBuf::from(".")))
        .unwrap_or_else(|| PathBuf::from("models/CodeRankEmbed"));

    let mut embedder = Embedder::new(model_path);

    let start = Instant::now();
    let results = hybrid::hybrid_search(&db, &mut embedder, query, &SearchScope::default(), limit)?;
    let elapsed = start.elapsed();

    println!("=== Hybrid search '{}' ({} results in {:.1}ms) ===",
        query, results.len(), elapsed.as_secs_f64() * 1000.0);

    for r in &results {
        let sym = r.symbol_name.as_deref().unwrap_or("");
        let kind = r.kind.as_deref().unwrap_or("");
        let tr = r.text_rank.map(|t| format!("T{t}")).unwrap_or_default();
        let vr = r.vector_rank.map(|v| format!("V{v}")).unwrap_or_default();
        println!("  [{:.4}] {kind} {sym} [{}:{}] {tr} {vr}",
            r.rrf_score, r.file_path, r.start_line);
    }
    if results.is_empty() {
        println!("  (no results)");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Trace flow
// ---------------------------------------------------------------------------

fn cmd_trace_flow(file: &str, line: u32, db_path: &Path, depth: u32) -> Result<()> {
    use bearwisdom::search::flow;

    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let steps = flow::trace_flow(&db, file, line, depth)?;
    let elapsed = start.elapsed();

    println!("=== Flow trace from {}:{} (depth {}, {} steps in {:.1}ms) ===",
        file, line, depth, steps.len(), elapsed.as_secs_f64() * 1000.0);

    for s in &steps {
        let sym = s.symbol.as_deref().unwrap_or("?");
        let ln = s.line.map(|l| l.to_string()).unwrap_or_default();
        let proto = s.protocol.as_deref().unwrap_or("");
        println!("  d={} {} {} [{}:{}] {} {}",
            s.depth, s.language, sym, s.file_path, ln, s.edge_type, proto);
    }
    if steps.is_empty() {
        println!("  (no flow edges from this location)");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Import SCIP
// ---------------------------------------------------------------------------

fn cmd_import_scip(scip_path: &Path, db_path: &Path, project_root: &Path) -> Result<()> {
    use bearwisdom::bridge::scip;

    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let start = Instant::now();
    let stats = scip::import_scip(&db, scip_path, project_root)?;
    let elapsed = start.elapsed();

    println!("=== SCIP Import ({:.1}ms) ===", elapsed.as_secs_f64() * 1000.0);
    println!("  Documents processed: {}", stats.documents_processed);
    println!("  Symbols matched:     {}", stats.symbols_matched);
    println!("  Edges created:       {}", stats.edges_created);
    println!("  Edges upgraded:      {}", stats.edges_upgraded);
    println!("  Symbols unmatched:   {}", stats.symbols_unmatched);

    Ok(())
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

fn cmd_history(db_path: &Path, limit: usize) -> Result<()> {
    use bearwisdom::search::history;

    let db = Database::open(db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let entries = history::recent_searches(&db.conn, None, limit)?;

    println!("=== Search History ({} entries) ===", entries.len());
    for e in &entries {
        let saved = if e.is_saved { " [saved]" } else { "" };
        println!("  [{}] {} (type={}, count={}){}", e.id, e.query, e.query_type, e.use_count, saved);
    }

    let saved = history::saved_searches(&db.conn)?;
    if !saved.is_empty() {
        println!();
        println!("  Saved searches: {}", saved.len());
        for s in &saved {
            println!("    [{}] {} ({})", s.id, s.query, s.query_type);
        }
    }

    Ok(())
}
