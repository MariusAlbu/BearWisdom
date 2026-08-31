// =============================================================================
// go/types.rs  —  Type declarations, struct fields, interface methods,
//                  type-subtree walker, and inline struct field extraction
// =============================================================================

use super::helpers::{
    build_method_elem_signature, extract_go_doc_comment, go_visibility, is_go_builtin_type,
    node_text, qualify, scope_from_prefix,
};
use super::tags;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Type declarations
// ---------------------------------------------------------------------------

pub(super) fn extract_type_declaration(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_spec" => {
                extract_type_spec(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }
            // `type Foo = Bar` — tree-sitter-go 0.23+ uses a distinct `type_alias` node.
            // Fields: `name` (type_identifier), `type` (_type)
            "type_alias" => {
                extract_type_alias_decl(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }
            _ => {}
        }
    }
}

/// Extract a `type_alias` node (`type Foo = Bar`).
///
/// tree-sitter-go shape:
/// ```text
/// type_alias
///   name: type_identifier   "Foo"
///   "="                     (anonymous)
///   type: _type             "Bar"
/// ```
///
/// `type Foo = Bar` is a Go TRUE alias: `Foo` shares `Bar`'s full method set.
/// We emit exactly one `TypeRef` naming the RHS head when that head is a
/// nameable, non-builtin type (`Bar`, `pkg.Bar`, `List[T]`), so the alias's
/// `field_type` collapses to its target and alias expansion can walk through
/// it. When the RHS is a structural type (`[]T`, `map[K]V`, `func(...)`,
/// `chan T`, `struct{…}`, `*T`) we emit no head ref — there is no nameable
/// target to walk to, so the alias stays a no-op for member resolution.
fn extract_type_alias_decl(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, source);
    if name.is_empty() {
        return;
    }

    let type_node = node.child_by_field_name("type");
    let type_text = type_node.map(|n| node_text(&n, source)).unwrap_or_default();

    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = go_visibility(&name);
    let doc_comment = extract_go_doc_comment(node, source);
    let sig = if type_text.is_empty() {
        format!("type {name} =")
    } else {
        format!("type {name} = {type_text}")
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });

    if let Some(rhs) = type_node {
        emit_alias_head_ref(&rhs, source, idx, refs);
    }
}

