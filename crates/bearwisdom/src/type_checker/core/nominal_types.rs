//! Numeric nominal identity and cached context provenance. No display-name keys.
use super::{Type, TypeArena, TypeId};
use std::{
    num::NonZeroU32,
    sync::atomic::{AtomicU64, Ordering},
};

/// Opaque, process-local configured-program context. Serialized values are
/// reminted on arena hydration; persistence is not authority to revive a handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NominalContextId(u64);

impl NominalContextId {
    pub(crate) fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(
            NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("nominal context identity overflow"),
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    Neutral,
    Legacy,
    Configured(NominalContextId),
    Mixed,
}

impl Scope {
    pub(super) fn capture(ty: &Type, children: &[Self]) -> Self {
        let child = |id: TypeId| children.get(id.index()).copied().unwrap_or(Self::Mixed);
        let many = |ids: &[TypeId]| {
            ids.iter()
                .fold(Self::Neutral, |scope, &id| scope.join(child(id)))
        };
        match ty {
            Type::Class(_) | Type::Decl { context: None, .. } => Self::Legacy,
            Type::Decl {
                context: Some(id), ..
            } => Self::Configured(*id),
            Type::UniqueSymbol(origin) => Self::Configured(origin.context),
            Type::Callable(c) => c
                .operands()
                .fold(Self::Configured(c.origin.context), |scope, &id| {
                    scope.join(child(id))
                }),
            Type::Object(object) => object
                .operands()
                .fold(Self::Configured(object.origin.context), |scope, &id| {
                    scope.join(child(id))
                }),
            Type::Apply { base, args } => child(*base).join(many(args)),
            Type::Operator(op) => op
                .operands()
                .fold(Self::Neutral, |scope, &id| scope.join(child(id))),
            Type::Function { params, return_ } => many(params).join(child(*return_)),
            Type::Tuple(items) | Type::Union(items) | Type::Intersection(items) => many(items),
            Type::Optional(inner)
            | Type::AsyncWrapper(inner)
            | Type::Iterator(inner)
            | Type::Constructor(inner)
            | Type::Indirect { inner, .. } => child(*inner),
            Type::Primitive(_)
            | Type::Intrinsic(_)
            | Type::Literal(_)
            | Type::Unknown
            | Type::Generic { .. }
            | Type::Region(_) => Self::Neutral,
        }
    }
    fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Neutral, scope) | (scope, Self::Neutral) => scope,
            (a, b) if a == b => a,
            _ => Self::Mixed,
        }
    }
}

impl TypeArena {
    pub fn decl(&self, qname: &str, symbol_id: i64) -> TypeId {
        self.intern_decl(qname, symbol_id, None)
    }
    /// Ingestion into a selected configured program, retaining physical row IDs.
    pub fn decl_in(&self, context: NominalContextId, qname: &str, symbol_id: i64) -> TypeId {
        self.intern_decl(qname, symbol_id, Some(context))
    }
    pub(super) fn intern_decl(
        &self,
        qname: &str,
        symbol_id: i64,
        context: Option<NominalContextId>,
    ) -> TypeId {
        let key = (context, symbol_id);
        if let Some(&id) = self.inner.read().unwrap().decl_by_symbol.get(&key) {
            return id;
        }
        let mut inner = self.inner.write().unwrap();
        // Recheck AND insert under one lock: competing display payloads cannot
        // manufacture two TypeIds for the same nominal identity.
        if let Some(&id) = inner.decl_by_symbol.get(&key) {
            return id;
        }
        let id =
            TypeId(NonZeroU32::new((inner.types.len() + 1) as u32).expect("arena index overflow"));
        inner.types.push(Type::Decl {
            symbol_id,
            qname: qname.into(),
            context,
        });
        inner
            .nominal_scopes
            .push(context.map(Scope::Configured).unwrap_or(Scope::Legacy));
        inner.decl_by_symbol.insert(key, id);
        id
    }
    /// O(1), including nested applications/callables/unions. A foreign nominal
    /// anywhere in a type prevents interpreting it through this program's view.
    pub fn accepts_nominal_context(&self, ty: TypeId, context: Option<NominalContextId>) -> bool {
        match self.inner.read().unwrap().nominal_scopes.get(ty.index()) {
            Some(Scope::Neutral) => true,
            Some(Scope::Legacy) => context.is_none(),
            Some(Scope::Configured(id)) => context == Some(*id),
            Some(Scope::Mixed) | None => false,
        }
    }
}

pub(super) fn refresh_contexts(types: &mut [Type]) {
    let mut contexts = rustc_hash::FxHashMap::default();
    for ty in types {
        let id = match ty {
            Type::Decl {
                context: Some(id), ..
            } => id,
            Type::UniqueSymbol(origin) => &mut origin.context,
            Type::Callable(c) => &mut c.origin.context,
            Type::Object(object) => &mut object.origin.context,
            _ => continue,
        };
        *id = *contexts.entry(*id).or_insert_with(NominalContextId::fresh);
    }
}

#[cfg(test)]
#[path = "nominal_types_tests.rs"]
mod tests;
