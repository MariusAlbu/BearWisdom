// =============================================================================
// parser/extractors/python/mod.rs  —  Python symbol and reference extractor
// =============================================================================


use super::{assignments, calls, helpers, statements, symbols, types};
use crate::types::{EdgeKind, ExtractionResult};
use crate::types::{ExtractedRef, ExtractedSymbol};
use super::helpers::node_text;
use rustc_hash::FxHashSet;
use std::collections::HashMap;
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------



/// Extract all symbols and references from Python source code.
pub fn extract(source: &str) -> ExtractionResult {
    let language = tree_sitter_python::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Python grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return ExtractionResult {
                symbols: vec![],
                refs: vec![],
                routes: vec![],
                db_sets: vec![],
                has_errors: true,
                demand_contributions: Vec::new(),
                alias_targets: Vec::new(),
            }
        }
    };

    let mut syms = Vec::new();
    let mut refs = Vec::new();

    let root = tree.root_node();

    // Build the import map once from the top-level CST so every call site can
    // annotate qualified call refs with their source module.
    let import_map = calls::build_import_map(root, source);

    // Collect the module-level `__all__` export contract. A name imported into
    // this file AND listed here is a genuine re-export; its Imports ref is
    // tagged `is_reexport=true` so the generic re-export walker follows it.
    let dunder_all = collect_dunder_all(root, source);

    extract_from_node(
        root, source, &mut syms, &mut refs, None, "", false, &import_map, &dunder_all,
    );

    // Second pass: scan the full CST for `type` nodes and emit TypeRef for
    // each non-builtin identifier found inside a type annotation context.
    if !syms.is_empty() {
        scan_type_annotation_nodes(root, source, 0, &mut refs);
    }

    let has_errors = tree.root_node().has_error();
    ExtractionResult::new(syms, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

/// Return the `source_symbol_index` to use for an import ref emitted during
/// top-level traversal.
///
/// When a `parent_index` is set (import inside a class or function body), the
/// parent symbol index is used directly — it was pushed before the recursive
/// call, so the index is valid by construction.
///
/// At the module level (`parent_index = None`) the ref is attributed to the
/// last symbol pushed so far, clamped to 0.  This means a module-level import
/// that appears before any symbol definition (e.g. at the top of the file)
/// gets `source_symbol_index = 0`, which becomes valid once the first function
/// or class below it is extracted.  Files that contain only imports and no
/// symbol-defining statements will still fire REF-001 from the canonical-form
/// validator because no symbol slot exists to attach to.
fn clamp_owner(parent_index: Option<usize>, symbols_len: usize) -> usize {
    match parent_index {
        Some(idx) => idx,
        None => symbols_len.saturating_sub(1),
    }
}

// ---------------------------------------------------------------------------
// `__all__` export-contract collection
// ---------------------------------------------------------------------------

/// Collect the names listed in the module-level `__all__` export contract.
///
/// Scans the direct children of `root` (module scope only — an `__all__`
/// inside a function or class body is not a package export contract) for the
/// three forms that grow `__all__`:
///
/// ```python
/// __all__ = ["A", "B"]      # assignment, list/tuple of string literals
/// __all__ += ["C"]           # augmented assignment
/// __all__.extend(["D"])      # extend / append call
/// ```
///
/// Only string-literal entries are collected; a computed `__all__`
/// (`__all__ = _gather()`, comprehension, `+= other.__all__`) is undecidable
/// without executing Python, so its non-literal entries are skipped — a name
/// only ever fails to be tagged, never mis-tagged.
fn collect_dunder_all(root: Node, source: &str) -> FxHashSet<String> {
    let mut names = FxHashSet::default();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "expression_statement" {
            continue;
        }
        let mut ec = child.walk();
        for inner in child.children(&mut ec) {
            match inner.kind() {
                // `__all__ = [...]` / `__all__ = (...)` and
                // `__all__ += [...]` — both expose `left` / `right` fields.
                "assignment" | "augmented_assignment" => {
                    let targets_dunder_all = inner
                        .child_by_field_name("left")
                        .map(|l| node_text(&l, source) == "__all__")
                        .unwrap_or(false);
                    if !targets_dunder_all {
                        continue;
                    }
                    if let Some(rhs) = inner.child_by_field_name("right") {
                        collect_string_literals(&rhs, source, &mut names);
                    }
                }
                // `__all__.extend([...])` / `__all__.append("X")` — a call
                // whose function is the `__all__.extend` / `__all__.append`
                // attribute. Collect the string literals from its arguments.
                "call" => {
                    if !call_targets_dunder_all_mutator(&inner, source) {
                        continue;
                    }
                    if let Some(args) = inner.child_by_field_name("arguments") {
                        collect_string_literals(&args, source, &mut names);
                    }
                }
                _ => {}
            }
        }
    }
    names
}

