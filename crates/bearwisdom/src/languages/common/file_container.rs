// =============================================================================
// languages/common/file_container.rs — the implicit file-level container
//
// A file-as-container language declares a container by having a file at all:
// the file itself is the struct / module / unit another file names by path.
// This module turns that implicit container into a real symbol at index 0 and
// re-roots the file's former top-level symbols underneath it, so a path-keyed
// import bind has a symbol to land on and the member surface hangs off it.
//
// The pass carries no language knowledge — it is pure ExtractedSymbol /
// ExtractedRef index-contract mechanics.
// =============================================================================

use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

/// The implicit file-level container a file-as-container language declares by
/// having a file at all.
pub struct FileContainer<'a> {
    pub name: &'a str,
    /// Identity of the container in the qualified-name index. Derived from the
    /// file path, never from the bare stem: many files share a stem, and a
    /// bare-stem key would collide with the declarations that already own it.
    pub qualified_name: &'a str,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    /// Last 0-based line of the file — the container spans the whole file.
    pub end_line: u32,
}

/// Materialize `container` as symbol index 0 of `result`, adopting every
/// symbol that had no structural parent. Every index link in the result
/// (symbol parent, ref source, route handler, db-set property) shifts by one.
/// No-op for an empty container name.
///
/// Preconditions on the extractor's output: every `source_symbol_index` names
/// a real symbol (a `0` sentinel for "no owner" would be re-attributed to the
/// first real symbol), and an unparented top-level symbol carries no
/// `scope_path` of its own (the container's qualified name replaces it).
///
/// The shift runs before the adopt: adopting first would parent the already
/// shifted symbols to a slot the container does not occupy yet.
pub fn materialize(result: &mut ExtractionResult, container: FileContainer<'_>) {
    if container.name.is_empty() {
        return;
    }
    shift_index_links(result);
    adopt_unparented(result, container.qualified_name);
    result.symbols.insert(0, container_symbol(&container));
}

/// Materialize `container` as the only symbol of a result whose extractor
/// declared nothing, and point every index link in the result at it.
///
/// Distinct from [`materialize`], which re-roots declarations that already
/// exist: here there is nothing to re-root, and every index link the extractor
/// emitted names a symbol that does not exist — so the links are re-seated on
/// the container rather than shifted.
///
/// Returns whether the container was inserted. It is not inserted for an empty
/// container name, for a result that already declares a symbol, or for one
/// carrying no index link for the container to own.
pub fn materialize_as_sole_owner(
    result: &mut ExtractionResult,
    container: FileContainer<'_>,
) -> bool {
    if container.name.is_empty() || !result.symbols.is_empty() || !has_index_links(result) {
        return false;
    }
    result.symbols.push(container_symbol(&container));
    for r in &mut result.refs {
        r.source_symbol_index = 0;
    }
    for route in &mut result.routes {
        route.handler_symbol_index = 0;
    }
    for db_set in &mut result.db_sets {
        db_set.property_symbol_index = 0;
    }
    true
}

/// Basename of `file_path` without its extension, for either path separator.
pub fn file_stem(file_path: &str) -> String {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string()
}

/// `file_path` without its extension, `/`-separated: the path-shaped identity
/// of the file's container.
pub fn path_stem(file_path: &str) -> String {
    let norm = file_path.replace('\\', "/");
    match norm.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/{}", file_stem(name)),
        None => file_stem(&norm),
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Whether the result carries any link that names a symbol by index.
fn has_index_links(result: &ExtractionResult) -> bool {
    !result.refs.is_empty() || !result.routes.is_empty() || !result.db_sets.is_empty()
}

/// Move every index link in `result` up by one slot, freeing index 0.
fn shift_index_links(result: &mut ExtractionResult) {
    for sym in &mut result.symbols {
        sym.parent_index = sym.parent_index.map(|parent| parent + 1);
    }
    for r in &mut result.refs {
        r.source_symbol_index += 1;
    }
    for route in &mut result.routes {
        route.handler_symbol_index += 1;
    }
    for db_set in &mut result.db_sets {
        db_set.property_symbol_index += 1;
    }
}

/// Parent every symbol without a structural parent to the container slot.
/// `scope_path` must equal the container's qualified name — the canonical-form
/// contract rejects a parented symbol whose scope disagrees with its parent.
fn adopt_unparented(result: &mut ExtractionResult, container_qualified_name: &str) {
    for sym in &mut result.symbols {
        if sym.parent_index.is_none() {
            sym.parent_index = Some(0);
            sym.scope_path = Some(container_qualified_name.to_string());
        }
    }
}

/// Build the container symbol. It spans the file from line 0 and is the one
/// symbol left without a parent, so every child sits at a strictly greater
/// index than its parent.
fn container_symbol(container: &FileContainer<'_>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: container.name.to_string(),
        qualified_name: container.qualified_name.to_string(),
        kind: container.kind,
        visibility: Some(container.visibility),
        start_line: 0,
        end_line: container.end_line,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

#[cfg(test)]
#[path = "file_container_tests.rs"]
mod tests;
