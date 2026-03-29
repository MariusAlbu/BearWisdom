// =============================================================================
// parser/extractors/typescript.rs  —  TypeScript / TSX extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class, Interface, Function (top-level), Method, Constructor, Property,
//   TypeAlias, Variable (const/let/var), Enum, EnumMember
//
// REFERENCES:
//   - `import` statements  → import records (module + named bindings)
//   - `call_expression`    → Calls edges
//   - `extends` / `implements` → Inherits / Implements edges
//   - `fetch(url)` / `axios.{get,post,put,delete}(url)` → candidates for
//     HTTP connector (stored as Calls with target = "fetch" | "axios.get" etc.)
//
// Approach:
//   Same two-pass approach as C#:
//   1. Build scope tree to get qualified names.
//   2. Walk CST to extract symbols and edges.
//
// Note on TSX:
//   TSX files use a slightly different grammar but the symbol node kinds are
//   identical.  The caller passes `is_tsx = true` to select the right grammar.
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind, ScopeTree};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration for TypeScript
// ---------------------------------------------------------------------------

static TS_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration",    name_field: "name" },
    ScopeKind { node_kind: "interface_declaration", name_field: "name" },
    ScopeKind { node_kind: "function_declaration",  name_field: "name" },
    // Arrow functions don't have a `name` field — handled separately.
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub struct TypeScriptExtraction {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub has_errors: bool,
}

/// Extract symbols and references from TypeScript or TSX source.
pub fn extract(source: &str, is_tsx: bool) -> TypeScriptExtraction {
    let language: tree_sitter::Language = if is_tsx {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load TypeScript grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return TypeScriptExtraction {
                symbols: vec![],
                refs: vec![],
                has_errors: true,
            }
        }
    };

    let has_errors = tree.root_node().has_error();
    let src_bytes = source.as_bytes();
    let root = tree.root_node();

    let scope_tree = scope_tree::build(root, src_bytes, TS_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(root, src_bytes, &scope_tree, &mut symbols, &mut refs, None);

    TypeScriptExtraction { symbols, refs, has_errors }
}

// ---------------------------------------------------------------------------
// Recursive visitor
// ---------------------------------------------------------------------------

