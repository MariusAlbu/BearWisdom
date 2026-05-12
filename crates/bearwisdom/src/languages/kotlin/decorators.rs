// =============================================================================
// kotlin/decorators.rs  —  Annotation extraction for Kotlin
//
// Kotlin annotation forms:
//   @Service                         → single `annotation` node
//   @GetMapping("/users")            → annotation → constructor_invocation
//   @Inject                          → annotation node inside modifiers
//
// Tree-sitter shapes (kotlin-ng 1.1):
//   annotation
//     type | constructor_invocation
//
// Annotations live inside a `modifiers` child of class/function/property
// declarations, or appear as standalone sibling nodes at file scope
// (file_annotation). We extract from both locations.
//
// Pattern matching (when expressions):
//   when_expression → when_entry (condition: type_test | range_test | expression)
//   type_test → type
//
// Lambda params:
//   lambda_literal → lambda_parameters → variable_declaration → identifier
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Decorators
// ---------------------------------------------------------------------------

/// Emit one `ExtractedRef` per annotation attached to `node`.
///
/// `node` should be a declaration node: `class_declaration`,
/// `function_declaration`, `property_declaration`, etc.
/// In Kotlin-ng annotations are children of `modifiers`.
pub(super) fn extract_decorators(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "modifiers" => {
                let mut mc = child.walk();
                for ann in child.children(&mut mc) {
                    emit_annotation(&ann, src, source_symbol_index, refs);
                }
            }
            // Direct annotation at file scope or as explicit child.
            "annotation" | "file_annotation" => {
                emit_annotation(&child, src, source_symbol_index, refs);
            }
            _ => {}
        }
    }
}

fn emit_annotation(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() != "annotation" && node.kind() != "file_annotation" {
        return;
    }
    if let Some(name) = annotation_name(node, src) {
        let first_arg = extract_first_string_arg(node, src);
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: first_arg,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
});
    }
}

/// Public variant for use from calls.rs (annotations inside function bodies).
pub(super) fn emit_annotation_ref_pub(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    emit_annotation(node, src, source_symbol_index, refs);
}

/// Public accessor to extract an annotation name — used by scan_all_type_refs
/// in extract.rs to ensure every annotation node in the tree emits a TypeRef.
pub(super) fn annotation_name_pub(node: &Node, src: &[u8]) -> Option<String> {
    annotation_name(node, src)
}

/// Extract the annotation name.
///
/// Kotlin annotation shape:
///   annotation → type | constructor_invocation
///   type → user_type → simple_user_type → simple_identifier
///   constructor_invocation → user_type → ...
fn annotation_name(node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Actual shape in kotlin-ng 1.1: annotation → "@" + user_type
            "user_type" => {
                if let Some(name) = name_from_user_type(&child, src) {
                    return Some(name);
                }
            }
            // @Foo (marker style) — direct type child (grammar-version fallback)
            "type" => {
                if let Some(name) = name_from_type_node(&child, src) {
                    return Some(name);
                }
            }
            // @GetMapping("/path") — constructor invocation
            "constructor_invocation" => {
                let mut cc = child.walk();
                for inner in child.children(&mut cc) {
                    if inner.kind() == "user_type" {
                        if let Some(name) = name_from_user_type(&inner, src) {
                            return Some(name);
                        }
                    }
                }
            }
            // Sometimes the identifier appears directly.
            "simple_identifier" | "identifier" => {
                let t = node_text(child, src);
                if !t.is_empty() {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

fn name_from_type_node(node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "user_type" {
            return name_from_user_type(&child, src);
        }
        if child.kind() == "simple_identifier" || child.kind() == "identifier" {
            return Some(node_text(child, src));
        }
    }
    None
}

fn name_from_user_type(node: &Node, src: &[u8]) -> Option<String> {
    // user_type → simple_user_type+
    // simple_user_type → name (simple_identifier) + optional type_arguments
    let mut last: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "simple_user_type" {
            if let Some(name_node) = child.child_by_field_name("name") {
                last = Some(node_text(name_node, src));
            } else {
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if inner.kind() == "simple_identifier" || inner.kind() == "identifier" {
                        last = Some(node_text(inner, src));
                        break;
                    }
                }
            }
        }
        if child.kind() == "simple_identifier" || child.kind() == "identifier" {
            last = Some(node_text(child, src));
        }
    }
    last
}

fn extract_first_string_arg(annotation_node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = annotation_node.walk();
    for child in annotation_node.children(&mut cursor) {
        if child.kind() == "constructor_invocation" {
            // Look for value_arguments → string
            let mut cc = child.walk();
            for inner in child.children(&mut cc) {
                if inner.kind() == "value_arguments" {
                    return first_string_in_args(&inner, src);
                }
            }
        }
    }
    None
}

fn first_string_in_args(args_node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = args_node.walk();
    for child in args_node.children(&mut cursor) {
        if child.kind() == "value_argument" {
            let mut ac = child.walk();
            for inner in child.children(&mut ac) {
                if inner.kind() == "string_literal" || inner.kind() == "multiline_string_literal" {
                    return strip_string(node_text(inner, src));
                }
            }
        }
        if child.kind() == "string_literal" || child.kind() == "multiline_string_literal" {
            return strip_string(node_text(child, src));
        }
    }
    None
}

fn strip_string(raw: String) -> Option<String> {
    let s = raw.trim_matches('"').trim_matches('\'').to_string();
    if s.is_empty() { None } else { Some(s) }
}

// ---------------------------------------------------------------------------
// When expression — pattern matching
// ---------------------------------------------------------------------------

