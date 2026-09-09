//! Private expression declarations are ID-addressable, never global candidates.
use super::*;

impl Compilation {
    pub(in crate::indexer::resolve::engine) fn module_site(
        &self,
        file: &str,
    ) -> super::super::module_graph::ModuleSite {
        self.modules.site(file)
    }
    pub(in crate::indexer::resolve::engine) fn accessible_at(
        &self,
        declaration: i64,
        site: &super::super::module_graph::ModuleSite,
    ) -> bool {
        self.modules
            .declaration_access(self.canonical_decl_id(declaration), site.module())
    }
    pub(in crate::indexer::resolve::engine) fn accessible_at_byte(
        &self,
        declaration: i64,
        site: &super::super::module_graph::ModuleSite,
        byte: u32,
    ) -> bool {
        self.modules
            .declaration_access(self.canonical_decl_id(declaration), site.module_at(byte))
    }
    pub(super) fn capture_lexical_visibility(&mut self, files: &[ParsedFile], ids: &SymbolIds) {
        for file in files {
            if let Some(graph) = &file.flow.lexical {
                for binding in &graph.lexical_only {
                    if let Some(id) = graph
                        .symbol_slots
                        .get(binding)
                        .copied()
                        .flatten()
                        .and_then(|slot| ids.row_id(&file.path, slot))
                    {
                        self.lexical_only.insert(id);
                    }
                }
            }
        }
        self.expand_lexical_visibility();
    }

    pub(super) fn expand_lexical_visibility(&mut self) {
        // Their children are also not unqualified globals. ID-based member
        // lookup remains available through an already-bound receiver.
        let mut pending: Vec<_> = self.lexical_only.iter().copied().collect();
        while let Some(parent) = pending.pop() {
            for &child in self.members_by_id.get(&parent).into_iter().flatten() {
                if self.lexical_only.insert(child) {
                    pending.push(child);
                }
            }
        }
    }

    pub(in crate::indexer::resolve::engine) fn root_candidates<'a>(
        &self,
        symbols: SymbolSet<'a>,
    ) -> SymbolSet<'a> {
        if self.lexical_only.is_empty() {
            return symbols;
        }
        SymbolSet::Owned(
            symbols
                .into_iter()
                .filter(|s| !self.lexical_only.contains(&s.id))
                .collect(),
        )
    }
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for Compilation {
    fn member_pattern(
        &self,
        member: i64,
    ) -> Option<&super::super::contract::member_applicability::ReceiverPattern> {
        self.extension_patterns
            .get(&member)
            .map(|pattern| pattern.as_ref())
    }
    fn declaration_accessible(&self, declaration: i64) -> bool {
        self.modules
            .declaration_access(self.canonical_decl_id(declaration), None)
    }
}

#[cfg(test)]
#[path = "compilation_visibility_tests.rs"]
mod tests;
