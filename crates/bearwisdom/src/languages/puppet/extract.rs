// =============================================================================
// languages/puppet/extract.rs  —  Puppet infrastructure-as-code extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class    — class_definition (Puppet class)
//   Class    — defined_resource_type (define <name> — callable resource type)
//   Function — function_declaration (Puppet 4+ function)
//   Function — node_definition (matches specific hosts)
//   Variable — resource_declaration (resource instance)
//
// REFERENCES:
//   Imports + Calls — include_statement (include foo::bar)
//   Imports + Calls — require_statement (require foo::bar)
//   Calls           — function_call
//   Calls           — resource_declaration → resource type
//   Inherits        — class_definition with class_inherits
//
// Grammar: tree-sitter-puppet (not yet in Cargo.toml — ready for when added).
// Puppet uses '::' as namespace separator; qualified names preserve it.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from a Puppet manifest (.pp).
///
/// Requires the tree-sitter-puppet grammar to be available as `language`.
/// Called by `PuppetPlugin::extract()` once the grammar is wired in.
#[allow(dead_code)]
pub fn extract(source: &str, language: tree_sitter::Language) -> crate::types::ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Puppet grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_manifest(tree.root_node(), source, &mut symbols, &mut refs);

    // Second pass: collect all resource_reference and function_call nodes
    // for ref coverage (catches nodes missed by dispatch_node traversal).
    collect_resource_references(tree.root_node(), source, &mut refs);
    collect_all_function_calls(tree.root_node(), source, &mut refs);

    // Post-filter: drop refs whose target is a locally-bound `$variable`
    // (class / define / function parameter, or lambda block variable) at
    // the ref's line. Puppet's lambda syntax `$xs.each |T $x| { …use $x… }`
    // emits each use of `$x` as a function_call → Calls ref; without this
    // every block variable lands in unresolved_refs. Same for class params:
    // `class foo ($directory) { … use $directory … }` similarly leaks.
    // Mirrors the TS / Kotlin / Scala type-param filter pattern.
    {
        let mut scopes: Vec<(String, u32, u32)> = Vec::new();
        collect_local_var_scopes(tree.root_node(), source, &mut scopes);
        if !scopes.is_empty() {
            refs.retain(|r| {
                !scopes.iter().any(|(name, start, end)| {
                    &r.target_name == name && r.line >= *start && r.line <= *end
                })
            });
        }
    }

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

/// Walk the tree and record every `$variable` binding that should be
/// considered local to a declaration. Three sources:
///   * `class_definition` / `defined_resource_type` / `function_declaration`
///     → walk the `parameter_list` child, extract each `$name` from
///     `parameter > expression > variable`.
///   * Same containers → walk the `block` child and collect the LHS `$name`
///     of every `assignment` statement (`$var = expr`). Puppet has
///     function-wide variable scope (no block scoping), so any assignment
///     anywhere in the body puts that variable in scope for the entire
///     declaration. This catches body-locals like `$mod_libs = $apache::…`
///     whose subscript uses (`$mod_libs[$mod]`) are emitted as
///     `resource_reference` Calls refs by the extractor.
///   * `lambda` → direct `variable` children are block params (`|$x, $y|`).
///
/// Each binding is scoped to the enclosing declaration's line range so
/// uses outside (or in sibling scopes) still resolve normally.
fn collect_local_var_scopes(
    node: Node,
    src: &str,
    out: &mut Vec<(String, u32, u32)>,
) {
    let kind = node.kind();
    let is_param_container = matches!(
        kind,
        "class_definition"
            | "defined_resource_type"
            | "function_declaration"
    );
    if is_param_container {
        let start_line = node.start_position().row as u32;
        let end_line = node.end_position().row as u32;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "parameter_list" {
                collect_param_variable_names(child, src, start_line, end_line, out);
            }
            if child.kind() == "block" {
                collect_block_assignment_vars(child, src, start_line, end_line, out);
            }
        }
    }
    // `$xs.each |$x| { … }` is parsed as `iterator_statement` in
    // tree-sitter-puppet (the grammar exposes `lambda` as a sibling kind
    // but Puppet's pipe-delimited block form lands here instead). Block
    // variables are direct `variable` children appearing before the
    // `block` child; walk in order and stop once we hit the body.
    if kind == "iterator_statement" || kind == "lambda" {
        let start_line = node.start_position().row as u32;
        let end_line = node.end_position().row as u32;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" {
                break;
            }
            if child.kind() == "variable" {
                let name = node_text(child, src);
                if !name.is_empty() {
                    out.push((name, start_line, end_line));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_local_var_scopes(child, src, out);
    }
}

