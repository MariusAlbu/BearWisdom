// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------
//
// Walks every reached PyPI dep root, header-only tree-sitter parses each
// .py/.pyi file, records top-level class/function/assignment names keyed
// against the distribution's normalized name. The Stage 2 loop consults
// this index to pull only files demanded by user refs (or re-export
// follow-through), skipping the rest of the site-packages tree.

use std::path::PathBuf;

use rayon::prelude::*;
use tree_sitter::{Node, Parser};

use super::walk::walk_python_external_root;
use super::SymbolLocationIndex;
use crate::ecosystem::externals::ExternalDepRoot;
use crate::walker::WalkedFile;

pub(crate) fn build_python_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_python_external_root(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }

    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_python_header(&src)
                .into_iter()
                .map(|name| (module.clone(), name, wf.absolute_path.clone()))
                .collect()
        })
        .collect();

    let mut index = SymbolLocationIndex::new();
    for batch in per_file {
        for (module, name, file) in batch {
            index.insert(module, name, file);
        }
    }
    index
}

/// Header-only tree-sitter scan of a Python source file. Returns every
/// top-level declaration's name — classes, functions (including `async
/// def`), module-level assignment targets (`FOO = 1`, `FOO: int = 1`,
/// `FOO, BAR = ...`). Does *not* recurse into function bodies or class
/// bodies; class methods are resolved later against the parsed class once
/// the file is pulled.
pub(super) fn scan_python_header(source: &str) -> Vec<String> {
    let language = tree_sitter_python::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_python_top_level_name(&child, bytes, &mut out);
    }
    out
}

/// Extract the name(s) declared by one direct child of `module` — tree-
/// sitter-python's root-node kind. Decorated definitions wrap the real
/// decl so we recurse through those; `expression_statement` is where
/// module-level assignments sit.
fn collect_python_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "class_definition" | "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        "decorated_definition" => {
            // Decorators wrap the underlying def/class.
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                if matches!(inner.kind(), "class_definition" | "function_definition") {
                    collect_python_top_level_name(&inner, bytes, out);
                }
            }
        }
        "expression_statement" => {
            // Module-level `FOO = ...` / `FOO: T = ...` / `a, b = ...`.
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                if matches!(inner.kind(), "assignment") {
                    if let Some(lhs) = inner.child_by_field_name("left") {
                        collect_assignment_lhs_names(&lhs, bytes, out);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Walk the LHS of an assignment node, recording every identifier name.
/// Handles `FOO`, `FOO: T`, and tuple/list unpacking `a, b = ...`.
fn collect_assignment_lhs_names(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "identifier" => {
            if let Ok(name) = node.utf8_text(bytes) {
                out.push(name.to_string());
            }
        }
        "tuple_pattern" | "list_pattern" | "pattern_list" | "expression_list" | "tuple" => {
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                collect_assignment_lhs_names(&inner, bytes, out);
            }
        }
        // `FOO: T` — the identifier is the left child.
        "typed_parameter" | "assignment" => {
            if let Some(name_node) = node.child_by_field_name("left") {
                collect_assignment_lhs_names(&name_node, bytes, out);
            } else if let Some(name_node) = node.child_by_field_name("name") {
                collect_assignment_lhs_names(&name_node, bytes, out);
            }
        }
        _ => {}
    }
}
