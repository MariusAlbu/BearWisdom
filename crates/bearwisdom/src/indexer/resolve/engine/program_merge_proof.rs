//! Candidate views are private until cross-declaration evidence stabilizes.
use super::super::{member_index::MemberNameId, program_graph::candidates::Pending};
use super::*;
use crate::indexer::lexical::globals::member_surface::{Key, Kind, Member, Modifier};
use crate::type_checker::core::types::{Intrinsic, Type, TypeId};
use rustc_hash::FxHashSet;

#[path = "program_merge_types.rs"]
pub(super) mod types;

#[path = "program_interface_heritage.rs"]
pub(super) mod heritage;

pub(super) fn build(
    program: ProgramId,
    sources: &[(String, SourceInstanceId)],
    modules: &ModuleGraph,
    tree: &Compilation,
    arena: &TypeArena,
) -> View {
    let mut allowed: FxHashSet<_> = modules
        .programs
        .pending(program)
        .iter()
        .map(|p| p.key)
        .collect();
    let mut rejected = FxHashSet::default();
    loop {
        let staged = modules.programs.staged(program, &allowed);
        let mut view = View::build(program, sources, modules, tree, arena, &staged, &rejected);
        let Some((invalid, invalid_heritage, properties)) = settle(
            &mut view, program, sources, modules, tree, arena, &allowed, 128,
        ) else {
            return View::empty();
        };
        if invalid.is_empty() && invalid_heritage.is_empty() {
            // Navigation keeps every physical part; semantic member selection
            // sees one property only after its complete equivalence proof.
            for Property { rows, ty, .. } in properties {
                let Some(&owner) = rows.first() else {
                    continue;
                };
                for row in rows {
                    view.canonical.insert(row, owner);
                }
                view.info.entry(owner).or_default().field_type_id = Some(ty);
            }
            return view;
        }
        // Monotone removal of candidate keys or canonical owners bounds rebuilds.
        // Rechecking closes dependencies through aliases, keys and global values.
        for key in invalid {
            allowed.remove(&key);
        }
        for (&row, owner) in &view.canonical {
            if invalid_heritage.contains(owner) {
                rejected.insert(row);
            }
        }
    }
}

type Settled = (Vec<i64>, FxHashSet<i64>, Vec<Property>);

