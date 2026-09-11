//! Import-side installation for a file's lexical cache: the declaration each
//! import binding names, the overload group it names when no single
//! declaration does, and the initial callable/constructor facts of values
//! bound from another binding.
use super::LexicalCache;
use crate::indexer::lexical::BindingId;
use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::type_checker::core::types::Type;

impl LexicalCache<'_> {
    pub(in crate::indexer::resolve::engine) fn install_initial_values(
        &mut self,
        lookup: &dyn SymbolLookup,
    ) {
        for (&binding, value) in &self.bindings.initial_values {
            let Some(id) = self.declarations.get(value).copied().flatten() else {
                continue;
            };
            match self.bindings.kinds.get(value) {
                Some(crate::types::SymbolKind::Function) => {
                    self.initial_callables.insert(binding, id);
                }
                Some(crate::types::SymbolKind::Class) => {
                    if let Some(symbol) = lookup.symbol_by_id(id) {
                        let instance = crate::indexer::resolve::engine::head_decl::nominal_head(
                            lookup, self.arena, symbol,
                        );
                        self.initial_types
                            .insert(binding, self.arena.intern(Type::Constructor(instance)));
                    }
                }
                _ => {}
            }
        }
    }

    /// An import bound to one declaration installs it. An import bound to an
    /// overload group installs the group's rows and its intersection type; the
    /// declaration slot stays empty because no single row is the target.
    pub(in crate::indexer::resolve::engine) fn install_imports(
        &mut self,
        path: &str,
        lookup: &dyn SymbolLookup,
    ) {
        for &binding in self.bindings.module.imports.keys() {
            if lookup.bound_import_namespace(path, binding) {
                self.import_namespaces.insert(binding);
                self.import_kinds
                    .insert(binding, crate::types::SymbolKind::Namespace);
            }
            let target = lookup.bound_import(path, binding, false);
            self.declarations.insert(binding, target);
            if target.is_none() {
                let rows = lookup.bound_import_overloads(path, binding);
                if !rows.is_empty() {
                    self.import_overloads.insert(binding, rows.to_vec());
                }
                if let Some(ty) = crate::indexer::resolve::engine::lexical_value::imported_overload_type(
                    lookup, self.arena, path, binding,
                ) {
                    self.initial_types.insert(binding, ty);
                }
            }
            if let Some(symbol) = target.and_then(|id| lookup.symbol_by_id(id)) {
                if let Ok(kind) = symbol.kind.parse() {
                    self.import_kinds.insert(binding, kind);
                }
                if let Some(ty) = lookup.field_type_id_of(symbol.id) {
                    self.initial_types.insert(binding, ty);
                }
            }
        }
    }

    /// The overload rows the import binding referenced at `byte` names, when
    /// that import bound a group rather than one declaration.
    pub(in crate::indexer::resolve::engine) fn import_overloads_at(&self, byte: u32) -> Option<&[i64]> {
        let binding: BindingId = *self.bindings.references.get(&byte)?;
        self.import_overloads.get(&binding).map(Vec::as_slice)
    }
}
