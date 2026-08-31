// =============================================================================
// csharp/calls.rs  —  Call argument and invocation-body extraction
// =============================================================================

use super::calls_narrowing::{
    extract_is_expression_refs, extract_switch_expression_type_refs,
    extract_type_ref_from_cast_type,
};
use super::helpers::node_text;
use super::types::simple_type_name;
use crate::types::{CallArg, ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

/// Maximum nesting depth for recursive `CallArg` construction. Arguments
/// deeper than this collapse to `CallArg::Other` rather than recursing further.
const MAX_ARG_DEPTH: u32 = 8;

/// First named child of `node` whose kind is `kind`, searched by index so the
/// returned node carries the tree lifetime. A `TreeCursor`-based search would
/// tie the result to a local cursor and fail to outlive it.
fn named_child_of_kind<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut i = 0;
    while let Some(child) = node.named_child(i) {
        if child.kind() == kind {
            return Some(child);
        }
        i += 1;
    }
    None
}

/// `true` when a `generic_name` sits in call position under `parent`: the
/// `name` of a member access or the `function` of an invocation — where it
/// names a generic METHOD, not a type.
pub(super) fn generic_name_in_method_position(parent: &Node, generic_name: &Node) -> bool {
    match parent.kind() {
        "member_access_expression" => parent
            .child_by_field_name("name")
            .map(|n| n.id() == generic_name.id())
            .unwrap_or(false),
        "invocation_expression" => parent
            .child_by_field_name("function")
            .map(|n| n.id() == generic_name.id())
            .unwrap_or(false),
        _ => false,
    }
}

/// Extract positional arguments from a C# `invocation_expression`'s
/// `argument_list`. Each `argument` named child wraps the value
/// expression. Captures string literals, verbatim strings, interpolated
/// strings without interpolation, identifiers, number / bool / null
/// literals, and the recursive expression shapes (ternary, array literal,
/// await, subscript, binary). Recursion is capped at `MAX_ARG_DEPTH`.
pub(super) fn extract_call_args(invocation: &Node, src: &[u8]) -> Vec<CallArg> {
    let Some(args_node) = invocation.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = args_node.walk();
    for child in args_node.named_children(&mut cursor) {
        if child.kind() != "argument" {
            continue;
        }
        // `argument` wraps an expression, preceded by an optional argument
        // name (`name: value`) and/or ref-kind modifier. Take the LAST named
        // child — the value expression — so a named argument captures its
        // value instead of degrading to `Other`.
        let mut inner_cursor = child.walk();
        let Some(expr) = child.named_children(&mut inner_cursor).last() else {
            out.push(CallArg::Other);
            continue;
        };
        out.push(extract_arg(&expr, src, 0));
    }
    out
}

