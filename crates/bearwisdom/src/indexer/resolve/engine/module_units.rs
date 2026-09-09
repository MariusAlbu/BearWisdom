//! Source-local module ownership is validated before global IDs are allocated.
use super::*;

impl ModuleGraph {
    pub(super) fn source_paths(&self) -> FxHashMap<String, ModuleId> {
        let mut counts = BTreeMap::<String, usize>::new();
        for (key, input) in &self.inputs {
            let path = module_paths::normalize(&input.path);
            if module_paths::normalize(key) == path {
                *counts.entry(path).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .filter(|(_, count)| *count == 1)
            .enumerate()
            .map(|(id, (path, _))| (path, ModuleId(id)))
            .collect()
    }

    pub(super) fn allocate_units(&mut self) {
        self.modules = (0..self.paths.len()).map(|_| Module::default()).collect();
        for (key, input) in &self.inputs {
            if module_paths::normalize(key) != module_paths::normalize(&input.path) {
                continue;
            }
            let Some(&root) = self.paths.get(&module_paths::normalize(&input.path)) else {
                continue;
            };
            self.units.insert((root, SourceModuleId(0)), root);
            let valid = valid_units(&input.units);
            for unit in &input.units {
                if valid.contains(&unit.id) {
                    let id = ModuleId(self.modules.len());
                    self.modules.push(Module::default());
                    self.units.insert((root, unit.id), id);
                }
            }
        }
    }
}

fn valid_units(units: &[super::super::module_input::InputUnit]) -> FxHashSet<SourceModuleId> {
    let mut parents = FxHashMap::default();
    let mut invalid = FxHashSet::default();
    for unit in units {
        if unit.id == SourceModuleId(0) || parents.insert(unit.id, unit.parent).is_some() {
            invalid.insert(unit.id);
        }
    }
    let mut states: FxHashMap<_, bool> = invalid.into_iter().map(|id| (id, false)).collect();
    let mut trail = Vec::new();
    let mut active = FxHashSet::default();
    for unit in units {
        let mut id = unit.id;
        let valid = loop {
            if id == SourceModuleId(0) {
                break true;
            }
            if let Some(&valid) = states.get(&id) {
                break valid;
            }
            if !active.insert(id) {
                break false;
            }
            trail.push(id);
            let Some(&parent) = parents.get(&id) else {
                break false;
            };
            id = parent;
        };
        for id in trail.drain(..) {
            active.remove(&id);
            states.insert(id, valid);
        }
    }
    states
        .into_iter()
        .filter_map(|(id, valid)| valid.then_some(id))
        .collect()
}

#[cfg(test)]
#[path = "module_units_tests.rs"]
mod tests;
