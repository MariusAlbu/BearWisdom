// =============================================================================
// languages/nix/extract.rs  —  Nix expression language extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Variable  — binding where value is a non-function expression
//   Function  — binding where value is a function_expression (lambda)
//   Variable  — inherit / inherit_from items
//
// REFERENCES:
//   Imports   — apply_expression calling `import` → path argument
//   Imports   — apply_expression calling `callPackage` → first path argument
//   Imports   — with_expression → the environment name (brings scope into context)
//   Calls     — apply_expression → function name (variable_expression / select_expression)
//
// Grammar: tree-sitter-nix (not yet in Cargo.toml — ready for when added).
// Nix is purely functional; every construct is an expression. The primary
// declaration form is `binding` inside attrset/let expressions.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from a Nix expression file.
///
/// Requires the tree-sitter-nix grammar to be available as `language`.
/// Called by `NixPlugin::extract()` once the grammar is wired in.
#[allow(dead_code)]
pub fn extract(source: &str, language: tree_sitter::Language) -> crate::types::ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Nix grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // The root of a Nix file is typically a single expression.
    // We walk the whole tree to capture top-level and let-bound symbols.
    visit_expr(tree.root_node(), source, &mut symbols, &mut refs, None, true);

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Expression traversal
// ---------------------------------------------------------------------------

/// Visit a Nix expression, extracting symbols and refs.
/// `top_level` is true when visiting the outermost expression in the file,
/// which controls whether bindings in attribute sets are treated as public.
fn visit_expr(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    top_level: bool,
) {
    match node.kind() {
        "attrset_expression" | "rec_attrset_expression" => {
            extract_attrset(node, src, symbols, refs, parent_index, top_level);
        }
        "let_expression" | "let_attrset_expression" => {
            extract_let(node, src, symbols, refs, parent_index);
        }
        "with_expression" => {
            extract_with(node, src, symbols.len(), refs);
            // Continue into the body
            if let Some(body) = node.child_by_field_name("body") {
                visit_expr(body, src, symbols, refs, parent_index, false);
            }
        }
        "apply_expression" => {
            let source_idx = symbols.len().saturating_sub(1);
            extract_apply(node, src, source_idx, refs);
            // Recurse into sub-expressions (chained applies, argument expressions)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    visit_expr(child, src, symbols, refs, parent_index, false);
                }
            }
        }
        "select_expression" => {
            // Emit a Calls ref for every select_expression
            let source_idx = symbols.len().saturating_sub(1);
            let name = resolve_call_name(node, src)
                .unwrap_or_else(|| node_text(node, src));
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
            // Recurse into sub-expressions
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    visit_expr(child, src, symbols, refs, parent_index, false);
                }
            }
        }
        "function_expression" => {
            // A lambda — not a named declaration at this level.
            // Visit formal parameter default values, then the body.
            visit_formal_defaults(node, src, symbols, refs, parent_index);
            if let Some(body) = node.child_by_field_name("body") {
                visit_expr(body, src, symbols, refs, parent_index, false);
            }
        }
        _ => {
            // Descend into child expressions
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    visit_expr(child, src, symbols, refs, parent_index, false);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Attribute set  (top-level or rec attrset)
// ---------------------------------------------------------------------------

fn extract_attrset(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    is_public: bool,
) {
    let vis = if is_public {
        Visibility::Public
    } else {
        Visibility::Private
    };

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "binding_set" => {
                // tree-sitter-nix 0.3: bindings are wrapped in binding_set
                extract_binding_set(child, src, symbols, refs, parent_index, vis);
            }
            "binding" => {
                extract_binding(&child, src, symbols, refs, parent_index, vis);
            }
            "inherit" => {
                extract_inherit(&child, src, symbols, parent_index, vis);
            }
            "inherit_from" => {
                extract_inherit_from(&child, src, symbols, refs, parent_index, vis);
            }
            _ => {}
        }
    }
}

