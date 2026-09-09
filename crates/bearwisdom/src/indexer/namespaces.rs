//! Source-addressed module bindings. Names are interned before path relations form.
use super::lexical::{BindingId, LexicalBindings, NameId, ScopeId};
use crate::types::{ExtractedRef, ExtractedSymbol};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum ExportDomain {
    #[default]
    Value,
    Type,
    Macro,
    Lifetime,
    ValueQuery,
}
impl From<bool> for ExportDomain {
    fn from(type_space: bool) -> Self {
        if type_space {
            Self::Type
        } else {
            Self::Value
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct SourceModuleId(pub u32);

pub(crate) struct Forms {
    pub patterns: &'static types::patterns::Forms,
    pub places: &'static crate::languages::common::call_args::PlaceSyntax,
    pub borrows: Option<&'static crate::languages::common::call_args::BorrowSyntax>,
    pub argument_groups: &'static [&'static str],
    pub traits: &'static traits::Forms,
    pub locals: &'static locals::Forms,
    pub types: &'static types::Forms,
    pub extensions: &'static super::lexical::detached::Forms,
    pub extension_constraints: &'static [&'static str],
    pub module: &'static str,
    pub body: &'static str,
    pub scopes: &'static [&'static str],
    pub declarations: &'static [(&'static str, &'static [ExportDomain])],
    pub access_items: &'static [&'static str],
    /// Container kind and optional required field for implicit public access.
    pub public_member_containers: &'static [(&'static str, &'static str)],
    pub imports: &'static super::lexical::import_names::Forms,
    pub rename_path: &'static str,
    pub path_nodes: &'static [(&'static str, &'static str, &'static str)],
    pub identifiers: &'static [&'static str],
    pub function: &'static str,
    pub visibility: &'static str,
    pub public: &'static str,
    pub attributes: &'static [&'static str],
    pub attribute_body: &'static str,
    pub conditional_attributes: &'static [&'static str],
    pub path_attribute: &'static str,
    pub file_extension: &'static str,
    pub directory_entry: &'static str,
    pub self_path: &'static str,
    pub parent_path: &'static str,
    pub crate_path: &'static str,
    pub self_type: &'static str,
    pub raw_prefix: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) enum Target {
    Declaration(usize),
    Module(SourceModuleId),
    Binding(BindingId),
    Select(Box<Target>, Vec<(NameId, ExportDomain)>, SourceModuleId),
    External(NameId),
    Missing,
    DeclarationPath(Box<Target>, Vec<NameId>),
    CrateRoot,
    Parent(SourceModuleId, u32),
    SourceFile(usize),
}
#[derive(Debug, Clone)]
pub(crate) struct Binding {
    pub domain: ExportDomain,
    pub targets: Vec<Target>,
}
#[derive(Debug, Clone)]
pub(crate) struct Unit {
    pub parent: Option<SourceModuleId>,
    pub scope: ScopeId,
    pub name: Option<NameId>,
    pub path: Option<String>,
    pub range: (u32, u32),
}
#[derive(Debug, Clone)]
pub(crate) struct DeclarationAccess {
    pub slot: usize,
    pub unit: SourceModuleId,
    pub scope: Option<Target>,
}
#[derive(Debug, Clone)]
pub(crate) struct Extension {
    pub owner: BindingId,
    pub unit: SourceModuleId,
    pub members: Vec<usize>,
    pub arity: usize,
    pub kinds: Vec<crate::type_checker::core::types::GenericParamKind>,
}
#[derive(Debug, Clone)]
pub(crate) struct SourceFile {
    pub owner: SourceModuleId,
    pub name: NameId,
    pub path: Option<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct Export {
    pub unit: SourceModuleId,
    pub name: NameId,
    pub binding: BindingId,
    pub domain: ExportDomain,
    pub access: Option<Target>,
    pub declaration: bool,
}
#[derive(Debug, Clone, Copy)]
/// `local` means authoritative source evidence: includes captured bare-value
/// misses, which must remain unknown rather than reopen a global name ladder.
pub(crate) struct Use {
    pub binding: BindingId,
    pub domain: ExportDomain,
    pub local: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathKeyword {
    Current,
    Parent,
    Crate,
}
#[derive(Debug, Clone, Default)]
pub struct NamespaceData {
    pub(crate) call_arguments: arguments::Table,
    pub(crate) traits: traits::Data,
    pub(crate) file_layout: Option<(&'static str, &'static str)>,
    pub(crate) source_files: Vec<SourceFile>,
    graph: LexicalBindings,
    names: HashMap<NameId, String>,
    path_keywords: HashMap<NameId, PathKeyword>,
    entries: HashMap<(ScopeId, NameId, ExportDomain), BindingId>,
    scope_units: HashMap<ScopeId, SourceModuleId>,
    opaque_scopes: std::collections::HashSet<ScopeId>,
    pub(crate) units: Vec<Unit>,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) exports: Vec<Export>,
    pub(crate) declaration_access: Vec<DeclarationAccess>,
    pub(crate) extensions: Vec<Extension>,
    pub(crate) extension_parameters: HashMap<BindingId, (Use, usize)>,
    pub(crate) roots: HashMap<u32, Use>,
    pub(crate) selectors: HashMap<u32, Use>,
    /// Method-call selector -> enclosing source declaration slot. Captured
    /// independently of extracted graph refs, including initializer cascades.
    pub(crate) method_calls: HashMap<u32, usize>,
    /// Exact borrow expression -> physical caller slot and source mutability.
    pub(crate) borrow_sites:
        HashMap<crate::types::SourceSpan, (usize, crate::type_checker::core::types::Mutability)>,
}

impl NamespaceData {
    pub(crate) fn spelling(&self, name: NameId) -> &str {
        &self.names[&name]
    }
    fn intern(&mut self, name: &str, forms: &Forms) -> NameId {
        let name = name.strip_prefix(forms.raw_prefix).unwrap_or(name);
        let id = self.graph.intern(name);
        for (keyword, kind) in [
            (forms.self_path, PathKeyword::Current),
            (forms.parent_path, PathKeyword::Parent),
            (forms.crate_path, PathKeyword::Crate),
        ] {
            if name == keyword {
                self.path_keywords.insert(id, kind);
            }
        }
        self.names.entry(id).or_insert_with(|| name.into());
        id
    }
    fn declare(
        &mut self,
        scope: ScopeId,
        name: NameId,
        domain: ExportDomain,
        target: Target,
    ) -> BindingId {
        let next = BindingId(self.bindings.len());
        let id = *self
            .entries
            .entry((scope, name, domain))
            .or_insert_with(|| {
                self.bindings.push(Binding {
                    domain,
                    targets: Vec::new(),
                });
                next
            });
        self.bindings[id.0].targets.push(target);
        id
    }
    fn query(&mut self, domain: ExportDomain, target: Target) -> BindingId {
        let id = BindingId(self.bindings.len());
        self.bindings.push(Binding {
            domain,
            targets: vec![target],
        });
        id
    }
    fn lookup(&self, mut scope: ScopeId, name: NameId, domain: ExportDomain) -> Option<Target> {
        let unit = self.scope_units.get(&scope)?;
        loop {
            if let Some(&binding) = self.entries.get(&(scope, name, domain)) {
                return Some(Target::Binding(binding));
            }
            // Unsupported wildcard providers may shadow any outer name. An explicit
            // binding in this same scope still takes precedence over glob imports.
            if self.opaque_scopes.contains(&scope) {
                return Some(Target::Missing);
            }
            if scope == self.units[unit.0 as usize].scope {
                return None;
            }
            scope = self.graph.scopes[scope.0].parent?;
        }
    }
    fn locally_attested<'a>(&'a self, mut target: &'a Target) -> bool {
        let mut visited = std::collections::HashSet::new();
        loop {
            match target {
                Target::External(_) | Target::CrateRoot | Target::Parent(_, _) => return false,
                Target::Select(base, _, _) => target = base,
                Target::Binding(id) => {
                    if !visited.insert(*id) {
                        return true;
                    }
                    let targets = &self.bindings[id.0].targets;
                    // Competing explicit imports are never a legacy-fallback opportunity.
                    if targets.len() != 1 {
                        return true;
                    }
                    target = &targets[0];
                }
                _ => return true,
            }
        }
    }
}

