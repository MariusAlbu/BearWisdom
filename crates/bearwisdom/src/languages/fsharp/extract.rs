// =============================================================================
// languages/fsharp/extract.rs  —  F# symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace  — `namespace`, `named_module`, `module_defn`
//   Function   — `function_or_value_defn` (has parameters)
//   Variable   — `function_or_value_defn` (no parameters / simple binding)
//   Class      — `type_definition` with `anon_type_defn`
//   Struct     — `type_definition` with `record_type_defn`
//   Enum       — `type_definition` with `union_type_defn` or `enum_type_defn`
//   EnumMember — `union_type_case` (inside union_type_defn)
//              — `enum_type_case` (inside enum_type_defn)
//   Field      — `record_field` (inside record_type_defn)
//   Interface  — `type_definition` with `interface_type_defn`
//   TypeAlias  — `type_definition` with `type_abbrev_defn`
//              — `module_abbrev` (module alias)
//   Struct     — `exception_definition`
//
// REFERENCES:
//   Imports    — `import_decl` (`open` declarations)
//   Calls      — `application_expression` (function application)
//   Implements — `interface_implementation` (`interface IFoo with ...`)
//   Inherits   — `class_inherits_decl` (`inherit BaseClass(args)`)
// =============================================================================

use crate::types::{EdgeKind, ExtractionResult, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

use super::applications::collect_applications;
use super::type_defs::extract_type_def;

/// Build the qualified name for a child symbol by prefixing the parent's qname.
/// Top-level symbols (no parent) use the bare name.
pub(super) fn qualify_with_parent(name: &str, parent_index: Option<usize>, symbols: &[ExtractedSymbol]) -> String {
    match parent_index.and_then(|i| symbols.get(i)) {
        Some(parent) => format!("{}.{}", parent.qualified_name, name),
        None => name.to_string(),
    }
}

/// Build the scope_path string from the parent's qualified_name. None when the
/// symbol is at file top level.
pub(super) fn scope_path_from_parent(parent_index: Option<usize>, symbols: &[ExtractedSymbol]) -> Option<String> {
    parent_index.and_then(|i| symbols.get(i)).map(|p| p.qualified_name.clone())
}

pub fn extract(source: &str) -> ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_fsharp::LANGUAGE_FSHARP.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return ExtractionResult::empty();
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Extract `#r "path/to/Assembly.dll"` directives from F# script files.
    // These are not in the tree-sitter AST but they declare external DLL
    // dependencies, so we emit an Imports ref whose target is the assembly's
    // base name (e.g., `Fornax.Core.dll` → `Fornax.Core`). The resolver's
    // wildcard-open check then classifies any bare-name ref from a file that
    // has such an import as external, identical to how `open Namespace` works.
    extract_hash_r_directives(source, &mut refs);

    visit(tree.root_node(), source, &mut symbols, &mut refs, None);

    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

pub(super) fn visit(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace" | "named_module" => {
                extract_namespace(&child, src, symbols, refs, parent_index);
            }
            "module_defn" => {
                extract_module_defn(&child, src, symbols, refs, parent_index);
            }
            "import_decl" => {
                extract_open(&child, src, symbols.len().saturating_sub(1), refs);
            }
            "function_or_value_defn" => {
                extract_let(&child, src, symbols, refs, parent_index);
            }
            "type_definition" => {
                extract_type_def(&child, src, symbols, refs, parent_index);
            }
            "module_abbrev" => {
                extract_module_abbrev(&child, src, symbols, parent_index);
            }
            "exception_definition" => {
                extract_exception_def(&child, src, symbols, parent_index);
            }
            "interface_implementation" => {
                extract_interface_implementation(&child, src, parent_index, refs);
                visit(child, src, symbols, refs, parent_index);
            }
            "class_inherits_decl" => {
                extract_class_inherits(&child, src, parent_index, refs);
            }
            // Collect application_expression and dot_expression refs from
            // method_or_prop_defn bodies (class/type member implementations).
            // These are not wrapped in function_or_value_defn so collect_applications
            // would not otherwise be called on them.
            "method_or_prop_defn" => {
                let source_idx = parent_index.unwrap_or(0);
                collect_applications(&child, src, source_idx, refs);
                visit(child, src, symbols, refs, parent_index);
            }
            _ => {
                visit(child, src, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Namespace / Module
// ---------------------------------------------------------------------------

fn extract_namespace(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = node.child_by_field_name("name")
        .map(|n| node_text(&n, src).to_string())
        .unwrap_or_default();

    if name.is_empty() {
        visit(node.clone(), src, symbols, refs, parent_index);
        return;
    }

    let line = node.start_position().row as u32;
    let kw = node.kind();
    let sig = format!("{} {}", kw, name);
    let qualified_name = qualify_with_parent(&name, parent_index, symbols);
    let scope_path = scope_path_from_parent(parent_index, symbols);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        scope_path,
        parent_index,
    });

    visit(*node, src, symbols, refs, Some(idx));
}

fn extract_module_defn(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // module_defn: `[access] module identifier = <body>`
    // module_abbrev (when grammar produces module_defn for abbreviations):
    //   `module L = Some.Long.Path` — the block contains only a long_identifier
    //   → emit TypeAlias instead of Namespace.
    let name = first_identifier_text(node, src);
    if name.is_empty() {
        visit(*node, src, symbols, refs, parent_index);
        return;
    }

    // Check if the block is a pure long_identifier (module alias) or real body.
    let is_alias = is_module_alias(node, src);
    let kind = if is_alias { SymbolKind::TypeAlias } else { SymbolKind::Namespace };
    let line = node.start_position().row as u32;
    let qualified_name = qualify_with_parent(&name, parent_index, symbols);
    let scope_path = scope_path_from_parent(parent_index, symbols);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("module {}", name)),
        doc_comment: None,
        scope_path,
        parent_index,
    });

    if !is_alias {
        visit(*node, src, symbols, refs, Some(idx));
    }
}

