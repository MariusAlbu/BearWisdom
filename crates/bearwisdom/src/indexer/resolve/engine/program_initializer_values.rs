//! Runtime constructor evidence and initializer dependencies belong to a source view.
use super::*;
use crate::indexer::lexical::globals::member_surface::{Kind, Modifier};
use crate::indexer::resolve::engine::head_decl::head_decl_id;
use crate::type_checker::core::types::GenericParamId;
use program_types::{
    initializers::{Class, Expression, Input},
    source_signatures::Bound,
};
use rustc_hash::FxHashSet;

use super::super::constructors::{self as selection, Candidate};
use crate::indexer::resolve::engine::contract::flow_cache::CallSignatureOrigin;
#[path = "program_object_values.rs"]
mod objects;

#[derive(Default)]
pub(super) struct Inputs<'a> {
    objects:
        FxHashMap<(SourceInstanceId, SourceSpan), Option<&'a program_types::initializers::Object>>,
    values: FxHashMap<(SourceInstanceId, SignatureId), Option<&'a Input>>,
    pub fields: FxHashMap<i64, Option<(SourceInstanceId, SignatureId)>>,
    classes: FxHashMap<i64, Vec<(SourceInstanceId, &'a Class)>>,
}
impl<'a> Inputs<'a> {
    pub(super) fn new(inputs: &Sources<'a>, view: &View) -> Self {
        let mut result = Self::default();
        for (_, &source, input) in inputs {
            for object in &input.objects {
                result
                    .objects
                    .entry((source, object.span))
                    .and_modify(|old| *old = None)
                    .or_insert(Some(object));
            }
            for value in &input.initializers {
                result
                    .values
                    .entry((source, value.signature))
                    .and_modify(|old| *old = None)
                    .or_insert(Some(value));
                if let Some(owner) = value.declaration.and_then(|row| view.canonical.get(&row)) {
                    result
                        .fields
                        .entry(*owner)
                        .and_modify(|old| *old = None)
                        .or_insert(Some((source, value.signature)));
                }
            }
            for class in &input.classes {
                if let Some(&owner) = view.canonical.get(&class.owner) {
                    result
                        .classes
                        .entry(owner)
                        .or_default()
                        .push((source, class));
                }
            }
        }
        result
    }
}

