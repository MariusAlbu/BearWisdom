//! Module graph traversal. An ambiguous intermediate namespace stays ambiguous,
//! even when two namespaces happen to export the same terminal declaration.
use super::*;

struct WalkState {
    remaining: usize,
    active: FxHashSet<(Target, ExportDomain)>,
}
impl Default for WalkState {
    fn default() -> Self {
        Self {
            remaining: 65_536,
            active: FxHashSet::default(),
        }
    }
}

impl ModuleGraph {
    pub(super) fn resolve_target(
        &self,
        target: Target,
        domain: ExportDomain,
        depth: usize,
    ) -> BindingResult {
        self.resolve_with_state(target, domain, depth, &mut WalkState::default())
    }

    fn resolve_with_state(
        &self,
        target: Target,
        domain: ExportDomain,
        depth: usize,
        state: &mut WalkState,
    ) -> BindingResult {
        if depth > 64 || state.remaining == 0 || !state.active.insert((target, domain)) {
            return BindingResult::Incomplete;
        }
        state.remaining -= 1;
        let result = self.resolve_inner(target, domain, depth, state);
        state.active.remove(&(target, domain));
        result
    }

    fn resolve_inner(
        &self,
        target: Target,
        domain: ExportDomain,
        depth: usize,
        state: &mut WalkState,
    ) -> BindingResult {
        match target {
            Target::Declaration(id) => BindingResult::Bound(id),
            Target::Assigned(module) => self.assigned(module, domain, depth, state),
            Target::Entity(id) => {
                let (declaration, namespace) = self.targets.entities[id];
                let value = self.resolve_with_state(declaration, domain, depth + 1, state);
                let scope = self.resolve_with_state(namespace, domain, depth + 1, state);
                match (value, scope) {
                    (BindingResult::Bound(id), BindingResult::Namespace(namespace)) => {
                        BindingResult::Entity {
                            declaration: Some(id),
                            overloads: None,
                            namespace,
                        }
                    }
                    (BindingResult::Overloads(id), BindingResult::Namespace(namespace)) => {
                        BindingResult::Entity {
                            declaration: None,
                            overloads: Some(id),
                            namespace,
                        }
                    }
                    (BindingResult::Missing, scope) => scope,
                    (
                        BindingResult::Incomplete
                        | BindingResult::Ambiguous
                        | BindingResult::Unconfigured,
                        _,
                    ) => value,
                    _ => BindingResult::Incomplete,
                }
            }
            Target::Namespace(id) => {
                if self.modules[id.0].incomplete {
                    BindingResult::Incomplete
                } else {
                    BindingResult::Namespace(id)
                }
            }
            Target::Overloads(id) => BindingResult::Overloads(id),
            Target::Missing | Target::Access(_) | Target::ScopePath(_) => BindingResult::Missing,
            Target::Incomplete => BindingResult::Incomplete,
            Target::Ambiguous => BindingResult::Ambiguous,
            Target::Unconfigured => BindingResult::Unconfigured,
            Target::Binding(module, binding, binding_domain) => {
                if self.modules[module.0].incomplete {
                    return BindingResult::Incomplete;
                }
                let Some(targets) = self.modules[module.0]
                    .imports
                    .get(&(binding, binding_domain))
                else {
                    return BindingResult::Missing;
                };
                targets
                    .iter()
                    .fold(BindingResult::Missing, |value, &target| {
                        value.merge(
                            self.resolve_with_state(target, binding_domain, depth + 1, state)
                                .with_competitor(targets.len() > 1),
                        )
                    })
            }
            Target::Export(module, name) => self.walk_export(
                module,
                name,
                domain,
                depth,
                None,
                &mut FxHashSet::default(),
                state,
            ),
            Target::Path(id) => {
                let (base, names, requester) = &self.targets.paths[id];
                let mut value = self.resolve_with_state(*base, domain, depth + 1, state);
                for &(name, selector_domain) in names {
                    value = match value {
                        BindingResult::Namespace(module)
                        | BindingResult::Entity {
                            namespace: module, ..
                        } => self.walk_export(
                            module,
                            name,
                            selector_domain.unwrap_or(domain),
                            depth + 1,
                            *requester,
                            &mut FxHashSet::default(),
                            state,
                        ),
                        BindingResult::Incomplete
                        | BindingResult::Ambiguous
                        | BindingResult::Unconfigured => return value,
                        _ => return BindingResult::Missing,
                    };
                }
                value
            }
        }
    }

