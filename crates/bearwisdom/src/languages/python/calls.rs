// =============================================================================
// python/calls.rs  —  Call extraction and import helpers for Python
// =============================================================================

use super::helpers::node_text;
use crate::types::{CallArg, ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use rustc_hash::FxHashSet;
use std::collections::HashMap;
use tree_sitter::Node;

/// Maximum nesting depth for recursive `CallArg` construction. Arguments
/// deeper than this collapse to `CallArg::Other` rather than recursing further.
const MAX_ARG_DEPTH: u32 = 32;

/// Extract positional arguments from a Python `call` node's `argument_list`.
/// Captures string literals (including triple-quoted forms), bare
/// identifiers, integer / float / `True` / `False` / `None`, lists of
/// string literals (used by Flask's `methods=['GET']`), and the recursive
/// expression shapes (conditional, list, await, splat, subscript, binary).
/// Keyword args and shapes the recursion doesn't cover become
/// `CallArg::Other`.
pub(super) fn extract_call_args(call_node: &Node, src: &str) -> Vec<CallArg> {
    let Some(args_node) = call_node.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = args_node.walk();
    for child in args_node.named_children(&mut cursor) {
        out.push(extract_arg(&child, src, 0));
    }
    out
}

/// Convert a single argument expression node to a `CallArg`, recursing for
/// composite expression kinds up to `MAX_ARG_DEPTH`.
fn extract_arg(node: &Node, src: &str, depth: u32) -> CallArg {
    if depth >= MAX_ARG_DEPTH {
        return CallArg::Other;
    }
    match node.kind() {
        "string" => CallArg::StringLit(strip_python_string(&node_text(node, src))),
        "concatenated_string" => CallArg::StringLit(
            node_text(node, src)
                .replace('"', "")
                .replace('\'', "")
                .replace("\\n", ""),
        ),
        "identifier" => CallArg::Ident(node_text(node, src)),
        "integer" | "float" => CallArg::Literal(node_text(node, src)),
        "true" | "false" | "none" => CallArg::Literal(node_text(node, src)),

        // `a if cond else b` — the condition's type does not affect the
        // result type; only the two value branches are preserved. The
        // grammar exposes no fields: the three named children are in source
        // order `[then, cond, else]`.
        "conditional_expression" => {
            let then_branch = node
                .named_child(0)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let else_branch = node
                .named_child(2)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Ternary {
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            }
        }

        // `[elem0, elem1, ...]` — recurse on each named child. Splat elements
        // inside (`*xs`) surface as `CallArg::Spread` children.
        "list" => {
            let mut cursor = node.walk();
            let elements = node
                .named_children(&mut cursor)
                .map(|child| extract_arg(&child, src, depth + 1))
                .collect();
            CallArg::ArrayLiteral { elements }
        }

        // `await expr` — single primary-expression child, no field.
        "await" => {
            let inner = node
                .named_child(0)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Await { expr: Box::new(inner) }
        }

        // `*xs` (iterable unpack) / `**kw` (mapping unpack) — single child,
        // no field. Both map to the spread operand.
        "list_splat" | "dictionary_splat" => {
            let inner = node
                .named_child(0)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Spread { expr: Box::new(inner) }
        }

        // `container[index]` — `value` is the container, `subscript` is the
        // (possibly multiple) index; take the first index expression.
        "subscript" => {
            let container = node
                .child_by_field_name("value")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let index = node
                .child_by_field_name("subscript")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::IndexAccess {
                container: Box::new(container),
                index: Box::new(index),
            }
        }

        // `left op right` for arithmetic (`+`, `*`, ...) and boolean
        // (`and` / `or`) operators — both expose `left` / `operator` /
        // `right` fields.
        "binary_operator" | "boolean_operator" => {
            let op = node
                .child_by_field_name("operator")
                .map(|n| node_text(&n, src))
                .unwrap_or_default();
            let left = node
                .child_by_field_name("left")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let right = node
                .child_by_field_name("right")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            }
        }

        // `a == b`, `a < b`, `a is not b`, ... — the grammar gives no
        // left/right fields here: operands are the named children and the
        // operator is the unnamed token between them (`is not` / `not in`
        // are single tokens). Capture the first two operands and that
        // operator token.
        "comparison_operator" => {
            let mut cursor = node.walk();
            let operands: Vec<Node> = node.named_children(&mut cursor).collect();
            let op = first_anonymous_token_text(node, src);
            let left = operands
                .first()
                .map(|n| extract_arg(n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let right = operands
                .get(1)
                .map(|n| extract_arg(n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            }
        }

        // `lambda u: u.name` — capture the lambda's own positional parameter
        // names so the chain walker can type them from the higher-order
        // method's callback-parameter signature.
        "lambda" => CallArg::Lambda { params: lambda_param_names(node, src) },

        _ => CallArg::Other,
    }
}

/// Collect the positional parameter identifier names of a Python `lambda`
/// argument. Names live under the `parameters` field as a `lambda_parameters`
/// node whose children are `identifier`s. A non-identifier binding (tuple
/// pattern, default, splat) yields an empty slot so positions stay aligned
/// with the callback signature.
fn lambda_param_names(node: &Node, src: &str) -> Vec<String> {
    let Some(params) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .map(|p| {
            if p.kind() == "identifier" {
                node_text(&p, src)
            } else {
                String::new()
            }
        })
        .collect()
}

/// Return the source text of the first unnamed (operator) token child of
/// `node`. Used for `comparison_operator`, whose operator has no field.
fn first_anonymous_token_text(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            return node_text(&child, src);
        }
    }
    String::new()
}

/// Strip surrounding quotes (single, double, triple-single, triple-double)
/// from a Python string literal's source text. Leaves `b`/`r`/`f` prefixes
/// intact since those don't appear in route URLs / call arg values where
/// this matters.
fn strip_python_string(raw: &str) -> String {
    let trimmed = raw
        .trim_start_matches('b')
        .trim_start_matches('r')
        .trim_start_matches('f')
        .trim_start_matches('B')
        .trim_start_matches('R')
        .trim_start_matches('F');
    trimmed
        .trim_start_matches("\"\"\"")
        .trim_end_matches("\"\"\"")
        .trim_start_matches("'''")
        .trim_end_matches("'''")
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

// ---------------------------------------------------------------------------
// Import map builder
// ---------------------------------------------------------------------------

/// Build a map from local name → fully-qualified module path by scanning the
/// immediate children of `root` for `import_statement` and
/// `import_from_statement` nodes.
///
/// Mapping rules:
/// - `import json`           → `"json"  → "json"`
/// - `import foo.bar`        → `"foo"   → "foo.bar"` (first segment is local)
/// - `import foo.bar as fb`  → `"fb"    → "foo.bar"`
/// - `from foo.bar import Baz`     → `"Baz" → "foo.bar"`
/// - `from foo import bar as b`    → `"b"   → "foo"`
///
/// Only the top-level module scope is scanned (not function bodies), which is
/// where almost all Python imports live.
pub(super) fn build_import_map(root: tree_sitter::Node, source: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                let mut ic = child.walk();
                for item in child.children(&mut ic) {
                    match item.kind() {
                        "dotted_name" => {
                            let full = node_text(&item, source);
                            // `import foo.bar` — local name is the first segment
                            let local = full.split('.').next().unwrap_or(&full).to_string();
                            map.insert(local, full);
                        }
                        "aliased_import" => {
                            // `import foo.bar as fb`
                            if let (Some(name_node), Some(alias_node)) = (
                                item.child_by_field_name("name"),
                                item.child_by_field_name("alias"),
                            ) {
                                let full = node_text(&name_node, source);
                                let alias = node_text(&alias_node, source);
                                if !alias.is_empty() {
                                    map.insert(alias, full);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "import_from_statement" => {
                let module = match child.child_by_field_name("module_name") {
                    Some(m) => node_text(&m, source).trim_start_matches('.').to_string(),
                    None => continue,
                };
                let module_id = child
                    .child_by_field_name("module_name")
                    .map(|n| n.id());

                let mut ic = child.walk();
                for item in child.children(&mut ic) {
                    if module_id.map_or(false, |id| item.id() == id) {
                        continue;
                    }
                    match item.kind() {
                        "dotted_name" | "identifier" => {
                            let name = node_text(&item, source);
                            if !name.is_empty() {
                                map.insert(name, module.clone());
                            }
                        }
                        "aliased_import" => {
                            // `from foo import bar as b`
                            if let Some(alias_node) = item.child_by_field_name("alias") {
                                let alias = node_text(&alias_node, source);
                                if !alias.is_empty() {
                                    map.insert(alias, module.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_calls_from_body(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    import_map: &HashMap<String, String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(func_node) = child.child_by_field_name("function") {
                let func_name = node_text(&func_node, source);

                // `isinstance(user, Admin)` — emit TypeRef to the second argument.
                // Also emit a Calls edge so the coverage engine's `call` node budget
                // is satisfied (isinstance IS a call, just with extra semantics).
                if func_name == "isinstance" {
                    extract_isinstance_type_ref(&child, source, source_symbol_index, refs);
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index,
                        target_name: "isinstance".to_string(),
                        kind: EdgeKind::Calls,
                        line: func_node.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: func_node.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                    extract_calls_from_body(&child, source, source_symbol_index, refs, import_map);
                    continue;
                }

                let chain = build_chain(&func_node, source);

                // Resolve module for qualified calls: if the chain root matches an
                // imported name, annotate the ref with its source module so the
                // resolver can trace `Person.objects.filter()` back to
                // `posthog.models` (where `Person` was imported from).
                let resolved_module = chain.as_ref().and_then(|c| {
                    if c.segments.len() >= 2 {
                        let root_name = &c.segments[0].name;
                        import_map.get(root_name).cloned()
                    } else {
                        None
                    }
                });

                let target_name = chain
                    .as_ref()
                    .and_then(|c| c.segments.last())
                    .map(|s| s.name.clone())
                    .or_else(|| {
                        let t = node_text(&func_node, source);
                        Some(t.rsplit('.').next().unwrap_or(&t).to_string())
                    });

                crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &func_node, refs);
                if let Some(target_name) = target_name {
                    let call_args = extract_call_args(&child, source);
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index,
                        target_name,
                        kind: EdgeKind::Calls,
                        line: func_node.start_position().row as u32,
                        col: 0,
                        module: resolved_module,
                        chain,
                        byte_offset: func_node.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args,
});
                }
            }
        }
        extract_calls_from_body(&child, source, source_symbol_index, refs, import_map);
    }
}

/// Emit TypeRef edges for `isinstance(obj, SomeClass)` or
/// `isinstance(obj, (ClassA, ClassB))`.
///
/// Python `call` node structure:
/// ```text
/// call
///   function: identifier "isinstance"
///   arguments: argument_list
///     identifier "obj"
///     "," (anonymous)
///     identifier "Admin"       ← single type
///     -- or --
///     tuple "(" identifier "Admin" "," identifier "User" ")"
/// ```
fn extract_isinstance_type_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let args = match node.child_by_field_name("arguments") {
        Some(a) => a,
        None => return,
    };

    // Collect all named argument children (skip commas / parens).
    let named_args: Vec<_> = {
        let mut cursor = args.walk();
        args.children(&mut cursor)
            .filter(|c| c.is_named() && c.kind() != "comment")
            .collect()
    };

    // Second argument (index 1) is the type or tuple of types.
    let type_arg = match named_args.get(1) {
        Some(a) => *a,
        None => return,
    };

    emit_isinstance_type_node(&type_arg, source, source_symbol_index, refs);
}

/// Emit TypeRef(s) for a type argument in `isinstance` — handles both a single
/// type identifier and a tuple of types `(Admin, User)`.
fn emit_isinstance_type_node(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            if !name.is_empty() {
                refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
        // `isinstance(x, (Admin, User))` — tuple of types.
        "tuple" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(&child, source);
                    if !name.is_empty() {
                        refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line: child.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
});
                    }
                }
            }
        }
        // `isinstance(x, pkg.MyClass)` — attribute access.
        "attribute" => {
            let name = node_text(node, source);
            if !name.is_empty() {
                refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

pub(super) fn build_chain(node: &Node, src: &str) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: &Node, src: &str, segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, src);
            let kind = if name == "self" || name == "cls" {
                SegmentKind::SelfRef
            } else {
                SegmentKind::Identifier
            };
            segments.push(ChainSegment {
                name,
                node_kind: "identifier".to_string(),
                kind,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                type_arg_ids: Vec::new(),
});
            Some(())
        }

        "attribute" => {
            let object = node.child_by_field_name("object")?;
            let attribute = node.child_by_field_name("attribute")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(&attribute, src),
                node_kind: "attribute".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                type_arg_ids: Vec::new(),
});
            Some(())
        }

        "call" => {
            let func = node.child_by_field_name("function")?;
            build_chain_inner(&func, src, segments)
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_import_statement(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    dunder_all: &FxHashSet<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                let full = node_text(&child, source);
                let parts: Vec<&str> = full.split('.').collect();
                let target = parts.last().unwrap_or(&full.as_str()).to_string();
                let module = if parts.len() > 1 {
                    Some(parts[..parts.len() - 1].join("."))
                } else {
                    None
                };
                // `import foo.bar` binds the TOP segment `foo` as the local name.
                let local = parts.first().copied().unwrap_or(full.as_str());
                refs.push(ExtractedRef { is_import_binding: false, is_reexport: dunder_all.contains(local),
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    col: 0,
                    module,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let full = node_text(&name_node, source);
                    let parts: Vec<&str> = full.split('.').collect();
                    let target = parts.last().unwrap_or(&full.as_str()).to_string();
                    let module = if parts.len() > 1 {
                        Some(parts[..parts.len() - 1].join("."))
                    } else {
                        None
                    };
                    // `import foo.bar as fb` binds the alias `fb` as the local name.
                    let local = child
                        .child_by_field_name("alias")
                        .map(|a| node_text(&a, source))
                        .unwrap_or_else(|| parts.first().map(|s| s.to_string()).unwrap_or_default());
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: dunder_all.contains(&local),
                        source_symbol_index: current_symbol_count,
                        target_name: target,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        col: 0,
                        module,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
            _ => {}
        }
    }
}

pub(super) fn extract_import_from_statement(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    dunder_all: &FxHashSet<String>,
) {
    let module = node.child_by_field_name("module_name").map(|m| {
        node_text(&m, source).trim_start_matches('.').to_string()
    });

    let module_name_node = node.child_by_field_name("module_name");

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "from" | "import" | "," | "import_prefix" => continue,
            _ => {}
        }
        if let Some(ref mn) = module_name_node {
            if child.id() == mn.id() {
                continue;
            }
        }

        match child.kind() {
            "dotted_name" | "identifier" => {
                let name = node_text(&child, source);
                // `from .mod import A` binds `A` (the imported name) locally.
                let is_reexport = dunder_all.contains(&name);
                refs.push(ExtractedRef { is_import_binding: false, is_reexport,
                    source_symbol_index: current_symbol_count,
                    target_name: name,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: module.clone(),
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    // The ref carries the SOURCE-side `name` for following; the
                    // LOCAL bound name is the alias when present, else `name`.
                    let name = node_text(&name_node, source);
                    let local = child
                        .child_by_field_name("alias")
                        .map(|a| node_text(&a, source))
                        .unwrap_or_else(|| name.clone());
                    let is_reexport = dunder_all.contains(&local);
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport,
                        source_symbol_index: current_symbol_count,
                        target_name: name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: module.clone(),
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
            "wildcard_import" => {
                refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                    source_symbol_index: current_symbol_count,
                    target_name: "*".to_string(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: module.clone(),
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
            _ => {}
        }
    }
}
// =============================================================================
// F-string interpolation (low priority -- call extraction only)
// =============================================================================

/// Extract calls from f-string interpolation expressions.
pub(super) fn extract_fstring_calls(
    node: &Node,
    source: &str,
    enclosing_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    import_map: &HashMap<String, String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "interpolation" || child.kind() == "fstring_expression" {
            let mut ic = child.walk();
            for expr in child.children(&mut ic) {
                if expr.is_named() {
                    extract_calls_from_body(&expr, source, enclosing_symbol_index, refs, import_map);
                }
            }
        }
    }
}
