// =============================================================================
// elixir/using_injection.rs — `use M` macro-injection modeling.
//
// `use M` runs `M.__using__/1` at compile time; the names it brings into the
// calling module's scope are whatever M's `defmacro __using__` (or an ExUnit
// `CaseTemplate`'s `using do`) `quote` block injects — its `import`/`alias`
// directives and its own nested `use` directives. Those directives live in M's
// source file, not the calling file, so a single-file extractor cannot see
// them. This module runs once per index pass over every parsed Elixir file,
// records each module's injection set keyed by the module's qualified name,
// and exposes a transitive expansion the file-context builder applies at every
// `use M` site.
//
// One hop binds what M itself injects (`import M`, `alias M.Repo`); the nested
// `use N` directives recurse so a `use DataCase` whose quote block does
// `use TestUtils` reaches `TestUtils`'s `import TestUtils` injection.
// =============================================================================

use std::collections::HashMap;

use tree_sitter::{Node, Parser};

use super::helpers::node_text;
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
                let set = harvest_using(do_block, src);
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

/// Scan a module body for a `defmacro __using__(…)` or `using do` form and
/// return the injection set from its `quote` block. Recurses through non-`call`
/// wrapper nodes (a multi-statement `do_block` nests its statements in a
/// `block`) but stops at the `using`/`defmacro` form itself.
fn harvest_using(body: Node, src: &str) -> Vec<ElixirInjection> {
    let mut out = Vec::new();
    harvest_using_inner(body, src, &mut out);
    out
}

fn harvest_using_inner(node: Node, src: &str, out: &mut Vec<ElixirInjection>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(callee) = call_head(&child, src) {
                // A nested `defmodule` owns its own `__using__`; `walk_modules`
                // harvests it under that module's qname. Don't pull it up.
                if callee == "defmodule" {
                    continue;
                }
                let is_using = match callee.as_str() {
                    "using" => true,
                    "defmacro" | "defmacrop" => first_arg_is_using(&child, src),
                    _ => false,
                };
                if is_using {
                    if let Some(do_block) = do_block_of(&child) {
                        collect_quote_directives(do_block, src, out);
                    }
                    continue;
                }
            }
        }
        harvest_using_inner(child, src, out);
    }
}

/// The directive set inside a `quote do … end`. Walks the form's do-block for a
/// nested `quote` call and harvests the `import`/`alias`/`use` directives in it.
fn collect_quote_directives(node: Node, src: &str, out: &mut Vec<ElixirInjection>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(callee) = call_head(&child, src) {
                if callee == "quote" {
                    if let Some(quote_block) = do_block_of(&child) {
                        harvest_directives(quote_block, src, out);
                    }
                    continue;
                }
            }
        }
        collect_quote_directives(child, src, out);
    }
}

/// Harvest `import`/`alias`/`use` directives that are direct statements of a
/// quote block.
fn harvest_directives(quote_block: Node, src: &str, out: &mut Vec<ElixirInjection>) {
    let mut cursor = quote_block.walk();
    for child in quote_block.children(&mut cursor) {
        if child.kind() != "call" {
            continue;
        }
        let Some(callee) = call_head(&child, src) else {
            continue;
        };
        match callee.as_str() {
            "import" => {
                if let Some(module) = directive_module(&child, src) {
                    out.push(ElixirInjection::Import { module });
                }
            }
            "use" => {
                if let Some(module) = directive_module(&child, src) {
                    out.push(ElixirInjection::Use { module });
                }
            }
            "alias" => {
                for (local, module) in alias_targets(&child, src) {
                    out.push(ElixirInjection::Alias { local, module });
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Small CST accessors
// ---------------------------------------------------------------------------

/// The leading `identifier`/`alias` callee name of a `call` node.
fn call_head(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "alias" => return Some(node_text(child, src)),
            _ => {}
        }
    }
    None
}

/// True when a `defmacro` call's first argument is the `__using__` head.
fn first_arg_is_using(node: &Node, src: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "arguments" {
            let mut ac = child.walk();
            for arg in child.children(&mut ac) {
                // `defmacro __using__(opts)` parses the head as a nested `call`
                // whose own callee identifier is `__using__`.
                if arg.kind() == "call" {
                    if let Some(head) = call_head(&arg, src) {
                        return head == "__using__";
                    }
                }
                if arg.kind() == "identifier" {
                    return node_text(arg, src) == "__using__";
                }
            }
        }
    }
    false
}

fn do_block_of<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == "do_block");
    found
}

/// The module path of a single-target `import M` / `use M` directive.
fn directive_module(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "arguments" {
            let mut ac = child.walk();
            for arg in child.children(&mut ac) {
                if arg.kind() == "alias" {
                    return Some(node_text(arg, src));
                }
            }
        }
    }
    None
}

/// The `(local, module)` bindings of an `alias` directive — handles the plain
/// form, the `as:` rename, and the `alias M.{A, B}` multi form.
fn alias_targets(node: &Node, src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let as_alias = alias_as_rename(node, src);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "arguments" {
            continue;
        }
        let mut ac = child.walk();
        for arg in child.children(&mut ac) {
            match arg.kind() {
                "alias" => {
                    let module = node_text(arg, src);
                    let last = module.rsplit('.').next().unwrap_or(&module).to_string();
                    let local = as_alias.clone().unwrap_or(last);
                    out.push((local, module));
                }
                // `alias M.{A, B}` — `dot` node: `alias "M" . tuple "{A, B}"`.
                "dot" => collect_multi_alias(&arg, src, &mut out),
                _ => {}
            }
        }
    }
    out
}

/// The `as: Name` rename of an `alias` directive, if present.
fn alias_as_rename(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "arguments" {
            continue;
        }
        let mut ac = child.walk();
        for arg in child.children(&mut ac) {
            if arg.kind() != "keywords" {
                continue;
            }
            let mut kc = arg.walk();
            for pair in arg.children(&mut kc) {
                if pair.kind() != "pair" {
                    continue;
                }
                let children: Vec<Node> = {
                    let mut pc = pair.walk();
                    pair.children(&mut pc).collect()
                };
                let key = children
                    .iter()
                    .find(|c| matches!(c.kind(), "keyword" | "identifier"))
                    .map(|c| node_text(*c, src))
                    .unwrap_or_default();
                let key = key.trim().trim_end_matches(':').trim_start_matches(':').trim();
                if key != "as" {
                    continue;
                }
                if let Some(v) = children
                    .iter()
                    .find(|c| matches!(c.kind(), "alias" | "identifier"))
                    .map(|c| node_text(*c, src))
                    .filter(|s| !s.is_empty())
                {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// `alias M.{A, B}` — emit `(A, M.A)`, `(B, M.B)`.
fn collect_multi_alias(dot: &Node, src: &str, out: &mut Vec<(String, String)>) {
    let children: Vec<Node> = {
        let mut c = dot.walk();
        dot.children(&mut c).collect()
    };
    let Some(dot_pos) = children.iter().position(|c| node_text(*c, src) == ".") else {
        return;
    };
    let Some(prefix) = children.first().map(|c| node_text(*c, src)) else {
        return;
    };
    let Some(right) = children.get(dot_pos + 1) else {
        return;
    };
    if matches!(right.kind(), "tuple" | "list" | "keywords") {
        let mut rc = right.walk();
        for item in right.children(&mut rc) {
            if matches!(item.kind(), "alias" | "identifier") {
                let name = node_text(item, src);
                if !name.is_empty() {
                    out.push((name.clone(), format!("{prefix}.{name}")));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "using_injection_tests.rs"]
mod tests;
