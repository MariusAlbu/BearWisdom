// =============================================================================
// languages/groovy/calls.rs  —  Receiver chain construction and call traversal
//
// Builds MemberChain values for `method_invocation` receivers, scans bodies for
// typed local declarations so chain segments carry `declared_type`, and walks
// subtrees collecting calls.
// =============================================================================

use super::node_helpers::{named_field_text, node_text};
use super::predicates;
use crate::types::{CallArg, ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use std::collections::HashMap;
use tree_sitter::Node;

/// Maximum nesting depth for recursive `CallArg` construction. Arguments
/// deeper than this collapse to `CallArg::Other` rather than recursing further.
const MAX_ARG_DEPTH: u32 = 8;

/// Extract the positional arguments from a `method_invocation`'s `arguments`
/// (`argument_list`) node.
///
/// Walks named children of the argument list, converting each to a `CallArg`.
/// Handles string/number/boolean/null literals, bare identifiers, and the
/// recursive expression shapes Groovy expresses (ternary, array literal,
/// subscript, binary). Groovy has no `await` and the grammar models no spread
/// argument node, so those shapes never arise. Recursion is capped at
/// `MAX_ARG_DEPTH` levels.
pub(super) fn extract_call_args(call_node: &Node, src: &str) -> Vec<CallArg> {
    let Some(args_node) = call_node.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut cursor = args_node.walk();
    for child in args_node.named_children(&mut cursor) {
        result.push(extract_arg(&child, src, 0));
    }
    result
}

/// Convert a single argument expression node to a `CallArg`, recursing for
/// composite expression kinds up to `MAX_ARG_DEPTH`.
fn extract_arg(node: &Node, src: &str, depth: u32) -> CallArg {
    if depth >= MAX_ARG_DEPTH {
        return CallArg::Other;
    }
    match node.kind() {
        "string_literal" => {
            // `"text"` or `'text'` — strip surrounding quotes.
            let raw = node_text(node, src);
            let inner = raw
                .trim_start_matches(['"', '\''])
                .trim_end_matches(['"', '\''])
                .to_string();
            CallArg::StringLit(inner)
        }
        "identifier" => CallArg::Ident(node_text(node, src).to_string()),
        "decimal_integer_literal"
        | "decimal_floating_point_literal"
        | "hex_integer_literal"
        | "hex_floating_point_literal"
        | "octal_integer_literal"
        | "binary_integer_literal"
        | "character_literal" => CallArg::Literal(node_text(node, src).to_string()),
        "true" | "false" | "null_literal" => CallArg::Literal(node.kind().to_string()),
        // `cond ? consequence : alternative` — the condition's type does not
        // affect the value type; only the two branches are preserved.
        "ternary_expression" => {
            let then_branch = node
                .child_by_field_name("consequence")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let else_branch = node
                .child_by_field_name("alternative")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Ternary {
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            }
        }
        // `[elem0, elem1, ...]` — recurse on each named child element.
        "array_literal" => {
            let mut cursor = node.walk();
            let elements = node
                .named_children(&mut cursor)
                .map(|child| extract_arg(&child, src, depth + 1))
                .collect();
            CallArg::ArrayLiteral { elements }
        }
        // `container[index]` — recurse on both sides.
        "array_access" => {
            let container = node
                .child_by_field_name("array")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let index = node
                .child_by_field_name("index")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::IndexAccess {
                container: Box::new(container),
                index: Box::new(index),
            }
        }
        // `left op right` — capture operator text and recurse on operands.
        "binary_expression" => {
            let op = node
                .child_by_field_name("operator")
                .map(|n| node_text(&n, src).to_string())
                .unwrap_or_default();
            let left = node
                .child_by_field_name("left")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let right = node
                .child_by_field_name("right")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            }
        }
        _ => CallArg::Other,
    }
}

