// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline scaffolding)
// ---------------------------------------------------------------------------
//
// Builds a cheap `(module_path, symbol_name) → file` map over every reached
// Go dep root by tree-sitter parsing each .go file and reading ONLY the
// top-level declarations (functions, methods, types, vars, consts). Function
// bodies are never walked — their inner nodes aren't inspected, so the
// allocation profile is O(#top_level_decls) rather than O(#ast_nodes). That
// cost is the one-time entry fee the demand-driven Stage 2 pipeline pays so
// it can then pull just the files its demand set actually needs.
//
// File scope matches `resolve_go_requested_packages` — only sub-packages the
// user imports, plus within-module transitives up to GO_SUBPKG_MAX_DEPTH.
// Platform-mismatched files are dropped via `go_platform::file_matches_host`.

use std::path::PathBuf;

use rayon::prelude::*;
use tree_sitter::{Node, Parser};

use super::reachability::resolve_go_requested_packages;
use super::SymbolLocationIndex;
use crate::ecosystem::externals::ExternalDepRoot;
use crate::walker::WalkedFile;

pub(crate) fn build_go_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    // Collect every walked file + its owning module path so each parallel
    // scanner task is self-contained.
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in resolve_go_requested_packages(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }

    // Parallel header-only scan. Each task returns (module, name, file)
    // tuples; we merge into one index at the end so the hot map isn't
    // under a lock during the scan.
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_go_header(&src)
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

/// Header-only tree-sitter scan of a Go source file. Returns the list of
/// top-level declaration names the file exports (or defines locally — the
/// caller filters by visibility if needed). Methods are keyed as
/// `ReceiverType.MethodName`; struct/interface/alias types are keyed by
/// their type name; top-level vars and consts are keyed by their identifier.
///
/// Bodies of functions and methods are *not* walked — we only inspect the
/// direct children of `source_file` and the immediate children of the
/// type/var/const declarations. No `block` is descended.
pub(super) fn scan_go_header(source: &str) -> Vec<String> {
    let language = tree_sitter_go::LANGUAGE.into();
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
        match child.kind() {
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(bytes) {
                        out.push(name.to_string());
                    }
                }
            }
            "method_declaration" => {
                let (recv, name) = method_decl_names(&child, source);
                match (recv, name) {
                    (Some(recv), Some(name)) => out.push(format!("{recv}.{name}")),
                    // Receiver type couldn't be parsed — still record the
                    // bare method name so unresolved lookups have something
                    // to match against.
                    (None, Some(name)) => out.push(name),
                    _ => {}
                }
            }
            "type_declaration" => {
                let mut sub_cursor = child.walk();
                for spec in child.children(&mut sub_cursor) {
                    if matches!(spec.kind(), "type_spec" | "type_alias") {
                        if let Some(name_node) = spec.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                out.push(name.to_string());
                            }
                        }
                    }
                }
            }
            "var_declaration" | "const_declaration" => {
                // Grouped declarations `var ( ... )` wrap their specs in a
                // `var_spec_list` / `const_spec_list` intermediate; single
                // specs are direct children. Handle both.
                collect_var_const_names(&child, bytes, &mut out);
            }
            _ => {}
        }
    }

    out
}

/// Walk a `var_declaration` / `const_declaration` node and append every
/// declared name onto `out`. Handles the grouped form `var ( a = 1; b = 2 )`
/// where tree-sitter-go wraps the specs in a `var_spec_list` intermediate.
fn collect_var_const_names(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "var_spec" | "const_spec" => collect_spec_names(&child, bytes, out),
            "var_spec_list" | "const_spec_list" => {
                // Recurse one level to reach the individual specs.
                let mut inner = child.walk();
                for spec in child.children(&mut inner) {
                    if matches!(spec.kind(), "var_spec" | "const_spec") {
                        collect_spec_names(&spec, bytes, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Pull identifier names out of a single `var_spec` / `const_spec`. Stops
/// at the first non-identifier named child (the declared type) or at `=`
/// (the start of the RHS expression list).
fn collect_spec_names(spec: &Node, bytes: &[u8], out: &mut Vec<String>) {
    let mut cursor = spec.walk();
    let mut past_names = false;
    for cc in spec.children(&mut cursor) {
        if !cc.is_named() {
            if cc.utf8_text(bytes) == Ok("=") {
                past_names = true;
            }
            continue;
        }
        if past_names {
            break;
        }
        if cc.kind() == "identifier" {
            if let Ok(name) = cc.utf8_text(bytes) {
                out.push(name.to_string());
            }
        } else {
            past_names = true;
        }
    }
}

/// Pull `(receiver_type_name, method_name)` out of a `method_declaration`
/// node without walking its body. Handles `*T`, `T`, and `T[K]` receivers.
fn method_decl_names(node: &Node, source: &str) -> (Option<String>, Option<String>) {
    let mut receiver: Option<String> = None;
    let mut method: Option<String> = None;
    let mut seen_receiver_list = false;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        match child.kind() {
            "parameter_list" if !seen_receiver_list => {
                seen_receiver_list = true;
                receiver = extract_receiver_type(&child, source);
            }
            "field_identifier" if method.is_none() => {
                if let Ok(s) = child.utf8_text(source.as_bytes()) {
                    method = Some(s.to_string());
                }
            }
            _ => {}
        }
    }
    (receiver, method)
}

fn extract_receiver_type(param_list: &Node, source: &str) -> Option<String> {
    let bytes = source.as_bytes();
    let mut cursor = param_list.walk();
    for child in param_list.children(&mut cursor) {
        if child.kind() != "parameter_declaration" {
            continue;
        }
        let mut ccursor = child.walk();
        for cc in child.children(&mut ccursor) {
            if !cc.is_named() {
                continue;
            }
            match cc.kind() {
                "type_identifier" => {
                    return cc.utf8_text(bytes).ok().map(String::from);
                }
                "pointer_type" => {
                    // *T — find the inner type_identifier / generic_type.
                    let mut inner = cc.walk();
                    for t in cc.children(&mut inner) {
                        if t.kind() == "type_identifier" {
                            return t.utf8_text(bytes).ok().map(String::from);
                        }
                        if t.kind() == "generic_type" {
                            if let Some(inner_name) = t.child_by_field_name("type") {
                                return inner_name.utf8_text(bytes).ok().map(String::from);
                            }
                        }
                    }
                }
                "generic_type" => {
                    // T[K] — unwrap to T.
                    if let Some(inner_name) = cc.child_by_field_name("type") {
                        return inner_name.utf8_text(bytes).ok().map(String::from);
                    }
                }
                _ => {}
            }
        }
    }
    None
}
