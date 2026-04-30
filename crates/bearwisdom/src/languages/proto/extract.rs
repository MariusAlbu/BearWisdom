// =============================================================================
// languages/proto/extract.rs  —  Protocol Buffers extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Struct      — message (core proto type)
//   Interface   — service (RPC service definition)
//   Method      — rpc (method within a service)
//   Enum        — enum
//   EnumMember  — enum_field (enum value)
//   Field       — field, map_field, oneof_field
//   Class       — extend (extension block)
//   Namespace   — package declaration
//
// REFERENCES:
//   TypeRef   — field/map_field/oneof_field → message_or_enum_type
//   TypeRef   — rpc → request + response message_or_enum_type
//   TypeRef   — extend → full_ident (extended type)
//   Imports   — import → path string
//
// Grammar: tree-sitter-proto (not yet in Cargo.toml — ready for when added).
// Node names follow the protobuf grammar conventions (proto2/proto3 shared).
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// Proto primitive types that should not generate TypeRef edges.
const PRIMITIVE_TYPES: &[&str] = &[
    "double", "float", "int32", "int64", "uint32", "uint64",
    "sint32", "sint64", "fixed32", "fixed64", "sfixed32", "sfixed64",
    "bool", "string", "bytes",
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from a .proto file.
///
/// Requires the tree-sitter-proto grammar to be available as `language`.
/// Called by `ProtoPlugin::extract()` once the grammar is wired in.
#[allow(dead_code)]
pub fn extract(source: &str, language: tree_sitter::Language) -> crate::types::ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Proto grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_file(tree.root_node(), source, &mut symbols, &mut refs);

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// File-level traversal
// ---------------------------------------------------------------------------

fn visit_file(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // Pre-pass: find the file's package declaration so that top-level message,
    // service, and enum symbols can be qualified as `<package>.<Name>`.
    let pkg = find_package_name(node, src);
    visit_file_with_package(node, src, symbols, refs, pkg.as_deref());
}

fn find_package_name(node: Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "source_file" | "file" => {
                if let Some(p) = find_package_name(child, src) {
                    return Some(p);
                }
            }
            "package" => {
                let name = collect_full_ident(&child, src);
                if !name.is_empty() {
                    return Some(name);
                }
            }
            _ => {}
        }
    }
    None
}

fn visit_file_with_package(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    pkg: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "source_file" | "file" => {
                visit_file_with_package(child, src, symbols, refs, pkg);
            }
            "package" => extract_package(&child, src, symbols),
            "import" => extract_import(&child, src, symbols, refs),
            "message" => extract_message(&child, src, symbols, refs, None, pkg),
            "service" => extract_service(&child, src, symbols, refs, pkg),
            "enum" => extract_enum(&child, src, symbols, refs, None, pkg),
            "extend" => extract_extend(&child, src, symbols, refs),
            _ => {}
        }
    }
}

/// Build a fully-qualified name as `[prefix.]name`, skipping prefix if empty.
fn qualify(prefix: Option<&str>, name: &str) -> String {
    match prefix {
        Some(p) if !p.is_empty() => format!("{p}.{name}"),
        _ => name.to_string(),
    }
}

// ---------------------------------------------------------------------------
// package  →  Namespace
// ---------------------------------------------------------------------------

fn extract_package(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    // package full_ident — collect all identifier children joined by "."
    let name = collect_full_ident(node, src);
    if name.is_empty() {
        return;
    }

    symbols.push(make_symbol(
        name.clone(),
        name.clone(),
        SymbolKind::Namespace,
        node,
        Some(format!("package {}", name)),
        None,
    ));
}

// ---------------------------------------------------------------------------
// import  →  Imports edge
// ---------------------------------------------------------------------------

fn extract_import(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let path = node
        .child_by_field_name("path")
        .map(|n| node_text(n, src))
        .or_else(|| find_string_literal(node, src));

    if let Some(raw) = path {
        let stripped = raw.trim_matches('"').trim_matches('\'').to_string();
        if stripped.is_empty() {
            return;
        }
        let idx = symbols.len();
        symbols.push(make_symbol(
            stripped.clone(),
            stripped.clone(),
            SymbolKind::Namespace,
            node,
            Some(format!("import \"{}\"", stripped)),
            None,
        ));
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: stripped.clone(),
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: Some(stripped),
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }
}

// ---------------------------------------------------------------------------
// message  →  Struct (with nested messages, enums, fields)
// ---------------------------------------------------------------------------

fn extract_message(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qname_prefix: Option<&str>,
) {
    let name = match message_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let qname = qualify(qname_prefix, &name);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        qname.clone(),
        SymbolKind::Struct,
        node,
        Some(format!("message {}", name)),
        parent_index,
    ));

    // Walk message_body for fields, nested messages, enums, oneofs.
    // Nested types qualify as `<message_qname>.<NestedName>`.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "message_body" {
            extract_message_body(&child, src, idx, symbols, refs, Some(qname.as_str()));
        }
    }
}

