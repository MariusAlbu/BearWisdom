//! Exact source recipes for trait implementation heads and bound obligations.
use super::*;
use crate::indexer::namespaces::traits::{Bound, Implementation, Owner};

impl Capture<'_> {
    pub(super) fn trait_parameter(
        &mut self,
        node: Node,
        binding: BindingId,
        index: usize,
        kind: GenericParamKind,
    ) {
        let Some(&index_of_header) = self.data.traits.header_at.get(&span(node)) else {
            return;
        };
        let header = &mut self.data.traits.headers[index_of_header];
        header.parameters.push((binding, kind));
        if let Owner::Implementation(owner) = header.owner {
            self.data.traits.parameters.insert(
                binding,
                (
                    Use {
                        binding: owner,
                        domain: ExportDomain::Type,
                        local: true,
                    },
                    index,
                ),
            );
        }
    }

    pub(super) fn trait_recipes(&mut self, node: Node) {
        self.qualified_call(node);
        let header = self
            .data
            .traits
            .header_at
            .get(&span(node))
            .map(|&i| self.data.traits.headers[i].clone());
        let owner = header
            .as_ref()
            .map(|h| h.owner)
            .or_else(|| self.slot(node).map(Owner::Declaration));
        let Some(owner) = owner else {
            return;
        };
        let forms = self.forms.traits;
        if let Some(header) = &header {
            if let Owner::Implementation(owner) = owner {
                let trait_type = node
                    .child_by_field_name(forms.implementation_trait)
                    .map(|n| strict(self.exact_expr(n)))
                    .unwrap_or(TypeExpr::Unknown);
                let receiver = node
                    .child_by_field_name(self.forms.extensions.target)
                    .map(|n| strict(self.exact_expr(n)))
                    .unwrap_or(TypeExpr::Unknown);
                self.data.traits.implementations.push(Implementation {
                    owner,
                    trait_type,
                    receiver,
                });
            } else if let Some(bounds) = node.child_by_field_name(forms.bounds) {
                let subject = TypeExpr::Source {
                    usage: Use {
                        binding: header.self_binding,
                        domain: ExportDomain::Type,
                        local: true,
                    },
                    legacy: None,
                };
                self.trait_bound(owner, bounds, subject);
            }
        }
        if let Some(parameters) = node.child_by_field_name(self.forms.types.generic_parameters) {
            let mut cursor = parameters.walk();
            for parameter in parameters.named_children(&mut cursor) {
                if let (Some(name), Some(bounds)) = (
                    parameter.child_by_field_name("name"),
                    parameter.child_by_field_name(forms.bounds),
                ) {
                    let subject = strict(self.exact_expr(name));
                    self.trait_bound(owner, bounds, subject);
                }
            }
        }
        let mut cursor = node.walk();
        for clause in node
            .named_children(&mut cursor)
            .filter(|n| n.kind() == forms.where_clause)
        {
            let mut cursor = clause.walk();
            for predicate in clause.named_children(&mut cursor).filter(|n| !n.is_extra()) {
                let subject = predicate
                    .child_by_field_name(forms.predicate_subject)
                    .map(|n| strict(self.exact_expr(n)))
                    .unwrap_or(TypeExpr::Unknown);
                if predicate.kind() == forms.where_predicate {
                    if let Some(bounds) = predicate.child_by_field_name(forms.bounds) {
                        self.trait_bound(owner, bounds, subject);
                        continue;
                    }
                }
                // Unsupported predicates remain obligations, never disappear into
                // an apparently unconditional implementation.
                self.data.traits.bounds.push(Bound {
                    owner,
                    span: span(predicate),
                    subject,
                    traits: vec![TypeExpr::Unknown],
                });
            }
        }
    }

    fn trait_bound(&mut self, owner: Owner, node: Node, subject: TypeExpr) {
        let mut cursor = node.walk();
        let mut traits: Vec<_> = node
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .map(|n| strict(self.exact_expr(n)))
            .collect();
        if traits.is_empty() {
            traits.push(TypeExpr::Unknown);
        }
        self.data.traits.bounds.push(Bound {
            owner,
            span: span(node),
            subject,
            traits,
        });
    }

    fn qualified_call(&mut self, node: Node) {
        let Some(&(_, function)) = self
            .forms
            .locals
            .calls
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
        else {
            return;
        };
        let Some(mut callee) = node.child_by_field_name(function) else {
            return;
        };
        while let Some(&(_, field)) = self
            .forms
            .locals
            .callable_wrappers
            .iter()
            .find(|&&(kind, _)| kind == callee.kind())
        {
            let Some(inner) = callee.child_by_field_name(field) else {
                return;
            };
            callee = inner;
        }
        let Some(&(_, base, name)) = self
            .forms
            .path_nodes
            .iter()
            .find(|&&(kind, _, _)| kind == callee.kind())
        else {
            return;
        };
        let Some(root) = callee
            .child_by_field_name(base)
            .filter(|n| n.kind() == self.forms.traits.qualified_wrapper)
        else {
            return;
        };
        let Some(selector) = callee.child_by_field_name(name) else {
            return;
        };
        let (kind, subject, trait_field) = self.forms.traits.qualified_type;
        let qualified = root.named_child(0).filter(|n| n.kind() == kind);
        let receiver = qualified
            .and_then(|n| n.child_by_field_name(subject))
            .map(|n| strict(self.exact_expr(n)))
            .unwrap_or(TypeExpr::Unknown);
        let trait_type = qualified
            .and_then(|n| n.child_by_field_name(trait_field))
            .map(|n| strict(self.exact_expr(n)))
            .unwrap_or(TypeExpr::Unknown);
        let mut ancestor = node.parent();
        let mut caller = None;
        while let Some(parent) = ancestor {
            if parent.kind() == self.forms.function {
                caller = self.slot(parent);
                break;
            }
            ancestor = parent.parent();
        }
        self.data.traits.qualified_calls.insert(
            selector.start_byte() as u32,
            crate::indexer::namespaces::traits::QualifiedCall {
                root: span(root),
                caller,
                receiver,
                trait_type,
            },
        );
    }
}

fn span(node: Node) -> SourceSpan {
    SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    }
}

/// Applicability has no legacy-name boundary. An unbound source recipe remains
/// an ID-addressed miss even when the project/provider has not been configured.
pub(super) fn strict(mut recipe: TypeExpr) -> TypeExpr {
    fn walk(recipe: &mut TypeExpr) {
        match recipe {
            TypeExpr::Legacy(_) => *recipe = TypeExpr::Unknown,
            TypeExpr::Source { usage, legacy } => {
                usage.local = true;
                *legacy = None;
            }
            TypeExpr::Apply(base, args)
            | TypeExpr::InputApplication { base, args, .. }
            | TypeExpr::OutputApplication { base, args } => {
                walk(base);
                for arg in args {
                    walk(arg);
                }
            }
            TypeExpr::Output { inputs, result } | TypeExpr::Function(inputs, result) => {
                for input in inputs {
                    walk(input);
                }
                walk(result);
            }
            TypeExpr::Indirect { region, inner, .. } => {
                if let Some(region) = region {
                    walk(region);
                }
                walk(inner);
            }
            TypeExpr::Tuple(items) | TypeExpr::Union(items) | TypeExpr::Intersection(items) => {
                for item in items {
                    walk(item);
                }
            }
            TypeExpr::Optional(inner) => walk(inner),
            _ => {}
        }
    }
    walk(&mut recipe);
    recipe
}

#[cfg(test)]
#[path = "namespace_type_traits_tests.rs"]
mod tests;