/// Extract bindings from a `binding_set` node.
fn extract_binding_set(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    vis: Visibility,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "binding" => {
                extract_binding(&child, src, symbols, refs, parent_index, vis);
            }
            "inherit" => {
                extract_inherit(&child, src, symbols, parent_index, vis);
            }
            "inherit_from" => {
                extract_inherit_from(&child, src, symbols, refs, parent_index, vis);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Let expression  (let ... in ...)
// ---------------------------------------------------------------------------

fn extract_let(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Bindings in `let` are private (local scope)
    let vis = Visibility::Private;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "binding_set" => {
                let mut bc = child.walk();
                for binding in child.children(&mut bc) {
                    match binding.kind() {
                        "binding" => {
                            extract_binding(&binding, src, symbols, refs, parent_index, vis);
                        }
                        "inherit" => {
                            extract_inherit(&binding, src, symbols, parent_index, vis);
                        }
                        "inherit_from" => {
                            extract_inherit_from(&binding, src, symbols, refs, parent_index, vis);
                        }
                        _ => {}
                    }
                }
            }
            "binding" => {
                extract_binding(&child, src, symbols, refs, parent_index, vis);
            }
            _ => {
                // `in` body expression — visit for nested lets/attrsets
                if is_expr_node(&child) && child.kind() != "let_expression" {
                    visit_expr(child, src, symbols, refs, parent_index, false);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Binding  (name = expr;)
// ---------------------------------------------------------------------------

fn extract_binding(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    vis: Visibility,
) {
    // attrpath gives the binding name (may be dotted: a.b.c).
    // For bindings with interpolated attrpaths (e.g. `${name} = expr`), the
    // name may not be statically extractable. In that case, still process the
    // value expression for refs — skip only the symbol creation.
    let name_opt = binding_name(node, src);

    let value = binding_value(node);

    let idx = if let Some(name) = name_opt {
        let kind = match value {
            Some(v) if is_function_expr(v) => SymbolKind::Function,
            _ => SymbolKind::Variable,
        };
        let sig = if kind == SymbolKind::Function {
            format!("{} = ...: ...", name)
        } else {
            format!("{} = ...", name)
        };
        let i = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name,
            kind,
            visibility: Some(vis),
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: node.end_position().column as u32,
            signature: Some(sig),
            doc_comment: None,
            scope_path: None,
            parent_index,
        });
        i
    } else {
        // Name not statically extractable (interpolated attrpath).
        // Use the nearest parent symbol as the ref source.
        parent_index.unwrap_or(symbols.len().saturating_sub(1))
    };

    // Visit the value expression for nested declarations and refs
    if let Some(v) = value {
        extract_value_refs(v, src, idx, symbols, refs);
    }
}

/// Extract refs and nested symbols from a binding's value expression.
fn extract_value_refs(
    node: Node,
    src: &str,
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "apply_expression" => {
            extract_apply(node, src, source_symbol_index, refs);
            // Recurse into all sub-expressions, including nested apply_expression
            // children. This handles curried application (`f a b` → two applies)
            // and ensures callPackage inner applies emit their Imports refs.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    extract_value_refs(child, src, source_symbol_index, symbols, refs);
                }
            }
        }
        "select_expression" => {
            // Emit a Calls ref for every select_expression (attribute access).
            // This covers both `pkgs.hello` as a value AND as a function in an apply.
            let name = resolve_call_name(node, src)
                .unwrap_or_else(|| node_text(node, src));
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
            // Recurse into sub-expressions (but not the attrpath — avoid double-emit)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    extract_value_refs(child, src, source_symbol_index, symbols, refs);
                }
            }
        }
        "attrset_expression" | "rec_attrset_expression" => {
            // Nested attrset — extract its bindings as children
            extract_attrset(node, src, symbols, refs, Some(source_symbol_index), false);
        }
        "let_expression" | "let_attrset_expression" => {
            extract_let(node, src, symbols, refs, Some(source_symbol_index));
        }
        "function_expression" => {
            // Visit formal parameter default values, then the lambda body.
            visit_formal_defaults(node, src, symbols, refs, Some(source_symbol_index));
            if let Some(body) = node.child_by_field_name("body") {
                extract_value_refs(body, src, source_symbol_index, symbols, refs);
            }
        }
        "with_expression" => {
            extract_with(node, src, source_symbol_index, refs);
            if let Some(body) = node.child_by_field_name("body") {
                extract_value_refs(body, src, source_symbol_index, symbols, refs);
            }
        }
        _ => {
            // Recurse looking for apply_expression, select_expression, and with_expression
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    extract_value_refs(child, src, source_symbol_index, symbols, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// inherit  (inherit name1 name2;)
// ---------------------------------------------------------------------------

fn extract_inherit(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    vis: Visibility,
) {
    // inherit has `inherited_attrs` field or identifier children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "inherited_attrs" => {
                let mut ac = child.walk();
                for attr in child.children(&mut ac) {
                    if attr.kind() == "identifier" {
                        let name = node_text(attr, src);
                        if !name.is_empty() {
                            emit_inherit_symbol(name, &attr, vis, parent_index, symbols);
                        }
                    }
                }
            }
            "identifier" => {
                // Direct identifier children (some grammar versions)
                let name = node_text(child, src);
                if !name.is_empty() && name != "inherit" {
                    emit_inherit_symbol(name, &child, vis, parent_index, symbols);
                }
            }
            _ => {}
        }
    }
}

