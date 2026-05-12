// =============================================================================
// type_checker/inheritance.rs — Shared inheritance-chain walk helper
//
// Used by JVM-family and C# resolvers to resolve bare method calls that come
// from a parent class not visible to the scope-chain walk (Step 1).
//
// PR 4 of decision-2026-04-27-e75: moved here from `indexer/resolve/`. PR 5
// will lift `resolve_via_inheritance` into a `TypeChecker::walk_inheritance`
// default method once `kind_compatible` is on the trait too — both are
// consumed together by the JVM/C# call sites.
//
// Design:
//   The caller has already failed Steps 1–N (scope chain, package/namespace,
//   imports, qualified name) and hands us the simple method name plus the
//   qname of the enclosing class.  We walk `inherits_map` upward — up to
//   `MAX_DEPTH` hops — trying `{ancestor_qname}.{method_name}` at each level.
//
//   Depth cap guards against pathological cycles in malformed source.
//
// All four resolvers (Java/Groovy, Kotlin, C#) share the same code path;
// only the strategy tag and visibility predicate differ.
// =============================================================================

use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
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
                    if is_visible(file_ctx, ref_ctx, sym) && kind_compatible(edge_kind, &sym.kind)
                    {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.85,
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

/// Extract the calling class qname from the scope chain.
///
/// The scope chain is built from `source_symbol.scope_path` which encodes the
/// enclosing scopes from innermost to outermost:
///
///   `"com.example.MyClass.myMethod"` →
///   scope_chain = ["com.example.MyClass.myMethod", "com.example.MyClass", "com.example"]
///
/// For a method call inside `myMethod`, scope_chain[0] is the *method* qname
/// and scope_chain[1] is the *class* qname.  We want the class, not the
/// method, so we return the first scope entry that looks like a class
/// (i.e., it has at least one component and does not look like a method —
/// we approximate this by taking the second entry when available, falling
/// back to the first).
///
/// Groovy/Java/Kotlin: the extractor always puts the method inside the class
/// in the scope_path, so scope_chain[0] is method-level, scope_chain[1] is
/// the class.  C# does the same.
///
/// Returns `None` when the scope chain has fewer than two entries (top-level
/// functions or files with no class).
pub fn enclosing_class_from_scope<'a>(scope_chain: &'a [String]) -> Option<&'a str> {
    // scope_chain[0] is the innermost scope (the method itself).
    // scope_chain[1] is the enclosing class.
    scope_chain.get(1).map(|s| s.as_str())
}
