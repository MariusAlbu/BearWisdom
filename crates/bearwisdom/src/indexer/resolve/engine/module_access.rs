//! Visibility is module ancestry, not a qualified-name comparison. Re-exporting
//! may hide a private path prefix but cannot widen its terminal item's access.
use super::*;

/// Snapshot-local module spans, installed once when a file environment is built.
#[derive(Default)]
pub(in crate::indexer::resolve::engine) struct Site {
    spans: Vec<(u32, u32, ModuleId)>,
    active: std::cell::Cell<Option<ModuleId>>,
}
impl Site {
    pub(in crate::indexer::resolve::engine) fn set_cursor(&self, byte: u32) {
        self.active.set(self.module_at(byte));
    }
    pub(in crate::indexer::resolve::engine) fn module_at(&self, byte: u32) -> Option<ModuleId> {
        self.spans
            .iter()
            .filter(|&&(start, end, _)| start <= byte && byte < end)
            .min_by_key(|&&(start, end, _)| end - start)
            .map(|&(_, _, id)| id)
    }
    pub(in crate::indexer::resolve::engine) fn module(&self) -> Option<ModuleId> {
        self.active.get()
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Guard {
    pub target: Target,
    pub scope: Option<Target>,
    pub origin: ModuleId,
    pub declaration: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    Public,
    Within(ModuleId),
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Entity {
    Declaration(i64),
    Module(ModuleId),
}

fn entity(result: BindingResult) -> Option<Entity> {
    match result {
        BindingResult::Bound(id) => Some(Entity::Declaration(id)),
        BindingResult::Namespace(id) => Some(Entity::Module(id)),
        _ => None,
    }
}

impl ModuleGraph {
    /// Extension attachment is authorized by declaration origins, not package
    /// names or path prefixes. Distinct crate roots never share inherent impls.
    pub(in crate::indexer::resolve::engine) fn extension_origin(
        &self,
        file: &str,
        origin: SourceModuleId,
        owner: i64,
    ) -> bool {
        self.extension_crate(file, origin, owner).unwrap_or(false)
    }
    fn extension_crate(&self, file: &str, origin: SourceModuleId, owner: i64) -> Option<bool> {
        let root = self.paths.get(&module_paths::normalize(file))?;
        let requester = *self.units.get(&(*root, origin))?;
        let declared = self.declaration_origins.get(&owner).copied().flatten()?;
        Some(self.crate_root(requester)? == self.crate_root(declared)?)
    }
    fn crate_root(&self, mut module: ModuleId) -> Option<ModuleId> {
        let mut seen = FxHashSet::default();
        loop {
            if !seen.insert(module) {
                return None;
            }
            match self.modules.get(module.0)?.parent {
                Some(parent) => module = parent,
                None => return Some(module),
            }
        }
    }
    pub(in crate::indexer::resolve::engine) fn site(&self, file: &str) -> Site {
        let file = module_paths::normalize(file);
        let spans = self
            .inputs
            .get(&file)
            .zip(self.paths.get(&file))
            .map(|(input, root)| {
                input
                    .source_spans
                    .iter()
                    .filter_map(|&(unit, start, end)| {
                        (start < end)
                            .then(|| self.units.get(&(*root, unit)).map(|&id| (start, end, id)))
                            .flatten()
                    })
                    .collect()
            })
            .unwrap_or_default();
        Site {
            spans,
            ..Default::default()
        }
    }
    pub(in crate::indexer::resolve::engine) fn declaration_access(
        &self,
        declaration: i64,
        requester: Option<ModuleId>,
    ) -> bool {
        match self
            .visibility
            .get(&Entity::Declaration(declaration))
            .copied()
            .unwrap_or(Scope::Public)
        {
            Scope::Public => true,
            Scope::Within(owner) => requester.is_some_and(|origin| self.descendant(origin, owner)),
            Scope::Unknown => false,
        }
    }

    pub(super) fn descendant(&self, mut module: ModuleId, ancestor: ModuleId) -> bool {
        let mut seen = FxHashSet::default();
        loop {
            if module == ancestor {
                return true;
            }
            if !seen.insert(module) {
                return false;
            }
            let Some(parent) = self.modules.get(module.0).and_then(|m| m.parent) else {
                return false;
            };
            module = parent;
        }
    }

    fn scope(&self, guard: &Guard) -> Scope {
        // Access recipes lower to module IDs before traversal. Do not resolve
        // arbitrary paths here: that would restart cycle/work accounting.
        let module = match guard.scope {
            None => return Scope::Public,
            Some(Target::Namespace(id)) => Some(id),
            Some(Target::ScopePath(id)) => self.targets.scope_ids.get(id).copied().flatten(),
            _ => None,
        };
        match module {
            Some(id) if self.descendant(guard.origin, id) => Scope::Within(id),
            _ => Scope::Unknown,
        }
    }

    pub(super) fn capture_visibility(&mut self) {
        self.targets.scope_ids = self
            .targets
            .scope_paths
            .iter()
            .map(|(base, names)| self.declared_scope(*base, names))
            .collect();
        let facts: Vec<_> = self
            .targets
            .guards
            .iter()
            .filter(|g| g.declaration)
            .filter_map(|g| {
                Some((
                    entity(self.resolve_target(g.target, ExportDomain::Type, 0))?,
                    self.scope(g),
                    g.origin,
                ))
            })
            .collect();
        for (id, scope, origin) in facts {
            if let Entity::Declaration(declaration) = id {
                self.declaration_origins
                    .entry(declaration)
                    .and_modify(|old| {
                        if *old != Some(origin) {
                            *old = None;
                        }
                    })
                    .or_insert(Some(origin));
            }
            self.visibility
                .entry(id)
                .and_modify(|old| {
                    if *old != scope {
                        *old = Scope::Unknown;
                    }
                })
                .or_insert(scope);
        }
    }

    /// A restriction names declarations, not aliases. Resolve once at rebuild;
    /// semantic access checks subsequently read the cached numeric scope ID.
    fn declared_scope(&self, base: Target, names: &[ExportNameId]) -> Option<ModuleId> {
        let Target::Namespace(mut module) = base else {
            return None;
        };
        let mut remaining = 65_536usize;
        for &name in names {
            let [Target::Access(id)] = self
                .modules
                .get(module.0)?
                .exports
                .get(&(name, ExportDomain::Type))?
                .as_slice()
            else {
                return None;
            };
            let guard = self.targets.guards.get(*id)?;
            if !guard.declaration {
                return None;
            }
            let mut target = guard.target;
            loop {
                remaining = remaining.checked_sub(1)?;
                match target {
                    Target::Namespace(id) => {
                        if self.modules.get(id.0)?.parent != Some(module) {
                            return None;
                        }
                        module = id;
                        break;
                    }
                    Target::Binding(owner, binding, domain) => {
                        let [next] = self
                            .modules
                            .get(owner.0)?
                            .imports
                            .get(&(binding, domain))?
                            .as_slice()
                        else {
                            return None;
                        };
                        target = *next;
                    }
                    _ => return None,
                }
            }
        }
        Some(module)
    }

    /// Return the accessible target and, for aliases, the access they promise.
    pub(super) fn export_access(
        &self,
        target: Target,
        requester: Option<ModuleId>,
    ) -> Option<(Target, Option<Scope>)> {
        let Target::Access(id) = target else {
            return Some((target, None));
        };
        let guard = &self.targets.guards[id];
        let scope = self.scope(guard);
        let allowed = match scope {
            Scope::Public => true,
            Scope::Within(owner) => requester.is_some_and(|origin| self.descendant(origin, owner)),
            Scope::Unknown => false,
        };
        allowed.then_some((guard.target, (!guard.declaration).then_some(scope)))
    }

    pub(super) fn valid_reexport(&self, result: BindingResult, promised: Scope) -> bool {
        let Some(entity) = entity(result) else {
            return true;
        };
        // Profiles without item-access recipes retain their public export contract.
        let actual = self
            .visibility
            .get(&entity)
            .copied()
            .unwrap_or(Scope::Public);
        match (promised, actual) {
            (Scope::Unknown, _) | (_, Scope::Unknown) => false,
            (_, Scope::Public) => true,
            (Scope::Within(inner), Scope::Within(outer)) => self.descendant(inner, outer),
            _ => false,
        }
    }
}

#[cfg(test)]
#[path = "module_access_tests.rs"]
mod tests;