/// Return true if a `module_defn` node is actually a module abbreviation:
/// `module L = Some.Long.Name` — the `block` field contains only a
/// `long_identifier` or `long_identifier_or_op` with no sub-declarations.
fn is_module_alias(node: &Node, src: &str) -> bool {
    // Look for the `block` field (produced by scoped()).
    if let Some(block) = node.child_by_field_name("block") {
        let k = block.kind();
        // Pure long identifier — alias
        if k == "long_identifier" || k == "long_identifier_or_op" {
            return true;
        }
        // block with a single long_identifier child
        if block.named_child_count() == 1 {
            if let Some(inner) = block.named_child(0) {
                let ik = inner.kind();
                if ik == "long_identifier" || ik == "long_identifier_or_op" {
                    return true;
                }
            }
        }
        // expression that is purely a dotted identifier (contains dots — no declarations)
        if k == "long_identifier" {
            return true;
        }
        // Heuristic: if the block text contains no newlines and looks like a dotted path
        let block_text = node_text(&block, src);
        if !block_text.contains('\n') && block_text.split('.').all(|seg| {
            !seg.is_empty() && seg.chars().all(|c| c.is_alphanumeric() || c == '_')
        }) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// #r directive → Imports  (F# script files only)
// ---------------------------------------------------------------------------

/// Scan source text for `#r "..."` directives and emit one Imports ref per
/// directive whose path contains a recognizable assembly name.
///
/// The DLL base name (without `.dll` extension) becomes the target, so
/// `#r "../../packages/Fornax.Core.dll"` → target `Fornax.Core`. The resolver
/// then applies `is_external_namespace_fallback` against the root segment
/// (`Fornax`), matching the same path that `open Fornax.*` would follow.
fn extract_hash_r_directives(src: &str, refs: &mut Vec<ExtractedRef>) {
    let line_starts: Vec<u32> = {
        let mut offsets = vec![0u32];
        let mut pos: u32 = 0;
        for b in src.bytes() {
            pos += 1;
            if b == b'\n' { offsets.push(pos); }
        }
        offsets
    };
    for (line_idx, line) in src.lines().enumerate() {
        let trimmed = line.trim();
        // Only process `#r "..."` lines; stop at the first non-directive,
        // non-blank line that is not a `#load` or `#if`/`#endif` to avoid
        // scanning the full file body.
        if !trimmed.starts_with("#r ") && !trimmed.starts_with("#r\"") {
            // Allow blank lines, comments, #load, #if, #endif, #else to
            // continue scanning; anything else ends the header section.
            let is_header_line = trimmed.is_empty()
                || trimmed.starts_with("//")
                || trimmed.starts_with("(*")
                || trimmed.starts_with("#load")
                || trimmed.starts_with("#if")
                || trimmed.starts_with("#else")
                || trimmed.starts_with("#endif")
                || trimmed.starts_with("#nowarn")
                || trimmed.starts_with("#I ");
            if !is_header_line {
                break;
            }
            continue;
        }

        // Extract the quoted path from `#r "path"`.
        let after_r = trimmed.trim_start_matches("#r").trim();
        let path = after_r.trim_matches('"');
        if path.is_empty() {
            continue;
        }

        // Derive the assembly name from the last path component.
        let filename = path.rsplit(['/', '\\']).next().unwrap_or(path);
        // Strip `.dll` (case-insensitive) to get the bare assembly name.
        let assembly = if filename.to_ascii_lowercase().ends_with(".dll") {
            &filename[..filename.len() - 4]
        } else {
            filename
        };

        if assembly.is_empty() {
            continue;
        }

        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: assembly.to_string(),
            kind: EdgeKind::Imports,
            line: line_idx as u32,
            module: Some(assembly.to_string()),
            chain: None,
            byte_offset: line_starts.get(line_idx).copied().unwrap_or(0),
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
}

// ---------------------------------------------------------------------------
// open declaration → Imports
// ---------------------------------------------------------------------------

fn extract_open(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // import_decl: `open LongIdentifier`
    let text = node_text(node, src);
    let module = text.trim_start_matches("open").trim().to_string();
    if module.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: module.clone(),
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: Some(module),
        chain: None,
        byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// let binding → Function / Variable
// ---------------------------------------------------------------------------

fn extract_let(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // function_or_value_defn: `let [rec] name [params] [: type] = body`
    // Name is in function_declaration_left or value_declaration_left
    let name = extract_let_name(node, src);
    if name.is_empty() {
        return;
    }

    // Determine if it's a function (has parameters) by checking for parameter nodes
    let has_params = has_function_params(node, src);
    let kind = if has_params { SymbolKind::Function } else { SymbolKind::Variable };
    let line = node.start_position().row as u32;
    let qualified_name = qualify_with_parent(&name, parent_index, symbols);
    let scope_path = scope_path_from_parent(parent_index, symbols);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("let {}", name)),
        doc_comment: None,
        scope_path,
        parent_index,
    });

    // Collect calls in the body and recurse for nested let bindings
    collect_applications(node, src, idx, refs);
    visit(*node, src, symbols, refs, Some(idx));
}

fn extract_let_name(node: &Node, src: &str) -> String {
    // Walk children looking for the declaration LHS
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration_left" => {
                // Has a direct `identifier` child for the function name
                return first_identifier_text(&child, src);
            }
            "value_declaration_left" => {
                // value_declaration_left → identifier_pattern → long_identifier_or_op
                // The `identifier_pattern` holds the binding name(s).
                // We want the first long_identifier_or_op inside the first
                // identifier_pattern — that is the binding name.
                return extract_value_decl_name(&child, src);
            }
            _ => {}
        }
    }
    // Fallback: first identifier
    first_identifier_text(node, src)
}