/// Convert a single C# expression node to a `CallArg`, recursing for composite
/// expression kinds up to `MAX_ARG_DEPTH`.
fn extract_arg(node: &Node, src: &[u8], depth: u32) -> CallArg {
    if depth >= MAX_ARG_DEPTH {
        return CallArg::Other;
    }
    match node.kind() {
        "string_literal" | "verbatim_string_literal" | "raw_string_literal" => {
            let raw = node_text(*node, src);
            let stripped = raw
                .trim_start_matches('@')
                .trim_start_matches('$')
                .trim_start_matches(['"', '\''])
                .trim_end_matches(['"', '\''])
                .to_string();
            CallArg::StringLit(stripped)
        }
        "interpolated_string_expression" => {
            // C# interpolated strings: `$"..."` (verbatim: `$@"..."`).
            // Strip the leading `$`, optional `@`, and surrounding
            // quotes; then replace any `{...}` interpolation holes
            // with `{}` placeholders. Pure-literal flat strings
            // become `StringLit`; those with at least one
            // interpolation become `TemplateLit`.
            let raw = node_text(*node, src);
            let inner = raw
                .trim_start_matches('$')
                .trim_start_matches('@')
                .trim_start_matches('$')
                .trim_start_matches(['"', '\''])
                .trim_end_matches(['"', '\''])
                .to_string();
            let has_interp = (0..node.child_count()).any(|i| {
                node.child(i)
                    .map(|c| c.kind() == "interpolation")
                    .unwrap_or(false)
            });
            if has_interp {
                CallArg::TemplateLit(replace_csharp_interpolations(&inner))
            } else {
                CallArg::StringLit(inner)
            }
        }
        "identifier" => CallArg::Ident(node_text(*node, src)),
        "integer_literal" | "real_literal" => CallArg::Literal(node_text(*node, src)),
        "boolean_literal" | "null_literal" => CallArg::Literal(node_text(*node, src)),
        // `cond ? consequence : alternative` — the condition's type does not
        // affect the value type; only the two branches are preserved.
        "conditional_expression" => {
            let then_branch = node
                .child_by_field_name("consequence")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let else_branch = node
                .child_by_field_name("alternative")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Ternary {
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            }
        }
        // `new int[]{ a, b }` / `new[]{ a, b }` — the elements live in the
        // `initializer_expression` child as named `expression` nodes.
        "array_creation_expression" | "implicit_array_creation_expression" => {
            let elements = named_child_of_kind(node, "initializer_expression")
                .map(|init| {
                    let mut cursor = init.walk();
                    init.named_children(&mut cursor)
                        .map(|el| extract_arg(&el, src, depth + 1))
                        .collect()
                })
                .unwrap_or_default();
            CallArg::ArrayLiteral { elements }
        }
        // `await expr` — the awaited expression is the only named child.
        "await_expression" => {
            let inner = node
                .named_child(0)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Await {
                expr: Box::new(inner),
            }
        }
        // `container[index]` — `expression` is the container, `subscript` is a
        // `bracketed_argument_list` whose first `argument` wraps the index.
        "element_access_expression" => {
            let container = node
                .child_by_field_name("expression")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let index = node
                .child_by_field_name("subscript")
                .and_then(|sub| named_child_of_kind(&sub, "argument"))
                .and_then(|arg| arg.named_child(0))
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::IndexAccess {
                container: Box::new(container),
                index: Box::new(index),
            }
        }
        // `left op right` — capture the operator source text and recurse on
        // both operands.
        "binary_expression" => {
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
        // `u => u.Name`, `(x, y) => f(x, y)`, `delegate(int z) { ... }` —
        // capture the lambda's own positional parameter names so the chain
        // walker can type them from the higher-order method's callback-
        // parameter signature.
        "lambda_expression" | "anonymous_method_expression" => CallArg::Lambda {
            params: lambda_param_names(node, src),
        },
        _ => CallArg::Other,
    }
}

/// Collect the positional parameter identifier names of a C# lambda /
/// anonymous-method argument. The `parameters` field is either a single
/// `implicit_parameter` (`u => ...`, whose node text IS the name) or a
/// `parameter_list` of `parameter` nodes carrying a `name` field. A parameter
/// without a plain `name` identifier yields an empty slot so positions stay
/// aligned with the callback signature.
fn lambda_param_names(node: &Node, src: &[u8]) -> Vec<String> {
    let Some(params) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    if params.kind() == "implicit_parameter" {
        return vec![node_text(params, src)];
    }
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter(|p| p.kind() == "parameter")
        .map(|p| {
            p.child_by_field_name("name")
                .map(|n| node_text(n, src))
                .unwrap_or_default()
        })
        .collect()
}

/// Replace `{...}` interpolation holes in a C# interpolated string with
/// `{}` placeholders so the result can be used as a normalized URL pattern.
fn replace_csharp_interpolations(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() != Some(&'{') {
            let mut depth = 1usize;
            for inner in chars.by_ref() {
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

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "invocation_expression" => {
                if let Some(callee) = child.child_by_field_name("function") {
                    let chain = build_chain(callee, src);
                    let name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| callee_name(callee, src));
                    crate::languages::emit_chain_type_ref(
                        &chain,
                        source_symbol_index,
                        &callee,
                        refs,
                    );
                    if !name.is_empty() && !is_csharp_keyword(&name) {
                        let call_args = extract_call_args(&child, src);
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain,
                            byte_offset: callee.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args,
                        });
                    }
                }
                // Recurse into arguments and method chain — calls may be nested.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            "object_creation_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let name = simple_type_name(type_node, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Instantiates,
                            line: type_node.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: type_node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                    }
                    // Also emit TypeRefs for type arguments: `new Dictionary<string, Foo>()`
                    super::types::extract_type_refs_from_type_node(
                        type_node,
                        src,
                        source_symbol_index,
                        refs,
                    );
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `new() { ... }` — target-typed new expression.
            // The type is inferred from context, but we should still extract
            // type refs from any type arguments or initializers.
            "implicit_object_creation_expression" => {
                // In target-typed new, the initializer may contain field/property assignments
                // with types. Extract type refs from initializers.
                let mut cursor2 = child.walk();
                for c in child.children(&mut cursor2) {
                    if matches!(
                        c.kind(),
                        "object_initializer" | "collection_initializer" | "array_initializer"
                    ) {
                        super::types::extract_type_refs_from_type_node(
                            c,
                            src,
                            source_symbol_index,
                            refs,
                        );
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `generic_name` in expression position. The type ARGUMENTS are
            // always type positions. The head is a type only OUTSIDE call
            // position: as an invocation's callee or a member access's `name`
            // (`Method<Foo>()`, `table.Column<string>(…)`) it names a generic
            // METHOD; as a member access's `expression` (`List<int>.Empty`)
            // it names a type.
            "generic_name" => {
                if generic_name_in_method_position(node, &child) {
                    let mut gnc = child.walk();
                    for part in child.children(&mut gnc) {
                        if part.kind() == "type_argument_list" {
                            let mut tac = part.walk();
                            for arg in part.children(&mut tac) {
                                super::types::extract_type_refs_from_type_node(
                                    arg,
                                    src,
                                    source_symbol_index,
                                    refs,
                                );
                            }
                        }
                    }
                } else {
                    super::types::extract_type_refs_from_type_node(
                        child,
                        src,
                        source_symbol_index,
                        refs,
                    );
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `type_argument_list` in expression position — emit TypeRef for each argument.
            "type_argument_list" => {
                let mut cursor2 = child.walk();
                for arg in child.children(&mut cursor2) {
                    super::types::extract_type_refs_from_type_node(
                        arg,
                        src,
                        source_symbol_index,
                        refs,
                    );
                }
            }
            // `user is Admin admin` / `user is Admin` — is_expression or
            // is_pattern_expression (tree-sitter-c-sharp uses both node kinds
            // depending on whether a pattern variable is present).
            "is_expression" | "is_pattern_expression" => {
                extract_is_expression_refs(&child, src, source_symbol_index, refs);
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `user switch { Admin a => a.Level, _ => 0 }` — switch_expression
            // with declaration_pattern or type_pattern arms.
            "switch_expression" => {
                extract_switch_expression_type_refs(&child, src, source_symbol_index, refs);
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `(Admin)user` — cast expression; emit TypeRef for the cast type.
            "cast_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    extract_type_ref_from_cast_type(type_node, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `value as Admin` — as_expression; emit TypeRef for the target type.
            // In tree-sitter-c-sharp the type is the `right` field (left=value, right=type).
            "as_expression" => {
                if let Some(type_node) = child.child_by_field_name("right") {
                    extract_type_ref_from_cast_type(type_node, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `typeof(Admin)` — emit TypeRef for the argument type.
            "typeof_expression" => {
                // tree-sitter-c-sharp: typeof_expression has a single type child
                // (not a named field in all grammar versions — scan children).
                let mut cursor2 = child.walk();
                for c in child.children(&mut cursor2) {
                    if !matches!(c.kind(), "typeof" | "(" | ")") {
                        extract_type_ref_from_cast_type(c, src, source_symbol_index, refs);
                        break;
                    }
                }
            }
            // `nameof(Symbol)` — emit a Calls-like ref for the named symbol.
            "nameof_expression" => {
                let mut cursor2 = child.walk();
                for c in child.children(&mut cursor2) {
                    if !matches!(c.kind(), "nameof" | "(" | ")") {
                        let name = match c.kind() {
                            "identifier" => node_text(c, src),
                            "member_access_expression" => c
                                .child_by_field_name("name")
                                .map(|n| node_text(n, src))
                                .unwrap_or_else(|| {
                                    let t = node_text(c, src);
                                    t.rsplit('.').next().unwrap_or(&t).to_string()
                                }),
                            _ => {
                                let t = node_text(c, src);
                                t.rsplit('.').next().unwrap_or(&t).to_string()
                            }
                        };
                        if !name.is_empty() && !is_csharp_keyword(&name) {
                            refs.push(ExtractedRef {
                                is_include: false,
                                is_import_binding: false,
                                is_reexport: false,
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: c.start_position().row as u32,
                                col: 0,
                                module: None,
                                chain: None,
                                byte_offset: c.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
                            });
                        }
                        break;
                    }
                }
            }
            // `foreach (TypeName item in collection)` — TypeRef for explicit type,
            // plus recurse into the body.
            // Note: tree-sitter-c-sharp uses "foreach_statement" (not "for_each_statement").
            "foreach_statement" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    extract_type_ref_from_cast_type(type_node, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `catch (ExceptionType e)` — TypeRef for the exception type.
            // tree-sitter-c-sharp: catch_clause has an unnamed catch_declaration child
            // (it is not a named field).  Scan children for catch_declaration.
            "catch_clause" => {
                let mut catch_cursor = child.walk();
                for catch_child in child.children(&mut catch_cursor) {
                    if catch_child.kind() == "catch_declaration" {
                        if let Some(type_node) = catch_child.child_by_field_name("type") {
                            extract_type_ref_from_cast_type(
                                type_node,
                                src,
                                source_symbol_index,
                                refs,
                            );
                        }
                        break;
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `using (var x = new Resource())` — recurse; TypeRef extracted from new expr.
            "using_statement" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // Local function statement inside a method body — emit Function symbol
            // and extract calls from its body.
            "local_function_statement" => {
                // Calls within the local function are attributed to the enclosing method.
                if let Some(body) = child.child_by_field_name("body") {
                    extract_calls_from_body(&body, src, source_symbol_index, refs);
                }
            }
            // `x => x.Name` — lambda expression body (single expression form).
            // Also handle `(x, y) => Compute(x, y)` — lambda body may contain calls.
            "lambda_expression" => {
                // Recurse into lambda body to find nested calls.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `condition ? trueExpr : falseExpr` — ternary expression.
            // Both branches may contain method calls.
            "conditional_expression" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `obj?.Property?.Method()` — null-conditional chain.
            // Recurse to find all calls in the chain.
            "null_conditional_member_access_expression"
            | "null_conditional_invocation_expression" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // String-literal text is opaque — do not descend. Tag/markup text
            // inside a verbatim or raw string would otherwise leak as Calls.
            "string_literal" | "verbatim_string_literal" | "raw_string_literal" => {}
            // `$"...{expr}..."` — only the `interpolation` holes are code;
            // the surrounding literal text is opaque. Descend solely into
            // interpolation children.
            "interpolated_string_expression" => {
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if inner.kind() == "interpolation" {
                        extract_calls_from_body(&inner, src, source_symbol_index, refs);
                    }
                }
            }
            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared predicates — keyword filter and callee-name extraction
// ---------------------------------------------------------------------------

/// C# keywords/operators that look like method calls but aren't.
pub(super) fn is_csharp_keyword(name: &str) -> bool {
    matches!(
        name,
        "nameof"
            | "typeof"
            | "sizeof"
            | "default"
            | "checked"
            | "unchecked"
            | "stackalloc"
            | "await"
            | "throw"
            | "yield"
            | "var"
            | "is"
            | "as"
            | "new"
            | "this"
            | "base"
            | "null"
            | "true"
            | "false"
            | "value"
    )
}

fn callee_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "member_access_expression" => node
            .child_by_field_name("name")
            .map(|n| node_text(n, src))
            .unwrap_or_else(|| {
                let t = node_text(node, src);
                t.rsplit('.').next().unwrap_or(&t).to_string()
            }),
        "generic_name" => {
            // Generic method call like `GetService<T>()` — extract just the name.
            let children: Vec<Node> = {
                let mut cursor = node.walk();
                node.children(&mut cursor).collect()
            };
            children
                .iter()
                .find(|c| c.kind() == "identifier")
                .map(|n| node_text(*n, src))
                .unwrap_or_default()
        }
        _ => {
            let t = node_text(node, src);
            t.rsplit('.').next().unwrap_or(&t).to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// MemberChain building
// ---------------------------------------------------------------------------

/// Build a structured member access chain from tree-sitter AST nodes.
///
/// Recursively walks nested `member_access_expression` nodes to produce
/// a `Vec<ChainSegment>` from root to leaf.
///
/// `this.repo.FindOne()` tree structure:
/// ```text
/// invocation_expression
///   function: member_access_expression
///     expression: member_access_expression
///       expression: this_expression "this"
///       name: identifier "repo"
///     name: identifier "FindOne"
/// ```
/// produces: `[this, repo, FindOne]`
pub(super) fn build_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "this_expression" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this_expression".to_string(),
                kind: SegmentKind::SelfRef,
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

        "base_expression" => {
            segments.push(ChainSegment {
                name: "base".to_string(),
                node_kind: "base_expression".to_string(),
                kind: SegmentKind::SelfRef,
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

        // `string.IsNullOrWhiteSpace(…)` — a static-member call on a
        // predefined-type keyword. The segment carries the aliased BCL type
        // as its declared type so the chain roots on it directly.
        "predefined_type" => {
            let kw = node_text(node, src);
            let bcl = super::keywords::bcl_type_for_keyword(&kw)?;
            segments.push(ChainSegment {
                name: kw,
                node_kind: "predefined_type".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: Some(bcl.to_string()),
                type_args: vec![],
                optional_chaining: false,
                byte_offset: node.start_byte() as u32,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }

        "identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: "identifier".to_string(),
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

        "generic_name" => {
            // `GetService<T>` — strip the generic args, keep just the identifier.
            let name = {
                let mut cursor = node.walk();
                let children: Vec<Node> = node.children(&mut cursor).collect();
                drop(cursor);
                children
                    .iter()
                    .find(|c| c.kind() == "identifier")
                    .map(|c| node_text(*c, src))
                    .unwrap_or_else(|| node_text(node, src))
            };
            segments.push(ChainSegment {
                name,
                node_kind: "generic_name".to_string(),
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
            let expr = node.child_by_field_name("expression")?;
            let name_node = node.child_by_field_name("name")?;

            // Recurse into the expression (receiver) to build the prefix chain.
            build_chain_inner(expr, src, segments)?;

            // The name may be a generic_name (e.g., `Foo<T>`) — extract identifier.
            let name = if name_node.kind() == "generic_name" {
                let mut cursor = name_node.walk();
                let children: Vec<Node> = name_node.children(&mut cursor).collect();
                drop(cursor);
                children
                    .iter()
                    .find(|c| c.kind() == "identifier")
                    .map(|c| node_text(*c, src))
                    .unwrap_or_else(|| node_text(name_node, src))
            } else {
                node_text(name_node, src)
            };

            segments.push(ChainSegment {
                name,
                node_kind: name_node.kind().to_string(),
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

        "conditional_access_expression" => {
            // C# `?.` operator: `foo?.Bar()`
            let expr = node.child_by_field_name("expression")?;
            let binding = node.child_by_field_name("binding")?;

            build_chain_inner(expr, src, segments)?;

            // The binding is a `member_binding_expression` with a `name` field.
            let name_node = binding.child_by_field_name("name").unwrap_or(binding);
            segments.push(ChainSegment {
                name: node_text(name_node, src),
                node_kind: binding.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: true,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }

        "invocation_expression" => {
            // Nested call in a chain: `a.B().C()` — walk into the function child,
            // then mark the invoked segment so the walker yields the function's
            // return type rather than the function value.
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, src, segments)?;
            if let Some(last) = segments.last_mut() {
                last.is_call = true;
            }
            Some(())
        }

        // `((Admin)x).Ban()` / `(x as Admin).Ban()` — the cast asserts the
        // type; the chain walker adopts the inner segment's `declared_type`.
        "cast_expression" => {
            let value = node.child_by_field_name("value")?;
            build_chain_inner(value, src, segments)?;
            if let Some(type_node) = node.child_by_field_name("type") {
                if let Some(last) = segments.last_mut() {
                    if last.declared_type.is_none() {
                        last.declared_type = Some(node_text(type_node, src));
                    }
                }
            }
            Some(())
        }
        "as_expression" => {
            let value = node.child_by_field_name("left")?;
            build_chain_inner(value, src, segments)?;
            if let Some(type_node) = node.child_by_field_name("right") {
                if let Some(last) = segments.last_mut() {
                    if last.declared_type.is_none() {
                        last.declared_type = Some(node_text(type_node, src));
                    }
                }
            }
            Some(())
        }

        // Unknown node — can't build a chain.
        _ => None,
    }
}
