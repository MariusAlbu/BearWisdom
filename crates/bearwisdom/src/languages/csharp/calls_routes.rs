// =============================================================================
// csharp/calls_routes.rs  —  ASP.NET attribute routes and Minimal API routes
// =============================================================================

use super::helpers::node_text;
use crate::types::ExtractedRoute;
use std::collections::HashMap;
use tree_sitter::Node;

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
                            let method_template =
                                attr_route_template(&attr, src).unwrap_or_else(|| String::from(""));
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
                        if let Some(content) = children
                            .iter()
                            .find(|c| c.kind() == "string_literal_content")
                        {
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
        return if action.is_empty() {
            "/".to_string()
        } else {
            action.to_string()
        };
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
        if child.kind() == "local_declaration_statement" || child.kind() == "variable_declaration" {
            collect_mapgroup_assignments(&child, src, prefixes);
            continue;
        }

        if child.kind() == "variable_declarator" {
            let var_name = child.child_by_field_name("name").map(|n| node_text(n, src));

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
                        if let Some(content) = children
                            .iter()
                            .find(|c| c.kind() == "string_literal_content")
                        {
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
