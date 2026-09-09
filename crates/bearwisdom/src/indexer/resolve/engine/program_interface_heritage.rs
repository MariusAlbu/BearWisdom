//! Interface inheritance is private until substituted source surfaces agree.
use super::*;
use crate::indexer::lexical::globals::InheritedMembers;
use crate::indexer::resolve::engine::{
    contract::generic_return::substitute, head_decl::head_decl_id,
};
use crate::type_checker::core::types::GenericParamId;

type Bindings = FxHashMap<GenericParamId, TypeId>;

#[path = "program_mixed_heritage.rs"]
mod mixed;

pub(in crate::indexer::resolve::engine) struct EffectiveMember {
    pub info: TypeInfo,
    pub signature: source_signatures::Bound,
    pub origin: super::super::nominal::Origin,
    pub optional: bool,
    pub readonly: bool,
}

#[derive(Default)]
struct Shape {
    order: InheritedMembers,
    bases: Vec<TypeId>,
    facts: Vec<Fact>,
    incomplete: bool,
    heritage: bool,
    generics: Option<source_signatures::Bound>,
}

pub(super) fn apply(
    view: &mut View,
    sources: &[(String, SourceInstanceId)],
    modules: &ModuleGraph,
    tree: &Compilation,
    arena: &TypeArena,
) -> FxHashSet<i64> {
    let mut shapes: FxHashMap<i64, Shape> = FxHashMap::default();
    let mut sources: Vec<_> = sources.iter().collect();
    sources.sort_by_key(|(_, source)| source.ordinal());
    for (path, source) in sources {
        let Some(input) = modules
            .inputs
            .get(path)
            .and_then(|m| m.globals.as_ref())
            .and_then(|g| g.types.as_ref())
        else {
            continue;
        };
        let lookup = Lookup {
            tree,
            view,
            source: view.sources.get(source),
        };
        let relation = types::Relation {
            lookup: &lookup,
            arena,
        };
        let names = input
            .names
            .iter()
            .filter_map(|(name, spelling)| view.members.name(spelling).map(|id| (*name, id)))
            .collect();
        for part in &input.interfaces {
            let Some(&owner) = view.canonical.get(&part.owner) else {
                continue;
            };
            let shape = shapes.entry(owner).or_insert_with(|| Shape {
                order: part.inherited_members,
                ..Default::default()
            });
            shape.incomplete |= shape.order != part.inherited_members;
            if let Some(group) = view.generic_declarations.get(&owner) {
                shape.generics = Some(group.bound.clone());
            } else if !part.plain_parameters {
                shape.incomplete = true;
            }
            let bases = part.bases.as_ref();
            shape.heritage |= bases.is_none_or(|bases| !bases.is_empty());
            shape.incomplete |= bases.is_none() || part.surface.is_none();
            for base in bases.into_iter().flatten() {
                let ty = relation.canonical(base.materialize(&lookup, arena, path), 0);
                match ty {
                    Some(ty) => {
                        if !shape.bases.contains(&ty) {
                            shape.bases.push(ty);
                        }
                    }
                    _ => shape.incomplete = true,
                }
            }
            for member in part.surface.as_deref().unwrap_or(&[]) {
                match fact(member, &names, &lookup, arena) {
                    Some(mut fact) => {
                        fact.source = Some(*source);
                        fact.owner = Some(owner);
                        shape.facts.push(fact);
                    }
                    None => shape.incomplete = true,
                }
            }
        }
    }
    let lookup = Lookup {
        tree,
        view,
        source: None,
    };
    let mut proof = Proof {
        shapes: &shapes,
        relation: types::Relation {
            lookup: &lookup,
            arena,
        },
        memo: Default::default(),
        active: Default::default(),
        heights: vec![],
        applications: Default::default(),
        exhausted: false,
        remaining: 0,
    };
    let mut rejected = FxHashSet::default();
    let mut edges = FxHashMap::default();
    let mut projections = Vec::new();
    let mut effective_members = FxHashMap::default();
    let mut effective_surfaces: FxHashMap<i64, Vec<EffectiveMember>> = FxHashMap::default();
    let mut nominal_surfaces = FxHashMap::default();
    for (&owner, shape) in &shapes {
        if !shape.heritage {
            continue;
        }
        proof.remaining = 4096;
        proof.exhausted = false;
        if let Some(facts) = proof.effective(owner) {
            edges.insert(
                owner,
                proof.applications.get(&owner).cloned().unwrap_or_default(),
            );
            let mut members: FxHashMap<MemberNameId, Option<Vec<i64>>> = FxHashMap::default();
            let mut nominal = super::super::nominal::Surface {
                order: shape.order,
                ..Default::default()
            };
            for fact in facts {
                {
                    let surface = effective_surfaces.entry(owner).or_default();
                    let mut info = fact
                        .member
                        .slot
                        .and_then(|row| view.info.get(&lookup.canonical_decl_id(row)))
                        .cloned()
                        .unwrap_or_default();
                    if matches!(fact.member.kind, Kind::Property | Kind::Index) {
                        info.field_type_id = fact.ty;
                    } else {
                        info.parameter_type_ids = Some(fact.signature.parameters.clone());
                        info.return_type_id = fact.signature.result;
                        info.generic_param_ids = fact.signature.generic_parameters.clone();
                        info.generic_param_default_ids = fact.signature.defaults.clone();
                    }
                    if let Some(row) = fact.member.slot {
                        effective_members.insert((owner, row), surface.len());
                    }
                    let origin = super::super::nominal::Origin {
                        owner: fact.owner,
                        source: fact.source.expect("source-owned heritage fact"),
                        signature: source_signatures::SignatureId(fact.member.span),
                        declaration: fact.member.slot,
                        name: match fact.key {
                            MemberKey::Named(name) => Some(name),
                            _ => None,
                        },
                    };
                    match fact.key {
                        MemberKey::Call => {
                            nominal.signature_facets = true;
                        }
                        MemberKey::Construct => {
                            nominal
                                .constructors
                                .push(super::super::nominal::Constructor {
                                    origin: origin.clone(),
                                    signature: fact.signature.clone(),
                                })
                        }
                        key => {
                            let key = match key {
                                MemberKey::Named(name) => view.name_keys.get(&name).copied(),
                                MemberKey::Unique(key) => Some(key),
                                MemberKey::Index(domain) => {
                                    Some(arena.intern(Type::Intrinsic(domain)))
                                }
                                _ => None,
                            };
                            if let Some(key) = key {
                                nominal.members.push(super::super::nominal::Member {
                                    property: crate::type_checker::core::types::TypeProperty {
                                        key,
                                        value: fact
                                            .signature
                                            .result
                                            .unwrap_or_else(|| arena.intern(Type::Unknown)),
                                        index: fact.member.kind == Kind::Index,
                                        optional: fact
                                            .member
                                            .modifiers
                                            .contains(&Modifier::Optional),
                                        readonly: fact
                                            .member
                                            .modifiers
                                            .contains(&Modifier::Readonly),
                                    },
                                    kind: fact.member.kind,
                                    origin: origin.clone(),
                                    signature: fact.signature.clone(),
                                });
                            } else {
                                nominal.incomplete = true;
                            }
                        }
                    }
                    surface.push(EffectiveMember {
                        info,
                        signature: fact.signature.clone(),
                        origin,
                        optional: fact.member.modifiers.contains(&Modifier::Optional),
                        readonly: fact.member.modifiers.contains(&Modifier::Readonly),
                    });
                }
                if let MemberKey::Named(name) = fact.key {
                    let rows = members.entry(name).or_insert_with(|| Some(vec![]));
                    match (rows.as_mut(), fact.member.slot) {
                        (Some(rows), Some(row)) => rows.push(row),
                        (_, None) => *rows = None,
                        _ => {}
                    }
                }
            }
            nominal_surfaces.insert(owner, nominal);
            projections.extend(
                members
                    .into_iter()
                    .map(|(name, rows)| (owner, name, rows.unwrap_or_default())),
            );
        } else {
            rejected.insert(owner);
        }
    }
    view.bases.install_interfaces(edges, &mut view.info, arena);
    // Ancestry is nominal, but the semantic base retains every structural branch.
    for (&owner, shape) in &shapes {
        if shape.heritage && !rejected.contains(&owner) {
            view.info.entry(owner).or_default().base_type_id = match shape.bases.as_slice() {
                [] => None,
                [ty] => Some(*ty),
                bases => Some(arena.intern(Type::Intersection(bases.to_vec()))),
            };
        }
    }
    view.effective_members = effective_members;
    view.effective_surfaces = effective_surfaces;
    view.nominal_surfaces.extend(nominal_surfaces);
    for (owner, name, rows) in projections {
        view.members.project_member(owner, name, rows);
    }
    rejected
}

