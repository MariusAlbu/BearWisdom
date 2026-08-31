// =============================================================================
// Directives: alias / import / use / require
//
// Emits Imports edges for Elixir's four scoping directives. Handles single,
// multi (`alias MyApp.{User, Post}`), and `as:`-renamed forms.
// =============================================================================

use super::helpers::{directive_target, node_text};
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Find an `as: <Alias>` keyword pair inside a directive's arguments and
/// return the alias name. Returns `None` when the directive has no `as:`.
///
/// tree-sitter-elixir parses `alias Foo.Bar, as: Baz` as:
///   call
///     identifier "alias"
///     arguments
///       alias "Foo.Bar"
///       keywords
///         pair
///           keyword ":as"
///           alias "Baz"
fn extract_directive_as_alias(node: &Node, src: &str) -> Option<String> {
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
                // pair has children: key (keyword/identifier), value (alias/identifier).
                // Find a "as" key, then return the value's text.
                let mut pc = pair.walk();
                let pair_children: Vec<Node> = pair.children(&mut pc).collect();
                let key_text = pair_children
                    .iter()
                    .find(|c| matches!(c.kind(), "keyword" | "identifier"))
                    .map(|c| node_text(*c, src))
                    .unwrap_or_default();
                // tree-sitter-elixir surfaces the key as `as: ` (with trailing
                // colon-space token) or `:as` depending on form. Strip both
                // colons and surrounding whitespace before comparing.
                let key_norm = key_text
                    .trim()
                    .trim_end_matches(':')
                    .trim_start_matches(':')
                    .trim();
                if key_norm != "as" {
                    continue;
                }
                let value = pair_children
                    .iter()
                    .find(|c| matches!(c.kind(), "alias" | "identifier"))
                    .map(|c| node_text(*c, src))
                    .filter(|s| !s.is_empty());
                if let Some(v) = value {
                    return Some(v);
                }
            }
        }
    }
    None
}

