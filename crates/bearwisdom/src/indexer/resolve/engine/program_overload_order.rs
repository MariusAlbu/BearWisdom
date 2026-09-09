//! Candidate order is a source fact, never a physical-row or hash ordering.
use super::*;
use crate::indexer::lexical::globals::member_surface::{OverloadOrder, SignatureOrder};

pub(in crate::indexer::resolve::engine::program_view) struct Fact {
    pub owner: i64,
    pub source: SourceInstanceId,
    pub span: crate::types::SourceSpan,
    pub order: SignatureOrder,
}

pub(super) fn candidates(
    lookup: &Lookup,
    members: &[&super::nominal::Member],
) -> Option<Vec<usize>> {
    let facts = members
        .iter()
        .map(|member| {
            Some(Fact {
                owner: member.origin.owner?,
                source: member.origin.source,
                span: member.origin.signature.0,
                order: member.signature.syntax.ordering?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    source_candidates(lookup, &facts)
}

pub(in crate::indexer::resolve::engine::program_view) fn source_candidates(
    lookup: &Lookup,
    members: &[Fact],
) -> Option<Vec<usize>> {
    let mut blocks: Vec<Vec<usize>> = Vec::new();
    let mut owners = std::collections::HashSet::new();
    let mut last = None;
    for (index, member) in members.iter().enumerate() {
        let owner = member.owner;
        let order = member.order;
        if order.policy != OverloadOrder::MergedGroupsLiteralFirst
            || order.group.start > member.span.start
            || order.group.end < member.span.end
        {
            return None;
        }
        if last != Some(owner) {
            if !owners.insert(owner) {
                return None;
            }
            blocks.push(Vec::new());
            last = Some(owner);
        }
        blocks.last_mut()?.push(index);
    }
    let mut input = Vec::new();
    for mut block in blocks {
        let first = members[*block.first()?].source;
        let multiple_sources = block.iter().any(|&i| members[i].source != first);
        let mut ranks = FxHashMap::default();
        for &i in &block {
            let source = members[i].source;
            let rank = if multiple_sources {
                *lookup.view.source_binding_order.as_ref()?.get(&source)?
            } else {
                0
            };
            ranks.insert(source, rank);
        }
        block.sort_by_key(|&i| (ranks[&members[i].source], members[i].span.start));
        input.extend(block);
    }
    let facts = input
        .iter()
        .map(|&i| {
            let member = &members[i];
            let order = member.order;
            (
                member.owner,
                (member.source, order.group),
                order.specialized,
            )
        })
        .collect::<Vec<_>>();
    Some(reorder(&facts).into_iter().map(|i| input[i]).collect())
}

fn reorder<O: Eq, G: Eq>(facts: &[(O, G, bool)]) -> Vec<usize> {
    let mut result = Vec::new();
    let mut last_owner = None;
    let mut last_group = None;
    let mut cutoff = 0;
    let mut index = 0;
    let mut specialized = 0;
    for (candidate, (owner, group, literal)) in facts.iter().enumerate() {
        if last_owner.is_none_or(|last| last == owner) {
            if last_group == Some(group) {
                index += 1;
            } else {
                last_group = Some(group);
                index = cutoff;
            }
        } else {
            index = result.len();
            cutoff = index;
            last_group = Some(group);
        }
        last_owner = Some(owner);
        let insert = if *literal {
            let at = specialized;
            specialized += 1;
            cutoff += 1;
            at
        } else {
            index
        };
        result.insert(insert.min(result.len()), candidate);
    }
    result
}

#[cfg(test)]
#[path = "program_overload_order_tests.rs"]
mod tests;
