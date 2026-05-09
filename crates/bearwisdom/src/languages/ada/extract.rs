// =============================================================================
// languages/ada/extract.rs — Ada extractor (tree-sitter-based)
//
// SYMBOLS:
//   Function  — `subprogram_declaration` and `subprogram_body`
//               (inner `function_specification` or `procedure_specification`)
//   Namespace — `package_declaration` and `package_body` (name field)
//   Struct    — `full_type_declaration` with record body
//   Enum      — `full_type_declaration` with enumeration body
//
// REFERENCES:
//   Imports   — `with_clause` → identifier children
//   Calls     — `procedure_call_statement` and `function_call`
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_ada::LANGUAGE.into())
        .is_err()
    {
        return ExtractionResult::empty();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::empty(),
    };

    let src = source.as_bytes();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    walk_node(tree.root_node(), src, &mut symbols, &mut refs, None);

    ExtractionResult::new(symbols, refs, tree.root_node().has_error())
}

fn walk_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    match node.kind() {
        "subprogram_declaration" | "subprogram_body" => {
            let idx = extract_subprogram(node, src, symbols, parent_idx);
            walk_children(node, src, symbols, refs, idx.or(parent_idx));
        }
        "generic_subprogram_declaration" => {
            // `generic ... function Foo (...) return T;` — extract the
            // inner subprogram spec same as a regular subprogram_declaration.
            // Without this, gnat-stdlib's generic functions (`Ada.Unchecked_Conversion`,
            // `Ada.Unchecked_Deallocation`, every `Ada.Containers.*.Generic_*`)
            // never enter the symbol table and the ~140 instantiation calls
            // per Ada-driver project go unresolved.
            let idx = extract_subprogram(node, src, symbols, parent_idx);
            walk_children(node, src, symbols, refs, idx.or(parent_idx));
        }
        "generic_package_declaration" => {
            // `generic ... package Foo is ... end Foo;` — emit one Namespace
            // for Foo, then walk the inner package_declaration's CHILDREN
            // directly (skipping the package_declaration node itself, which
            // would otherwise double-emit and produce qnames like
            // `Foo.Foo.X` for every member).
            let mut cursor = node.walk();
            let mut idx: Option<usize> = None;
            let mut inner_decl: Option<Node> = None;
            for child in node.children(&mut cursor) {
                if child.kind() == "package_declaration" {
                    let inner_name = child
                        .child_by_field_name("name")
                        .map(|n| text(n, src))
                        .unwrap_or_default();
                    if !inner_name.is_empty() {
                        idx = Some(push_sym(node, inner_name, SymbolKind::Namespace, symbols, parent_idx));
                    }
                    inner_decl = Some(child);
                    break;
                }
            }
            // Walk other children (generic_formal_part) under the parent;
            // walk the inner package_declaration's children under the new
            // namespace so its members nest correctly without duplication.
            let mut cur = node.walk();
            for child in node.children(&mut cur) {
                if matches!(child.kind(), "package_declaration") {
                    let mut cur2 = child.walk();
                    for inner in child.children(&mut cur2) {
                        walk_node(inner, src, symbols, refs, idx.or(parent_idx));
                    }
                } else {
                    walk_node(child, src, symbols, refs, parent_idx);
                }
            }
            let _ = inner_decl;
        }
        "package_declaration" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Namespace, symbols, parent_idx);
                walk_children(node, src, symbols, refs, Some(idx));
            } else {
                walk_children(node, src, symbols, refs, parent_idx);
            }
        }
        "package_body" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Namespace, symbols, parent_idx);
                walk_children(node, src, symbols, refs, Some(idx));
            } else {
                walk_children(node, src, symbols, refs, parent_idx);
            }
        }
        "full_type_declaration" => {
            let idx = extract_type_decl(node, src, symbols, parent_idx);
            walk_children(node, src, symbols, refs, idx.or(parent_idx));
        }
        "component_declaration" => {
            // Record field: `Field_Name : Field_Type;` inside `record ... end record`.
            // Same structure as object_declaration / parameter_specification —
            // identifier(s) before the colon, type after. Emit one Field
            // symbol per declared name qualified under the enclosing struct
            // so `members_of(<struct.qname>)` returns the field for record-
            // method dispatch (`This.CCER`, `Display.Buffers`).
            let mut cursor = node.walk();
            let mut names: Vec<(Node, String)> = Vec::new();
            let mut type_name: Option<String> = None;
            let mut seen_colon = false;
            for child in node.children(&mut cursor) {
                let kind = child.kind();
                if kind == ":" {
                    seen_colon = true;
                    continue;
                }
                if !seen_colon {
                    if kind == "identifier" {
                        let name = text(child, src);
                        if !name.is_empty() {
                            names.push((child, name));
                        }
                    }
                    continue;
                }
                if type_name.is_some() {
                    continue;
                }
                if matches!(kind, "identifier" | "selected_component") {
                    let t = text(child, src);
                    if !t.is_empty() && !is_ada_mode_keyword(&t) {
                        type_name = Some(t);
                    }
                } else if matches!(kind, "subtype_indication" | "subtype_mark" | "component_definition") {
                    let mut cur = child.walk();
                    for inner in child.children(&mut cur) {
                        if matches!(inner.kind(), "identifier" | "selected_component") {
                            let t = text(inner, src);
                            if !t.is_empty() && !is_ada_mode_keyword(&t) {
                                type_name = Some(t);
                                break;
                            }
                        }
                    }
                }
            }
            for (n, name) in names {
                let idx = push_sym(n, name, SymbolKind::Field, symbols, parent_idx);
                if let (Some(sym), Some(ty)) = (symbols.get_mut(idx), type_name.as_ref()) {
                    sym.signature = Some(format!("type: {ty}"));
                }
            }
            walk_children(node, src, symbols, refs, parent_idx);
        }
        "object_declaration" | "parameter_specification" => {
            // `X : T;` / `X : T := init;` / `X : in out T;` etc. We emit a
            // Variable symbol per declared name with the type encoded into
            // `signature` so the resolver can chain `X.Method` →
            // `<type-of-X>.Method`. This is the minimum-viable type tracking
            // for Ada record-method dispatch (`This.CCER`, `Result.Append`).
            //
            // Defining names live before the `:` (collected as `identifier`
            // children of `_defining_identifier_list`); the type appears
            // after — first non-keyword identifier-shaped node we encounter.
            let mut cursor = node.walk();
            let mut names: Vec<(Node, String)> = Vec::new();
            let mut type_name: Option<String> = None;
            let mut seen_colon = false;
            for child in node.children(&mut cursor) {
                let kind = child.kind();
                if kind == ":" {
                    seen_colon = true;
                    continue;
                }
                if !seen_colon {
                    // Identifier(s) before colon are the defining names.
                    if kind == "identifier" {
                        let name = text(child, src);
                        if !name.is_empty() {
                            names.push((child, name));
                        }
                    }
                    continue;
                }
                // After the colon — first identifier-shaped node is the type.
                if type_name.is_some() {
                    continue;
                }
                if matches!(kind, "identifier" | "selected_component") {
                    let t = text(child, src);
                    if !t.is_empty() && !is_ada_mode_keyword(&t) {
                        type_name = Some(t);
                    }
                } else if matches!(
                    kind,
                    "subtype_indication" | "subtype_mark"
                ) {
                    // Walk one layer deeper for the inner identifier.
                    let mut cur = child.walk();
                    for inner in child.children(&mut cur) {
                        if matches!(
                            inner.kind(),
                            "identifier" | "selected_component"
                        ) {
                            let t = text(inner, src);
                            if !t.is_empty() && !is_ada_mode_keyword(&t) {
                                type_name = Some(t);
                                break;
                            }
                        }
                    }
                }
            }
            for (n, name) in names {
                let idx = push_sym(n, name, SymbolKind::Variable, symbols, parent_idx);
                if let (Some(sym), Some(ty)) = (symbols.get_mut(idx), type_name.as_ref()) {
                    sym.signature = Some(format!("type: {ty}"));
                }
            }
            walk_children(node, src, symbols, refs, parent_idx);
        }
        "generic_instantiation" => {
            // `package String_Vectors is new Ada.Containers.Vectors (...)` or
            // `function To_Address is new Ada.Unchecked_Conversion (...)`.
            // Emit the local name as a symbol whose signature encodes the
            // generic source so the resolver can chain through:
            //   Result : String_Vectors.Vector → String_Vectors is an
            //   instantiation of Ada.Containers.Vectors → look up Append on
            //   Ada.Containers.Vectors.
            let name_node = node.child_by_field_name("name");
            let local_name = name_node.map(|n| text(n, src));
            let mut cursor = node.walk();
            let mut seen_new = false;
            let mut is_package = false;
            let mut generic_name: Option<String> = None;
            for child in node.children(&mut cursor) {
                let kind = child.kind();
                match kind {
                    "package" => is_package = true,
                    "procedure" | "function" => is_package = false,
                    "new" => seen_new = true,
                    "identifier" | "selected_component" if seen_new && generic_name.is_none() => {
                        generic_name = Some(text(child, src));
                    }
                    _ => {}
                }
            }
            if let Some(name) = local_name.filter(|n| !n.is_empty()) {
                let kind = if is_package { SymbolKind::Namespace } else { SymbolKind::Function };
                let idx = push_sym(node, name, kind, symbols, parent_idx);
                if let (Some(sym), Some(g)) = (symbols.get_mut(idx), generic_name) {
                    sym.signature = Some(format!("instantiates {g}"));
                }
            }
            walk_children(node, src, symbols, refs, parent_idx);
        }
        "subprogram_renaming_declaration" => {
            // `procedure Put_Line (S : String) renames Trendy_Terminal.IO.Put_Line;`
            // declares Put_Line as a local alias inside the enclosing package.
            // Without recognizing this, calls like `Put_Line(...)` from a file
            // that does `use SP.Terminal;` can't find the symbol — the bare-name
            // resolver looks up `members_of("SP.Terminal")` and gets nothing
            // because the rename never produced a symbol row.
            //
            // Emit a real symbol so it lands in the symbol table under the
            // parent package's qname (e.g. `SP.Terminal.Put_Line`). Best-effort
            // name extraction: tree-sitter-ada exposes the alias either via the
            // `name` field on the inner specification, or as the first
            // identifier-shaped child in the rename node.
            let alias = subprogram_rename_alias(node, src);
            if let Some(name) = alias.filter(|n| !n.is_empty()) {
                push_sym(node, name, SymbolKind::Function, symbols, parent_idx);
            }
        }
        "package_renaming_declaration" => {
            // `package Trace renames Simple_Logging;` brings Simple_Logging
            // into scope as `Trace`. Without this, every `Trace.<x>` call
            // references an undefined symbol (Simple_Logging is typically
            // an external Ada library — Alire's lib uses `Trace renames
            // Simple_Logging` for ~600 unresolveds in alire).
            //
            // Two emissions:
            //  1. An Imports edge so the file-local `Trace.x` resolution
            //     works via the resolver's alias-substitution path.
            //  2. A Namespace symbol for the alias under the enclosing
            //     package's qname (e.g. `SP.Strings.ASU`) with `signature
            //     = "renames Ada.Strings.Unbounded"` — this makes nested
            //     package renames discoverable through `members_of(parent)`
            //     for files that `use parent;` and access the alias bare.
            //     The resolver detects the `renames <target>` signature
            //     and chains through to the target package.
            //
            // tree-sitter-ada emits the rename as:
            //   package identifier "<alias>" renames <identifier|selected_component> ;
            let sym_idx = parent_idx.unwrap_or(0);
            let mut cursor = node.walk();
            let mut alias_name: Option<String> = None;
            let mut target_module: Option<String> = None;
            let mut seen_renames = false;
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "renames" => seen_renames = true,
                    "identifier" => {
                        if !seen_renames {
                            alias_name = Some(text(child, src));
                        } else {
                            target_module = Some(text(child, src));
                        }
                    }
                    "selected_component" => {
                        if seen_renames {
                            target_module = Some(text(child, src));
                        }
                    }
                    _ => {}
                }
            }
            if let (Some(alias), Some(target)) = (alias_name, target_module) {
                if !alias.is_empty() && !target.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: alias.clone(),
                        kind: EdgeKind::Imports,
                        line: node.start_position().row as u32,
                        module: Some(target.clone()),
                        chain: None,
                        byte_offset: 0,
                        namespace_segments: Vec::new(),
                    });
                    // Emit the alias as a real Namespace symbol qualified
                    // under its parent so cross-file `members_of(parent)`
                    // sees it. The resolver looks at `signature` to chain
                    // alias.<x> → target.<x>.
                    let alias_idx = push_sym(node, alias, SymbolKind::Namespace, symbols, parent_idx);
                    if let Some(sym) = symbols.get_mut(alias_idx) {
                        sym.signature = Some(format!("renames {target}"));
                    }
                }
            }
        }
        "with_clause" | "use_clause" | "use_type_clause" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // `with X;` makes X visible dot-qualified. `use X;` brings X's
            // exports into bare scope; `use type X;` only brings primitive
            // operators of type X into scope. All three produce Imports edges
            // so the resolver's FileContext sees them as wildcard candidates.
            // Children include `identifier` (simple) and `selected_component`
            // (dotted: Ada.Text_IO) nodes for each package name.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "identifier" => {
                        let name = text(child, src);
                        if !name.is_empty() {
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
                    "selected_component" => {
                        // Use the full text (e.g. "Ada.Text_IO") as module name
                        let name = text(child, src);
                        if !name.is_empty() {
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
                    _ => {}
                }
            }
        }
        "procedure_call_statement" | "function_call" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // Skip Ada attribute references — `System'To_Address(X)`,
            // `Vector'Length(V)`, `Type'First(T)` etc. parse as
            // procedure/function calls because they have an
            // actual_parameter_part, but they're attribute applications
            // on a type/object, not calls to a named subprogram. Without
            // this filter, ada-drivers' SVD register code produces ~1k
            // bogus `System` calls that pollute the unresolved bank.
            if !is_attribute_reference(node) {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = text(name_node, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: node.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                            namespace_segments: Vec::new(),
                        });
                    }
                }
            }
            walk_children(node, src, symbols, refs, parent_idx);
        }
        _ => {
            walk_children(node, src, symbols, refs, parent_idx);
        }
    }
}