pub(super) fn extract_directive(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
    directive: &str,
) {
    // `import M` brings M's whole public function surface into bare-name
    // scope; `use M` runs M's `__using__/1` macro, which at minimum makes
    // M's own top-level definitions reachable bare in the calling module —
    // both are wildcard-eligible. `alias`/`require` bind only the qualified
    // name itself and never widen bare-name lookup.
    let is_binding = !matches!(directive, "import" | "use");

    // Walk arguments to collect ALL alias/identifier children — handles both
    // single: `alias MyApp.User` and multi: `alias MyApp.{User, Post}`.
    let mut emitted = false;
    // Pre-scan arguments for `as: <alias>` keyword. When present, the emitted
    // Imports ref's target_name uses the alias instead of the module's last
    // segment. Without this, `alias Foo.Bar, as: Baz` extracts as
    // `imported_name = "Bar"`, and the resolver's alias-lookup loop never
    // matches the user's reference to `Baz`.
    let as_alias = extract_directive_as_alias(node, src);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "arguments" => {
                let mut ac = child.walk();
                for arg in child.children(&mut ac) {
                    match arg.kind() {
                        "alias" | "identifier" => {
                            let name = node_text(arg, src);
                            if !name.is_empty() {
                                // Always carry the directive's own target as
                                // `module` — including a single-segment,
                                // undotted name, whose qname IS the bare name
                                // — so `alias_module_qname`/`wildcard_import`
                                // have a module path to search under.
                                let module = Some(name.clone());
                                let default_simple =
                                    name.rsplit('.').next().unwrap_or(&name).to_string();
                                let simple = as_alias.clone().unwrap_or(default_simple);
                                refs.push(ExtractedRef {
                                    is_include: false,
                                    is_import_binding: is_binding,
                                    is_reexport: false,
                                    source_symbol_index: current_symbol_count,
                                    target_name: simple,
                                    kind: EdgeKind::Imports,
                                    line: arg.start_position().row as u32,
                                    col: 0,
                                    module,
                                    chain: None,
                                    byte_offset: arg.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
                                });
                                emitted = true;
                            }
                        }
                        // `alias MyApp.{User, Post}` — list or tuple of aliases after a dot
                        "list" | "tuple" => {
                            let mut lc = arg.walk();
                            for item in arg.children(&mut lc) {
                                if item.kind() == "alias" || item.kind() == "identifier" {
                                    let name = node_text(item, src);
                                    if !name.is_empty() {
                                        let module = Some(name.clone());
                                        let simple =
                                            name.rsplit('.').next().unwrap_or(&name).to_string();
                                        refs.push(ExtractedRef {
                                            is_include: false,
                                            is_import_binding: is_binding,
                                            is_reexport: false,
                                            source_symbol_index: current_symbol_count,
                                            target_name: simple,
                                            kind: EdgeKind::Imports,
                                            line: item.start_position().row as u32,
                                            col: 0,
                                            module,
                                            chain: None,
                                            byte_offset: item.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
                                        });
                                        emitted = true;
                                    }
                                }
                            }
                        }
                        // `alias MyApp.{User, Post}` — tree-sitter-elixir represents this as:
                        //   arguments → dot { alias "MyApp" . tuple "{User, Post}" }
                        // The `dot` node has the module prefix and the right-side tuple of names.
                        "dot" => {
                            emitted |= extract_qualified_multi_alias(
                                &arg,
                                src,
                                current_symbol_count,
                                refs,
                                is_binding,
                            );
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    // Fallback to directive_target if arguments walk didn't find anything.
    if !emitted {
        let target = directive_target(node, src).unwrap_or_default();
        if target.is_empty() {
            return;
        }
        let module = Some(target.clone());
        let simple = target.rsplit('.').next().unwrap_or(&target).to_string();
        refs.push(ExtractedRef {
            is_include: false,
            is_import_binding: is_binding,
            is_reexport: false,
            source_symbol_index: current_symbol_count,
            target_name: simple,
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            col: 0,
            module,
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
}

/// Handle `alias MyApp.{User, Post}` — the `binary_operator` node for `.`
/// whose right side is a `tuple` or `list` containing the module names.
///
/// `is_binding` carries the caller's directive-kind classification (`alias`/
/// `require` bind a qualified name; `import`/`use` are wildcard-eligible)
/// through to the emitted refs.
///
/// Returns true if at least one ref was emitted.
fn extract_qualified_multi_alias(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
    is_binding: bool,
) -> bool {
    // The binary_operator for `MyApp.{User, Post}` has children:
    //   alias "MyApp"  .  tuple "{User, Post}"
    let children: Vec<tree_sitter::Node> = {
        let mut c = node.walk();
        node.children(&mut c).collect()
    };

    // Find the `.` operator.
    let dot_pos = children.iter().position(|c| node_text(*c, src) == ".");
    if dot_pos.is_none() {
        return false;
    }

    // The prefix is the left side (before `.`).
    let prefix = if let Some(left) = children.first() {
        node_text(*left, src)
    } else {
        return false;
    };

    // The right side (after `.`) should be a tuple or list: `{User, Post}`.
    let right = if let Some(dot_idx) = dot_pos {
        children.get(dot_idx + 1)
    } else {
        None
    };

    let right = match right {
        Some(r) => r,
        None => return false,
    };

    let mut emitted = false;
    if right.kind() == "tuple" || right.kind() == "list" || right.kind() == "keywords" {
        let mut rc = right.walk();
        for item in right.children(&mut rc) {
            if item.kind() == "alias" || item.kind() == "identifier" {
                let simple_name = node_text(item, src);
                if !simple_name.is_empty() {
                    let full_module = format!("{prefix}.{simple_name}");
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: is_binding,
                        is_reexport: false,
                        source_symbol_index: current_symbol_count,
                        target_name: simple_name,
                        kind: EdgeKind::Imports,
                        line: item.start_position().row as u32,
                        col: 0,
                        module: Some(full_module),
                        chain: None,
                        byte_offset: item.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                    emitted = true;
                }
            }
        }
    } else if right.kind() == "alias" || right.kind() == "identifier" {
        // Fallback: `alias MyApp.User` as binary_operator form.
        let name = format!("{prefix}.{}", node_text(*right, src));
        let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
        refs.push(ExtractedRef {
            is_include: false,
            is_import_binding: is_binding,
            is_reexport: false,
            source_symbol_index: current_symbol_count,
            target_name: simple,
            kind: EdgeKind::Imports,
            line: right.start_position().row as u32,
            col: 0,
            module: Some(name),
            chain: None,
            byte_offset: right.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
        emitted = true;
    }

    emitted
}
