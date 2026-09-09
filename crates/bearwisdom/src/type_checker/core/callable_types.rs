//! Complete source callable evidence. Origins are identities, not assignment proofs.
use super::{NominalContextId, TypeArena, TypeId};
use crate::types::SourceSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CallableOrigin {
    pub(super) context: NominalContextId,
    source: usize,
    pub signature: SourceSpan,
}
impl CallableOrigin {
    pub(crate) fn source(self) -> usize {
        self.source
    }
    pub(crate) fn new(context: NominalContextId, source: usize, signature: SourceSpan) -> Self {
        Self {
            context,
            source,
            signature,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CallableParameter<T> {
    /// Together with the callable origin, this is the parameter's source identity.
    pub declaration: SourceSpan,
    pub ty: T,
    pub optional: bool,
    pub rest: bool,
    pub receiver: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CallableGeneric<T> {
    pub parameter: T,
    pub constraint: Option<T>,
    pub default: Option<T>,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CallablePredicate<T> {
    /// Exact parameter slot in this signature, never its spelling.
    pub parameter: SourceSpan,
    pub asserted: Option<T>,
    pub asserts: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Callable<T, O = CallableOrigin> {
    pub origin: O,
    pub generics: Vec<CallableGeneric<T>>,
    pub parameters: Vec<CallableParameter<T>>,
    pub result: T,
    pub predicate: Option<CallablePredicate<T>>,
    pub complete: bool,
}
impl<T, O> Callable<T, O> {
    pub fn operands(&self) -> impl Iterator<Item = &T> {
        std::iter::once(&self.result)
            .chain(self.parameters.iter().map(|p| &p.ty))
            .chain(self.generics.iter().flat_map(|p| {
                [
                    Some(&p.parameter),
                    p.constraint.as_ref(),
                    p.default.as_ref(),
                ]
                .into_iter()
                .flatten()
            }))
            .chain(self.predicate.iter().filter_map(|p| p.asserted.as_ref()))
    }
    pub fn map<'a, U, P>(&'a self, origin: P, mut map: impl FnMut(&'a T) -> U) -> Callable<U, P> {
        Callable {
            origin,
            complete: self.complete,
            generics: self
                .generics
                .iter()
                .map(|p| CallableGeneric {
                    parameter: map(&p.parameter),
                    constraint: p.constraint.as_ref().map(&mut map),
                    default: p.default.as_ref().map(&mut map),
                })
                .collect(),
            parameters: self
                .parameters
                .iter()
                .map(|p| CallableParameter {
                    declaration: p.declaration,
                    ty: map(&p.ty),
                    optional: p.optional,
                    rest: p.rest,
                    receiver: p.receiver,
                })
                .collect(),
            result: map(&self.result),
            predicate: self.predicate.as_ref().map(|p| CallablePredicate {
                parameter: p.parameter,
                asserted: p.asserted.as_ref().map(&mut map),
                asserts: p.asserts,
            }),
        }
    }
}
impl Callable<TypeId> {
    pub(super) fn format(&self, arena: &TypeArena, out: &mut String) {
        out.push('(');
        for (i, p) in self.parameters.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            if p.receiver {
                out.push_str("this: ");
            }
            if p.rest {
                out.push_str("...");
            }
            arena.format_type_into(p.ty, out);
            if p.optional {
                out.push('?');
            }
        }
        out.push_str(") => ");
        if let Some(p) = &self.predicate {
            if p.asserts {
                out.push_str("asserts ");
            }
            out.push_str("parameter");
            if let Some(ty) = p.asserted {
                out.push_str(" is ");
                arena.format_type_into(ty, out);
            }
        } else {
            arena.format_type_into(self.result, out);
        }
    }
}

#[cfg(test)]
#[path = "callable_types_tests.rs"]
mod tests;