fn emit_inherit_symbol(
    name: String,
    node: &Node,
    vis: Visibility,
    parent_index: Option<usize>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Variable,
        visibility: Some(vis),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("inherit {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// inherit_from  (inherit (src) name1 name2;)
// ---------------------------------------------------------------------------

fn extract_inherit_from(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    vis: Visibility,
) {
    // The source expression is a parenthesized expression containing a variable name
    let source_name = find_inherit_from_source(node, src);

    // Emit an Imports ref to the source if it's a named variable
    let dummy_source_idx = symbols.len();
    if let Some(src_name) = &source_name {
        refs.push(ExtractedRef {
            source_symbol_index: dummy_source_idx,
            target_name: src_name.clone(),
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
        });
    }

    // Extract the inherited attribute names
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let name = node_text(child, src);
            if !name.is_empty() && name != "inherit" {
                emit_inherit_symbol(name, &child, vis, parent_index, symbols);
            }
        } else if child.kind() == "inherited_attrs" {
            let mut ac = child.walk();
            for attr in child.children(&mut ac) {
                if attr.kind() == "identifier" {
                    let name = node_text(attr, src);
                    if !name.is_empty() {
                        emit_inherit_symbol(name.clone(), &attr, vis, parent_index, symbols);
                    }
                }
            }
        }
    }
}

/// Find the source expression name in `inherit (src) ...`.
///
/// tree-sitter-nix 0.3: `inherit_from` has an `expression` named field that
/// holds the source attrset expression (the part in parentheses).
fn find_inherit_from_source(node: &Node, src: &str) -> Option<String> {
    // Primary: use the `expression` named field (tree-sitter-nix 0.3+)
    if let Some(expr) = node.child_by_field_name("expression") {
        if let Some(name) = resolve_var_name(expr, src) {
            return Some(name);
        }
        return first_identifier_text(&expr, src);
    }
    // Fallback: iterate children looking for parenthesized or variable expressions.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "parenthesized_expression" | "expression" => {
                if let Some(ident) = first_identifier_text(&child, src) {
                    return Some(ident);
                }
                let mut cc = child.walk();
                for inner in child.children(&mut cc) {
                    if inner.kind() == "variable_expression" || inner.kind() == "identifier" {
                        return Some(node_text(inner, src));
                    }
                }
            }
            "variable_expression" | "identifier" => {
                let t = node_text(child, src);
                if !t.is_empty() && t != "inherit" {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// apply_expression  (function call / import)
// ---------------------------------------------------------------------------

fn extract_apply(
    node: Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // apply_expression: function field + argument
    let func_node = node.child_by_field_name("function")
        .or_else(|| first_child_of_kind(&node, "variable_expression"))
        .or_else(|| {
            // First child that is an expression
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if is_expr_node(&child) {
                        return Some(child);
                    }
                }
            }
            None
        });

    let func_name = func_node.and_then(|n| {
        // For curried applies like `(f a) b`, the outer apply's function is another apply.
        // Recursively resolve to find the original function name.
        resolve_apply_func_name(n, src)
    });

    // If the function name can't be resolved (e.g. anonymous lambda `(x: ...)` in
    // function position, or a complex expression), use the node text as a fallback
    // target so coverage correlation can still match this apply site.
    let func_name = match func_name {
        Some(n) => n,
        None => {
            // Emit a minimal Calls ref with whatever text we can extract from the
            // function node, truncated to avoid noise. This ensures the apply site
            // registers a ref rather than being silently unmatched.
            let fallback = func_node.map(|n| {
                let t = node_text(n, src);
                // Limit to 80 chars to avoid giant lambda bodies as ref targets
                if t.len() > 80 { t[..80].to_string() } else { t }
            });
            if let Some(target) = fallback {
                if !target.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: target,
                        kind: EdgeKind::Calls,
                        line: node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                    });
                }
            }
            return;
        }
    };

    // `import` is a keyword/builtin in Nix — emit Imports edge when arg is a path.
    // If the arg is a complex expression (e.g. `nixpkgs + "/path"`), fall through
    // to emit a Calls edge for `import` itself so every apply site has a ref.
    if func_name == "import" {
        if let Some(arg) = apply_argument(&node) {
            if let Some(p) = extract_path_or_string(arg, src) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: p.clone(),
                    kind: EdgeKind::Imports,
                    line: node.start_position().row as u32,
                    module: Some(p),
                    chain: None,
                    byte_offset: 0,
                });
                return;
            }
        }
        // Path not extractable — fall through to emit Calls -> "import".
    }

    // `callPackage path {}` — emit Imports edge to the package path.
    // If the arg is not a literal path (unusual), fall through to a Calls edge.
    if func_name == "callPackage" || func_name.ends_with(".callPackage") {
        if let Some(arg) = apply_argument(&node) {
            if let Some(p) = extract_path_or_string(arg, src) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: p.clone(),
                    kind: EdgeKind::Imports,
                    line: node.start_position().row as u32,
                    module: Some(p),
                    chain: None,
                    byte_offset: 0,
                });
                return;
            }
        }
        // Path not extractable — fall through to emit Calls -> "callPackage".
    }

    // General function application — emit Calls edge.
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: func_name,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
    });
}

