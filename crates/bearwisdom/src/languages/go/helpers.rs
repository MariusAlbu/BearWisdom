// =============================================================================
// go/helpers.rs  —  Shared utilities for the Go extractor
// =============================================================================

use crate::types::Visibility;
use tree_sitter::Node;

pub(super) fn node_text(node: &Node, source: &str) -> String {
    source[node.start_byte()..node.end_byte()].to_string()
}

pub(super) fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

pub(super) fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_string())
    }
}

/// Go visibility: exported names start with a Unicode uppercase letter.
pub(super) fn go_visibility(name: &str) -> Option<Visibility> {
    match name.chars().next() {
        Some(c) if c.is_uppercase() => Some(Visibility::Public),
        Some(_) => Some(Visibility::Private),
        None => None,
    }
}

/// Test functions match `TestXxx`, `BenchmarkXxx`, `ExampleXxx`.
pub(super) fn is_test_function(name: &str) -> bool {
    name.starts_with("Test") || name.starts_with("Benchmark") || name.starts_with("Example")
}

/// Collect consecutive `// ...` line-comment nodes that are unbroken previous
/// siblings of this node and return them as a doc comment string.
pub(super) fn extract_go_doc_comment(node: &Node, source: &str) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();

    let mut current = node.prev_sibling();
    while let Some(sib) = current {
        match sib.kind() {
            "comment" => {
                let text = node_text(&sib, source);
                if text.starts_with("//") {
                    lines.push(text);
                    current = sib.prev_sibling();
                } else {
                    break;
                }
            }
            _ => break,
        }
    }

    if lines.is_empty() {
        return None;
    }

    lines.reverse();
    Some(lines.join("\n"))
}

/// Build a signature from the first line of the declaration, trimming the
/// opening `{` so it reads as a clean signature.
pub(super) fn build_fn_signature_from_source(node: &Node, source: &str) -> Option<String> {
    let text = node_text(node, source);
    let first_line = text.lines().next()?;
    let sig = first_line
        .trim_end_matches('{')
        .trim_end()
        .to_string();
    if sig.is_empty() { None } else { Some(sig) }
}

/// Build a signature for a `method_elem` from its source.
///
/// Form: `MethodName(params) result`
pub(super) fn build_method_elem_signature(node: &Node, source: &str) -> Option<String> {
    let text = node_text(node, source);
    if text.is_empty() { None } else { Some(text) }
}

/// Extract the base type name from a `pointer_type` node (`*Foo` → `"Foo"`).
pub(super) fn pointer_type_name(node: &Node, source: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" {
            return node_text(&child, source);
        }
        if child.kind() == "pointer_type" {
            // Handle `**Foo`
            return pointer_type_name(&child, source);
        }
    }
    // Fallback: strip leading `*` from raw text.
    node_text(node, source).trim_start_matches('*').to_string()
}

/// Return true for Go builtin types that don't reference user symbols.
pub(super) fn is_go_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "bool" | "byte" | "complex64" | "complex128" | "error"
            | "float32" | "float64"
            | "int" | "int8" | "int16" | "int32" | "int64"
            | "rune" | "string" | "uint" | "uint8" | "uint16"
            | "uint32" | "uint64" | "uintptr"
            | "any" | "comparable"
    )
}

/// Extract a simple type name from a Go type node for TypeRef emission.
///
/// Handles:
/// - `type_identifier`            → `"Foo"`
/// - `pointer_type`               → `"Foo"` (strips `*`)
/// - `qualified_type`             → `"Foo"` (last segment of `pkg.Foo`)
/// - `slice_type`                 → recursively extracts element type
/// - `map_type`                   → recursively extracts value type
/// - `array_type`                 → `[N]Foo` → extracts element type
/// - `channel_type`               → `chan Foo` → extracts element type
/// - `generic_type`               → `List[T]` → base type name (Go 1.18+)
pub(super) fn extract_go_type_name(node: &Node, source: &str) -> String {
    match node.kind() {
        "type_identifier" => node_text(node, source),
        "pointer_type" => {
            // Find the inner type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    return extract_go_type_name(&child, source);
                }
            }
            String::new()
        }
        "qualified_type" => {
            // `pkg.Type` — take the last segment.
            let text = node_text(node, source);
            text.rsplit('.').next().unwrap_or(&text).to_string()
        }
        "slice_type" => {
            // `[]Foo` — extract element type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    return extract_go_type_name(&child, source);
                }
            }
            String::new()
        }
        "map_type" => {
            // `map[K]V` — extract value type (second named child).
            let named: Vec<_> = {
                let mut cursor = node.walk();
                node.children(&mut cursor).filter(|c| c.is_named()).collect()
            };
            if named.len() >= 2 {
                return extract_go_type_name(&named[1], source);
            }
            String::new()
        }
        "array_type" => {
            // `[N]Foo` — extract element type (last named child after the length).
            // tree-sitter-go: array_type { length: _, element: _type }
            if let Some(elem) = node.child_by_field_name("element") {
                return extract_go_type_name(&elem, source);
            }
            // Fallback: last named child.
            let named: Vec<_> = {
                let mut cursor = node.walk();
                node.children(&mut cursor).filter(|c| c.is_named()).collect()
            };
            if let Some(last) = named.last() {
                return extract_go_type_name(last, source);
            }
            String::new()
        }
        "channel_type" => {
            // `chan Foo` / `<-chan Foo` — extract element type (last named child).
            let mut cursor = node.walk();
            let mut last_name = String::new();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    let name = extract_go_type_name(&child, source);
                    if !name.is_empty() {
                        last_name = name;
                    }
                }
            }
            last_name
        }
        "generic_type" => {
            // `List[int]` (Go 1.18+) — extract base type name.
            // tree-sitter-go generic_type: type_identifier, type_arguments
            if let Some(base) = node.child_by_field_name("name") {
                return node_text(&base, source);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" {
                    return node_text(&child, source);
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Emit TypeRef edges for all non-builtin named types referenced inside a
/// `function_type` node (`func(A, B) C`).
///
/// This covers both parameter types and the return type.
pub(super) fn extract_function_type_refs(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<crate::types::ExtractedRef>,
) {
    use crate::types::EdgeKind;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "parameter_list" | "result" => {
                let mut inner = child.walk();
                for param_child in child.children(&mut inner) {
                    if !param_child.is_named() {
                        continue;
                    }
                    match param_child.kind() {
                        "parameter_declaration" | "variadic_parameter_declaration" => {
                            // The type is the last named child that isn't an identifier.
                            let type_node = (0..param_child.child_count())
                                .filter_map(|i| param_child.child(i))
                                .filter(|c| c.is_named() && c.kind() != "identifier")
                                .last();
                            if let Some(tn) = type_node {
                                let name = extract_go_type_name(&tn, source);
                                if !name.is_empty() && !is_go_builtin_type(&name) {
                                    refs.push(crate::types::ExtractedRef {
                                        source_symbol_index,
                                        target_name: name,
                                        kind: EdgeKind::TypeRef,
                                        line: tn.start_position().row as u32,
                                        module: None,
                                        chain: None,
                                        byte_offset: 0,
                                                                            namespace_segments: Vec::new(),
});
                                }
                            }
                        }
                        _ => {
                            // Bare type in result or single-type result.
                            let name = extract_go_type_name(&param_child, source);
                            if !name.is_empty() && !is_go_builtin_type(&name) {
                                refs.push(crate::types::ExtractedRef {
                                    source_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: param_child.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
