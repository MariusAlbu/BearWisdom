//! Typed facts keyed by declaration identity and execution scope. A nested
//! function's writes cannot mutate its parent's forward-inference state.
use super::cause::Cause;
use crate::indexer::lexical::{BindingId, LexicalBindings, ScopeId};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

#[path = "lexical_value_recipes.rs"]
mod values;

#[path = "lexical_initializers.rs"]
mod initializers;

pub(super) struct LexicalCache<'a> {
    pub(super) bindings: &'a LexicalBindings,
    cursor: Cell<u32>,
    declared: Vec<Option<TypeId>>,
    call_type_args: HashMap<u32, Vec<TypeId>>,
    member_type_args: HashMap<u32, Vec<TypeId>>,
    declarations: HashMap<BindingId, Option<i64>>,
    initial_callables: HashMap<BindingId, i64>,
    initial_types: HashMap<BindingId, TypeId>,
    value_recipes: HashMap<BindingId, values::BoundValue>,
    expression_recipes: HashMap<crate::types::SourceSpan, values::BoundValue>,
    value_context: Option<values::Context<'a>>,
    import_kinds: HashMap<BindingId, crate::types::SymbolKind>,
    import_namespaces: std::collections::HashSet<BindingId>,
    arena: &'a TypeArena,
    inferred_declarations: RefCell<HashMap<BindingId, TypeId>>,
    contextual: RefCell<HashMap<BindingId, TypeId>>,
    facts: RefCell<HashMap<(BindingId, ScopeId), Vec<(u32, TypeId)>>>,
    causes: RefCell<HashMap<(BindingId, ScopeId), Cause>>,
    callables: RefCell<HashMap<(BindingId, ScopeId), Vec<(u32, i64)>>>,
}

#[cfg(test)]
#[path = "lexical_cache_tests.rs"]
mod tests;

impl<'a> LexicalCache<'a> {
    pub(super) fn new(bindings: &'a LexicalBindings, arena: &'a TypeArena) -> Self {
        Self {
            bindings,
            cursor: Cell::new(0),
            declarations: HashMap::new(),
            initial_callables: HashMap::new(),
            initial_types: HashMap::new(),
            value_recipes: bindings
                .types
                .values
                .iter()
                .map(|(&binding, value)| (binding, values::lower(value, &|_| None, &|_| None, 0)))
                .collect(),
            expression_recipes: HashMap::new(),
            value_context: None,
            import_kinds: HashMap::new(),
            import_namespaces: Default::default(),
            member_type_args: HashMap::new(),
            call_type_args: bindings
                .call_type_args
                .iter()
                .map(|(&byte, args)| {
                    (
                        byte,
                        args.iter().map(|arg| arena.intern_type_str(arg)).collect(),
                    )
                })
                .collect(),
            arena,
            inferred_declarations: RefCell::new(HashMap::new()),
            contextual: RefCell::new(HashMap::new()),
            declared: bindings
                .bindings
                .iter()
                .map(|b| b.annotation.as_deref().map(|ty| arena.intern_type_str(ty)))
                .collect(),
            facts: RefCell::new(HashMap::new()),
            causes: RefCell::new(HashMap::new()),
            callables: RefCell::new(HashMap::new()),
        }
    }

    pub(super) fn set_cursor(&self, byte: u32) {
        self.cursor.set(byte);
    }

    pub(super) fn install_declarations(
        &mut self,
        path: &str,
        ids: &crate::indexer::symbol_ids::SymbolIds,
    ) {
        for (&binding, &slot) in &self.bindings.symbol_slots {
            self.declarations
                .insert(binding, slot.and_then(|slot| ids.row_id(path, slot)));
        }
    }