// ---------------------------------------------------------------------------
// Formal parameter defaults  ({ pkgs ? import <nixpkgs> { }, ... }: body)
// ---------------------------------------------------------------------------

/// Visit the default-value expressions of formal parameters in a
/// `function_expression` whose arguments are an attrset pattern.
///
/// Tree-sitter-nix represents formal parameters as children of the
/// `function_expression`'s first child (`formals` or `formal_set`).
/// Each `formal` node may have a default value after `?`.
///
/// These defaults are not reachable via body traversal, so we visit them
/// explicitly to capture apply_expressions like `import <nixpkgs> { }`.
fn visit_formal_defaults(
    func_node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(symbols.len().saturating_sub(1));
    // The formals container is typically the first named child before `:`.
    let mut outer_cursor = func_node.walk();
    for child in func_node.children(&mut outer_cursor) {
        // Look for `formals`, `formal_set`, or `formal` nodes
        match child.kind() {
            "formals" | "formal_set" => {
                let mut fc = child.walk();
                for formal in child.children(&mut fc) {
                    if formal.kind() == "formal" {
                        visit_formal_default(formal, src, source_idx, symbols, refs);
                    }
                }
            }
            "formal" => {
                visit_formal_default(child, src, source_idx, symbols, refs);
            }
            _ => {}
        }
    }
}

fn visit_formal_default(
    formal: Node,
    src: &str,
    source_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // formal: identifier ? default_expr
    // The default expression is the expression child after `?`
    let mut cursor = formal.walk();
    let mut past_question_mark = false;
    for child in formal.children(&mut cursor) {
        if !child.is_named() && node_text(child, src) == "?" {
            past_question_mark = true;
            continue;
        }
        if past_question_mark && is_expr_node(&child) {
            extract_value_refs(child, src, source_idx, symbols, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// with_expression  (with pkgs; ...)
// ---------------------------------------------------------------------------

fn extract_with(
    node: Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // with_expression: environment field is the namespace being brought in scope
    let env = node.child_by_field_name("environment").or_else(|| {
        // First expression child (the namespace before `;`)
        let mut cursor = node.walk();
        let found: Option<Node> = {
            let mut iter = node.children(&mut cursor);
            iter.find(|c| is_expr_node(c))
        };
        found
    });

    if let Some(env_node) = env {
        if let Some(name) = resolve_var_name(env_node, src) {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Name/value helpers for bindings
// ---------------------------------------------------------------------------

/// Get the binding name from `attrpath` (may be dotted: a.b.c → "a.b.c").
fn binding_name(node: &Node, src: &str) -> Option<String> {
    let attrpath = node
        .child_by_field_name("attrpath")
        .or_else(|| first_child_of_kind(node, "attrpath"))?;

    // Collect identifier children joined by "."
    let mut parts = Vec::new();
    let mut cursor = attrpath.walk();
    for child in attrpath.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "attr" {
            let t = node_text(child, src);
            if !t.is_empty() && t != "." {
                parts.push(t);
            }
        }
        // Also handle interpolated attrs (${...}) — skip those for now
    }

    if parts.is_empty() {
        // Fallback: first identifier in the binding
        first_identifier_text(node, src)
    } else {
        Some(parts.join("."))
    }
}

/// Get the value node from a binding (the expression after `=`).
fn binding_value<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    node.child_by_field_name("expression")
        .or_else(|| {
            // Find the expression after `=` sign
            let mut cursor = node.walk();
            let mut past_eq = false;
            for child in node.children(&mut cursor) {
                if past_eq && is_expr_node(&child) {
                    return Some(child);
                }
                if node_is_eq_sign(&child) {
                    past_eq = true;
                }
            }
            None
        })
}

fn node_is_eq_sign(node: &Node) -> bool {
    // Anonymous `=` token
    node.kind() == "=" || (!node.is_named() && node.kind() == "=")
}

fn is_function_expr(node: Node) -> bool {
    matches!(node.kind(), "function_expression" | "lambda")
}

// ---------------------------------------------------------------------------
// Call name resolution
// ---------------------------------------------------------------------------

/// Resolve the ultimate function name from the function position of an apply_expression.
/// Handles curried applies: `(f a) b` → `f a` is another apply → resolve recursively.
fn resolve_apply_func_name(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "variable_expression" | "identifier" | "select_expression" => resolve_call_name(node, src),
        "apply_expression" => {
            // Curried call: resolve the inner function
            let inner_func = node.child_by_field_name("function").or_else(|| {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if is_expr_node(&child) {
                            return Some(child);
                        }
                    }
                }
                None
            });
            inner_func.and_then(|n| resolve_apply_func_name(n, src))
        }
        "parenthesized_expression" => {
            // The inner expression is the actual function — recurse into it.
            // E.g. `(builtins.fetchTarball url) {}` has a parenthesized apply as func.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    if let Some(name) = resolve_apply_func_name(child, src) {
                        return Some(name);
                    }
                }
            }
            None
        }
        _ => resolve_call_name(node, src),
    }
}

