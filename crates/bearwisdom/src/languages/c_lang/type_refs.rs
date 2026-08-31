// =============================================================================
// languages/c_lang/type_refs.rs  —  parameter-type and full-CST type-ref sweep
// =============================================================================

use tree_sitter::Node;

use super::helpers::node_text;
use super::predicates;
use super::typerefs::{emit_typerefs_for_type_descriptor, is_cpp_keyword};
use crate::types::{EdgeKind, ExtractedRef};

/// True when `node` is a `type_identifier` sitting in the leading type slot of a
/// declaration or parameter while the real type token was demoted to an `ERROR`
/// sibling — the structural fingerprint of a qualifier macro that stole the type
/// position (`PERL_CALLCONV void f`, `pTHX_ SV* sv`, `CURL_EXTERN void g`, the
/// SAL annotations `_In_ HANDLE h` / `IN HANDLE h` / `__out DWORD* n`).
///
/// Without a preprocessor tree-sitter binds the macro to the `type` field and,
/// finding a second type token where it expected a declarator, demotes the real
/// type — and any trailing parameter name it can no longer place — to an `ERROR`
/// child of the same declaration / parameter. Two surplus-token layouts occur:
///   * the `ERROR` immediately follows the type slot (`_Inout_ int* p`), or
///   * the misparsed real type is bound as the declarator and the trailing
///     parameter name is the `ERROR` (`_In_ HANDLE h`, `IN HANDLE h`).
/// Either way the parent owns an `ERROR` child after the type slot, a shape a
/// well-formed `parameter_declaration` / `declaration` whose type field is a
/// `type_identifier` never produces — so suppressing the leading
/// `type_identifier` is sound.
///
/// Gated strictly:
///   * the `type_identifier` must be a direct child of `declaration` or
///     `parameter_declaration` (never inside a `template_argument_list`,
///     `cast_expression`, or `type_descriptor`),
///   * it must occupy the `type` field,
///   * the demotion ERROR carrying an identifier must sit either among the type
///     token's own siblings (`_Inout_ int* p`, `EXTERN_C HRESULT __stdcall`) or,
///     for a parameter, as a sibling of the whole `parameter_declaration` inside
///     the `parameter_list` (`_In_ HANDLE h`, where the macro stole the type
///     slot, the real type was bound as the declarator, and the trailing
///     parameter name spilled into the ERROR).
pub(super) fn is_qualifier_macro_position(node: &Node) -> bool {
    if node.kind() != "type_identifier" {
        return false;
    }
    is_qualifier_type_slot(node)
}

/// Shared gate for the qualifier-macro fingerprint over a type-slot node (a
/// `type_identifier` or `macro_type_specifier`): the node occupies the `type`
/// field of a `declaration` / `parameter_declaration`, and a demoted-identifier
/// ERROR proves a qualifier macro stole the slot.
fn is_qualifier_type_slot(node: &Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if !matches!(parent.kind(), "declaration" | "parameter_declaration") {
        return false;
    }
    // Must be the type-field token, not a declarator or trailing identifier.
    let in_type_field = parent
        .child_by_field_name("type")
        .is_some_and(|t| t.id() == node.id());
    if !in_type_field {
        return false;
    }
    // The demotion ERROR sits among the type token's own siblings…
    if has_demoted_error_sibling(node) {
        return true;
    }
    // …or, for a parameter whose real type was bound as the declarator, the
    // trailing parameter name spilled into an ERROR sibling of the whole
    // `parameter_declaration` inside the enclosing `parameter_list`.
    if parent.kind() == "parameter_declaration" {
        return has_demoted_error_sibling(&parent);
    }
    false
}