fn extract_message_body(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    qname_prefix: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "field" => extract_field(&child, src, parent_index, symbols, refs),
            "map_field" => extract_map_field(&child, src, parent_index, symbols, refs),
            "oneof" => extract_oneof(&child, src, parent_index, symbols, refs),
            "message" => extract_message(&child, src, symbols, refs, Some(parent_index), qname_prefix),
            "enum" => extract_enum(&child, src, symbols, refs, Some(parent_index), qname_prefix),
            "extend" => extract_extend(&child, src, symbols, refs),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// service  →  Interface
// ---------------------------------------------------------------------------

fn extract_service(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    qname_prefix: Option<&str>,
) {
    let name = match service_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let qname = qualify(qname_prefix, &name);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        qname,
        SymbolKind::Interface,
        node,
        Some(format!("service {}", name)),
        None,
    ));

    // Extract rpc methods
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "rpc" {
            extract_rpc(&child, src, idx, symbols, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// rpc  →  Method
// ---------------------------------------------------------------------------

fn extract_rpc(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // rpc_name is a child node type (not a named field); get its identifier
    let name = find_child_of_kind(node, "rpc_name")
        .and_then(|n| first_identifier_text(&n, src))
        .or_else(|| first_identifier_text(node, src));

    let name = match name {
        Some(n) => n,
        None => return,
    };

    // Collect request and response types from message_or_enum_type children
    let msg_types = collect_message_or_enum_types(node, src);
    let req_type = msg_types.first().cloned().unwrap_or_default();
    let resp_type = msg_types.get(1).cloned().unwrap_or_default();

    let sig = format!("rpc {}({}) returns ({})", name, req_type, resp_type);

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Method,
        node,
        Some(sig),
        Some(parent_index),
    ));

    // TypeRef to request type
    if !req_type.is_empty() && !is_primitive(&req_type) {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: req_type,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }

    // TypeRef to response type
    if !resp_type.is_empty() && !is_primitive(&resp_type) {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: resp_type,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }
}

// ---------------------------------------------------------------------------
// enum  →  Enum + EnumMember children
// ---------------------------------------------------------------------------

fn extract_enum(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qname_prefix: Option<&str>,
) {
    let name = match enum_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let qname = qualify(qname_prefix, &name);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        qname,
        SymbolKind::Enum,
        node,
        Some(format!("enum {}", name)),
        parent_index,
    ));

    // Extract enum_field children from enum_body
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "enum_body" {
            let mut bc = child.walk();
            for field in child.children(&mut bc) {
                if field.kind() == "enum_field" {
                    extract_enum_field(&field, src, idx, symbols);
                }
            }
        }
    }

    let _ = refs; // No refs from enum declarations themselves
}

fn extract_enum_field(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    // enum_field: first identifier child is the name
    let name = match first_identifier_text(node, src) {
        Some(n) => n,
        None => return,
    };

    symbols.push(make_symbol(
        name.clone(),
        name.clone(),
        SymbolKind::EnumMember,
        node,
        Some(name),
        Some(parent_index),
    ));
}

// ---------------------------------------------------------------------------
// field  →  Field
// ---------------------------------------------------------------------------

