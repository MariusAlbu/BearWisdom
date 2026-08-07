// =============================================================================
// engine/extension_method — extension-method fallback for a missed instance
// member: a callable declared OUTSIDE the receiver's type whose signature
// marks its first parameter with `this`.
// =============================================================================

use crate::indexer::resolve::engine::contract::chain_walker::{
    extension_receiver_type, parse_type_head_and_args,
};
use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup};
use crate::type_checker::core::types::TypeArena;

use super::chain::{head_qname, is_callable, Receiver, MAX_SUPERTYPE_DEPTH};

/// An EXTENSION METHOD found after an instance-member miss: a method declared
/// OUTSIDE the receiver's type whose signature marks its first parameter with
/// `this` (`void UseSnapshot(this ModelBuilder builder, …)`) and whose
/// receiver-parameter head names the receiver's type — or one of its
/// supertypes, so an extension on a base applies to the derived receiver.
/// The `(this ` signature shape is the gate: only extension declarations
/// carry it, so the probe is structurally inert for every other language.
///
/// Among matching candidates the CLOSEST receiver match wins — an overload
/// set declares one `member` per receiver shape (`Should(this IEnumerable…)`,
/// `Should(this object…)`), and only the closest match's return type is the
/// receiver's real yield. `implicit_root_types` closes the climb: every
/// declaration inherits the language's root without writing it, so a
/// root-receiver extension applies to a receiver with no base list. Two
/// DIFFERENT declaring qnames at the same best distance decline as ambiguous.
pub(super) fn lookup_extension_method(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv: Receiver,
    member: &str,
    implicit_root_types: &[&str],
) -> Option<Symbol> {
    let recv_head = head_qname(arena, recv.ty)?;
    // The receiver's simple name plus its supertypes', climbed breadth-first
    // and bounded like the member walk itself. BFS order = distance order.
    let mut recv_names: Vec<String> = Vec::new();
    let mut frontier: Vec<String> = vec![recv_head.clone()];
    for _ in 0..=MAX_SUPERTYPE_DEPTH {
        let mut next = Vec::new();
        for head in &frontier {
            let simple = head.rsplit('.').next().unwrap_or(head);
            if !recv_names.iter().any(|n| n == simple) {
                recv_names.push(simple.to_string());
            }
            for parent in lookup.parent_class_qnames(head) {
                next.push(parent.clone());
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    for root in implicit_root_types {
        if !recv_names.iter().any(|n| n == root) {
            recv_names.push((*root).to_string());
        }
    }
    let mut hit: Option<(usize, Symbol)> = None;
    for cand in lookup.by_name(member) {
        if !is_callable(&cand.kind) {
            continue;
        }
        let Some(sig) = cand.signature.as_deref() else {
            continue;
        };
        let Some(recv_param) = extension_receiver_type(sig) else {
            continue;
        };
        let (param_head, _args) = parse_type_head_and_args(&recv_param);
        let param_simple = param_head.rsplit('.').next().unwrap_or(param_head);
        let Some(dist) = recv_names.iter().position(|n| n == param_simple) else {
            continue;
        };
        match &hit {
            None => hit = Some((dist, cand.clone())),
            Some((best, h)) => {
                if dist < *best {
                    hit = Some((dist, cand.clone()));
                } else if dist == *best && h.qualified_name != cand.qualified_name {
                    return None;
                }
            }
        }
    }
    hit.map(|(_, s)| s)
}
