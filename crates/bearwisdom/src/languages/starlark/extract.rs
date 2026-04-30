// =============================================================================
// languages/starlark/extract.rs  —  Starlark / Bazel BUILD extractor
//
// Uses the tree-sitter-starlark grammar for reliable extraction.
//
// What we extract
// ---------------
// SYMBOLS:
//   Function — `function_definition` (def name(...):)
//   Variable — `assignment` (name = value) — includes rule(), struct(), etc.
//
// REFERENCES:
//   Imports  — `call` where callee is `load` identifier
//   Calls    — `call` nodes (function invocations)
// =============================================================================

use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind,
    Visibility,
};
use crate::types::ExtractionResult;
use tree_sitter::{Node, Parser};

// Rule-like builtins that define rule types → emit as Function
const RULE_FUNCS: &[&str] = &["rule", "macro", "aspect", "repository_rule"];
// Struct-like builtins
const STRUCT_FUNCS: &[&str] = &["provider", "struct"];

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_starlark::LANGUAGE.into())
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

    walk(tree.root_node(), src, &mut symbols, &mut refs, None);

    ExtractionResult::new(symbols, refs, tree.root_node().has_error())
}

fn walk(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    match node.kind() {
        "function_definition" => {
            let idx = extract_function(node, src, symbols, parent_idx);
            // Recurse into body for nested assignments and calls.
            walk_children(node, src, symbols, refs, idx.or(parent_idx));
        }
        "assignment" => {
            let idx = extract_assignment(node, src, symbols, refs, parent_idx);
            // Also recurse so nested structures inside RHS are captured.
            walk_children(node, src, symbols, refs, idx.or(parent_idx));
        }
        "call" => {
            extract_call(node, src, refs, parent_idx);
            walk_children(node, src, symbols, refs, parent_idx);
        }
        _ => {
            walk_children(node, src, symbols, refs, parent_idx);
        }
    }
}

fn extract_function(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = text(name_node, src);
    if name.is_empty() {
        return None;
    }
    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("def {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index: parent_idx,
    });
    Some(idx)
}

