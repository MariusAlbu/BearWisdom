//! Deferred type operations. An interned operation is identity, not a proof
//! of evaluation, assignability or completeness of any of its operands.
use super::{TypeArena, TypeId};

/// Source modifiers are distinct: omission preserves a mapped source modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum MappedModifier {
    Preserve,
    Add,
    Remove,
}

/// Structural keys are singleton/domain types, never strings used for lookup.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TypeProperty<T> {
    pub key: T,
    pub value: T,
    pub optional: bool,
    pub readonly: bool,
    pub index: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TypeOperator<T> {
    /// Captured object structure. Completeness/equality still require evaluation.
    Object(Vec<TypeProperty<T>>),
    Mapped {
        parameter: T,
        keys: T,
        remap: Option<T>,
        value: T,
        optional: MappedModifier,
        readonly: MappedModifier,
    },
    KeyOf(T),
    Readonly(T),
    IndexedAccess {
        object: T,
        index: T,
    },
    /// Pattern-only source binder. Evaluation must not treat it as a value.
    Infer(T),
    Conditional {
        check: T,
        extends: T,
        when_true: T,
        when_false: T,
        /// Source-attested naked parameter, non-parameter, or deferred until
        /// an alias/head is resolved. Substitution must not recompute this.
        distributive: Option<bool>,
    },
}

impl<T> TypeOperator<T> {
    pub fn operands(&self) -> impl Iterator<Item = &T> {
        let (fixed, properties): (_, &[TypeProperty<T>]) = match self {
            Self::Object(properties) => ([None, None, None, None], properties),
            Self::Mapped {
                parameter,
                keys,
                remap,
                value,
                ..
            } => (
                [Some(parameter), Some(keys), remap.as_ref(), Some(value)],
                &[],
            ),
            Self::KeyOf(inner) | Self::Readonly(inner) | Self::Infer(inner) => {
                ([Some(inner), None, None, None], &[])
            }
            Self::IndexedAccess { object, index } => ([Some(object), Some(index), None, None], &[]),
            Self::Conditional {
                check,
                extends,
                when_true,
                when_false,
                ..
            } => (
                [
                    Some(check),
                    Some(extends),
                    Some(when_true),
                    Some(when_false),
                ],
                &[],
            ),
        };
        fixed
            .into_iter()
            .flatten()
            .chain(properties.iter().flat_map(|p| [&p.key, &p.value]))
    }

    pub fn map<'a, U>(&'a self, mut map: impl FnMut(&'a T) -> U) -> TypeOperator<U> {
        match self {
            Self::Object(properties) => TypeOperator::Object(
                properties
                    .iter()
                    .map(|p| TypeProperty {
                        key: map(&p.key),
                        value: map(&p.value),
                        optional: p.optional,
                        readonly: p.readonly,
                        index: p.index,
                    })
                    .collect(),
            ),
            Self::Mapped {
                parameter,
                keys,
                remap,
                value,
                optional,
                readonly,
            } => TypeOperator::Mapped {
                parameter: map(parameter),
                keys: map(keys),
                remap: remap.as_ref().map(&mut map),
                value: map(value),
                optional: *optional,
                readonly: *readonly,
            },
            Self::KeyOf(inner) => TypeOperator::KeyOf(map(inner)),
            Self::Readonly(inner) => TypeOperator::Readonly(map(inner)),
            Self::Infer(inner) => TypeOperator::Infer(map(inner)),
            Self::IndexedAccess { object, index } => TypeOperator::IndexedAccess {
                object: map(object),
                index: map(index),
            },
            Self::Conditional {
                check,
                extends,
                when_true,
                when_false,
                distributive,
            } => TypeOperator::Conditional {
                check: map(check),
                extends: map(extends),
                when_true: map(when_true),
                when_false: map(when_false),
                distributive: *distributive,
            },
        }
    }
}

impl TypeOperator<TypeId> {
    pub(super) fn format(&self, arena: &TypeArena, out: &mut String) {
        match self {
            Self::Infer(parameter) => {
                out.push_str("infer ");
                arena.format_type_into(*parameter, out);
            }
            Self::Object(properties) => {
                out.push('{');
                for (index, p) in properties.iter().enumerate() {
                    if index > 0 {
                        out.push_str("; ");
                    }
                    if p.readonly {
                        out.push_str("readonly ");
                    }
                    if p.index {
                        out.push('[');
                    }
                    arena.format_type_into(p.key, out);
                    if p.index {
                        out.push(']');
                    }
                    if p.optional {
                        out.push('?');
                    }
                    out.push_str(": ");
                    arena.format_type_into(p.value, out);
                }
                out.push('}');
            }
            Self::Mapped {
                parameter,
                keys,
                remap,
                value,
                optional,
                readonly,
            } => {
                out.push_str("{ ");
                match readonly {
                    MappedModifier::Preserve => {}
                    MappedModifier::Add => out.push_str("+readonly "),
                    MappedModifier::Remove => out.push_str("-readonly "),
                }
                out.push('[');
                arena.format_type_into(*parameter, out);
                out.push_str(" in ");
                arena.format_type_into(*keys, out);
                if let Some(remap) = remap {
                    out.push_str(" as ");
                    arena.format_type_into(*remap, out);
                }
                out.push(']');
                match optional {
                    MappedModifier::Preserve => {}
                    MappedModifier::Add => out.push_str("+?"),
                    MappedModifier::Remove => out.push_str("-?"),
                }
                out.push_str(": ");
                arena.format_type_into(*value, out);
                out.push_str(" }");
            }
            Self::KeyOf(inner) | Self::Readonly(inner) => {
                out.push_str(if matches!(self, Self::KeyOf(_)) {
                    "keyof ("
                } else {
                    "readonly ("
                });
                arena.format_type_into(*inner, out);
                out.push(')');
            }
            Self::IndexedAccess { object, index } => {
                out.push('(');
                arena.format_type_into(*object, out);
                out.push_str(")[");
                arena.format_type_into(*index, out);
                out.push(']');
            }
            Self::Conditional {
                check,
                extends,
                when_true,
                when_false,
                ..
            } => {
                out.push('(');
                arena.format_type_into(*check, out);
                out.push_str(" extends ");
                arena.format_type_into(*extends, out);
                out.push_str(" ? ");
                arena.format_type_into(*when_true, out);
                out.push_str(" : ");
                arena.format_type_into(*when_false, out);
                out.push(')');
            }
        }
    }
}

#[cfg(test)]
#[path = "type_operators_tests.rs"]
mod tests;