    pub(super) fn install_types(
        &mut self,
        path: &str,
        ids: &crate::indexer::symbol_ids::SymbolIds,
        lookup: &'a super::compilation::Compilation,
        selected: &dyn super::contract::SymbolLookup,
    ) {
        let binder = super::lexical_type_ids::TypeBinder {
            graph: self.bindings,
            path,
            ids,
            lookup: selected,
            source: Some(lookup),
            arena: self.arena,
        };
        let configured = selected.nominal_context().is_some();
        let materialize = |recipe| {
            if configured {
                super::program_types::lower(recipe, &binder).materialize(selected, self.arena, path)
            } else {
                binder.materialize(recipe)
            }
        };
        if configured {
            self.declared.fill(None);
            self.call_type_args.clear();
            for declaration in self.declarations.values_mut() {
                *declaration = declaration.filter(|&id| selected.symbol_by_id(id).is_some());
            }
        }
        let names: HashMap<_, _> = self
            .bindings
            .interned_names()
            .filter_map(|(id, spelling)| {
                super::contract::SymbolLookup::member_index(lookup)?
                    .name(spelling)
                    .map(|name| (id, name))
            })
            .collect();
        let owner = |slot| {
            ids.row_id(path, slot)
                .filter(|&id| super::contract::SymbolLookup::symbol_by_id(lookup, id).is_some())
        };
        let name = |id| names.get(&id).copied();
        self.value_recipes = self
            .bindings
            .types
            .values
            .iter()
            .map(|(&binding, value)| (binding, values::lower(value, &owner, &name, 0)))
            .collect();
        self.expression_recipes = self
            .bindings
            .types
            .expressions
            .iter()
            .map(|(&span, value)| (span, values::lower(value, &owner, &name, 0)))
            .collect();
        self.value_context = Some(
            values::Context::new(lookup, path, self.bindings.types.reference_fields).with_patterns(
                self.bindings
                    .types
                    .pattern_heads
                    .iter()
                    .map(|(&byte, recipe)| (byte, materialize(recipe)))
                    .collect(),
            ),
        );
        self.member_type_args = self
            .bindings
            .types
            .member_arguments
            .iter()
            .map(|(&byte, recipes)| (byte, recipes.iter().map(materialize).collect()))
            .collect();
        for (&binding, recipe) in &self.bindings.types.annotations {
            if configured || recipe.is_bound() {
                self.declared[binding.0] = Some(materialize(recipe));
            }
        }
        for (&binding, &slot) in &self.bindings.types.receiver_values {
            let ty = ids
                .row_id(path, slot)
                .and_then(|id| super::contract::SymbolLookup::canonical_type_info(lookup, id))
                .and_then(|info| info.receiver_type_id)
                .unwrap_or_else(|| self.arena.intern(Type::Unknown));
            self.declared[binding.0] = Some(ty);
        }
        for (&binding, recipe) in &self.bindings.types.constructed {
            self.initial_types.insert(binding, materialize(recipe));
        }
        if configured {
            self.install_initializers(selected);
        }
        for (&byte, recipes) in &self.bindings.types.arguments {
            if configured
                || recipes
                    .iter()
                    .any(crate::indexer::lexical::type_syntax::TypeExpr::is_bound)
            {
                self.call_type_args
                    .insert(byte, recipes.iter().map(materialize).collect());
            }
        }
    }

    pub(super) fn reference(
        &self,
        byte: u32,
    ) -> Option<super::contract::flow_cache::LocalReference> {
        let binding = *self.bindings.references.get(&byte)?;
        let mut value = self.value_at(binding, self.cursor.get());
        value.type_args = self
            .call_type_args
            .get(&byte)
            .or_else(|| self.member_type_args.get(&byte))
            .cloned()
            .unwrap_or_default();
        Some(value)
    }

