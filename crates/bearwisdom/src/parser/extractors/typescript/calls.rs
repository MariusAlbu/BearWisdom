use super::helpers::node_text;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

pub(super) fn extract_calls(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                if let Some(func_node) = child.child_by_field_name("function") {
                    let chain = build_chain(func_node, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| callee_name_fallback(func_node, src));

                    super::super::emit_chain_type_ref(&chain, source_symbol_index, &func_node, refs);
                    if !target_name.is_empty() && target_name != "undefined" {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: func_node.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }
            // `sql\`SELECT ...\`` / `gql\`query { ... }\`` — tagged template expression.
            // The first child (the tag) is the function being called.
            "tagged_template_expression" => {
                // tree-sitter field: "tag" is the function, "template" is the literal.
                let tag_node = child.child_by_field_name("tag");
                if let Some(tag) = tag_node {
                    let chain = build_chain(tag, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| callee_name_fallback(tag, src));
                    super::super::emit_chain_type_ref(&chain, source_symbol_index, &tag, refs);
                    if !target_name.is_empty() && target_name != "undefined" {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: tag.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                // Recurse for any nested calls inside the template.
                extract_calls(&child, src, source_symbol_index, refs);
            }
            // JSX: `<Component />` or `<Component>...</Component>` is a call
            // to the component function/class.  Emit a Calls edge for user-
            // defined components (PascalCase) — lowercase tags are HTML intrinsics.
            "jsx_self_closing_element" | "jsx_opening_element" => {
                // The tag name is the first named child (identifier or member_expression).
                let tag = child
                    .child_by_field_name("name")
                    .or_else(|| child.named_child(0));
                if let Some(tag_node) = tag {
                    let tag_name = node_text(tag_node, src);
                    // PascalCase = user component; lowercase = HTML intrinsic.
                    if !tag_name.is_empty()
                        && tag_name.chars().next().map_or(false, |c| c.is_uppercase())
                    {
                        // Member expression: `<Foo.Bar />` → chain + TypeRef.
                        let chain = build_chain(tag_node, src);
                        let target = chain
                            .as_ref()
                            .and_then(|c| c.segments.last())
                            .map(|s| s.name.clone())
                            .unwrap_or(tag_name);
                        super::super::emit_chain_type_ref(&chain, source_symbol_index, &tag_node, refs);
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: target,
                            kind: EdgeKind::Calls,
                            line: tag_node.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }
            _ => {
                extract_calls(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Build a structured member access chain from tree-sitter AST nodes.
///
/// Recursively walks nested `member_expression` nodes (left-recursive) to
/// produce a `Vec<ChainSegment>` from inside-out.
///
/// `this.repo.findOne()` tree structure:
/// ```text
/// member_expression @function
///   member_expression @object
///     this @object
///     property_identifier "repo"
///   property_identifier "findOne"
/// ```
/// produces: `[this, repo, findOne]`
pub(super) fn build_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

pub(super) fn build_chain_inner(
    node: Node,
    src: &[u8],
    segments: &mut Vec<ChainSegment>,
) -> Option<()> {
    match node.kind() {
        "this" | "super" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "member_expression" => {
            let object = node.child_by_field_name("object")?;
            let property = node.child_by_field_name("property")?;

            // Check for optional chaining: `?.` between object and property.
            let is_optional = (0..node.child_count()).any(|i| {
                node.child(i)
                    .map(|c| c.kind() == "optional_chain")
                    .unwrap_or(false)
            });

            // Recurse into the object to build the prefix chain.
            build_chain_inner(object, src, segments)?;

            segments.push(ChainSegment {
                name: node_text(property, src),
                node_kind: property.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: is_optional,
            });
            Some(())
        }

        "subscript_expression" => {
            // `this.handlers['click']`
            let object = node.child_by_field_name("object")?;
            let index = node.child_by_field_name("index")?;

            build_chain_inner(object, src, segments)?;

            segments.push(ChainSegment {
                name: node_text(index, src),
                node_kind: "subscript_expression".to_string(),
                kind: SegmentKind::ComputedAccess,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "call_expression" => {
            // Nested call in a chain: `a.b().c()` — the object is a call_expression.
            // Walk into its function child to continue the chain.
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, src, segments)
        }

        // Unknown node — can't build a chain from this.
        _ => None,
    }
}

/// Fallback for when `build_chain()` returns `None`.
pub(super) fn callee_name_fallback(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "member_expression" => {
            // Fall back to just the property name (last segment).
            node.child_by_field_name("property")
                .map(|n| node_text(n, src))
                .unwrap_or_else(|| node_text(node, src))
        }
        _ => {
            let t = node_text(node, src);
            t.rsplit('.').next().unwrap_or(&t).to_string()
        }
    }
}
