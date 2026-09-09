//! Import/export spelling is decoded here, before semantic binding starts.
use super::{BindingId, LexicalBindings, NameId, ScopeId};
use crate::indexer::namespaces::SourceModuleId;
use crate::types::{SourceSpan, SymbolKind};
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

#[path = "lexical_module_aliases.rs"]
mod aliases;
#[path = "lexical_module_names.rs"]
mod names;
#[path = "lexical_module_scopes.rs"]
pub(crate) mod scopes;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ImportForm {
    Default,
    Named,
    Namespace,
}
#[derive(Debug)]
pub(crate) struct ModuleForms {
    pub import_require: &'static str,
    pub import_alias: &'static str,
    pub assignment_token: &'static str,
    pub containers: &'static [&'static str],
    pub literal_names: &'static [&'static str],
    pub identifier_names: &'static [&'static str],
    pub declaration_wrappers: &'static [&'static str],
    pub augmentation: (&'static str, &'static str, &'static str),
    pub ambient_token: &'static str,
    pub import: &'static str,
    pub export: &'static str,
    pub import_forms: &'static [(&'static str, ImportForm)],
    pub import_containers: &'static [&'static str],
    pub declaration_lists: &'static [&'static str],
    pub export_clause: &'static str,
    pub export_specifier: &'static str,
    pub namespace_export: &'static str,
    pub selections: &'static [(&'static str, &'static str, &'static str, bool)],
    pub extensions: &'static [&'static str],
    pub substitutions: &'static [(&'static str, &'static [&'static str])],
    pub directory_entry: &'static str,
    pub wildcard_exclusions: &'static [&'static str],
}

