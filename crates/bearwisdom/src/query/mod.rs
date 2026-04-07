// Existing modules.
pub mod error;
pub use error::{QueryError, QueryResult};
pub mod cache;
pub mod definitions;
pub mod references;

// New modules added in this update.
pub mod hierarchy;
pub mod architecture;
pub mod blast_radius;
pub mod call_hierarchy;
pub mod completion;
pub mod concepts;
pub mod context;
pub mod diagnostics;
pub mod investigate;
pub mod search;
pub mod coverage;
pub mod full_trace;
pub mod stats;
pub mod subgraph;
pub mod symbol_info;
pub mod workspace;
pub mod dead_code;

// ---------------------------------------------------------------------------
// Shared query options — slim by default, opt-in for verbose
// ---------------------------------------------------------------------------

/// Controls output verbosity for query functions.
///
/// Both MCP and CLI construct this from user parameters.  The `Default` impl
/// produces slim output suitable for LLM consumption.  Use [`QueryOptions::full()`]
/// for human-readable / debugging output.
#[derive(Debug, Clone)]
pub struct QueryOptions {
    /// Include function/method signatures in results.
    pub include_signature: bool,
    /// Include XML doc comments (C#) or JSDoc (TS).
    pub include_doc: bool,
    /// Include child symbols (methods of a class, etc.) in symbol_info.
    pub include_children: bool,
    /// Truncate grep line content to this many bytes.  0 = unlimited.
    pub max_line_length: u32,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            include_signature: false,
            include_doc: false,
            include_children: false,
            max_line_length: 120,
        }
    }
}

impl QueryOptions {
    /// All details enabled, no truncation.
    pub fn full() -> Self {
        Self {
            include_signature: true,
            include_doc: true,
            include_children: true,
            max_line_length: 0,
        }
    }
}
