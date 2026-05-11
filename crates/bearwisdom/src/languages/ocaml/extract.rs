// =============================================================================
// languages/ocaml/extract.rs — OCaml extractor (tree-sitter-based)
//
// SYMBOLS:
//   Function  — `value_definition` whose `let_binding.pattern` is a `value_name`
//               and whose body contains a `fun_expression` or has parameters
//             — `value_specification` (in .mli / sig bodies)
//             — `external` (C FFI binding)
//   Variable  — `value_definition` (simple binding without params)
//   TypeAlias — `type_definition` where `type_binding.synonym` is set
//   Enum      — `type_definition` with variant constructors (variant_declaration)
//   Struct    — `type_definition` with record (record_declaration)
//             — `exception_definition`
//   Namespace — `module_definition`
//   Interface — `module_type_definition`
//   Class     — `class_definition`
//
// REFERENCES:
//   Imports      — `open_module` → module field
//   Calls        — `application_expression` → function field
//   Inherits     — `inheritance_definition` → class field
//   Instantiates — `new_expression` → class_path
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};
use tree_sitter::{Node, Parser};

/// Build the qualified name for a child symbol by prefixing the parent's qname.
/// OCaml's `module M = struct ... end` introduces a Namespace symbol whose
/// qname is the module path; descendants must inherit that prefix.
fn qualify_with_parent(name: &str, parent_idx: Option<usize>, symbols: &[ExtractedSymbol]) -> String {
    match parent_idx.and_then(|i| symbols.get(i)) {
        Some(parent) => format!("{}.{}", parent.qualified_name, name),
        None => name.to_string(),
    }
}

/// Build the scope_path string from the parent's qualified_name. None when the
/// symbol is at file top level.
fn scope_path_from_parent(parent_idx: Option<usize>, symbols: &[ExtractedSymbol]) -> Option<String> {
    parent_idx.and_then(|i| symbols.get(i)).map(|p| p.qualified_name.clone())
}

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    // Use interface grammar for .mli files
    let is_interface = file_path.ends_with(".mli");
    let lang = if is_interface {
        tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into()
    } else {
        tree_sitter_ocaml::LANGUAGE_OCAML.into()
    };

    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return ExtractionResult::empty();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::empty(),
    };

    let src = source.as_bytes();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    walk_node(tree.root_node(), src, &mut symbols, &mut refs, None, None);

    ExtractionResult::new(symbols, refs, tree.root_node().has_error())
}

