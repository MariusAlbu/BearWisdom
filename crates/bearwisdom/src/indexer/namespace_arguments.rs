//! Source-owned semantic operands; display payloads are not a fallback here.
use super::*;
use crate::types::CallArg;
use tree_sitter::Node;

pub(crate) type Table = HashMap<u32, Option<Vec<CallArg>>>;

pub(super) fn capture(node: Node, source: &[u8], forms: &Forms, table: &mut Table) {
    let Some(&(_, field)) = forms
        .locals
        .calls
        .iter()
        .find(|&&(kind, _)| kind == node.kind())
    else {
        return;
    };
    let Some(mut callee) = node.child_by_field_name(field) else {
        return;
    };
    loop {
        if let Some(&(_, field)) = forms
            .locals
            .callable_wrappers
            .iter()
            .find(|&&(kind, _)| kind == callee.kind())
        {
            let Some(inner) = callee.child_by_field_name(field) else {
                break;
            };
            callee = inner;
        } else if forms.argument_groups.contains(&callee.kind()) {
            let mut cursor = callee.walk();
            let mut children = callee.named_children(&mut cursor).filter(|n| !n.is_extra());
            let Some(inner) = children.next() else {
                break;
            };
            if children.next().is_some() {
                break;
            }
            callee = inner;
        } else {
            break;
        }
    }
    let selector = forms
        .locals
        .selectors
        .iter()
        .find(|&&(kind, _)| kind == callee.kind())
        .and_then(|&(_, field)| callee.child_by_field_name(field));
    let supported = selector.is_some() || forms.locals.identifiers.contains(&callee.kind());
    let byte = selector.unwrap_or(callee).start_byte() as u32;
    let arguments = (supported
        && !node.has_error()
        && node.child_by_field_name("arguments").is_some())
    .then(|| {
        crate::languages::common::call_args::extract_call_args_with_places(
            &node,
            source,
            forms.borrows,
            Some(forms.places),
        )
    });
    // A nested value invocation can share its first token with another call.
    // Neither duplicate selectors nor unsupported callees authorize first-wins.
    table
        .entry(byte)
        .and_modify(|old| *old = None)
        .or_insert(arguments);
}

#[cfg(test)]
#[path = "namespace_arguments_tests.rs"]
mod tests;
