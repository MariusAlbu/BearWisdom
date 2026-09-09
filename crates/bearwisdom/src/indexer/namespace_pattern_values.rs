//! Pattern leaves project source-bound payload fields rather than whole values.
use super::*;
use crate::indexer::lexical::type_syntax::ValueExpr;

pub(crate) struct Forms {
    pub arms: &'static [(&'static str, &'static str)],
    pub matches: &'static [(&'static str, &'static str, &'static str)],
    pub wrappers: &'static [&'static str],
    pub condition: &'static str,
    pub variants: &'static [(&'static str, &'static str, bool)],
    pub fields: &'static [(&'static str, &'static str, &'static str)],
    pub rest: &'static [&'static str],
    pub ordered_fields: &'static str,
    pub variant_declaration: &'static str,
}

impl Forms {
    pub(crate) fn positional_field(&self, node: Node) -> bool {
        node.parent().is_some_and(|p| {
            p.kind() == self.ordered_fields
                && p.parent()
                    .is_some_and(|owner| owner.kind() == self.variant_declaration)
        })
    }
}

impl Capture<'_> {
    pub(super) fn pattern_values(&mut self, node: Node, uses: &mut TypeUses) {
        let forms = self.forms.patterns;
        let Some(&(_, field)) = forms.arms.iter().find(|&&(kind, _)| kind == node.kind()) else {
            return;
        };
        let Some(pattern) = node.child_by_field_name(field) else {
            return;
        };
        // Unsupported shapes still own their bindings; never inherit an outer
        // namesake's fact or let legacy inference fabricate a payload type.
        self.unknown_pattern(pattern, uses);
        let mut ancestor = node.parent();
        while let Some(parent) = ancestor {
            if let Some(&(_, value, body)) = forms
                .matches
                .iter()
                .find(|&&(kind, _, _)| kind == parent.kind())
            {
                let Some(body) = parent.child_by_field_name(body) else {
                    return;
                };
                if body.start_byte() > node.start_byte() || body.end_byte() < node.end_byte() {
                    return;
                }
                let value = parent
                    .child_by_field_name(value)
                    .and_then(|n| self.value(n, 0))
                    .unwrap_or(ValueExpr::Unknown);
                self.project_pattern(pattern, value, uses, 0);
                return;
            }
            if self.forms.scopes.contains(&parent.kind()) {
                return;
            }
            ancestor = parent.parent();
        }
    }

    fn unknown_pattern(&self, node: Node, uses: &mut TypeUses) {
        if let Some(&binding) = self.data.graph.declarations.get(&SourceSpan {
            start: node.start_byte() as u32,
            end: node.end_byte() as u32,
        }) {
            uses.values.insert(binding, ValueExpr::Unknown);
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.unknown_pattern(child, uses);
        }
    }

    fn project_pattern(&mut self, node: Node, value: ValueExpr, uses: &mut TypeUses, depth: usize) {
        if depth >= 32 || node.has_error() {
            return;
        }
        let forms = self.forms.patterns;
        if forms.wrappers.contains(&node.kind()) {
            let mut cursor = node.walk();
            let children: Vec<_> = node
                .named_children(&mut cursor)
                .filter(|n| !n.is_extra() && node.child_by_field_name(forms.condition) != Some(*n))
                .collect();
            if children.len() == 1 {
                self.project_pattern(children[0], value, uses, depth + 1);
            }
            return;
        }
        if self.forms.locals.identifiers.contains(&node.kind()) {
            if let Some(binding) = self.direct_binding(node) {
                uses.values.insert(binding, value);
            }
            return;
        }
        let Some(&(_, field, named)) = forms
            .variants
            .iter()
            .find(|&&(kind, _, _)| kind == node.kind())
        else {
            return;
        };
        let Some(path) = node.child_by_field_name(field) else {
            return;
        };
        let Some(&(_, owner, selector)) = self
            .forms
            .path_nodes
            .iter()
            .find(|&&(kind, _, _)| kind == path.kind())
        else {
            return;
        };
        let (Some(owner), Some(selector)) = (
            path.child_by_field_name(owner),
            path.child_by_field_name(selector),
        ) else {
            return;
        };
        let byte = selector.start_byte() as u32;
        let head = traits::strict(self.expr(owner));
        uses.pattern_heads.insert(byte, head);
        let variant = self.data.intern(
            selector.utf8_text(self.source).unwrap_or_default(),
            self.forms,
        );
        let mut cursor = node.walk();
        let fields: Vec<_> = node
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra() && *n != path)
            .collect();
        // Positional rest needs arity evidence; do not shift later fields.
        if !named && fields.iter().any(|n| forms.rest.contains(&n.kind())) {
            return;
        }
        for (position, child) in fields.into_iter().enumerate() {
            let (field, leaf) = if named {
                let Some(&(_, name, pattern)) = forms
                    .fields
                    .iter()
                    .find(|&&(kind, _, _)| kind == child.kind())
                else {
                    continue;
                };
                let Some(name) = child.child_by_field_name(name) else {
                    continue;
                };
                let field = self
                    .data
                    .intern(name.utf8_text(self.source).unwrap_or_default(), self.forms);
                (field, child.child_by_field_name(pattern).unwrap_or(name))
            } else {
                (self.data.intern(&position.to_string(), self.forms), child)
            };
            self.project_pattern(
                leaf,
                ValueExpr::VariantField {
                    variant,
                    field,
                    byte,
                    operand: Box::new(value.clone()),
                },
                uses,
                depth + 1,
            );
        }
    }
}

#[cfg(test)]
#[path = "namespace_pattern_values_tests.rs"]
mod tests;
