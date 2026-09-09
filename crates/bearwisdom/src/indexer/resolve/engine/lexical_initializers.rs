//! Publish configured initializer results through source declaration BindingIds.
use super::*;
use crate::indexer::lexical::type_syntax::initializers::Expression;

impl LexicalCache<'_> {
    pub(super) fn install_initializers(
        &mut self,
        selected: &dyn super::super::contract::SymbolLookup,
    ) {
        let mut values = HashMap::new();
        for input in &self.bindings.types.initializers {
            if input.annotated {
                continue;
            }
            let Some(target) = input.target else {
                continue;
            };
            let Some(&binding) = self.bindings.declarations.get(&target) else {
                continue;
            };
            let value = selected
                .source_initializer_type(input.signature.0, target)
                .flatten();
            // Captured source objects, functions and constructors own their
            // results. Their misses must erase legacy nominal-head seeds;
            // unsupported expression families retain their existing evaluator.
            if value.is_none()
                && !matches!(
                    input.expression,
                    Expression::Construct { .. }
                        | Expression::Object(_)
                        | Expression::Callable { .. }
                        | Expression::Iife { .. }
                )
            {
                continue;
            }
            values
                .entry(binding)
                .and_modify(|old| *old = None)
                .or_insert(value);
        }
        for (binding, value) in values {
            self.initial_types.insert(
                binding,
                value.unwrap_or_else(|| self.arena.intern(Type::Unknown)),
            );
        }
    }
}