/// `local_open_ctx` carries the opened module name when the current node is
/// inside a `local_open_expression` body. All `application_expression` nodes
/// within that body use it as their module qualifier so `Fmt.(any ",")` emits
/// `any` with `module=Some("Fmt")` rather than bare.
fn walk_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    local_open_ctx: Option<&str>,
) {
    match node.kind() {
        "value_definition" => {
            let idx = extract_value_def(node, src, symbols, parent_idx);
            walk_children(node, src, symbols, refs, idx.or(parent_idx), local_open_ctx);
        }
        "type_definition" => {
            let idx = extract_type_def(node, src, symbols, parent_idx);
            walk_children(node, src, symbols, refs, idx.or(parent_idx), local_open_ctx);
        }
        "module_definition" => {
            let idx = extract_module_def(node, src, symbols, parent_idx);
            walk_children(node, src, symbols, refs, idx.or(parent_idx), local_open_ctx);
        }
        "open_module" => {
            if let Some(mod_node) = node.child_by_field_name("module") {
                let name = text(mod_node, src);
                if !name.is_empty() {
                    // Emit a symbol so coverage can match the open_module node kind.
                    let sym_idx = symbols.len();
                    symbols.push(ExtractedSymbol {
                        qualified_name: name.clone(),
                        name: name.clone(),
                        kind: SymbolKind::Variable,
                        visibility: Some(Visibility::Public),
                        start_line: node.start_position().row as u32,
                        end_line: node.end_position().row as u32,
                        start_col: 0,
                        end_col: 0,
                        signature: Some(format!("open {name}")),
                        doc_comment: None,
                        scope_path: None,
                        parent_index: parent_idx,
                    });
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::Imports,
                        line: node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                        namespace_segments: Vec::new(),
                    });
                }
            }
        }
        "application_expression" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // `function` field is the callee
            if let Some(fn_node) = node.child_by_field_name("function") {
                // `Module.(expr)` local-open is not a callable symbol name —
                // the inner expression resolves separately when walked below.
                if fn_node.kind() != "local_open_expression" {
                    let (target_name, extracted_module) = match fn_node.kind() {
                        "value_path" => split_value_path(fn_node, src),
                        // `constructor_path` covers qualified constructors like
                        // `Command.Args.S` — split into (S, Some("Command.Args"))
                        // so the module-qualified resolver step can find them.
                        "constructor_path" => split_constructor_path(fn_node, src),
                        _ => (text(fn_node, src), None),
                    };
                    // Skip polymorphic variant constructors (`Ok, `Error, `P, etc.),
                    // names with newlines (multi-line expressions, not real callees),
                    // and names containing spaces or brackets — those come from
                    // attribute-annotated calls like `(aux [@tailcall])` which are
                    // not direct symbol references.
                    if !target_name.is_empty()
                        && !target_name.starts_with('`')
                        && !target_name.contains('\n')
                        && !target_name.contains(' ')
                        && !target_name.contains('[')
                    {
                        // Prefer the explicitly extracted module; fall back to
                        // the inherited local_open context when no qualifier was
                        // present in the source text.
                        let module = extracted_module
                            .or_else(|| local_open_ctx.map(|m| m.to_string()));
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: node.start_position().row as u32,
                            module,
                            chain: None,
                            byte_offset: 0,
                            namespace_segments: Vec::new(),
                        });
                    }
                }
            }
            walk_children(node, src, symbols, refs, parent_idx, local_open_ctx);
        }
        "local_open_expression" => {
            // Propagate the opened module name into all child nodes so every
            // `application_expression` within the body emits a qualified Calls
            // ref (module=Some("Fmt") for `Fmt.(any ",")`, including nested
            // calls like `Fmt.(option ~none:(any "") ...)` where `any` is
            // inside a labeled argument subtree).
            if let Some(mod_node) = node.named_child(0) {
                let opened_module = text(mod_node, src);
                if !opened_module.is_empty() {
                    walk_children(node, src, symbols, refs, parent_idx, Some(&opened_module));
                    return;
                }
            }
            walk_children(node, src, symbols, refs, parent_idx, local_open_ctx);
        }
        "exception_definition" => {
            let idx = extract_exception_def(node, src, symbols, parent_idx);
            walk_children(node, src, symbols, refs, idx.or(parent_idx), local_open_ctx);
        }
        "module_type_definition" => {
            let idx = extract_module_type_def(node, src, symbols, parent_idx);
            walk_children(node, src, symbols, refs, idx.or(parent_idx), local_open_ctx);
        }
        "class_definition" => {
            let idx = extract_class_def(node, src, symbols, parent_idx);
            walk_children(node, src, symbols, refs, idx.or(parent_idx), local_open_ctx);
        }
        "external" => {
            extract_external(node, src, symbols, parent_idx);
            // No children to recurse into for external declarations
        }
        "value_specification" => {
            extract_value_specification(node, src, symbols, parent_idx);
        }
        // Attributes (`[@attr payload]`, `[@@attr payload]`) contain expression-
        // like payloads (e.g. `[@@deriving irmin ~pp]`) that parse as application
        // expressions but are not runtime calls — skip them entirely.
        "attribute" | "item_attribute" | "floating_attribute" => {}
        "inheritance_definition" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // `class` field is the parent class expression
            if let Some(cls_node) = node.child_by_field_name("class") {
                let name = first_identifier_in_subtree(cls_node, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::Inherits,
                        line: node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                        namespace_segments: Vec::new(),
                    });
                }
            }
            walk_children(node, src, symbols, refs, parent_idx, local_open_ctx);
        }
        "new_expression" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // `class_path` child contains the class name
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "class_path" {
                    let name = first_identifier_in_subtree(child, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name: name,
                            kind: EdgeKind::Instantiates,
                            line: node.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                            namespace_segments: Vec::new(),
                        });
                    }
                    break;
                }
            }
            walk_children(node, src, symbols, refs, parent_idx, local_open_ctx);
        }
        _ => {
            walk_children(node, src, symbols, refs, parent_idx, local_open_ctx);
        }
    }
}

