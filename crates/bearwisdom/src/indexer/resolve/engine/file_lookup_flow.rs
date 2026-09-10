//! Transitional flow surface: migrated files read typed lexical facts and
//! require ID-addressed writes. Name-only writes remain legacy-only.
use super::*;

#[cfg(test)]
#[path = "file_lookup_flow_tests.rs"]
mod tests;

impl FileLookup<'_> {
    /// A callback-only graph reserves its lexical binding even before a
    /// contextual type is available. Legacy name maps must not supply a type
    /// for that identity, because a nested callback may reuse the spelling.
    fn callback_binding_at_cursor(&self, name: &str) -> bool {
        self.callback_lexical
            .as_ref()
            .is_some_and(|cache| cache.attested_binding(name).is_some())
    }

    pub(crate) fn record_failed_write(
        &self,
        reference: usize,
        cause: Option<crate::indexer::resolve::engine::cause::Cause>,
    ) {
        if let Some(cache) = &self.lexical {
            if let Some(&binding) = cache.bindings.writes.get(&reference) {
                if let Some(cause) = cause {
                    cache.record_cause(binding, cause);
                } else if let Some(arena) = self.tree.type_arena() {
                    cache.record(
                        binding,
                        arena.intern(crate::type_checker::core::types::Type::Unknown),
                        false,
                    );
                }
            }
        }
    }
    pub(crate) fn record_rhs_type(&self, reference: usize, legacy_name: &str, ty: TypeId) {
        if let Some(cache) = &self.lexical {
            if let Some(&binding) = cache.bindings.writes.get(&reference) {
                cache.record(
                    binding,
                    ty,
                    cache.bindings.initializers.contains(&reference),
                );
            }
        } else {
            self.record_local_type_id(legacy_name.to_owned(), ty);
        }
    }
    pub(crate) fn record_rhs_text(&self, reference: usize, legacy_name: &str, ty: String) {
        if self.lexical.is_some() {
            if let Some(arena) = self.tree.type_arena() {
                self.record_rhs_type(reference, legacy_name, arena.intern_type_str(&ty));
            }
        } else {
            self.record_local_type(legacy_name.to_owned(), ty);
        }
    }
    pub(crate) fn record_rhs_cause(
        &self,
        reference: usize,
        legacy_name: &str,
        cause: crate::indexer::resolve::engine::cause::Cause,
    ) {
        if let Some(cache) = &self.lexical {
            if let Some(&binding) = cache.bindings.writes.get(&reference) {
                cache.record_cause(binding, cause);
            }
        } else {
            self.record_root_cause_hint(legacy_name.to_owned(), cause);
        }
    }
    pub(crate) fn record_symbol_type(&self, symbol: usize, legacy_name: &str, ty: TypeId) {
        if let Some(cache) = &self.lexical {
            if let Some(&binding) = cache.bindings.symbols.get(&symbol) {
                cache.record(binding, ty, true);
            }
        } else {
            self.record_local_type_id(legacy_name.to_owned(), ty);
        }
    }
    pub(crate) fn record_symbol_callable(&self, symbol: usize, legacy_name: &str, target: i64) {
        if let Some(cache) = &self.lexical {
            if let Some(&binding) = cache.bindings.symbols.get(&symbol) {
                cache.record_callable(binding, target);
            }
        } else if let Some(declaration) = self.tree.symbol_by_id(target) {
            self.record_local_callable_head(
                legacy_name.to_owned(),
                declaration.qualified_name.clone(),
            );
        }
    }
}

