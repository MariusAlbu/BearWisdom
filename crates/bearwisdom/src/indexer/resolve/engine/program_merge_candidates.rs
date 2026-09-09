//! Unproved groups are private inputs, never public Bound results.
use super::*;
use crate::types::SymbolKind;
use rustc_hash::FxHashSet;

#[derive(Clone)]
pub(in crate::indexer::resolve::engine) struct Pending {
    pub key: i64,
    name: GlobalNameId,
    group: Group,
    pub parts: Vec<(SourceInstanceId, super::super::program_input::Part)>,
}

pub(super) fn capture(name: GlobalNameId, parts: &[Candidate<'_>]) -> Option<Pending> {
    let first = parts.first()?;
    let longest = parts.iter().max_by_key(|p| p.parameters.len())?;
    if parts.len() < 2
        || parts.iter().any(|p| {
            !p.live
                || !p.part.type_space
                || p.part.kind != SymbolKind::Interface
                || !(p.part.plain_header || p.part.type_heritage)
                || !longest.parameters.starts_with(&p.parameters)
                || p.part.surface.is_none()
                || p.part.binding.is_none()
                || !p
                    .rules
                    .contains(&(SymbolKind::Interface, SymbolKind::Interface))
        })
    {
        return None;
    }
    Some(Pending {
        key: first.part.declaration?,
        name,
        group: Group {
            parts: parts
                .iter()
                .map(|p| DeclarationPart {
                    source: p.source,
                    declaration: p.part.declaration.unwrap(),
                    parameters: p.parameters.clone(),
                })
                .collect(),
        },
        parts: parts.iter().map(|p| (p.source, p.part.clone())).collect(),
    })
}

impl Graph {
    pub(in crate::indexer::resolve::engine) fn pending(&self, program: ProgramId) -> &[Pending] {
        self.environment(program)
            .filter(|env| env.complete)
            .map(|env| env.pending.as_slice())
            .unwrap_or(&[])
    }

    /// Only a private validation view receives this clone. Public admission is
    /// decided after all dependent candidates stabilize; the source graph stays fenced.
    pub(in crate::indexer::resolve::engine) fn staged(
        &self,
        program: ProgramId,
        allowed: &FxHashSet<i64>,
    ) -> Self {
        let mut staged = self.clone();
        if self.environment(program).is_none_or(|env| !env.complete) {
            return staged;
        }
        let env = &mut staged.programs[program.index];
        let mut promoted = FxHashSet::default();
        for candidate in &env.pending {
            if !allowed.contains(&candidate.key) {
                continue;
            }
            let id = GlobalBindingId {
                program,
                index: env.groups.len(),
            };
            env.groups.push(candidate.group.clone());
            env.globals
                .insert((candidate.name, true), Result::Bound(id));
            for (source, part) in &candidate.parts {
                env.sources[source.index]
                    .declarations
                    .insert((BindingId(part.binding.unwrap()), true), Result::Bound(id));
                promoted.insert(part.declaration.unwrap());
            }
        }
        env.rejected.retain(|row| !promoted.contains(row));
        staged
    }
}

#[cfg(test)]
#[path = "program_merge_candidates_tests.rs"]
mod tests;
