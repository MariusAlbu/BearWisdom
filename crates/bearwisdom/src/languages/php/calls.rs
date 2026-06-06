// =============================================================================
// php/calls.rs  —  Call extraction and import ref helpers for PHP
// =============================================================================

use super::helpers::node_text;
use crate::types::{CallArg, ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

/// Maximum nesting depth for recursive `CallArg` construction. Expressions
/// deeper than this collapse to `CallArg::Other` rather than recursing further.
const MAX_ARG_DEPTH: u32 = 8;

// ---------------------------------------------------------------------------
// Argument extraction
// ---------------------------------------------------------------------------

/// Extract positional arguments from a PHP `arguments` node belonging to a
/// member_call / static_call / function_call. Captures string and encapsed
/// string literals, name and qualified_name identifiers (so chains like
/// `Model::class` resolve to the bare class name), numeric and boolean
/// literals, and the recursive expression shapes (ternary, array, spread,
/// subscript, binary). Recursion is capped at `MAX_ARG_DEPTH` levels.
pub(super) fn extract_call_args(call_node: &Node, src: &[u8]) -> Vec<CallArg> {
    let args_node = match call_node.child_by_field_name("arguments") {
        Some(n) => n,
        None => {
            let mut cursor = call_node.walk();
            let found = call_node
                .children(&mut cursor)
                .find(|c| c.kind() == "arguments");
            match found {
                Some(n) => n,
                None => return Vec::new(),
            }
        }
    };
    let mut out = Vec::new();
    let mut cursor = args_node.walk();
    for child in args_node.named_children(&mut cursor) {
        // PHP grammar wraps each positional value in an `argument` node.
        let value_node = if child.kind() == "argument" {
            let mut ac = child.walk();
            let mut inner = None;
            for v in child.named_children(&mut ac) {
                if v.kind() != "name" || node_text(&v, src) != "name" {
                    inner = Some(v);
                    break;
                }
            }
            match inner {
                Some(n) => n,
                None => continue,
            }
        } else {
            child
        };
        out.push(extract_arg(&value_node, src, 0));
    }
    out
}

/// Convert a single PHP expression node to a `CallArg`, recursing for composite
/// expression kinds up to `MAX_ARG_DEPTH`.
fn extract_arg(node: &Node, src: &[u8], depth: u32) -> CallArg {
    if depth >= MAX_ARG_DEPTH {
        return CallArg::Other;
    }
    match node.kind() {
        "string" => CallArg::StringLit(strip_php_string(&node_text(node, src))),
        "encapsed_string" => CallArg::TemplateLit(strip_php_string(&node_text(node, src))),
        "integer" | "float" => CallArg::Literal(node_text(node, src)),
        "true" | "false" | "null" => CallArg::Literal(node.kind().to_string()),
        "name" | "identifier" | "qualified_name" => {
            let raw = node_text(node, src);
            let simple = raw.rsplit('\\').next().unwrap_or(&raw).to_string();
            CallArg::Ident(simple)
        }
        // `User::class` — class_constant_access_expression (PHP 8 grammar).
        "class_constant_access_expression" | "scoped_property_access_expression" => {
            let class_node = node
                .child_by_field_name("scope")
                .or_else(|| node.child_by_field_name("class"));
            if let Some(cn) = class_node {
                let raw = node_text(&cn, src);
                let simple = raw.rsplit('\\').next().unwrap_or(&raw).to_string();
                CallArg::Ident(simple)
            } else {
                CallArg::Other
            }
        }
        "variable_name" => {
            let raw = node_text(node, src);
            CallArg::Ident(raw.trim_start_matches('$').to_string())
        }
        // Recursive expression shapes — produce structured variants instead of Other.
        // `cond ? body : alternative` (and the short form `cond ?: alternative`,
        // where `body` is absent and the then-value is the condition itself).
        // The condition's type does not affect the result type; only the two
        // value branches are preserved.
        "conditional_expression" => {
            let then_node = node
                .child_by_field_name("body")
                .or_else(|| node.child_by_field_name("condition"));
            let else_node = node.child_by_field_name("alternative");
            let then_branch = then_node
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let else_branch = else_node
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Ternary {
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            }
        }
        // `[$a, $b]` / `array($a, $b)` — each element is wrapped in an
        // `array_element_initializer`. Recurse on the element value; `...$x`
        // spreads inside the array surface as `CallArg::Spread` children.
        "array_creation_expression" => {
            let mut cursor = node.walk();
            let elements = node
                .named_children(&mut cursor)
                .filter(|c| c.kind() == "array_element_initializer")
                .map(|elem| extract_array_element(&elem, src, depth + 1))
                .collect();
            CallArg::ArrayLiteral { elements }
        }
        // `...$args` — variadic unpacking carries the unpacked operand as its
        // sole named child.
        "variadic_unpacking" => {
            let inner = node
                .named_child(0)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Spread {
                expr: Box::new(inner),
            }
        }
        // `$arr[$i]` — positional children: container then index. `$arr[]`
        // (append) has no index child, leaving the index `Other`.
        "subscript_expression" => {
            let container = node
                .named_child(0)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let index = node
                .named_child(1)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::IndexAccess {
                container: Box::new(container),
                index: Box::new(index),
            }
        }
        // `$a + $b`, `$a . $b` — capture operator text and recurse on operands.
        "binary_expression" => {
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
        // `fn($u) => $u->name`, `function($a, $b) { ... }` — capture the
        // closure's own positional parameter names (without the `$` sigil) so
        // the chain walker can type them from the higher-order method's
        // callback-parameter signature.
        "arrow_function" | "anonymous_function" => {
            CallArg::Lambda { params: php_lambda_param_names(node, src) }
        }
        _ => CallArg::Other,
    }
}

/// Collect the positional parameter names of a PHP arrow-function /
/// anonymous-function argument. The `parameters` field is a `formal_parameters`
/// list of `simple_parameter` nodes whose `name` field is a `variable_name`
/// (`$u`); the leading `$` sigil is stripped. A parameter without a plain
/// variable name yields an empty slot so positions stay aligned with the
/// callback signature.
fn php_lambda_param_names(node: &Node, src: &[u8]) -> Vec<String> {
    let Some(params) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter(|p| p.kind() == "simple_parameter" || p.kind() == "variadic_parameter")
        .map(|p| {
            p.child_by_field_name("name")
                .map(|n| node_text(&n, src).trim_start_matches('$').to_string())
                .unwrap_or_default()
        })
        .collect()
}

/// Convert one `array_element_initializer` to a `CallArg`. A plain element
/// (`$v`) recurses on its value; a key=>value pair (`$k => $v`) takes the
/// value (last expression child); a `...$x` element produces a `Spread`.
fn extract_array_element(node: &Node, src: &[u8], depth: u32) -> CallArg {
    if depth >= MAX_ARG_DEPTH {
        return CallArg::Other;
    }
    let mut cursor = node.walk();
    let exprs: Vec<Node> = node
        .named_children(&mut cursor)
        .filter(|c| c.kind() != "by_ref")
        .collect();
    match exprs.last() {
        Some(value) => extract_arg(value, src, depth),
        None => CallArg::Other,
    }
}

fn strip_php_string(raw: &str) -> String {
    raw.trim_start_matches('"')
        .trim_end_matches('"')
        .trim_start_matches('\'')
        .trim_end_matches('\'')
        .to_string()
}

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Each arm that recurses explicitly MUST `continue` — otherwise the
    // unconditional recursion at the bottom of the loop visits the same
    // subtree again, doubling work at every nesting level. A method chain
    // N deep (common in Laravel fluent builders) otherwise costs O(2^N)
    // recursions and ref-pushes, blowing memory into hundreds of MiB.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "member_call_expression" | "nullsafe_member_call_expression" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let callee = node_text(&name_node, src);
                    let chain = build_chain(&child, src);
                    let call_args = extract_call_args(&child, src);
                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &name_node, refs);
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index,
                        target_name: callee,
                        kind: EdgeKind::Calls,
                        line: name_node.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain,
                        byte_offset: name_node.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args,
                    });
                }
                // Recurse into the object expression and arguments to find nested calls.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
                continue;
            }

            "static_call_expression" | "scoped_call_expression" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let callee = node_text(&name_node, src);
                    let chain = build_chain(&child, src);
                    let call_args = extract_call_args(&child, src);
                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &name_node, refs);
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index,
                        target_name: callee,
                        kind: EdgeKind::Calls,
                        line: name_node.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain,
                        byte_offset: name_node.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args,
                    });
                }
                // Recurse into arguments to find nested calls.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
                continue;
            }

            "object_creation_expression" => {
                let cls_node_opt = if let Some(n) = child.child_by_field_name("class_type") {
                    Some(n)
                } else {
                    let mut c = child.walk();
                    let mut found = None;
                    for n in child.children(&mut c) {
                        if n.kind() == "name"
                            || n.kind() == "qualified_name"
                            || n.kind() == "identifier"
                            || n.kind() == "variable_name"
                        {
                            found = Some(n);
                            break;
                        }
                    }
                    found
                };
                if let Some(cls_node) = cls_node_opt {
                    let cls_name = node_text(&cls_node, src);
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index,
                        target_name: cls_name,
                        kind: EdgeKind::Instantiates,
                        line: cls_node.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: cls_node.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }

            "function_call_expression" => {
                if let Some(fn_node) = child.child_by_field_name("function") {
                    let callee = node_text(&fn_node, src);
                    let simple = callee.rsplit('\\').next().unwrap_or(&callee).to_string();
                    let call_args = extract_call_args(&child, src);
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index,
                        target_name: simple,
                        kind: EdgeKind::Calls,
                        line: fn_node.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: fn_node.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args,
                    });
                }
            }

            // `match($x) { 1 => 'one', default => 'other' }` — recurse into arms.
            "match_expression" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
                continue;
            }

            // `fn($x) => $x->name` — recurse into body.
            "arrow_function" => {
                if let Some(body) = child.child_by_field_name("body") {
                    extract_calls_from_body(&body, src, source_symbol_index, refs);
                }
                continue;
            }

            // `function() use ($x) { ... }` — anonymous function.
            // Extract calls from the body; `use` clause variables are already in scope.
            "anonymous_function_creation_expression" => {
                if let Some(body) = child.child_by_field_name("body") {
                    extract_calls_from_body(&body, src, source_symbol_index, refs);
                }
                continue;
            }

            // `include 'file.php'` / `require_once 'config.php'` — emit Imports edge.
            "include_expression" | "include_once_expression"
            | "require_expression" | "require_once_expression" => {
                extract_include_require(&child, src, refs, source_symbol_index);
            }

            // `"Hello $name and {$obj->method()}"` — interpolated string with embedded expressions.
            "encapsed_string" => {
                extract_encapsed_string_calls(&child, src, source_symbol_index, refs);
                continue;
            }

            _ => {}
        }
        extract_calls_from_body(&child, src, source_symbol_index, refs);
    }
}