/// Extract TypeRef edges from `is` checks inside `when_expression`.
///
/// ```kotlin
/// when (user) {
///     is Admin -> user.level
///     is Student -> user.grade
/// }
/// ```
///
/// Tree-sitter: when_expression → when_entry (condition: type_test → type)
pub(super) fn extract_when_patterns(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "when_entry" {
            let mut ec = child.walk();
            for item in child.children(&mut ec) {
                extract_when_entry_condition(&item, src, source_symbol_index, refs);
            }
        }
    }
}

fn extract_when_entry_condition(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "type_test" => {
            // type_test → is + user_type (actual kotlin-ng 1.1 shape)
            // Also handles type_test → type (grammar spec fallback)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "user_type" => {
                        if let Some(name) = name_from_user_type(&child, src) {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                                            namespace_segments: Vec::new(),
                                                            call_args: Vec::new(),
});
                        }
                    }
                    "type" => {
                        if let Some(name) = name_from_type_node(&child, src) {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                                            namespace_segments: Vec::new(),
                                                            call_args: Vec::new(),
});
                        }
                    }
                    _ => {}
                }
            }
        }
        // Recurse into nested expressions.
        "when_condition" | "condition" | "when_entry" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_when_entry_condition(&child, src, source_symbol_index, refs);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Lambda parameters
// ---------------------------------------------------------------------------

/// Extract `Variable` symbols for parameters declared in lambda literals.
///
/// ```kotlin
/// users.map { user -> user.name }
/// ```
///
/// Tree-sitter: lambda_literal → lambda_parameters → variable_declaration → identifier
///
/// This is also exported as `extract_lambda_params_in_body` for use from calls.rs.
pub(super) fn extract_lambda_params(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    extract_lambda_params_in_body(node, src, source_symbol_index, symbols);
}

pub(super) fn extract_lambda_params_in_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    // Walk all lambda_literal nodes anywhere under `node`.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "lambda_literal" || child.kind() == "annotated_lambda" {
            let lambda = if child.kind() == "annotated_lambda" {
                // annotated_lambda → annotation* + lambda_literal
                let mut lc = child.walk();
                let found = child.children(&mut lc).find(|c| c.kind() == "lambda_literal");
                found
            } else {
                Some(child)
            };
            if let Some(ll) = lambda {
                extract_lambda_literal_params(&ll, src, source_symbol_index, symbols);
            }
        } else {
            extract_lambda_params_in_body(&child, src, source_symbol_index, symbols);
        }
    }
}

fn extract_lambda_literal_params(
    lambda_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = lambda_node.walk();
    for child in lambda_node.children(&mut cursor) {
        if child.kind() == "lambda_parameters" {
            let mut pc = child.walk();
            for param in child.children(&mut pc) {
                match param.kind() {
                    "variable_declaration" => {
                        push_lambda_param(&param, src, source_symbol_index, symbols);
                    }
                    "multi_variable_declaration" => {
                        let mut mc = param.walk();
                        for inner in param.children(&mut mc) {
                            if inner.kind() == "variable_declaration" {
                                push_lambda_param(&inner, src, source_symbol_index, symbols);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn push_lambda_param(
    node: &Node,
    src: &[u8],
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    // variable_declaration → identifier (name), type?
    let mut cursor = node.walk();
    let mut name: Option<String> = None;
    let mut type_name: Option<String> = None;

    for child in node.children(&mut cursor) {
        match child.kind() {
            "simple_identifier" | "identifier" => {
                if name.is_none() {
                    name = Some(node_text(child, src));
                }
            }
            "type" => {
                type_name = name_from_type_node(&child, src);
            }
            _ => {}
        }
    }

    let name = match name {
        Some(n) => n,
        None => return,
    };

    let signature = type_name.map(|t| format!("{name}: {t}"));

    symbols.push(ExtractedSymbol {
        qualified_name: name.clone(),
        name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::extract::extract;
    use crate::types::{EdgeKind, SymbolKind};

    fn decorator_refs(source: &str) -> Vec<(String, Option<String>)> {
        extract(source)
            .refs
            .into_iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| (r.target_name, r.module))
            .collect()
    }

    #[test]
    fn annotation_on_class() {
        let src = "@Service\nclass UserService {}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "Service"), "refs: {dr:?}");
    }

    #[test]
    fn annotation_with_route_arg() {
        let src = r#"class C {
    @GetMapping("/users")
    fun get() {}
}"#;
        let dr = decorator_refs(src);
        let found = dr.iter().find(|(n, _)| n == "GetMapping");
        assert!(found.is_some(), "GetMapping not in refs: {dr:?}");
        assert_eq!(found.unwrap().1, Some("/users".to_string()));
    }

    #[test]
    fn multiple_annotations() {
        let src = "@Service\n@Transactional\nclass Svc {}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "Service"), "refs: {dr:?}");
        assert!(dr.iter().any(|(n, _)| n == "Transactional"), "refs: {dr:?}");
    }

    #[test]
    fn when_expression_is_check() {
        let src = r#"
class Matcher {
    fun check(user: Any) {
        when (user) {
            is Admin -> 1
            is Student -> 2
            else -> 0
        }
    }
}
"#;
        let dr = decorator_refs(src);
        assert!(
            dr.iter().any(|(n, _)| n == "Admin"),
            "Admin TypeRef not found; refs: {dr:?}"
        );
        assert!(
            dr.iter().any(|(n, _)| n == "Student"),
            "Student TypeRef not found; refs: {dr:?}"
        );
    }

    #[test]
    fn lambda_params_extracted() {
        let src = r#"
class Mapper {
    fun map() {
        val names = users.map { user -> user.name }
    }
}
"#;
        let r = extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "user" && s.kind == SymbolKind::Variable),
            "lambda param 'user' not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }
}

