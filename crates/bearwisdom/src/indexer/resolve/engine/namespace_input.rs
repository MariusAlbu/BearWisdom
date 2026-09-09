//! Lower captured source bindings to the shared module graph's durable recipes.
use super::module_input::*;
use crate::indexer::{
    namespaces::{NamespaceData, Target},
    symbol_ids::SymbolIds,
};
use crate::types::ParsedFile;

pub(super) fn capture(file: &ParsedFile, ids: &SymbolIds) -> Option<ModuleInput> {
    let data = file.flow.namespaces.as_ref()?;
    let mut input = ModuleInput {
        binding_epoch: BINDING_EPOCH,
        path: super::module_paths::normalize(&file.path),
        content_hash: file.content_hash.clone(),
        ..Default::default()
    };
    input.file_layout = data
        .file_layout
        .map(|(extension, directory_entry)| FileLayout {
            extension: extension.into(),
            directory_entry: directory_entry.into(),
        });
    input.extensions = data
        .extensions
        .iter()
        .map(|extension| InputExtension {
            binding: extension.owner.0,
            origin: extension.unit,
            members: extension
                .members
                .iter()
                .filter_map(|&slot| ids.row_id(&file.path, slot))
                .collect(),
            arity: extension.arity,
            kinds: extension.kinds.clone(),
        })
        .collect();
    input.source_files = data
        .source_files
        .iter()
        .map(|item| InputSourceFile {
            owner: item.owner,
            name: data.spelling(item.name).into(),
            path: item.path.clone(),
        })
        .collect();
    input.units = data
        .units
        .iter()
        .enumerate()
        .skip(1)
        .map(|(id, unit)| InputUnit {
            id: SourceModuleId(id as u32),
            parent: unit.parent.unwrap_or_default(),
            source_name: unit.name.map(|n| data.spelling(n).into()),
            source_path: unit.path.clone(),
            ..Default::default()
        })
        .collect();
    input.source_spans = data
        .units
        .iter()
        .enumerate()
        .map(|(id, unit)| (SourceModuleId(id as u32), unit.range.0, unit.range.1))
        .collect();
    input.declaration_access = data
        .declaration_access
        .iter()
        .map(|access| InputTarget::Access {
            target: Box::new(lower(&Target::Declaration(access.slot), data, file, ids)),
            scope: access
                .scope
                .as_ref()
                .map(|scope| Box::new(lower(scope, data, file, ids))),
            origin: access.unit,
            declaration: true,
        })
        .collect();
    for (id, binding) in data.bindings.iter().enumerate() {
        for target in &binding.targets {
            input.imports.push(InputBinding {
                binding: id,
                domain: binding.domain,
                target: lower(target, data, file, ids),
            });
        }
    }
    for export in &data.exports {
        let item = InputExport {
            name: data.spelling(export.name).into(),
            domain: export.domain,
            target: InputTarget::Access {
                target: Box::new(InputTarget::Binding {
                    module: SourceModuleId(0),
                    binding: export.binding.0,
                    domain: export.domain,
                }),
                scope: export
                    .access
                    .as_ref()
                    .map(|scope| Box::new(lower(scope, data, file, ids))),
                origin: export.unit,
                declaration: export.declaration,
            },
        };
        if export.unit.0 == 0 {
            input.exports.push(item);
        } else {
            input.units[export.unit.0 as usize - 1].exports.push(item);
        }
    }
    Some(input)
}

fn lower(target: &Target, data: &NamespaceData, file: &ParsedFile, ids: &SymbolIds) -> InputTarget {
    match target {
        Target::Declaration(slot) => ids
            .row_id(&file.path, *slot)
            .map(InputTarget::Declaration)
            .unwrap_or(InputTarget::Missing),
        Target::Module(id) => InputTarget::LocalNamespace(*id),
        Target::Binding(id) => InputTarget::Binding {
            module: SourceModuleId(0),
            binding: id.0,
            domain: data.bindings[id.0].domain,
        },
        Target::Select(base, selectors, origin) => InputTarget::ContextPath {
            base: Box::new(lower(base, data, file, ids)),
            selectors: selectors
                .iter()
                .map(|(name, domain)| (data.spelling(*name).into(), *domain))
                .collect(),
            origin: *origin,
        },
        Target::DeclarationPath(base, names) => InputTarget::DeclarationPath {
            base: Box::new(lower(base, data, file, ids)),
            names: names
                .iter()
                .map(|name| data.spelling(*name).into())
                .collect(),
        },
        Target::External(name) => InputTarget::ExternalRoot(data.spelling(*name).into()),
        Target::CrateRoot => InputTarget::CrateRoot,
        Target::Parent(unit, count) => InputTarget::Parent(*unit, *count),
        Target::SourceFile(id) => InputTarget::SourceFile(*id),
        Target::Missing => InputTarget::Missing,
    }
}

pub(super) fn use_fact(
    modules: &super::module_graph::ModuleGraph,
    lookup: &dyn super::contract::SymbolLookup,
    file: &str,
    usage: crate::indexer::namespaces::Use,
) -> Option<super::contract::flow_cache::LocalReference> {
    use super::module_graph::BindingResult;
    let result = modules.binding_in(file, SourceModuleId(0), usage.binding, usage.domain);
    if !usage.local && result == BindingResult::Unconfigured {
        return None;
    }
    let declaration = result.declaration();
    let kind = if matches!(result, BindingResult::Namespace(_)) {
        crate::types::SymbolKind::Namespace
    } else {
        declaration
            .and_then(|id| lookup.symbol_by_id(id))
            .and_then(|s| s.kind.parse().ok())
            .unwrap_or(crate::types::SymbolKind::Variable)
    };
    // Only a captured type-space selector may use a type alias as a static
    // receiver. A type-only declaration does not imply a runtime value.
    let value_type =
        if kind == crate::types::SymbolKind::TypeAlias && usage.domain == ExportDomain::Type {
            declaration
                .and_then(|id| lookup.symbol_by_id(id))
                .zip(lookup.type_arena())
                .map(|(symbol, arena)| super::head_decl::nominal_head(lookup, arena, symbol))
        } else {
            declaration.and_then(|id| lookup.field_type_id_of(id))
        };
    Some(super::contract::flow_cache::LocalReference {
        declaration,
        kind,
        type_args: Vec::new(),
        value_type,
        callable: None,
    })
}

#[cfg(test)]
#[path = "namespace_input_tests.rs"]
mod tests;
