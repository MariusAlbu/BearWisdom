// =============================================================================
// c_lang/symbols.rs  —  Symbol pushers for C/C++
// =============================================================================

use super::helpers::{
    detect_visibility, enclosing_scope, extract_doc_comment, extract_declarator_name,
    find_child_by_kind, first_type_identifier, is_constructor_name, node_text,
};
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Helpers — type reference emission
// ---------------------------------------------------------------------------

/// Emit a single TypeRef edge from `source_idx` to the type named by `name_node`.
fn push_typeref(name_node: Node, src: &[u8], source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    let name = node_text(name_node, src);
    if name.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: name,
        kind: EdgeKind::TypeRef,
        line: name_node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

/// Walk a `type_descriptor` (or any node) and emit TypeRef for every
/// `type_identifier` found.  Stops at leaf nodes — does not recurse into
/// sub-expressions to avoid false positives.
pub(super) fn emit_typerefs_for_type_descriptor(
    node: Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "type_identifier" => {
            push_typeref(node, src, source_idx, refs);
        }
        "primitive_type" | "auto" | "void" => {
            // primitives — skip
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                emit_typerefs_for_type_descriptor(child, src, source_idx, refs);
            }
        }
    }
}

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
        node_text(name_node, src)
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

pub(super) fn push_typedef(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "pointer_declarator" | "function_declarator" => {
                let name = first_type_identifier(&child, src);
                if let Some(name) = name {
                    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
                    let qualified_name = scope_tree::qualify(&name, scope);
                    let scope_path = scope_tree::scope_path(scope);

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
                }
                return;
            }
            _ => {}
        }
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
                });
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// template_declaration — C++ `template<typename T> class/struct/fn { ... }`
// ---------------------------------------------------------------------------

/// Returns the inner declaration node (class/struct/function/etc) and its
/// optional symbol index after pushing it.  The caller is responsible for
/// recursing into the body.
///
/// We emit one TypeRef per type-parameter constraint when present (e.g.
/// `template<typename T, typename U = int>` → TypeRef to `int`).
pub(super) fn push_template_decl<'a>(
    node: &'a Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    language: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> (Option<usize>, Option<Node<'a>>) {
    // The inner declaration is the last named child that is not the template
    // parameter list.
    let mut inner: Option<Node<'a>> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "template_parameter_list" => {
                // emit TypeRef for default type arguments  e.g. `typename T = Foo`
                emit_template_param_typerefs(&child, src, symbols.len(), refs);
            }
            "class_specifier" | "struct_specifier" | "union_specifier"
            | "function_definition" | "alias_declaration" | "declaration"
            | "concept_definition" => {
                inner = Some(child);
            }
            _ => {}
        }
    }

    let inner_node = match inner {
        Some(n) => n,
        None => return (None, None),
    };

    // Push a symbol for the inner declaration.
    let idx = match inner_node.kind() {
        "class_specifier" => {
            push_specifier(&inner_node, src, scope_tree, SymbolKind::Class, symbols, parent_index)
        }
        "struct_specifier" => {
            push_specifier(&inner_node, src, scope_tree, SymbolKind::Struct, symbols, parent_index)
        }
        "union_specifier" => {
            push_specifier(&inner_node, src, scope_tree, SymbolKind::Struct, symbols, parent_index)
        }
        "function_definition" => {
            push_function_def(&inner_node, src, scope_tree, language, symbols, parent_index)
        }
        "concept_definition" => {
            push_concept_def(&inner_node, src, scope_tree, symbols, parent_index)
        }
        _ => None,
    };

    (idx, Some(inner_node))
}

// ---------------------------------------------------------------------------
// concept_definition — C++20 `template<typename T> concept Foo = expr;`
// ---------------------------------------------------------------------------

