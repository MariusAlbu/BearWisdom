// =============================================================================
// indexer/resolve/engine/testkit_fixtures — constructors for the synthetic
// values rule tests feed the engine: symbol rows, refs, imports, file and ref
// contexts. The `Lookup` double itself lives in `testkit`. Test-only.
// =============================================================================

use std::sync::Arc;

use crate::indexer::resolve::engine::contract::{
    FileContext, ImportEntry, RefContext, Symbol,
};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};

/// A symbol-index row.
pub(crate) fn sym(id: i64, name: &str, qname: &str, kind: &str, file: &str) -> Symbol {
    Symbol {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from(file),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

/// `sym` with a signature attached — for rungs that read the declaration's
/// signature text (extension receivers, param patterns).
pub(crate) fn sym_with_sig(
    id: i64,
    name: &str,
    qname: &str,
    kind: &str,
    file: &str,
    signature: &str,
) -> Symbol {
    let mut s = sym(id, name, qname, kind, file);
    s.signature = Some(signature.to_string());
    s
}

/// An `import name from module` entry.
pub(crate) fn import(name: &str, module: Option<&str>) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: module.map(|s| s.to_string()),
        alias: None,
        is_wildcard: false,
    }
}

/// A file context with the given imports and namespace.
pub(crate) fn file_ctx(imports: Vec<ImportEntry>, ns: Option<&str>) -> FileContext {
    FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports,
        file_namespace: ns.map(|s| s.to_string()),
    }
}

/// A bare `Calls` ref to `target`.
pub(crate) fn call_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

/// A source symbol the ref lives in.
pub(crate) fn source_symbol(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

/// A ref context for `r` in `sym`, with the given scope chain.
pub(crate) fn ref_ctx<'a>(
    r: &'a ExtractedRef,
    sym: &'a ExtractedSymbol,
    scope_chain: Vec<String>,
) -> RefContext<'a> {
    RefContext {
        extracted_ref: r,
        source_symbol: sym,
        scope_chain,
        file_package_id: None,
    }
}

/// Permissive kind predicate — accepts every candidate kind.
pub(crate) fn accept_any(_: EdgeKind, _: &str) -> bool {
    true
}

// Flow-cache surface of the `Lookup` double: backed by the plain maps its
// builders fill, so rule tests can seed local-binding types without a real
// per-file cache.
impl crate::indexer::resolve::engine::contract::FlowCacheLookup for super::testkit::Lookup {
    fn local_type(&self, name: &str) -> Option<String> {
        self.local_types.get(name).cloned()
    }
    fn local_callable_head(&self, name: &str) -> Option<String> {
        self.local_callable_heads.get(name).cloned()
    }
}
