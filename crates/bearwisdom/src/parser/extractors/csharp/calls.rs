// =============================================================================
// csharp/calls.rs  —  Call, route, and member-chain extraction
// =============================================================================

use super::helpers::node_text;
use super::types::simple_type_name;
use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedRoute, MemberChain, SegmentKind,
};
use std::collections::HashMap;
use tree_sitter::Node;

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
                    if !name.is_empty() && !is_csharp_keyword(&name) {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            "object_creation_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let name = simple_type_name(type_node, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Instantiates,
                            line: type_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// C# keywords/operators that look like method calls but aren't.
pub(super) fn is_csharp_keyword(name: &str) -> bool {
    matches!(
        name,
        "nameof" | "typeof" | "sizeof" | "default" | "checked" | "unchecked"
        | "stackalloc" | "await" | "throw" | "yield" | "var" | "is" | "as"
        | "new" | "this" | "base" | "null" | "true" | "false" | "value"
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
            });
            Some(())
        }

        "invocation_expression" => {
            // Nested call in a chain: `a.B().C()` — the expression is an invocation.
            // Walk into the function child to continue the chain.
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, src, segments)
        }

        // Unknown node — can't build a chain.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// HTTP Route extraction
// ---------------------------------------------------------------------------

/// Extract the class-level `[Route("...")]` attribute value for ASP.NET controllers.
///
/// Example: `[Route("api/categories")]` → `Some("api/categories")`
pub(super) fn extract_class_route_prefix(class_node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = class_node.walk();
    for child in class_node.children(&mut cursor) {
        if child.kind() == "attribute_list" {
            let mut al_cursor = child.walk();
            for attr in child.children(&mut al_cursor) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        let name = node_text(name_node, src);
                        if name == "Route" {
                            return attr_route_template(&attr, src);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Attribute-based route extraction with optional class-level prefix.
pub(super) fn extract_attribute_routes_with_prefix(
    node: &Node,
    src: &[u8],
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
    class_prefix: Option<&str>,
) {
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        if child.kind() == "attribute_list" {
            let mut al_cursor = child.walk();
            for attr in child.children(&mut al_cursor) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        let attr_name = node_text(name_node, src);
                        if let Some(method) = http_method_from_attribute(&attr_name) {
                            let method_template = attr_route_template(&attr, src)
                                .unwrap_or_else(|| String::from(""));
                            // Combine class prefix with method template.
                            let template = match class_prefix {
                                Some(prefix) if !prefix.is_empty() => {
                                    let p = prefix.trim_matches('/');
                                    let m = method_template.trim_matches('/');
                                    if m.is_empty() {
                                        format!("/{p}")
                                    } else {
                                        format!("/{p}/{m}")
                                    }
                                }
                                _ => {
                                    if method_template.is_empty() {
                                        "/".to_string()
                                    } else {
                                        method_template
                                    }
                                }
                            };
                            routes.push(ExtractedRoute {
                                handler_symbol_index,
                                http_method: method.to_string(),
                                template,
                            });
                        }
                    }
                }
            }
        }
    }
}

pub(super) fn http_method_from_attribute(name: &str) -> Option<&'static str> {
    // Strip generic suffix if present: `HttpGet<T>` → `HttpGet`
    let base = name.split('<').next().unwrap_or(name);
    match base {
        "HttpGet" | "MapGet" => Some("GET"),
        "HttpPost" | "MapPost" => Some("POST"),
        "HttpPut" | "MapPut" => Some("PUT"),
        "HttpDelete" | "MapDelete" => Some("DELETE"),
        "HttpPatch" | "MapPatch" => Some("PATCH"),
        "Route" => Some("ANY"),
        _ => None,
    }
}

