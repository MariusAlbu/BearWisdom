//! Source object members keep navigation origins apart from structural values.
use super::*;
use crate::indexer::lexical::globals::member_surface::Kind;
use crate::indexer::resolve::engine::{
    contract::flow_cache::{CallSignatureOrigin, ObjectMember, OverloadCall},
    member_index::MemberNameId,
};
use crate::type_checker::core::types::{
    Callable, LitValue, ObjectOrigin, SourceObject, Type, TypeId,
};
use crate::types::SourceSpan;

pub(super) struct Member {
    pub name: MemberNameId,
    pub key: TypeId,
    pub span: SourceSpan,
    pub declaration: Option<i64>,
    pub kind: Kind,
}
#[derive(Default)]
pub(super) struct Index(FxHashMap<ObjectOrigin, Option<Vec<Member>>>);
impl Index {
    pub(super) fn capture(view: &View, inputs: &computed_keys::Sources, arena: &TypeArena) -> Self {
        let mut index = Self::default();
        for (_, &source, input) in inputs {
            let names: FxHashMap<_, _> = input.names.iter().map(|(id, name)| (*id, name)).collect();
            for object in &input.objects {
                let members = object.members.as_ref().and_then(|members| {
                    members
                        .iter()
                        .map(|member| {
                            Some(Member {
                                name: *view.sources.get(&source)?.member_names.get(&member.name)?,
                                key: arena.intern(Type::Literal(LitValue::Str(
                                    (*names.get(&member.name)?).clone(),
                                ))),
                                span: member.span,
                                declaration: member.declaration,
                                kind: member.kind,
                            })
                        })
                        .collect()
                });
                index
                    .0
                    .entry(ObjectOrigin::new(
                        view.context,
                        source.ordinal(),
                        object.span,
                    ))
                    .and_modify(|old| *old = None)
                    .or_insert(members);
            }
        }
        index
    }
    pub(super) fn members(&self, origin: ObjectOrigin) -> Option<&[Member]> {
        self.0.get(&origin)?.as_deref()
    }
}

fn member<'a>(
    lookup: &'a Lookup,
    object: &'a SourceObject<TypeId>,
    name: MemberNameId,
) -> Option<(&'a Member, TypeId)> {
    let mut members = lookup
        .view
        .objects
        .members(object.origin)?
        .iter()
        .filter(|member| member.name == name);
    let member = members.next()?;
    if members.next().is_some() {
        return None;
    }
    let mut values = object
        .properties
        .iter()
        .filter(|property| property.key == member.key);
    let property = values.next()?;
    if values.next().is_some() || property.optional || property.index {
        return None;
    }
    Some((member, property.value))
}

pub(super) fn read(
    lookup: &Lookup,
    receiver: TypeId,
    name: MemberNameId,
) -> Option<Result<ObjectMember, ()>> {
    let arena = lookup.type_arena()?;
    let Type::Object(object) = arena.get(receiver) else {
        return None;
    };
    Some((|| {
        if !(lookup as &dyn SymbolLookup).accepts_type_context(arena, receiver) {
            return Err(());
        }
        let (member, value) = member(lookup, &object, name).ok_or(())?;
        Ok(ObjectMember {
            declaration: member.declaration,
            value,
        })
    })())
}

pub(super) fn value(lookup: &Lookup, receiver: TypeId, declaration: i64) -> Option<Option<TypeId>> {
    let arena = lookup.type_arena()?;
    let Type::Object(object) = arena.get(receiver) else {
        return None;
    };
    Some((|| {
        if !(lookup as &dyn SymbolLookup).accepts_type_context(arena, receiver) {
            return None;
        }
        let mut members = lookup
            .view
            .objects
            .members(object.origin)?
            .iter()
            .filter(|member| member.declaration == Some(declaration));
        let selected = members.next()?;
        if members.next().is_some() {
            return None;
        }
        member(lookup, &object, selected.name).map(|(_, value)| value)
    })())
}

