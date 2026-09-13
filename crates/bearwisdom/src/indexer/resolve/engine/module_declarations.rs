//! A literal `declare module "X"` unit is the module named X. Admission is
//! syntax evidence only; a configured program's own selection is authoritative
//! where one exists.
use super::super::module_input::InputUnit;
use super::*;
use crate::indexer::lexical::modules::scopes::Kind;

/// True when this unit is a literal module declaration, admitted or not.
pub(super) fn is_literal(unit: &InputUnit) -> bool {
    unit.source_scope
        .as_ref()
        .is_some_and(|scope| scope.kind == Kind::Literal)
}

/// The specifier a unit declares, when the unit is a top-level, complete,
/// container-valid, ambient literal declaration carrying a name.
pub(super) fn admitted_specifier(unit: &InputUnit) -> Option<&str> {
    let scope = unit.source_scope.as_ref()?;
    if scope.kind != Kind::Literal
        || unit.parent != SourceModuleId(0)
        || !scope.complete
        || !scope.container_valid
        || !scope.ambient
    {
        return None;
    }
    unit.source_name.as_deref()
}

/// Add a provider part for every admitted, non-relative declaration in a
/// non-isolated source. An entry already present wins: a configured program
/// selected its own sources and merged its augmentations into them.
pub(super) fn seed_providers(
    providers: &mut BTreeMap<String, program_modules::Provider>,
    inputs: &BTreeMap<String, ModuleInput>,
) {
    let mut declared: BTreeMap<&str, Vec<(String, SourceModuleId)>> = BTreeMap::new();
    for (key, input) in inputs {
        if module_paths::normalize(key) != module_paths::normalize(&input.path) {
            continue;
        }
        // An isolated source augments a module it does not own, and a source
        // without declaration evidence proves nothing either way.
        if input.globals.as_ref().map(|globals| globals.isolated) != Some(false) {
            continue;
        }
        for unit in &input.units {
            let Some(name) = admitted_specifier(unit) else {
                continue;
            };
            // A relative ambient name addresses one file, not a global key.
            if module_paths::relative_base(&input.path, name).is_some() {
                continue;
            }
            declared
                .entry(name)
                .or_default()
                .push((input.path.clone(), unit.id));
        }
    }
    for (name, mut parts) in declared {
        if providers.contains_key(name) {
            continue;
        }
        parts.sort_by_key(|(path, unit)| (path.clone(), unit.0));
        providers.entry(name.to_owned()).or_default().parts = parts;
    }
}

#[cfg(test)]
#[path = "module_declarations_tests.rs"]
mod tests;
