//! Source-bound computed keys. Strings are consumed only when interning the
//! source name arenas; value/member traversal never consults display metadata.
use super::{Lookup, SourceInstanceId, View};
use crate::indexer::lexical::{
    globals::member_surface::{Key, Root, ValueUse},
    type_syntax::signatures::MemberValue,
    BindingId, NameId,
};
use crate::indexer::namespaces::ExportDomain;
use crate::indexer::resolve::engine::{
    contract::SymbolLookup,
    lexical_type_ids::TypeBinder,
    member_index::MemberNameId,
    module_graph::ExportNameId,
    module_input::SourceModuleId,
    program_types::{self, source_signatures::SignatureId, Recipe},
};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::{ParsedFile, SourceSpan, SymbolKind};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Value {
    Unknown,
    Declaration(i64),
    Import {
        binding: usize,
        unit: SourceModuleId,
    },
    Global(NameId),
    Annotated {
        declaration: Option<i64>,
        recipe: Recipe,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Computed {
    pub site: SourceSpan,
    root: Value,
    selectors: Vec<NameId>,
    #[serde(default)]
    usage: ValueUse,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(in crate::indexer::resolve::engine) struct Input {
    classes: Vec<i64>,
    members: Vec<MemberValue<i64>>,
    pub keys: Vec<Computed>,
    #[serde(default)]
    pub queries: Vec<Computed>,
}

pub(in crate::indexer::resolve::engine) fn capture(
    file: &ParsedFile,
    binder: &TypeBinder,
) -> Input {
    let graph = binder.graph;
    let root = |root: &Root<BindingId>| match root {
        Root::Unknown => Value::Unknown,
        Root::Global(name) => Value::Global(*name),
        Root::Binding(binding) if graph.module.imports.contains_key(binding) => Value::Import {
            binding: binding.0,
            unit: graph
                .module
                .import_units
                .get(binding)
                .copied()
                .unwrap_or(SourceModuleId(0)),
        },
        Root::Binding(binding) => {
            let declaration = graph
                .symbol_slots
                .get(binding)
                .copied()
                .flatten()
                .and_then(|slot| binder.ids.row_id(binder.path, slot));
            if let Some(recipe) = graph.types.annotations.get(binding) {
                Value::Annotated {
                    declaration,
                    recipe: program_types::lower(recipe, binder),
                }
            } else {
                declaration
                    .map(Value::Declaration)
                    .unwrap_or(Value::Unknown)
            }
        }
    };
    let computed = |(site, key): &(SourceSpan, Key<BindingId>)| match key {
        Key::Computed {
            root: value,
            selectors,
            usage,
            ..
        } => Computed {
            site: *site,
            root: root(value),
            selectors: selectors.clone(),
            usage: *usage,
        },
        _ => Computed {
            site: *site,
            root: Value::Unknown,
            selectors: vec![],
            usage: ValueUse::Runtime,
        },
    };
    Input {
        classes: file
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind == SymbolKind::Class)
            .filter_map(|(slot, _)| binder.ids.row_id(binder.path, slot))
            .collect(),
        members: graph
            .types
            .member_values
            .iter()
            .filter_map(|member| {
                Some(MemberValue {
                    owner: binder.ids.row_id(binder.path, member.owner)?,
                    name: member.name,
                    signature: member.signature,
                    static_: member.static_,
                    readable: member.readable,
                })
            })
            .collect(),
        keys: graph.types.computed_keys.iter().map(computed).collect(),
        queries: graph.types.value_queries.iter().map(computed).collect(),
    }
}

pub(super) type Sources<'a> = [(&'a String, &'a SourceInstanceId, &'a program_types::Input)];
pub(super) trait Reader {
    fn recipe(&self, source: SourceInstanceId, recipe: &Recipe) -> TypeId;
    fn field(&self, row: i64) -> Option<TypeId>;
    fn signature(&self, source: SourceInstanceId, signature: SignatureId) -> Option<TypeId>;
    fn expand(&self, ty: TypeId) -> Option<TypeId>;
    fn bases(&self, owner: i64) -> Option<Vec<TypeId>>;
}
#[derive(Default)]
pub(super) struct Values {
    classes: FxHashSet<i64>,
    members: FxHashMap<(i64, MemberNameId, bool), Vec<(SourceInstanceId, SignatureId, bool)>>,
    names: FxHashMap<(SourceInstanceId, NameId), (MemberNameId, Option<ExportNameId>)>,
}

pub(super) fn prepare(view: &mut View, inputs: &Sources) -> Values {
    let mut values = Values::default();
    for (_, &source, input) in inputs {
        for (name, spelling) in &input.names {
            values.names.insert(
                (source, *name),
                (
                    view.members.intern_name(spelling),
                    view.modules.export_name(spelling),
                ),
            );
        }
        values.classes.extend(
            input
                .computed
                .classes
                .iter()
                .filter_map(|row| view.canonical.get(row))
                .copied(),
        );
        for member in &input.computed.members {
            let Some(&owner) = view.canonical.get(&member.owner) else {
                continue;
            };
            let Some(&(name, _)) = values.names.get(&(source, member.name)) else {
                continue;
            };
            values
                .members
                .entry((owner, name, member.static_))
                .or_default()
                .push((source, member.signature, member.readable));
        }
    }
    values
}

