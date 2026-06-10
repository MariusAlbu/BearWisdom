// =============================================================================
// languages/common/call_args.rs — shared call-argument extraction
//
// Language-agnostic conversion of a tree-sitter `call_expression` / `new_expression`
// argument list into `Vec<CallArg>`. The TypeScript and JavaScript grammars share
// the node kinds and field names this walks (`arguments`, `arrow_function`,
// `function_expression`, `formal_parameters`, `required_parameter`, `identifier`,
// `string`, `template_string`, the recursive expression shapes), so a single
// implementation serves both extractors.
// =============================================================================

use crate::types::CallArg;
use tree_sitter::Node;

/// Maximum nesting depth for recursive `CallArg` construction. Arguments
/// deeper than this collapse to `CallArg::Other` rather than recursing further.
const MAX_ARG_DEPTH: u32 = 32;

/// Extract text for a node from the raw byte buffer.
fn node_text(node: Node, src: &[u8]) -> String {
    src.get(node.start_byte()..node.end_byte())
        .and_then(|b| std::str::from_utf8(b).ok())
        .unwrap_or("")
        .to_string()
}

/// Extract the positional arguments from a `call_expression`'s `arguments` node.
///
/// Walks named children of the `arguments` list, converting each to a `CallArg`.
/// Handles string literals, template literals with/without interpolation, tagged
/// template bodies, bare identifiers, numeric/boolean literals, and the six
/// recursive expression shapes (ternary, array, await, spread, subscript,
/// binary). Recursion is capped at `MAX_ARG_DEPTH` levels.
pub fn extract_call_args(call_node: &Node, src: &[u8]) -> Vec<CallArg> {
    let Some(args_node) = call_node.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut cursor = args_node.walk();
    for child in args_node.named_children(&mut cursor) {
        result.push(extract_arg(&child, src, 0));
    }
    result
}

