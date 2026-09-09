//! Materialized source recipes: serialized strings stay at the ingestion boundary.
use super::module_paths::PathRules;
use crate::indexer::{
    lexical::{
        modules::{ExportTarget, Import},
        BindingId, LexicalBindings,
    },
    symbol_ids::SymbolIds,
};
use crate::types::ParsedFile;
use serde::{Deserialize, Serialize};

/// BindingIds are snapshot-local: a source hash alone cannot validate a cache
/// across a change to the syntax binder's ID allocation or declaration policy.
pub(super) const BINDING_EPOCH: u32 = 78;

#[path = "module_scope_inputs.rs"]
mod source_units;

pub(super) use crate::indexer::namespaces::{ExportDomain, SourceModuleId};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct InputUnit {
    #[serde(default)]
    pub assignments: Vec<(InputTarget, ExportDomain)>,
    #[serde(default)]
    pub source_scope: Option<source_units::Evidence>,
    #[serde(default)]
    pub source_name: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    pub id: SourceModuleId,
    pub parent: SourceModuleId,
    pub exports: Vec<InputExport>,
    pub imports: Vec<InputBinding>,
    pub stars: Vec<(InputTarget, ExportDomain)>,
    pub wildcard_exclusions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum InputTarget {
    Incomplete,
    Assigned {
        module: String,
    },
    Entity {
        declaration: Box<Self>,
        namespace: Box<Self>,
    },
    Declaration(i64),
    Declarations(Vec<i64>),
    Overloads(Vec<i64>),
    From {
        module: String,
        name: String,
    },
    Namespace {
        module: String,
    },
    Path {
        base: Box<Self>,
        names: Vec<String>,
    },
    Missing,
    LocalNamespace(SourceModuleId),
    LocalExport {
        module: SourceModuleId,
        name: String,
    },
    Binding {
        module: SourceModuleId,
        binding: usize,
        domain: ExportDomain,
    },
    Select {
        base: Box<Self>,
        selectors: Vec<(String, ExportDomain)>,
    },
    ContextPath {
        base: Box<Self>,
        selectors: Vec<(String, ExportDomain)>,
        origin: SourceModuleId,
    },
    DeclarationPath {
        base: Box<Self>,
        names: Vec<String>,
    },
    Access {
        target: Box<Self>,
        scope: Option<Box<Self>>,
        origin: SourceModuleId,
        declaration: bool,
    },
    CrateRoot,
    ExternalRoot(String),
    Parent(SourceModuleId, u32),
    SourceFile(usize),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct InputSourceFile {
    pub owner: SourceModuleId,
    pub name: String,
    pub path: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FileLayout {
    pub extension: String,
    pub directory_entry: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct InputExport {
    pub name: String,
    #[serde(default)]
    pub domain: ExportDomain,
    pub target: InputTarget,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct InputBinding {
    pub binding: usize,
    #[serde(default)]
    pub domain: ExportDomain,
    pub target: InputTarget,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct InputExtension {
    pub binding: usize,
    pub origin: SourceModuleId,
    pub members: Vec<i64>,
    pub arity: usize,
    #[serde(default)]
    pub kinds: Vec<crate::type_checker::core::types::GenericParamKind>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct ModuleInput {
    #[serde(default)]
    pub assignments: Vec<(InputTarget, ExportDomain)>,
    #[serde(default)]
    pub source_complete: Option<bool>,
    #[serde(default)]
    pub globals: Option<super::program_input::Input>,
    #[serde(default)]
    pub bases: Vec<super::base_receiver::Input>,
    #[serde(default)]
    pub traits: super::module_trait_inputs::Data,
    #[serde(default)]
    pub extensions: Vec<InputExtension>,
    #[serde(default)]
    pub declaration_access: Vec<InputTarget>,
    #[serde(default)]
    pub source_spans: Vec<(SourceModuleId, u32, u32)>,
    #[serde(default)]
    pub file_layout: Option<FileLayout>,
    #[serde(default)]
    pub source_files: Vec<InputSourceFile>,
    #[serde(default)]
    pub binding_epoch: u32,
    pub path: String,
    pub content_hash: String,
    pub exports: Vec<InputExport>,
    pub imports: Vec<InputBinding>,
    pub stars: Vec<(String, bool)>,
    pub paths: PathRules,
    pub signatures: Vec<super::module_type_inputs::Signature>,
    #[serde(default)]
    pub scoped_declarations: Vec<Vec<i64>>,
    #[serde(default)]
    pub units: Vec<InputUnit>,
    #[serde(default)]
    pub wildcard_exclusions: Vec<String>,
}

pub(super) fn capture(file: &ParsedFile, ids: &SymbolIds) -> Option<ModuleInput> {
    if let Some(input) = super::namespace_input::capture(file, ids) {
        return Some(input);
    }
    let graph = file.flow.lexical.as_ref()?;
    let syntax = &graph.module;
    let forms = syntax.forms?;
    let mut input = ModuleInput {
        globals: super::program_input::capture(file, ids),
        binding_epoch: BINDING_EPOCH,
        path: super::module_paths::normalize(&file.path),
        content_hash: file.content_hash.clone(),
        stars: syntax.stars.clone(),
        scoped_declarations: scoped_declarations(file, ids),
        wildcard_exclusions: forms
            .wildcard_exclusions
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        paths: PathRules {
            extensions: forms.extensions.iter().map(|s| (*s).to_owned()).collect(),
            substitutions: forms
                .substitutions
                .iter()
                .map(|(suffix, items)| {
                    (
                        (*suffix).to_owned(),
                        items.iter().map(|s| (*s).to_owned()).collect(),
                    )
                })
                .collect(),
            directory_entry: forms.directory_entry.into(),
        },
        ..Default::default()
    };
    source_units::capture(file, graph, ids, &mut input);
    input.exports = exports(&syntax.exports, graph, &file.path, ids);
    input.assignments = assignments(&syntax.assignments, graph, &file.path, ids);
    Some(input)
}

fn exports(
    exports: &[crate::indexer::lexical::modules::Export],
    graph: &LexicalBindings,
    path: &str,
    ids: &SymbolIds,
) -> Vec<InputExport> {
    let mut result = Vec::new();
    for export in exports {
        for domain in [
            ExportDomain::Value,
            ExportDomain::Type,
            ExportDomain::ValueQuery,
        ] {
            // An explicit type-only export shadows wildcard namesakes even
            // where it cannot supply a runtime value.
            let target = if export.type_only && domain == ExportDomain::Value {
                InputTarget::Missing
            } else {
                exported(&export.target, graph, domain, path, ids)
            };
            result.push(InputExport {
                name: export.name.clone(),
                domain,
                target,
            });
        }
    }
    result
}

fn exported(
    target: &ExportTarget,
    graph: &LexicalBindings,
    domain: ExportDomain,
    path: &str,
    ids: &SymbolIds,
) -> InputTarget {
    match target {
        ExportTarget::From(import) => imported(import, graph, domain, path, ids),
        ExportTarget::Module(unit) => InputTarget::LocalNamespace(*unit),
        ExportTarget::Local { value, ty } => {
            let binding = if domain == ExportDomain::Type {
                *ty
            } else if domain == ExportDomain::ValueQuery
                && ty.is_some_and(|id| graph.module.imports.contains_key(&id))
            {
                *ty
            } else {
                *value
            };
            local(graph, binding, domain, path, ids)
        }
        ExportTarget::Alias { binding, selectors } => InputTarget::Path {
            base: Box::new(local(graph, *binding, domain, path, ids)),
            names: selectors.clone(),
        },
        ExportTarget::Unknown => InputTarget::Missing,
    }
}

fn assignments(
    targets: &[ExportTarget],
    graph: &LexicalBindings,
    path: &str,
    ids: &SymbolIds,
) -> Vec<(InputTarget, ExportDomain)> {
    targets
        .iter()
        .flat_map(|target| {
            [
                ExportDomain::Value,
                ExportDomain::Type,
                ExportDomain::ValueQuery,
            ]
            .map(|domain| (exported(target, graph, domain, path, ids), domain))
        })
        .collect()
}

fn imported(
    import: &Import,
    graph: &LexicalBindings,
    domain: ExportDomain,
    path: &str,
    ids: &SymbolIds,
) -> InputTarget {
    use crate::indexer::lexical::modules::ImportSource;
    if import.type_only && domain == ExportDomain::Value {
        return InputTarget::Missing;
    }
    let base = match &import.source {
        ImportSource::Named { module, name } => InputTarget::From {
            module: module.clone(),
            name: name.clone(),
        },
        ImportSource::Namespace(module) => InputTarget::Namespace {
            module: module.clone(),
        },
        ImportSource::Assignment(module) => InputTarget::Assigned {
            module: module.clone(),
        },
        ImportSource::Binding(binding) => local(graph, *binding, domain, path, ids),
        ImportSource::Entity { binding, namespace } => InputTarget::Entity {
            declaration: Box::new(local_declaration(
                graph,
                *binding,
                domain == ExportDomain::Type,
                path,
                ids,
            )),
            namespace: Box::new(InputTarget::LocalNamespace(*namespace)),
        },
    };
    if import.selectors.is_empty() {
        base
    } else {
        InputTarget::Path {
            base: Box::new(base),
            names: import.selectors.clone(),
        }
    }
}

fn local(
    graph: &LexicalBindings,
    binding: Option<BindingId>,
    domain: ExportDomain,
    path: &str,
    ids: &SymbolIds,
) -> InputTarget {
    let Some(binding) = binding else {
        return InputTarget::Missing;
    };
    if graph.module.ambiguous_imports.contains(&binding) {
        return InputTarget::Missing;
    }
    if graph.module.imports.contains_key(&binding) {
        return InputTarget::Binding {
            module: graph
                .module
                .import_units
                .get(&binding)
                .copied()
                .unwrap_or_default(),
            binding: binding.0,
            domain,
        };
    }
    local_declaration(graph, binding, domain == ExportDomain::Type, path, ids)
}

fn local_declaration(
    graph: &LexicalBindings,
    binding: BindingId,
    type_space: bool,
    path: &str,
    ids: &SymbolIds,
) -> InputTarget {
    if type_space {
        if let Some(slots) = graph.type_symbol_slots.get(&binding) {
            return slots
                .iter()
                .map(|&slot| ids.row_id(path, slot))
                .collect::<Option<Vec<_>>>()
                .map(InputTarget::Declarations)
                .unwrap_or(InputTarget::Missing);
        }
    }
    if graph.symbol_slots.get(&binding) == Some(&None)
        && !graph.overload_bindings.contains(&binding)
    {
        return InputTarget::Incomplete;
    }
    if !type_space && graph.overload_bindings.contains(&binding) {
        let rows: Option<Vec<_>> = graph
            .symbols
            .iter()
            .filter(|(_, id)| **id == binding)
            .map(|(&slot, _)| ids.row_id(path, slot))
            .collect();
        if let Some(mut rows) = rows.filter(|rows| rows.len() > 1) {
            rows.sort_unstable();
            rows.dedup();
            return InputTarget::Overloads(rows);
        }
    }
    graph
        .symbol_slots
        .get(&binding)
        .copied()
        .flatten()
        .and_then(|slot| ids.row_id(path, slot))
        .map(InputTarget::Declaration)
        .unwrap_or(InputTarget::Missing)
}

/// Every captured type row is a fence, including singleton and invalid groups.
/// The first row is the logical group's representative; prefer a runtime class
/// over an interface so constructor/static behavior retains the value domain.
pub(super) fn scoped_declarations(file: &ParsedFile, ids: &SymbolIds) -> Vec<Vec<i64>> {
    let Some(graph) = &file.flow.lexical else {
        return Vec::new();
    };
    let mut groups = Vec::new();
    for (&binding, slots) in &graph.type_symbol_slots {
        let mut rows: Vec<_> = slots
            .iter()
            .filter_map(|&slot| ids.row_id(&file.path, slot))
            .collect();
        if rows.len() == slots.len() && graph.mergeable_types.contains(&binding) {
            let representative = slots
                .iter()
                .position(|&slot| file.symbols[slot].kind == crate::types::SymbolKind::Class)
                .unwrap_or_else(|| {
                    rows.iter()
                        .enumerate()
                        .min_by_key(|(_, id)| *id)
                        .map(|(i, _)| i)
                        .unwrap_or(0)
                });
            rows.swap(0, representative);
            groups.push(rows);
        } else {
            groups.extend(rows.into_iter().map(|id| vec![id]));
        }
    }
    groups.sort_unstable();
    groups
}

#[cfg(test)]
#[path = "module_input_tests.rs"]
mod tests;
