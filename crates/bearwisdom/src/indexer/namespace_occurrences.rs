//! Attest call selector addresses from syntax before binding capture. Legacy
//! zero offsets cannot become identity by searching the source for a spelling.
use super::*;
use tree_sitter::Node;

struct Part<'tree> {
    node: Node<'tree>,
    call: Option<Node<'tree>>,
}

pub(super) fn borrow_site<'tree>(
    node: Node<'tree>,
    forms: &Forms,
) -> Option<(
    crate::types::SourceSpan,
    crate::type_checker::core::types::Mutability,
    Node<'tree>,
)> {
    let (_, mutable) = forms.borrows?.capture(node)?;
    let span = crate::types::SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    };
    let mut owner = node.parent()?;
    loop {
        if owner.kind() == forms.function {
            return Some((span, mutable, owner));
        }
        // Closures need their own declaration identity, not an enclosing namesake.
        if forms.locals.closures.contains(&owner.kind())
            || forms.locals.barriers.contains(&owner.kind())
        {
            return None;
        }
        owner = owner.parent()?;
    }
}

/// Only a dot-call has an implicit receiver adjustment. UFCS and function
/// values have different argument positions and must not inherit this fact.
pub(super) fn method_site<'tree>(node: Node<'tree>, forms: &Forms) -> Option<(u32, Node<'tree>)> {
    let &(_, function) = forms
        .locals
        .calls
        .iter()
        .find(|&&(kind, _)| kind == node.kind())?;
    let mut callee = node.child_by_field_name(function)?;
    while let Some(&(_, field)) = forms
        .locals
        .callable_wrappers
        .iter()
        .find(|&&(kind, _)| kind == callee.kind())
    {
        callee = callee.child_by_field_name(field)?;
    }
    let &(_, _, field) = forms
        .locals
        .receivers
        .iter()
        .find(|&&(kind, _, _)| kind == callee.kind())?;
    let byte = callee.child_by_field_name(field)?.start_byte() as u32;
    let mut owner = node.parent()?;
    loop {
        if owner.kind() == forms.function {
            return Some((byte, owner));
        }
        owner = owner.parent()?;
    }
}

pub(super) fn stamp(root: Node, source: &[u8], forms: &Forms, refs: &mut [ExtractedRef]) {
    for reference in refs
        .iter_mut()
        .filter(|r| r.kind == crate::types::EdgeKind::Calls)
    {
        let Some(chain) = &mut reference.chain else {
            continue;
        };
        let byte = reference.byte_offset as usize;
        let Some(mut node) = root.named_descendant_for_byte_range(byte, byte + 1) else {
            continue;
        };
        let mut candidates = Vec::new();
        loop {
            if forms
                .locals
                .calls
                .iter()
                .any(|&(kind, _)| kind == node.kind())
            {
                if let Some(parts) = flatten(node, forms) {
                    if parts.len() == chain.segments.len() {
                        candidates.push(parts);
                    }
                }
            }
            let Some(parent) = node.parent().filter(|p| p.start_byte() == byte) else {
                break;
            };
            node = parent;
        }
        // Multiple same-shaped call expressions need richer extraction identity.
        // Never select one by matching its last segment's display name.
        if candidates.len() != 1 {
            continue;
        }
        for (segment, part) in chain.segments.iter_mut().zip(candidates.pop().unwrap()) {
            segment.byte_offset = part.node.start_byte() as u32;
            segment.is_call = part.call.is_some();
            if part.call.is_none() && forms.places.named_selectors.contains(&part.node.kind()) {
                if let Ok(text) = part.node.utf8_text(source) {
                    segment.name = text
                        .strip_prefix(forms.raw_prefix)
                        .unwrap_or(text)
                        .to_owned();
                }
            }
            if let Some(call) = part.call {
                segment.call_args =
                    crate::languages::common::call_args::extract_call_args_with_borrows(
                        &call,
                        source,
                        forms.borrows,
                    );
            }
        }
    }
}

fn flatten<'tree>(node: Node<'tree>, forms: &Forms) -> Option<Vec<Part<'tree>>> {
    let local = forms.locals;
    if node.kind() == forms.traits.qualified_wrapper {
        return Some(vec![Part { node, call: None }]);
    }
    if forms.identifiers.contains(&node.kind()) {
        return Some(vec![Part { node, call: None }]);
    }
    if let Some(&(_, field)) = local.calls.iter().find(|&&(kind, _)| kind == node.kind()) {
        let mut parts = flatten(node.child_by_field_name(field)?, forms)?;
        parts.last_mut()?.call = Some(node);
        return Some(parts);
    }
    if let Some(&(_, field)) = local
        .callable_wrappers
        .iter()
        .find(|&&(kind, _)| kind == node.kind())
    {
        return flatten(node.child_by_field_name(field)?, forms);
    }
    if let Some(&(_, base, selector)) = local
        .receivers
        .iter()
        .chain(forms.path_nodes)
        .find(|&&(kind, _, _)| kind == node.kind())
    {
        let mut parts = flatten(node.child_by_field_name(base)?, forms)?;
        parts.push(Part {
            node: node.child_by_field_name(selector)?,
            call: None,
        });
        return Some(parts);
    }
    if local.value_wrappers.contains(&node.kind()) {
        return flatten(node.named_child(0)?, forms);
    }
    None
}

#[cfg(test)]
#[path = "namespace_occurrences_tests.rs"]
mod tests;
