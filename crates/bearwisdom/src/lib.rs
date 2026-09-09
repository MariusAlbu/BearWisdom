// =============================================================================
// bearwisdom  —  universal code intelligence engine
//
// BearWisdom replaces LSP servers entirely: tree-sitter parsing builds a
// SQLite graph that answers go-to-definition, find-references, call hierarchy,
// blast-radius, and architecture queries in milliseconds.
//
// Key subsystems:
//   • Parser layer   — 18 dedicated extractors + generic fallback (31 grammars)
//   • Indexer        — full parallel index + incremental file-change index
//   • Connectors     — 21 cross-framework connectors (routes, DI, events, gRPC, …)
//   • Query layer    — search, symbol_info, references, call_hierarchy, blast_radius,
//                      architecture, diagnostics, completion, context, investigate
//   • Search         — FTS5 trigram, nucleo fuzzy, grep (ripgrep), hybrid RRF,
//                      vector KNN (CodeRankEmbed via ONNX)
//   • SCIP import    — merge SCIP index edges (confidence 1.0) into the graph
//   • DB             — SQLite WAL + sqlite-vec, connection pool (DbPool)
// =============================================================================

pub mod alloc_probe;
pub mod connectors;
pub mod containment;
pub mod db;
pub mod ecosystem;
mod flow_meta;
pub mod indexer;
pub mod languages;
pub mod memory_cap;
pub mod occurrence;
pub mod panic_hook;
pub mod parser;
pub mod query;
pub mod resolution_oracle;
pub mod search;
pub mod symbol_key;
pub mod type_checker;
pub mod types;
pub mod walker;

pub use panic_hook::install_fail_fast_panic_hook;

// Re-export the most commonly used entry points at the crate root so callers
// don't need to dig through module paths.
pub use db::audit::{AuditRecord, AuditSessionSummary, AuditStats};
pub use db::metrics::{QueryMetrics, QueryStats};
pub use db::{db_exists, resolve_db_path, Database, DbPool, PoolGuard};
pub use indexer::full::{full_index, full_index_engine, ProgressFn};
pub use indexer::incremental::{
    git_reindex, incremental_index, reindex_files, ChangeKind, FileChangeEvent,
};
pub use indexer::post_index::embed_chunks;
pub use indexer::resolve::engine::trace;
pub use indexer::resolve_diff::{resolve_diff, EdgeKey, ResolveDiff};
pub use indexer::service::{
    last_indexed_at_ms, IndexService, IndexServiceOptions, ReindexStats, LAST_INDEXED_AT_MS_KEY,
};
pub use query::stats::{
    concept_count, flow_diagnostics, flow_edge_breakdown, flow_edge_count_by_type, flow_edges_data,
    index_stats, resolution_breakdown, unresolved_flow_count, FlowDiagnostics, FlowEdgeBreakdown,
    FlowEdgeRow, FlowEdgeTypeBucket, FlowEdgesData, FlowPairing, ResolutionBreakdown,
    SingleEndedExample, UnresolvedTarget,
};
pub use types::{EdgeKind, EdgeSource, IndexStats, PackageInfo, Symbol, SymbolKind};
pub use walker::WalkedFile;

// Re-export query result types for consumers that only depend on this crate.
pub use indexer::ref_cache::RefCache;
pub use query::architecture::{ArchitectureOverview, HotspotSymbol, LanguageStats, SymbolSummary};
pub use query::blast_radius::{AffectedSymbol, BlastRadiusResult};
pub use query::cache::QueryCache;
pub use query::call_hierarchy::CallHierarchyItem;
pub use query::concepts::ConceptSummary;
pub use query::diagnostics::{
    low_confidence_edges, workspace_diagnostics, FileDiagnosticSummary, LowConfidenceBucket,
    LowConfidenceReport, WorkspaceDiagnostics,
};
pub use query::error::{QueryError, QueryResult};
pub use query::hierarchy::{
    hierarchical_graph, Breadcrumb, HierarchyEdge, HierarchyNode, HierarchyResult,
};
pub use query::investigate::{BlastRadiusSlim, InvestigateOptions, InvestigateResult, SlimSymbol};
pub use query::occurrence_census::{occurrence_census, OccurrenceCensus};
pub use query::pattern::{pattern_search, PatternMatch};
pub use query::search::SearchResult;
pub use query::subgraph::{GraphEdge, GraphNode, SubgraphResult};
pub use query::symbol_info::{FileSymbol, FileSymbolsMode, SymbolDetail};
pub use query::unresolved_by_cause::{unresolved_by_cause, ByCauseReport, CauseGroup, CauseSample};
pub use query::unresolved_classify::{
    classify_unresolved, ClassificationBucket, ClassificationReport, SampleEntry,
    UnresolvedCategory,
};
pub use query::workspace::{
    list_packages, package_dependencies, workspace_graph, workspace_overview,
};
pub use query::workspace::{
    PackageDependency, PackageStats, WorkspaceGraphEdge, WorkspaceOverview,
};
pub use query::QueryOptions;

pub use indexer::scip::{import_scip, ScipImportStats};