/// For each `parameter` inside a `parameter_list`, walk the subtree for
/// the first `variable` node and record its text (`$name`).
fn collect_param_variable_names(
    param_list: Node,
    src: &str,
    start_line: u32,
    end_line: u32,
    out: &mut Vec<(String, u32, u32)>,
) {
    let mut cursor = param_list.walk();
    for param in param_list.children(&mut cursor) {
        if param.kind() != "parameter" {
            continue;
        }
        if let Some(name) = first_variable_descendant(&param, src) {
            out.push((name, start_line, end_line));
        }
    }
}

/// Recursively walk a `block` node and record the LHS `$variable` of every
/// `assignment` statement as a body-local binding scoped to [start_line,
/// end_line]. Puppet has function-wide variable scope — an assignment
/// anywhere in the body (`$mod_libs = $apache::mod_libs`) makes that name
/// available throughout the enclosing declaration, including uses inside
/// nested `if`/`elsif`/`else` sub-blocks that tree-sitter emits as
/// `resource_reference` refs (e.g. `$mod_libs[$mod]`).
fn collect_block_assignment_vars(
    block: Node,
    src: &str,
    start_line: u32,
    end_line: u32,
    out: &mut Vec<(String, u32, u32)>,
) {
    let mut cursor = block.walk();
    for stmt in block.children(&mut cursor) {
        match stmt.kind() {
            "assignment" => {
                // assignment: variable ('=' | '+=') expression
                // The first child is the LHS variable.
                if let Some(first) = stmt.child(0) {
                    if first.kind() == "variable" {
                        let name = node_text(first, src);
                        if !name.is_empty() {
                            out.push((name, start_line, end_line));
                        }
                    }
                }
            }
            // Puppet is function-scoped: recurse into nested control-flow
            // blocks so assignments inside `if`/`elsif`/`unless`/`case`
            // sub-blocks are also captured.
            "if_statement" | "unless_statement" | "case_statement" | "elsif_statement"
            | "else_statement" | "case_item" | "default_case" => {
                let mut inner = stmt.walk();
                for child in stmt.children(&mut inner) {
                    if child.kind() == "block" {
                        collect_block_assignment_vars(child, src, start_line, end_line, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Pre-order left-first search for the first `variable` descendant.
fn first_variable_descendant(node: &Node, src: &str) -> Option<String> {
    if node.kind() == "variable" {
        let t = node_text(*node, src);
        if !t.is_empty() {
            return Some(t);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(v) = first_variable_descendant(&child, src) {
            return Some(v);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Manifest traversal
// ---------------------------------------------------------------------------

fn visit_manifest(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch_node(&child, src, symbols, refs, None);
    }
}

fn dispatch_node(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    match node.kind() {
        "class_definition" => extract_class_definition(node, src, symbols, refs),
        "defined_resource_type" => extract_defined_resource_type(node, src, symbols, refs),
        "function_declaration" => extract_function_declaration(node, src, symbols, refs),
        "node_definition" => extract_node_definition(node, src, symbols, refs),
        "resource_declaration" => {
            extract_resource_declaration(node, src, symbols, refs, parent_index)
        }
        "include_statement" => extract_include_or_require(node, src, refs, parent_index),
        "require_statement" => extract_include_or_require(node, src, refs, parent_index),
        "function_call" => extract_function_call(node, src, refs, parent_index),
        _ => {
            // Recurse into block-like containers.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                dispatch_node(&child, src, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// class <name> [inherits <parent>] [($params)] { ... }  →  Class
// ---------------------------------------------------------------------------

fn extract_class_definition(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_class_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let sig = build_class_signature(node, src, &name);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name.clone(),
        SymbolKind::Class,
        node,
        Some(sig),
        None,
    ));

    // Check for `inherits <parent>` — emit Inherits edge.
    if let Some(parent) = find_inherits_name(node, src) {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: parent,
            kind: EdgeKind::Inherits,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
        });
    }

    // Recurse into the class body.
    visit_class_body(node, src, symbols, refs, idx);
}

fn find_class_name(node: &Node, src: &str) -> Option<String> {
    // class_definition: `class` keyword, then class identifier.
    let mut cursor = node.walk();
    let mut saw_class_keyword = false;
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Skip the `class` keyword token.
            "class" => { saw_class_keyword = true; }
            "identifier" | "class_identifier" if saw_class_keyword => {
                return Some(node_text(child, src));
            }
            _ => {}
        }
    }
    // Fallback: first identifier/class_identifier child.
    let mut cursor2 = node.walk();
    for child in node.children(&mut cursor2) {
        if matches!(child.kind(), "identifier" | "class_identifier") {
            return Some(node_text(child, src));
        }
    }
    None
}

fn find_inherits_name(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    let mut in_inherits = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "class_inherits" {
            // class_inherits node contains the parent class identifier.
            let mut cc = child.walk();
            for ic in child.children(&mut cc) {
                if matches!(ic.kind(), "identifier" | "class_identifier") {
                    return Some(node_text(ic, src));
                }
            }
        }
        // Handle inline `inherits` keyword followed by identifier.
        if node_text(child, src) == "inherits" {
            in_inherits = true;
        } else if in_inherits && matches!(child.kind(), "identifier" | "class_identifier") {
            return Some(node_text(child, src));
        }
    }
    None
}

fn build_class_signature(node: &Node, src: &str, name: &str) -> String {
    // Take the first line of the class definition as the signature.
    let first_line = node_text(*node, src)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if first_line.is_empty() {
        format!("class {}", name)
    } else {
        first_line
    }
}

fn visit_class_body(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "block" || child.kind() == "body" {
            let mut bc = child.walk();
            for stmt in child.children(&mut bc) {
                dispatch_node(&stmt, src, symbols, refs, Some(parent_index));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// define <name> [($params)] { ... }  →  Class
// ---------------------------------------------------------------------------

fn extract_defined_resource_type(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_define_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let first_line = node_text(*node, src)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let sig = if first_line.is_empty() {
        format!("define {}", name)
    } else {
        first_line
    };

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Class,
        node,
        Some(sig),
        None,
    ));

    visit_class_body(node, src, symbols, refs, idx);
}

fn find_define_name(node: &Node, src: &str) -> Option<String> {
    // defined_resource_type: `define` keyword, then name.
    let mut cursor = node.walk();
    let mut saw_define = false;
    for child in node.children(&mut cursor) {
        if node_text(child, src) == "define" {
            saw_define = true;
        } else if saw_define && matches!(child.kind(), "identifier" | "class_identifier") {
            return Some(node_text(child, src));
        }
    }
    // Fallback via named fields.
    if let Some(n) = node.child_by_field_name("class_identifier") {
        return Some(node_text(n, src));
    }
    if let Some(n) = node.child_by_field_name("identifier") {
        return Some(node_text(n, src));
    }
    None
}

// ---------------------------------------------------------------------------
// function <name>(...): <type> { ... }  →  Function
// ---------------------------------------------------------------------------

fn extract_function_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_function_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let first_line = node_text(*node, src)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let sig = if first_line.is_empty() {
        format!("function {}", name)
    } else {
        first_line
    };

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Function,
        node,
        Some(sig),
        None,
    ));

    // Recurse into function body.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch_node(&child, src, symbols, refs, Some(idx));
    }
}

fn find_function_name(node: &Node, src: &str) -> Option<String> {
    // function_declaration: `function` keyword, then identifier.
    let mut cursor = node.walk();
    let mut saw_function = false;
    for child in node.children(&mut cursor) {
        if node_text(child, src) == "function" {
            saw_function = true;
        } else if saw_function && child.kind() == "identifier" {
            return Some(node_text(child, src));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// node '<name>' { ... }  →  Function
// ---------------------------------------------------------------------------

fn extract_node_definition(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_node_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let sig = format!("node '{}'", name);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Function,
        node,
        Some(sig),
        None,
    ));

    visit_class_body(node, src, symbols, refs, idx);
}

fn find_node_name(node: &Node, src: &str) -> Option<String> {
    // node_definition: `node` keyword, then node_name (string/regex/default/identifier).
    if let Some(nn) = node.child_by_field_name("node_name") {
        return Some(node_text(nn, src).trim_matches('"').trim_matches('\'').to_string());
    }
    // Fallback: first string or identifier after `node`.
    let mut cursor = node.walk();
    let mut saw_node = false;
    for child in node.children(&mut cursor) {
        if node_text(child, src) == "node" {
            saw_node = true;
        } else if saw_node {
            match child.kind() {
                "string" | "identifier" | "default" => {
                    let t = node_text(child, src)
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string();
                    return Some(t);
                }
                _ => {}
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// resource_declaration → Variable + Calls to resource type
// ---------------------------------------------------------------------------

fn extract_resource_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let res_type = match find_resource_type(node, src) {
        Some(t) => t,
        None => return,
    };
    let title = find_resource_title(node, src).unwrap_or_else(|| res_type.clone());

    let name = format!("{}[{}]", res_type, title);
    let sig = format!("{} {{ '{}': ... }}", res_type, title);

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some(sig),
        parent_index,
    ));

    // Calls edge to the resource type.
    refs.push(ExtractedRef {
        source_symbol_index: idx,
        target_name: res_type,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
    });
}

fn find_resource_type(node: &Node, src: &str) -> Option<String> {
    // resource_declaration has a `type` field or the first class_identifier/identifier child.
    if let Some(t) = node.child_by_field_name("type") {
        return Some(node_text(t, src));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "class_identifier" | "identifier") {
            return Some(node_text(child, src));
        }
    }
    None
}

fn find_resource_title(node: &Node, src: &str) -> Option<String> {
    // The title is typically a string literal after the resource type name.
    if let Some(t) = node.child_by_field_name("title") {
        let raw = node_text(t, src);
        return Some(raw.trim_matches('"').trim_matches('\'').to_string());
    }
    // Fallback: first string child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            let raw = node_text(child, src);
            return Some(raw.trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// include / require → Imports + Calls
// ---------------------------------------------------------------------------

fn extract_include_or_require(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);

    // Collect all class identifiers from the statement.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "class_identifier" | "identifier") {
            let name = node_text(child, src);
            // Imports edge.
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: name.clone(),
                kind: EdgeKind::Imports,
                line: child.start_position().row as u32,
                module: Some(name.clone()),
                chain: None,
                byte_offset: 0,
            });
            // Calls edge.
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: name,
                kind: EdgeKind::Calls,
                line: child.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// function_call → Calls
// ---------------------------------------------------------------------------

fn extract_function_call(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);
    // Emit ref at the function_call node's line (not the identifier child's line)
    // so coverage correlation matches the function_call ref_node_kind.
    let line = node.start_position().row as u32;

    // Try identifier, class_identifier, qualified_name, variable — take the first.
    let name = {
        let mut found = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let k = child.kind();
            if matches!(k, "identifier" | "class_identifier" | "variable" | "qualified_name" | "name") {
                found = node_text(child, src);
                break;
            }
        }
        if found.is_empty() {
            // Fallback: take the first-line text of the node itself (before `(`)
            let raw = node_text(*node, src);
            raw.lines().next().unwrap_or("").split('(').next().unwrap_or("").trim().to_string()
        } else {
            found
        }
    };

    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: if name.is_empty() { "fn".to_string() } else { name },
        kind: EdgeKind::Calls,
        line,
        module: None,
        chain: None,
        byte_offset: 0,
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

/// Walk the entire tree and emit a Calls ref for every `function_call` node.
/// This second pass catches function_calls not visited by dispatch_node.
fn collect_all_function_calls(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() == "function_call" {
        let line = node.start_position().row as u32;
        let mut name = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let k = child.kind();
            if matches!(k, "identifier" | "class_identifier" | "variable" | "qualified_name") {
                name = node_text(child, src);
                break;
            }
        }
        if name.is_empty() {
            let raw = node_text(node, src);
            name = raw.lines().next().unwrap_or("").split('(').next().unwrap_or("fn").trim().to_string();
        }
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: if name.is_empty() { "fn".to_string() } else { name },
            kind: EdgeKind::Calls,
            line,
            module: None,
            chain: None,
            byte_offset: 0,
        });
        // Recurse into function_call children to find nested calls
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_all_function_calls(child, src, refs);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_all_function_calls(child, src, refs);
    }
}