pub(crate) fn syntax_for(prefix: &str) -> Option<&'static Forms> {
    match prefix {
        "rust" => Some(&crate::languages::rust_lang::namespaces::FORMS),
        _ => None,
    }
}

pub(crate) fn capture(
    root: tree_sitter::Node,
    source: &[u8],
    prefix: &str,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
) -> Option<NamespaceData> {
    Some(ingest::capture(
        root,
        source,
        syntax_for(prefix)?,
        symbols,
        refs,
    ))
}

pub(crate) fn stamp_calls(
    root: tree_sitter::Node,
    source: &[u8],
    prefix: &str,
    refs: &mut [ExtractedRef],
) {
    if let Some(forms) = syntax_for(prefix) {
        occurrences::stamp(root, source, forms, refs);
    }
}

/// Reuse the namespace pass's scope/name arena; local BindingIds and namespace
/// recipe BindingIds are separate domains and are joined only by explicit Uses.
pub(crate) fn capture_locals(
    data: &mut NamespaceData,
    root: tree_sitter::Node,
    source: &[u8],
    prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &[ExtractedRef],
    policy: super::flow::BindingSymbols,
) -> LexicalBindings {
    if let Some(forms) = syntax_for(prefix) {
        locals::capture(data, root, source, forms, symbols, refs, policy);
        types::capture(data, root, source, forms, symbols);
    }
    std::mem::take(&mut data.graph)
}

#[path = "namespace_arguments.rs"]
pub(crate) mod arguments;
#[path = "namespace_ingest.rs"]
mod ingest;
#[path = "namespace_locals.rs"]
pub(crate) mod locals;
#[path = "namespace_occurrences.rs"]
mod occurrences;
#[path = "namespace_paths.rs"]
mod paths;
#[path = "namespace_source_files.rs"]
mod source_files;
#[cfg(test)]
#[path = "namespaces_tests.rs"]
mod tests;
#[path = "namespace_traits.rs"]
pub(crate) mod traits;
#[path = "namespace_types.rs"]
pub(crate) mod types;
