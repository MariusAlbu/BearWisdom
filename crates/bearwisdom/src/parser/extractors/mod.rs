pub mod bash;
pub mod c_lang;
pub mod cpp;
pub mod csharp;
pub mod dart;
pub mod elixir;
pub mod generic;
pub mod go;
pub mod java;
pub mod javascript;
pub mod kotlin;
pub mod php;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod scala;
pub mod swift;
pub mod typescript;

use crate::types::{EdgeKind, ExtractedDbSet, ExtractedRef, ExtractedRoute, ExtractedSymbol, MemberChain};
use tree_sitter::Node;

/// When a call has a chain (e.g. `Foo::bar()`, `Foo.bar()`, `ClassName.method()`),
/// emit a `TypeRef` for the type prefix — the segment immediately before the final
/// method name — if it looks like a type (starts with uppercase).
///
/// This ensures that static/scoped calls create edges to both the method (Calls)
/// *and* the owning type (TypeRef).  Works across all languages.
pub fn emit_chain_type_ref(
    chain: &Option<MemberChain>,
    source_symbol_index: usize,
    func_node: &Node,
    refs: &mut Vec<ExtractedRef>,
) {
    let c = match chain.as_ref() {
        Some(c) if c.segments.len() >= 2 => c,
        _ => return,
    };
    let type_seg = &c.segments[c.segments.len() - 2];
    if type_seg
        .name
        .chars()
        .next()
        .map_or(false, |ch| ch.is_uppercase())
    {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: type_seg.name.clone(),
            kind: EdgeKind::TypeRef,
            line: func_node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

/// Universal extraction result returned by all language plugins.
///
/// Older extractors that define their own result structs (e.g., `TypeScriptExtraction`,
/// `GoExtraction`) will be migrated to return this type directly. During the transition,
/// the dispatch code in `full.rs` destructures them into this shape.
pub struct ExtractionResult {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub routes: Vec<ExtractedRoute>,
    pub db_sets: Vec<ExtractedDbSet>,
    pub has_errors: bool,
}

impl ExtractionResult {
    pub fn new(
        symbols: Vec<ExtractedSymbol>,
        refs: Vec<ExtractedRef>,
        has_errors: bool,
    ) -> Self {
        Self {
            symbols,
            refs,
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors,
        }
    }

    /// Create a result with routes and db_sets (used by C# extractor).
    pub fn with_connectors(
        symbols: Vec<ExtractedSymbol>,
        refs: Vec<ExtractedRef>,
        routes: Vec<ExtractedRoute>,
        db_sets: Vec<ExtractedDbSet>,
        has_errors: bool,
    ) -> Self {
        Self { symbols, refs, routes, db_sets, has_errors }
    }

    /// Empty result — no symbols, no refs, no errors.
    pub fn empty() -> Self {
        Self {
            symbols: Vec::new(),
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: false,
        }
    }
}