fn extract_node(
    node: Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" => {
                let idx = push_class(&child, src, scope_tree, symbols, parent_index);
                // Heritage clause (extends / implements).
                extract_heritage(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, idx);
                }
            }

            "interface_declaration" => {
                let idx = push_interface(&child, src, scope_tree, symbols, parent_index);
                extract_heritage(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, idx);
                }
            }

            "function_declaration" => {
                let idx = push_function(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_param_and_return_types(&child, src, sym_idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls(&body, src, sym_idx, refs);
                    }
                }
            }

            "export_statement" => {
                // `export class Foo {}` / `export function bar() {}`
                // Recurse — the declaration itself is a child node.
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            "method_definition" => {
                let idx = push_method(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    // Constructor parameter properties:
                    // `constructor(private db: DatabaseRepository)` creates a class property.
                    if symbols[sym_idx].kind == SymbolKind::Constructor {
                        extract_constructor_params(
                            &child, src, scope_tree, symbols, refs, parent_index,
                        );
                    }
                    // Parameter types and return type for all methods.
                    extract_param_and_return_types(&child, src, sym_idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls(&body, src, sym_idx, refs);
                    }
                }
            }

            "public_field_definition" | "field_definition" => {
                push_ts_field(&child, src, scope_tree, symbols, refs, parent_index);
            }

            // Interface property signatures: `db: Database;`
            "property_signature" => {
                push_ts_field(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "type_alias_declaration" => {
                push_type_alias(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "enum_declaration" => {
                push_enum(&child, src, scope_tree, symbols, parent_index);
            }

            "lexical_declaration" | "variable_declaration" => {
                // `const Foo = ...` / `let bar = ...`
                push_variable_decl(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "import_statement" => {
                push_import(&child, src, symbols.len(), refs);
            }

            _ => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol pushers
// ---------------------------------------------------------------------------

fn push_class(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Class,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("class {name}")),
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_interface(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Interface,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("interface {name}")),
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_function(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("return_type")
        .map(|r| node_text(r, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("function {name}{params}{ret}").trim().to_string()),
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_method(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let kind = if name == "constructor" {
        SymbolKind::Constructor
    } else {
        SymbolKind::Method
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_ts_field(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Property,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path,
        parent_index,
    });

    // Extract TypeRef from field type annotation: `db: DatabaseRepository`
    if let Some(type_ann) = node.child_by_field_name("type") {
        extract_type_ref_from_annotation(&type_ann, src, idx, refs);
    }
}

/// Extract constructor parameter properties.
///
/// In TypeScript, `constructor(private db: DatabaseRepository)` is shorthand
/// for declaring a class property `db` of type `DatabaseRepository`.
/// Tree-sitter represents this as:
///
/// ```text
/// method_definition [constructor]
///   formal_parameters
///     required_parameter
///       accessibility_modifier  ← "private"/"public"/"protected"/"readonly"
///       identifier "db"
///       type_annotation
///         type_identifier "DatabaseRepository"
/// ```
///
/// For each such parameter, we emit:
/// 1. A property symbol (`AlbumService.db`)
/// 2. A TypeRef ref from the property to the type name
fn extract_constructor_params(
    method_node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let params = match method_node.child_by_field_name("parameters") {
        Some(p) => p,
        None => return,
    };

    let mut cursor = params.walk();
    for param in params.children(&mut cursor) {
        if param.kind() != "required_parameter" {
            continue;
        }

        // Check for accessibility modifier (private/public/protected/readonly).
        let has_modifier = param
            .children(&mut param.walk())
            .any(|c| c.kind() == "accessibility_modifier" || c.kind() == "readonly");
        if !has_modifier {
            continue;
        }

        // Get the parameter name.
        let name_node = match param.child_by_field_name("pattern") {
            Some(n) => n,
            None => continue,
        };
        let name = node_text(name_node, src);

        // Build qualified name relative to the class scope.
        let parent_scope = if method_node.start_byte() > 0 {
            scope_tree::find_scope_at(scope_tree, method_node.start_byte() - 1)
        } else {
            None
        };
        let qualified_name = scope_tree::qualify(&name, parent_scope);
        let scope_path = scope_tree::scope_path(parent_scope);

        let prop_idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::Property,
            visibility: detect_visibility(&param, src),
            start_line: param.start_position().row as u32,
            end_line: param.end_position().row as u32,
            start_col: param.start_position().column as u32,
            end_col: param.end_position().column as u32,
            signature: None,
            doc_comment: None,
            scope_path,
            parent_index,
        });

        // Extract TypeRef from the type annotation.
        if let Some(type_ann) = param.child_by_field_name("type") {
            extract_type_ref_from_annotation(&type_ann, src, prop_idx, refs);
        }
    }
}

/// Extract TypeRef edges for function/method parameter types and return type.
///
/// For `findAll(id: string, filter: FilterDto): Promise<Album[]>`, emits:
/// - TypeRef from findAll → FilterDto (parameter type)
/// - TypeRef from findAll → Promise (return type)
///
/// Skips primitive types (string, number, boolean, void, any, etc.) since they
/// don't reference user-defined symbols.
fn extract_param_and_return_types(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Parameter types.
    if let Some(params) = node.child_by_field_name("parameters") {
        for i in 0..params.child_count() {
            if let Some(param) = params.child(i) {
                if param.kind() == "required_parameter" || param.kind() == "optional_parameter" {
                    if let Some(type_ann) = param.child_by_field_name("type") {
                        extract_type_ref_from_annotation(&type_ann, src, source_symbol_index, refs);
                    }
                }
            }
        }
    }

    // Return type.
    if let Some(ret_type) = node.child_by_field_name("return_type") {
        extract_type_ref_from_annotation(&ret_type, src, source_symbol_index, refs);
    }
}

/// Extract a TypeRef from a type annotation node.
/// Handles simple types (`DatabaseRepository`), generic types (`Repository<User>`),
/// and qualified types (`db.Kysely`).
fn extract_type_ref_from_annotation(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Walk into type_annotation → the actual type node.
    // type_annotation children: ":" + the type itself
    let type_node = if node.kind() == "type_annotation" {
        let count = node.child_count();
        let mut found = None;
        for i in 0..count {
            if let Some(child) = node.child(i) {
                if child.kind() != ":" {
                    found = Some(child);
                    break;
                }
            }
        }
        found
    } else {
        Some(*node)
    };
    let Some(type_node) = type_node else { return };

    match type_node.kind() {
        "type_identifier" | "identifier" => {
            let type_name = node_text(type_node, src);
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
        "generic_type" => {
            // Repository<User> → extract "Repository"
            if let Some(name) = type_node.child_by_field_name("name") {
                let type_name = node_text(name, src);
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: type_name,
                    kind: EdgeKind::TypeRef,
                    line: type_node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
        "nested_type_identifier" | "member_expression" => {
            // db.Kysely → extract the full dotted name
            let type_name = node_text(type_node, src);
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
        _ => {}
    }
}

fn push_type_alias(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

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
        signature: Some(format!("type {name}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });

    // Extract TypeRef from type alias value: `type UserId = string`
    if let Some(value) = node.child_by_field_name("value") {
        extract_type_ref_from_annotation(&value, src, idx, refs);
    }
}

fn push_enum(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: qualified_name.clone(),
        kind: SymbolKind::Enum,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("enum {name}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });

    // Enum members.
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for member in body.children(&mut cursor) {
            if member.kind() == "enum_assignment" || member.kind() == "property_identifier" || member.kind() == "identifier" {
                let mname_node = if member.kind() == "enum_assignment" {
                    member.child_by_field_name("name")
                } else {
                    Some(member)
                };
                if let Some(mn) = mname_node {
                    let mname = node_text(mn, src);
                    symbols.push(ExtractedSymbol {
                        name: mname.clone(),
                        qualified_name: format!("{qualified_name}.{mname}"),
                        kind: SymbolKind::EnumMember,
                        visibility: None,
                        start_line: member.start_position().row as u32,
                        end_line: member.end_position().row as u32,
                        start_col: member.start_position().column as u32,
                        end_col: member.end_position().column as u32,
                        signature: None,
                        doc_comment: None,
                        scope_path: Some(qualified_name.clone()),
                        parent_index: Some(idx),
                    });
                }
            }
        }
    }
}

fn push_variable_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let scope_path = scope_tree::scope_path(parent_scope);

    // `const Foo = ...` — get declarators.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                // Only capture simple identifiers (not destructuring).
                if name_node.kind() == "identifier" {
                    let name = node_text(name_node, src);
                    let qualified_name = scope_tree::qualify(&name, parent_scope);
                    let idx = symbols.len();
                    symbols.push(ExtractedSymbol {
                        name: name.clone(),
                        qualified_name,
                        kind: SymbolKind::Variable,
                        visibility: detect_visibility(node, src),
                        start_line: child.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: child.start_position().column as u32,
                        end_col: child.end_position().column as u32,
                        signature: Some(format!("const {name}")),
                        doc_comment: None,
                        scope_path: scope_path.clone(),
                        parent_index,
                    });

                    // Extract TypeRef from variable type annotation: `const repo: Repository`
                    if let Some(type_ann) = child.child_by_field_name("type") {
                        extract_type_ref_from_annotation(&type_ann, src, idx, refs);
                    }
                }
            }
        }
    }
}

fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Import source: `import { X, Y } from './foo'` or `import Foo from 'bar'`
    let module_path = node
        .child_by_field_name("source")
        .map(|s| {
            node_text(s, src)
                .trim_matches('"')
                .trim_matches('\'')
                .to_string()
        });

    // Named imports: `{ X, Y as Z }` → push one ref per binding.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_clause" => {
                let mut ic = child.walk();
                for item in child.children(&mut ic) {
                    match item.kind() {
                        "identifier" => {
                            // Default import: `import Foo from ...`
                            refs.push(ExtractedRef {
                                source_symbol_index: current_symbol_count,
                                target_name: node_text(item, src),
                                kind: EdgeKind::TypeRef,
                                line: item.start_position().row as u32,
                                module: module_path.clone(),
                                chain: None,
                            });
                        }
                        "named_imports" => {
                            let mut ni = item.walk();
                            for spec in item.children(&mut ni) {
                                if spec.kind() == "import_specifier" {
                                    // `name` = imported name, `alias` = local alias.
                                    // We use the imported name for resolution.
                                    let imported_name = spec
                                        .child_by_field_name("name")
                                        .map(|n| node_text(n, src))
                                        .unwrap_or_else(|| node_text(spec, src));
                                    refs.push(ExtractedRef {
                                        source_symbol_index: current_symbol_count,
                                        target_name: imported_name,
                                        kind: EdgeKind::TypeRef,
                                        line: spec.start_position().row as u32,
                                        module: module_path.clone(),
                                        chain: None,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Heritage clause (extends / implements)
// ---------------------------------------------------------------------------

fn extract_heritage(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_heritage" => {
                let mut hc = child.walk();
                for clause in child.children(&mut hc) {
                    match clause.kind() {
                        "extends_clause" => {
                            let mut ec = clause.walk();
                            for type_node in clause.children(&mut ec) {
                                if type_node.kind() == "identifier" || type_node.kind() == "type_identifier" {
                                    refs.push(ExtractedRef {
                                        source_symbol_index: source_idx,
                                        target_name: node_text(type_node, src),
                                        kind: EdgeKind::Inherits,
                                        line: type_node.start_position().row as u32,
                                        module: None,
                                        chain: None,
                                    });
                                }
                            }
                        }
                        "implements_clause" => {
                            let mut ic = clause.walk();
                            for type_node in clause.children(&mut ic) {
                                if type_node.kind() == "type_identifier" || type_node.kind() == "identifier" {
                                    refs.push(ExtractedRef {
                                        source_symbol_index: source_idx,
                                        target_name: node_text(type_node, src),
                                        kind: EdgeKind::Implements,
                                        line: type_node.start_position().row as u32,
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
            "extends_clause" => {
                // Direct child for interfaces.
                let mut ec = child.walk();
                for type_node in child.children(&mut ec) {
                    if type_node.kind() == "identifier" || type_node.kind() == "type_identifier" {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: node_text(type_node, src),
                            kind: EdgeKind::Inherits,
                            line: type_node.start_position().row as u32,
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

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

fn extract_calls(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(func_node) = child.child_by_field_name("function") {
                let name = callee_name(func_node, src);
                if !name.is_empty() && name != "undefined" {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: func_node.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
        }
        extract_calls(&child, src, source_symbol_index, refs);
    }
}

fn callee_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "member_expression" => {
            // `foo.bar` — extract the full dotted name for API matching.
            let obj = node
                .child_by_field_name("object")
                .map(|n| node_text(n, src))
                .unwrap_or_default();
            let prop = node
                .child_by_field_name("property")
                .map(|n| node_text(n, src))
                .unwrap_or_default();
            if obj.is_empty() || prop.is_empty() {
                node_text(node, src)
            } else {
                format!("{obj}.{prop}")
            }
        }
        _ => {
            let t = node_text(node, src);
            t.rsplit('.').next().unwrap_or(&t).to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // TypeScript accessibility modifiers are direct children.
            "accessibility_modifier" => {
                let text = node_text(child, src);
                match text.as_str() {
                    "public" => return Some(Visibility::Public),
                    "private" => return Some(Visibility::Private),
                    "protected" => return Some(Visibility::Protected),
                    _ => {}
                }
            }
            "export" => return Some(Visibility::Public),
            _ => {}
        }
    }
    None
}

/// Collect a JSDoc comment immediately before `node`.
fn extract_jsdoc(node: &Node, src: &[u8]) -> Option<String> {
    let sib = node.prev_sibling()?;
    if sib.kind() == "comment" {
        let text = node_text(sib, src);
        if text.starts_with("/**") {
            return Some(text);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "typescript_tests.rs"]
mod tests;
