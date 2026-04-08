// =============================================================================
// rust/symbols.rs  —  Symbol extractors for the Rust extractor
// =============================================================================

use super::helpers::{
    detect_visibility, extract_doc_comment, extract_signature, node_text, qualify, scope_from_prefix,
};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(super) fn extract_function(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let signature = extract_signature(node, source);

    let kind = if super::helpers::has_test_attribute(node, source) {
        SymbolKind::Test
    } else {
        SymbolKind::Function
    };

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

/// Same as `extract_function` but always emits `Method` kind (used inside impl blocks).
pub(super) fn extract_method_from_fn(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let signature = extract_signature(node, source);

    let kind = if super::helpers::has_test_attribute(node, source) {
        SymbolKind::Test
    } else {
        SymbolKind::Method
    };

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_struct(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);

    let mut sig = format!("struct {name}");
    if let Some(tp) = node.child_by_field_name("type_parameters") {
        sig.push_str(&node_text(&tp, source));
    }

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Struct,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_enum(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("enum {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Enum,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

/// Extract `enum_variant` children from an enum body into the symbol list.
/// Also emits TypeRef edges for any named types in variant field declarations.
pub(super) fn extract_enum_variants(
    body: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "enum_variant" {
            // tree-sitter-rust uses `name` field on enum_variant nodes.
            // Fall back to the first named identifier child if the field is missing.
            let field_name_node = child.child_by_field_name("name");
            let name_node = if field_name_node.is_some() {
                field_name_node
            } else {
                let mut variant_cursor = child.walk();
                let found = child
                    .children(&mut variant_cursor)
                    .find(|n| n.is_named() && n.kind() == "identifier");
                found
            };

            if let Some(name_node) = name_node {
                let name = node_text(&name_node, source);
                let qualified_name = qualify(&name, qualified_prefix);
                let sym_idx = symbols.len();
                symbols.push(ExtractedSymbol {
                    name,
                    qualified_name,
                    kind: SymbolKind::EnumMember,
                    visibility: None,
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: extract_doc_comment(&child, source),
                    scope_path: scope_from_prefix(qualified_prefix),
                    parent_index,
                });

                // Extract attributes on the enum variant (e.g. #[default], #[serde(rename="...")]).
                super::decorators::extract_decorators(&child, source, sym_idx, refs);

                // Emit TypeRefs for any typed fields in the variant body.
                // Covers tuple variants `Error(ErrorKind)` and struct variants
                // `Point { x: f32, y: f32 }` whose field types are type_identifiers.
                let mut vc = child.walk();
                for variant_child in child.children(&mut vc) {
                    match variant_child.kind() {
                        // Tuple variant: `Error(ErrorKind, String)`
                        "ordered_field_declaration_list" => {
                            let mut fc = variant_child.walk();
                            for field in variant_child.children(&mut fc) {
                                if field.kind() == "type" || field.is_named() {
                                    extract_type_refs_from_type_node(&field, source, sym_idx, refs);
                                }
                            }
                        }
                        // Struct variant: `Point { x: f32, y: f32 }`
                        "field_declaration_list" => {
                            let mut fc = variant_child.walk();
                            for field in variant_child.children(&mut fc) {
                                if field.kind() == "field_declaration" {
                                    if let Some(type_node) = field.child_by_field_name("type") {
                                        extract_type_refs_from_type_node(
                                            &type_node, source, sym_idx, refs,
                                        );
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

pub(super) fn extract_trait(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("trait {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Interface,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_type_alias(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("type {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_const(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);

    let mut sig = format!("const {name}");
    if let Some(ty) = node.child_by_field_name("type") {
        sig.push_str(": ");
        sig.push_str(&node_text(&ty, source));
    }

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_static(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);

    let mut sig = format!("static {name}");
    if let Some(ty) = node.child_by_field_name("type") {
        sig.push_str(": ");
        sig.push_str(&node_text(&ty, source));
    }

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

/// `macro_rules! foo { ... }` — emit a Function symbol for the macro name.
///
/// tree-sitter-rust 0.24 shape (`macro_definition` node):
/// ```text
/// macro_definition
///   name: identifier  "foo"
///   macro_rule+
/// ```
pub(super) fn extract_macro_rules(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    if name.is_empty() {
        return None;
    }
    let qualified_name = qualify(&name, qualified_prefix);
    let doc_comment = extract_doc_comment(node, source);

    Some(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: detect_visibility(node),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("macro_rules! {name}")),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

pub(super) fn extract_mod(
    node: &Node,
    source: &str,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) -> Option<ExtractedSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_visibility(node);
    let doc_comment = extract_doc_comment(node, source);
    let sig = format!("mod {name}");

    Some(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    })
}

// ---------------------------------------------------------------------------
// Struct field extraction
// ---------------------------------------------------------------------------

/// Walk the body of a `struct_item` or `union_item` and emit `Field` symbols
/// for each declared field, plus `TypeRef` edges for named (non-primitive) types.
///
/// tree-sitter-rust shape:
/// ```text
/// struct_item
///   name: identifier "MyStruct"
///   body: field_declaration_list
///     field_declaration
///       name: field_identifier  "field_name"
///       type: _type             SomeType
/// ```
pub(super) fn extract_struct_fields(
    struct_node: &Node,
    source: &str,
    struct_sym_index: usize,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let body = match struct_node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() != "field_declaration" {
            continue;
        }

        let name_node = match child.child_by_field_name("name") {
            Some(n) => n,
            None => continue,
        };
        let field_name = node_text(&name_node, source);
        if field_name.is_empty() {
            continue;
        }

        let qualified_name = qualify(&field_name, qualified_prefix);
        let visibility = detect_visibility(&child);

        // Build a concise signature: `field_name: TypeText`
        let sig = if let Some(type_node) = child.child_by_field_name("type") {
            let type_text = node_text(&type_node, source);
            Some(format!("{field_name}: {type_text}"))
        } else {
            Some(field_name.clone())
        };

        symbols.push(ExtractedSymbol {
            name: field_name.clone(),
            qualified_name,
            kind: SymbolKind::Field,
            visibility,
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: child.end_position().column as u32,
            signature: sig,
            doc_comment: extract_doc_comment(&child, source),
            scope_path: scope_from_prefix(qualified_prefix),
            parent_index: Some(struct_sym_index),
        });

        // Emit TypeRef for non-primitive field types.
        if let Some(type_node) = child.child_by_field_name("type") {
            extract_type_refs_from_type_node(&type_node, source, struct_sym_index, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// Function/method signature type extraction
// ---------------------------------------------------------------------------

/// Emit TypeRef edges for all named types in a function/method signature:
/// parameter types and return type.
///
/// This covers `type_identifier`, `scoped_type_identifier`, `generic_type`,
/// `reference_type`, `pointer_type`, `dynamic_trait_type` (`dyn Trait`), and
/// `abstract_type` (`impl Trait`) nodes found in the parameter list and
/// return type.
pub(super) fn extract_fn_signature_type_refs(
    fn_node: &Node,
    source: &str,
    sym_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Walk parameter list.
    if let Some(params) = fn_node.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for child in params.children(&mut cursor) {
            match child.kind() {
                "parameter" | "self_parameter" | "variadic_parameter" => {
                    if let Some(type_node) = child.child_by_field_name("type") {
                        extract_type_refs_from_type_node(&type_node, source, sym_index, refs);
                    }
                }
                _ => {}
            }
        }
    }

    // Walk return type.
    if let Some(ret) = fn_node.child_by_field_name("return_type") {
        extract_type_refs_from_type_node(&ret, source, sym_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Generic type node → TypeRef walker
// ---------------------------------------------------------------------------

/// Recursively walk a Rust type node and emit `TypeRef` edges for every
/// named (non-primitive) type referenced within it.
///
/// Handles:
/// - `type_identifier`          → direct named type `Foo`
/// - `scoped_type_identifier`   → `foo::Bar` — leaf name only
/// - `generic_type`             → `Vec<T>` — base + type arguments
/// - `type_arguments`           → `<T, U>` — each argument type
/// - `reference_type`           → `&T`, `&mut T`
/// - `pointer_type`             → `*const T`, `*mut T`
/// - `dynamic_trait_type`       → `dyn Error + Send`
/// - `abstract_type`            → `impl Trait`
/// - `array_type`               → `[T; N]`
/// - `tuple_type`               → `(A, B)`
/// - `optional_type`            → `Option<T>` (if present as distinct node)
pub(super) fn extract_type_refs_from_type_node(
    node: &Node,
    source: &str,
    sym_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "type_identifier" => {
            let name = node_text(node, source);
            if !name.is_empty() && !is_rust_primitive(&name) {
                refs.push(make_type_ref(sym_index, name, node.start_position().row as u32));
            }
        }

        "scoped_type_identifier" => {
            // `foo::Bar` — emit a TypeRef for the leaf name.
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(&n, source))
                .unwrap_or_else(|| {
                    let text = node_text(node, source);
                    text.rsplit("::").next().unwrap_or(&text).to_string()
                });
            if !name.is_empty() && !is_rust_primitive(&name) {
                refs.push(make_type_ref(sym_index, name, node.start_position().row as u32));
            }
        }

        "generic_type" => {
            // `Vec<T>` — emit TypeRef for the base type, then recurse into type_arguments.
            if let Some(base) = node.child_by_field_name("type") {
                extract_type_refs_from_type_node(&base, source, sym_index, refs);
            }
            if let Some(args) = node.child_by_field_name("type_arguments") {
                extract_type_refs_from_type_node(&args, source, sym_index, refs);
            }
        }

        "type_arguments" => {
            // `<A, B, C>` — recurse into each child type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_type_node(&child, source, sym_index, refs);
                }
            }
        }

        "reference_type" | "pointer_type" => {
            // `&T`, `&mut T`, `*const T` — recurse into the inner type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() && child.kind() != "mutable_specifier" && child.kind() != "lifetime" {
                    extract_type_refs_from_type_node(&child, source, sym_index, refs);
                }
            }
        }

        "dynamic_type" | "dynamic_trait_type" => {
            // `dyn Error + Send` — emit TypeRef for each trait name.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_type_node(&child, source, sym_index, refs);
                }
            }
        }

        "abstract_type" => {
            // `impl Trait` — emit a TypeRef at the abstract_type node's own line
            // so the coverage budget for `abstract_type` in ref_node_kinds is
            // consumed, THEN also emit TypeRefs for the inner trait name(s).
            let trait_name = super::calls::rust_type_node_name(node, source);
            if !trait_name.is_empty() && !is_rust_primitive(&trait_name) {
                refs.push(make_type_ref(sym_index, trait_name, node.start_position().row as u32));
            }
            // Also recurse so that individual type_identifier nodes inside are covered.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_type_node(&child, source, sym_index, refs);
                }
            }
        }

        "array_type" => {
            if let Some(elem) = node.child_by_field_name("element") {
                extract_type_refs_from_type_node(&elem, source, sym_index, refs);
            }
        }

        "tuple_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_type_node(&child, source, sym_index, refs);
                }
            }
        }

        // trait_bounds in a scoped context: `T: Clone + Send`
        "trait_bounds" => {
            super::patterns::extract_trait_bounds(node, source, sym_index, refs);
        }

        // Lifetime annotations — no TypeRef needed.
        "lifetime" => {}

        _ => {
            // Generic recursion for any other container node.
            if node.is_named() && node.child_count() > 0 {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.is_named() {
                        extract_type_refs_from_type_node(&child, source, sym_index, refs);
                    }
                }
            }
        }
    }
}

fn make_type_ref(sym_index: usize, name: String, line: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: sym_index,
        target_name: name,
        kind: EdgeKind::TypeRef,
        line,
        module: None,
        chain: None,
    }
}

/// Heuristic: return `true` for Rust scalar primitives that we don't want to
/// emit TypeRef edges for.  We keep this narrow so that stdlib types such as
/// `Vec`, `Arc`, `Box`, `Option`, etc. do produce TypeRef edges — they are
/// legitimate cross-symbol references (generic containers, trait objects, etc.).
pub(super) fn is_rust_primitive(name: &str) -> bool {
    matches!(
        name,
        // Primitive types
        "bool" | "char" | "str" | "String"
        | "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
        | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
        | "f32" | "f64"
        | "Self" | "self" | "()" | "!" | "never"
        // Wildcard type placeholder
        | "_"
        // Attribute keywords (language built-ins, not crate names)
        | "cfg_attr" | "cfg" | "derive" | "allow" | "warn" | "deny" | "forbid"
        | "deprecated" | "must_use" | "inline" | "repr" | "doc" | "test"
    )
}

// ---------------------------------------------------------------------------
// Associated type extraction in trait body
// ---------------------------------------------------------------------------

/// Extract `associated_type` nodes from a trait body, emitting TypeAlias symbols.
///
/// tree-sitter-rust shape (inside `trait_item` body):
/// ```text
/// associated_type
///   name: identifier  "Output"
///   [bounds: trait_bounds]
/// ```
pub(super) fn extract_trait_associated_types(
    trait_body: &Node,
    source: &str,
    trait_sym_index: usize,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = trait_body.walk();
    for child in trait_body.children(&mut cursor) {
        if child.kind() != "associated_type" {
            continue;
        }
        let name_node = match child.child_by_field_name("name") {
            Some(n) => n,
            None => continue,
        };
        let name = node_text(&name_node, source);
        if name.is_empty() {
            continue;
        }
        let qualified_name = qualify(&name, qualified_prefix);
        let sym_idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::TypeAlias,
            visibility: None,
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: child.end_position().column as u32,
            signature: Some(format!("type {name}")),
            doc_comment: extract_doc_comment(&child, source),
            scope_path: scope_from_prefix(qualified_prefix),
            parent_index: Some(trait_sym_index),
        });
        // Emit TypeRef for bounds if present.
        if let Some(bounds) = child.child_by_field_name("bounds") {
            super::patterns::extract_trait_bounds(&bounds, source, sym_idx, refs);
        }
    }
}
