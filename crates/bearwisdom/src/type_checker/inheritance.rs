// =============================================================================
// type_checker/inheritance.rs — Shared inheritance-chain walk helper
//
// Used by JVM-family and C# resolvers to resolve bare method calls that come
// from a parent class not visible to the local scope walk.
//
// Design:
//   The caller hands us the simple method name plus the qname of the enclosing
//   class.  We walk `inherits_map` upward — up to `MAX_DEPTH` hops — trying
//   `{ancestor_qname}.{method_name}` at each level.
//
//   Depth cap guards against pathological cycles in malformed source.
//
// All four resolvers (Java/Groovy, Kotlin, C#) share the same code path;
// only the strategy tag and visibility predicate differ.
// =============================================================================

use crate::indexer::resolve::engine::{
    FileContext, RefContext, Resolution, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::types::EdgeKind;

/// Maximum ancestor hops before we give up.
const MAX_DEPTH: usize = 10;

/// Walk the inheritance chain from `calling_class` looking for `method_name`.
///
/// Returns `Some(Resolution)` at confidence 0.85 the first time an ancestor
/// defines a symbol whose kind is compatible with `edge_kind` and is visible
/// according to `is_visible`.
///
/// `is_visible` receives the same arguments as the language resolver's own
/// `is_visible` method — callers pass a closure that forwards to `self.is_visible`.
pub fn resolve_via_inheritance<F>(
    calling_class: &str,
    method_name: &str,
    edge_kind: EdgeKind,
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
    kind_compatible: fn(EdgeKind, &str) -> bool,
    is_visible: F,
    strategy: &'static str,
) -> Option<Resolution>
where
    F: Fn(&FileContext, &RefContext, &crate::indexer::resolve::engine::SymbolInfo) -> bool,
{
    let mut class_qname = calling_class;
    for _ in 0..MAX_DEPTH {
        match lookup.parent_class_qname(class_qname) {
            None => break,
            Some(parent_qname) => {
                let candidate = format!("{parent_qname}.{method_name}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if is_visible(file_ctx, ref_ctx, sym) && kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: RESOLVED_CONFIDENCE,
                            strategy,
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
                class_qname = parent_qname;
            }
        }
    }
    None
}

/// The qualified name of the type that encloses the reference's source symbol —
/// the calling class for an implicit-`this` method call.
///
/// Resolved structurally from the source symbol's containment chain, so a
/// method-source ref yields its *class*, not the surrounding package. Returns
/// `None` for top-level functions and files with no enclosing type.
pub fn enclosing_class_from_scope<'a>(
    source_qname: &str,
    lookup: &'a dyn SymbolLookup,
) -> Option<&'a str> {
    lookup.enclosing_type_qname(source_qname)
}
