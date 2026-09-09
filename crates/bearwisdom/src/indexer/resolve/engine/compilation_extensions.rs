//! Source-bound detached bodies attach only through nominal declaration IDs.
use super::super::contract::member_applicability::{self, ReceiverPattern};
use super::*;

impl Compilation {
    pub(super) fn rebind_extension_owners(&mut self) {
        let mut next = FxHashMap::default();
        let mut patterns = FxHashMap::default();
        for input in self.modules.inputs.values() {
            for extension in &input.extensions {
                let bound = self.extension_pattern(input, extension);
                for &member in &extension.members {
                    if self.by_id.contains_key(&member) {
                        next.entry(member)
                            .and_modify(|old| {
                                if *old != bound {
                                    *old = None;
                                }
                            })
                            .or_insert_with(|| bound.clone());
                    }
                }
            }
        }
        let captured: FxHashSet<_> = self
            .extension_owners
            .keys()
            .chain(next.keys())
            .copied()
            .collect();
        // Both prior and newly attested members are fences against stale local
        // parent slots. An unresolved or changed owner cannot retain membership.
        for members in self.members_by_id.values_mut() {
            members.retain(|member| !captured.contains(member));
        }
        self.enclosing_type_by_id
            .retain(|member, _| !captured.contains(member));
        for (&member, bound) in &next {
            if let Some((owner, pattern)) = bound {
                self.members_by_id.entry(*owner).or_default().push(member);
                self.enclosing_type_by_id.insert(member, *owner);
                if let Some(pattern) = pattern {
                    patterns.insert(member, Arc::clone(pattern));
                }
            }
        }
        self.extension_owners = next
            .into_iter()
            .map(|(member, bound)| (member, bound.map(|(owner, _)| owner)))
            .collect();
        self.extension_patterns = patterns;
        self.member_index.rebuild(&self.members_by_id, &self.by_id);
    }

    fn extension_pattern(
        &self,
        input: &super::super::module_input::ModuleInput,
        extension: &super::super::module_input::InputExtension,
    ) -> Option<(i64, Option<Arc<ReceiverPattern>>)> {
        let id = self
            .modules
            .binding(
                &input.path,
                crate::indexer::lexical::BindingId(extension.binding),
                true,
            )
            .declaration()?;
        let id = self.canonical_decl_id(id);
        let symbol = self.by_id.get(&id)?;
        let parameters = self
            .type_info_by_id
            .get(&id)
            .map(|info| info.generic_param_ids.clone())
            .unwrap_or_default();
        if parameters.len() != extension.arity
            || extension.kinds.len() != parameters.len()
            || parameters
                .iter()
                .zip(&extension.kinds)
                .any(|(&param, &kind)| self.arena.generic_kind(param) != kind)
        {
            return None;
        }
        let base = self.arena.decl(&symbol.qualified_name, id);
        let ty = if parameters.is_empty() {
            base
        } else {
            self.arena.intern(Type::Apply {
                base,
                args: parameters
                    .iter()
                    .map(|&param| self.arena.generic_type(param))
                    .collect(),
            })
        };
        let expanded = member_applicability::expand(self, &self.arena, ty)?;
        let owner = super::super::head_decl::head_decl_id(&self.arena, expanded)?;
        if !self
            .by_id
            .get(&owner)
            .is_some_and(|s| matches!(s.kind.as_str(), "struct" | "enum" | "union"))
            || !self
                .modules
                .extension_origin(&input.path, extension.origin, owner)
        {
            return None;
        }
        let arity = self
            .type_info_by_id
            .get(&owner)
            .map_or(0, |info| info.generic_param_ids.len());
        if super::super::chain::apply_args(&self.arena, expanded).len() != arity {
            return None;
        }
        let pattern = (id != owner).then(|| {
            Arc::new(ReceiverPattern {
                ty: expanded,
                parameters,
            })
        });
        Some((owner, pattern))
    }
}

#[cfg(test)]
#[path = "compilation_extensions_tests.rs"]
mod tests;
