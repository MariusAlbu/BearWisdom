// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

use std::path::PathBuf;

use rayon::prelude::*;
use tree_sitter::{Node, Parser};

use super::walk_cargo_root;
use super::SymbolLocationIndex;
use crate::ecosystem::externals::ExternalDepRoot;

pub(crate) fn build_cargo_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, crate::walker::WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_cargo_root(dep) {
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
            scan_rust_header(&src)
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

/// Header-only tree-sitter scan of a Rust source file. Returns top-level
/// item names — structs, enums, unions, traits, type aliases, functions,
/// constants, statics, macros. Function / method / impl bodies are never
/// descended; we record `ReceiverType::method_name` inside `impl` blocks so
/// the chain walker can locate methods the same way it does on Go.
pub(super) fn scan_rust_header(source: &str) -> Vec<String> {
    let language = tree_sitter_rust::LANGUAGE.into();
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
        collect_rust_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_rust_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "function_item"
        | "function_signature_item"
        | "struct_item"
        | "union_item"
        | "enum_item"
        | "type_item"
        | "const_item"
        | "static_item"
        | "mod_item"
        | "macro_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        "trait_item" => {
            // Trait declaration: emit the trait name itself, then the bare
            // names of every associated item (method, const, type) inside
            // the trait body so chain-walker / by-name lookups can locate
            // the file. Mirrors `impl_item` exactly except the receiver
            // here is the trait, not a concrete type, so we also emit the
            // qualified `Trait::method` form for type-scoped lookups.
            //
            // Real motivation: axum's `IntoResponse::into_response` lives
            // here. Without this branch the trait file is walked but its
            // method names never enter `SymbolLocationIndex`, so the
            // `expand.rs` bare-name fallback can never locate the file
            // for `x.into_response()` calls in user code.
            let trait_name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(bytes).ok().map(String::from));
            if let Some(name) = trait_name.as_ref() {
                out.push(name.clone());
            }
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for inner in body.children(&mut cursor) {
                    if matches!(
                        inner.kind(),
                        "function_item"
                            | "function_signature_item"
                            | "associated_type"
                            | "const_item"
                            | "type_item"
                    ) {
                        if let Some(name_node) = inner.child_by_field_name("name") {
                            if let Ok(method) = name_node.utf8_text(bytes) {
                                out.push(method.to_string());
                                if let Some(t) = trait_name.as_ref() {
                                    out.push(format!("{t}::{method}"));
                                }
                            }
                        }
                    }
                }
            }
        }
        "impl_item" => {
            // `impl Foo { fn bar() {} }` — surface the receiver name plus each
            // associated item so methods are locatable as `Foo::bar`.
            let recv = node
                .child_by_field_name("type")
                .and_then(|t| rust_type_identifier(&t, bytes));
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for inner in body.children(&mut cursor) {
                    if matches!(
                        inner.kind(),
                        "function_item"
                            | "function_signature_item"
                            | "associated_type"
                            | "const_item"
                    ) {
                        if let Some(name_node) = inner.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                out.push(name.to_string());
                                if let Some(recv) = recv.as_ref() {
                                    out.push(format!("{recv}::{name}"));
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Extract a simple type identifier from `impl <Type>`'s type node, unwrapping
/// generic and reference wrappers. Returns None for unrecognized shapes.
fn rust_type_identifier(node: &Node, bytes: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => node.utf8_text(bytes).ok().map(String::from),
        "generic_type" | "reference_type" | "scoped_type_identifier" => {
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                if let Some(name) = rust_type_identifier(&inner, bytes) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}