/// Extract the binding name from a `value_declaration_left` node.
///
/// The structure is:
///   value_declaration_left
///     identifier_pattern
///       long_identifier_or_op   ← this is the name
///       [identifier_pattern …]  ← these are parameters (ignored here)
fn extract_value_decl_name(node: &Node, src: &str) -> String {
    // First named child should be identifier_pattern
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier_pattern" {
            // First child of identifier_pattern is long_identifier_or_op
            let mut ic = child.walk();
            for ipc in child.children(&mut ic) {
                if ipc.kind() == "long_identifier_or_op" {
                    let t = node_text(&ipc, src).to_string();
                    if !t.is_empty() {
                        return t;
                    }
                }
            }
            // Fallback: direct identifier under identifier_pattern
            return first_identifier_text(&child, src);
        }
    }
    // Fallback: direct identifier under value_declaration_left
    first_identifier_text(node, src)
}

fn has_function_params(node: &Node, src: &str) -> bool {
    let _ = src;
    // If function_declaration_left has more than one identifier child, it has params
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_declaration_left" {
            // count identifier/pattern children beyond the first (name)
            let mut c2 = child.walk();
            let count = child.children(&mut c2)
                .filter(|n| n.kind() == "identifier" || n.kind() == "typed_pattern" || n.kind() == "argument_patterns")
                .count();
            return count > 1;
        }
    }
    false
}


// ---------------------------------------------------------------------------
// module_abbrev, exception_definition, interface_implementation, class_inherits
// ---------------------------------------------------------------------------

/// `module L = Some.Long.Name` → TypeAlias symbol named `L`.
fn extract_module_abbrev(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // module_abbrev: `module <identifier> = <long_identifier_or_op>`
    // First identifier child is the alias name.
    let name = first_identifier_text(node, src);
    if name.is_empty() { return; }
    let qualified_name = qualify_with_parent(&name, parent_index, symbols);
    let scope_path = scope_path_from_parent(parent_index, symbols);
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("module {}", name)),
        doc_comment: None,
        scope_path,
        parent_index,
    });
}

/// `exception MyError of string` → Struct symbol named `MyError`.
fn extract_exception_def(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // exception_definition: `exception <exception_name> [of <type>]`
    // The name node kind may be `exception_name` or a plain `identifier`.
    let name = node
        .child_by_field_name("exception_name")
        .map(|n| node_text(&n, src).to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| first_identifier_text(node, src));
    if name.is_empty() { return; }
    let qualified_name = qualify_with_parent(&name, parent_index, symbols);
    let scope_path = scope_path_from_parent(parent_index, symbols);
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Struct,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("exception {}", name)),
        doc_comment: None,
        scope_path,
        parent_index,
    });
}

