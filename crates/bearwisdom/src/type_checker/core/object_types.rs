//! Anonymous object identity is source-owned; structural equality is a separate proof.
use super::{NominalContextId, TypeArena, TypeId, TypeOperator, TypeProperty};
use crate::types::SourceSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ObjectOrigin {
    pub(super) context: NominalContextId,
    source: usize,
    pub span: SourceSpan,
}
impl ObjectOrigin {
    pub(crate) fn new(context: NominalContextId, source: usize, span: SourceSpan) -> Self {
        Self {
            context,
            source,
            span,
        }
    }
    pub(crate) fn source(&self) -> usize {
        self.source
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SourceObject<T> {
    pub origin: ObjectOrigin,
    pub properties: Vec<TypeProperty<T>>,
}
impl<T> SourceObject<T> {
    pub fn operands(&self) -> impl Iterator<Item = &T> {
        self.properties.iter().flat_map(|p| [&p.key, &p.value])
    }
    pub fn map<'a, U>(&'a self, mut map: impl FnMut(&'a T) -> U) -> SourceObject<U> {
        SourceObject {
            origin: self.origin,
            properties: self
                .properties
                .iter()
                .map(|p| TypeProperty {
                    key: map(&p.key),
                    value: map(&p.value),
                    optional: p.optional,
                    readonly: p.readonly,
                    index: p.index,
                })
                .collect(),
        }
    }
}
impl SourceObject<TypeId> {
    pub(super) fn format(&self, arena: &TypeArena, out: &mut String) {
        TypeOperator::Object(self.properties.clone()).format(arena, out);
    }
}

#[cfg(test)]
#[path = "object_types_tests.rs"]
mod tests;
