//! Source-global wrapper binding ends here; member traversal only sees TypeIds.
use super::*;
use crate::type_checker::core::types::{Intrinsic, LitValue, Type, TypeId};

pub(super) fn bind_legacy(
    modules: &ModuleGraph,
    tree: &Compilation,
) -> FxHashMap<Intrinsic, Option<i64>> {
    let mut requests =
        std::collections::BTreeMap::<Intrinsic, std::collections::BTreeSet<&str>>::new();
    let mut candidates = FxHashMap::<&str, Vec<Option<i64>>>::default();
    for source in modules.inputs.values() {
        if source.binding_epoch != super::super::module_input::BINDING_EPOCH {
            continue;
        }
        let Some(input) = &source.globals else {
            continue;
        };
        if let Some(types) = &input.types {
            let names: FxHashMap<_, _> = types
                .names
                .iter()
                .map(|(id, name)| (*id, name.as_str()))
                .collect();
            for (kind, name) in &types.intrinsic_members {
                if let Some(&name) = names.get(name) {
                    requests.entry(*kind).or_default().insert(name);
                }
            }
        }
        if input.isolated {
            continue;
        }
        for part in &input.roots {
            if part.unit != super::super::module_input::SourceModuleId(0) || !part.type_space {
                continue;
            }
            let row = (input.complete
                && matches!(
                    part.kind,
                    crate::types::SymbolKind::Interface | crate::types::SymbolKind::Class
                ))
            .then_some(part.declaration)
            .flatten()
            .filter(|&row| tree.symbol_by_id(row).is_some())
            .map(|row| tree.canonical_decl_id(row));
            candidates.entry(&part.name).or_default().push(row);
        }
    }
    requests
        .into_iter()
        .map(|(kind, names)| {
            let row = if names.len() == 1 {
                names
                    .first()
                    .and_then(|name| candidates.get(name))
                    .and_then(|rows| agreed(rows))
            } else {
                None
            };
            (kind, row)
        })
        .collect()
}

fn agreed(rows: &[Option<i64>]) -> Option<i64> {
    let first = *rows.first()?;
    rows.iter()
        .all(|row| *row == first)
        .then_some(first)
        .flatten()
}

/// A literal's exact value stays on its source fact. Only member lookup asks
/// for the apparent object type. Missing language/provider evidence is final.
pub(in crate::indexer::resolve::engine) fn project(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    ty: TypeId,
) -> Option<TypeId> {
    let kind = match arena.get(ty) {
        Type::Intrinsic(kind) => kind,
        Type::UniqueSymbol(_) => Intrinsic::Symbol,
        Type::Literal(LitValue::Str(_) | LitValue::Utf16(_)) => Intrinsic::String,
        Type::Literal(LitValue::Number(_)) => Intrinsic::Number,
        Type::Literal(LitValue::Bool(_)) => Intrinsic::Boolean,
        Type::Literal(LitValue::BigInt { .. }) => Intrinsic::BigInt,
        _ => return Some(ty),
    };
    lookup.intrinsic_member_type(kind)
}

#[cfg(test)]
#[path = "intrinsic_members_tests.rs"]
mod tests;
