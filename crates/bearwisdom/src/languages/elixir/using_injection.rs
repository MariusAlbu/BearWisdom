// =============================================================================
// elixir/using_injection.rs — `use M` macro-injection modeling.
//
// `use M` runs `M.__using__/1` at compile time; the names it brings into the
// calling module's scope are whatever M's `defmacro __using__` (or an ExUnit
// `CaseTemplate`'s `using do`) `quote` block injects — its `import`/`alias`
// directives, literal `def`/`defmacro` statements, and its own nested `use`
// directives. Those directives live in M's source file, not the calling
// file, so a single-file extractor cannot see them. This module runs once
// per index pass over every parsed Elixir file, records each module's
// injection set keyed by the module's qualified name, and exposes a
// transitive expansion the file-context builder applies at every `use M` /
// `import M` site. The CST detail of extracting one module's injection set —
// quote-block directives, literal defs, the case-template proxy idiom, plain
// top-level `use` — lives in `using_harvest`.
//
// One hop binds what M itself injects (`import M`, `alias M.Repo`); nested
// `use N` directives are resolved transitively by `flattened_injections_for`,
// so a `use DataCase` whose quote block does `use TestUtils` reaches
// `TestUtils`'s `import TestUtils` injection, however many hops deep.
// =============================================================================

use std::collections::{HashMap, HashSet};

use tree_sitter::{Node, Parser};

use super::helpers::node_text;
use super::using_harvest::harvest_module_injections;
use crate::types::ParsedFile;

/// One directive a module's `__using__` quote block injects into every caller
/// of `use <that module>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ElixirInjection {
    /// `import M` — brings M's public functions into bare-name scope. Binds a
    /// bare call to `M.<fn>` through the generic imported-namespace rung.
    Import { module: String },
    /// `alias M.Foo` / `alias M.Foo, as: Bar` — binds the local name to the
    /// module qname through the generic alias→module-qname rung.
    Alias { local: String, module: String },
    /// `use N` — N's own injection set is pulled in transitively.
    Use { module: String },
    /// `def`/`defp`/`defmacro`/`defmacrop <name>(...)` written directly as a
    /// quote-block statement — the function becomes real code in every
    /// consuming module, with no textual trace in that module's own source.
    /// Captured by name only, no arity; BearWisdom resolves calls by name.
    /// `is_macro` mirrors the extractor's own `def`/`defmacro` split
    /// (`SymbolKind::Method` vs `SymbolKind::Function`) so a synthesized
    /// member matches the same `kind_compatible` rules as a hand-written one.
    Def { name: String, is_macro: bool },
}

/// Cross-file Elixir state: every module's `__using__`/`using do` injection
/// set, keyed by the defining module's qualified name.
///
/// Resolvers reach it via `project_ctx.plugin_state.get::<ElixirProjectState>()`.
#[derive(Debug, Default)]
pub struct ElixirProjectState {
    injections: HashMap<String, Vec<ElixirInjection>>,
}

impl ElixirProjectState {
    pub(crate) fn injections_for(&self, module_qname: &str) -> Option<&[ElixirInjection]> {
        self.injections.get(module_qname).map(Vec::as_slice)
    }

    /// The fully resolved injection set for `module_qname`: its own harvested
    /// entries, plus every entry reachable by following nested `Use{X}` hops
    /// transitively. `Use` entries are consumed and never appear in the
    /// result. A module reached more than once (a cycle, or two paths
    /// converging on the same dependency) is expanded only once.
    pub(crate) fn flattened_injections_for(&self, module_qname: &str) -> Vec<&ElixirInjection> {
        let mut out = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        self.flatten_into(module_qname, &mut visited, &mut out);
        out
    }

    // `visited` holds owned `String`s rather than borrows: the qnames it
    // collects come from two different lifetimes (the caller's own
    // `module_qname` argument and `Use { module }` strings borrowed from
    // `self`), which a single borrowed-str set cannot unify.
    fn flatten_into<'a>(
        &'a self,
        module_qname: &str,
        visited: &mut HashSet<String>,
        out: &mut Vec<&'a ElixirInjection>,
    ) {
        if !visited.insert(module_qname.to_string()) {
            return;
        }
        let Some(entries) = self.injections.get(module_qname) else {
            return;
        };
        for inj in entries {
            match inj {
                ElixirInjection::Use { module } => self.flatten_into(module, visited, out),
                other => out.push(other),
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn from_map(injections: HashMap<String, Vec<ElixirInjection>>) -> Self {
        Self { injections }
    }
}

/// Build the module-qname → injection-set map across the whole project.
pub fn build_using_injection_map(parsed: &[ParsedFile]) -> ElixirProjectState {
    let mut injections: HashMap<String, Vec<ElixirInjection>> = HashMap::new();
    for pf in parsed {
        if pf.language != "elixir" {
            continue;
        }
        let Some(src) = pf.content.as_deref() else {
            continue;
        };
        collect_file_injections(src, &mut injections);
    }
    ElixirProjectState { injections }
}

// ---------------------------------------------------------------------------
// Per-file CST walk
// ---------------------------------------------------------------------------

fn collect_file_injections(src: &str, injections: &mut HashMap<String, Vec<ElixirInjection>>) {
    let language: tree_sitter::Language = tree_sitter_elixir::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return;
    }
    let Some(tree) = parser.parse(src, None) else {
        return;
    };
    walk_modules(tree.root_node(), src, "", injections);
}

/// Walk `defmodule` nodes, tracking the qualified-name prefix, and harvest each
/// module's `__using__`/`using do` injection set.
fn walk_modules(
    node: Node,
    src: &str,
    prefix: &str,
    injections: &mut HashMap<String, Vec<ElixirInjection>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some((module_name, do_block)) = module_header(&child, src) {
                let qname = if prefix.is_empty() {
                    module_name
                } else {
                    format!("{prefix}.{module_name}")
                };
                let set = harvest_module_injections(do_block, src);
                if !set.is_empty() {
                    injections.entry(qname.clone()).or_default().extend(set);
                }
                walk_modules(do_block, src, &qname, injections);
                continue;
            }
        }
        walk_modules(child, src, prefix, injections);
    }
}

/// When `node` is a `defmodule Name do … end` call, return `(name, do_block)`.
fn module_header<'a>(node: &Node<'a>, src: &str) -> Option<(String, Node<'a>)> {
    let mut name: Option<String> = None;
    let mut do_block: Option<Node<'a>> = None;
    let mut found_keyword = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" if !found_keyword => {
                if node_text(child, src) == "defmodule" {
                    found_keyword = true;
                }
            }
            "alias" if found_keyword && name.is_none() => {
                name = Some(node_text(child, src));
            }
            "arguments" if found_keyword && name.is_none() => {
                let mut ac = child.walk();
                for arg in child.children(&mut ac) {
                    if arg.kind() == "alias" {
                        name = Some(node_text(arg, src));
                        break;
                    }
                }
            }
            "do_block" => do_block = Some(child),
            _ => {}
        }
    }
    match (found_keyword, name, do_block) {
        (true, Some(n), Some(b)) => Some((n, b)),
        _ => None,
    }
}

#[cfg(test)]
#[path = "using_injection_tests.rs"]
mod tests;
