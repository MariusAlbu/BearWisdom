// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

use std::path::PathBuf;

use rayon::prelude::*;
use tree_sitter::{Node, Parser};

use super::walk::walk_hex_root;
use super::SymbolLocationIndex;
use crate::ecosystem::externals::ExternalDepRoot;
use crate::walker::WalkedFile;

pub(crate) fn build_hex_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_hex_root(dep) {
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
            let names = match wf.language {
                "elixir" => scan_elixir_header(&src),
                "erlang" => scan_erlang_header(&src),
                "gleam" => scan_gleam_header(&src),
                _ => Vec::new(),
            };
            names
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

/// Header-only tree-sitter scan of an Elixir source file. Records every
/// top-level `defmodule Foo.Bar` name and, inside each module body, the
/// names declared via `def`, `defp`, `defmacro`, `defstruct`, `defguard`,
/// `defmodule` (nested), `def type`, `defprotocol`, `defimpl`. Function
/// bodies are never descended.
pub(super) fn scan_elixir_header(source: &str) -> Vec<String> {
    let language = tree_sitter_elixir::LANGUAGE.into();
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
    walk_elixir_body(&root, bytes, &mut out, 0);
    out
}

fn walk_elixir_body(node: &Node, bytes: &[u8], out: &mut Vec<String>, depth: u32) {
    if depth > 6 { return }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Elixir's grammar represents every form as a `call` node whose first
        // `identifier` child is the macro name (`defmodule`, `def`, `defp`,
        // `defmacro`, `defstruct`, `defprotocol`, `defimpl`, `defguard`),
        // followed by the form's arguments. We walk the whole tree because
        // module-body statements are wrapped in `do_block` / `body` nodes
        // under the outer call.
        if child.kind() == "call" {
            let mut ic = child.walk();
            let children: Vec<Node> = child.children(&mut ic).collect();
            if let Some(head) = children.iter().find(|n| n.kind() == "identifier") {
                if let Ok(head_text) = head.utf8_text(bytes) {
                    let is_decl = matches!(
                        head_text,
                        "defmodule" | "def" | "defp" | "defmacro" | "defmacrop"
                            | "defstruct" | "defprotocol" | "defimpl" | "defguard"
                            | "defguardp" | "defdelegate" | "defexception" | "defcallback"
                    );
                    if is_decl {
                        if let Some(args) = children.iter().find(|n| n.kind() == "arguments") {
                            if let Some(name) = first_elixir_arg_name(args, bytes) {
                                out.push(name);
                            }
                        }
                    }
                }
            }
        }
        // Always recurse so nested def/defmodule inside do_block body get
        // visited — the grammar introduces `do_block` / `stab_clause`
        // intermediates we don't have to enumerate explicitly.
        walk_elixir_body(&child, bytes, out, depth + 1);
    }
}

fn first_elixir_arg_name(args: &Node, bytes: &[u8]) -> Option<String> {
    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        match child.kind() {
            "alias" | "identifier" => {
                if let Ok(t) = child.utf8_text(bytes) {
                    return Some(t.to_string());
                }
            }
            // `def foo(x, y)` — the LHS is a `call` whose head identifier is
            // the function name.
            "call" => {
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if inner.kind() == "identifier" {
                        if let Ok(t) = inner.utf8_text(bytes) {
                            return Some(t.to_string());
                        }
                    }
                }
            }
            // `def foo when ... do ... end` wraps into `binary_operator`.
            "binary_operator" => {
                return first_elixir_arg_name(&child, bytes);
            }
            _ => {}
        }
    }
    None
}

/// Header-only tree-sitter scan of an Erlang source file. Records every
/// top-level `function_declaration` name. Erlang also has `-record(...)`
/// and `-type(...)` attribute forms; record definitions are captured since
/// the chain walker references records as types.
pub(super) fn scan_erlang_header(source: &str) -> Vec<String> {
    let language = tree_sitter_erlang::LANGUAGE.into();
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
            "fun_decl" | "function_declaration" | "function" => {
                // Erlang function clause head: `name(args) -> body.`
                // The first `atom` child is the function name.
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if inner.kind() == "atom" {
                        if let Ok(t) = inner.utf8_text(bytes) {
                            out.push(t.to_string());
                            break;
                        }
                    }
                    // clause-wrapped: walk one level down.
                    if inner.kind() == "function_clause" || inner.kind() == "clause" {
                        let mut cc = inner.walk();
                        for sub in inner.children(&mut cc) {
                            if sub.kind() == "atom" {
                                if let Ok(t) = sub.utf8_text(bytes) {
                                    out.push(t.to_string());
                                }
                                break;
                            }
                        }
                        break;
                    }
                }
            }
            "attribute" | "record_decl" | "type_alias" => {
                // `-record(name, {...}).` — second atom is the record name.
                let mut ic = child.walk();
                let mut seen_first_atom = false;
                for inner in child.children(&mut ic) {
                    if inner.kind() == "atom" {
                        if !seen_first_atom { seen_first_atom = true; continue }
                        if let Ok(t) = inner.utf8_text(bytes) {
                            out.push(t.to_string());
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Header-only tree-sitter scan of a Gleam source file. Records top-level
/// `pub fn`, `pub type`, `pub const`, plus their private counterparts.
pub(super) fn scan_gleam_header(source: &str) -> Vec<String> {
    let language = tree_sitter_gleam::LANGUAGE.into();
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
            "function" | "constant" | "type_definition" | "external_function"
            | "external_type" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(t) = name_node.utf8_text(bytes) {
                        out.push(t.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    out
}