fn extract_value_def(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    // `value_definition` children: `let_binding` nodes
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "let_binding" {
            if let Some(pat) = child.child_by_field_name("pattern") {
                let name = text(pat, src);
                if name.is_empty() { continue; }
                // Check if it's a function (has `parameter` children in let_binding)
                let has_params = (0..child.child_count())
                    .any(|i| child.child(i).map_or(false, |n| n.kind() == "parameter"));
                // Also check body: if fun_expression → it's a function
                let has_fun_body = child
                    .child_by_field_name("body")
                    .map(|b| b.kind() == "fun_expression" || b.kind() == "function_expression")
                    .unwrap_or(false);

                let kind = if has_params || has_fun_body {
                    SymbolKind::Function
                } else {
                    SymbolKind::Variable
                };

                let qualified_name = qualify_with_parent(&name, parent_idx, symbols);
                let scope_path = scope_path_from_parent(parent_idx, symbols);
                let idx = symbols.len();
                symbols.push(ExtractedSymbol {
                    qualified_name,
                    name,
                    kind,
                    visibility: Some(Visibility::Public),
                    start_line: node.start_position().row as u32,
                    end_line: node.end_position().row as u32,
                    start_col: 0,
                    end_col: 0,
                    signature: None,
                    doc_comment: None,
                    scope_path,
                    parent_index: parent_idx,
                });
                return Some(idx);
            }
        }
    }
    None
}

fn extract_type_def(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    // `type_definition` has `type_binding` children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_binding" {
            let name = child
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();
            if name.is_empty() { continue; }

            // Determine kind from body
            let body_opt = child.child_by_field_name("body");
            let kind = match body_opt {
                Some(body) => match body.kind() {
                    "variant_declaration" => SymbolKind::Enum,
                    "record_declaration" => SymbolKind::Struct,
                    _ => SymbolKind::TypeAlias,
                },
                None => {
                    // `equation` field = type alias (`type name = OtherType`)
                    if child.child_by_field_name("equation").is_some() {
                        SymbolKind::TypeAlias
                    } else {
                        SymbolKind::Struct
                    }
                }
            };

            let qualified_name = qualify_with_parent(&name, parent_idx, symbols);
            let scope_path = scope_path_from_parent(parent_idx, symbols);
            let idx = symbols.len();
            symbols.push(ExtractedSymbol {
                qualified_name: qualified_name.clone(),
                name,
                kind,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: 0,
                end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path: scope_path.clone(),
                parent_index: parent_idx,
            });

            // For variant types, emit each constructor as a child symbol so
            // that constructor applications resolve. Constructors live at module
            // scope in OCaml — not under the type name — so qualify them via
            // the same parent as the type itself (the enclosing module or None
            // at file top level). `scope_path` is that parent's qname.
            if let Some(body) = body_opt {
                if body.kind() == "variant_declaration" {
                    extract_variant_constructors(
                        body,
                        src,
                        symbols,
                        parent_idx,
                        scope_path.as_deref(),
                    );
                }
            }

            return Some(idx);
        }
    }
    None
}

/// Emit one `Struct`-kinded symbol per `constructor_declaration` child of a
/// `variant_declaration` node. GADT constructors (`| S : 'a t list -> 'a t`)
/// are included — `constructor_name` is always the first named child.
///
/// `module_scope` is the enclosing module's qualified name, or `None` at file
/// top level. OCaml constructors are in module scope, not type scope — so
/// `type t = A | B` inside `module M` produces `M.A` and `M.B`, not `M.t.A`.
fn extract_variant_constructors(
    variant_decl: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
    module_scope: Option<&str>,
) {
    let mut cursor = variant_decl.walk();
    for child in variant_decl.children(&mut cursor) {
        if child.kind() == "constructor_declaration" {
            let ctor_name = extract_constructor_name(child, src);
            if ctor_name.is_empty() { continue; }
            let qualified_name = match module_scope {
                Some(scope) => format!("{scope}.{ctor_name}"),
                None => ctor_name.clone(),
            };
            symbols.push(ExtractedSymbol {
                qualified_name,
                name: ctor_name,
                kind: SymbolKind::Struct,
                visibility: Some(Visibility::Public),
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: 0,
                end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path: module_scope.map(str::to_string),
                parent_index: parent_idx,
            });
        }
    }
}