/// `interface IFoo with ...` → Implements edge targeting the interface name.
pub(super) fn extract_interface_implementation(
    node: &Node,
    src: &str,
    parent_index: Option<usize>,
    refs: &mut Vec<ExtractedRef>,
) {
    // interface_implementation: `interface <_type> with <member_defns>`
    // Grammar: interface keyword, then a `_type` child (simple_type, named_type,
    // long_identifier_or_op, generic_type, etc.), then optional `with` body.
    // We want the last identifier in the type chain (the simple unqualified name)
    // or the full text if it's a simple type.
    let source_idx = parent_index.unwrap_or(0);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let t = node_text(&child, src);
        // Skip the `interface` keyword and empty nodes
        if t == "interface" || t.is_empty() { continue; }
        // The `with` keyword marks end of the type section
        if t == "with" { break; }
        // Skip other keyword tokens (unlikely but defensive)
        if child.child_count() == 0 && is_keyword(t) { continue; }
        // This is the type node — extract the last identifier as the interface name.
        // For `simple_type → long_identifier → "System" "." "IDisposable"` we want "IDisposable".
        // For a bare `identifier` node we just use its text.
        let iface_name = last_identifier_text(child, src);
        if !iface_name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: iface_name,
                kind: EdgeKind::Implements,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }
        break;
    }
}

/// `inherit Animal(name)` → Inherits edge targeting `Animal`.
pub(super) fn extract_class_inherits(
    node: &Node,
    src: &str,
    parent_index: Option<usize>,
    refs: &mut Vec<ExtractedRef>,
) {
    // class_inherits_decl: `inherit <_type> [<args>]`
    // Grammar: `inherit scoped(seq(_type, optional(_expression)), indent, dedent)`
    // The type child (simple_type, long_identifier_or_op, etc.) holds the base class name.
    let source_idx = parent_index.unwrap_or(0);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let t = node_text(&child, src);
        if t == "inherit" || t.is_empty() { continue; }
        // Skip pure keyword tokens
        if child.child_count() == 0 && is_keyword(t) { continue; }
        // The type node — extract first identifier as base class name.
        let base_name = first_identifier_from_type(child, src);
        if !base_name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: base_name,
                kind: EdgeKind::Inherits,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn node_text<'a>(node: &Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

pub(super) fn first_identifier_text(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let t = node_text(&child, src).to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    String::new()
}

/// Find the LAST identifier in the direct children of `node`.
/// Useful for `simple_type → long_identifier → "System" "." "IDisposable"`
/// where we want "IDisposable" (the last segment).
fn last_identifier_text(node: Node, src: &str) -> String {
    // If the node itself has identifier children, get the last one.
    let mut last = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let t = node_text(&child, src).to_string();
                if !t.is_empty() { last = t; }
            }
            // Recurse one level into wrapper types (simple_type, long_identifier, etc.)
            k if !k.starts_with('"') => {
                let inner = last_identifier_text(child, src);
                if !inner.is_empty() { last = inner; }
            }
            _ => {}
        }
    }
    if last.is_empty() {
        // No identifier children — maybe the node IS an identifier
        if node.kind() == "identifier" {
            let t = node_text(&node, src).to_string();
            if !t.is_empty() { return t; }
        }
    }
    last
}

/// Find the FIRST identifier in a type node (for base class names in `inherit`).
/// Handles `simple_type`, `long_identifier_or_op`, `named_type`, bare `identifier`.
fn first_identifier_from_type(node: Node, src: &str) -> String {
    // If the node is directly an identifier, return its text.
    if node.kind() == "identifier" {
        return node_text(&node, src).to_string();
    }
    // Recurse into children until we find the first identifier.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let t = node_text(&child, src).to_string();
            if !t.is_empty() { return t; }
        }
        let inner = first_identifier_from_type(child, src);
        if !inner.is_empty() { return inner; }
    }
    String::new()
}

pub(super) fn is_keyword(s: &str) -> bool {
    matches!(s,
        "let" | "in" | "if" | "then" | "else" | "match" | "with"
        | "fun" | "function" | "type" | "and" | "or" | "not"
        | "begin" | "end" | "do" | "done" | "for" | "while"
        | "try" | "finally" | "raise" | "failwith" | "failwithf"
        | "true" | "false" | "null" | "void" | "open" | "module"
        | "namespace" | "of" | "rec" | "mutable" | "new" | "inherit"
        | "override" | "abstract" | "static" | "member" | "val"
        | "interface" | "class" | "struct" | "exception" | "yield"
        | "return" | "async" | "seq" | "task" | "query"
    )
}
