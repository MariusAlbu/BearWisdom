// =============================================================================
// ecosystem/dart_sdk_symbol_index.rs — Stage 2 demand symbol location index
//
// Builds the `(module, name) → file` map `DartSdkEcosystem` and
// `FlutterSdkEcosystem` offer for demand-driven materialization: a
// header-only tree-sitter scan of every `.dart` file under the given dep
// roots, collecting top-level declaration names without a full parse.
// =============================================================================

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use tree_sitter::{Node, Parser};

use crate::ecosystem::externals::ExternalDepRoot;
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::walker::WalkedFile;

/// Build a `(module, name) → file` index over the given dep roots using
/// a header-only tree-sitter parse. Called by both `DartSdkEcosystem` and
/// `FlutterSdkEcosystem` (via `dart_sdk::build_dart_symbol_index`).
pub(crate) fn build_dart_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        collect_dart_files_recursive(&dep.root, dep, &mut work);
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
            scan_dart_top_level(&src)
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

fn collect_dart_files_recursive(
    dir: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<(String, WalkedFile)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') || matches!(name, "test" | "tests") {
                    continue;
                }
            }
            collect_dart_files_recursive(&path, dep, out);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".dart") {
                continue;
            }
            let rel = path.to_string_lossy().replace('\\', "/");
            out.push((
                dep.module_path.clone(),
                WalkedFile {
                    relative_path: format!("ext:{}:{}", dep.ecosystem, rel),
                    absolute_path: path,
                    language: "dart",
                },
            ));
        }
    }
}

/// Header-only tree-sitter scan — top-level class/mixin/enum/extension/fn names.
fn scan_dart_top_level(source: &str) -> Vec<String> {
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
    let mut out = Vec::new();
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
                .or_else(|| find_first_identifier(node))
            {
                if let Ok(t) = name_node.utf8_text(bytes) {
                    out.push(t.to_string());
                }
            }
        }
        _ => {}
    }
}

fn find_first_identifier<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return Some(child);
        }
    }
    None
}

#[cfg(test)]
#[path = "dart_sdk_symbol_index_tests.rs"]
mod tests;