/// Build a MemberChain from the `object` field of a `method_invocation` node
/// plus the final call segment name.
///
/// The chain walker and the external classifier both consume MemberChain.
/// For `file.path.endsWith(s)` with declared type `File` on `file`:
///   segments = [file (Identifier, declared_type=File), path (Property), endsWith (Property)]
///
/// Returns `None` when the receiver is too complex to model (e.g. a closure
/// literal, a cast expression) — the call is then emitted as a bare ref.
pub(super) fn build_receiver_chain(
    obj_node: &Node,
    final_method: &str,
    src: &str,
    local_types: &HashMap<String, String>,
) -> Option<MemberChain> {
    let mut segments: Vec<ChainSegment> = Vec::new();
    collect_receiver_segments(obj_node, src, local_types, &mut segments, 0)?;

    // The final method/property call is the chain leaf.
    segments.push(ChainSegment {
        name: final_method.to_string(),
        node_kind: "method_invocation".to_string(),
        kind: SegmentKind::Property,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call: false,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    });

    if segments.len() < 2 {
        return None;
    }

    Some(MemberChain { segments })
}

/// Recursively collect chain segments from a receiver expression.
///
/// `depth` guards against pathologically deep chains (e.g. 20-segment builder
/// APIs) blowing the stack or producing noise the chain walker can't use.
fn collect_receiver_segments(
    node: &Node,
    src: &str,
    local_types: &HashMap<String, String>,
    segments: &mut Vec<ChainSegment>,
    depth: usize,
) -> Option<()> {
    // Cap recursion: chains deeper than 8 segments aren't useful for
    // type inference since we'd lose the root type anyway.
    if depth > 8 {
        return None;
    }

    match node.kind() {
        "identifier" => {
            let name = node_text(node, src).to_string();
            if predicates::is_interpolation_marker(&name) || predicates::is_groovy_keyword(&name) {
                return None;
            }
            let declared_type = local_types.get(&name).cloned();
            segments.push(ChainSegment {
                name,
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }
        "this" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }
        "field_access" => {
            // field_access has `object` and `field` fields.
            let inner_obj = node.child_by_field_name("object")?;
            let field_node = node.child_by_field_name("field")?;
            let field_name = node_text(&field_node, src).to_string();
            if field_name.is_empty() {
                return None;
            }
            collect_receiver_segments(&inner_obj, src, local_types, segments, depth + 1)?;
            segments.push(ChainSegment {
                name: field_name,
                node_kind: "field_access".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }
        "method_invocation" => {
            // Chained call: `foo().bar()`. Treat `foo()` as a call segment.
            // The method's name is the segment; the object of `foo()` is the
            // sub-chain. Recurse into the sub-chain first.
            let inner_name = match named_field_text(node, "name", src) {
                Some(n) => n,
                None => return None,
            };
            // A `$`-named segment is the GString interpolation marker, not a
            // real receiver; reject the chain so the leaf call falls back to a
            // bare ref instead of carrying a `$` segment.
            if predicates::is_interpolation_marker(&inner_name) {
                return None;
            }
            if let Some(inner_obj) = node.child_by_field_name("object") {
                collect_receiver_segments(&inner_obj, src, local_types, segments, depth + 1)?;
            } else {
                // Bare call at the root of the chain (e.g. `GradleRunner.create()`).
                // Treat the method name itself as a type-access root segment so the
                // chain walker can probe static methods on it.
                segments.push(ChainSegment {
                    name: inner_name.clone(),
                    node_kind: "method_invocation".to_string(),
                    kind: SegmentKind::TypeAccess,
                    declared_type: None,
                    type_args: Vec::new(),
                    optional_chaining: false,
                    byte_offset: 0,
                    declared_type_id: None,
                    is_call: false,
                    call_args: Vec::new(),
                    type_arg_ids: Vec::new(),
                });
                return Some(());
            }
            segments.push(ChainSegment {
                name: inner_name,
                node_kind: "method_invocation".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }
        _ => {
            // Parenthesized expressions, casts, new expressions, etc.
            // Fall back to None — the call is emitted as a bare ref.
            None
        }
    }
}

/// Scan a method/function/closure body for typed local variable and for-loop
/// declarations, returning a map of variable-name → declared type name.
///
/// Recognizes:
///   `File file = ...`          (local_variable_declaration with explicit type)
///   `for (File file : files)`  (enhanced_for_statement with explicit type)
///
/// `def` declarations and bare assignments are excluded — the type is unknown
/// at extraction time for those.
pub(super) fn scan_local_types(root: &Node, src: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    collect_local_types(root, src, &mut map);
    map
}

fn collect_local_types(node: &Node, src: &str, map: &mut HashMap<String, String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "local_variable_declaration" => {
                // Grammar: local_variable_declaration has `type` and `declarator` fields.
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_name = node_text(&type_node, src).to_string();
                    // Skip `def` and primitive type keywords — their type is not a class.
                    if !type_name.is_empty()
                        && type_name != "def"
                        && !type_name.chars().next().map_or(false, |c| c.is_lowercase())
                    {
                        // Strip generic parameters: `List<File>` → `List`
                        let base_type = type_name
                            .split('<')
                            .next()
                            .unwrap_or(&type_name)
                            .trim_end_matches(|c: char| {
                                !c.is_alphanumeric() && c != '_' && c != '.'
                            })
                            .to_string();
                        // Collect each declarator's variable name.
                        let mut dc = child.walk();
                        for decl in child.children(&mut dc) {
                            if decl.kind() == "variable_declarator" {
                                if let Some(name_node) = decl.child_by_field_name("name") {
                                    let var_name = node_text(&name_node, src).to_string();
                                    if !var_name.is_empty() {
                                        map.insert(var_name, base_type.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                collect_local_types(&child, src, map);
            }
            "enhanced_for_statement" => {
                // Grammar: enhanced_for_statement has `type` and `name` fields.
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_name = node_text(&type_node, src).to_string();
                    if !type_name.is_empty()
                        && type_name != "def"
                        && !type_name.chars().next().map_or(false, |c| c.is_lowercase())
                    {
                        let base_type = type_name
                            .split('<')
                            .next()
                            .unwrap_or(&type_name)
                            .trim_end_matches(|c: char| {
                                !c.is_alphanumeric() && c != '_' && c != '.'
                            })
                            .to_string();
                        if let Some(name_node) = child.child_by_field_name("name") {
                            let var_name = node_text(&name_node, src).to_string();
                            if !var_name.is_empty() {
                                map.insert(var_name, base_type);
                            }
                        }
                    }
                }
                collect_local_types(&child, src, map);
            }
            _ => {
                collect_local_types(&child, src, map);
            }
        }
    }
}

/// Emit an `Instantiates` ref for a `new Type(args)` node.
///
/// The `type` field holds a `_simple_type` whose text may be qualified
/// (`pkg.Type`) and may carry generic args (`List<String>`); the ref's
/// `target_name` is the bare simple name so it matches the indexed class
/// symbol. The ref is anchored at the type node's start byte (the invariant
/// `byte_offset == (line, col)` position) so the flow correlator can bind a
/// `def x = new Type(...)` initializer to this construction.
fn emit_instantiates(node: &Node, src: &str, source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    let Some(type_node) = node.child_by_field_name("type") else {
        return;
    };
    let raw = node_text(&type_node, src);
    let simple = raw
        .split('<')
        .next()
        .unwrap_or(raw)
        .rsplit('.')
        .next()
        .unwrap_or(raw)
        .trim();
    if simple.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: simple.to_string(),
        kind: EdgeKind::Instantiates,
        line: type_node.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: type_node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

/// Walk subtree collecting `method_invocation` nodes and emit Calls refs.
pub(super) fn visit_for_calls(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
    local_types: &HashMap<String, String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_invocation" => {
                super::ast_visit::extract_call(&child, src, source_idx, refs, local_types);
                visit_for_calls(&child, src, source_idx, refs, local_types);
            }
            "object_creation_expression" => {
                // `new Type(args)` — emit an Instantiates ref at the type node so
                // the constructed type both produces an edge and roots forward
                // flow inference for `def x = new Type(...)`. Recurse to capture
                // nested constructors and calls inside the argument list.
                emit_instantiates(&child, src, source_idx, refs);
                visit_for_calls(&child, src, source_idx, refs, local_types);
            }
            "enhanced_for_statement" | "local_variable_declaration" => {
                // Merge any new local type declarations scoped to this block
                // into a child map. We collect_local_types to find new bindings
                // introduced inside sub-blocks (closures, nested for loops) and
                // visit the body with the extended map.
                let mut child_types = local_types.clone();
                collect_local_types(&child, src, &mut child_types);
                visit_for_calls(&child, src, source_idx, refs, &child_types);
            }
            _ => {
                visit_for_calls(&child, src, source_idx, refs, local_types);
            }
        }
    }
}