// Flow cache: methods implemented over `locals` and `locals_id`.
impl<'a> FlowCacheLookup for FileLookup<'a> {
    fn evaluated_receiver(&self, receiver: TypeId) -> Option<Option<TypeId>> {
        self.structural().evaluated_receiver(receiver)
    }
    fn source_object_member(
        &self,
        receiver: TypeId,
        selector: u32,
    ) -> Option<Result<super::super::contract::flow_cache::ObjectMember, ()>> {
        self.program
            .as_ref()?
            .object_member(receiver, *self.method_names.get(&selector)?)
    }
    fn object_member_type(&self, receiver: TypeId, member: i64) -> Option<Option<TypeId>> {
        self.structural().object_member_type(receiver, member)
    }
    fn overloaded_call(
        &self,
        receiver: TypeId,
        selector: u32,
        actual: &[TypeId],
        explicit: &[TypeId],
    ) -> Option<Result<super::super::contract::flow_cache::OverloadCall, ()>> {
        let program = self.program.as_ref()?;
        if self.source_private_member(selector).is_some() {
            return None;
        }
        let name = *self.method_names.get(&selector)?;
        let callbacks = self
            .lexical
            .as_ref()
            .and_then(|c| c.bindings.globals.as_ref())
            .map(|g| &g.calls.callbacks);
        super::super::program_view::overload_calls::contextual(
            program,
            receiver,
            name,
            actual,
            explicit,
            &|index| callbacks.is_some_and(|c| c.contains_key(&(selector, index))),
            &|index, context| {
                super::super::program_view::callback_bodies::infer(
                    program,
                    self,
                    self.lexical.as_ref()?.bindings,
                    callbacks?.get(&(selector, index))?,
                    context,
                )
            },
            false,
        )
    }
    fn receiver_member_info(
        &self,
        owner: i64,
        member: i64,
    ) -> Option<&super::super::contract::TypeInfo> {
        self.structural().receiver_member_info(owner, member)
    }
    fn intrinsic_member_type(
        &self,
        kind: crate::type_checker::core::types::Intrinsic,
    ) -> Option<TypeId> {
        if let Some(program) = &self.program {
            return program.intrinsic_member_type(kind);
        }
        self.lexical
            .as_ref()?
            .bindings
            .types
            .intrinsic_members
            .get(&kind)?;
        self.tree.intrinsic_member_type(kind)
    }
    fn nominal_context(&self) -> Option<crate::type_checker::core::types::NominalContextId> {
        self.structural().nominal_context()
    }
    fn source_global_type(&self, name: crate::indexer::lexical::NameId) -> Option<Option<i64>> {
        self.structural().source_global_type(name)
    }
    fn source_value_type(&self, site: crate::types::SourceSpan) -> Option<Option<TypeId>> {
        self.structural().source_value_type(site)
    }
    fn source_initializer_type(
        &self,
        owner: crate::types::SourceSpan,
        target: crate::types::SourceSpan,
    ) -> Option<Option<TypeId>> {
        self.structural().source_initializer_type(owner, target)
    }
    fn source_signature_parameter(
        &self,
        owner: crate::types::SourceSpan,
        index: usize,
    ) -> Option<crate::type_checker::core::types::GenericParamId> {
        self.structural().source_signature_parameter(owner, index)
    }
    fn source_callable_origin(
        &self,
        owner: crate::types::SourceSpan,
    ) -> Option<crate::type_checker::core::types::CallableOrigin> {
        self.structural().source_callable_origin(owner)
    }
    fn source_unique_symbol(&self, declaration: crate::types::SourceSpan) -> Option<TypeId> {
        self.structural().source_unique_symbol(declaration)
    }
    fn value_expression(&self, span: crate::types::SourceSpan) -> Option<TypeId> {
        if self.program.is_some() {
            if let Some(atom) = self
                .lexical
                .as_ref()?
                .bindings
                .globals
                .as_ref()?
                .calls
                .atoms
                .get(&span)
            {
                return Some(self.tree.type_arena()?.intern(atom.ty()));
            }
        }
        self.lexical.as_ref()?.value_expression(span)
    }
    fn source_call_arguments(&self, selector: u32) -> Option<Result<&[crate::types::CallArg], ()>> {
        if self.program.is_some() {
            if let Some(globals) = self
                .lexical
                .as_ref()
                .and_then(|cache| cache.bindings.globals.as_ref())
            {
                return Some(
                    globals
                        .calls
                        .arguments
                        .get(&selector)
                        .and_then(|args| args.as_deref())
                        .ok_or(()),
                );
            }
        }
        Some(
            self.call_arguments?
                .get(&selector)
                .and_then(|args| args.as_deref())
                .ok_or(()),
        )
    }
    fn borrow_argument(&self, span: crate::types::SourceSpan, operand: TypeId) -> Option<TypeId> {
        use crate::type_checker::core::types::{Indirection, Lifetime, Type};
        let &(owner, mutability) = self.borrow_sites.get(&span)?;
        self.tree.symbol_by_id(owner)?;
        let arena = self.tree.type_arena()?;
        if matches!(arena.get(operand), Type::Unknown | Type::Class(_)) {
            return None;
        }
        Some(arena.intern(Type::Indirect {
            kind: Indirection::Reference(Lifetime::Inference {
                owner,
                byte: span.start,
            }),
            mutability,
            inner: operand,
        }))
    }
    fn qualified_call_site(&self, selector: u32) -> bool {
        self.trait_file
            .is_some_and(|file| file.qualified_calls.contains_key(&selector))
    }
    fn qualified_call(
        &self,
        selector: u32,
        actual: &[TypeId],
        explicit: &[TypeId],
    ) -> Option<Result<super::super::contract::flow_cache::BoundCall, ()>> {
        let call = self.trait_file?.qualified_calls.get(&selector)?;
        let Some(&name) = self.method_names.get(&selector) else {
            return Some(Err(()));
        };
        Some(super::super::trait_selection::qualified(
            self.tree.trait_graph(),
            self,
            self.tree.type_arena()?,
            call,
            name,
            actual,
            explicit,
        ))
    }
    fn bound_method(
        &self,
        receiver: TypeId,
        selector: u32,
    ) -> Option<Result<super::super::contract::flow_cache::BoundMethod, ()>> {
        if self.program.is_some() {
            if let Some(declaration) = self.source_private_member(selector) {
                return Some(declaration.and_then(|id| {
                    super::super::program_view::select_private(
                        self,
                        self.tree.type_arena().ok_or(())?,
                        receiver,
                        id,
                    )
                }));
            }
            return Some(
                self.method_names
                    .get(&selector)
                    .ok_or(())
                    .and_then(|&name| {
                        super::super::program_view::select_method(
                            self,
                            self.tree.type_arena().ok_or(())?,
                            receiver,
                            name,
                        )
                    }),
            );
        }
        let file = self.trait_file?;
        file.selectors.get(&selector)?;
        let Some(&name) = self.method_names.get(&selector) else {
            return Some(Err(()));
        };
        let caller = *self.method_calls.get(&selector)?;
        Some(super::super::trait_selection::select(
            self.tree.trait_graph(),
            file,
            self,
            self.tree.type_arena()?,
            receiver,
            name,
            selector,
            caller,
        ))
    }
    fn source_member_name(
        &self,
        selector: u32,
    ) -> Option<Result<super::super::member_index::MemberNameId, ()>> {
        self.program.as_ref()?;
        Some(self.method_names.get(&selector).copied().ok_or(()))
    }
    fn source_private_member(&self, selector: u32) -> Option<Result<i64, ()>> {
        self.private_members.get(&selector).map(|id| id.ok_or(()))
    }
    fn method_call_region(&self, selector: u32) -> Option<TypeId> {
        let owner = *self.method_calls.get(&selector)?;
        self.tree.symbol_by_id(owner)?;
        Some(
            self.tree
                .type_arena()?
                .intern(crate::type_checker::core::types::Type::Region(
                    crate::type_checker::core::types::Lifetime::Inference {
                        owner,
                        byte: selector,
                    },
                )),
        )
    }
    fn member_pattern(
        &self,
        member: i64,
    ) -> Option<&super::super::contract::member_applicability::ReceiverPattern> {
        self.tree.member_pattern(member)
    }
    fn declaration_accessible(&self, declaration: i64) -> bool {
        self.tree.accessible_at(declaration, &self.module_site)
    }
    fn namespace_member(
        &self,
        selector: u32,
    ) -> Option<crate::indexer::resolve::engine::contract::flow_cache::LocalReference> {
        self.lexical
            .as_ref()
            .and_then(|cache| cache.namespace_member(selector))
            .or_else(|| self.namespace_selectors.get(&selector).cloned())
    }
    fn namespace_root(&self, byte: u32) -> bool {
        self.lexical
            .as_ref()
            .is_some_and(|cache| cache.namespace_root(byte))
    }
    fn argument_reference(
        &self,
        span: crate::types::SourceSpan,
    ) -> Option<crate::indexer::resolve::engine::contract::flow_cache::LocalReference> {
        let global = self
            .lexical
            .as_ref()
            .and_then(|cache| cache.bindings.globals.as_ref())
            .and_then(|g| g.arguments.get(&span));
        if let Some((program, name)) = self.program.as_ref().zip(global) {
            return Some(program.global_value(*name));
        }
        self.lexical.as_ref()?.argument_reference(span)
    }
    fn member_type_arguments(&self, selector: u32) -> Option<&[TypeId]> {
        self.lexical.as_ref()?.member_type_arguments(selector)
    }
    fn local_reference(
        &self,
        byte: u32,
    ) -> Option<crate::indexer::resolve::engine::contract::flow_cache::LocalReference> {
        let global = || {
            let name = self
                .lexical
                .as_ref()?
                .bindings
                .globals
                .as_ref()?
                .values
                .get(&byte)?;
            let mut value = self.program.as_ref()?.global_value(*name);
            value.type_args = self.lexical.as_ref()?.root_type_arguments(byte).to_vec();
            Some(value)
        };
        global()
            .or_else(|| {
                self.lexical
                    .as_ref()
                    .and_then(|cache| cache.reference(byte))
            })
            .or_else(|| {
                self.callback_lexical
                    .as_ref()
                    .and_then(|cache| cache.reference(byte))
            })
            .or_else(|| self.namespace_roots.get(&byte).cloned())
    }
    fn record_contextual_type(&self, parameter: crate::types::SourceSpan, ty: TypeId) {
        if let Some(cache) = &self.lexical {
            if let Some(&binding) = cache.bindings.declarations.get(&parameter) {
                cache.record_contextual(binding, ty);
                return;
            }
        }
        if let Some(cache) = &self.callback_lexical {
            if let Some(&binding) = cache.bindings.declarations.get(&parameter) {
                cache.record_contextual(binding, ty);
            }
        }
    }
    /// Return the inferred type of `name` from the per-file forward-inference
    /// cache. Returns `None` when the name has not been bound by an earlier ref.
    fn local_type(&self, name: &str) -> Option<String> {
        if self.lexical.is_some() {
            return None;
        }
        if self.callback_binding_at_cursor(name) {
            return None;
        }
        self.locals.borrow().get(name).cloned()
    }