/// True when a named sibling after `node` is an `ERROR` wrapping an identifier —
/// the surplus token (a demoted real type or trailing parameter name) the parser
/// could not place once a qualifier macro stole the type slot. A non-ERROR
/// sibling (a `pointer_declarator`, a declarator `identifier`) does not
/// disqualify; the ERROR may sit past it.
///
/// The ERROR must wrap a demoted type/identifier token — the shape
/// `ERROR(identifier)` / `ERROR(type_identifier)`. An empty ERROR or one holding
/// only punctuation (`ERROR(} })`) is not this fingerprint, so a real type in a
/// declaration with unrelated trailing garbage is not suppressed.
fn has_demoted_error_sibling(node: &Node) -> bool {
    let mut sib = node.next_sibling();
    while let Some(s) = sib {
        if s.is_named() && s.kind() == "ERROR" {
            let mut ec = s.walk();
            if s.children(&mut ec)
                .any(|c| matches!(c.kind(), "identifier" | "type_identifier"))
            {
                return true;
            }
        }
        sib = s.next_sibling();
    }
    false
}

/// True when `node` is a `macro_type_specifier` (`_In_reads_(n)`, `_Out_writes_(c)`)
/// sitting in the type slot of a `declaration` / `parameter_declaration` whose
/// real type was demoted to an ERROR sibling — the SAL-annotation-with-argument
/// fingerprint. A SAL annotation parses as `macro_type_specifier { name (...) }`
/// where the parenthesised count expression is captured as a `type_descriptor`;
/// recursing in would emit that count token (`n`) as a bogus TypeRef. The
/// ERROR-sibling demotion proves the specifier is an annotation that stole the
/// type slot, not a genuine type-producing macro.
pub(super) fn is_annotation_macro_specifier(node: &Node) -> bool {
    if node.kind() != "macro_type_specifier" {
        return false;
    }
    is_qualifier_type_slot(node)
}

// ---------------------------------------------------------------------------
// Parameter type ref emission
// ---------------------------------------------------------------------------

/// Walk a function_definition's declarator chain to find parameter_list,
/// then emit TypeRef for each parameter's type_identifier.
pub(super) fn emit_param_type_refs(
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
                        // A qualifier macro in the parameter's leading slot
                        // (`pTHX_ SV* sv`, `_In_ HANDLE h`) demotes the real type
                        // to an ERROR sibling; a SAL annotation with a count
                        // argument (`_In_reads_(n)`) parses as a
                        // `macro_type_specifier` over the same fingerprint.
                        // Suppress the macro, not the real type.
                        if is_qualifier_macro_position(&type_node)
                            || is_annotation_macro_specifier(&type_node)
                        {
                            continue;
                        }
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
///
/// Calling-convention / export-qualifier macros that tree-sitter parses as a
/// leading type token are dropped by a catalog-driven `retain` in the caller,
/// after this sweep — see `extract_with_file`.
pub(super) fn sweep_typerefs<'a>(
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
                    && !predicates::is_c_primitive_type(&name)
                    && !predicates::is_c_compiler_intrinsic(&name)
                    && !predicates::is_template_param(&name)
                    && !is_cpp_keyword(&name)
                    && !is_qualifier_macro_position(&child)
                {
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: default_sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
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
            "macro_type_specifier" if is_annotation_macro_specifier(&child) => {
                // A SAL annotation with a count argument (`_In_reads_(n)`) that
                // stole the type slot. Its parenthesised count expression is
                // captured as a `type_descriptor`; recursing in would emit the
                // count token as a bogus TypeRef. Skip the subtree entirely.
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
                                    is_include: false,
                                    is_import_binding: false,
                                    is_reexport: false,
                                    source_symbol_index: default_sym_idx,
                                    target_name: name,
                                    kind: EdgeKind::Inherits,
                                    line: base.start_position().row as u32,
                                    col: 0,
                                    module: None,
                                    chain: None,
                                    byte_offset: base.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
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
                                            is_include: false,
                                            is_import_binding: false,
                                            is_reexport: false,
                                            source_symbol_index: default_sym_idx,
                                            target_name: name,
                                            kind: EdgeKind::Inherits,
                                            line: inner.start_position().row as u32,
                                            col: 0,
                                            module: None,
                                            chain: None,
                                            byte_offset: inner.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
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
            "string_literal"
            | "comment"
            | "number_literal"
            | "char_literal"
            | "concatenated_string" => {}
            _ => {
                sweep_typerefs(child, src, default_sym_idx, language, refs);
            }
        }
    }
}