    fn assigned(
        &self,
        module: ModuleId,
        domain: ExportDomain,
        depth: usize,
        state: &mut WalkState,
    ) -> BindingResult {
        let mut pending = vec![module];
        let mut visited = FxHashSet::default();
        let mut targets = Vec::new();
        while let Some(owner) = pending.pop() {
            if !visited.insert(owner) {
                return BindingResult::Incomplete;
            }
            if state.remaining == 0 {
                return BindingResult::Incomplete;
            }
            state.remaining -= 1;
            let data = &self.modules[owner.0];
            if data.incomplete {
                return BindingResult::Incomplete;
            }
            pending.extend(data.assignment_parts.iter().copied());
            if let Some(assignments) = data.assignments.get(&domain) {
                if assignments.len() != 1 {
                    return BindingResult::Ambiguous;
                }
                targets.extend(assignments.iter().copied());
            }
        }
        if targets.is_empty() {
            return BindingResult::Namespace(module);
        }
        targets
            .iter()
            .fold(BindingResult::Missing, |result, &target| {
                result.merge(
                    self.resolve_with_state(target, domain, depth + 1, state)
                        .with_competitor(targets.len() > 1),
                )
            })
    }

    fn walk_export(
        &self,
        module: ModuleId,
        name: ExportNameId,
        domain: ExportDomain,
        depth: usize,
        requester: Option<ModuleId>,
        visiting: &mut FxHashSet<(ModuleId, ExportNameId, ExportDomain)>,
        state: &mut WalkState,
    ) -> BindingResult {
        // Shared barrels are visited once per export key, not once per path.
        let mut pending = vec![(module, name)];
        let mut result = BindingResult::Missing;
        while let Some((module, name)) = pending.pop() {
            if !visiting.insert((module, name, domain)) {
                continue;
            }
            if state.remaining == 0 {
                return BindingResult::Incomplete;
            }
            state.remaining -= 1;
            let data = &self.modules[module.0];
            if data.incomplete {
                return BindingResult::Incomplete;
            }
            if let Some(explicit) = data.exports.get(&(name, domain)) {
                // An explicit missing entry also blocks wildcard fallthrough.
                for &target in explicit {
                    let Some((target, promised)) = self.export_access(target, requester) else {
                        result = result
                            .merge(BindingResult::Missing.with_competitor(explicit.len() > 1));
                        continue;
                    };
                    if explicit.len() == 1 && promised.is_none() && requester.is_none() {
                        if let Target::Export(module, name) = target {
                            pending.push((module, name));
                            continue;
                        }
                    }
                    let candidate = self.resolve_with_state(target, domain, depth + 1, state);
                    let candidate =
                        if promised.is_some_and(|scope| !self.valid_reexport(candidate, scope)) {
                            BindingResult::Missing
                        } else {
                            candidate
                        };
                    result = result.merge(candidate.with_competitor(explicit.len() > 1));
                }
            } else if !data.wildcard_exclusions.contains(&name) {
                for &(target, star_domain) in &data.stars {
                    if star_domain != domain {
                        continue;
                    }
                    // Legacy wildcard exports promise public visibility. A
                    // caller's private access must not pass through a barrel.
                    if requester.is_some() {
                        let candidate =
                            match self.resolve_with_state(target, domain, depth + 1, state) {
                                BindingResult::Namespace(module) => self.walk_export(
                                    module,
                                    name,
                                    domain,
                                    depth + 1,
                                    None,
                                    &mut FxHashSet::default(),
                                    state,
                                ),
                                BindingResult::Ambiguous => BindingResult::Ambiguous,
                                _ => BindingResult::Incomplete,
                            };
                        result = result.merge(candidate);
                        continue;
                    }
                    match target {
                        Target::Namespace(module) => pending.push((module, name)),
                        _ => match self.resolve_with_state(target, domain, depth + 1, state) {
                            BindingResult::Namespace(module) => pending.push((module, name)),
                            BindingResult::Ambiguous => {
                                result = result.merge(BindingResult::Ambiguous)
                            }
                            _ => result = result.merge(BindingResult::Incomplete),
                        },
                    }
                }
            }
        }
        result
    }

    #[cfg(test)]
    pub(super) fn export(
        &self,
        module: ModuleId,
        name: ExportNameId,
        domain: ExportDomain,
        visiting: &mut FxHashSet<(ModuleId, ExportNameId, ExportDomain)>,
    ) -> BindingResult {
        self.walk_export(
            module,
            name,
            domain,
            0,
            None,
            visiting,
            &mut WalkState::default(),
        )
    }
}

#[cfg(test)]
#[path = "module_walk_tests.rs"]
mod tests;