    /// Single-branch wrapper over `local_type` for the union-aware chain walker.
    fn local_type_union(&self, name: &str) -> Option<Vec<String>> {
        self.local_type(name).map(|t| vec![t])
    }

    /// Bind `name` to `type_name` in the String forward-inference cache. Evicts
    /// any prior TypeId binding for `name` so the two caches never both hold a
    /// stale entry for the same name — a reassignment's latest write wins
    /// regardless of which cache it lands in (`resolve_root` probes `locals_id`
    /// before `locals`).
    ///
    /// Does NOT evict `root_cause_hints` — that cache is consulted only as
    /// `resolve_root`'s last resort, strictly after `local_type_id`/`local_type`
    /// both miss, so a stale hint left behind by an earlier failed seed for
    /// this name is never read once this record succeeds.
    fn record_local_type(&self, name: String, type_name: String) {
        if self.lexical.is_some() || self.callback_binding_at_cursor(&name) {
            return;
        } // Scoped writes require declaration identity.
        self.locals_id.borrow_mut().remove(&name);
        self.locals.borrow_mut().insert(name, type_name);
    }

    /// Return the canonical TypeId binding for `name`. Preferred by the chain
    /// walker root step over `local_type` so non-nominal types (primitives,
    /// optionals, generics) are not nominalized on the round-trip.
    fn local_type_id(&self, name: &str) -> Option<TypeId> {
        if let Some(cache) = &self.lexical {
            return cache.local_type(name);
        }
        if let Some(cache) = &self.callback_lexical {
            if let Some(ty) = cache.attested_local_type(name) {
                return ty;
            }
        }
        self.locals_id.borrow().get(name).copied()
    }

