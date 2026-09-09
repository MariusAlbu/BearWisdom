//! Each candidate gets a private lexical overlay; no shared contextual writes.
use super::merge_proof::types::Relation;
use super::*;
use crate::indexer::lexical::{
    globals::call_arguments::callbacks::{Callback, Expr},
    BindingId, LexicalBindings,
};
use crate::indexer::resolve::engine::contract::generic_return::substitute;
use crate::indexer::resolve::engine::contract::FlowCacheLookup;
use crate::type_checker::core::types::{
    Callable, CallableParameter, CallablePredicate, Intrinsic, LitValue, Type, TypeId,
};

#[path = "program_callback_predicates.rs"]
mod predicates;

pub(super) fn inventory(
    view: &mut View,
    source: SourceInstanceId,
    input: &super::super::program_types::Input,
) {
    use crate::indexer::lexical::globals::member_surface::{Kind, Modifier};
    for class in &input.classes {
        if !view.canonical.contains_key(&class.owner) {
            continue;
        }
        for member in class.surface.iter().flatten() {
            if member.kind != Kind::Method
                || member
                    .modifiers
                    .iter()
                    .any(|m| !matches!(m, Modifier::Abstract))
            {
                continue;
            }
            let Some(row) = member.slot.filter(|id| view.canonical.contains_key(id)) else {
                continue;
            };
            let signature = source_signatures::SignatureId(member.span);
            if !input
                .source_signatures
                .iter()
                .any(|s| s.id == signature && s.declaration == Some(row))
            {
                continue;
            }
            view.callable_members
                .entry(row)
                .and_modify(|prior| *prior = None)
                .or_insert(Some((source, signature)));
        }
    }
}

pub(in crate::indexer::resolve::engine) fn infer(
    lookup: &Lookup,
    file: &dyn SymbolLookup,
    graph: &LexicalBindings,
    callback: &Callback,
    context: TypeId,
) -> Option<TypeId> {
    let arena = lookup.type_arena()?;
    let relation = Relation { lookup, arena };
    let context = relation.canonical(context, 0)?;
    let Type::Callable(context) = arena.get(context) else {
        return None;
    };
    if !context.complete || !context.generics.is_empty() {
        return None;
    }
    let signature = lookup.signature(source_signatures::SignatureId(callback.signature))?;
    if !signature.generic_parameters.is_empty()
        || signature.parameters.len() != callback.parameters.len()
        || signature.syntax.parameters.len() != callback.parameters.len()
        || callback.parameters.len() > context.parameters.len()
    {
        return None;
    }
    let mut bindings = FxHashMap::default();
    let mut parameters = Vec::new();
    for (index, &site) in callback.parameters.iter().enumerate() {
        let source = &signature.syntax.parameters[index];
        let target = &context.parameters[index];
        if source.rest || source.receiver || target.rest || target.receiver {
            return None;
        }
        let ty = if source.type_span.is_some() {
            relation.canonical(signature.parameters[index], 0)?
        } else {
            relation.canonical(target.ty, 0)?
        };
        let binding = *graph.declarations.get(&site)?;
        if bindings.insert(binding, ty).is_some() {
            return None;
        }
        parameters.push(CallableParameter {
            declaration: source.span,
            ty,
            optional: source.optional,
            rest: false,
            receiver: false,
        });
    }
    let evaluator = Evaluation {
        lookup,
        file,
        graph,
        relation,
        bindings,
    };
    let inferred = evaluator.value(&callback.body, 0)?;
    let (result, predicate) = if signature.syntax.result.is_some() {
        let result = signature.result?;
        if !evaluator.relation.argument(inferred, result)? {
            return None;
        }
        (result, None)
    } else {
        let predicate = predicates::infer(arena, graph, callback, &parameters)?;
        (widen(arena, inferred), predicate)
    };
    Some(arena.intern(Type::Callable(Box::new(Callable {
        origin: lookup.source_callable_origin(callback.signature)?,
        parameters,
        generics: vec![],
        result,
        predicate,
        complete: true,
    }))))
}

struct Evaluation<'a> {
    lookup: &'a Lookup<'a>,
    file: &'a dyn SymbolLookup,
    graph: &'a LexicalBindings,
    relation: Relation<'a>,
    bindings: FxHashMap<BindingId, TypeId>,
}

