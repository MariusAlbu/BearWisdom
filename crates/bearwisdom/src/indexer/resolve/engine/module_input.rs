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

#[path = "module_input_lowering.rs"]
mod lowering;
#[path = "module_scope_inputs.rs"]
mod source_units;
use lowering::{assignments, exports, imported};

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
    /// The export name a default import asks for, as the capture spelled it.
    #[serde(default)]
    pub default_name: Option<String>,
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
    /// The export name a default import asks for, as the capture spelled it.
    #[serde(default)]
    pub default_name: Option<String>,
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
        default_name: Some(forms.default_export_name.to_owned()),
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
