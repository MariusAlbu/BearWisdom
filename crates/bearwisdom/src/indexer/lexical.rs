//! Snapshot-local lexical identities. Spelling is interned at ingestion; scope
//! traversal and all binding/type relations use distinct numeric ID domains.
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NameId(u32);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub(crate) usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingId(pub(crate) usize);

#[derive(Debug, Clone)]
pub struct Scope {
    pub(crate) parent: Option<ScopeId>,
    pub(crate) children: Vec<ScopeId>,
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) function: ScopeId,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub(crate) scope: ScopeId,
    pub(crate) available_from: u32,
    /// Annotation text is an ingestion payload, converted once to TypeId.
    pub(crate) annotation: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LexicalBindings {
    names: HashMap<String, NameId>,
    entries: HashMap<(ScopeId, NameId), BindingId>,
    ordered_entries: HashMap<(ScopeId, NameId), Vec<BindingId>>,
    pub(crate) declaration_starts: HashMap<u32, BindingId>,
    pub(crate) declaration_slots: HashMap<(u32, u32), Option<usize>>,
    pub(crate) capture_barriers: HashSet<ScopeId>,
    type_entries: HashMap<(ScopeId, NameId), BindingId>,
    pub(crate) type_parameters: HashMap<BindingId, (u32, u32, usize)>,
    pub(crate) type_parameter_sites: HashMap<BindingId, type_syntax::signatures::SignatureId>,
    pub(crate) types: type_syntax::TypeUses,
    pub(crate) module: modules::ModuleSyntax,
    pub(crate) globals: Option<globals::Capture>,
    pub(crate) scopes: Vec<Scope>,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) writes: HashMap<usize, BindingId>,
    pub(crate) initializers: HashSet<usize>,
    pub(crate) symbols: HashMap<usize, BindingId>,
    pub(crate) symbol_slots: HashMap<BindingId, Option<usize>>,
    pub(crate) dual_types: HashMap<BindingId, BindingId>,
    pub(crate) type_symbol_slots: HashMap<BindingId, Vec<usize>>,
    pub(crate) mergeable_types: HashSet<BindingId>,
    pub(crate) overload_bindings: HashSet<BindingId>,
    pub(crate) references: HashMap<u32, BindingId>,
    pub(crate) argument_reads: HashMap<crate::types::SourceSpan, BindingId>,
    /// Call-root syntax payload; decoded once into TypeIds by the file cache.
    pub(crate) call_type_args: HashMap<u32, Vec<String>>,
    pub(crate) kinds: HashMap<BindingId, crate::types::SymbolKind>,
    pub(crate) declarations: HashMap<crate::types::SourceSpan, BindingId>,
    pub(crate) lexical_only: HashSet<BindingId>,
    pub(crate) initial_values: HashMap<BindingId, BindingId>,
    pub(crate) preserve_non_union_type: bool,
    /// Declaration slots whose types are owned by source syntax, never by an
    /// extractor's synthetic initializer TypeRef.
    pub(crate) source_owned_types: HashSet<usize>,
}

impl LexicalBindings {
    pub(crate) fn attach_symbol(&mut self, slot: usize, binding: BindingId) {
        self.symbols.insert(slot, binding);
        self.symbol_slots
            .entry(binding)
            .and_modify(|old| {
                if *old != Some(slot) {
                    *old = None;
                }
            })
            .or_insert(Some(slot));
    }

    /// Token decoding boundary for legacy assignment-query captures.
    pub(crate) fn symbol_at(&self, byte: u32, name: &str) -> Option<usize> {
        let binding = self
            .declaration_starts
            .get(&byte)
            .copied()
            .or_else(|| self.binding_at(byte, self.name_id(name)?))?;
        self.symbol_slots.get(&binding).copied().flatten()
    }

    pub(crate) fn intern(&mut self, name: &str) -> NameId {
        let next = NameId(self.names.len() as u32);
        *self.names.entry(name.to_owned()).or_insert(next)
    }

    /// Ingestion-only bridge between this name arena and compilation name arenas.
    pub(crate) fn interned_names(&self) -> impl Iterator<Item = (NameId, &str)> {
        self.names.iter().map(|(text, &id)| (id, text.as_str()))
    }

    /// Transitional token-to-ID boundary for callers still passing extracted text.
    pub(crate) fn name_id(&self, name: &str) -> Option<NameId> {
        self.names.get(name).copied()
    }

    pub(crate) fn add_scope(
        &mut self,
        parent: Option<ScopeId>,
        start: u32,
        end: u32,
        function: bool,
    ) -> ScopeId {
        let id = ScopeId(self.scopes.len());
        let owner = if function {
            id
        } else {
            parent.map(|p| self.scopes[p.0].function).unwrap_or(id)
        };
        self.scopes.push(Scope {
            parent,
            children: Vec::new(),
            start,
            end,
            function: owner,
        });
        if let Some(p) = parent {
            self.scopes[p.0].children.push(id);
        }
        id
    }

    pub(crate) fn declare(
        &mut self,
        scope: ScopeId,
        name: NameId,
        available_from: u32,
        annotation: Option<String>,
    ) -> BindingId {
        if let Some(&id) = self.entries.get(&(scope, name)) {
            return id;
        }
        let id = BindingId(self.bindings.len());
        self.bindings.push(Binding {
            scope,
            available_from,
            annotation,
        });
        self.entries.insert((scope, name), id);
        id
    }

