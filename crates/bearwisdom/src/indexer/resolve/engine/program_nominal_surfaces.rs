//! Source materialization boundary: names enter here, queries receive only IDs.
use super::*;
use crate::indexer::lexical::globals::{
    member_surface::{Key, Kind, Modifier},
    InheritedMembers,
};
use crate::indexer::resolve::engine::member_index::MemberNameId;
use crate::type_checker::core::types::{Intrinsic, LitValue, Type, TypeId, TypeProperty};

#[derive(Clone, Debug)]
pub(in crate::indexer::resolve::engine) struct Origin {
    pub owner: Option<i64>,
    pub source: SourceInstanceId,
    pub signature: source_signatures::SignatureId,
    pub declaration: Option<i64>,
    pub name: Option<MemberNameId>,
}

#[derive(Clone)]
pub(in crate::indexer::resolve::engine) struct Member {
    pub property: TypeProperty<TypeId>,
    pub kind: Kind,
    pub origin: Origin,
    pub signature: source_signatures::Bound,
}

#[derive(Default)]
pub(in crate::indexer::resolve::engine) struct Surface {
    pub order: InheritedMembers,
    pub bases: Vec<TypeId>,
    pub members: Vec<Member>,
    pub constructors: Vec<Constructor>,
    pub incomplete: bool,
    pub signature_facets: bool,
}

#[derive(Clone)]
pub(in crate::indexer::resolve::engine) struct Constructor {
    pub origin: Origin,
    pub signature: source_signatures::Bound,
}

pub(super) fn bind(
    view: &mut View,
    sources: &[(String, SourceInstanceId)],
    modules: &ModuleGraph,
    tree: &Compilation,
    arena: &TypeArena,
) {
    let mut surfaces: FxHashMap<i64, Surface> = FxHashMap::default();
    let mut sources: Vec<_> = sources.iter().collect();
    sources.sort_by_key(|(_, id)| id.ordinal());
    for (path, source) in sources {
        let Some(input) = modules
            .inputs
            .get(path)
            .and_then(|m| m.globals.as_ref())
            .and_then(|g| g.types.as_ref())
        else {
            continue;
        };
        // This is the source-name ingestion boundary, not semantic recovery.
        let names: FxHashMap<_, _> = input
            .names
            .iter()
            .filter(|(_, name)| !name.contains('\\'))
            .map(|(id, name)| {
                (
                    *id,
                    (
                        view.members.intern_name(name),
                        arena.intern(Type::Literal(LitValue::Str(name.clone()))),
                    ),
                )
            })
            .collect();
        view.key_names
            .extend(names.values().map(|&(name, key)| (key, name)));
        view.name_keys.extend(names.values().copied());
        let lookup = Lookup {
            tree,
            view,
            source: view.sources.get(source),
        };
        for part in &input.interfaces {
            let Some(&owner) = view.canonical.get(&part.owner) else {
                continue;
            };
            let surface = surfaces.entry(owner).or_insert_with(|| Surface {
                order: part.inherited_members,
                ..Default::default()
            });
            surface.incomplete |= surface.order != part.inherited_members
                || part.surface.is_none()
                || part.bases.is_none();
            for base in part.bases.as_deref().unwrap_or(&[]) {
                let base = base.materialize(&lookup, arena, path);
                if !surface.bases.contains(&base) {
                    surface.bases.push(base);
                }
            }
            for member in part.surface.as_deref().unwrap_or(&[]) {
                if member.kind == Kind::Construct {
                    if member.key != Key::Construct || !member.modifiers.is_empty() {
                        surface.incomplete = true;
                        continue;
                    }
                    if let Some(signature) =
                        lookup.signature(source_signatures::SignatureId(member.span))
                    {
                        surface.constructors.push(Constructor {
                            signature: signature.clone(),
                            origin: Origin {
                                owner: Some(owner),
                                source: *source,
                                signature: source_signatures::SignatureId(member.span),
                                declaration: member.slot,
                                name: None,
                            },
                        });
                    } else {
                        surface.incomplete = true;
                    }
                    continue;
                }
                if member.kind == Kind::Call {
                    surface.signature_facets = true;
                    // Signature facets are not keyof keys; never invent names.
                    continue;
                }
                let bound = (|| {
                    if member
                        .modifiers
                        .iter()
                        .any(|m| !matches!(m, Modifier::Readonly | Modifier::Optional))
                    {
                        return None;
                    }
                    let signature = lookup
                        .signature(source_signatures::SignatureId(member.span))?
                        .clone();
                    let (name, key, index) = match member.key {
                        Key::Named(id) if matches!(member.kind, Kind::Property | Kind::Method) => {
                            let &(name, key) = names.get(&id)?;
                            (Some(name), key, false)
                        }
                        Key::Computed { .. }
                            if matches!(member.kind, Kind::Property | Kind::Method) =>
                        {
                            let key = lookup.computed_key(member.key_span?).flatten()?;
                            if !matches!(arena.get(key), Type::UniqueSymbol(_)) {
                                return None;
                            }
                            (None, key, false)
                        }
                        Key::Index if member.kind == Kind::Index => {
                            let [key] = signature.parameters.as_slice() else {
                                return None;
                            };
                            if !matches!(
                                arena.get(*key),
                                Type::Intrinsic(
                                    Intrinsic::String | Intrinsic::Number | Intrinsic::Symbol
                                )
                            ) {
                                return None;
                            }
                            (None, *key, true)
                        }
                        _ => return None,
                    };
                    let optional = member.modifiers.contains(&Modifier::Optional);
                    let readonly = member.modifiers.contains(&Modifier::Readonly);
                    if index && optional {
                        return None;
                    }
                    let value = signature
                        .result
                        .unwrap_or_else(|| arena.intern(Type::Unknown));
                    Some(Member {
                        property: TypeProperty {
                            key,
                            value,
                            index,
                            optional,
                            readonly,
                        },
                        kind: member.kind,
                        origin: Origin {
                            owner: Some(owner),
                            source: *source,
                            signature: source_signatures::SignatureId(member.span),
                            declaration: member.slot,
                            name,
                        },
                        signature,
                    })
                })();
                match bound {
                    Some(member) => surface.members.push(member),
                    None => surface.incomplete = true,
                }
            }
        }
    }
    view.nominal_surfaces = surfaces;
}

#[cfg(test)]
#[path = "program_nominal_surfaces_tests.rs"]
mod tests;