/// Walk the entire tree and emit a Calls ref for every `resource_reference` node.
fn collect_resource_references(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() == "resource_reference" {
        // resource_reference: Type['title'] — always emit at the node's line
        let line = node.start_position().row as u32;
        let mut name = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let k = child.kind();
            if matches!(k, "class_identifier" | "identifier" | "variable") {
                name = node_text(child, src);
                break;
            }
        }
        if name.is_empty() {
            // Take just the type part (before '[')
            let raw = node_text(node, src);
            name = raw.split('[').next().unwrap_or("").trim().to_string();
        }
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: if name.is_empty() { "Resource".to_string() } else { name },
            kind: EdgeKind::TypeRef,
            line,
            module: None,
            chain: None,
            byte_offset: 0,
        });
        // Still recurse to find nested resource_references
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_resource_references(child, src, refs);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_resource_references(child, src, refs);
    }
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

#[cfg(test)]
mod local_scope_tests {
    use super::*;
    use tree_sitter::Parser;

    fn puppet_lang() -> tree_sitter::Language {
        tree_sitter_puppet::LANGUAGE.into()
    }

    fn parse_and_collect(src: &str) -> Vec<(String, u32, u32)> {
        let mut parser = Parser::new();
        parser.set_language(&puppet_lang()).unwrap();
        let tree = parser.parse(src, None).unwrap();
        let mut out = Vec::new();
        collect_local_var_scopes(tree.root_node(), src, &mut out);
        out
    }