pub(super) fn attr_route_template(attr_node: &Node, src: &[u8]) -> Option<String> {
    use super::helpers::find_child_kind;
    // In tree-sitter-c-sharp the attribute argument list is a child NODE of kind
    // `attribute_argument_list` — it is NOT a named field, so child_by_field_name
    // will always return None.  We must find it by kind.
    //
    // Structure:
    //   attribute
    //     identifier              ← name (this IS a named field)
    //     attribute_argument_list ← kind (NOT a named field)
    //       (
    //       attribute_argument
    //         string_literal
    //           string_literal_content  ← raw text, no quotes
    //       )
    let arg_list = find_child_kind(attr_node, "attribute_argument_list")?;
    let mut cursor = arg_list.walk();
    for arg in arg_list.children(&mut cursor) {
        if arg.kind() == "attribute_argument" {
            let mut ac = arg.walk();
            for child in arg.children(&mut ac) {
                match child.kind() {
                    "string_literal" => {
                        // Prefer string_literal_content (the text without surrounding quotes).
                        let children: Vec<Node> = {
                            let mut sc = child.walk();
                            child.children(&mut sc).collect()
                        };
                        if let Some(content) = children.iter().find(|c| c.kind() == "string_literal_content") {
                            return Some(node_text(*content, src));
                        }
                        // Fallback: strip quotes from the whole string_literal text.
                        let raw = node_text(child, src);
                        return Some(raw.trim_matches('"').to_string());
                    }
                    "verbatim_string_literal" => {
                        let raw = node_text(child, src);
                        let stripped = raw.trim_start_matches('@').trim_matches('"');
                        return Some(stripped.to_string());
                    }
                    "interpolated_string_expression" => {
                        return Some("{dynamic}".to_string());
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

/// Combine a route prefix with a route template.
///
/// Examples:
///   ("api/auth", "login")       → "api/auth/login"
///   ("api/auth", "/")           → "api/auth"
///   ("", "login")               → "login"
///   ("api/catalog", "{id:int}") → "api/catalog/{id:int}"
pub(super) fn combine_route_prefix(prefix: &str, action: &str) -> String {
    let prefix = prefix.trim_matches('/');
    let action = action.trim_matches('/');

    if prefix.is_empty() {
        return if action.is_empty() { "/".to_string() } else { action.to_string() };
    }
    if action.is_empty() {
        return prefix.to_string();
    }
    format!("{prefix}/{action}")
}

/// Minimal-API route registration inside method bodies:
///   `app.MapGet("/api/items", ...)` etc.
///
/// Also resolves `MapGroup` prefixes:
///   `var api = app.MapGroup("api/orders"); api.MapGet("/", handler);`
///   → route template becomes `"api/orders"` instead of `"/"`.
pub(super) fn extract_minimal_api_routes(
    body: &Node,
    src: &[u8],
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
) {
    let group_prefixes = build_mapgroup_prefixes(body, src);
    extract_minimal_api_routes_inner(body, src, handler_symbol_index, routes, &group_prefixes);
}

/// Build a map of variable names to their accumulated MapGroup prefix.
fn build_mapgroup_prefixes<'a>(body: &Node<'a>, src: &[u8]) -> HashMap<String, String> {
    let mut prefixes: HashMap<String, String> = HashMap::new();
    collect_mapgroup_assignments(body, src, &mut prefixes);
    prefixes
}

/// Recursively walk a block collecting `var X = expr.MapGroup("prefix")` assignments.
fn collect_mapgroup_assignments(node: &Node, src: &[u8], prefixes: &mut HashMap<String, String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "local_declaration_statement"
            || child.kind() == "variable_declaration"
        {
            collect_mapgroup_assignments(&child, src, prefixes);
            continue;
        }

        if child.kind() == "variable_declarator" {
            let var_name = child
                .child_by_field_name("name")
                .map(|n| node_text(n, src));

            // The initializer is a direct child of variable_declarator after `=`.
            let mut found_eq = false;
            let mut init_expr: Option<Node> = None;
            let mut vc = child.walk();
            for vchild in child.children(&mut vc) {
                if vchild.kind() == "=" {
                    found_eq = true;
                } else if found_eq && vchild.kind() == "invocation_expression" {
                    init_expr = Some(vchild);
                    break;
                }
            }

            if let (Some(var_name), Some(init)) = (var_name, init_expr) {
                if let Some(prefix) = resolve_mapgroup_chain(&init, src, prefixes) {
                    prefixes.insert(var_name, prefix);
                }
            }
            continue;
        }

        collect_mapgroup_assignments(&child, src, prefixes);
    }
}

/// Resolve the group prefix from a (possibly chained) expression.
fn resolve_mapgroup_chain(
    node: &Node,
    src: &[u8],
    prefixes: &HashMap<String, String>,
) -> Option<String> {
    if node.kind() != "invocation_expression" {
        return None;
    }

    let func_node = node.child_by_field_name("function")?;

    if func_node.kind() == "member_access_expression" {
        let method_name = node_text(func_node.child_by_field_name("name")?, src);
        let object = func_node.child_by_field_name("expression")?;

        if method_name == "MapGroup" {
            let arg_list = node.child_by_field_name("arguments")?;
            let group_path = first_string_arg(&arg_list, src)?;
            let receiver_prefix = resolve_receiver_prefix(&object, src, prefixes);

            return Some(combine_route_prefix(
                &receiver_prefix.unwrap_or_default(),
                &group_path,
            ));
        }

        // Fluent chain: `.HasApiVersion(...)`, etc. — recurse into the object.
        return resolve_mapgroup_chain(&object, src, prefixes);
    }

    None
}

/// Get the accumulated prefix for a receiver expression.
fn resolve_receiver_prefix(
    object: &Node,
    src: &[u8],
    prefixes: &HashMap<String, String>,
) -> Option<String> {
    match object.kind() {
        "identifier" => {
            let name = node_text(*object, src);
            prefixes.get(&name).cloned()
        }
        "invocation_expression" => resolve_mapgroup_chain(object, src, prefixes),
        _ => None,
    }
}

/// Get the variable name from the receiver of a member_access_expression.
fn get_receiver_name(func_node: &Node, src: &[u8]) -> Option<String> {
    let object = func_node.child_by_field_name("expression")?;
    if object.kind() == "identifier" {
        Some(node_text(object, src))
    } else {
        None
    }
}

/// Inner recursive route extractor with group prefix support.
fn extract_minimal_api_routes_inner(
    body: &Node,
    src: &[u8],
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
    group_prefixes: &HashMap<String, String>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "invocation_expression" {
            if let Some(func_node) = child.child_by_field_name("function") {
                if func_node.kind() == "member_access_expression" {
                    if let Some(method_name_node) = func_node.child_by_field_name("name") {
                        let method_name = node_text(method_name_node, src);
                        if let Some(http_method) = http_method_from_attribute(&method_name) {
                            if let Some(arg_list) = child.child_by_field_name("arguments") {
                                if let Some(template) = first_string_arg(&arg_list, src) {
                                    let prefix = get_receiver_name(&func_node, src)
                                        .and_then(|name| group_prefixes.get(&name).cloned())
                                        .unwrap_or_default();

                                    let full_template = if prefix.is_empty() {
                                        template
                                    } else {
                                        combine_route_prefix(&prefix, &template)
                                    };

                                    routes.push(ExtractedRoute {
                                        handler_symbol_index,
                                        http_method: http_method.to_string(),
                                        template: full_template,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        extract_minimal_api_routes_inner(&child, src, handler_symbol_index, routes, group_prefixes);
    }
}

pub(super) fn first_string_arg(arg_list: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = arg_list.walk();
    for arg in arg_list.children(&mut cursor) {
        if arg.kind() == "argument" {
            let mut ac = arg.walk();
            for child in arg.children(&mut ac) {
                match child.kind() {
                    "string_literal" => {
                        // Prefer the `string_literal_content` child (no surrounding quotes).
                        let children: Vec<Node> = {
                            let mut sc = child.walk();
                            child.children(&mut sc).collect()
                        };
                        if let Some(content) = children.iter().find(|c| c.kind() == "string_literal_content") {
                            return Some(node_text(*content, src));
                        }
                        let raw = node_text(child, src);
                        return Some(raw.trim_matches('"').to_string());
                    }
                    "verbatim_string_literal" => {
                        let raw = node_text(child, src);
                        return Some(raw.trim_start_matches('@').trim_matches('"').to_string());
                    }
                    _ => {}
                }
            }
        }
    }
    None
}