    /// Bind `name` directly to a TypeId, bypassing `format_type` serialization.
    /// Evicts any prior String binding for `name` so a later reassignment that
    /// resolves to a TypeId supersedes an earlier String binding (and vice
    /// versa via `record_local_type`).
    fn record_local_type_id(&self, name: String, id: TypeId) {
        if self.lexical.is_some() || self.callback_binding_at_cursor(&name) {
            return;
        } // Never guess a lambda binding from its name.
        self.locals.borrow_mut().remove(&name);
        self.locals_id.borrow_mut().insert(name, id);
    }

    fn record_root_cause_hint(
        &self,
        name: String,
        cause: crate::indexer::resolve::engine::cause::Cause,
    ) {
        if self.lexical.is_some() || self.callback_binding_at_cursor(&name) {
            return;
        }
        self.root_cause_hints.borrow_mut().insert(name, cause);
    }

    fn local_callable_head(&self, name: &str) -> Option<String> {
        if self.lexical.is_some() || self.callback_binding_at_cursor(name) {
            return None;
        }
        self.local_callable_heads.borrow().get(name).cloned()
    }

    fn record_local_callable_head(&self, name: String, qname: String) {
        if self.lexical.is_some() || self.callback_binding_at_cursor(&name) {
            return;
        }
        self.local_callable_heads.borrow_mut().insert(name, qname);
    }

