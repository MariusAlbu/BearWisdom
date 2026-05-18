// =============================================================================
// languages/groovy/calls.rs  —  Receiver chain construction and call traversal
//
// Builds MemberChain values for `method_invocation` receivers, scans bodies for
// typed local declarations so chain segments carry `declared_type`, and walks
// subtrees collecting calls.
// =============================================================================

use crate::types::{ChainSegment, ExtractedRef, MemberChain, SegmentKind};
use super::predicates;
use super::node_helpers::{named_field_text, node_text};
use std::collections::HashMap;
use tree_sitter::Node;

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
            if name.is_empty() || predicates::is_groovy_keyword(&name) {
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
                        let base_type = type_name.split('<').next().unwrap_or(&type_name).trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.').to_string();
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
                        let base_type = type_name.split('<').next().unwrap_or(&type_name).trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.').to_string();
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