fn resolve_call_name(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "variable_expression" => {
            node.child_by_field_name("name")
                .map(|n| node_text(n, src))
                .or_else(|| first_identifier_text(&node, src))
        }
        "identifier" => Some(node_text(node, src)),
        "select_expression" => {
            // e.g., `lib.makeOverridable` or `pkgs.stdenv`
            // Build the full dotted path
            let mut parts = Vec::new();
            collect_select_path(node, src, &mut parts);
            if parts.is_empty() { None } else { Some(parts.join(".")) }
        }
        _ => None,
    }
}

fn collect_select_path(node: Node, src: &str, parts: &mut Vec<String>) {
    match node.kind() {
        "select_expression" => {
            // select_expression: expression.attrpath
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "variable_expression" | "identifier" => {
                        if let Some(n) = first_identifier_text(&child, src)
                            .or_else(|| {
                                child.child_by_field_name("name")
                                    .map(|n| node_text(n, src))
                            })
                        {
                            parts.push(n);
                        }
                    }
                    "select_expression" => collect_select_path(child, src, parts),
                    "attrpath" | "attr" | "identifier" => {
                        parts.push(node_text(child, src));
                    }
                    _ => {}
                }
            }
        }
        "variable_expression" => {
            if let Some(n) = first_identifier_text(&node, src) {
                parts.push(n);
            }
        }
        "identifier" => {
            parts.push(node_text(node, src));
        }
        _ => {}
    }
}

fn resolve_var_name(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "variable_expression" => {
            node.child_by_field_name("name")
                .map(|n| node_text(n, src))
                .or_else(|| first_identifier_text(&node, src))
        }
        "identifier" => Some(node_text(node, src)),
        _ => first_identifier_text(&node, src),
    }
}

fn apply_argument<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    // The argument in apply_expression is the second expression child
    if let Some(arg) = node.child_by_field_name("argument") {
        return Some(arg);
    }
    // Fallback: second named expression child
    let mut cursor = node.walk();
    let mut count = 0usize;
    for child in node.children(&mut cursor) {
        if is_expr_node(&child) {
            if count == 1 {
                return Some(child);
            }
            count += 1;
        }
    }
    None
}

fn extract_path_or_string(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "path_expression" | "hpath_expression" | "spath_expression" => {
            Some(node_text(node, src))
        }
        "string_expression" | "indented_string_expression" => {
            // Strip quotes
            let raw = node_text(node, src);
            Some(raw.trim_matches('"').trim_matches('\'').to_string())
        }
        _ => {
            // Recurse into parenthesized
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(p) = extract_path_or_string(child, src) {
                    return Some(p);
                }
            }
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_expr_node(node: &Node) -> bool {
    // Named expression nodes to recurse into.
    // `interpolation` is included so that ${ ... } string interpolations are
    // traversed — apply_expression nodes inside them would otherwise be invisible.
    matches!(
        node.kind(),
        "attrset_expression"
            | "rec_attrset_expression"
            | "let_expression"
            | "let_attrset_expression"
            | "with_expression"
            | "apply_expression"
            | "function_expression"
            | "lambda"
            | "if_expression"
            | "assert_expression"
            | "select_expression"
            | "binary_expression"
            | "unary_expression"
            | "parenthesized_expression"
            | "list_expression"
            | "path_expression"
            | "string_expression"
            | "indented_string_expression"
            | "interpolation"
            | "variable_expression"
            | "integer_expression"
            | "float_expression"
            | "uri_expression"
            | "has_attr_expression"
    )
}

fn first_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

fn first_identifier_text(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let t = node_text(child, src);
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
