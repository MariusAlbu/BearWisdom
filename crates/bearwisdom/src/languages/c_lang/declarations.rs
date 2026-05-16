// =============================================================================
// c_lang/declarations.rs  —  Symbol pushers for functions, classes, namespaces,
// typedefs, declarations, enums, and include directives
// =============================================================================

use super::helpers::{
    detect_visibility, enclosing_scope, extract_doc_comment, extract_declarator_name,
    find_child_by_kind, first_type_identifier, is_constructor_name, node_text,
};
use super::typerefs::{
    emit_typerefs_for_type_descriptor, find_real_specifier_name, looks_like_attribute_macro,
};
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(super) fn push_function_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    language: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let decl_node = node.child_by_field_name("declarator")?;
    let (name, is_destructor) = extract_declarator_name(&decl_node, src);
    let name = name?;

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kind = if is_destructor {
        SymbolKind::Method
    } else if language != "c" && is_constructor_name(&name, scope) {
        SymbolKind::Constructor
    } else if scope.is_some() {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let visibility = detect_visibility(node, src);
    let ret_type = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();
    let params = decl_node
        .child_by_field_name("parameters")
        .or_else(|| find_child_by_kind(&decl_node, "parameter_list"))
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let signature = Some(format!("{ret_type} {name}{params}").trim().to_string());

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_specifier(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name = if let Some(name_node) = node.child_by_field_name("name") {
        let raw = node_text(name_node, src);
        // Tree-sitter-cpp doesn't expand macros, so a header pattern like
        // `class Q_WIDGETS_EXPORT QMessageBox : public QDialog` parses
        // with `Q_WIDGETS_EXPORT` bound to the `name` field and the real
        // class name as a sibling type_identifier. The same holds for
        // every Qt module export macro, every dllexport-style attribute
        // macro, and any project-defined visibility macro. Detect the
        // SCREAMING_SNAKE_CASE shape and look one level deeper for the
        // real identifier — purely structural, no macro name list.
        if looks_like_attribute_macro(&raw) {
            find_real_specifier_name(node, src).unwrap_or(raw)
        } else {
            raw
        }
    } else {
        // Anonymous struct/union/enum — emit with a synthetic name so the
        // coverage engine can match this node.
        let kw = match kind {
            SymbolKind::Class  => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum   => "enum",
            _                  => "struct",
        };
        format!("__anon_{kw}_{}", node.start_position().row)
    };

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kw = match kind {
        SymbolKind::Class  => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum   => "enum",
        _                  => "struct",
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{kw} {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_namespace(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("namespace {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// C++ `namespace alias = target;`. Tree-sitter exposes the alias under
/// the `name` field and the target as a sibling subtree containing one or
/// more `namespace_identifier` nodes (single-segment for `namespace Dc =
/// DeriveColors;`, multiple for nested forms like `namespace fs =
/// std::filesystem;`).
///
/// Emit a Namespace symbol for the alias so resolvers find it under
/// `same-file` lookup; emit TypeRef refs for each target identifier so
/// the alias→target relationship is preserved in the graph. Resolution of
/// `alias::member` to `target::member` is a follow-up — for now the
/// alias's own ref load (the largest single bucket on KeePassXC's
/// Phantom-style code) goes from `unresolved` to `resolved-same-file`,
/// and the target identifiers themselves get tracked refs.
pub(super) fn push_namespace_alias(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let Some(name_node) = node.child_by_field_name("name") else { return };
    let name = node_text(name_node, src);
    if name.is_empty() { return }

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("namespace {name} = ...")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // Emit TypeRef for each `namespace_identifier` after the `=`. The first
    // child past the `=` token is the target; nested namespace targets
    // surface multiple identifiers we want to track.
    let mut cursor = node.walk();
    let mut past_equals = false;
    for child in node.children(&mut cursor) {
        if !past_equals {
            if child.kind() == "=" { past_equals = true; }
            continue;
        }
        emit_namespace_target_refs(&child, src, idx, refs);
    }
}

fn emit_namespace_target_refs(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if matches!(node.kind(), "namespace_identifier" | "type_identifier") {
        let name = node_text(*node, src);
        if !name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
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
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        emit_namespace_target_refs(&child, src, source_idx, refs);
    }
}

pub(super) fn push_typedef(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // For C/C++ `typedef <source_type> <new_name>;`, the grammar lays out
    // children left-to-right as:
    //   typedef  <source_type_node>  <declarator>  ;
    //
    // The declarator (the LAST eligible child, not the first) is the new
    // type alias being introduced.  However, C allows multiple declarators
    // in a single typedef, e.g.:
    //
    //   typedef struct { ... } TypeA, TypeB;
    //   typedef unsigned long size_t, ULONG_PTR;
    //
    // In this case there are TWO names and both should become TypeAlias
    // symbols.  The grammar represents them as two separate `type_identifier`
    // children after the struct/union/enum body.
    //
    // Strategy:
    //   1. Find all eligible declarator children (type_identifier,
    //      pointer_declarator, function_declarator).
    //   2. The FIRST one is potentially the source type (e.g. `HttpRequestPtr`
    //      in `typedef HttpRequestPtr Request;`) — only emit it if there is
    //      no second declarator (it IS the alias in that case).
    //   3. Always emit the LAST declarator as a TypeAlias.
    //   4. If there are more than two declarators, emit all but the first
    //      as TypeAlias symbols (the first is the source type).
    //
    // In practice this means:
    //   typedef T Foo;          → 2 children → emit last (Foo)
    //   typedef struct{} A, B;  → struct body + 2 type_identifiers → both A and B
    let mut cursor = node.walk();
    let mut declarators: Vec<Node> = Vec::new();
    // Check whether there is a struct/union/enum/class body child (anonymous
    // inline specifier).  If yes, ALL subsequent type_identifier nodes are new
    // aliases.  If no, only the LAST one is the new alias.
    let mut has_specifier_body = false;
    // Capture trailing ERROR's identifier when tree-sitter recovers from an
    // unknown macro inside the typedef. Real shapes hit:
    //   typedef __u32 __bitwise __le32;
    //     → [type_id __u32, type_id __bitwise, ERROR(__le32)] — ERROR holds the alias.
    //   typedef __u32 __attribute__((bitwise)) __le32;
    //     → [type_id __u32, function_declarator __attribute__, ERROR(__le32)].
    // Without this, the alias name `__le32` was silently dropped and 5K
    // call/type_ref edges in zig-compiler-fresh's vendored Linux types.h
    // could never resolve.
    let mut trailing_error_name: Option<String> = None;
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Inline specifier with body → all following identifiers are aliases
            "struct_specifier" | "union_specifier" | "enum_specifier" | "class_specifier" => {
                if child.child_by_field_name("body").is_some() {
                    has_specifier_body = true;
                }
                // Reset error tracking — the body legitimately captures its own state.
                trailing_error_name = None;
            }
            // Declarator variants that introduce a new alias name. The
            // canonical shape is `type_identifier`; `pointer_declarator`
            // wraps `typedef X *Y` (the alias name `Y` is inside);
            // `function_declarator` wraps `typedef X (*Fn)(Args)`.
            // **`array_declarator`** wraps `typedef X Y[N]` — common for
            // GMP-compatible types (`typedef bf_t mpz_t[1];`) and zlib
            // typedefs that swipl bundles. **`parenthesized_declarator`**
            // wraps GCC-attribute / `(*name)(...)` shapes that nest
            // around the real declarator.
            "type_identifier"
            | "pointer_declarator"
            | "function_declarator"
            | "array_declarator"
            | "parenthesized_declarator" => {
                declarators.push(child);
                trailing_error_name = None;
            }
            "ERROR" => {
                // Take the single identifier from inside the ERROR if there is one.
                let mut ec = child.walk();
                let idents: Vec<_> = child
                    .children(&mut ec)
                    .filter(|n| n.kind() == "identifier" || n.kind() == "type_identifier")
                    .collect();
                if idents.len() == 1 {
                    trailing_error_name = Some(node_text(idents[0], src));
                }
            }
            _ => {}
        }
    }

    // If the last meaningful node was an ERROR with one identifier, it almost
    // certainly holds the alias name — promote it as the last "declarator" and
    // skip the parsed-but-bogus declarators above it.
    if let Some(name) = trailing_error_name {
        let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
        let scope_path = scope_tree::scope_path(scope);
        let qualified_name = scope_tree::qualify(&name, scope);
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::TypeAlias,
            visibility: None,
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: node.end_position().column as u32,
            signature: Some(format!("typedef {name}")),
            doc_comment: extract_doc_comment(node, src),
            scope_path,
            parent_index,
        });
        return;
    }

    if declarators.is_empty() {
        return;
    }

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let scope_path = scope_tree::scope_path(scope);
    let doc = extract_doc_comment(node, src);

    // Determine which declarators represent new alias names.
    // - If there is an inline specifier body (e.g. `typedef struct{...} A, B`)
    //   ALL declarators in the list are alias names.
    // - Otherwise, only the last one is the alias (the earlier ones are the
    //   source type chain, e.g. `typedef const unsigned long * Foo`).
    let aliases_start = if has_specifier_body { 0 } else { declarators.len().saturating_sub(1) };

    for decl in &declarators[aliases_start..] {
        let Some(name) = first_type_identifier(decl, src) else { continue; };
        let qualified_name = scope_tree::qualify(&name, scope);
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::TypeAlias,
            visibility: None,
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: node.end_position().column as u32,
            signature: Some(format!("typedef {name}")),
            doc_comment: doc.clone(),
            scope_path: scope_path.clone(),
            parent_index,
        });
    }
}