pub(super) fn push_concept_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // concept_definition: `concept` `identifier` `=` expression
    let name_node = if let Some(n) = node.child_by_field_name("name") {
        n
    } else {
        let mut cursor = node.walk();
        let found = node.children(&mut cursor).find(|c| c.kind() == "identifier");
        found?
    };

    let name = node_text(name_node, src);
    if name.is_empty() {
        return None;
    }

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias, // concepts are type-constraint aliases
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("concept {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    Some(idx)
}

fn emit_template_param_typerefs(
    param_list: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = param_list.walk();
    for child in param_list.children(&mut cursor) {
        // `optional_type_parameter_declaration` is the node kind for
        // `typename T = SomeType` — it has a default type after `=`.
        // `type_parameter_declaration` is the plain `typename T` variant (no default).
        if child.kind() == "optional_type_parameter_declaration" {
            // Walk children after `=` and emit TypeRef for any named type.
            let mut after_eq = false;
            let mut ic = child.walk();
            for inner in child.children(&mut ic) {
                if inner.kind() == "=" {
                    after_eq = true;
                } else if after_eq {
                    // Could be `type_identifier`, `template_type`, `qualified_identifier`, etc.
                    emit_typerefs_for_type_descriptor(inner, src, source_idx, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// alias_declaration — C++ `using Alias = Type;`
// ---------------------------------------------------------------------------

pub(super) fn push_alias_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Structure: `using` <name> `=` `type_descriptor` `;`
    // The name can be: type_identifier, template_type (e.g. `using Foo<T> = Bar`)
    // or qualified_identifier. Skip non-identifier second children.
    let name_node = match node.child(1) {
        Some(n) if matches!(
            n.kind(),
            "type_identifier" | "identifier" | "template_type" | "qualified_identifier"
        ) => n,
        _ => return,
    };
    let name = node_text(name_node, src);
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("using {name} = ...")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // TypeRef for the aliased type.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_descriptor" {
            emit_typerefs_for_type_descriptor(child, src, idx, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// using_declaration — C++ `using std::vector;`  (namespace using, no `=`)
// ---------------------------------------------------------------------------

pub(super) fn push_using_decl(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The identifier after `using` is a qualified_identifier or identifier.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "qualified_identifier" | "identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// preproc_def — `#define FOO value`  → Constant/Variable
// ---------------------------------------------------------------------------

pub(super) fn push_preproc_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Children: `#define`, `identifier`, optional `preproc_arg`
    let name_node = match node.child(1) {
        Some(n) if n.kind() == "identifier" => n,
        _ => return,
    };
    let name = node_text(name_node, src);
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let value = node
        .child(2)
        .filter(|n| n.kind() == "preproc_arg")
        .map(|n| node_text(n, src));
    let signature = Some(match &value {
        Some(v) => format!("#define {name} {v}"),
        None => format!("#define {name}"),
    });

    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// preproc_function_def — `#define MAX(a, b) ...`  → Function
// ---------------------------------------------------------------------------

pub(super) fn push_preproc_function_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Children: `#define`, `identifier`, `preproc_params`, optional `preproc_arg`
    let name_node = match node.child(1) {
        Some(n) if n.kind() == "identifier" => n,
        _ => return,
    };
    let name = node_text(name_node, src);
    let params = node
        .child(2)
        .filter(|n| n.kind() == "preproc_params")
        .map(|n| node_text(n, src))
        .unwrap_or_default();

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("#define {name}{params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}

pub(super) fn extract_bases(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "base_class_clause" {
            let mut bc = child.walk();
            for base in child.children(&mut bc) {
                match base.kind() {
                    "type_identifier" => {
                        let name = node_text(base, src);
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: name,
                            kind: EdgeKind::Inherits,
                            line: base.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                    "base_class_specifier" => {
                        let mut ic = base.walk();
                        for inner in base.children(&mut ic) {
                            if inner.kind() == "type_identifier" {
                                let name = node_text(inner, src);
                                refs.push(ExtractedRef {
                                    source_symbol_index: source_idx,
                                    target_name: name,
                                    kind: EdgeKind::Inherits,
                                    line: inner.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
