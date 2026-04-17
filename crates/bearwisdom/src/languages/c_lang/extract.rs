// =============================================================================
// parser/extractors/c_lang/mod.rs  —  C and C++ symbol and reference extractor
// =============================================================================


use super::predicates;
use super::calls::extract_calls_from_body;
use super::helpers::node_text;
use super::symbols::{
    emit_typerefs_for_type_descriptor, extract_bases, extract_enum_body, push_alias_decl,
    push_declaration, push_function_def, push_include, push_namespace, push_preproc_def,
    push_preproc_function_def, push_specifier, push_template_decl, push_typedef, push_using_decl,
};

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static C_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "struct_specifier", name_field: "name" },
    ScopeKind { node_kind: "enum_specifier",   name_field: "name" },
    ScopeKind { node_kind: "union_specifier",  name_field: "name" },
];

pub(crate) static CPP_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "namespace_definition", name_field: "name" },
    ScopeKind { node_kind: "class_specifier",      name_field: "name" },
    ScopeKind { node_kind: "struct_specifier",     name_field: "name" },
    ScopeKind { node_kind: "enum_specifier",       name_field: "name" },
    ScopeKind { node_kind: "union_specifier",      name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Return `true` when `source` contains C++-only constructs that indicate the
/// file should be parsed with the C++ grammar even if the language id was
/// detected as `"c"` (happens for `.h` files in mixed C/C++ projects).
fn is_cpp_content(source: &str) -> bool {
    // Fast byte-scan: look for C++-only keywords before the first function
    // body (i.e. the first `{`). Using byte search avoids regex overhead.
    let sentinel = source.find('{').unwrap_or(source.len());
    let header = &source[..sentinel];
    for token in ["namespace ", "template<", "template <", "class ", "operator "] {
        if header.contains(token) {
            return true;
        }
    }
    false
}

pub fn extract(source: &str, language: &str) -> super::ExtractionResult {
    // Upgrade ".h" files that contain C++-only constructs to the C++ grammar.
    // The language-profile detector maps ".h" → "c" (correct for pure C
    // projects), but in mixed or C++-only projects the header files contain
    // namespaces, templates, and classes that require the C++ grammar and the
    // CPP_SCOPE_KINDS scope config.
    let effective_language = if language == "c" && is_cpp_content(source) {
        "cpp"
    } else {
        language
    };

    let lang: tree_sitter::Language = if effective_language == "c" {
        tree_sitter_c::LANGUAGE.into()
    } else {
        tree_sitter_cpp::LANGUAGE.into()
    };

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load C/C++ grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_config = if effective_language == "c" { C_SCOPE_KINDS } else { CPP_SCOPE_KINDS };
    let scope_tree = scope_tree::build(root, src, scope_config);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(root, src, &scope_tree, effective_language, &mut symbols, &mut refs, None);

    // Full-CST type-ref sweep: emit TypeRef for every non-builtin type_identifier
    // and a ref for every template_argument_list in the CST.  This ensures the
    // ref coverage engine can match all type_identifier and template_argument_list
    // nodes, regardless of their depth or syntactic context.
    let sweep_idx = symbols.len().saturating_sub(1);
    sweep_typerefs(root, src, sweep_idx, effective_language, &mut refs);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Recursive node visitor
// ---------------------------------------------------------------------------

fn extract_node<'a>(
    node: Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    language: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "preproc_include" => {
                push_include(&child, src, symbols.len(), refs);
            }

            // C++ `template<typename T> class/struct/fn { ... }`
            "template_declaration" if language != "c" => {
                let (idx, inner_node) = push_template_decl(
                    &child, src, scope_tree, language, symbols, refs, parent_index,
                );
                if let Some(inner) = inner_node {
                    // Inherit/bases for class/struct inner.
                    if let Some(sym_idx) = idx {
                        match inner.kind() {
                            "class_specifier" | "struct_specifier" => {
                                extract_bases(&inner, src, sym_idx, refs);
                            }
                            _ => {}
                        }
                    }
                    // Recurse into body.
                    let body_opt = inner.child_by_field_name("body");
                    if let Some(body) = body_opt {
                        match inner.kind() {
                            "function_definition" => {
                                let sym_idx = idx.unwrap_or_else(|| symbols.len().saturating_sub(1));
                                extract_calls_from_body(&body, src, sym_idx, refs);
                                // Also extract nested symbols inside the function body.
                                extract_node(body, src, scope_tree, language, symbols, refs, idx);
                            }
                            _ => {
                                extract_node(body, src, scope_tree, language, symbols, refs, idx);
                            }
                        }
                    }
                }
            }

            // C++ `using Alias = Type;`
            "alias_declaration" if language != "c" => {
                push_alias_decl(&child, src, scope_tree, symbols, refs, parent_index);
            }

            // C++ `using std::vector;`
            "using_declaration" if language != "c" => {
                push_using_decl(&child, src, symbols.len(), refs);
            }

            // `#define FOO value`
            "preproc_def" => {
                push_preproc_def(&child, src, scope_tree, symbols, parent_index);
            }

            // `#define MAX(a, b) expr`
            "preproc_function_def" => {
                push_preproc_function_def(&child, src, scope_tree, symbols, parent_index);
            }

            "function_definition" => {
                let idx = push_function_def(&child, src, scope_tree, language, symbols, parent_index);
                // Even if push_function_def returns None (e.g. operator overloads
                // not yet handled), still recurse into the body for nested symbols.
                let sym_idx = idx.unwrap_or_else(|| symbols.len().saturating_sub(1));
                // Emit TypeRef for the return type.
                if let Some(ret_node) = child.child_by_field_name("type") {
                    emit_typerefs_for_type_descriptor(ret_node, src, sym_idx, refs);
                }
                // Emit TypeRef for each parameter type.
                emit_param_type_refs(&child, src, sym_idx, refs);
                if let Some(body) = child.child_by_field_name("body") {
                    // Ref extraction (calls, type refs, new, etc.)
                    extract_calls_from_body(&body, src, sym_idx, refs);
                    // Symbol extraction for nested declarations, local classes, etc.
                    extract_node(body, src, scope_tree, language, symbols, refs, idx);
                }
            }

            "type_definition" => {
                let pre_typedef_len = symbols.len();
                push_typedef(&child, src, scope_tree, symbols, parent_index);
                let post_typedef_len = symbols.len();

                // Emit TypeRef from each new TypeAlias symbol to its source type.
                // This populates field_type_name("TSocketChannelPtr") so the chain
                // walker can dereference typedef aliases (e.g., TSocketChannelPtr → SocketChannel).
                if let Some(type_node) = child.child_by_field_name("type") {
                    match type_node.kind() {
                        "struct_specifier" | "union_specifier" => {
                            let spec_idx = push_specifier(
                                &type_node, src, scope_tree, SymbolKind::Struct,
                                symbols, parent_index,
                            );
                            if let Some(body) = type_node.child_by_field_name("body") {
                                extract_node(body, src, scope_tree, language, symbols, refs, spec_idx);
                            }
                        }
                        "enum_specifier" => {
                            let spec_idx = push_specifier(
                                &type_node, src, scope_tree, SymbolKind::Enum,
                                symbols, parent_index,
                            );
                            if let Some(body) = type_node.child_by_field_name("body") {
                                extract_enum_body(&body, src, scope_tree, symbols, spec_idx);
                            }
                        }
                        // Emit TypeRef from the typedef alias to the source type.
                        // e.g., `typedef SocketChannel* SocketChannelPtr;`
                        //   → TypeRef from SocketChannelPtr → SocketChannel
                        // This lets field_type_name("SocketChannelPtr") return "SocketChannel"
                        // after the type_info pass processes it.
                        "type_identifier" | "pointer_declarator" | "template_type"
                        | "qualified_identifier" => {
                            for sym_idx in pre_typedef_len..post_typedef_len {
                                emit_typerefs_for_type_descriptor(type_node, src, sym_idx, refs);
                            }
                        }
                        _ => {}
                    }
                }
            }

            "struct_specifier" | "union_specifier" => {
                let idx = push_specifier(&child, src, scope_tree, SymbolKind::Struct, symbols, parent_index);
                if language != "c" {
                    if let Some(sym_idx) = idx {
                        extract_bases(&child, src, sym_idx, refs);
                    }
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, language, symbols, refs, idx);
                }
            }

            "enum_specifier" => {
                let idx = push_specifier(&child, src, scope_tree, SymbolKind::Enum, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_enum_body(&body, src, scope_tree, symbols, idx);
                }
            }

            "class_specifier" if language != "c" => {
                let idx = push_specifier(&child, src, scope_tree, SymbolKind::Class, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_bases(&child, src, sym_idx, refs);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, language, symbols, refs, idx);
                }
            }

            "namespace_definition" if language != "c" => {
                let idx = push_namespace(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, language, symbols, refs, idx);
                }
            }

            "declaration" | "field_declaration" => {
                // Capture the symbol count before pushing so we know which
                // symbols were just introduced by this declaration.
                let pre_decl_len = symbols.len();
                push_declaration(&child, src, scope_tree, symbols, parent_index);

                // For type TypeRefs: attribute them to the newly declared
                // variable/field symbol (not the parent class/function).
                // This populates field_type_name("ClassName.field") in the
                // type_info map, which the chain walker uses for type inference.
                //
                // If push_declaration pushed no new symbols (e.g. it was a
                // type-only forward declaration), fall back to the parent.
                let type_source_idx = if symbols.len() > pre_decl_len {
                    symbols.len().saturating_sub(1)
                } else {
                    parent_index.unwrap_or(symbols.len().saturating_sub(1))
                };
                // For calls in initialisers, use parent scope (consistent with prior
                // behaviour and avoids false field_type attribution from RHS expressions).
                let call_source_idx = parent_index.unwrap_or(symbols.len().saturating_sub(1));

                // If the declaration's type is itself a struct/class/enum, extract
                // that specifier as a symbol too (e.g. `struct Foo { int x; } var;`).
                if let Some(type_node) = child.child_by_field_name("type") {
                    match type_node.kind() {
                        "struct_specifier" | "union_specifier" => {
                            let spec_idx = push_specifier(
                                &type_node, src, scope_tree, SymbolKind::Struct,
                                symbols, parent_index,
                            );
                            if language != "c" {
                                if let Some(sidx) = spec_idx {
                                    extract_bases(&type_node, src, sidx, refs);
                                }
                            }
                            if let Some(body) = type_node.child_by_field_name("body") {
                                extract_node(body, src, scope_tree, language, symbols, refs, spec_idx);
                            }
                        }
                        "enum_specifier" => {
                            let spec_idx = push_specifier(
                                &type_node, src, scope_tree, SymbolKind::Enum,
                                symbols, parent_index,
                            );
                            if let Some(body) = type_node.child_by_field_name("body") {
                                extract_enum_body(&body, src, scope_tree, symbols, spec_idx);
                            }
                        }
                        "class_specifier" if language != "c" => {
                            let spec_idx = push_specifier(
                                &type_node, src, scope_tree, SymbolKind::Class,
                                symbols, parent_index,
                            );
                            if let Some(sidx) = spec_idx {
                                extract_bases(&type_node, src, sidx, refs);
                            }
                            if let Some(body) = type_node.child_by_field_name("body") {
                                extract_node(body, src, scope_tree, language, symbols, refs, spec_idx);
                            }
                        }
                        "type_identifier" => {
                            let name = node_text(type_node, src);
                            if !name.is_empty() && !predicates::is_c_builtin(&name) {
                                refs.push(ExtractedRef {
                                    source_symbol_index: type_source_idx,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: type_node.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                });
                            }
                        }
                        "template_type" | "qualified_identifier" => {
                            emit_typerefs_for_type_descriptor(type_node, src, type_source_idx, refs);
                        }
                        _ => {}
                    }
                }
                // Emit Calls refs for call_expressions in declaration initialisers
                // (e.g. `static int x = compute_len("abc");`).
                extract_calls_from_body(&child, src, call_source_idx, refs);
                // Also recurse fully into the declaration so that nested
                // struct/enum/union specifiers in initializers and complex
                // declarators are extracted as symbols.
                extract_node(child, src, scope_tree, language, symbols, refs, parent_index);
            }

            // Global-scope expression statements: e.g. `DEFINE_ALLOCATOR(argv_realloc, ...)`.
            // These are function-like macro invocations that tree-sitter parses as
            // `expression_statement` → `call_expression` at the top level.
            "expression_statement" => {
                let source_idx = parent_index.unwrap_or(symbols.len().saturating_sub(1));
                extract_calls_from_body(&child, src, source_idx, refs);
                // Recurse for symbol extraction (e.g. compound literals with inline struct defs)
                extract_node(child, src, scope_tree, language, symbols, refs, parent_index);
            }

            // Recurse into ERROR nodes — tree-sitter ERROR blocks often wrap valid
            // C++ that the grammar doesn't fully understand (e.g. C++20 features).
            // Skipping them silently causes massive coverage misses in projects that
            // use modern C++ (like entt which uses C++20 concepts/modules).
            "ERROR" | "MISSING" => {
                extract_node(child, src, scope_tree, language, symbols, refs, parent_index);
            }

            _ => {
                extract_node(child, src, scope_tree, language, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parameter type ref emission
// ---------------------------------------------------------------------------

/// Walk a function_definition's declarator chain to find parameter_list,
/// then emit TypeRef for each parameter's type_identifier.
fn emit_param_type_refs(
    func_node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The parameter_list lives inside the function_declarator inside the
    // declarator field. We walk the declarator subtree to find it.
    if let Some(decl_node) = func_node.child_by_field_name("declarator") {
        emit_param_types_from_declarator(&decl_node, src, source_idx, refs);
    }
}

fn emit_param_types_from_declarator(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "parameter_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "parameter_declaration" {
                    if let Some(type_node) = child.child_by_field_name("type") {
                        emit_typerefs_for_type_descriptor(type_node, src, source_idx, refs);
                    }
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                emit_param_types_from_declarator(&child, src, source_idx, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Full-CST type-ref sweep
// ---------------------------------------------------------------------------

/// Walk the entire CST and emit:
///   - TypeRef for every named `type_identifier` that is not a C/C++ builtin.
///   - A Calls ref for every `template_argument_list` (represents generic type usage).
///   - TypeRef for every `base_class_clause` — the inherits ref.
///   - TypeRef for every `sizeof_expression` argument type.
///
/// This sweep runs after the main extraction and ensures the coverage engine can
/// match all relevant ref-producing node kinds regardless of nesting depth.
fn sweep_typerefs<'a>(
    node: Node<'a>,
    src: &[u8],
    default_sym_idx: usize,
    language: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" => {
                let name = node_text(child, src);
                if !name.is_empty()
                    && !predicates::is_c_builtin(&name)
                    && !predicates::is_template_param(&name)
                {
                    refs.push(ExtractedRef {
                        source_symbol_index: default_sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
                // type_identifier is a leaf — no children to recurse into.
            }
            "template_argument_list" => {
                // Recurse into children for nested type_identifiers, but do NOT
                // emit a synthetic "<template_args>" ref — that token can never
                // resolve and only inflates unresolved counts.
                sweep_typerefs(child, src, default_sym_idx, language, refs);
            }
            "base_class_clause" if language != "c" => {
                // Emit Inherits refs for base class identifiers.
                let mut ic = child.walk();
                for base in child.children(&mut ic) {
                    match base.kind() {
                        "type_identifier" => {
                            let name = node_text(base, src);
                            if !name.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index: default_sym_idx,
                                    target_name: name,
                                    kind: EdgeKind::Inherits,
                                    line: base.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                });
                            }
                        }
                        "base_class_specifier" => {
                            let mut bsc = base.walk();
                            for inner in base.children(&mut bsc) {
                                if inner.kind() == "type_identifier" {
                                    let name = node_text(inner, src);
                                    if !name.is_empty() {
                                        refs.push(ExtractedRef {
                                            source_symbol_index: default_sym_idx,
                                            target_name: name,
                                            kind: EdgeKind::Inherits,
                                            line: inner.start_position().row as u32,
                                            module: None,
                                            chain: None,
                                        });
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                sweep_typerefs(child, src, default_sym_idx, language, refs);
            }
            "sizeof_expression" => {
                // Emit TypeRef for the argument type of sizeof.
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if inner.kind() == "type_descriptor" {
                        emit_typerefs_for_type_descriptor(inner, src, default_sym_idx, refs);
                    }
                }
                // The sweep will emit TypeRef for type_identifier children too.
                sweep_typerefs(child, src, default_sym_idx, language, refs);
            }
            // Skip string/comment nodes that have no useful type info.
            "string_literal" | "comment" | "number_literal" | "char_literal"
            | "concatenated_string" => {}
            _ => {
                sweep_typerefs(child, src, default_sym_idx, language, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests

