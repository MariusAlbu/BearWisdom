//! Infer source object and IIFE results without executing their bodies.
use super::*;
use crate::type_checker::core::types::{
    CallableOrigin, Intrinsic, LitValue, ObjectOrigin, SourceObject, TypeProperty,
};

impl Solver<'_> {
    pub(super) fn object(&self, source: SourceInstanceId, span: SourceSpan) -> Option<TypeId> {
        self.memoized(Key::Object(source, span), || {
            let input = self
                .initializers
                .objects
                .get(&(source, span))
                .copied()
                .flatten()?;
            let origin = ObjectOrigin::new(self.view.context, source.ordinal(), span);
            let layout = self.view.objects.members(origin)?;
            let members = input.members.as_ref()?;
            if members.len() != layout.len() || members.len() > 4096 {
                return None;
            }
            let mut keys = FxHashSet::default();
            let mut properties = Vec::new();
            for (input, member) in members.iter().zip(layout) {
                if input.span != member.span || !keys.insert(member.key) {
                    return None;
                }
                let value = widen(self.arena, self.operand(source, &input.value, 0)?);
                properties.push(TypeProperty {
                    key: member.key,
                    value,
                    optional: false,
                    readonly: false,
                    index: false,
                });
            }
            Some(
                self.arena
                    .intern(Type::Object(Box::new(SourceObject { origin, properties }))),
            )
        })
    }

    pub(super) fn operand(
        &self,
        source: SourceInstanceId,
        expression: &Expression<Recipe>,
        depth: usize,
    ) -> Option<TypeId> {
        if depth >= 64 {
            return None;
        }
        match expression {
            Expression::Typed(recipe) => Some(self.recipe(source, recipe)),
            _ => self.expression(source, expression, depth),
        }
    }

    pub(super) fn callable(
        &self,
        source: SourceInstanceId,
        id: SignatureId,
        body: &Expression<Recipe>,
        depth: usize,
    ) -> Option<TypeId> {
        let mut signature = self.bound_signature(source, id)?;
        if signature.syntax.result.is_none() {
            signature.result = Some(widen(self.arena, self.operand(source, body, depth + 1)?));
        }
        let callable = signature.callable(
            CallableOrigin::new(self.view.context, source.ordinal(), id.0),
            self.arena,
        )?;
        Some(self.arena.intern(Type::Callable(Box::new(callable))))
    }

    pub(super) fn iife(
        &self,
        source: SourceInstanceId,
        id: SignatureId,
        arguments: &[Expression<Recipe>],
        body: &Expression<Recipe>,
        depth: usize,
    ) -> Option<TypeId> {
        let mut signature = self.bound_signature(source, id)?;
        let actual = arguments
            .iter()
            .map(|arg| self.operand(source, arg, depth + 1))
            .collect::<Option<Vec<_>>>()?;
        let result = self.operand(source, body, depth + 1)?;
        let lookup = self.lookup(source);
        let relation = super::super::super::merge_proof::types::Relation {
            lookup: &lookup,
            arena: self.arena,
        };
        if signature.syntax.result.is_some() {
            if !relation.argument(result, signature.result?)? {
                return None;
            }
        } else {
            signature.result = Some(widen(self.arena, result));
        }
        Some(
            super::super::super::call_arguments::applicable(&relation, &signature, &actual, &[])??
                .result,
        )
    }
}

fn widen(arena: &TypeArena, ty: TypeId) -> TypeId {
    let kind = match arena.get(ty) {
        Type::Literal(LitValue::Str(_) | LitValue::Utf16(_)) => Intrinsic::String,
        Type::Literal(LitValue::Int(_) | LitValue::Number(_)) => Intrinsic::Number,
        Type::Literal(LitValue::Bool(_)) => Intrinsic::Boolean,
        Type::Literal(LitValue::BigInt { .. }) => Intrinsic::BigInt,
        _ => return ty,
    };
    arena.intern(Type::Intrinsic(kind))
}