struct Proof<'a> {
    shapes: &'a FxHashMap<i64, Shape>,
    relation: types::Relation<'a>,
    memo: FxHashMap<i64, (Option<Vec<Fact>>, usize)>,
    active: FxHashSet<i64>,
    heights: Vec<usize>,
    applications: FxHashMap<i64, Vec<TypeId>>,
    exhausted: bool,
    remaining: usize,
}

impl Proof<'_> {
    fn effective(&mut self, owner: i64) -> Option<Vec<Fact>> {
        if let Some((result, height)) = self.memo.get(&owner).cloned() {
            if self.active.len() + height > 64 {
                self.exhausted = true;
                return None;
            }
            self.note_height(height);
            return result;
        }
        if self.remaining == 0 || self.active.len() >= 64 {
            self.exhausted = true;
            return None;
        }
        if !self.active.insert(owner) {
            return None;
        }
        self.remaining -= 1;
        self.heights.push(1);
        let result = self.surface(owner);
        let height = self.heights.pop().unwrap();
        self.note_height(height);
        self.active.remove(&owner);
        if self.exhausted {
            return None;
        }
        self.memo.insert(owner, (result.clone(), height));
        result
    }

    fn note_height(&mut self, height: usize) {
        if let Some(parent) = self.heights.last_mut() {
            *parent = (*parent).max(height + 1);
        }
    }

    fn surface(&mut self, owner: i64) -> Option<Vec<Fact>> {
        let shape = self.shapes.get(&owner)?;
        if shape.incomplete {
            return None;
        }
        let mut inherited: Vec<Fact> = Vec::new();
        let mut applications = Vec::new();
        for &base in &shape.bases {
            let (base_facts, base_applications) = self.base_surface(base)?;
            applications.extend(base_applications);
            let prior_keys: FxHashSet<_> = inherited.iter().map(|f| f.key).collect();
            if !self.groups_compatible(&base_facts, &inherited, false) {
                return None;
            }
            for fact in base_facts {
                if fact.key == MemberKey::Construct
                    || shape.order != InheritedMembers::DeclarationOrder
                    || !prior_keys.contains(&fact.key)
                {
                    inherited.push(fact);
                }
            }
        }
        if !self.groups_compatible(&shape.facts, &inherited, true) {
            return None;
        }
        let mut effective = shape.facts.clone();
        effective.extend(inherited.into_iter().filter(|base| {
            base.key == MemberKey::Construct || !shape.facts.iter().any(|own| own.key == base.key)
        }));
        for fact in effective
            .iter()
            .filter(|fact| fact.key == MemberKey::Construct)
        {
            if shape.order != InheritedMembers::DeclarationOrder {
                return None;
            }
            let origin = crate::type_checker::core::types::CallableOrigin::new(
                self.relation.lookup.view.context,
                fact.source?.ordinal(),
                fact.member.span,
            );
            let callable = fact.signature.callable(origin, self.relation.arena)?;
            self.relation.valid_constructor(&callable)?;
        }
        for index in effective
            .iter()
            .filter(|f| matches!(f.key, MemberKey::Index(_)))
        {
            for value in &effective {
                let applies = matches!(
                    (index.key, value.key),
                    (
                        MemberKey::Index(Intrinsic::String),
                        MemberKey::Named(_) | MemberKey::Index(Intrinsic::Number)
                    ) | (MemberKey::Index(Intrinsic::Symbol), MemberKey::Unique(_))
                );
                if applies && !self.relation.assignable(value.ty?, index.ty?) {
                    return None;
                }
            }
        }
        self.applications.insert(owner, applications);
        Some(effective)
    }

    fn argument_satisfies(
        &self,
        argument: TypeId,
        bound: TypeId,
        seen: &mut FxHashSet<GenericParamId>,
    ) -> bool {
        if self.relation.assignable(argument, bound) {
            return true;
        }
        let Type::Generic { param } = self.relation.arena.get(argument) else {
            return false;
        };
        if seen.len() >= 64 || !seen.insert(param) {
            return false;
        }
        let constraint = self
            .shapes
            .values()
            .filter_map(|shape| shape.generics.as_ref())
            .find_map(|generics| {
                generics
                    .generic_parameters
                    .iter()
                    .position(|&p| p == param)
                    .and_then(|index| generics.constraints.get(index).copied().flatten())
            });
        constraint.is_some_and(|constraint| self.argument_satisfies(constraint, bound, seen))
    }

    fn substituted(&self, mut fact: Fact, bindings: &Bindings) -> Fact {
        let apply = |ty| substitute(self.relation.arena, ty, bindings);
        fact.ty = fact.ty.map(apply);
        fact.signature.parameters = fact.signature.parameters.into_iter().map(apply).collect();
        fact.signature.result = fact.signature.result.map(apply);
        fact.signature.constraints = fact
            .signature
            .constraints
            .into_iter()
            .map(|ty| ty.map(apply))
            .collect();
        fact.signature.defaults = fact
            .signature
            .defaults
            .into_iter()
            .map(|ty| ty.map(apply))
            .collect();
        fact
    }

    fn compatible(&self, own: &Fact, base: &Fact, override_: bool) -> bool {
        if own.member.kind != base.member.kind {
            return false;
        }
        let optional = |fact: &Fact| fact.member.modifiers.contains(&Modifier::Optional);
        if override_ {
            if optional(own) && !optional(base) {
                return false;
            }
        } else if [Modifier::Optional, Modifier::Readonly]
            .iter()
            .any(|m| own.member.modifiers.contains(m) != base.member.modifiers.contains(m))
        {
            return false;
        }
        let (Some(from), Some(to)) = (own.ty, base.ty) else {
            return false;
        };
        if own.member.kind == Kind::Property || own.member.kind == Kind::Index {
            return if override_ {
                self.relation.assignable(from, to)
            } else {
                self.relation.equal(from, to)
            };
        }
        if override_
            && own.member.kind == Kind::Method
            && self
                .relation
                .lookup
                .view
                .callable_policy
                .and_then(|p| p.bivariant_methods)
                .is_some()
        {
            let callable = |fact: &Fact| {
                let origin = crate::type_checker::core::types::CallableOrigin::new(
                    self.relation.lookup.view.context,
                    fact.source?.ordinal(),
                    fact.member.span,
                );
                fact.signature.callable(origin, self.relation.arena)
            };
            return callable(own)
                .zip(callable(base))
                .is_some_and(|(a, b)| self.relation.method(&a, &b) == Some(true));
        }
        let a = &own.signature;
        let b = &base.signature;
        if a.generic_parameters.len() != b.generic_parameters.len()
            || a.parameters.len() != b.parameters.len()
        {
            return false;
        }
        let flags = |signature: &source_signatures::Bound| {
            signature
                .syntax
                .parameters
                .iter()
                .map(|p| (p.optional, p.rest))
                .collect::<Vec<_>>()
        };
        if flags(a) != flags(b) {
            return false;
        }
        let bindings = b
            .generic_parameters
            .iter()
            .copied()
            .zip(
                a.generic_parameters
                    .iter()
                    .map(|&p| self.relation.arena.generic_type(p)),
            )
            .collect();
        let equal = |left: Option<TypeId>, right: Option<TypeId>| match (left, right) {
            (None, None) => true,
            (Some(a), Some(b)) => self
                .relation
                .equal(a, substitute(self.relation.arena, b, &bindings)),
            _ => false,
        };
        if a.constraints.len() != b.constraints.len()
            || !a
                .constraints
                .iter()
                .zip(&b.constraints)
                .all(|(&a, &b)| equal(a, b))
        {
            return false;
        }
        self.relation
            .equal(from, substitute(self.relation.arena, to, &bindings))
    }

    fn groups_compatible(&self, own: &[Fact], bases: &[Fact], override_: bool) -> bool {
        let mut groups: FxHashMap<MemberKey, Vec<&Fact>> = FxHashMap::default();
        for fact in own {
            groups.entry(fact.key).or_default().push(fact);
        }
        for base in bases {
            // Anonymous construct signatures accumulate as overloads. No own
            // constructor can replace or erase a base signature by key equality.
            if base.key == MemberKey::Construct {
                continue;
            }
            if let Some(group) = groups.get(&base.key) {
                if !group
                    .iter()
                    .any(|own| self.compatible(own, base, override_))
                {
                    return false;
                }
            }
        }
        if !override_ {
            for own in own {
                if own.key == MemberKey::Construct {
                    continue;
                }
                if bases.iter().any(|base| base.key == own.key)
                    && !bases
                        .iter()
                        .any(|base| base.key == own.key && self.compatible(own, base, false))
                {
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
#[path = "program_interface_heritage_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "program_method_heritage_tests.rs"]
mod method_tests;

#[cfg(test)]
#[path = "program_constructor_heritage_tests.rs"]
mod construct_tests;