/// Convert a single AST expression node to a `CallArg`, recursing for composite
/// expression kinds up to `MAX_ARG_DEPTH`.
fn extract_arg(node: &Node, src: &[u8], depth: u32) -> CallArg {
    if depth >= MAX_ARG_DEPTH {
        return CallArg::Other;
    }
    match node.kind() {
        "string" => {
            // `"text"` or `'text'` — strip surrounding quotes.
            let raw = node_text(*node, src);
            let inner = raw
                .trim_start_matches(['"', '\'', '`'])
                .trim_end_matches(['"', '\'', '`'])
                .to_string();
            CallArg::StringLit(inner)
        }
        "template_string" => {
            // `` `text ${expr} more` `` — check for substitution children.
            let has_substitution = (0..node.child_count()).any(|i| {
                node.child(i)
                    .map(|c| c.kind() == "template_substitution")
                    .unwrap_or(false)
            });
            if has_substitution {
                let raw = node_text(*node, src);
                let replaced = replace_template_substitutions(&raw);
                CallArg::TemplateLit(replaced)
            } else {
                let raw = node_text(*node, src);
                let inner = raw.trim_matches('`').to_string();
                CallArg::StringLit(inner)
            }
        }
        "tagged_template_expression" => {
            // `` gql`query Foo { ... }` `` — capture tag + body.
            let tag_node = node.child_by_field_name("tag");
            let tmpl_node = node.child_by_field_name("template");
            let tag = tag_node.map(|n| node_text(n, src)).unwrap_or_default();
            let body = tmpl_node
                .map(|n| {
                    let raw = node_text(n, src);
                    raw.trim_matches('`').to_string()
                })
                .unwrap_or_default();
            CallArg::TaggedTemplate { tag, body }
        }
        "identifier" => CallArg::Ident(node_text(*node, src)),
        "number" => CallArg::Literal(node_text(*node, src)),
        "true" | "false" | "null" | "undefined" => CallArg::Literal(node.kind().to_string()),
        "object" => {
            let pairs = extract_object_property_pairs(node, src);
            if pairs.is_empty() {
                CallArg::Other
            } else {
                CallArg::ObjectKeys(pairs)
            }
        }
        // Recursive expression shapes — produce structured variants instead of Other.
        "ternary_expression" => {
            // `cond ? consequence : alternative` — the condition's type does not
            // affect the value type; only the two branches are preserved.
            let then_node = node.child_by_field_name("consequence");
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
        "array" => {
            // `[elem0, elem1, ...]` — recurse on each named child. Spread
            // elements inside the array become `CallArg::Spread` children.
            let mut cursor = node.walk();
            let elements = node
                .named_children(&mut cursor)
                .map(|child| extract_arg(&child, src, depth + 1))
                .collect();
            CallArg::ArrayLiteral { elements }
        }
        "await_expression" => {
            // `await expr` — recurse on the awaited expression.
            let inner = node
                .child_by_field_name("value")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Await {
                expr: Box::new(inner),
            }
        }
        "spread_element" => {
            // `...expr` — recurse on the spread operand.
            let inner = node
                .named_child(0)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Spread {
                expr: Box::new(inner),
            }
        }
        "subscript_expression" => {
            // `container[index]` — recurse on both sides.
            let container = node
                .child_by_field_name("object")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let index = node
                .child_by_field_name("index")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::IndexAccess {
                container: Box::new(container),
                index: Box::new(index),
            }
        }
        "binary_expression" => {
            // `left op right` — capture operator text and recurse on operands.
            let op = node
                .child_by_field_name("operator")
                .map(|n| node_text(n, src))
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
        // `x => ...`, `(a, b) => ...`, `function (a) { ... }` — capture the
        // lambda's own parameter names so the chain walker can type them from
        // the higher-order method's callback-parameter signature.
        "arrow_function" | "function_expression" | "function_declaration" | "function" => {
            CallArg::Lambda {
                params: lambda_param_names(node, src),
            }
        }
        _ => CallArg::Other,
    }
}

/// Collect the positional parameter identifier names of an arrow / function
/// argument. Handles the bare single-param arrow (`x => ...`, whose param is a
/// direct `parameter`-field identifier with no `formal_parameters` wrapper) and
/// the parenthesized form (`(a, b) => ...`, params under the `parameters`
/// field). A parameter whose binding is not a plain identifier (destructuring,
/// rest) yields an empty slot so positions stay aligned with the signature.
fn lambda_param_names(node: &Node, src: &[u8]) -> Vec<String> {
    // Bare single-param arrow: `x => ...`. The param is the `parameter` field,
    // an identifier with no `formal_parameters` wrapper.
    if let Some(p) = node.child_by_field_name("parameter") {
        if p.kind() == "identifier" {
            return vec![node_text(p, src)];
        }
    }
    let Some(params) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .map(|param| match param.kind() {
            "required_parameter" | "optional_parameter" => param
                .child_by_field_name("pattern")
                .filter(|n| n.kind() == "identifier")
                .map(|n| node_text(n, src))
                .unwrap_or_default(),
            "identifier" => node_text(param, src),
            _ => String::new(),
        })
        .collect()
}

/// Replace `${...}` spans in a raw template literal text with `{}` placeholders.
pub fn replace_template_substitutions(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut depth = 1usize;
            while let Some(inner) = chars.next() {
                match inner {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            out.push_str("{}");
        } else {
            out.push(c);
        }
    }
    out
}

/// Collect statically-determinable `(key, optional string-literal value)`
/// pairs from an `object` (object literal) node. Recognises shorthand
/// identifiers (`{ foo }`), explicit pairs (`{ foo: handler }` /
/// `{ foo: 'bar' }`), method shorthand (`{ getUser(...) { ... } }`), and
/// string-keyed entries (`{ "foo": ... }`). The value slot is populated
/// only when the value is a plain string literal or a flat template literal
/// without interpolation; identifiers, function expressions, member
/// accesses, and computed values all leave the slot `None`. Computed keys
/// (`{ [dyn]: v }`) and spread elements (`{ ...rest }`) are skipped.
fn extract_object_property_pairs(object_node: &Node, src: &[u8]) -> Vec<(String, Option<String>)> {
    let mut pairs = Vec::new();
    let mut cursor = object_node.walk();
    for child in object_node.named_children(&mut cursor) {
        match child.kind() {
            "pair" => {
                let Some(key_node) = child.child_by_field_name("key") else {
                    continue;
                };
                let name = match key_node.kind() {
                    "property_identifier" | "identifier" => node_text(key_node, src),
                    "string" => node_text(key_node, src)
                        .trim_start_matches(['"', '\'', '`'])
                        .trim_end_matches(['"', '\'', '`'])
                        .to_string(),
                    _ => continue,
                };
                if name.is_empty() {
                    continue;
                }
                let value = child
                    .child_by_field_name("value")
                    .and_then(|v| match v.kind() {
                        "string" => Some(
                            node_text(v, src)
                                .trim_start_matches(['"', '\'', '`'])
                                .trim_end_matches(['"', '\'', '`'])
                                .to_string(),
                        ),
                        "template_string" => {
                            let has_subst = (0..v.child_count()).any(|i| {
                                v.child(i)
                                    .map(|c| c.kind() == "template_substitution")
                                    .unwrap_or(false)
                            });
                            if has_subst {
                                None
                            } else {
                                Some(node_text(v, src).trim_matches('`').to_string())
                            }
                        }
                        _ => None,
                    });
                pairs.push((name, value));
            }
            "shorthand_property_identifier" | "property_identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    pairs.push((name, None));
                }
            }
            "method_definition" => {
                if let Some(key_node) = child.child_by_field_name("name") {
                    let name = node_text(key_node, src);
                    if !name.is_empty() {
                        pairs.push((name, None));
                    }
                }
            }
            _ => {}
        }
    }
    pairs
}
