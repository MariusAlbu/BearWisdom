//! Global contributions require actual source-module containment, not a name.
use super::super::module_input::{InputUnit, ModuleInput, SourceModuleId};
use crate::indexer::lexical::modules::scopes::Kind;

pub(super) fn valid(input: &ModuleInput, isolated: bool) -> bool {
    let Some(globals) = &input.globals else {
        return false;
    };
    globals.roots.iter().all(|part| part.unit.0 == 0)
        && globals
            .augmentations
            .iter()
            .all(|part| augmentation(input, part.unit, isolated))
        && input
            .units
            .iter()
            .filter(|unit| {
                unit.source_scope
                    .as_ref()
                    .is_some_and(|s| s.kind == Kind::Augmentation)
            })
            .all(|unit| augmentation(input, unit.id, isolated))
}

fn unique(input: &ModuleInput, owner: SourceModuleId) -> Option<&InputUnit> {
    let mut units = input.units.iter().filter(|unit| unit.id == owner);
    let unit = units.next()?;
    units.next().is_none().then_some(unit)
}

fn augmentation(input: &ModuleInput, owner: SourceModuleId, isolated: bool) -> bool {
    let Some(unit) = unique(input, owner).filter(|unit| unit.id.0 != 0) else {
        return false;
    };
    if !unit
        .source_scope
        .as_ref()
        .is_some_and(|s| s.kind == Kind::Augmentation && s.complete && s.container_valid)
    {
        return false;
    }
    if unit.parent.0 == 0 {
        return isolated;
    }
    unique(input, unit.parent).is_some_and(|parent| {
        parent.parent.0 == 0
            && parent.source_scope.as_ref().is_some_and(|s| {
                s.kind == Kind::Literal && s.complete && s.container_valid && s.ambient
            })
    })
}

#[cfg(test)]
#[path = "program_contributions_tests.rs"]
mod tests;
