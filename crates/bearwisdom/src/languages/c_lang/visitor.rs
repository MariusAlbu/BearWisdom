// =============================================================================
// languages/c_lang/visitor.rs  —  recursive CST walk that emits symbols + refs
// =============================================================================

use tree_sitter::Node;

use super::calls::extract_calls_from_body;
use super::helpers::node_text;
use super::macro_misparse::{
    detect_macro_class_misparse, emit_misparsed_base_class_refs, push_misparsed_class,
};
use super::predicates;
use super::symbols::{
    emit_typerefs_for_type_descriptor, extract_bases, extract_enum_body, push_alias_decl,
    push_declaration, push_function_def, push_include, push_namespace, push_namespace_alias,
    push_preproc_def, push_preproc_function_def, push_specifier, push_template_decl, push_typedef,
    push_using_decl,
};
use super::type_refs::emit_param_type_refs;
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};

// ---------------------------------------------------------------------------
// Recursive node visitor
// ---------------------------------------------------------------------------

pub(super) fn extract_node<'a>(
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
                // Salvage path: tree-sitter-cpp doesn't expand macros, so
                //   class Q_WIDGETS_EXPORT QMessageBox : public QDialog { ... }
                // gets misparsed as a function whose return type is the
                // class_specifier `class Q_WIDGETS_EXPORT` (with the macro
                // bound to the `name` field) and whose function name is
                // `QMessageBox`. Detect that shape and emit a Class symbol
                // for the real name instead — without it Qt-wide visibility
                // macros (and any project-defined `EXPORT` shim) silently
                // erase every class declaration that uses them.
                if let Some(real_name) = detect_macro_class_misparse(&child, src) {
                    let salvaged_idx = push_misparsed_class(
                        &child, &real_name, src, scope_tree, symbols, parent_index,
                    );
                    let sym_idx = salvaged_idx.unwrap_or_else(|| symbols.len().saturating_sub(1));
                    // Body is a compound_statement here; recurse for inner
                    // declarations (Q_OBJECT macros, member fields, methods).
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_node(body, src, scope_tree, language, symbols, refs, salvaged_idx);
                    }
                    // Skip the normal function_definition path so the same
                    // node doesn't also produce a method symbol.
                    if let Some(idx) = salvaged_idx {
                        // Emit Inherits TypeRef for the base class buried in
                        // the ERROR sibling (`: public QDialog`).
                        emit_misparsed_base_class_refs(&child, src, idx, refs);
                    }
                    continue;
                }

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

            // C++ namespace aliases: `namespace Dc = DeriveColors;`. Emit
            // the alias name as a Namespace symbol so subsequent uses of
            // `Dc::member` (which the chain extractor surfaces as a
            // TypeRef on the alias name) can resolve same-file. Also emit
            // a TypeRef from the alias to the target namespace so the
            // resolver chain can follow `Dc → DeriveColors` for member
            // lookup if/when target-aware alias substitution lands.
            "namespace_alias_definition" if language != "c" => {
                push_namespace_alias(&child, src, scope_tree, symbols, refs, parent_index);
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
                            if !name.is_empty() && !predicates::is_c_primitive_type(&name) {
                                refs.push(ExtractedRef {
                                    source_symbol_index: type_source_idx,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: type_node.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
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