/// Emit the single alias-target `TypeRef` for a Go alias RHS, but only when the
/// RHS head is a nameable, non-builtin type identifier. The head ref's
/// `target_name` is what `field_type` collapses to and what alias expansion
/// rewrites the alias type to.
///
/// Nameable heads (`Bar`, `pkg.Bar`, `Map[K]V`) emit one ref. Structural RHS
/// shapes (`slice_type`, `map_type`, `function_type`, `channel_type`,
/// `struct_type`, `interface_type`, `pointer_type`, `array_type`) have no
/// nameable head — emitting the inner element/key/param as the head would
/// rewrite the alias to the wrong type, so we emit nothing.
fn emit_alias_head_ref(
    rhs: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // `List[int]` (Go 1.18+) recurses to its base via `go_type_ref_target`,
    // which may itself be a `qualified_type` (`pkg.List[int]`). Every other
    // structural RHS shape has no nameable head.
    let (head, module) = match rhs.kind() {
        "type_identifier" | "qualified_type" | "generic_type" => {
            match super::qualified_types::go_type_ref_target(rhs, source) {
                Some(parts) => parts,
                None => return,
            }
        }
        _ => return,
    };

    if head.is_empty() || is_go_builtin_type(&head) {
        return;
    }

    refs.push(ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index,
        target_name: head,
        kind: EdgeKind::TypeRef,
        line: rhs.start_position().row as u32,
        col: 0,
        module,
        chain: None,
        byte_offset: rhs.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

/// `type_spec` children (positional, named):
///   type_identifier (name), [=], struct_type | interface_type | other_type
fn extract_type_spec(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    // The first named child is always the type_identifier (name).
    // The second named child is the type body (may be struct_type, interface_type,
    // or any other type expression).
    let mut named_children: Vec<Node> = {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .filter(|c| c.is_named())
            .collect()
    };

    if named_children.is_empty() {
        return;
    }

    let name_node = named_children.remove(0);
    if name_node.kind() != "type_identifier" {
        return;
    }
    let name = node_text(&name_node, source);

    // `named_children` now holds [type_body] (after removing the name node).
    // For `type Foo = Bar` the `=` is an anonymous node so it doesn't appear
    // in named_children; the type body is still the first (and only) remaining.
    //
    // For generic types like `type Result[T any] struct { ... }`, tree-sitter-go
    // emits a `type_parameter_list` or `type_parameter_declaration` node BEFORE
    // the actual type body.  Skip over those so we find the struct_type / interface_type.
    let type_node = match named_children.into_iter().find(|n| {
        !matches!(
            n.kind(),
            "type_parameter_list" | "type_parameter_declaration" | "type_constraints"
        )
    }) {
        Some(n) => n,
        None => return,
    };

    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = go_visibility(&name);
    let doc_comment = extract_go_doc_comment(node, source);

    match type_node.kind() {
        "struct_type" => {
            let sig = format!("type {name} struct");
            let idx = symbols.len();
            let struct_prefix = qualify(&name, qualified_prefix);
            symbols.push(ExtractedSymbol {
                name: name.clone(),
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
                byte_offset: 0,
                declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
            });
            extract_struct_fields(&type_node, source, symbols, refs, Some(idx), &struct_prefix);
        }

        "interface_type" => {
            let sig = format!("type {name} interface");
            let idx = symbols.len();
            let iface_prefix = qualify(&name, qualified_prefix);
            symbols.push(ExtractedSymbol {
                name: name.clone(),
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
                byte_offset: 0,
                declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
            });
            extract_interface_methods_with_refs(
                &type_node,
                source,
                symbols,
                refs,
                Some(idx),
                &iface_prefix,
            );
        }

        _ => {
            // Defined type (`type Foo Bar`, `type Stack []Item`). A Go defined
            // type does NOT share the underlying type's method set — its own
            // methods live under qname `Foo.M` and must resolve against `Foo`.
            // So it emits no alias-target ref: with no `field_type`, no
            // expandable `AliasTarget` is synthesized and alias expansion stays
            // a no-op for the defined type. True aliases (`type Foo = Bar`, the
            // `type_alias` node) are the only Go form that synthesizes an
            // expandable target — see `extract_type_alias_decl`.
            let type_text = node_text(&type_node, source);
            let sig = format!("type {name} {type_text}");
            symbols.push(ExtractedSymbol {
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
                byte_offset: 0,
                declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Struct fields
// ---------------------------------------------------------------------------

/// Walk the `struct_type` → `field_declaration_list` and emit Field symbols.
///
/// Embedded (anonymous) fields also emit `Inherits` refs.
pub(super) fn extract_struct_fields(
    struct_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    struct_prefix: &str,
) {
    // struct_type children: `struct` (anon keyword), field_declaration_list
    let mut cursor = struct_node.walk();
    for child in struct_node.children(&mut cursor) {
        if child.kind() == "field_declaration_list" {
            extract_field_declaration_list(
                &child,
                source,
                symbols,
                refs,
                parent_index,
                struct_prefix,
            );
        }
    }
}

fn extract_field_declaration_list(
    list_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    struct_prefix: &str,
) {
    let mut cursor = list_node.walk();
    for child in list_node.children(&mut cursor) {
        if child.kind() == "field_declaration" {
            extract_field_declaration(&child, source, symbols, refs, parent_index, struct_prefix);
        }
    }
}

/// A `field_declaration` is one of:
///
///   Named:    `field_identifier+ type`   — one or more names, then a type
///   Embedded: `type_identifier`          — just the embedded type name
///   Embedded: `pointer_type`             — `*EmbeddedType`
///
/// We distinguish these by looking for `field_identifier` children.
fn extract_field_declaration(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    struct_prefix: &str,
) {
    let mut field_names: Vec<String> = Vec::new();
    let mut type_text: Option<String> = None;
    let mut embedded_type: Option<(String, Option<String>)> = None;
    let mut tag_doc: Option<String> = None;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        match child.kind() {
            "field_identifier" => {
                field_names.push(node_text(&child, source));
            }
            // Embedded field — `EmbeddedType`, `*EmbeddedType`, or the
            // package-qualified forms `pkg.Type` / `*pkg.Type` (e.g.
            // `http.Handler`, `sync.Mutex`). The package qualifier, when
            // present, carries through as `module` on the Inherits edge.
            "type_identifier" | "pointer_type" | "qualified_type"
                if field_names.is_empty() && type_text.is_none() =>
            {
                embedded_type = super::qualified_types::go_type_ref_target(&child, source);
            }
            "raw_string_literal" => {
                // Struct tag: `json:"name" db:"col"`
                let raw = node_text(&child, source);
                let parsed = tags::parse_struct_tags(&raw);
                if !parsed.is_empty() {
                    tag_doc = Some(tags::format_tags(&parsed));
                }
            }
            _ => {
                // Any other named child after field_identifier(s) is the type.
                if !field_names.is_empty() {
                    type_text = Some(node_text(&child, source));
                    // Walk the type subtree to emit TypeRef for every
                    // type_identifier within it (handles slices, maps, pointers,
                    // channels, and nested anonymous structs).
                    emit_type_refs_from_subtree(
                        &child,
                        source,
                        parent_index.unwrap_or(0),
                        refs,
                        struct_prefix,
                        symbols,
                        parent_index,
                    );
                }
            }
        }
    }

    if let Some((et, module)) = embedded_type {
        if !et.is_empty() {
            // Emit Inherits edge from the struct (parent_index) to the embedded type.
            refs.push(ExtractedRef {
                is_include: false,
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: parent_index.unwrap_or(0),
                target_name: et.clone(),
                kind: EdgeKind::Inherits,
                line: node.start_position().row as u32,
                col: 0,
                module,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
            // Also emit a Field symbol (the embedded type acts as an accessible field).
            symbols.push(ExtractedSymbol {
                name: et.clone(),
                qualified_name: qualify(&et, struct_prefix),
                kind: SymbolKind::Field,
                visibility: go_visibility(&et),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: scope_from_prefix(struct_prefix),
                parent_index,
                byte_offset: 0,
                declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
            });
        }
    } else {
        // Named fields.
        //
        // We intentionally do NOT emit a TypeRef from the raw `type_text`
        // string here — that path produced garbage target_names for composite
        // types like `[]*Handler`, `map[string]User`, or `protoimpl.MessageState`
        // (the full qualified text was stored as a flat target_name and never
        // resolved). Instead, `emit_type_refs_from_subtree` below walks the
        // type tree and emits one correctly-shaped TypeRef per embedded
        // type_identifier / qualified_type node.
        let type_str = type_text.unwrap_or_default();

        for field_name in field_names {
            let vis = go_visibility(&field_name);
            let sig = if type_str.is_empty() {
                field_name.clone()
            } else {
                format!("{field_name} {type_str}")
            };
            symbols.push(ExtractedSymbol {
                name: field_name.clone(),
                qualified_name: qualify(&field_name, struct_prefix),
                kind: SymbolKind::Field,
                visibility: vis,
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: Some(sig),
                doc_comment: tag_doc.clone(),
                scope_path: scope_from_prefix(struct_prefix),
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

// ---------------------------------------------------------------------------
// Type subtree walker (TypeRef emission + nested struct field recursion)
// ---------------------------------------------------------------------------

/// Walk a Go type node and:
///   1. Emit `TypeRef` for every `type_identifier` that is not a builtin.
///   2. Recurse into `struct_type` → `field_declaration_list` so that nested
///      anonymous struct fields are also captured as `Field` symbols.
///
/// This ensures that complex field types like `[]*Handler`, `map[string]User`,
/// `chan Event`, and inline `struct { … }` all produce the correct coverage.
fn emit_type_refs_from_subtree(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    struct_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    match node.kind() {
        "type_identifier" => {
            let name = node_text(node, source);
            if !name.is_empty() && !is_go_builtin_type(&name) {
                refs.push(ExtractedRef {
                    is_include: false,
                    is_import_binding: false,
                    is_reexport: false,
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

        // Qualified type `pkg.Type` (e.g. `protoimpl.MessageState`,
        // `http.Handler`, `sync.Mutex`). Emit a single ref with the member
        // name as target and the package as `module` — the resolver's
        // external-classification step uses `module` to match against the
        // file's import list, which is how protobuf-runtime / sync / http
        // get classified as external packages. Do NOT recurse: the
        // type_identifier child would otherwise produce a duplicate ref
        // with no module set.
        "qualified_type" => {
            let parts = super::qualified_types::qualified_type_parts(node, source);
            if let Some((package, name)) = parts {
                if !name.is_empty() && !is_go_builtin_type(&name) {
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: node.start_position().row as u32,
                        col: 0,
                        module: Some(package),
                        chain: None,
                        byte_offset: node.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
        }

        // Nested anonymous struct — recurse into its field_declaration_list
        // so the inner field_declaration nodes are also captured.
        "struct_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "field_declaration_list" {
                    extract_field_declaration_list(
                        &child,
                        source,
                        symbols,
                        refs,
                        parent_index,
                        struct_prefix,
                    );
                }
            }
        }

        // For all other container types (slice_type, map_type, pointer_type,
        // channel_type, array_type, qualified_type, etc.) just recurse into
        // named children to find any type_identifier nodes within.
        _ => {
            if node.is_named() && node.child_count() > 0 {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.is_named() {
                        emit_type_refs_from_subtree(
                            &child,
                            source,
                            source_symbol_index,
                            refs,
                            struct_prefix,
                            symbols,
                            parent_index,
                        );
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Interface method elements
// ---------------------------------------------------------------------------

/// Walk the `interface_type` node and emit `Method` symbols for each
/// `method_elem`.
///
/// `method_elem` children: field_identifier (name), parameter_list (params),
/// result?
fn extract_interface_methods(
    iface_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    iface_prefix: &str,
) {
    // Use a local refs buffer; we don't need a `refs` parameter since
    // callers don't thread it through, but we emit TypeRefs via calls.
    // Actually, we need refs from the outer context. Add a dummy approach:
    // defer to the fn_signature extractor for param/return type TypeRefs.
    let mut cursor = iface_node.walk();
    for child in iface_node.children(&mut cursor) {
        if child.kind() != "method_elem" {
            continue;
        }

        // Find the field_identifier child by index (avoids cursor borrow issue).
        let name = (0..child.named_child_count())
            .filter_map(|i| child.named_child(i))
            .find(|c| c.kind() == "field_identifier")
            .map(|n| node_text(&n, source));

        let name = match name {
            Some(n) => n,
            None => continue,
        };

        let qualified_name = qualify(&name, iface_prefix);
        let visibility = go_visibility(&name);
        let signature = build_method_elem_signature(&child, source);

        symbols.push(ExtractedSymbol {
            name,
            qualified_name,
            kind: SymbolKind::Method,
            visibility,
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: child.end_position().column as u32,
            signature,
            doc_comment: None,
            scope_path: scope_from_prefix(iface_prefix),
            parent_index,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
    }
}

fn extract_interface_methods_with_refs(
    iface_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    iface_prefix: &str,
) {
    let mut cursor = iface_node.walk();
    for child in iface_node.children(&mut cursor) {
        if child.kind() != "method_elem" {
            continue;
        }

        let name = (0..child.named_child_count())
            .filter_map(|i| child.named_child(i))
            .find(|c| c.kind() == "field_identifier")
            .map(|n| node_text(&n, source));

        let name = match name {
            Some(n) => n,
            None => continue,
        };

        let qualified_name = qualify(&name, iface_prefix);
        let visibility = go_visibility(&name);
        let signature = build_method_elem_signature(&child, source);
        let sym_idx = symbols.len();

        symbols.push(ExtractedSymbol {
            name,
            qualified_name,
            kind: SymbolKind::Method,
            visibility,
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: child.end_position().column as u32,
            signature,
            doc_comment: None,
            scope_path: scope_from_prefix(iface_prefix),
            parent_index,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });

        // Emit TypeRef edges for parameter and return types of this method_elem.
        super::calls::extract_fn_signature_type_refs(&child, source, sym_idx, refs);
    }
}
// ---------------------------------------------------------------------------
// Inline struct field extraction
// ---------------------------------------------------------------------------

/// Walk an arbitrary expression subtree looking for anonymous `struct_type`
/// nodes and extract their fields.  This covers patterns like:
///
///   `data := struct{ Name string }{...}`
///   `rows := []struct{ URL string; Status int }{{...}, {...}}`
///
/// We stop descending into `function_literal` / `func_literal` nodes so we
/// don't accidentally steal fields from closures declared inside the RHS.
pub(super) fn extract_inline_struct_fields(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    match node.kind() {
        "struct_type" => {
            extract_struct_fields(node, source, symbols, refs, parent_index, qualified_prefix);
        }
        // Don't descend into closures — they are separate symbols.
        "function_literal" | "func_literal" => {}
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_inline_struct_fields(
                        &child,
                        source,
                        symbols,
                        refs,
                        parent_index,
                        qualified_prefix,
                    );
                }
            }
        }
    }
}
