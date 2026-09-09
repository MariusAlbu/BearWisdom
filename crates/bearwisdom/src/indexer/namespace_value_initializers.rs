//! Bind local initializer operands once at their actual source positions.
use super::*;
use crate::indexer::lexical::type_syntax::ValueExpr;

impl Capture<'_> {
    pub(super) fn initializer(&mut self, node: Node, uses: &mut TypeUses) {
        let Some(&(_, lhs, rhs)) = self
            .forms
            .locals
            .declarations
            .iter()
            .find(|&&(kind, _, _)| kind == node.kind())
        else {
            return;
        };
        let (Some(pattern), Some(value)) =
            (node.child_by_field_name(lhs), node.child_by_field_name(rhs))
        else {
            return;
        };
        let Some(binding) = self.direct_binding(pattern) else {
            return;
        };
        if let Some(recipe) = self.value(value, 0) {
            uses.values.insert(binding, recipe);
        } else if let Some(recipe) = self.constructed(value) {
            uses.constructed.insert(binding, recipe.clone());
            if node
                .child_by_field_name(self.forms.types.annotation)
                .is_none()
            {
                if let Some(slot) = self
                    .data
                    .graph
                    .symbol_slots
                    .get(&binding)
                    .copied()
                    .flatten()
                {
                    uses.fields.insert(slot, recipe);
                }
            }
        }
    }

    pub(super) fn place_expression(&mut self, node: Node, uses: &mut TypeUses) {
        if self.forms.places.recognizes(node) {
            uses.expressions.insert(
                SourceSpan {
                    start: node.start_byte() as u32,
                    end: node.end_byte() as u32,
                },
                self.value(node, 0).unwrap_or(ValueExpr::Unknown),
            );
        }
    }

    pub(super) fn value(&mut self, node: Node, depth: usize) -> Option<ValueExpr> {
        if depth >= 32 || node.has_error() {
            return Some(ValueExpr::Unknown);
        }
        if self.forms.argument_groups.contains(&node.kind()) {
            let mut cursor = node.walk();
            let mut children = node.named_children(&mut cursor).filter(|n| !n.is_extra());
            let Some(child) = children.next() else {
                return Some(ValueExpr::Unknown);
            };
            return if children.next().is_none() {
                self.value(child, depth + 1)
            } else {
                Some(ValueExpr::Unknown)
            };
        }
        if self.forms.locals.identifiers.contains(&node.kind()) {
            let byte = node.start_byte() as u32;
            let binding = node
                .utf8_text(self.source)
                .ok()
                .map(|s| s.strip_prefix(self.forms.raw_prefix).unwrap_or(s))
                .and_then(|name| self.data.graph.name_id(name))
                .and_then(|name| self.data.graph.binding_at(byte, name));
            return Some(
                binding
                    .map(|binding| ValueExpr::Read { binding, byte })
                    .unwrap_or(ValueExpr::Unknown),
            );
        }
        if self.forms.places.recognizes(node) {
            use crate::languages::common::call_args::Place;
            return Some(match self.forms.places.capture(node) {
                Some(Place::Field(operand, selector)) => {
                    let operand =
                        Box::new(self.value(operand, depth + 1).unwrap_or(ValueExpr::Unknown));
                    let text = selector.utf8_text(self.source).unwrap_or_default();
                    if self.forms.places.named_selectors.contains(&selector.kind()) {
                        ValueExpr::Field {
                            name: self.data.intern(text, self.forms),
                            byte: selector.start_byte() as u32,
                            operand,
                        }
                    } else if self.forms.places.tuple_selectors.contains(&selector.kind()) {
                        text.parse()
                            .ok()
                            .map(|index| ValueExpr::TupleIndex { index, operand })
                            .unwrap_or(ValueExpr::Unknown)
                    } else {
                        ValueExpr::Unknown
                    }
                }
                Some(Place::Dereference(operand)) => ValueExpr::Dereference {
                    operand: Box::new(self.value(operand, depth + 1).unwrap_or(ValueExpr::Unknown)),
                },
                None => ValueExpr::Unknown,
            });
        }
        let forms = self.forms.borrows?;
        if node.kind() != forms.node {
            return None;
        }
        let span = SourceSpan {
            start: node.start_byte() as u32,
            end: node.end_byte() as u32,
        };
        let Some((operand, mutability)) = forms.capture(node) else {
            return Some(ValueExpr::Unknown);
        };
        let Some(&(owner, attested)) = self.data.borrow_sites.get(&span) else {
            return Some(ValueExpr::Unknown);
        };
        if mutability != attested {
            return Some(ValueExpr::Unknown);
        }
        Some(ValueExpr::Borrow {
            owner,
            span,
            mutability,
            operand: Box::new(self.value(operand, depth + 1).unwrap_or(ValueExpr::Unknown)),
        })
    }

    fn constructed(&mut self, node: Node) -> Option<TypeExpr> {
        if self.forms.argument_groups.contains(&node.kind()) {
            let mut cursor = node.walk();
            let mut children = node.named_children(&mut cursor).filter(|n| !n.is_extra());
            let child = children.next()?;
            if children.next().is_some() {
                return None;
            }
            return self.constructed(child);
        }
        if let Some(&(_, field)) = self
            .forms
            .types
            .constructions
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
        {
            return node.child_by_field_name(field).map(|ty| self.expr(ty));
        }
        if self.forms.types.tuple_values.contains(&node.kind()) {
            let mut cursor = node.walk();
            return Some(TypeExpr::Tuple(
                node.named_children(&mut cursor)
                    .filter(|n| !n.is_extra())
                    .map(|n| self.constructed(n).unwrap_or(TypeExpr::Unknown))
                    .collect(),
            ));
        }
        None
    }
}

#[cfg(test)]
#[path = "namespace_value_initializers_tests.rs"]
mod tests;