    pub(super) fn root_type_arguments(&self, byte: u32) -> &[TypeId] {
        self.call_type_args
            .get(&byte)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(super) fn value_expression(&self, span: crate::types::SourceSpan) -> Option<TypeId> {
        self.evaluate_value(self.expression_recipes.get(&span)?, 0)
    }

    fn evaluate_value(&self, value: &values::BoundValue, depth: usize) -> Option<TypeId> {
        values::evaluate(
            value,
            self.arena,
            depth,
            self.value_context.as_ref(),
            &|source, position, depth| self.type_at_depth(source, position, depth),
        )
    }

    pub(super) fn argument_reference(
        &self,
        span: crate::types::SourceSpan,
    ) -> Option<super::contract::flow_cache::LocalReference> {
        let binding = *self.bindings.argument_reads.get(&span)?;
        Some(self.value_at(binding, span.start))
    }

    pub(super) fn namespace_member(
        &self,
        selector: u32,
    ) -> Option<super::contract::flow_cache::LocalReference> {
        let binding = *self.bindings.module.members.get(&selector)?;
        let mut value = self.value_at(binding, self.cursor.get());
        value.type_args = self
            .member_type_args
            .get(&selector)
            .cloned()
            .unwrap_or_default();
        Some(value)
    }

    pub(super) fn namespace_root(&self, byte: u32) -> bool {
        self.bindings
            .references
            .get(&byte)
            .is_some_and(|binding| self.import_namespaces.contains(binding))
    }

    fn value_at(
        &self,
        binding: BindingId,
        byte: u32,
    ) -> super::contract::flow_cache::LocalReference {
        super::contract::flow_cache::LocalReference {
            declaration: self.declarations.get(&binding).copied().flatten(),
            kind: self
                .import_kinds
                .get(&binding)
                .or_else(|| self.bindings.kinds.get(&binding))
                .copied()
                .unwrap_or(crate::types::SymbolKind::Variable),
            value_type: self.type_at(binding, byte),
            type_args: Vec::new(),
            callable: self.callable_at(binding, byte),
        }
    }

    pub(super) fn member_type_arguments(&self, selector: u32) -> Option<&[TypeId]> {
        self.member_type_args.get(&selector).map(Vec::as_slice)
    }

    pub(super) fn install_initial_values(&mut self, lookup: &dyn super::contract::SymbolLookup) {
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
                        let instance = super::head_decl::nominal_head(lookup, self.arena, symbol);
                        self.initial_types
                            .insert(binding, self.arena.intern(Type::Constructor(instance)));
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn install_imports(
        &mut self,
        path: &str,
        lookup: &dyn super::contract::SymbolLookup,
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
                if let Some(ty) =
                    super::lexical_value::imported_overload_type(lookup, self.arena, path, binding)
                {
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

    /// Reset pass-local inference; syntax declarations belong to this snapshot.
    pub(super) fn clear(&self) {
        self.inferred_declarations.borrow_mut().clear();
        self.contextual.borrow_mut().clear();
        self.facts.borrow_mut().clear();
        self.causes.borrow_mut().clear();
        self.callables.borrow_mut().clear();
        self.cursor.set(0);
    }
    pub(super) fn binding(&self, name: &str) -> Option<BindingId> {
        self.bindings
            .binding_at(self.cursor.get(), self.bindings.name_id(name)?)
    }

    fn function(&self) -> Option<ScopeId> {
        self.function_at(self.cursor.get())
    }

    fn function_at(&self, byte: u32) -> Option<ScopeId> {
        Some(self.bindings.scopes[self.bindings.scope_at(byte)?.0].function)
    }

    pub(super) fn local_type(&self, name: &str) -> Option<TypeId> {
        self.type_of(self.binding(name)?)
    }

    fn type_of(&self, binding: BindingId) -> Option<TypeId> {
        self.type_at(binding, self.cursor.get())
    }

    fn type_at(&self, binding: BindingId, byte: u32) -> Option<TypeId> {
        self.type_at_depth(binding, byte, 0)
    }

    fn type_at_depth(&self, binding: BindingId, byte: u32, depth: usize) -> Option<TypeId> {
        if depth >= 128 {
            return None;
        }
        let metadata = &self.bindings.bindings[binding.0];
        // A temporal-dead-zone diagnostic does not erase declaration identity:
        // an explicit annotation still names the property's static definition.
        if byte < metadata.available_from {
            return self.declared[binding.0];
        }
        let initializer = || {
            self.value_recipes
                .get(&binding)
                .and_then(|value| self.evaluate_value(value, depth))
        };
        let declared = self.declared[binding.0]
            .map(|declared| {
                initializer()
                    .and_then(|actual| values::refine_regions(self.arena, declared, actual, 0))
                    .unwrap_or(declared)
            })
            .or_else(|| self.initial_types.get(&binding).copied())
            .or_else(|| self.contextual.borrow().get(&binding).copied())
            .or_else(|| self.inferred_declarations.borrow().get(&binding).copied())
            .or_else(initializer);
        // TS/JS's non-union declared type is not replaced by a structurally
        // assignable namesake on reassignment. Union narrowing remains flow-based.
        if let Some(ty) = declared {
            if self.bindings.preserve_non_union_type
                && !matches!(self.arena.get(ty), Type::Union(_))
            {
                return Some(ty);
            }
        }
        let mut function = self.function_at(byte)?;
        let facts = self.facts.borrow();
        loop {
            if let Some((_, ty)) = facts.get(&(binding, function)).and_then(|versions| {
                versions
                    .iter()
                    .rev()
                    .find(|(position, _)| *position <= byte)
            }) {
                return Some(*ty);
            }
            let Some(parent) = self.bindings.scopes[function.0].parent else {
                break;
            };
            function = self.bindings.scopes[parent.0].function;
        }
        declared
    }

    pub(super) fn record(&self, binding: BindingId, ty: TypeId, initializer: bool) {
        if let Some(function) = self.function() {
            if initializer
                && function
                    == self.bindings.scopes[self.bindings.bindings[binding.0].scope.0].function
            {
                self.inferred_declarations
                    .borrow_mut()
                    .entry(binding)
                    .or_insert(ty);
            }
            self.facts
                .borrow_mut()
                .entry((binding, function))
                .or_default()
                .push((self.cursor.get(), ty));
        }
    }

    /// A callback declaration's context belongs to that binding, not to the
    /// caller's current execution scope. Conflicting contexts remain unknown.
    pub(super) fn record_contextual(&self, binding: BindingId, ty: TypeId) {
        self.contextual
            .borrow_mut()
            .entry(binding)
            .and_modify(|old| {
                if *old != ty {
                    *old = self.arena.intern(Type::Unknown);
                }
            })
            .or_insert(ty);
    }

    pub(super) fn record_cause(&self, binding: BindingId, cause: Cause) {
        self.record(binding, self.arena.intern(Type::Unknown), false);
        if let Some(function) = self.function() {
            self.causes.borrow_mut().insert((binding, function), cause);
        }
    }
    pub(super) fn cause(&self, name: &str) -> Option<Cause> {
        self.causes
            .borrow()
            .get(&(self.binding(name)?, self.function()?))
            .copied()
    }
    pub(super) fn record_callable(&self, binding: BindingId, declaration: i64) {
        if let Some(function) = self.function() {
            self.callables
                .borrow_mut()
                .entry((binding, function))
                .or_default()
                .push((self.cursor.get(), declaration));
        }
    }
    pub(super) fn callable(&self, name: &str) -> Option<i64> {
        self.callable_at(self.binding(name)?, self.cursor.get())
    }

    fn callable_at(&self, binding: BindingId, byte: u32) -> Option<i64> {
        let mut function = self.function_at(byte)?;
        let facts = self.callables.borrow();
        loop {
            if let Some((_, id)) = facts.get(&(binding, function)).and_then(|versions| {
                versions
                    .iter()
                    .rev()
                    .find(|(position, _)| *position <= byte)
            }) {
                return Some(*id);
            }
            let Some(parent) = self.bindings.scopes[function.0].parent else {
                break;
            };
            function = self.bindings.scopes[parent.0].function;
        }
        self.initial_callables.get(&binding).copied()
    }
}