/// True when `call` is `__all__.extend(...)` or `__all__.append(...)` — an
/// attribute call whose object is the `__all__` identifier and whose attribute
/// is a list-mutator that adds names.
fn call_targets_dunder_all_mutator(call: &Node, source: &str) -> bool {
    let Some(func) = call.child_by_field_name("function") else {
        return false;
    };
    if func.kind() != "attribute" {
        return false;
    }
    let object_is_dunder_all = func
        .child_by_field_name("object")
        .map(|o| node_text(&o, source) == "__all__")
        .unwrap_or(false);
    let attr = func
        .child_by_field_name("attribute")
        .map(|a| node_text(&a, source))
        .unwrap_or_default();
    object_is_dunder_all && matches!(attr.as_str(), "extend" | "append")
}

/// Recursively collect the decoded value of every `string` literal reachable
/// from `node` (list / tuple / argument-list elements). Non-string entries are
/// ignored — a computed entry contributes nothing.
fn collect_string_literals(node: &Node, source: &str, out: &mut FxHashSet<String>) {
    if node.kind() == "string" {
        let decoded = strip_python_string_literal(&node_text(node, source));
        if !decoded.is_empty() {
            out.insert(decoded);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            collect_string_literals(&child, source, out);
        }
    }
}

/// Strip surrounding quotes (single / double / triple forms) and a leading
/// string prefix from a Python `string` literal's source text. Mirrors the
/// quote-stripping in `calls::strip_python_string` for the names that appear in
/// an `__all__` list.
fn strip_python_string_literal(raw: &str) -> String {
    let trimmed = raw
        .trim_start_matches(['b', 'r', 'f', 'B', 'R', 'F', 'u', 'U']);
    trimmed
        .trim_start_matches("\"\"\"")
        .trim_end_matches("\"\"\"")
        .trim_start_matches("'''")
        .trim_end_matches("'''")
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

pub(super) fn extract_from_node(
    node: Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
    import_map: &HashMap<String, String>,
    dunder_all: &FxHashSet<String>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                symbols::extract_function_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                    &[],
                    import_map,
                );
            }

            "class_definition" => {
                symbols::extract_class_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    import_map,
                    &[],
                );
            }

            "decorated_definition" => {
                symbols::extract_decorated_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                    import_map,
                );
            }

            "import_statement" => {
                // Attach to the enclosing symbol when one exists, otherwise
                // to the most-recently-pushed symbol (index 0 fallback).
                // The index is always clamped so it stays in-bounds.
                let owner = clamp_owner(parent_index, symbols.len());
                calls::extract_import_statement(&child, source, refs, owner, dunder_all);
            }

            "import_from_statement" => {
                let owner = clamp_owner(parent_index, symbols.len());
                calls::extract_import_from_statement(&child, source, refs, owner, dunder_all);
            }

            // `from __future__ import annotations` — emit Imports refs for
            // each imported name.  The grammar has no `module_name` field;
            // instead the `__future__` keyword is a bare node, and the imported
            // names appear as `dotted_name` or `identifier` children.
            "future_import_statement" => {
                let owner = clamp_owner(parent_index, symbols.len());
                let mut cursor = child.walk();
                for fc in child.children(&mut cursor) {
                    match fc.kind() {
                        "dotted_name" | "identifier" => {
                            let name = helpers::node_text(&fc, source);
                            if !name.is_empty() && name != "__future__" {
                                refs.push(crate::types::ExtractedRef { is_import_binding: false, is_reexport: false,
                                    source_symbol_index: owner,
                                    target_name: name,
                                    kind: crate::types::EdgeKind::Imports,
                                    line: fc.start_position().row as u32,
                                    col: 0,
                                    module: Some("__future__".to_string()),
                                    chain: None,
                                    byte_offset: fc.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }

            // `type Point = tuple[int, int]` (Python 3.12+)
            "type_alias_statement" => {
                let enclosing = parent_index.unwrap_or(0);
                types::extract_type_alias_top_level(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing,
                );
            }

            "expression_statement" => {
                assignments::extract_assignment_if_any(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
                // Also extract any call expressions inside the statement (e.g.
                // `foo()` or `bar.baz()` at module/class body level).
                calls::extract_calls_from_body(
                    &child,
                    source,
                    parent_index.unwrap_or(0),
                    refs,
                    import_map,
                );
                // Extract TypeRef from variable type annotations:
                // `items: List[str] = []` — the `assignment.type` field.
                extract_annotation_type_refs(
                    &child,
                    source,
                    parent_index.unwrap_or(0),
                    refs,
                );
            }

            // `foo()` / `bar.baz()` at module or class body level.
            // `call` can also appear as a direct child when not wrapped in
            // `expression_statement` (rare but possible in some parse trees).
            "call" => {
                calls::extract_calls_from_body(
                    &child,
                    source,
                    parent_index.unwrap_or(0),
                    refs,
                    import_map,
                );
            }

            // `with open('f') as fh:` — context manager
            "with_statement" => {
                let enclosing = parent_index.unwrap_or(0);
                statements::extract_with_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing,
                    import_map,
                );
            }

            // `match command: case ...:` — structural pattern matching (3.10+)
            "match_statement" => {
                let enclosing = parent_index.unwrap_or(0);
                statements::extract_match_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing,
                    import_map,
                );
            }

            // Recurse into ERROR/MISSING nodes to recover whatever tree-sitter
            // managed to parse inside the erroneous region.
            "ERROR" | "MISSING" | _ => {
                extract_from_node(
                    child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                    import_map,
                    dunder_all,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Annotation TypeRef helper
// ---------------------------------------------------------------------------

/// Walk an `expression_statement` for annotated assignments and emit a
/// `TypeRef` edge for the type annotation.
///
/// ```python
/// items: List[str] = []      # assignment with `type` field
/// count: int                  # bare annotation (no value)
/// ```
fn extract_annotation_type_refs(
    expr_stmt: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = expr_stmt.walk();
    for child in expr_stmt.children(&mut cursor) {
        if child.kind() == "assignment" {
            if let Some(type_node) = child.child_by_field_name("type") {
                emit_type_ref_from_annotation(&type_node, source, source_symbol_index, refs);
            }
        }
    }
}

fn emit_type_ref_from_annotation(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            if !name.is_empty()
                && !matches!(name.as_str(), "None" | "int" | "str" | "float" | "bool" | "bytes")
            {
                refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
        // `uuid.UUID` / `sqlalchemy.orm.Session` — emit a single qualified ref
        // so the resolver can route via the module import. Do NOT recurse —
        // that would leak separate bare refs for each segment.
        "attribute" => {
            if let Some(attr) = node.child_by_field_name("attribute") {
                let name = node_text(&attr, source);
                if !name.is_empty() {
                    let module = node
                        .child_by_field_name("object")
                        .map(|o| node_text(&o, source))
                        .filter(|s| !s.is_empty());
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: attr.start_position().row as u32,
                        col: 0,
                        module,
                        chain: None,
                        byte_offset: attr.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    emit_type_ref_from_annotation(&child, source, source_symbol_index, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Full-tree type annotation scan
// ---------------------------------------------------------------------------

/// Recursively scan the entire CST for `type` nodes (Python type annotation
/// wrappers) and emit a TypeRef for the identifier inside each one.
///
/// Python grammar uses a `type` node to wrap type annotation expressions such
/// as `-> Foo` or `: Bar`. This catches all parameter and return type
/// annotations anywhere in the file, including inside nested functions and
/// lambdas that the main walker does not descend into.
fn scan_type_annotation_nodes(
    node: tree_sitter::Node,
    source: &str,
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type" if child.is_named() => {
                // Emit a TypeRef at the `type` node's start line for the first
                // non-builtin identifier inside the annotation. Pure-builtin
                // annotations (e.g. bare `str`) emit nothing — there's no real
                // target to reference.
                if let Some(name) = collect_first_nonbuiltin_type_name(&child, source) {
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
            "generic_type" | "union_type" if child.is_named() => {
                if let Some(name) = collect_first_nonbuiltin_type_name(&child, source) {
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
            _ => {}
        }
        scan_type_annotation_nodes(child, source, sym_idx, refs);
    }
}

/// Walk a `type` node and emit TypeRef for any identifier inside it that is
/// not a Python builtin type.
fn emit_type_ref_from_type_node(
    node: &tree_sitter::Node,
    source: &str,
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            if !name.is_empty()
                && !matches!(name.as_str(), "int" | "float" | "str" | "bool" | "bytes"
                    | "None" | "list" | "dict" | "set" | "tuple" | "type" | "object" | "complex")
            {
                refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                    source_symbol_index: sym_idx,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    emit_type_ref_from_type_node(&child, source, sym_idx, refs);
                }
            }
        }
    }
}

/// Walk a `type` node and return the first non-builtin identifier found.
/// Returns `None` only if everything inside is a builtin (e.g. bare `str`).
fn collect_first_nonbuiltin_type_name(
    node: &tree_sitter::Node,
    source: &str,
) -> Option<String> {
    if node.kind() == "identifier" {
        let name = node_text(node, source);
        if !name.is_empty()
            && !matches!(name.as_str(), "int" | "float" | "str" | "bool" | "bytes"
                | "None" | "list" | "dict" | "set" | "tuple" | "type" | "object" | "complex")
        {
            return Some(name);
        }
        return None;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            if let Some(name) = collect_first_nonbuiltin_type_name(&child, source) {
                return Some(name);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