    #[test]
    fn class_param_scoped_to_class_body() {
        let src = "class foo ($directory = '/tmp') {\n  file { $directory: }\n}\n";
        let scopes = parse_and_collect(src);
        assert!(
            scopes.iter().any(|(n, _, _)| n == "$directory"),
            "expected $directory in scopes: {scopes:?}"
        );
    }

    #[test]
    fn lambda_block_variable_scoped_to_lambda() {
        let src = "class foo {\n  $xs.each |$x| {\n    notify { $x: }\n  }\n}\n";
        let scopes = parse_and_collect(src);
        assert!(
            scopes.iter().any(|(n, _, _)| n == "$x"),
            "expected $x from lambda: {scopes:?}"
        );
    }

    #[test]
    fn typed_lambda_variable_captured() {
        // Puppet lambda with a type annotation: `|Hash $directory|`
        let src = "class foo {\n  $xs.each |Hash $directory| {\n    notify { $directory: }\n  }\n}\n";
        let scopes = parse_and_collect(src);
        assert!(
            scopes.iter().any(|(n, _, _)| n == "$directory"),
            "typed lambda var should be captured: {scopes:?}"
        );
    }

    #[test]
    fn out_of_scope_variable_not_captured_wrongly() {
        // `$other` is not a parameter anywhere; should NOT appear.
        let src = "class foo ($directory) { notify { $other: } }\n";
        let scopes = parse_and_collect(src);
        assert!(
            !scopes.iter().any(|(n, _, _)| n == "$other"),
            "$other must not be captured: {scopes:?}"
        );
    }