/// Extract an Imports edge from an `include`/`require`/`include_once`/`require_once` expression.
pub(super) fn extract_include_require(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    source_symbol_index: usize,
) {
    // The path expression is the only named child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" || child.kind() == "encapsed_string" {
            let raw = node_text(&child, src);
            // Strip surrounding quotes.
            let path = raw
                .trim_start_matches('"')
                .trim_end_matches('"')
                .trim_start_matches('\'')
                .trim_end_matches('\'')
                .to_string();
            if path.is_empty() {
                continue;
            }
            let parts: Vec<&str> = path.split('/').collect();
            let target = parts
                .last()
                .unwrap_or(&path.as_str())
                .trim_end_matches(".php")
                .to_string();
            let module = if parts.len() > 1 {
                Some(parts[..parts.len() - 1].join("/"))
            } else {
                None
            };
            refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                source_symbol_index,
                target_name: target,
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
}

/// Extract calls from interpolated expressions inside a PHP double-quoted string.
///
/// `"Hello {$obj->greet()}"` — the `{$obj->greet()}` part is an embedded expression.
fn extract_encapsed_string_calls(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `{$expr}` — variable_name or expression inside braces.
            "variable_name" => {} // simple var, no call
            // Expression-level interpolation (e.g. method call inside `{...}`).
            _ if child.is_named() => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Variable extraction helpers for PHP
// ---------------------------------------------------------------------------

/// Extract Variable symbols from a PHP `foreach` statement.
///
/// ```text
/// foreach ($items as $key => $value) { ... }
/// ```
pub(super) fn extract_foreach_vars(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<crate::types::ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    source_symbol_index: usize,
) {
    use crate::types::{ExtractedSymbol, SymbolKind, Visibility};
    use super::helpers::{qualify, scope_from_prefix};

    // Value field: `$value` (or `$key => $value` — the value is the last binding).
    if let Some(value_node) = node.child_by_field_name("value") {
        push_php_foreach_var(&value_node, src, symbols, parent_index, qualified_prefix);
    }
    // Key field (optional): `$key`.
    if let Some(key_node) = node.child_by_field_name("key") {
        push_php_foreach_var(&key_node, src, symbols, parent_index, qualified_prefix);
    }

    // Recurse into the body.
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, src, source_symbol_index, refs);
    }

    // Fallback: walk children for variable_name nodes when fields are absent.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "body" || child.kind() == "foreach" || child.kind() == "as" {
            continue;
        }
        if child.kind() == "variable_name" {
            use super::helpers::{qualify, scope_from_prefix};
            let raw = node_text(&child, src);
            let name = raw.trim_start_matches('$').to_string();
            if !name.is_empty() && name != "this" {
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name: qualify(&name, qualified_prefix),
                    kind: SymbolKind::Variable,
                    visibility: Some(Visibility::Public),
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: None,
                    scope_path: scope_from_prefix(qualified_prefix),
                    parent_index,
                                    byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
            }
        }
    }
}

