// =============================================================================
// search/  —  full search engine module tree
//
// Phase 1: grep (on-demand text/regex search)
// Phase 2: content_index + content_search (FTS5 trigram)
// Phase 3: fuzzy (nucleo-based file/symbol finder)
// Phase 4: chunker + embedder + vector_store + hybrid (embeddings + vector search)
// Phase 5: flow (cross-language flow graph)
// Phase 6: history (search recall + saved searches)
// =============================================================================

pub mod chunker;
pub mod content_index;
pub mod content_search;
pub mod embedder;
pub mod flow;
pub mod fuzzy;
pub mod grep;
pub mod history;
pub mod hybrid;
pub mod scope;
pub mod vector_store;