    pub(crate) fn scope_at(&self, byte: u32) -> Option<ScopeId> {
        let mut scope = ScopeId(0);
        let root = self.scopes.first()?;
        if byte < root.start || byte >= root.end {
            return None;
        }
        loop {
            let children = &self.scopes[scope.0].children;
            let next = children.partition_point(|id| self.scopes[id.0].start <= byte);
            let child = next.checked_sub(1).map(|i| children[i]);
            match child {
                Some(id) if byte < self.scopes[id.0].end => scope = id,
                _ => return Some(scope),
            }
        }
    }

    pub(crate) fn lookup(&self, mut scope: ScopeId, name: NameId) -> Option<BindingId> {
        loop {
            if let Some(id) = self.entries.get(&(scope, name)) {
                return Some(*id);
            }
            scope = self.scopes[scope.0].parent?;
        }
    }

    pub(crate) fn binding_at(&self, byte: u32, name: NameId) -> Option<BindingId> {
        let mut scope = self.scope_at(byte)?;
        loop {
            if let Some(entries) = self.ordered_entries.get(&(scope, name)) {
                let next = entries.partition_point(|id| self.bindings[id.0].available_from <= byte);
                if let Some(index) = next.checked_sub(1) {
                    return Some(entries[index]);
                }
            }
            if let Some(&binding) = self.entries.get(&(scope, name)) {
                return Some(binding);
            }
            if self.capture_barriers.contains(&scope) {
                return None;
            }
            scope = self.scopes[scope.0].parent?;
        }
    }

    /// Ordered declarations never merge: the initializer still sees the previous
    /// binding, and source-addressed writes name the new declaration directly.
    pub(crate) fn declare_ordered(
        &mut self,
        scope: ScopeId,
        name: NameId,
        available_from: u32,
        annotation: Option<String>,
    ) -> BindingId {
        let id = BindingId(self.bindings.len());
        self.bindings.push(Binding {
            scope,
            available_from,
            annotation,
        });
        self.ordered_entries
            .entry((scope, name))
            .or_default()
            .push(id);
        id
    }

    /// A type-only declaration/import is an unavailable value, not a global search.
    /// A real value declaration still wins in the separate value namespace.
    pub(crate) fn reference_binding_at(&self, byte: u32, name: NameId) -> Option<BindingId> {
        self.binding_at(byte, name)
            .or_else(|| self.type_binding_at(byte, name))
    }

    /// Source value expressions use the value namespace. A pure type declaration
    /// does not shadow an outer/configured value; imports still reserve a name
    /// in their own scope, including imports forbidden in the value domain.
    pub(crate) fn value_expression_binding_at(&self, byte: u32, name: NameId) -> Option<BindingId> {
        let mut scope = self.scope_at(byte)?;
        loop {
            if let Some(entries) = self.ordered_entries.get(&(scope, name)) {
                let next = entries.partition_point(|id| self.bindings[id.0].available_from <= byte);
                if let Some(index) = next.checked_sub(1) {
                    return Some(entries[index]);
                }
            }
            if let Some(&binding) = self.entries.get(&(scope, name)) {
                return Some(binding);
            }
            if let Some(&binding) = self.type_entries.get(&(scope, name)) {
                if self.module.imports.contains_key(&binding) {
                    return Some(binding);
                }
            }
            if self.capture_barriers.contains(&scope) {
                return None;
            }
            scope = self.scopes[scope.0].parent?;
        }
    }

    pub(crate) fn type_binding_at(&self, byte: u32, name: NameId) -> Option<BindingId> {
        let mut scope = self.scope_at(byte)?;
        loop {
            if let Some(&binding) = self.type_entries.get(&(scope, name)) {
                return Some(binding);
            }
            scope = self.scopes[scope.0].parent?;
        }
    }

    pub(super) fn declare_type(&mut self, scope: ScopeId, name: NameId) -> BindingId {
        if let Some(&id) = self.type_entries.get(&(scope, name)) {
            return id;
        }
        let id = BindingId(self.bindings.len());
        self.bindings.push(Binding {
            scope,
            available_from: self.scopes[scope.0].start,
            annotation: None,
        });
        self.type_entries.insert((scope, name), id);
        id
    }
}

#[path = "lexical_globals.rs"]
pub(crate) mod globals;
#[path = "lexical_modules.rs"]
pub(crate) mod modules;
#[path = "lexical_selections.rs"]
mod selections;
#[path = "lexical_type_syntax.rs"]
pub(crate) mod type_syntax;

#[path = "lexical_declaration_sites.rs"]
mod declaration_sites;
#[path = "lexical_detached.rs"]
pub(crate) mod detached;
#[path = "lexical_import_names.rs"]
pub(crate) mod import_names;
#[path = "lexical_ingest.rs"]
mod ingest;
#[path = "lexical_merges.rs"]
mod merges;
#[path = "lexical_symbols.rs"]
pub(crate) mod symbol_rows;
pub(crate) use ingest::LexicalSyntax;
pub(crate) use ingest::{capture, syntax_for};

#[cfg(test)]
#[path = "lexical_tests.rs"]
mod tests;
