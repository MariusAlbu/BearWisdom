//! Source class/interface names are lowered once; materialization consumes IDs.
use super::super::member_index::{MemberIndex, MemberNameId};
use super::*;
use crate::indexer::lexical::globals::member_surface::Key;

#[derive(Default)]
pub(super) struct Index(FxHashMap<i64, FxHashMap<MemberNameId, Vec<i64>>>);

impl Index {
    pub(super) fn capture(view: &mut View, sources: &computed_keys::Sources) -> Self {
        let mut result = Self::default();
        for (_, &source, input) in sources {
            // This is the source ingestion boundary, not lookup recovery.
            let names: FxHashMap<_, _> = input
                .names
                .iter()
                .map(|(name, spelling)| (*name, view.members.intern_name(spelling)))
                .collect();
            view.sources.get_mut(&source).unwrap().member_names = names.clone();
            let declarations = input
                .classes
                .iter()
                .map(|part| (part.owner, &part.surface))
                .chain(
                    input
                        .interfaces
                        .iter()
                        .map(|part| (part.owner, &part.surface)),
                );
            for (owner, surface) in declarations {
                let Some(&owner) = view.canonical.get(&owner) else {
                    continue;
                };
                for member in surface.iter().flatten() {
                    let Key::Named(name) = member.key else {
                        continue;
                    };
                    let Some(&name) = names.get(&name) else {
                        continue;
                    };
                    let rows = result.0.entry(owner).or_default().entry(name).or_default();
                    if let Some(row) = member.slot.filter(|row| view.canonical.contains_key(row)) {
                        if !rows.contains(&row) {
                            rows.push(row);
                        }
                    }
                }
            }
        }
        result
    }

    pub(super) fn project(&self, members: &mut MemberIndex) {
        members.project_source_members(&self.0);
    }
}

#[cfg(test)]
#[path = "program_source_members_tests.rs"]
mod tests;
