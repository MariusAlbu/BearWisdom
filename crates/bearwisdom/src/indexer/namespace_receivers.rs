//! Receiver syntax is captured once; method calls consume only bound type IDs.
use super::*;
use crate::type_checker::core::types::{Indirection, Mutability};

impl Capture<'_> {
    pub(super) fn receiver_value(&self, node: Node, slot: usize, uses: &mut TypeUses) {
        let Some(params) = node.child_by_field_name(self.forms.types.parameters) else {
            return;
        };
        let mut cursor = params.walk();
        let Some(parameter) = params
            .named_children(&mut cursor)
            .find(|n| self.is_receiver(*n))
        else {
            return;
        };
        let mut cursor = parameter.walk();
        let Some(value) = parameter.child_by_field_name("pattern").or_else(|| {
            parameter
                .named_children(&mut cursor)
                .find(|n| self.forms.locals.identifiers.contains(&n.kind()))
        }) else {
            return;
        };
        let span = SourceSpan {
            start: value.start_byte() as u32,
            end: value.end_byte() as u32,
        };
        if let Some(&binding) = self.data.graph.declarations.get(&span) {
            uses.receiver_values.insert(binding, slot);
        }
    }

    pub(super) fn signature_item(&self, node: Node) -> bool {
        node.kind() == self.forms.function
            || (self.forms.traits.members.contains(&node.kind())
                && node
                    .child_by_field_name(self.forms.types.parameters)
                    .is_some())
    }
    pub(super) fn is_receiver(&self, node: Node) -> bool {
        self.forms.types.ignored_parameters.contains(&node.kind())
            || node
                .child_by_field_name("pattern")
                .is_some_and(|n| n.kind() == self.forms.self_path)
    }

    /// The second recipe is the output-elision candidate, NOT the entire Self
    /// type (whose generic arguments may carry unrelated regions).
    pub(super) fn receiver(&mut self, params: Node) -> Option<(TypeExpr, TypeExpr)> {
        let mut cursor = params.walk();
        let node = params
            .named_children(&mut cursor)
            .find(|n| self.is_receiver(*n))?;
        let unsupported = (TypeExpr::Unknown, TypeExpr::Unknown);
        let forms = self.forms.types;
        if let Some(ty) = node.child_by_field_name(forms.annotation) {
            if ty.utf8_text(self.source).ok() == Some(self.forms.self_type) {
                return Some((self.exact_expr(ty), TypeExpr::Unknown));
            }
            let Some(&(_, field, true)) = forms
                .indirections
                .iter()
                .find(|&&(kind, _, _)| kind == ty.kind())
            else {
                return Some(unsupported);
            };
            if !ty
                .child_by_field_name(field)
                .is_some_and(|n| n.utf8_text(self.source).ok() == Some(self.forms.self_type))
            {
                return Some(unsupported);
            }
            let recipe = self.exact_expr(ty);
            let region = match &recipe {
                TypeExpr::Indirect {
                    region: Some(region),
                    ..
                } => *region.clone(),
                TypeExpr::Indirect {
                    kind: Indirection::Reference(region),
                    ..
                } => TypeExpr::Region(*region),
                _ => TypeExpr::Unknown,
            };
            return Some((recipe, region));
        }
        let name = self.data.intern(self.forms.self_type, self.forms);
        let inner = self
            .data
            .graph
            .scope_at(node.start_byte() as u32)
            .and_then(|scope| self.data.lookup(scope, name, ExportDomain::Type))
            .map(|target| self.bound_target(target, String::new()))
            .unwrap_or(TypeExpr::Unknown);
        let mut cursor = node.walk();
        if !node
            .children(&mut cursor)
            .any(|n| n.kind() == forms.receiver_reference)
        {
            return Some((inner, TypeExpr::Unknown));
        }
        let mut cursor = node.walk();
        let region = node
            .named_children(&mut cursor)
            .find(|n| n.kind() == forms.lifetime_node)
            .map(|n| self.lifetime_expr(n))
            .or_else(|| self.elided_region(node))
            .unwrap_or(TypeExpr::Unknown);
        let mut cursor = node.walk();
        let mutability = if node
            .named_children(&mut cursor)
            .any(|n| n.kind() == forms.mutable_node)
        {
            Mutability::Mutable
        } else {
            Mutability::Shared
        };
        Some((
            TypeExpr::Indirect {
                kind: Indirection::Reference(Lifetime::Unknown),
                mutability,
                region: Some(Box::new(region.clone())),
                inner: Box::new(inner),
            },
            region,
        ))
    }
}

#[cfg(test)]
#[path = "namespace_receivers_tests.rs"]
mod tests;