    #[test]
    fn body_assignment_in_define_captured() {
        // Mirrors the puppet-apache pattern:
        //   define apache::mod (...) {
        //     $mod_libs = $apache::mod_libs
        //     if $mod in $mod_libs { $x = $mod_libs[$mod] }
        //   }
        // `$mod_libs` is a body-local assignment; its subscript use
        // `$mod_libs[$mod]` is emitted as a resource_reference ref and
        // should be suppressed.
        let src = concat!(
            "define apache::mod (\n",
            "  Optional[String] $package = undef,\n",
            ") {\n",
            "  $mod_libs = $apache::mod_libs\n",
            "  if $mod in $mod_libs {\n",
            "    $_lib = $mod_libs[$mod]\n",
            "  }\n",
            "}\n",
        );
        let scopes = parse_and_collect(src);
        assert!(
            scopes.iter().any(|(n, _, _)| n == "$mod_libs"),
            "body-assignment $mod_libs should be captured: {scopes:?}"
        );
    }

    #[test]
    fn body_assignment_in_nested_if_captured() {
        // Assignments inside if/elsif sub-blocks are also function-scoped in
        // Puppet, so they too should suppress refs throughout the declaration.
        let src = concat!(
            "class apache::mod::php (\n",
            "  Optional[String] $package_name = undef,\n",
            ") {\n",
            "  $mod_packages = $apache::mod_packages\n",
            "  if $package_name {\n",
            "    $_pkg = $package_name\n",
            "  } elsif $mod in $mod_packages {\n",
            "    $_pkg = $mod_packages[$mod]\n",
            "  }\n",
            "}\n",
        );
        let scopes = parse_and_collect(src);
        assert!(
            scopes.iter().any(|(n, _, _)| n == "$mod_packages"),
            "body-assignment $mod_packages should be captured: {scopes:?}"
        );
        // $package_name is a formal parameter — should also be present
        assert!(
            scopes.iter().any(|(n, _, _)| n == "$package_name"),
            "formal param $package_name should be captured: {scopes:?}"
        );
    }
}