fn extract_subprogram(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let name = match child.kind() {
            "function_specification" | "procedure_specification" => {
                child
                    .child_by_field_name("name")
                    .map(|n| text(n, src))
            }
            _ => None,
        };
        if let Some(name) = name {
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Function, symbols, parent_idx);
                return Some(idx);
            }
        }
    }
    None
}

/// Pull the alias name out of a `subprogram_renaming_declaration` node.
/// Tree-sitter-ada wraps the alias in either a `procedure_specification`
/// or `function_specification` (both expose a `name` field).
fn subprogram_rename_alias(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "procedure_specification" | "function_specification"
        ) {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = text(name_node, src);
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

fn extract_type_decl(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    // Gather identifiers (name) and determine kind from body
    let mut name = String::new();
    let mut kind = SymbolKind::Struct;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" if name.is_empty() => {
                name = text(child, src);
            }
            "enumeration_type_definition" => {
                kind = SymbolKind::Enum;
            }
            "record_type_definition" => {
                kind = SymbolKind::Struct;
            }
            _ => {}
        }
    }

    if name.is_empty() { return None; }
    let idx = push_sym(node, name, kind, symbols, parent_idx);
    Some(idx)
}

fn push_sym(
    node: Node,
    name: String,
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> usize {
    let idx = symbols.len();
    // Qualify by the parent symbol's qualified_name when one exists. Ada is
    // package-scoped: `package Trace is procedure Debug ...` ⇒ Debug must
    // be reachable via the qname `Trace.Debug` so cross-file callers like
    // `Trace.Debug(msg)` resolve via the engine's qualified_name lookup
    // (Step 5 in resolve_common).
    let qualified_name = match parent_idx.and_then(|i| symbols.get(i)) {
        Some(parent) if !parent.qualified_name.is_empty() => {
            format!("{}.{}", parent.qualified_name, name)
        }
        _ => name.clone(),
    };
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
        scope_path: None,
        parent_index: parent_idx,
    });
    idx
}

fn walk_children(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, src, symbols, refs, parent_idx);
    }
}

fn text(node: Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").trim().to_string()
}

/// True if the token is one of Ada's parameter / object mode markers,
/// not a type identifier. Used to skip `in`, `out`, `aliased`, etc.
/// when scanning a `parameter_specification` for the actual type.
fn is_ada_mode_keyword(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "in" | "out" | "access" | "aliased" | "constant" | "not" | "null" | "exception"
    )
}

/// True if the call node is actually an Ada attribute reference such as
/// `System'To_Address(X)` or `Vec'Length(X)`. tree-sitter-ada parses these
/// as `procedure_call_statement` / `function_call` because they have an
/// `actual_parameter_part`, but they're attribute applications — not calls
/// to a named subprogram. Detect by the presence of a `tick` child.
fn is_attribute_reference(node: Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "tick" {
            return true;
        }
    }
    false
}