fn extract_assignment(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    // assignment.left is a `pattern` — usually an identifier.
    let left = node.child_by_field_name("left")?;
    let name = match left.kind() {
        "identifier" => text(left, src),
        _ => {
            // Try the first named child that's an identifier.
            let count = left.child_count();
            (0..count)
                .filter_map(|i| left.child(i))
                .find(|c| c.kind() == "identifier")
                .map(|c| text(c, src))
                .unwrap_or_default()
        }
    };
    if name.is_empty() {
        return None;
    }

    // Inspect the RHS to determine kind.
    let right = node.child_by_field_name("right");
    let (kind, sig) = if let Some(rhs) = right {
        classify_rhs(rhs, src, &name)
    } else {
        (SymbolKind::Variable, None)
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: sig,
        doc_comment: None,
        scope_path: None,
        parent_index: parent_idx,
    });

    // Emit a call ref for the RHS callee if applicable.
    if let Some(rhs) = right {
        if let Some(callee) = rhs_callee_name(rhs, src) {
            let sym_idx = parent_idx.unwrap_or(0);
            refs.push(ExtractedRef {
                source_symbol_index: sym_idx,
                target_name: callee,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
    }

    Some(idx)
}

fn classify_rhs(node: Node, src: &[u8], lhs_name: &str) -> (SymbolKind, Option<String>) {
    // Unwrap expression wrapper if needed.
    let actual = unwrap_expression(node);
    if actual.kind() == "call" {
        if let Some(callee_node) = actual.child_by_field_name("function") {
            let callee = text(callee_node, src);
            if RULE_FUNCS.contains(&callee.as_str()) {
                return (SymbolKind::Function, Some(format!("{} = {}(...)", lhs_name, callee)));
            }
            if STRUCT_FUNCS.contains(&callee.as_str()) {
                return (SymbolKind::Struct, Some(format!("{} = {}(...)", lhs_name, callee)));
            }
            if callee.ends_with("_test") {
                return (SymbolKind::Test, Some(format!("{} = {}(...)", lhs_name, callee)));
            }
        }
    }
    (SymbolKind::Variable, None)
}

/// If the RHS resolves to a call, return the callee name.
fn rhs_callee_name(node: Node, src: &[u8]) -> Option<String> {
    let actual = unwrap_expression(node);
    if actual.kind() == "call" {
        if let Some(callee_node) = actual.child_by_field_name("function") {
            let name = text(callee_node, src);
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// Unwrap an `expression` or `primary_expression` node to get the actual content.
fn unwrap_expression(node: Node) -> Node {
    match node.kind() {
        "expression" | "primary_expression" => {
            if node.named_child_count() == 1 {
                if let Some(child) = node.named_child(0) {
                    return child;
                }
            }
            node
        }
        _ => node,
    }
}

fn extract_call(
    node: Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    let sym_idx = parent_idx.unwrap_or(0);
    // call.function is a `primary_expression` — could be identifier or attribute.
    if let Some(fn_node) = node.child_by_field_name("function") {
        let name = callee_text(fn_node, src);
        if !name.is_empty() {
            // Special-case `load(...)` → Imports ref.
            if name == "load" {
                extract_load_refs(node, src, sym_idx, refs);
                return;
            }

            // Skip method calls on literal values — string/list/dict literal
            // method calls (`"\n".join(args)`, `[].append(x)`, `{}.items()`)
            // bind to builtin types, never to user-defined symbols, so the
            // ref produces nothing but unresolved noise.
            if starts_with_literal(&name) {
                return;
            }

            // Sub-call chains like `_normalize(path).split` or
            // `gather_runfiles(...).merge` produce a callee text containing
            // parens/brackets/newlines. The chain root isn't a single
            // identifier so the ref can't bind to a user symbol — skip.
            if name.contains('(') || name.contains('[') || name.contains('\n') {
                return;
            }

            // Build a MemberChain for dotted attribute access (`ctx.actions.run`).
            // target_name is kept as the full dotted string (e.g. "ctx.actions.run_shell")
            // so that the predicate-based external fallback still fires when the chain
            // resolver misses. The chain carries structured segments for the chain walker.
            let chain = if fn_node.kind() == "attribute" {
                build_attribute_chain(fn_node, src)
            } else {
                None
            };
            let target_name = name;

            refs.push(ExtractedRef {
                source_symbol_index: sym_idx,
                target_name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
                chain,
                byte_offset: fn_node.start_byte() as u32,
                            namespace_segments: Vec::new(),
});
        }
    }
}

/// Build a `MemberChain` from a tree-sitter `attribute` node.
///
/// For `ctx.actions.run_shell` the tree looks like:
/// ```text
/// attribute
///   object: attribute
///     object: identifier "ctx"
///     attribute: identifier "actions"
///   attribute: identifier "run_shell"
/// ```
/// We produce segments: [ctx (Identifier), actions (Property), run_shell (Property)].
fn build_attribute_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    collect_attribute_segments(node, src, &mut segments)?;
    if segments.len() < 2 {
        return None;
    }
    Some(MemberChain { segments })
}

fn collect_attribute_segments(node: Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" => {
            let name = text(node, src);
            if name.is_empty() {
                return None;
            }
            let kind = SegmentKind::Identifier;
            segments.push(ChainSegment {
                name,
                node_kind: "identifier".to_string(),
                kind,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }
        "attribute" => {
            let object = node.child_by_field_name("object")?;
            let attribute = node.child_by_field_name("attribute")?;
            collect_attribute_segments(object, src, segments)?;
            let attr_name = text(attribute, src);
            if attr_name.is_empty() {
                return None;
            }
            segments.push(ChainSegment {
                name: attr_name,
                node_kind: "attribute".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }
        // Starlark `primary_expression` wrappers — unwrap one level.
        "primary_expression" => {
            if let Some(child) = node.named_child(0) {
                collect_attribute_segments(child, src, segments)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn extract_load_refs(
    call_node: Node,
    src: &[u8],
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // load("label", "sym1", alias="sym2", ...)
    if let Some(args) = call_node.child_by_field_name("arguments") {
        let mut cursor = args.walk();
        let children: Vec<Node> = args.named_children(&mut cursor).collect();
        if children.is_empty() {
            return;
        }
        // First argument is the module label.
        let module_label = string_value(children[0], src);
        if module_label.is_empty() {
            return;
        }
        refs.push(ExtractedRef {
            source_symbol_index: sym_idx,
            target_name: module_label.clone(),
            kind: EdgeKind::Imports,
            line: call_node.start_position().row as u32,
            module: Some(module_label.clone()),
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
        // Remaining args are symbol names or `alias = "sym"` pairs.
        // For aliased loads use the alias (the name used inside this file)
        // as target_name — the resolver needs to match local invocations
        // like `_npm_local_package_store(...)` back to the import. The
        // upstream/source name is preserved in the Imports ref's `module`
        // field via the encoded module label.
        for arg in &children[1..] {
            let sym = match arg.kind() {
                "string" => string_value(*arg, src),
                "keyword_argument" => arg
                    .child_by_field_name("name")
                    .map(|n| text(n, src))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| {
                        arg.child_by_field_name("value")
                            .map(|v| string_value(v, src))
                            .unwrap_or_default()
                    }),
                _ => String::new(),
            };
            if !sym.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: sym,
                    kind: EdgeKind::Imports,
                    line: call_node.start_position().row as u32,
                    module: Some(module_label.clone()),
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
    }
}

/// Extract string content from a `string` node (strips quotes).
fn string_value(node: Node, src: &[u8]) -> String {
    let raw = text(node, src);
    raw.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// Get the text of a callee node; for `attribute` (a.b), return "a.b".
fn callee_text(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" => text(node, src),
        "attribute" => text(node, src),
        _ => {
            // Try named child identifier.
            if node.named_child_count() >= 1 {
                if let Some(first) = node.named_child(0) {
                    if first.kind() == "identifier" {
                        return text(first, src);
                    }
                }
            }
            String::new()
        }
    }
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
        walk(child, src, symbols, refs, parent_idx);
    }
}

fn text(node: Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").trim().to_string()
}

/// True when a callee chain text starts with a string/list/dict literal —
/// e.g. `"\n".join(...)`, `[].append(x)`, `{}.items()`. Method calls on
/// literals always bind to builtin types, never user symbols.
fn starts_with_literal(name: &str) -> bool {
    let first = name.chars().next();
    matches!(first, Some('"') | Some('\'') | Some('[') | Some('{') | Some('('))
}
