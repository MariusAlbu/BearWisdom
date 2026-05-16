// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

use std::path::PathBuf;

use rayon::prelude::*;
use tree_sitter::{Node, Parser};

use super::walk::walk_dart_root;
use super::SymbolLocationIndex;
use crate::ecosystem::externals::ExternalDepRoot;
use crate::walker::WalkedFile;

pub(crate) fn build_dart_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_dart_root(dep) {
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
            scan_dart_header(&src)
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

/// Header-only tree-sitter scan of a Dart source file. Records top-level
/// class / mixin / enum / extension / function / typedef names.
fn scan_dart_header(source: &str) -> Vec<String> {
    let language: tree_sitter::Language = tree_sitter_dart::LANGUAGE.into();
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
        collect_dart_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_dart_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "class_declaration"
        | "class_definition"
        | "mixin_declaration"
        | "enum_declaration"
        | "extension_declaration"
        | "function_signature"
        | "function_declaration"
        | "getter_signature"
        | "setter_signature"
        | "type_alias" => {
            if let Some(name_node) = node
                .child_by_field_name("name")
                .or_else(|| find_first_named_child(node, "identifier"))
            {
                if let Ok(t) = name_node.utf8_text(bytes) {
                    out.push(t.to_string());
                }
            }
        }
        _ => {}
    }
}

fn find_first_named_child<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}