impl Solver<'_> {
    pub(super) fn initializer(
        &self,
        source: SourceInstanceId,
        signature: SignatureId,
    ) -> Option<TypeId> {
        let result = self.memoized(Key::Initializer(source, signature), || {
            let input = self
                .initializers
                .values
                .get(&(source, signature))
                .copied()
                .flatten()?;
            if input.annotated {
                return None;
            }
            if let Expression::Construct {
                callee,
                arguments,
                types,
            } = &input.expression
            {
                let call = self.construct(source, *callee, arguments, types, 0)?;
                let result = call.return_type;
                self.constructor_calls
                    .borrow_mut()
                    .insert((source, signature), call);
                Some(result)
            } else {
                self.expression(source, &input.expression, 0)
            }
        });
        if result.is_none() {
            self.constructor_calls
                .borrow_mut()
                .remove(&(source, signature));
        }
        result
    }
    fn expression(
        &self,
        source: SourceInstanceId,
        expression: &Expression<Recipe>,
        depth: usize,
    ) -> Option<TypeId> {
        if depth >= 64 {
            return None;
        }
        match expression {
            Expression::Unknown => None,
            Expression::Object(span) => self.object(source, *span),
            Expression::Callable { signature, body } => {
                self.callable(source, *signature, body, depth)
            }
            Expression::Iife {
                signature,
                arguments,
                body,
            } => self.iife(source, *signature, arguments, body, depth),
            Expression::Read(site) => self.query(source, *site),
            // Literal widening/const context needs declaration-owned evidence.
            // Typed operands are used by constructor applicability, not published
            // as unqualified mutable-field inference.
            Expression::Typed(_) => None,
            Expression::Construct {
                callee,
                arguments,
                types,
            } => self
                .construct(source, *callee, arguments, types, depth)
                .map(|call| call.return_type),
        }
    }
    fn construct(
        &self,
        source: SourceInstanceId,
        callee: SourceSpan,
        arguments: &[Expression<Recipe>],
        types: &[Recipe],
        depth: usize,
    ) -> Option<selection::Call> {
        let callee = self.query(source, callee)?;
        let arguments = arguments
            .iter()
            .map(|arg| match arg {
                Expression::Typed(recipe) => Some(self.recipe(source, recipe)),
                _ => self.expression(source, arg, depth + 1),
            })
            .collect::<Option<Vec<_>>>()?;
        let types = types
            .iter()
            .map(|ty| self.recipe(source, ty))
            .collect::<Vec<_>>();
        let candidates =
            self.constructors(callee, &mut FxHashSet::default(), false, &Cell::new(4096))?;
        selection::select(
            &self.lookup(source),
            self.arena,
            candidates,
            &arguments,
            &types,
        )
    }
    fn bound_signature(&self, source: SourceInstanceId, id: SignatureId) -> Option<Bound> {
        let input = self.signatures.get(&(source, id)).copied().flatten()?;
        let mut bound = self.view.sources.get(&source)?.signatures.get(&id)?.clone();
        program_types::source_signatures::materialize_with_query(
            input,
            &mut bound,
            &self.lookup(source),
            self.arena,
            self.sources[&source].0,
            &|site| self.query(source, site),
        );
        (bound.generic_parameters.len() == input.generics.len()).then_some(bound)
    }
    fn constructors(
        &self,
        receiver: TypeId,
        seen: &mut FxHashSet<i64>,
        inherited: bool,
        remaining: &Cell<usize>,
    ) -> Option<Vec<Candidate>> {
        remaining.set(remaining.get().checked_sub(1)?);
        let receiver = self.expand(receiver)?;
        let (ty, class_value) = match self.arena.get(receiver) {
            Type::Constructor(inner) => (inner, true),
            _ => (receiver, false),
        };
        let owner = *self.view.canonical.get(&head_decl_id(self.arena, ty)?)?;
        if seen.len() >= 64 || !seen.insert(owner) {
            return None;
        }
        let result = if class_value {
            self.class_constructors(ty, owner, seen, inherited, remaining)
        } else {
            self.interface_constructors(ty, owner, seen, remaining)
        };
        seen.remove(&owner);
        result
    }
    fn interface_constructors(
        &self,
        ty: TypeId,
        owner: i64,
        seen: &mut FxHashSet<i64>,
        remaining: &Cell<usize>,
    ) -> Option<Vec<Candidate>> {
        let parts = self.interfaces.get(&owner)?;
        let params = &self.view.info.get(&owner)?.generic_param_ids;
        let args = match self.arena.get(ty) {
            Type::Apply { args, .. } => args,
            _ => vec![],
        };
        if params.len() != args.len() {
            return None;
        }
        let bindings = params.iter().copied().zip(args).collect();
        let mut result = Vec::new();
        for (source, part) in parts {
            for member in part
                .surface
                .as_ref()?
                .iter()
                .filter(|m| m.kind == Kind::Construct)
            {
                remaining.set(remaining.get().checked_sub(1)?);
                if member.modifiers.iter().any(|m| {
                    matches!(
                        m,
                        Modifier::Abstract
                            | Modifier::Private
                            | Modifier::Protected
                            | Modifier::Static
                    )
                }) {
                    return None;
                }
                result.push(Candidate {
                    owner,
                    origin: Some(CallSignatureOrigin {
                        source: *source,
                        span: member.span,
                        declaration: member.slot,
                    }),
                    signature: substitute(
                        self.arena,
                        self.bound_signature(*source, SignatureId(member.span))?,
                        &bindings,
                    ),
                });
            }
        }
        // Inherited owner blocks follow declared base order. Across merged
        // providers this needs source binding order too, even if their
        // constructor signatures live in a single third source.
        let mut parts: Vec<_> = parts.iter().collect();
        let providers: FxHashSet<_> = parts
            .iter()
            .filter(|(_, part)| part.bases.as_ref().is_some_and(|b| !b.is_empty()))
            .map(|(source, _)| *source)
            .collect();
        let ordered = providers.len() < 2 || self.view.source_binding_order.is_some();
        if let Some(order) = &self.view.source_binding_order {
            parts.sort_by_key(|(source, _)| order.get(source).copied());
        }
        for (source, part) in parts {
            if part.inherited_members
                != crate::indexer::lexical::globals::InheritedMembers::DeclarationOrder
            {
                return None;
            }
            for base in part.bases.as_ref()? {
                let base = self.expand(self.recipe(*source, base))?;
                result.extend(self.constructors(
                    generic_return::substitute(self.arena, base, &bindings),
                    seen,
                    true,
                    remaining,
                )?);
            }
        }
        if !ordered {
            for candidate in &mut result {
                candidate.signature.syntax.ordering = None;
            }
        }
        Some(result)
    }
    fn class_constructors(
        &self,
        ty: TypeId,
        owner: i64,
        seen: &mut FxHashSet<i64>,
        inherited: bool,
        remaining: &Cell<usize>,
    ) -> Option<Vec<Candidate>> {
        let [(source, class)] = self.initializers.classes.get(&owner)?.as_slice() else {
            return None;
        };
        if class.abstract_ && !inherited {
            return None;
        }
        let mut generics = match class.generics {
            Some(id) => self.bound_signature(*source, id)?,
            None => Bound::default(),
        };
        let params = &self.view.info.get(&owner)?.generic_param_ids;
        if *params != generics.generic_parameters {
            return None;
        }
        let (base, args) = match self.arena.get(ty) {
            Type::Apply { base, args } => (base, Some(args)),
            _ => (ty, None),
        };
        let bindings = match &args {
            Some(args) if args.len() == params.len() => {
                params.iter().copied().zip(args.iter().copied()).collect()
            }
            Some(_) => return None,
            None => FxHashMap::default(),
        };
        let result = if params.is_empty() {
            base
        } else {
            self.arena.intern(Type::Apply {
                base,
                args: args.unwrap_or_else(|| {
                    params.iter().map(|&p| self.arena.generic_type(p)).collect()
                }),
            })
        };
        let members: Vec<_> = class
            .surface
            .as_ref()?
            .iter()
            .filter(|m| m.kind == Kind::Construct)
            .collect();
        let mut candidates = Vec::new();
        for member in members.iter().filter(|m| {
            !members.iter().any(|other| other.signature.body.is_none())
                || m.signature.body.is_none()
        }) {
            remaining.set(remaining.get().checked_sub(1)?);
            if member
                .modifiers
                .iter()
                .any(|m| matches!(m, Modifier::Private | Modifier::Protected))
            {
                return None;
            }
            let signature = self.bound_signature(*source, SignatureId(member.span))?;
            if !signature.generic_parameters.is_empty() {
                return None;
            }
            candidates.push(Candidate {
                owner,
                origin: Some(CallSignatureOrigin {
                    source: *source,
                    span: member.span,
                    declaration: member.slot,
                }),
                signature,
            });
        }
        if members.is_empty() {
            let bases = if class.has_base {
                self.bases(owner)?
            } else {
                vec![]
            };
            match bases.as_slice() {
                [] => candidates.push(Candidate {
                    owner,
                    origin: None,
                    signature: Bound::default(),
                }),
                [base] => candidates.extend(
                    self.constructors(
                        self.arena
                            .intern(Type::Constructor(generic_return::substitute(
                                self.arena, *base, &bindings,
                            ))),
                        seen,
                        true,
                        remaining,
                    )?,
                ),
                _ => return None,
            }
        }
        if !bindings.is_empty() {
            generics = substitute(self.arena, generics, &bindings);
            generics.generic_parameters.clear();
            generics.constraints.clear();
            generics.defaults.clear();
        }
        for candidate in &mut candidates {
            let signature = &mut candidate.signature;
            *signature = substitute(self.arena, signature.clone(), &bindings);
            signature.generic_parameters = generics.generic_parameters.clone();
            signature.constraints = generics.constraints.clone();
            signature.defaults = generics.defaults.clone();
            signature.result = Some(result);
        }
        Some(candidates)
    }
}

fn substitute(
    arena: &TypeArena,
    mut bound: Bound,
    bindings: &FxHashMap<GenericParamId, TypeId>,
) -> Bound {
    let rewrite = |ty| generic_return::substitute(arena, ty, bindings);
    bound.parameters = bound.parameters.into_iter().map(rewrite).collect();
    bound.result = bound.result.map(rewrite);
    bound.constraints = bound
        .constraints
        .into_iter()
        .map(|ty| ty.map(rewrite))
        .collect();
    bound.defaults = bound
        .defaults
        .into_iter()
        .map(|ty| ty.map(rewrite))
        .collect();
    bound
}

#[cfg(test)]
#[path = "program_initializer_values_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "program_structural_arguments_tests.rs"]
mod structural_tests;

#[cfg(test)]
#[path = "program_local_initializers_tests.rs"]
mod local_tests;

#[cfg(test)]
#[path = "program_constructor_selection_tests.rs"]
mod selection_tests;

#[cfg(test)]
#[path = "program_object_initializers_tests.rs"]
mod object_tests;