    fn root_cause_hint(&self, name: &str) -> Option<crate::indexer::resolve::engine::cause::Cause> {
        if let Some(cache) = &self.lexical {
            return cache.cause(name);
        }
        if self.callback_binding_at_cursor(name) {
            return None;
        }
        self.root_cause_hints.borrow().get(name).copied()
    }

    /// Activate the lexical scope and forward-fact position for this reference.
    fn set_cursor(&self, byte: u32) {
        self.module_site.set_cursor(byte);
        if let Some(cache) = &self.lexical {
            cache.set_cursor(byte);
        }
        if let Some(cache) = &self.callback_lexical {
            cache.set_cursor(byte);
        }
    }

    fn has_local_binding(&self, name: &str) -> bool {
        self.lexical
            .as_ref()
            .is_some_and(|cache| cache.binding(name).is_some())
            || self.callback_binding_at_cursor(name)
    }

    fn local_callable_id(&self, name: &str) -> Option<i64> {
        if let Some(cache) = &self.lexical {
            return cache.callable(name);
        }
        self.callback_lexical
            .as_ref()
            .and_then(|cache| cache.attested_callable(name).flatten())
    }

    /// No-op: CFG narrowing installation remains separate from lexical identity.
    fn install_local_cache(
        &self,
        _narrowings: Vec<crate::types::Narrowing>,
        _discriminants: Vec<crate::types::DiscriminantNarrowing>,
        _cfg: crate::indexer::flow_cfg::FileCfg,
    ) {
    }

    /// Evict pass-local inference. The immutable declaration graph is file-owned.
    fn clear_local_cache(&self) {
        if let Some(cache) = &self.lexical {
            cache.clear();
        }
        if let Some(cache) = &self.callback_lexical {
            cache.clear();
        }
        self.locals.borrow_mut().clear();
        self.locals_id.borrow_mut().clear();
        self.root_cause_hints.borrow_mut().clear();
        self.local_callable_heads.borrow_mut().clear();
    }
}