impl Evaluation<'_> {
    fn value(&self, expression: &Expr, depth: usize) -> Option<TypeId> {
        if depth >= 64 {
            return None;
        }
        let arena = self.relation.arena;
        let ty = match expression {
            Expr::Unknown => return None,
            Expr::Atom(atom) => arena.intern(atom.ty()),
            Expr::Read(site) => self
                .graph
                .argument_reads
                .get(site)
                .and_then(|id| self.bindings.get(id))
                .copied()
                .or_else(|| self.file.argument_reference(*site)?.value_type)?,
            Expr::TypeTest { operand, .. } => {
                self.value(operand, depth + 1)?;
                arena.intern(Type::Intrinsic(Intrinsic::Boolean))
            }
            Expr::Not(operand) => {
                self.value(operand, depth + 1)?;
                arena.intern(Type::Intrinsic(Intrinsic::Boolean))
            }
            Expr::Call {
                receiver,
                selector,
                arguments,
            } => {
                let receiver = self.value(receiver, depth + 1)?;
                let name = self.file.source_member_name(*selector)?.ok()?;
                let actual = arguments
                    .iter()
                    .map(|a| self.value(a, depth + 1))
                    .collect::<Option<Vec<_>>>()?;
                if let Some(call) = super::overload_calls::contextual(
                    self.lookup,
                    receiver,
                    name,
                    &actual,
                    &[],
                    &|_| false,
                    &|_, _| None,
                    true,
                ) {
                    call.ok()?.return_type
                } else {
                    let selected = super::select_method(self.lookup, arena, receiver, name).ok()?;
                    let (source, signature) = self
                        .lookup
                        .view
                        .callable_members
                        .get(&selected.declaration)
                        .copied()
                        .flatten()?;
                    let mut signature = self
                        .lookup
                        .view
                        .sources
                        .get(&source)?
                        .signatures
                        .get(&signature)?
                        .clone();
                    let owner = crate::indexer::resolve::engine::head_decl::head_decl_id(
                        arena,
                        selected.receiver,
                    )?;
                    let bindings = crate::indexer::resolve::engine::bound_call::receiver_bindings(
                        self.lookup,
                        arena,
                        selected.receiver,
                        Some(owner),
                    );
                    let rewrite = |ty| substitute(arena, ty, &bindings);
                    signature.parameters = signature.parameters.into_iter().map(rewrite).collect();
                    signature.result = signature.result.map(rewrite);
                    signature.constraints = signature
                        .constraints
                        .into_iter()
                        .map(|c| c.map(rewrite))
                        .collect();
                    signature.defaults = signature
                        .defaults
                        .into_iter()
                        .map(|c| c.map(rewrite))
                        .collect();
                    super::call_arguments::applicable(&self.relation, &signature, &actual, &[])??
                        .result
                }
            }
            Expr::Member { receiver, selector } => {
                let receiver = self.value(receiver, depth + 1)?;
                let receiver = super::project_intrinsic(self.lookup, arena, receiver)?;
                let owner =
                    crate::indexer::resolve::engine::head_decl::head_decl_id(arena, receiver)?;
                let name = self.file.source_member_name(*selector)?.ok()?;
                let surface = self.lookup.nominal_surface(owner)?;
                if surface.incomplete {
                    return None;
                }
                let mut members = surface
                    .members
                    .iter()
                    .filter(|m| m.origin.name == Some(name));
                let member = members.next()?;
                if members.next().is_some()
                    || member.kind
                        != crate::indexer::lexical::globals::member_surface::Kind::Property
                {
                    return None;
                }
                let bindings = crate::indexer::resolve::engine::bound_call::receiver_bindings(
                    self.lookup,
                    arena,
                    receiver,
                    Some(owner),
                );
                let value = substitute(arena, member.property.value, &bindings);
                if member.property.optional {
                    arena.intern(Type::Optional(value))
                } else {
                    value
                }
            }
        };
        self.relation.canonical(ty, 0)
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

fn narrow(
    arena: &TypeArena,
    original: TypeId,
    kind: Intrinsic,
    negated: bool,
    depth: usize,
) -> Option<TypeId> {
    if depth >= 64 {
        return None;
    }
    if let Type::Union(parts) = arena.get(original) {
        let mut kept = Vec::new();
        for part in parts {
            let ty = narrow(arena, part, kind, negated, depth + 1)?;
            if !matches!(arena.get(ty), Type::Intrinsic(Intrinsic::Never)) {
                kept.push(ty);
            }
        }
        return Some(match kept.as_slice() {
            [] => arena.intern(Type::Intrinsic(Intrinsic::Never)),
            [one] => *one,
            _ => arena.intern(Type::Union(kept)),
        });
    }
    let Type::Intrinsic(actual) = arena.get(widen(arena, original)) else {
        return None;
    };
    if !matches!(
        actual,
        Intrinsic::String
            | Intrinsic::Number
            | Intrinsic::Boolean
            | Intrinsic::BigInt
            | Intrinsic::Symbol
            | Intrinsic::Undefined
            | Intrinsic::Null
            | Intrinsic::Never
    ) {
        return None;
    }
    Some(if (actual == kind) != negated {
        original
    } else {
        arena.intern(Type::Intrinsic(Intrinsic::Never))
    })
}

#[cfg(test)]
#[path = "program_callback_bodies_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "program_callback_negation_tests.rs"]
mod negation_tests;