/// Returns true if `node` is or contains a `function_declarator` child,
/// indicating this declarator represents a function forward declaration.
fn has_function_declarator(node: &Node) -> bool {
    if node.kind() == "function_declarator" {
        return true;
    }
    // pointer_declarator and parenthesized_declarator can wrap a function_declarator,
    // e.g. `(*fp)(int)` or `virtual int area() = 0` which becomes
    // `pointer_declarator` → `function_declarator`.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if has_function_declarator(&child) {
            return true;
        }
    }
    false
}

pub(super) fn push_declaration(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let type_str = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let scope_path = scope_tree::scope_path(scope);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let name_opt = match child.kind() {
            // `identifier` — plain declarations and non-struct members
            // `field_identifier` — struct/union member names in C grammar
            "identifier" | "field_identifier" => Some(node_text(child, src)),
            // Declarator variants that wrap an identifier
            "init_declarator" | "pointer_declarator" | "reference_declarator"
            | "array_declarator" | "parenthesized_declarator"
            | "function_declarator" | "abstract_function_declarator" => {
                first_type_identifier(&child, src)
            }
            // C++17 structured bindings: `auto [a, b] = expr;`
            "structured_binding_declarator" => first_type_identifier(&child, src),
            _ => None,
        };
        if let Some(name) = name_opt {
            let qualified_name = scope_tree::qualify(&name, scope);
            // Forward declarations whose declarator is (or contains) a
            // function_declarator represent function/method signatures, not variables.
            let kind = if has_function_declarator(&child) {
                if scope.is_some() {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                }
            } else {
                SymbolKind::Variable
            };
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name,
                kind,
                visibility: detect_visibility(node, src),
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: Some(format!("{type_str} {name}")),
                doc_comment: None,
                scope_path: scope_path.clone(),
                parent_index,
            });
        }
    }
}

pub(super) fn extract_enum_body(
    body: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let enum_qname = parent_index
        .and_then(|i| symbols.get(i))
        .map(|s| s.qualified_name.clone())
        .unwrap_or_default();

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "enumerator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, src);
                let qualified_name = if enum_qname.is_empty() {
                    name.clone()
                } else {
                    format!("{enum_qname}.{name}")
                };
                let scope = enclosing_scope(scope_tree, child.start_byte(), child.end_byte());
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
                    doc_comment: None,
                    scope_path: scope_tree::scope_path(scope),
                    parent_index,
                });
            }
        }
    }
}

pub(super) fn push_include(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "string_literal" | "system_lib_string" => {
                let raw = node_text(child, src);
                let path = raw.trim_matches('"').trim_matches('<').trim_matches('>');
                let target_name = path.rsplit('/').next().unwrap_or(path).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name,
                    kind: EdgeKind::Imports,
                    line: node.start_position().row as u32,
                    module: Some(path.to_string()),
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
                return;
            }
            _ => {}
        }
    }
}