fn signature(lookup: &Lookup, callable: &Callable<TypeId>) -> Option<source_signatures::Bound> {
    if !callable.complete
        || !(lookup as &dyn SymbolLookup)
            .accepts_type_context(lookup.type_arena()?, callable.result)
    {
        return None;
    }
    let source = lookup.view.sources.values().find(|source| {
        source
            .identity
            .is_some_and(|id| id.ordinal() == callable.origin.source())
    })?;
    let mut bound = source
        .signatures
        .get(&source_signatures::SignatureId(callable.origin.signature))?
        .clone();
    if bound.parameters.len() != callable.parameters.len()
        || bound.generic_parameters.len() != callable.generics.len()
    {
        return None;
    }
    bound.parameters = callable.parameters.iter().map(|p| p.ty).collect();
    bound.result = Some(callable.result);
    bound.generic_parameters = callable
        .generics
        .iter()
        .map(|g| match lookup.type_arena()?.get(g.parameter) {
            Type::Generic { param } => Some(param),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    bound.constraints = callable.generics.iter().map(|g| g.constraint).collect();
    bound.defaults = callable.generics.iter().map(|g| g.default).collect();
    Some(bound)
}

pub(super) fn comparison(
    lookup: &Lookup,
    object: &SourceObject<TypeId>,
) -> Option<Vec<super::nominal::Member>> {
    let source = lookup.view.sources.values().find_map(|s| {
        s.identity
            .filter(|id| id.ordinal() == object.origin.source())
    })?;
    let layout = lookup.view.objects.members(object.origin)?;
    if object.properties.len() != layout.len() {
        return None;
    }
    layout
        .iter()
        .map(|entry| {
            let (_, value) = member(lookup, object, entry.name)?;
            let bound = if entry.kind == Kind::Method {
                let Type::Callable(callable) = lookup.type_arena()?.get(value) else {
                    return None;
                };
                signature(lookup, &callable)?
            } else {
                source_signatures::Bound::default()
            };
            Some(super::nominal::Member {
                property: object
                    .properties
                    .iter()
                    .find(|p| p.key == entry.key)?
                    .clone(),
                kind: entry.kind,
                signature: bound,
                origin: super::nominal::Origin {
                    owner: None,
                    source,
                    signature: source_signatures::SignatureId(entry.span),
                    declaration: entry.declaration,
                    name: Some(entry.name),
                },
            })
        })
        .collect()
}

pub(super) fn call(
    lookup: &Lookup,
    receiver: TypeId,
    name: MemberNameId,
    actual: &[TypeId],
    explicit: &[TypeId],
    deferred: &dyn Fn(usize) -> bool,
    callback: &dyn Fn(usize, TypeId) -> Option<TypeId>,
) -> Option<Result<OverloadCall, ()>> {
    let arena = lookup.type_arena()?;
    let Type::Object(object) = arena.get(receiver) else {
        return None;
    };
    Some((|| {
        if !(lookup as &dyn SymbolLookup).accepts_type_context(arena, receiver) {
            return Err(());
        }
        let (member, value) = member(lookup, &object, name).ok_or(())?;
        if !matches!(member.kind, Kind::Method | Kind::Property) {
            return Err(());
        }
        let Type::Callable(callable) = arena.get(value) else {
            return Err(());
        };
        let signature = signature(lookup, &callable).ok_or(())?;
        let selected = super::call_selection::select(
            &super::merge_proof::types::Relation { lookup, arena },
            &[&signature],
            None,
            actual,
            explicit,
            deferred,
            callback,
            false,
        )
        .ok_or(())?;
        let source = lookup
            .view
            .sources
            .values()
            .find_map(|source| {
                source
                    .identity
                    .filter(|id| id.ordinal() == callable.origin.source())
            })
            .ok_or(())?;
        Ok(OverloadCall {
            origins: vec![CallSignatureOrigin {
                source,
                span: callable.origin.signature,
                declaration: member.declaration,
            }],
            selected: 0,
            return_type: selected.applied.result,
            parameters: selected.applied.parameters,
        })
    })())
}