fn extract_module_def(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    // `module_definition` children: `module_binding`
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "module_binding" {
            // Children include `module_name`
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "module_name" {
                    let name = text(gc, src);
                    if name.is_empty() { continue; }
                    let qualified_name = qualify_with_parent(&name, parent_idx, symbols);
                    let scope_path = scope_path_from_parent(parent_idx, symbols);
                    let idx = symbols.len();
                    symbols.push(ExtractedSymbol {
                        qualified_name,
                        name,
                        kind: SymbolKind::Namespace,
                        visibility: Some(Visibility::Public),
                        start_line: node.start_position().row as u32,
                        end_line: node.end_position().row as u32,
                        start_col: 0,
                        end_col: 0,
                        signature: None,
                        doc_comment: None,
                        scope_path,
                        parent_index: parent_idx,
                    });
                    return Some(idx);
                }
            }
        }
    }
    None
}

/// `exception Not_found` / `exception Invalid_arg of string` → Struct symbol.
/// Grammar: `exception optional(_attribute) constructor_declaration repeat(item_attribute)`
/// The constructor name is in `constructor_declaration`.
fn extract_exception_def(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    // Find the constructor_declaration child and get its constructor_name
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "constructor_declaration" {
            let name = extract_constructor_name(child, src);
            if name.is_empty() { continue; }
            let qualified_name = qualify_with_parent(&name, parent_idx, symbols);
            let scope_path = scope_path_from_parent(parent_idx, symbols);
            let idx = symbols.len();
            symbols.push(ExtractedSymbol {
                qualified_name,
                name,
                kind: SymbolKind::Struct,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: 0,
                end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path,
                parent_index: parent_idx,
            });
            return Some(idx);
        }
    }
    None
}

/// `module type S = sig ... end` → Interface symbol.
/// Grammar: `module_type_definition ... _module_type_name ...`
/// The name node kind is `module_type_name`.
fn extract_module_type_def(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    // Walk children to find module_type_name
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "module_type_name" {
            let name = text(child, src);
            if name.is_empty() { continue; }
            let qualified_name = qualify_with_parent(&name, parent_idx, symbols);
            let scope_path = scope_path_from_parent(parent_idx, symbols);
            let idx = symbols.len();
            symbols.push(ExtractedSymbol {
                qualified_name,
                name,
                kind: SymbolKind::Interface,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: 0,
                end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path,
                parent_index: parent_idx,
            });
            return Some(idx);
        }
    }
    None
}

/// `class point x0 y0 = object ... end` → Class symbol.
/// Grammar: `class_definition ... sep1('and', class_binding)`
/// The class name is `class_name` inside `class_binding`.
fn extract_class_def(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_binding" {
            // class_binding has a class_name child
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "class_name" {
                    let name = text(gc, src);
                    if name.is_empty() { continue; }
                    let qualified_name = qualify_with_parent(&name, parent_idx, symbols);
                    let scope_path = scope_path_from_parent(parent_idx, symbols);
                    let idx = symbols.len();
                    symbols.push(ExtractedSymbol {
                        qualified_name,
                        name,
                        kind: SymbolKind::Class,
                        visibility: Some(Visibility::Public),
                        start_line: node.start_position().row as u32,
                        end_line: node.end_position().row as u32,
                        start_col: 0,
                        end_col: 0,
                        signature: None,
                        doc_comment: None,
                        scope_path,
                        parent_index: parent_idx,
                    });
                    return Some(idx);
                }
            }
        }
    }
    None
}

/// `external string_length : string -> int = "caml_string_length"` → Function symbol.
/// Grammar: `external optional(_attribute) _value_name _polymorphic_typed = repeat1(string)`
/// The function name is in `_value_name` which aliases to `value_name`.
fn extract_external(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "value_name" {
            let name = text(child, src);
            if name.is_empty() { return; }
            let qualified_name = qualify_with_parent(&name, parent_idx, symbols);
            let scope_path = scope_path_from_parent(parent_idx, symbols);
            symbols.push(ExtractedSymbol {
                qualified_name,
                name,
                kind: SymbolKind::Function,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: 0,
                end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path,
                parent_index: parent_idx,
            });
            return;
        }
    }
}

