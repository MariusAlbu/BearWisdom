// =============================================================================
// elixir/using_harvest.rs — CST detail for one module's `__using__` injection
// set: the directives its `using`/`defmacro __using__` quote block injects,
// PLUS every plain `use X` statement at the module's own top level (Elixir
// compiles `use X` into literal code in whichever module contains it,
// regardless of whether that statement sits inside a using-quote or directly
// in the module body — both count as "this module's own injected surface").
//
// Three directive shapes are harvested from a quote block: `import`/`use`/
// `alias` (delegated to `ElixirInjection`'s existing variants) and, new here,
// literal `def`/`defp`/`defmacro`/`defmacrop` statements — a macro that
// defines a function directly inside its `__using__` quote (ExMachina's
// `build/1,2,3`) makes that function real code in every consuming module,
// with no textual trace in the consumer's own source.
//
// A nested `defmacro __using__(args) do … end` found INSIDE a quote block is
// the "case template" idiom (`ExUnit.CaseTemplate`): the outer quote doesn't
// inject directives directly, it re-defines `__using__` for whoever uses the
// *consuming* module, delegating to a same-module helper function. That
// helper's own `quote do … end` carries the real directives, one call away —
// `follow_proxy_call` resolves the delegate by name within the same module
// body and harvests its quote the same way.
// =============================================================================

use tree_sitter::Node;

use super::helpers::{call_identifier, function_name_arity, node_text};
use super::using_injection::ElixirInjection;

/// Harvest one module's OWN injection set: its `using`/`defmacro __using__`
/// quote-block directives, plus any plain top-level `use X` statements.
pub(super) fn harvest_module_injections(body: Node, src: &str) -> Vec<ElixirInjection> {
    let mut out = Vec::new();
    harvest_using_form(body, src, body, &mut out);
    harvest_plain_uses(body, src, &mut out);
    out
}

// ---------------------------------------------------------------------------
// `using`/`defmacro __using__` quote-block harvesting
// ---------------------------------------------------------------------------

/// Recurse through `node` looking for the module's `using`/`defmacro
/// __using__` form and harvest its quote block. `module_body` is carried
/// through unchanged — it is the module's own do_block, the search scope
/// `follow_proxy_call` uses when a nested `defmacro __using__` delegates to a
/// sibling helper function. Recurses through non-`call` wrapper nodes (a
/// multi-statement `do_block` nests its statements in a `block`) but stops at
/// the `using`/`defmacro` form itself. A nested `defmodule` owns its own
/// `__using__`, harvested separately under that module's qname — not pulled
/// up here.
fn harvest_using_form(node: Node, src: &str, module_body: Node, out: &mut Vec<ElixirInjection>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(callee) = call_head(&child, src) {
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
                        collect_quote_directives(do_block, src, module_body, out);
                    }
                    continue;
                }
            }
        }
        harvest_using_form(child, src, module_body, out);
    }
}

/// The directive set inside a `quote do … end`. Walks the form's do-block for
/// a nested `quote` call and harvests the `import`/`alias`/`use`/`def`
/// directives in it.
fn collect_quote_directives(node: Node, src: &str, module_body: Node, out: &mut Vec<ElixirInjection>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(callee) = call_head(&child, src) {
                if callee == "quote" {
                    if let Some(quote_block) = do_block_of(&child) {
                        harvest_directives(quote_block, src, module_body, out);
                    }
                    continue;
                }
            }
        }
        collect_quote_directives(child, src, module_body, out);
    }
}

/// Harvest `import`/`use`/`alias`/`def`-family directives that are direct
/// statements of a quote block. A nested `defmacro __using__` is the
/// case-template idiom, not a real member — its delegate call is followed
/// instead of being recorded as a `Def`.
fn harvest_directives(quote_block: Node, src: &str, module_body: Node, out: &mut Vec<ElixirInjection>) {
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
            "def" | "defp" | "defmacro" | "defmacrop" => {
                let (name, _arity) = function_name_arity(&child, src);
                if name.is_empty() {
                    continue;
                }
                if name == "__using__" {
                    if let Some(nested_do) = do_block_of(&child) {
                        follow_proxy_call(nested_do, module_body, src, out);
                    }
                    continue;
                }
                let is_macro = matches!(callee.as_str(), "defmacro" | "defmacrop");
                out.push(ElixirInjection::Def { name, is_macro });
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Case-template proxy follow
// ---------------------------------------------------------------------------

/// `nested_do_block` is a `defmacro __using__(args) do … end` found INSIDE
/// another module's own `__using__` quote. Its body is a call that delegates
/// to a helper defined elsewhere in the SAME module (`unquote(__MODULE__).
/// __proxy__(__MODULE__, opts)`); resolve the delegate by name within
/// `module_body` and harvest its own `quote do … end` the same way a direct
/// `using`/`defmacro __using__` form would be harvested.
fn follow_proxy_call(nested_do_block: Node, module_body: Node, src: &str, out: &mut Vec<ElixirInjection>) {
    let mut cursor = nested_do_block.walk();
    for child in nested_do_block.children(&mut cursor) {
        match child.kind() {
            "call" => {
                if let Some(fn_name) = call_identifier(&child, src) {
                    if let Some(helper_do_block) = find_sibling_def_body(module_body, &fn_name, src) {
                        collect_quote_directives(helper_do_block, src, module_body, out);
                    }
                }
            }
            "block" => follow_proxy_call(child, module_body, src, out),
            _ => {}
        }
    }
}

/// Find `def name(...) do … end` (any def/defmacro kind) as a direct
/// statement of `scope`, and return its do_block.
fn find_sibling_def_body<'a>(scope: Node<'a>, name: &str, src: &str) -> Option<Node<'a>> {
    let mut cursor = scope.walk();
    for child in scope.children(&mut cursor) {
        match child.kind() {
            "call" => {
                let (def_name, _arity) = function_name_arity(&child, src);
                if def_name == name {
                    return do_block_of(&child);
                }
            }
            "block" => {
                if let Some(found) = find_sibling_def_body(child, name, src) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Plain top-level `use X`
// ---------------------------------------------------------------------------

/// Plain `use X` statements at the module's own top level (siblings of, not
/// nested inside, a `using`/`defmacro __using__` form) compile X's injected
/// surface directly into THIS module just as surely as a `using`-quoted
/// `use X` does — captured the same way so a later `use <this module>` or
/// `import <this module>` sees it too.
fn harvest_plain_uses(body: Node, src: &str, out: &mut Vec<ElixirInjection>) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "call" => {
                if let Some(callee) = call_head(&child, src) {
                    if callee == "use" {
                        if let Some(module) = directive_module(&child, src) {
                            out.push(ElixirInjection::Use { module });
                        }
                    }
                }
            }
            "block" => harvest_plain_uses(child, src, out),
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
