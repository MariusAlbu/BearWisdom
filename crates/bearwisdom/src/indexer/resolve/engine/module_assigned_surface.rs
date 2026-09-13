//! A module that assigns a VALUE publishes that value's declared type's
//! members as its named exports. The member set is materialized while the
//! graph is being rebuilt; the traversal itself stays ID-only.
use super::*;

impl ModuleGraph {
    /// Attach the member surface of every export-assigned value declaration.
    /// Two modules assigning the same declaration share one surface.
    pub(super) fn install_assigned_surfaces(&mut self, lookup: &dyn SymbolLookup) {
        let mut built: FxHashMap<i64, Option<ModuleId>> = FxHashMap::default();
        for (module, domain, declaration) in self.assigned_declarations() {
            let surface = match built.get(&declaration).copied() {
                Some(surface) => surface,
                None => {
                    let surface = self.materialize_surface(declaration, lookup);
                    built.insert(declaration, surface);
                    surface
                }
            };
            if let Some(surface) = surface {
                self.modules[module.0]
                    .assigned_surface
                    .insert(domain, surface);
            }
        }
    }

    /// The value declaration each export assignment resolves to, per module
    /// and domain.
    fn assigned_declarations(&self) -> Vec<(ModuleId, ExportDomain, i64)> {
        let mut found = Vec::new();
        for (index, data) in self.modules.iter().enumerate() {
            for domain in [
                ExportDomain::Value,
                ExportDomain::Type,
                ExportDomain::ValueQuery,
            ] {
                if !data.assignments.contains_key(&domain) {
                    continue;
                }
                let module = ModuleId(index);
                if let Some(declaration) = self
                    .resolve_target(Target::Assigned(module), domain, 0)
                    .declaration()
                {
                    found.push((module, domain, declaration));
                }
            }
        }
        found
    }

    /// One module whose named exports are the members of the declaration's
    /// declared type. `None` when that type names no member-bearing owner.
    fn materialize_surface(
        &mut self,
        declaration: i64,
        lookup: &dyn SymbolLookup,
    ) -> Option<ModuleId> {
        let arena = lookup.type_arena()?;
        let declared = lookup.field_type_id_of(declaration)?;
        let owner = super::super::head_decl::head_decl_id(arena, declared)?;
        let members = lookup.members_of_id(owner);
        if members.is_empty() {
            return None;
        }
        let id = ModuleId(self.modules.len());
        let mut module = Module::default();
        for member in members.iter() {
            let name = intern(&mut self.names, &member.name);
            for domain in [ExportDomain::Value, ExportDomain::ValueQuery] {
                module
                    .exports
                    .entry((name, domain))
                    .or_default()
                    .push(Target::Declaration(member.id));
            }
        }
        self.modules.push(module);
        Some(id)
    }
}

#[cfg(test)]
#[path = "module_assigned_surface_tests.rs"]
mod tests;