/// `val foo : int -> int` in .mli / sig body → Function symbol.
/// Grammar: `value_specification 'val' optional(_attribute) _value_name _polymorphic_typed`
fn extract_value_specification(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "value_name" {
            let name = text(child, src);
            if name.is_empty() { return; }
            // value_specifications are always function-typed (val f : a -> b)
            // but we use Function kind since that's what the spec says.
            let qualified_name = qualify_with_parent(&name, parent_idx, symbols);
            let scope_path = scope_path_from_parent(parent_idx, symbols);
            symbols.push(ExtractedSymbol {
                qualified_name,
                name,
                kind: SymbolKind::Function,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: 0,
                end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path,
                parent_index: parent_idx,
            });
            return;
        }
    }
}

/// Extract the constructor name from a `constructor_declaration` node.
/// Grammar: constructor_declaration = choice(_constructor_name, alias(...), ...)
/// The `_constructor_name` aliases to `constructor_name`.
fn extract_constructor_name(node: Node, src: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "constructor_name" {
            return text(child, src);
        }
    }
    // Fallback: first identifier-like child
    text(node, src)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string()
}

/// Find the first identifier (`value_name`, `class_name`, `constructor_name`,
/// or plain `identifier`) in the subtree rooted at `node`.
fn first_identifier_in_subtree(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "value_name" | "class_name" | "constructor_name" | "module_name"
        | "module_type_name" => {
            let t = text(node, src);
            if !t.is_empty() { return t; }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let t = first_identifier_in_subtree(child, src);
        if !t.is_empty() { return t; }
    }
    String::new()
}

fn walk_children(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    local_open_ctx: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, src, symbols, refs, parent_idx, local_open_ctx);
    }
}

/// Split a `value_path` node into `(function_name, module_qualifier)`.
///
/// A `value_path` in tree-sitter-ocaml 0.24 has positional children:
///   - zero or more `module_path` / `module_name` components (the qualifier)
///   - a final `value_name` (the function name)
///
/// The raw text of the `module_path` child already encodes the full dotted
/// qualifier (e.g. `"Stdlib.List"`), so we can read it directly.
fn split_value_path(node: Node, src: &[u8]) -> (String, Option<String>) {
    let count = node.named_child_count();
    if count == 0 {
        return (text(node, src), None);
    }
    // Last named child is the value_name (or parenthesized_operator).
    // Everything before it forms the module qualifier.
    let last = match node.named_child(count - 1) {
        Some(n) => n,
        None => return (text(node, src), None),
    };
    let fn_name = text(last, src);
    if count == 1 {
        // No qualifier — plain value_name.
        return (fn_name, None);
    }
    // Collect all children except the last to form the qualifier string.
    // For `Data.Map.find`: children are [module_path("Data.Map"), value_name("find")]
    // so the module_path raw text is "Data.Map" directly.
    let module_parts: Vec<String> = (0..count - 1)
        .filter_map(|i| node.named_child(i))
        .map(|n| text(n, src))
        .collect();
    let module = module_parts.join(".");
    (fn_name, if module.is_empty() { None } else { Some(module) })
}

/// Split a `constructor_path` node into `(constructor_name, module_qualifier)`.
///
/// Grammar: `constructor_path = module_path? constructor_name`.
/// For `Command.Args.S`: module_path = "Command.Args", constructor_name = "S".
/// For bare `Circle`: no module_path, constructor_name = "Circle".
fn split_constructor_path(node: Node, src: &[u8]) -> (String, Option<String>) {
    let count = node.named_child_count();
    if count == 0 {
        return (text(node, src), None);
    }
    // The last named child is always the constructor_name.
    let last = match node.named_child(count - 1) {
        Some(n) => n,
        None => return (text(node, src), None),
    };
    let ctor_name = text(last, src);
    if count == 1 {
        return (ctor_name, None);
    }
    let module_parts: Vec<String> = (0..count - 1)
        .filter_map(|i| node.named_child(i))
        .map(|n| text(n, src))
        .collect();
    let module = module_parts.join(".");
    (ctor_name, if module.is_empty() { None } else { Some(module) })
}

fn text(node: Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").trim().to_string()
}