#[derive(Debug, Clone)]
pub(crate) struct Import {
    pub source: ImportSource,
    pub type_only: bool,
    pub selectors: Vec<String>,
}
#[derive(Debug, Clone)]
pub(crate) enum ImportSource {
    Named {
        module: String,
        name: String,
    },
    Namespace(String),
    Assignment(String),
    Binding(Option<BindingId>),
    Entity {
        binding: BindingId,
        namespace: SourceModuleId,
    },
}
#[derive(Debug, Clone)]
pub(crate) enum ExportTarget {
    Local {
        value: Option<BindingId>,
        ty: Option<BindingId>,
    },
    From(Import),
    Module(SourceModuleId),
    Alias {
        binding: Option<BindingId>,
        selectors: Vec<String>,
    },
    Unknown,
}
#[derive(Debug, Clone)]
pub(crate) struct Export {
    pub name: String,
    pub target: ExportTarget,
    pub type_only: bool,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleSyntax {
    pub forms: Option<&'static ModuleForms>,
    pub imports: HashMap<BindingId, Import>,
    pub import_units: HashMap<BindingId, SourceModuleId>,
    pub ambiguous_imports: HashSet<BindingId>,
    pub units: Vec<scopes::Unit>,
    pub complete: bool,
    pub exports: Vec<Export>,
    pub assignments: Vec<ExportTarget>,
    pub stars: Vec<(String, bool)>,
    pub members: HashMap<u32, BindingId>,
    pub qualified_types: HashMap<SourceSpan, BindingId>,
}

pub(super) fn capture(
    root: Node,
    source: &[u8],
    forms: &'static ModuleForms,
    graph: &mut LexicalBindings,
) -> ModuleSyntax {
    let mut result = ModuleSyntax {
        forms: Some(forms),
        complete: !root.has_error(),
        ..Default::default()
    };
    let mut units = scopes::capture(root, source, forms, graph);
    aliases::namespaces(&units, graph, &mut result);
    result.complete &= capture_imports(
        root,
        (ScopeId(0), SourceModuleId(0)),
        source,
        forms,
        graph,
        &mut result,
    );
    for (unit, body) in &mut units {
        unit.complete &= capture_imports(
            *body,
            (unit.scope, unit.id),
            source,
            forms,
            graph,
            &mut result,
        );
    }
    let root_exports = capture_exports(root, source, forms, graph, &units, false);
    result.complete &= root_exports.complete;
    result.exports = root_exports.exports;
    result.assignments = root_exports.assignments;
    result.stars = root_exports.stars;
    for (index, (unit, body)) in units.iter().enumerate() {
        let exports = capture_exports(*body, source, forms, graph, &units, unit.ambient);
        let mut unit = unit.clone();
        unit.complete &= exports.complete;
        unit.exports = exports.exports;
        unit.stars = exports.stars;
        unit.assignments = exports.assignments;
        debug_assert_eq!(unit.id.0 as usize, index + 1);
        result.units.push(unit);
    }
    result
}

fn capture_imports(
    root: Node,
    owner: (ScopeId, SourceModuleId),
    source: &[u8],
    forms: &ModuleForms,
    graph: &mut LexicalBindings,
    result: &mut ModuleSyntax,
) -> bool {
    let mut complete = !root.has_error();
    let mut cursor = root.walk();
    // Imports are installed first: exports may rename imported bindings.
    for node in root
        .named_children(&mut cursor)
        .filter(|n| n.kind() == forms.import)
    {
        if let Some(required) = node
            .named_child(0)
            .filter(|n| n.kind() == forms.import_require)
        {
            complete &= aliases::required(node, required, owner, source, graph, result);
            continue;
        }
        let Some(module) = node
            .child_by_field_name("source")
            .and_then(|n| text(n, source))
        else {
            complete = false;
            continue;
        };
        complete &= import_children(
            node,
            source,
            forms,
            graph,
            result,
            &module,
            token(node, "type"),
            owner,
        );
    }
    complete &= aliases::internal(root, owner, source, forms, graph, result);
    complete
}

fn capture_exports(
    root: Node,
    source: &[u8],
    forms: &ModuleForms,
    graph: &LexicalBindings,
    units: &[(scopes::Unit, Node)],
    ambient: bool,
) -> ModuleSyntax {
    let mut result = ModuleSyntax {
        complete: !root.has_error(),
        ..Default::default()
    };
    let mut cursor = root.walk();
    // Export declarations/assignments disable ambient implicit exports; an
    // `export` modifier on a declaration does not. Compiler-checked behavior.
    let implicit = ambient
        && !root
            .named_children(&mut cursor)
            .any(|n| n.kind() == forms.export && n.child_by_field_name("declaration").is_none());
    if implicit {
        let mut cursor = root.walk();
        for node in root
            .named_children(&mut cursor)
            .filter(|n| n.kind() != forms.export && n.kind() != forms.import && !n.is_extra())
        {
            declaration_exports(node, source, forms, graph, &mut result, false, false, units);
        }
    }
    let mut cursor = root.walk();
    for node in root
        .named_children(&mut cursor)
        .filter(|n| n.kind() == forms.export)
    {
        if token(node, forms.assignment_token) {
            let value = node
                .child_by_field_name("value")
                .or_else(|| node.named_child(0));
            let target = value.and_then(|value| aliases::target(value, source, forms, graph));
            result.complete &= target.is_some() && result.assignments.is_empty();
            result
                .assignments
                .push(target.unwrap_or(ExportTarget::Unknown));
            continue;
        }
        let source_node = node.child_by_field_name("source");
        let from = source_node.and_then(|n| text(n, source));
        if source_node.is_some() && from.is_none() {
            result.complete = false;
            continue;
        }
        let type_only = token(node, "type");
        let mut children = node.walk();
        let clause = node
            .named_children(&mut children)
            .find(|n| n.kind() == forms.export_clause);
        let mut children = node.walk();
        let namespace = node
            .named_children(&mut children)
            .find(|n| n.kind() == forms.namespace_export);
        if let Some(clause) = clause {
            let mut specs = clause.walk();
            for spec in clause
                .named_children(&mut specs)
                .filter(|n| n.kind() == forms.export_specifier)
            {
                let Some(name) = spec.child_by_field_name("name") else {
                    result.complete = false;
                    continue;
                };
                let Some(local) = text(name, source) else {
                    result.complete = false;
                    continue;
                };
                let exported = if let Some(alias) = spec.child_by_field_name("alias") {
                    let Some(name) = text(alias, source) else {
                        result.complete = false;
                        continue;
                    };
                    name
                } else {
                    local.clone()
                };
                let type_only = type_only || token(spec, "type");
                let target = from
                    .as_ref()
                    .map(|module| {
                        ExportTarget::From(Import {
                            source: ImportSource::Named {
                                module: module.clone(),
                                name: local.clone(),
                            },
                            type_only,
                            selectors: Vec::new(),
                        })
                    })
                    .unwrap_or_else(|| {
                        local_target(graph, graph.name_id(&local), name.start_byte() as u32)
                    });
                result.exports.push(Export {
                    name: exported,
                    target,
                    type_only,
                });
            }
        } else if let Some(namespace) = namespace {
            if let Some(name) = namespace.named_child(0).and_then(|n| text(n, source)) {
                let target = from
                    .map(|module| {
                        ExportTarget::From(Import {
                            source: ImportSource::Namespace(module),
                            type_only,
                            selectors: Vec::new(),
                        })
                    })
                    .unwrap_or(ExportTarget::Unknown);
                result.exports.push(Export {
                    name,
                    target,
                    type_only,
                });
            } else {
                result.complete = false;
            }
        } else if token(node, "*") {
            if let Some(module) = from {
                result.stars.push((module, type_only));
            } else {
                result.complete = false;
            }
        } else if let Some(declaration) = node.child_by_field_name("declaration") {
            declaration_exports(
                declaration,
                source,
                forms,
                graph,
                &mut result,
                token(node, "default"),
                type_only,
                units,
            );
        } else if token(node, "default") {
            let target = node
                .child_by_field_name("value")
                .and_then(|value| {
                    let name = text(value, source)?;
                    Some(local_target(
                        graph,
                        graph.name_id(&name),
                        value.start_byte() as u32,
                    ))
                })
                .unwrap_or(ExportTarget::Unknown);
            result.exports.push(Export {
                name: "default".into(),
                target,
                type_only,
            });
        } else {
            result.complete = false;
        }
    }
    result
}

fn import_children(
    node: Node,
    source: &[u8],
    forms: &ModuleForms,
    graph: &mut LexicalBindings,
    output: &mut ModuleSyntax,
    module: &str,
    type_only: bool,
    owner: (ScopeId, SourceModuleId),
) -> bool {
    if let Some(&(_, form)) = forms
        .import_forms
        .iter()
        .find(|&&(kind, _)| kind == node.kind())
    {
        let (name, local) = match form {
            ImportForm::Default => ("default".to_owned(), node),
            ImportForm::Namespace => {
                let Some(local) = node.named_child(0) else {
                    return false;
                };
                (String::new(), local)
            }
            ImportForm::Named => {
                let Some(name) = node.child_by_field_name("name") else {
                    return false;
                };
                let Some(imported) = text(name, source) else {
                    return false;
                };
                (imported, node.child_by_field_name("alias").unwrap_or(name))
            }
        };
        let Some(local_name) = text(local, source) else {
            return false;
        };
        let type_only = type_only || token(node, "type");
        let id = graph.intern(&local_name);
        let binding = if type_only {
            graph.declare_type(owner.0, id)
        } else {
            graph.declare(owner.0, id, 0, None)
        };
        graph.type_entries.entry((owner.0, id)).or_insert(binding);
        graph.kinds.entry(binding).or_insert(SymbolKind::Variable);
        graph.declarations.insert(
            SourceSpan {
                start: local.start_byte() as u32,
                end: local.end_byte() as u32,
            },
            binding,
        );
        output.import_units.insert(binding, owner.1);
        let source = if matches!(form, ImportForm::Namespace) {
            ImportSource::Namespace(module.to_owned())
        } else {
            ImportSource::Named {
                module: module.to_owned(),
                name,
            }
        };
        if output
            .imports
            .insert(
                binding,
                Import {
                    source,
                    type_only,
                    selectors: Vec::new(),
                },
            )
            .is_some()
        {
            output.ambiguous_imports.insert(binding);
        }
        return true;
    }
    // Only grammar-declared containers can contain default/named import tokens.
    // Namespace/import-equals/attributes must not leak nested identifiers here.
    if !forms.import_containers.contains(&node.kind()) {
        return false;
    }
    let mut cursor = node.walk();
    let mut complete = true;
    for child in node
        .named_children(&mut cursor)
        .filter(|n| !n.is_extra() && n.kind() != "string")
    {
        complete &= import_children(
            child, source, forms, graph, output, module, type_only, owner,
        );
    }
    complete
}

fn declaration_exports(
    node: Node,
    source: &[u8],
    forms: &ModuleForms,
    graph: &LexicalBindings,
    output: &mut ModuleSyntax,
    default: bool,
    type_only: bool,
    units: &[(scopes::Unit, Node)],
) {
    if node.kind() == forms.import_alias {
        if let Some(name) = node.named_child(0).and_then(|n| text(n, source)) {
            output.exports.push(Export {
                target: local_target(graph, graph.name_id(&name), node.start_byte() as u32),
                name,
                type_only,
            });
        } else {
            output.complete = false;
        }
        return;
    }
    let module_kind = scopes::kind(node, forms);
    if let Some((unit, _)) = units.iter().find(|(unit, _)| {
        module_kind == Some(unit.kind)
            && unit.range.start == node.start_byte() as u32
            && unit.range.end == node.end_byte() as u32
    }) {
        if unit.kind == scopes::Kind::Namespace {
            if let Some(name) = node
                .child_by_field_name("name")
                .filter(|n| forms.identifier_names.contains(&n.kind()))
                .and_then(|n| text(n, source))
            {
                output.exports.push(Export {
                    name,
                    target: ExportTarget::Module(unit.id),
                    type_only,
                });
            } else {
                output.complete = false;
            }
        }
        return;
    }
    if forms.declaration_wrappers.contains(&node.kind()) {
        let mut cursor = node.walk();
        let children: Vec<_> = node
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .collect();
        if let [child] = children.as_slice() {
            if scopes::kind(*child, forms).is_some() {
                declaration_exports(
                    *child, source, forms, graph, output, default, type_only, units,
                );
                return;
            }
        }
        output.complete = false;
        return;
    }
    if let Some(name) = node.child_by_field_name("name") {
        if let Some(local) = text(name, source) {
            let target = local_target(graph, graph.name_id(&local), name.start_byte() as u32);
            output.exports.push(Export {
                name: if default { "default".into() } else { local },
                target,
                type_only,
            });
        }
        return;
    }
    if !forms.declaration_lists.contains(&node.kind()) {
        if default {
            output.exports.push(Export {
                name: "default".into(),
                target: ExportTarget::Unknown,
                type_only,
            });
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        declaration_exports(
            child, source, forms, graph, output, default, type_only, units,
        );
    }
}

fn local_target(graph: &LexicalBindings, name: Option<NameId>, byte: u32) -> ExportTarget {
    ExportTarget::Local {
        value: name.and_then(|name| graph.binding_at(byte, name)),
        ty: name.and_then(|name| graph.type_binding_at(byte, name)),
    }
}

fn token(node: Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|child| child.kind() == kind);
    found
}
fn text(node: Node, source: &[u8]) -> Option<String> {
    let raw = node.utf8_text(source).ok()?;
    if node.kind() == "string" {
        names::decode(raw)
    } else {
        Some(raw.to_owned())
    }
}

#[cfg(test)]
#[path = "lexical_modules_tests.rs"]
mod tests;