fn extract_field(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // field: type child + name identifier
    // Name is the last identifier before the `=` number
    let name = field_name(node, src);
    let name = match name {
        Some(n) => n,
        None => return,
    };

    let type_name = field_type_name(node, src);
    let sig = match &type_name {
        Some(t) => format!("{} {}", t, name),
        None => name.clone(),
    };

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Field,
        node,
        Some(sig),
        Some(parent_index),
    ));

    // TypeRef to field type (non-primitive message/enum types only)
    if let Some(t) = type_name {
        if !is_primitive(&t) {
            refs.push(ExtractedRef {
                source_symbol_index: idx,
                target_name: t,
                kind: EdgeKind::TypeRef,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
    }
}

// ---------------------------------------------------------------------------
// map_field  →  Field
// ---------------------------------------------------------------------------

fn extract_map_field(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // map<key_type, value_type> name = number;
    let name = match field_name(node, src) {
        Some(n) => n,
        None => return,
    };

    // Collect the two type identifiers (key_type, value_type)
    let msg_types = collect_message_or_enum_types(node, src);
    let val_type = msg_types.first().cloned();
    let sig = format!("map<...> {}", name);

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Field,
        node,
        Some(sig),
        Some(parent_index),
    ));

    // TypeRef to value type if it's a message/enum type
    if let Some(t) = val_type {
        if !is_primitive(&t) {
            refs.push(ExtractedRef {
                source_symbol_index: idx,
                target_name: t,
                kind: EdgeKind::TypeRef,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
    }
}

// ---------------------------------------------------------------------------
// oneof  →  scope with oneof_field children
// ---------------------------------------------------------------------------

fn extract_oneof(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // oneof name { ... }
    let name = match first_identifier_text(node, src) {
        Some(n) => n,
        None => return,
    };

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name.clone(),
        SymbolKind::Field,
        node,
        Some(format!("oneof {}", name)),
        Some(parent_index),
    ));

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "oneof_field" {
            extract_field(&child, src, idx, symbols, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// extend  →  Class + TypeRef to extended type
// ---------------------------------------------------------------------------

fn extract_extend(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // extend full_ident { ... }
    let target = collect_full_ident(node, src);
    if target.is_empty() {
        return;
    }

    let name = format!("extend {}", target);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Class,
        node,
        Some(format!("extend {}", target)),
        None,
    ));

    refs.push(ExtractedRef {
        source_symbol_index: idx,
        target_name: target,
        kind: EdgeKind::TypeRef,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Name extraction helpers
// ---------------------------------------------------------------------------

fn message_name(node: &Node, src: &str) -> Option<String> {
    find_child_of_kind(node, "message_name")
        .and_then(|n| first_identifier_text(&n, src))
}

fn service_name(node: &Node, src: &str) -> Option<String> {
    find_child_of_kind(node, "service_name")
        .and_then(|n| first_identifier_text(&n, src))
}

fn enum_name(node: &Node, src: &str) -> Option<String> {
    find_child_of_kind(node, "enum_name")
        .and_then(|n| first_identifier_text(&n, src))
}

/// Get field name: the identifier that appears just before `= <number>`.
/// In proto, field grammar is: [label] type fieldName = fieldNumber [options];
/// The name identifier is the last non-keyword identifier before `=`.
fn field_name(node: &Node, src: &str) -> Option<String> {
    // Try field_name named field first
    if let Some(n) = node.child_by_field_name("field_name") {
        return Some(node_text(n, src));
    }
    // Fallback: collect all identifiers, return the last one (the field name)
    let mut last_ident: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            last_ident = Some(node_text(child, src));
        }
        // Stop at `=` sign
        if node_text(child, src) == "=" {
            break;
        }
    }
    last_ident
}

/// Get field type name — either from a message_or_enum_type or type keyword.
fn field_type_name(node: &Node, src: &str) -> Option<String> {
    // Try message_or_enum_type first (complex type)
    if let Some(t) = collect_message_or_enum_types(node, src).into_iter().next() {
        return Some(t);
    }
    // Try type keyword child (primitive)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let k = child.kind();
        if k.starts_with("keyword_") || PRIMITIVE_TYPES.contains(&node_text(child, src).as_str()) {
            return Some(node_text(child, src));
        }
    }
    None
}

/// Collect all `message_or_enum_type` texts from immediate children or inside `type` wrappers.
fn collect_message_or_enum_types(node: &Node, src: &str) -> Vec<String> {
    let mut types = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "message_or_enum_type" {
            let name = collect_dotted_identifiers(&child, src);
            if !name.is_empty() {
                types.push(name);
            }
        } else if child.kind() == "type" {
            // type node wraps message_or_enum_type for complex types
            let mut tc = child.walk();
            for tc_child in child.children(&mut tc) {
                if tc_child.kind() == "message_or_enum_type" {
                    let name = collect_dotted_identifiers(&tc_child, src);
                    if !name.is_empty() {
                        types.push(name);
                    }
                }
            }
        }
    }
    types
}

/// Collect all identifiers in a node joined with `.` (for dotted names).
fn collect_dotted_identifiers(node: &Node, src: &str) -> String {
    let mut parts = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "full_ident" {
            let t = node_text(child, src);
            if !t.is_empty() && t != "." {
                parts.push(t);
            }
        }
    }
    parts.join(".")
}

/// Collect full_ident text from immediate identifier children (dotted package/type name).
fn collect_full_ident(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "full_ident" {
            return collect_dotted_identifiers(&child, src);
        }
    }
    // Fallback: join all identifier children
    collect_dotted_identifiers(node, src)
}

fn find_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

fn first_identifier_text(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let t = node_text(child, src);
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn find_string_literal(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let k = child.kind();
        if k == "string_literal" || k == "string" || k.contains("string") {
            return Some(node_text(child, src));
        }
    }
    None
}

fn is_primitive(name: &str) -> bool {
    PRIMITIVE_TYPES.contains(&name)
}

fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    node: &Node,
    signature: Option<String>,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index,
    }
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
