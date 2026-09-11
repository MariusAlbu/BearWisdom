//! Neutral tree walkers shared by language-owned callback adapters.
use crate::types::SourceSpan;
use tree_sitter::Node;

pub(crate) fn span(node: Node) -> SourceSpan {
    SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    }
}

pub(crate) fn named_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

pub(crate) fn node_text<'a>(source: &'a [u8], node: Node) -> Option<&'a str> {
    std::str::from_utf8(source.get(node.start_byte()..node.end_byte())?).ok()
}

pub(crate) fn collect_boundaries<'tree>(
    body: Node<'tree>,
    is_boundary: fn(&str) -> bool,
) -> Vec<Node<'tree>> {
    fn visit<'tree>(
        node: Node<'tree>,
        body: Node<'tree>,
        is_boundary: fn(&str) -> bool,
        out: &mut Vec<Node<'tree>>,
    ) {
        if node != body && is_boundary(node.kind()) {
            out.push(node);
            return;
        }
        for child in named_children(node) {
            visit(child, body, is_boundary, out);
        }
    }
    let mut out = Vec::new();
    visit(body, body, is_boundary, &mut out);
    out
}

pub(crate) fn scoped_range(node: Node, body: Node, is_scope: fn(&str) -> bool) -> SourceSpan {
    let start = node.start_byte() as u32;
    let mut current = Some(node);
    while let Some(candidate) = current {
        if candidate != body && is_scope(candidate.kind()) {
            return SourceSpan {
                start,
                end: candidate.end_byte() as u32,
            };
        }
        current = candidate.parent();
    }
    SourceSpan {
        start,
        end: body.end_byte() as u32,
    }
}