fn push_php_foreach_var(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    use crate::types::{ExtractedSymbol, SymbolKind, Visibility};
    use super::helpers::{qualify, scope_from_prefix};

    // Resolve the effective variable_name node.
    let var_found: Option<tree_sitter::Node>;
    if node.kind() == "by_ref" {
        // `foreach ($items as &$item)` — walk children to find variable_name.
        let mut found = None;
        for i in 0..node.child_count() {
            if let Some(ch) = node.child(i) {
                if ch.kind() == "variable_name" {
                    found = Some(ch);
                    break;
                }
            }
        }
        var_found = found;
    } else if node.kind() == "variable_name" {
        var_found = Some(*node);
    } else {
        var_found = None;
    }

    if let Some(var_node) = var_found {
        let raw = node_text(&var_node, src);
        let name = raw.trim_start_matches('$').to_string();
        if !name.is_empty() && name != "this" {
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: qualify(&name, qualified_prefix),
                kind: SymbolKind::Variable,
                visibility: Some(Visibility::Public),
                start_line: var_node.start_position().row as u32,
                end_line: var_node.end_position().row as u32,
                start_col: var_node.start_position().column as u32,
                end_col: var_node.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index,
                            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
        }
    }
}

/// Extract TypeRef edges from catch clauses in a `try_statement`.
pub(super) fn extract_try_catch_types(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<crate::types::ExtractedRef>,
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    source_symbol_index: usize,
) {
    use crate::types::{EdgeKind, ExtractedSymbol, SymbolKind, Visibility};
    use super::helpers::{qualify, scope_from_prefix};

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `try { ... }` body.
            "compound_statement" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `catch (ExceptionType $e) { ... }`.
            "catch_clause" => {
                // Collect exception type(s).
                if let Some(type_node) = child.child_by_field_name("type") {
                    extract_catch_type_refs(&type_node, src, refs, source_symbol_index);
                }
                // Catch variable.
                if let Some(var_node) = child.child_by_field_name("variable") {
                    let raw = node_text(&var_node, src);
                    let name = raw.trim_start_matches('$').to_string();
                    if !name.is_empty() {
                        symbols.push(ExtractedSymbol {
                            name: name.clone(),
                            qualified_name: qualify(&name, qualified_prefix),
                            kind: SymbolKind::Variable,
                            visibility: Some(Visibility::Public),
                            start_line: var_node.start_position().row as u32,
                            end_line: var_node.end_position().row as u32,
                            start_col: var_node.start_position().column as u32,
                            end_col: var_node.end_position().column as u32,
                            signature: None,
                            doc_comment: None,
                            scope_path: scope_from_prefix(qualified_prefix),
                            parent_index,
                                                    byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
                    }
                }
                // Recurse into catch body.
                let mut cc = child.walk();
                for cb in child.children(&mut cc) {
                    if cb.kind() == "compound_statement" {
                        extract_calls_from_body(&cb, src, source_symbol_index, refs);
                    }
                }
            }
            // `finally { ... }`.
            "finally_clause" => {
                let mut fc = child.walk();
                for fb in child.children(&mut fc) {
                    if fb.kind() == "compound_statement" {
                        extract_calls_from_body(&fb, src, source_symbol_index, refs);
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_catch_type_refs(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<crate::types::ExtractedRef>,
    source_symbol_index: usize,
) {
    use crate::types::EdgeKind;
    match node.kind() {
        "named_type" | "name" | "qualified_name" => {
            let name = node_text(node, src);
            let simple = name.rsplit('\\').next().unwrap_or(&name).to_string();
            if !simple.is_empty() {
                refs.push(crate::types::ExtractedRef { is_import_binding: false, is_reexport: false,
                    source_symbol_index,
                    target_name: simple,
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
        // `ExceptionA|ExceptionB` — union of exception types.
        "union_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_catch_type_refs(&child, src, refs, source_symbol_index);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_catch_type_refs(&child, src, refs, source_symbol_index);
                }
            }
        }
    }
}

/// Extract Variable symbols from `list($a, $b) = ...` or `[$a, $b] = ...`.
pub(super) fn extract_list_destructuring(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    use crate::types::{ExtractedSymbol, SymbolKind, Visibility};
    use super::helpers::{qualify, scope_from_prefix};

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable_name" => {
                let raw = node_text(&child, src);
                let name = raw.trim_start_matches('$').to_string();
                if !name.is_empty() && name != "this" {
                    symbols.push(ExtractedSymbol {
                        name: name.clone(),
                        qualified_name: qualify(&name, qualified_prefix),
                        kind: SymbolKind::Variable,
                        visibility: Some(Visibility::Public),
                        start_line: child.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: child.start_position().column as u32,
                        end_col: child.end_position().column as u32,
                        signature: None,
                        doc_comment: None,
                        scope_path: scope_from_prefix(qualified_prefix),
                        parent_index,
                                            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
                }
            }
            // Nested array destructure element.
            "array_element" | "list_literal" | "array_creation_expression" => {
                extract_list_destructuring(&child, src, symbols, parent_index, qualified_prefix);
            }
            _ => {}
        }
    }
}

/// Extract TypeRef edges from a PHP type node, handling nullable, union, and intersection types.
pub(super) fn extract_type_refs_from_php_type(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<crate::types::ExtractedRef>,
    source_symbol_index: usize,
) {
    use crate::types::EdgeKind;
    match node.kind() {
        // `?string` — nullable type: unwrap to inner type.
        "nullable_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_php_type(&child, src, refs, source_symbol_index);
                }
            }
        }
        // `string|int` — union type.
        "union_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_php_type(&child, src, refs, source_symbol_index);
                }
            }
        }
        // `Foo&Bar` — intersection type.
        "intersection_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_php_type(&child, src, refs, source_symbol_index);
                }
            }
        }
        // `(A&B)|C` — disjunctive normal form type (PHP 8.2+).
        "disjunctive_normal_form_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_php_type(&child, src, refs, source_symbol_index);
                }
            }
        }
        "named_type" | "name" | "qualified_name" | "identifier" => {
            let name = node_text(node, src);
            let simple = name.rsplit('\\').next().unwrap_or(&name).to_string();
            // Skip PHP built-in scalar types.
            if !simple.is_empty()
                && !matches!(
                    simple.as_str(),
                    "string" | "int" | "float" | "bool" | "array" | "object" | "null"
                        | "void" | "never" | "mixed" | "callable" | "iterable"
                        | "self" | "static" | "parent"
                )
            {
                refs.push(crate::types::ExtractedRef { is_import_binding: false, is_reexport: false,
                    source_symbol_index,
                    target_name: simple,
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

pub(super) fn build_chain(node: &Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: &Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "variable_name" => {
            let raw = node_text(node, src);
            let name = raw.trim_start_matches('$').to_string();
            let kind = if name == "this" {
                SegmentKind::SelfRef
            } else {
                SegmentKind::Identifier
            };
            segments.push(ChainSegment {
                name,
                node_kind: "variable_name".to_string(),
                kind,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
});
            Some(())
        }

        "name" | "identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
});
            Some(())
        }

        "member_access_expression" => {
            let object = node.child_by_field_name("object")?;
            let name_node = node.child_by_field_name("name")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(&name_node, src),
                node_kind: "member_access_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
});
            Some(())
        }

        "member_call_expression" => {
            let object = node.child_by_field_name("object")?;
            let name_node = node.child_by_field_name("name")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(&name_node, src),
                node_kind: "member_call_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
});
            Some(())
        }

        "static_call_expression" | "scoped_call_expression" => {
            // `scoped_call_expression` (PHP 8 grammar) uses field "scope" for the
            // class and field "name" for the method.  Older grammars may use "class".
            // Try both so the chain is built regardless of grammar version.
            let class_node = node
                .child_by_field_name("scope")
                .or_else(|| node.child_by_field_name("class"))?;
            let name_node = node.child_by_field_name("name")?;
            segments.push(ChainSegment {
                name: node_text(&class_node, src),
                node_kind: "class".to_string(),
                kind: SegmentKind::TypeAccess,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
});
            segments.push(ChainSegment {
                name: node_text(&name_node, src),
                node_kind: "static_call_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
});
            Some(())
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Use declaration / import reference extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_use_declaration(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_use_clause" => {
                push_use_ref_for_name(&child, src, refs, current_symbol_count);
            }
            "qualified_name" | "name" => {
                let full = node_text(&child, src);
                push_fq_import(full, child.start_position().row as u32, child.start_byte() as u32, refs, current_symbol_count);
            }
            _ => {}
        }
    }
}

