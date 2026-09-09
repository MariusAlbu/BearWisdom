//! Source-owned trait declarations/implementations and lexical availability.
use super::*;
use crate::indexer::namespaces::traits::{Availability, Header, Owner};
use crate::types::SourceSpan;

impl Builder<'_> {
    pub(super) fn trait_header(&mut self, node: Node, scope: ScopeId, unit: SourceModuleId) {
        let forms = self.forms.traits;
        let (owner, self_binding) = if node.kind() == forms.declaration {
            let Some(slot) = self.slot(node) else {
                return;
            };
            let name = self.data.intern(self.forms.self_type, self.forms);
            // Trait Self is not the trait object or a concrete implementor.
            let binding = self
                .data
                .declare(scope, name, ExportDomain::Type, Target::Missing);
            (Owner::Declaration(slot), binding)
        } else if node.kind() == self.forms.extensions.extension
            && node
                .child_by_field_name(forms.implementation_trait)
                .is_some()
        {
            let Some(extension) = self.data.extensions.last() else {
                return;
            };
            (Owner::Implementation(extension.owner), extension.owner)
        } else {
            return;
        };
        self.data.traits.self_bindings.insert(self_binding);
        let mut enabled = !node.has_error();
        let mut ancestor = Some(node);
        while let Some(current) = ancestor {
            if self.conditional(current) {
                enabled = false;
            }
            ancestor = current.parent();
        }
        let mut members = Vec::new();
        if let Some(body) = node.child_by_field_name(self.forms.body) {
            let mut cursor = body.walk();
            for member in body
                .named_children(&mut cursor)
                .filter(|n| forms.members.contains(&n.kind()))
            {
                if self.conditional(member) {
                    enabled = false;
                }
                match self.slot(member) {
                    Some(slot) => members.push(slot),
                    None => enabled = false,
                }
            }
        } else {
            enabled = false;
        }
        let mut cursor = node.walk();
        let negative = node
            .children(&mut cursor)
            .any(|n| n.kind() == forms.negative_token);
        self.data.traits.header_at.insert(
            SourceSpan {
                start: node.start_byte() as u32,
                end: node.end_byte() as u32,
            },
            self.data.traits.headers.len(),
        );
        self.data.traits.headers.push(Header {
            owner,
            declaration: self.slot(node),
            self_binding,
            unit,
            scope,
            span: SourceSpan {
                start: node.start_byte() as u32,
                end: node.end_byte() as u32,
            },
            members,
            enabled,
            negative,
            parameters: Vec::new(),
        });
    }

    pub(super) fn trait_availability(&mut self) {
        self.data.traits.value_partners = self
            .data
            .entries
            .iter()
            .filter_map(|(&(scope, name, domain), binding)| {
                (domain == ExportDomain::Type)
                    .then(|| self.data.entries.get(&(scope, name, ExportDomain::Value)))
                    .flatten()
                    .map(|value| (binding.0, value.0))
            })
            .collect();
        self.data.traits.value_partners.sort_unstable();
        // Group entries once and share one environment per source scope. A file
        // with thousands of calls must not rescan/copy every binding per call.
        let mut entries: HashMap<ScopeId, Vec<BindingId>> =
            self.data.traits.anonymous_imports.clone();
        for (&(scope, _, domain), &binding) in &self.data.entries {
            if domain == ExportDomain::Type {
                entries.entry(scope).or_default().push(binding);
            }
        }
        for &byte in self.data.method_calls.keys() {
            let Some(mut scope) = self.data.graph.scope_at(byte) else {
                continue;
            };
            self.data.traits.method_scopes.insert(byte, scope);
            let unit = self.data.scope_units[&scope];
            loop {
                if self.data.traits.available.contains_key(&scope) {
                    break;
                }
                // Trait-method availability is not ordinary type-name lookup:
                // shadowing a trait's alias does not remove its methods in scope.
                let parent = (scope != self.data.units[unit.0 as usize].scope)
                    .then_some(self.data.graph.scopes[scope.0].parent)
                    .flatten();
                let mut bindings = entries.remove(&scope).unwrap_or_default();
                bindings.sort_unstable_by_key(|b| b.0);
                self.data.traits.available.insert(
                    scope,
                    Availability {
                        parent,
                        bindings,
                        complete: !self.data.opaque_scopes.contains(&scope),
                    },
                );
                let Some(parent) = parent else {
                    break;
                };
                scope = parent;
            }
        }
    }
}

#[cfg(test)]
#[path = "namespace_ingest_traits_tests.rs"]
mod tests;