impl Values {
    pub(super) fn value(
        &self,
        lookup: &Lookup,
        arena: &TypeArena,
        path: &str,
        source: SourceInstanceId,
        key: &Computed,
        reader: &dyn Reader,
    ) -> Option<TypeId> {
        let mut selectors = key.selectors.as_slice();
        let mut ty = match &key.root {
            Value::Unknown => return None,
            Value::Annotated {
                declaration,
                recipe,
            } => {
                if let Some(row) = declaration {
                    lookup.symbol_by_id(*row)?;
                }
                reader.recipe(source, recipe)
            }
            Value::Declaration(row) => self.declaration(lookup, arena, *row, reader)?,
            Value::Import { binding, unit } => {
                let domain = if key.usage == ValueUse::Runtime {
                    ExportDomain::Value
                } else {
                    ExportDomain::ValueQuery
                };
                let mut target =
                    lookup
                        .view
                        .modules
                        .binding_in(path, *unit, BindingId(*binding), domain);
                while let (Some(namespace), Some((name, rest))) =
                    (target.namespace(), selectors.split_first())
                {
                    let domain = if key.usage == ValueUse::Query {
                        ExportDomain::ValueQuery
                    } else {
                        ExportDomain::Value
                    };
                    target = lookup.view.modules.select_export_in(
                        namespace,
                        self.names.get(&(source, *name))?.1?,
                        domain,
                    );
                    selectors = rest;
                }
                self.declaration(lookup, arena, target.declaration()?, reader)?
            }
            Value::Global(name) => self.declaration(
                lookup,
                arena,
                lookup.global_value(*name).declaration?,
                reader,
            )?,
        };
        for name in selectors {
            ty = self.member(
                lookup,
                arena,
                ty,
                self.names.get(&(source, *name))?.0,
                reader,
            )?;
        }
        let ty = reader.expand(ty)?;
        (lookup as &dyn SymbolLookup)
            .accepts_type_context(arena, ty)
            .then_some(ty)
    }

    fn declaration(
        &self,
        lookup: &Lookup,
        arena: &TypeArena,
        row: i64,
        reader: &dyn Reader,
    ) -> Option<TypeId> {
        lookup.symbol_by_id(row)?;
        if self.classes.contains(&lookup.canonical_decl_id(row)) {
            Some(arena.intern(Type::Constructor(
                (lookup as &dyn SymbolLookup).declaration_type(arena, row)?,
            )))
        } else {
            reader.field(row)
        }
    }

    fn member(
        &self,
        lookup: &Lookup,
        arena: &TypeArena,
        receiver: TypeId,
        name: MemberNameId,
        reader: &dyn Reader,
    ) -> Option<TypeId> {
        use crate::indexer::resolve::engine::{contract::generic_return, head_decl::head_decl_id};
        if !(lookup as &dyn SymbolLookup).accepts_type_context(arena, receiver) {
            return None;
        }
        let receiver = reader.expand(receiver)?;
        if let Some(member) = lookup.object_member(receiver, name) {
            return member.ok().map(|m| m.value);
        }
        let (receiver, static_) = match arena.get(receiver) {
            Type::Constructor(inner) => (inner, true),
            _ => (receiver, false),
        };
        let mut pending = vec![(receiver, FxHashSet::default())];
        for _ in 0..4096 {
            let Some((receiver, mut seen)) = pending.pop() else {
                return None;
            };
            let owner = lookup.canonical_decl_id(head_decl_id(arena, receiver)?);
            if seen.len() >= 64 || !seen.insert(owner) {
                return None;
            }
            let info = lookup.canonical_type_info(owner)?;
            let args = match arena.get(receiver) {
                Type::Apply { args, .. } => args,
                _ => vec![],
            };
            if !static_ && args.len() != info.generic_param_ids.len() {
                return None;
            }
            let substitutions = info.generic_param_ids.iter().copied().zip(args).collect();
            if !static_ {
                if let Some(proved) = lookup.view.query_members.get(&(owner, name)) {
                    return proved.map(|ty| generic_return::substitute(arena, ty, &substitutions));
                }
            }
            if let Some(candidates) = self.members.get(&(owner, name, static_)) {
                // Duplicate source declarations require merge compatibility,
                // even if their display types or provisional TypeIds agree.
                return match candidates.as_slice() {
                    [(source, signature, true)] => Some(generic_return::substitute(
                        arena,
                        reader.signature(*source, *signature)?,
                        &substitutions,
                    )),
                    _ => None,
                };
            }
            // Source order is preserved on type-base recipes. A private query
            // may use them before the final tables exist; failed heritage
            // admission removes the owner and rebinds every dependent query.
            for base in reader.bases(owner)?.into_iter().rev() {
                pending.push((
                    generic_return::substitute(arena, base, &substitutions),
                    seen.clone(),
                ));
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "program_computed_keys_tests.rs"]
mod tests;