fn settle(
    view: &mut View,
    program: ProgramId,
    sources: &[(String, SourceInstanceId)],
    modules: &ModuleGraph,
    tree: &Compilation,
    arena: &TypeArena,
    allowed: &FxHashSet<i64>,
    rounds: usize,
) -> Option<Settled> {
    let inputs = sources
        .iter()
        .map(|(path, source)| {
            Some((
                path,
                source,
                modules.inputs.get(path)?.globals.as_ref()?.types.as_ref()?,
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    for _ in 0..rounds {
        let mut invalid_heritage = super::generic_groups::rejected(view, tree, arena);
        invalid_heritage.extend(heritage::apply(view, sources, modules, tree, arena));
        let mut invalid = Vec::new();
        let mut properties = Vec::new();
        for candidate in modules
            .programs
            .pending(program)
            .iter()
            .filter(|p| allowed.contains(&p.key))
        {
            match compatible(candidate, sources, modules, view, tree, arena) {
                Some(proof) => properties.extend(proof),
                None => invalid.push(candidate.key),
            }
        }
        let (local, rejected_local, local_proofs) =
            local_properties(sources, modules, view, tree, arena)?;
        invalid_heritage.extend(rejected_local);
        let global_keys: FxHashSet<_> = properties.iter().map(|p| (p.owner, p.key)).collect();
        properties.extend(
            local_proofs
                .into_iter()
                .filter(|p| !global_keys.contains(&(p.owner, p.key))),
        );
        let evidence = query_evidence(view, &local, tree, arena);
        view.query_members = evidence;
        if !super::value_queries::bind(view, &inputs, tree, arena) {
            return Some((invalid, invalid_heritage, properties));
        }
        view.materialize(&inputs, tree, arena);
        super::nominal::bind(view, sources, modules, tree, arena);
    }
    // A resource limit is not a stable proof; never expose this partial view.
    None
}

fn query_evidence(
    view: &View,
    properties: &[Property],
    tree: &Compilation,
    arena: &TypeArena,
) -> FxHashMap<(i64, MemberNameId), Option<TypeId>> {
    let mut evidence = FxHashMap::default();
    let lookup = Lookup {
        tree,
        view,
        source: None,
    };
    let relation = types::Relation {
        lookup: &lookup,
        arena,
    };
    for (&owner, surface) in &view.effective_surfaces {
        for member in surface {
            let Some(name) = member.origin.name else {
                continue;
            };
            // Complete heritage proof preserves the receiver-owned read type.
            // Non-properties remain authoritative barriers, never method returns.
            let ty = member
                .info
                .field_type_id
                .and_then(|ty| relation.canonical(ty, 0));
            evidence
                .entry((owner, name))
                .and_modify(|prior| {
                    if *prior != ty {
                        *prior = None;
                    }
                })
                .or_insert(ty);
        }
    }
    for property in properties {
        if let MemberKey::Named(name) = property.key {
            // Heritage evidence may refine the declaration; it takes precedence.
            evidence
                .entry((property.owner, name))
                .or_insert(Some(property.ty));
        }
    }
    evidence
}

struct Property {
    owner: i64,
    key: MemberKey,
    rows: Vec<i64>,
    ty: TypeId,
}

fn property(owner: i64, group: &[&Fact], relation: &types::Relation) -> Option<Property> {
    let first = group.first()?;
    if group.iter().any(|other| {
        other.member.kind != Kind::Property
            || other.member.modifiers != first.member.modifiers
            || !matches!((first.ty, other.ty), (Some(a), Some(b)) if relation.equal(a, b))
    }) {
        return None;
    }
    Some(Property {
        owner,
        key: first.key,
        rows: group.iter().filter_map(|fact| fact.member.slot).collect(),
        ty: relation.canonical(first.ty?, 0)?,
    })
}

#[derive(Default)]
struct InterfaceFacts {
    parts: usize,
    missing_surface: bool,
    incomplete: bool,
    named: FxHashMap<MemberNameId, Vec<Option<Fact>>>,
    facts: Vec<Fact>,
}

fn local_properties(
    sources: &[(String, SourceInstanceId)],
    modules: &ModuleGraph,
    view: &View,
    tree: &Compilation,
    arena: &TypeArena,
) -> Option<(Vec<Property>, FxHashSet<i64>, Vec<Property>)> {
    let mut owners: FxHashMap<i64, InterfaceFacts> = FxHashMap::default();
    let mut sources: Vec<_> = sources.iter().collect();
    sources.sort_by_key(|(_, source)| source.ordinal());
    for (path, source) in sources {
        let input = modules.inputs.get(path)?.globals.as_ref()?.types.as_ref()?;
        let names: FxHashMap<_, _> = input
            .names
            .iter()
            .filter_map(|(name, spelling)| view.members.name(spelling).map(|id| (*name, id)))
            .collect();
        let lookup = Lookup {
            tree,
            view,
            source: view.sources.get(source),
        };
        for part in &input.interfaces {
            let Some(&owner) = view.canonical.get(&part.owner) else {
                continue;
            };
            let shape = owners.entry(owner).or_default();
            shape.parts += 1;
            shape.missing_surface |= part.surface.is_none();
            shape.incomplete |=
                !part.plain_parameters && !view.generic_declarations.contains_key(&owner);
            for member in part.surface.as_deref().unwrap_or(&[]) {
                let fact = fact(member, &names, &lookup, arena);
                if let Key::Named(name) = member.key {
                    shape
                        .named
                        .entry(*names.get(&name)?)
                        .or_default()
                        .push(fact.clone());
                }
                match fact {
                    Some(fact) => shape.facts.push(fact),
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
    let relation = types::Relation {
        lookup: &lookup,
        arena,
    };
    // A complete named-property group can break a productive query cycle.
    // Unknown computed keys are a separate identity domain; whole-owner checks
    // still run before publication and discard all evidence after rejection.
    let mut local = Vec::new();
    let mut rejected = FxHashSet::default();
    let mut complete = Vec::new();
    for (owner, shape) in owners.into_iter().filter(|(_, shape)| shape.parts > 1) {
        if !shape.missing_surface {
            local.extend(
                shape
                    .named
                    .values()
                    .filter(|group| group.len() > 1)
                    .filter_map(|group| {
                        property(
                            owner,
                            &group
                                .iter()
                                .map(Option::as_ref)
                                .collect::<Option<Vec<_>>>()?,
                            &relation,
                        )
                    }),
            );
        }
        match (!shape.missing_surface && !shape.incomplete)
            .then(|| compatible_facts(owner, &shape.facts, &relation))
            .flatten()
        {
            Some(proofs) => complete.extend(proofs),
            None => {
                rejected.insert(owner);
            }
        }
    }
    Some((local, rejected, complete))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum MemberKey {
    Named(MemberNameId),
    Unique(TypeId),
    Call,
    Construct,
    Index(Intrinsic),
}
#[derive(Clone)]
struct Fact {
    owner: Option<i64>,
    key: MemberKey,
    member: Member<i64, usize>,
    ty: Option<TypeId>,
    signature: source_signatures::Bound,
    source: Option<SourceInstanceId>,
}

fn compatible(
    candidate: &Pending,
    sources: &[(String, SourceInstanceId)],
    modules: &ModuleGraph,
    view: &View,
    tree: &Compilation,
    arena: &TypeArena,
) -> Option<Vec<Property>> {
    let owner = *view
        .canonical
        .get(&candidate.parts.first()?.1.declaration?)?;
    let mut facts = Vec::new();
    for (source, part) in &candidate.parts {
        let (path, _) = sources.iter().find(|(_, id)| id == source)?;
        let input = modules.inputs.get(path)?.globals.as_ref()?.types.as_ref()?;
        let lookup = Lookup {
            tree,
            view,
            source: view.sources.get(source),
        };
        let names: FxHashMap<_, _> = input
            .names
            .iter()
            .filter_map(|(name, spelling)| view.members.name(spelling).map(|id| (*name, id)))
            .collect();
        for member in part.surface.as_deref().unwrap_or(&[]) {
            facts.push(fact(member, &names, &lookup, arena)?);
        }
    }
    let lookup = Lookup {
        tree,
        view,
        source: None,
    };
    let relation = types::Relation {
        lookup: &lookup,
        arena,
    };
    compatible_facts(owner, &facts, &relation)
}

fn compatible_facts(
    owner: i64,
    facts: &[Fact],
    relation: &types::Relation,
) -> Option<Vec<Property>> {
    let mut groups: FxHashMap<MemberKey, Vec<&Fact>> = FxHashMap::default();
    let mut properties = Vec::new();
    for fact in facts {
        groups.entry(fact.key).or_default().push(fact);
    }
    for group in groups.values().filter(|group| group.len() > 1) {
        let first = group[0];
        if matches!(first.key, MemberKey::Index(_)) {
            return None;
        }
        for other in &group[1..] {
            if other.member.kind != first.member.kind
                || other.member.modifiers != first.member.modifiers
            {
                return None;
            }
            if first.member.kind == Kind::Property
                && !matches!((first.ty, other.ty), (Some(a), Some(b)) if relation.equal(a, b))
            {
                return None;
            }
        }
        if first.member.kind == Kind::Property {
            properties.push(property(owner, group, relation)?);
        }
    }
    for index in facts
        .iter()
        .filter(|f| matches!(f.key, MemberKey::Index(_)))
    {
        let to = index.ty?;
        for value in facts {
            let applies = matches!(
                (index.key, value.key),
                (
                    MemberKey::Index(Intrinsic::String),
                    MemberKey::Named(_) | MemberKey::Index(Intrinsic::Number)
                ) | (MemberKey::Index(Intrinsic::Symbol), MemberKey::Unique(_))
            );
            if applies && !value.ty.is_some_and(|from| relation.assignable(from, to)) {
                return None;
            }
        }
    }
    Some(properties)
}

fn fact(
    member: &Member<i64, usize>,
    names: &FxHashMap<crate::indexer::lexical::NameId, MemberNameId>,
    lookup: &Lookup,
    arena: &TypeArena,
) -> Option<Fact> {
    if member
        .modifiers
        .iter()
        .any(|m| !matches!(m, Modifier::Readonly | Modifier::Optional))
    {
        return None;
    }
    if member.kind != Kind::Property
        && member.kind != Kind::Index
        && member.modifiers.contains(&Modifier::Readonly)
    {
        return None;
    }
    let signature = lookup.signature(source_signatures::SignatureId(member.span))?;
    let key = match member.key {
        Key::Named(name) if matches!(member.kind, Kind::Property | Kind::Method) => {
            MemberKey::Named(*names.get(&name)?)
        }
        Key::Computed { .. } if matches!(member.kind, Kind::Property | Kind::Method) => {
            let ty = lookup.computed_key(member.key_span?).flatten()?;
            if !matches!(arena.get(ty), Type::UniqueSymbol(_)) {
                return None;
            }
            MemberKey::Unique(ty)
        }
        Key::Call if member.kind == Kind::Call => MemberKey::Call,
        Key::Construct if member.kind == Kind::Construct => MemberKey::Construct,
        Key::Index if member.kind == Kind::Index => {
            let [parameter] = signature.parameters.as_slice() else {
                return None;
            };
            let relation = types::Relation { lookup, arena };
            let Type::Intrinsic(
                domain @ (Intrinsic::String | Intrinsic::Number | Intrinsic::Symbol),
            ) = arena.get(relation.canonical(*parameter, 0)?)
            else {
                return None;
            };
            if member.modifiers.contains(&Modifier::Optional)
                || !signature.generic_parameters.is_empty()
            {
                return None;
            }
            MemberKey::Index(domain)
        }
        _ => return None,
    };
    let mut ty = signature.result;
    if matches!(member.kind, Kind::Method | Kind::Call | Kind::Construct) {
        ty = ty.map(|return_| {
            arena.intern(Type::Function {
                params: signature.parameters.clone(),
                return_,
            })
        });
    }
    if member.modifiers.contains(&Modifier::Optional) {
        ty = ty.map(|ty| arena.intern(Type::Optional(ty)));
    }
    Some(Fact {
        owner: None,
        key,
        member: member.clone(),
        ty,
        signature: signature.clone(),
        source: None,
    })
}

#[cfg(test)]
#[path = "program_merge_proof_tests.rs"]
pub(super) mod tests;