fn push_use_ref_for_name(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" {
            let full = node_text(&child, src);
            push_fq_import(full, child.start_position().row as u32, child.start_byte() as u32, refs, current_symbol_count);
            return;
        }
    }
}

/// Push an Imports edge for a fully-qualified PHP name like `Foo\Bar\Baz`.
fn push_fq_import(
    full: String,
    line: u32,
    byte_offset: u32,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let parts: Vec<&str> = full.split('\\').collect();
    let target = parts.last().unwrap_or(&full.as_str()).to_string();
    let module = if parts.len() > 1 {
        Some(parts[..parts.len() - 1].join("\\"))
    } else {
        None
    };
    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: current_symbol_count,
        target_name: target,
        kind: EdgeKind::Imports,
        line,
        module,
        chain: None,
        byte_offset,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
            col: 0,
});
}

pub(super) fn extract_trait_use(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" {
            let full = node_text(&child, src);
            let parts: Vec<&str> = full.split('\\').collect();
            let target = parts.last().unwrap_or(&full.as_str()).to_string();
            let module = if parts.len() > 1 {
                Some(parts[..parts.len() - 1].join("\\"))
            } else {
                None
            };
            refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                source_symbol_index: current_symbol_count.saturating_sub(1),
                target_name: target,
                kind: EdgeKind::Implements,
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
}
