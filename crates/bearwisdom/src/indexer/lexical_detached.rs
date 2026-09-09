//! Bind out-of-line bodies to local nominal declarations at source ingestion.
use super::{BindingId, LexicalBindings, ScopeId};
use crate::types::ExtractedSymbol;
use std::collections::HashMap;
use tree_sitter::Node;

pub(crate) struct Forms {
    pub scopes: &'static [&'static str],
    pub isolated_scopes: &'static [&'static str],
    pub opaque_bindings: &'static [&'static str],
    pub imports: Option<&'static super::import_names::Forms>,
    pub name_prefixes: &'static [&'static str],
    pub declarations: &'static [(&'static str, bool)],
    pub extension: &'static str,
    pub target: &'static str,
    pub wrappers: &'static [(&'static str, &'static str)],
    pub identifiers: &'static [&'static str],
    pub members: &'static [&'static str],
    pub excluded_fields: &'static [&'static str],
}

pub(crate) fn capture(root: Node, source: &[u8], forms: &Forms, symbols: &mut [ExtractedSymbol]) {
    let mut anchors = HashMap::new();
    for (slot, symbol) in symbols.iter().enumerate() {
        anchors
            .entry((symbol.start_line, symbol.start_col))
            .and_modify(|s| *s = None)
            .or_insert(Some(slot));
    }
    let mut graph = LexicalBindings::default();
    let root_scope = graph.add_scope(None, root.start_byte() as u32, root.end_byte() as u32, true);
    let mut builder = Builder {
        graph,
        owners: HashMap::new(),
        extensions: Vec::new(),
        anchors,
        source,
        forms,
        opaque: std::collections::HashSet::new(),
        wildcards: std::collections::HashSet::new(),
    };
    builder.walk(root, root_scope);
    for (node, scope) in &builder.extensions {
        if forms
            .excluded_fields
            .iter()
            .any(|field| node.child_by_field_name(field).is_some())
        {
            continue;
        }
        let Some(mut target) = node.child_by_field_name(forms.target) else {
            continue;
        };
        while let Some(&(_, field)) = forms
            .wrappers
            .iter()
            .find(|&&(kind, _)| kind == target.kind())
        {
            let Some(inner) = target.child_by_field_name(field) else {
                break;
            };
            target = inner;
        }
        if !forms.identifiers.contains(&target.kind()) {
            continue;
        }
        let owner = target
            .utf8_text(source)
            .ok()
            .and_then(|s| builder.graph.name_id(builder.name(s)))
            .and_then(|name| builder.graph.lookup(*scope, name))
            .and_then(|binding| builder.owner(binding, *scope));
        let (Some(owner), Some(body)) = (owner, node.child_by_field_name("body")) else {
            continue;
        };
        let mut cursor = body.walk();
        for member in body
            .named_children(&mut cursor)
            .filter(|n| forms.members.contains(&n.kind()))
        {
            if let Some(slot) = builder.slot(member) {
                if symbols[slot].parent_index.is_none() {
                    symbols[slot].parent_index = Some(owner);
                }
            }
        }
    }
    builder.trait_body_ancestry(symbols);
}

struct Builder<'a, 'tree> {
    graph: LexicalBindings,
    owners: HashMap<BindingId, Option<usize>>,
    extensions: Vec<(Node<'tree>, ScopeId)>,
    anchors: HashMap<(u32, u32), Option<usize>>,
    source: &'a [u8],
    forms: &'a Forms,
    opaque: std::collections::HashSet<ScopeId>,
    wildcards: std::collections::HashSet<ScopeId>,
}

impl<'tree> Builder<'_, 'tree> {
    fn trait_body_ancestry(&self, symbols: &mut [ExtractedSymbol]) {
        for (node, _) in &self.extensions {
            if !self
                .forms
                .excluded_fields
                .iter()
                .any(|field| node.child_by_field_name(field).is_some())
            {
                continue;
            }
            let Some(container) = self.slot(*node) else {
                continue;
            };
            let mut ancestor = node.parent();
            while let Some(parent) = ancestor {
                if self.forms.scopes.contains(&parent.kind()) {
                    if let Some(owner) = self.slot(parent).filter(|&owner| owner != container) {
                        symbols[container].parent_index = Some(owner);
                        break;
                    }
                }
                ancestor = parent.parent();
            }
            let Some(body) = node.child_by_field_name("body") else {
                continue;
            };
            let mut cursor = body.walk();
            for member in body
                .named_children(&mut cursor)
                .filter(|n| self.forms.members.contains(&n.kind()))
            {
                if let Some(slot) = self.slot(member) {
                    // This is the physical impl container, never a guessed
                    // nominal owner. Filtering must see nested body ancestry.
                    if symbols[slot].parent_index.is_none() {
                        symbols[slot].parent_index = Some(container);
                    }
                }
            }
        }
    }

    fn owner(&self, binding: BindingId, mut scope: ScopeId) -> Option<usize> {
        loop {
            // Imported/qualified names need their own binder. Their presence
            // cannot turn an outer namesake into proof of nominal ownership.
            if self.opaque.contains(&scope) {
                return None;
            }
            if scope == self.graph.bindings[binding.0].scope {
                break;
            }
            if self.wildcards.contains(&scope) {
                return None;
            }
            scope = self.graph.scopes[scope.0].parent?;
        }
        self.owners.get(&binding).copied().flatten()
    }

    fn slot(&self, node: Node) -> Option<usize> {
        let point = node.start_position();
        self.anchors
            .get(&(point.row as u32, point.column as u32))
            .copied()
            .flatten()
    }

    fn name<'a>(&self, name: &'a str) -> &'a str {
        self.forms
            .name_prefixes
            .iter()
            .find_map(|prefix| name.strip_prefix(prefix))
            .unwrap_or(name)
    }

    fn walk(&mut self, node: Node<'tree>, inherited: ScopeId) {
        if self.forms.opaque_bindings.contains(&node.kind()) {
            self.opaque.insert(inherited);
        }
        if let Some(names) = self
            .forms
            .imports
            .and_then(|forms| super::import_names::capture(node, self.source, forms))
        {
            for name in names.exposed {
                let name = self.graph.intern(self.name(name));
                let binding = self.graph.declare(inherited, name, 0, None);
                self.owners.insert(binding, None);
            }
            if names.unknown {
                self.opaque.insert(inherited);
            }
            if names.wildcard {
                self.wildcards.insert(inherited);
            }
            return;
        }
        if let Some(&(_, nominal)) = self
            .forms
            .declarations
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
        {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(self.source).ok())
            {
                let name = self.graph.intern(self.name(name));
                let binding = self.graph.declare(inherited, name, 0, None);
                let slot = nominal.then(|| self.slot(node)).flatten();
                self.owners
                    .entry(binding)
                    .and_modify(|s| *s = None)
                    .or_insert(slot);
            }
        }
        let scope = if self.forms.scopes.contains(&node.kind()) {
            let parent = (!self.forms.isolated_scopes.contains(&node.kind())).then_some(inherited);
            self.graph.add_scope(
                parent,
                node.start_byte() as u32,
                node.end_byte() as u32,
                false,
            )
        } else {
            inherited
        };
        if node.kind() == self.forms.extension {
            self.extensions.push((node, scope));
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, scope);
        }
    }
}

#[cfg(test)]
#[path = "lexical_detached_tests.rs"]
mod tests;
