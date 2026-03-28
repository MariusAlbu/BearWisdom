// =============================================================================
// bearwisdom  —  hybrid code intelligence engine (tree-sitter + LSP)
//
// Design goals vs. v2:
//   • All v2 capabilities preserved unchanged (scope tree, 5-priority resolver,
//     HTTP-route and EF Core connectors)
//   • New `lsp` module: lifecycle manager for external language servers
//   • New `bridge` module: GraphBridge (merges LSP edges into SQLite) and
//     BackgroundEnricher (idle-time resolution of unresolved_refs)
//   • New `EdgeKind::LspResolved` and `EdgeSource` type for edge provenance
//   • New `lsp_edge_meta` table in the schema for LSP edge bookkeeping
// =============================================================================

pub mod bridge;
pub mod connectors;
pub mod db;
pub mod indexer;
pub mod lsp;
pub mod parser;
pub mod query;
pub mod search;
pub mod types;
pub mod walker;

// Re-export the most commonly used entry points at the crate root so callers
// don't need to dig through module paths.
pub use db::{resolve_db_path, db_exists, Database, DbPool, PoolGuard};
pub use indexer::full::{full_index, ProgressFn};
pub use indexer::incremental::{incremental_index, reindex_files, ChangeKind, FileChangeEvent};
pub use types::{EdgeKind, EdgeSource, Symbol, SymbolKind};
pub use indexer::post_index::embed_chunks;
pub use walker::WalkedFile;

// Re-export query result types for consumers that only depend on this crate.
pub use query::architecture::{ArchitectureOverview, HotspotSymbol, LanguageStats, SymbolSummary};
pub use query::blast_radius::{AffectedSymbol, BlastRadiusResult};
pub use query::call_hierarchy::CallHierarchyItem;
pub use query::concepts::ConceptSummary;
pub use query::search::SearchResult;
pub use query::subgraph::{GraphEdge, GraphNode, SubgraphResult};
pub use query::symbol_info::{SymbolDetail, FileSymbol, FileSymbolsMode};
pub use query::investigate::{InvestigateOptions, InvestigateResult, SlimSymbol, BlastRadiusSlim};
pub use query::QueryOptions;

// Re-export new v3 types.
pub use bridge::enricher::BackgroundEnricher;
pub use bridge::graph_bridge::GraphBridge;
pub use bridge::scip::{import_scip, ScipImportStats};
pub use lsp::manager::LspManager;
pub use lsp::types::{Language, ServerState, ServerStatus};
