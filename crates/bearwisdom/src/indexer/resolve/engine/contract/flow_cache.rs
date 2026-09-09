// =============================================================================
// contract/flow_cache — per-file flow-typing cache surface
//
// The resolver calls `install_local_cache` at the start of each file, moves
// `set_cursor` before resolving each ref, and calls `record_local_type` after
// a resolve succeeds with a yield type. Chain walkers consult `local_type`
// first in Phase 1 so a local variable's inferred type takes precedence over
// same-named globals.
//
// Split from the structural `SymbolLookup` surface (its supertrait): the
// structural methods answer "what does the index contain", these answer
// "what has this file's ref loop learned so far". All methods default to
// no-ops — synthetic test lookups and non-caching impls don't have to opt in.
// =============================================================================

use crate::type_checker::core::types::TypeId;

/// A selected static method declaration and its numeric substitution evidence.
#[derive(Clone)]
pub struct BoundMethod {
    pub(crate) declaration: i64,
    pub(crate) receiver: TypeId,
    pub(crate) adjusted: TypeId,
    pub(crate) bindings:
        rustc_hash::FxHashMap<crate::type_checker::core::types::GenericParamId, TypeId>,
}

/// Fully qualified call evidence; receiver arguments are not ordinary parameters.
pub struct BoundCall {
    pub(crate) declaration: i64,
    pub(crate) return_type: Option<TypeId>,
    pub(crate) parameters: Vec<TypeId>,
    pub(crate) receiver_arguments: usize,
}

/// A complete source-owned overload group, distinct from its selected signature.
pub struct OverloadCall {
    pub(crate) origins: Vec<CallSignatureOrigin>,
    pub(crate) selected: usize,
    pub(crate) return_type: TypeId,
    pub(crate) parameters: Vec<TypeId>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CallSignatureOrigin {
    pub source: super::super::program_graph::SourceInstanceId,
    pub span: crate::types::SourceSpan,
    pub declaration: Option<i64>,
}

/// A source occurrence already bound at ingestion. Missing persisted identity
/// is not permission to fall through to a spelling-based rule.
#[derive(Clone)]
pub struct LocalReference {
    pub declaration: Option<i64>,
    pub kind: crate::types::SymbolKind,
    pub value_type: Option<TypeId>,
    pub type_args: Vec<TypeId>,
    /// Independently captured callable provenance; never the navigation target.
    pub callable: Option<i64>,
}

pub struct ObjectMember {
    pub(crate) declaration: Option<i64>,
    pub(crate) value: TypeId,
}

pub trait FlowCacheLookup {
    /// Outer None keeps legacy expansion. Some(None) is a configured proof failure.
    fn evaluated_receiver(&self, _receiver: TypeId) -> Option<Option<TypeId>> {
        None
    }
    fn source_object_member(
        &self,
        _receiver: TypeId,
        _selector: u32,
    ) -> Option<Result<ObjectMember, ()>> {
        None
    }
    fn object_member_type(&self, _receiver: TypeId, _member: i64) -> Option<Option<TypeId>> {
        None
    }
    /// None: not an overloaded source method. Some(Err): authoritative barrier.
    fn overloaded_call(
        &self,
        _receiver: TypeId,
        _selector: u32,
        _actual: &[TypeId],
        _explicit: &[TypeId],
    ) -> Option<Result<OverloadCall, ()>> {
        None
    }
    /// Proved receiver-owned signature, separate from shared navigation-row types.
    fn receiver_member_info(&self, _owner: i64, _member: i64) -> Option<&super::TypeInfo> {
        None
    }
    /// Bound apparent object type for an intrinsic member receiver; no name fallback.
    fn intrinsic_member_type(
        &self,
        _kind: crate::type_checker::core::types::Intrinsic,
    ) -> Option<TypeId> {
        None
    }
    /// Selected configured-program identity. Workspace-only environments cannot
    /// interpret nominal types owned by a program-specific view.
    fn nominal_context(&self) -> Option<crate::type_checker::core::types::NominalContextId> {
        None
    }
    /// Outer None is unconfigured. Some(None) is an authoritative global miss.
    fn source_global_type(&self, _name: crate::indexer::lexical::NameId) -> Option<Option<i64>> {
        None
    }
    /// Source-signature owner within this exact configured source snapshot.
    fn source_signature_parameter(
        &self,
        _owner: crate::types::SourceSpan,
        _index: usize,
    ) -> Option<crate::type_checker::core::types::GenericParamId> {
        None
    }
    fn source_callable_origin(
        &self,
        _owner: crate::types::SourceSpan,
    ) -> Option<crate::type_checker::core::types::CallableOrigin> {
        None
    }
    fn source_unique_symbol(&self, _declaration: crate::types::SourceSpan) -> Option<TypeId> {
        None
    }
    fn source_value_type(&self, _site: crate::types::SourceSpan) -> Option<Option<TypeId>> {
        None
    }
    /// Source initializer signature and its exact local declaration token.
    /// Some(None) is an authoritative configured-source miss.
    fn source_initializer_type(
        &self,
        _owner: crate::types::SourceSpan,
        _target: crate::types::SourceSpan,
    ) -> Option<Option<TypeId>> {
        None
    }
    /// Exact source recipe shared by arguments and local initializer evaluation.
    fn value_expression(&self, _span: crate::types::SourceSpan) -> Option<TypeId> {
        None
    }
    /// None: unmigrated source. Some(Err): captured source cannot attest this call.
    /// An attested zero-argument call is Some(Ok(&[])), never a display fallback.
    fn source_call_arguments(
        &self,
        _selector: u32,
    ) -> Option<Result<&[crate::types::CallArg], ()>> {
        None
    }
    /// Attested explicit borrow plus the operand's bound type; never a dot-call adjustment.
    fn borrow_argument(&self, _span: crate::types::SourceSpan, _operand: TypeId) -> Option<TypeId> {
        None
    }
    /// Numeric source-kind guard before typing operands for a qualified call.
    fn qualified_call_site(&self, _selector: u32) -> bool {
        false
    }
    /// Source selector plus typed operands; no declaration/member spelling bridge.
    fn qualified_call(
        &self,
        _selector: u32,
        _actual: &[TypeId],
        _explicit: &[TypeId],
    ) -> Option<Result<BoundCall, ()>> {
        None
    }
    /// None is an unmigrated source; Some(Err) is authoritative, not a name fallback.
    fn bound_method(&self, _receiver: TypeId, _selector: u32) -> Option<Result<BoundMethod, ()>> {
        None
    }
    /// Source-owned non-call selectors. Some(Err) forbids display-name recovery.
    fn source_member_name(
        &self,
        _selector: u32,
    ) -> Option<Result<super::super::member_index::MemberNameId, ()>> {
        None
    }
    /// Private selectors attest a lexical declaration, never a receiver name.
    fn source_private_member(&self, _selector: u32) -> Option<Result<i64, ()>> {
        None
    }
    /// A CST-attested dot-call inference region; absent for legacy or missing owners.
    fn method_call_region(&self, _selector: u32) -> Option<TypeId> {
        None
    }
    /// An inherent member declared on a particular alias/application.
    fn member_pattern(
        &self,
        _member: i64,
    ) -> Option<&super::member_applicability::ReceiverPattern> {
        None
    }
    /// ID-addressed access from the active source module. Unconfigured profiles
    /// retain their existing contract; captured private/unknown scopes do not.
    fn declaration_accessible(&self, _declaration: i64) -> bool {
        true
    }
    /// An identifier argument's own use address, independent of the call cursor.
    fn argument_reference(&self, _span: crate::types::SourceSpan) -> Option<LocalReference> {
        None
    }
    /// Member selector address, deliberately distinct from the lexical root address.
    fn member_type_arguments(&self, _selector: u32) -> Option<&[TypeId]> {
        None
    }
    fn local_reference(&self, _byte: u32) -> Option<LocalReference> {
        None
    }
    /// Captured namespace export selector, not a member-name lookup.
    fn namespace_member(&self, _selector: u32) -> Option<LocalReference> {
        None
    }
    /// Namespace facet of a source-bound entity, independent of its callable kind.
    fn namespace_root(&self, _byte: u32) -> bool {
        false
    }
    /// Ingestion address -> BindingId -> TypeId. Never guess a callback's
    /// declaration from a parameter spelling at the caller's cursor.
    fn record_contextual_type(&self, _parameter: crate::types::SourceSpan, _ty: TypeId) {}
    /// A known lexical declaration must not fall through to unrelated globals
    /// merely because its type has not been captured or initialized.
    fn has_local_binding(&self, _name: &str) -> bool {
        false
    }
    fn local_callable_id(&self, _name: &str) -> Option<i64> {
        None
    }
    /// Look up the inferred type of a local variable in the currently-active
    /// file scope. Honors active conditional narrowings via the cursor set by
    /// `set_cursor`. Returns `None` when the name is not tracked.
    fn local_type(&self, _name: &str) -> Option<String> {
        None
    }

    /// Multi-branch variant: when the CFG has narrowed `name` to a `Union`,
    /// each branch is a separate entry; for the common `Single` case it is
    /// a one-element vec. Consumers that can dispatch across union members
    /// (the type-arena chain walker) call this instead of `local_type`.
    /// The default impl delegates to `local_type` for trait implementors
    /// that do not yet expose CFG facts.
    fn local_type_union(&self, name: &str) -> Option<Vec<String>> {
        self.local_type(name).map(|s| vec![s])
    }

    /// The active discriminated-union guard for `name` — `(prop, literal)` —
    /// at the current cursor. The chain walker uses it to select a union branch.
    fn local_discriminant(&self, _name: &str) -> Option<(String, String, bool)> {
        None
    }

    /// Install a fresh local-type cache for the next file's resolution pass.
    /// `narrowings` should be pre-sorted innermost-first (smallest range first).
    fn install_local_cache(
        &self,
        _narrowings: Vec<crate::types::Narrowing>,
        _discriminants: Vec<crate::types::DiscriminantNarrowing>,
        _cfg: crate::indexer::flow_cfg::FileCfg,
    ) {
    }

    /// Move the cache cursor to the given byte offset. The resolver calls
    /// this before each ref so narrowing lookups see the correct byte range.
    fn set_cursor(&self, _byte: u32) {}

    /// Record a successfully-inferred local-variable type. Called by the
    /// resolver after a flow-binding ref resolves with a non-`None`
    /// `resolved_yield_type`.
    fn record_local_type(&self, _name: String, _type_name: String) {}

    /// Canonical TypeId form of `local_type`. Returns the TypeId stored by
    /// `record_local_type_id` for `name`, or `None` when no TypeId binding
    /// exists. Preferred over `local_type` by the chain walker's root step so
    /// non-nominal types (primitives, optionals, generics) survive the cache
    /// round-trip without being nominalized to `Class`.
    fn local_type_id(&self, _name: &str) -> Option<TypeId> {
        None
    }

    /// Store the canonical TypeId for a local binding directly, avoiding the
    /// `format_type` → `intern_type_str` round-trip that nominalizes
    /// `Primitive`/`Optional`/`Generic` to `Class`.
    fn record_local_type_id(&self, _name: String, _id: TypeId) {}

    /// The qualified name of the declaration a local binding's value points
    /// at, when the binding's own field/return type was never captured (a
    /// destructured `$Ret`-synthesized member — see
    /// `chain::callable_member_qname_on`). Consulted only by the bare-name-call
    /// rule (`LocalFlowHeadRule`), never by the chain walker's root step —
    /// unlike `local_type_id`, this name-only pointer never re-roots a chain
    /// that continues past the binding onto a member-less leaf.
    fn local_callable_head(&self, _name: &str) -> Option<String> {
        None
    }

    /// Record the qualified name `local_callable_head` reads back for `name`.
    fn record_local_callable_head(&self, _name: String, _qname: String) {}

    /// Clear the cache at end of file. Keeps leftover bindings from bleeding
    /// into the next file's pass.
    fn clear_local_cache(&self) {}

    /// Record why a local binding's forward-inferred type could not be
    /// seeded — e.g. `const x = f()` where `f` resolved but carries no
    /// captured return type. A later ref rooted on `x` reads this via
    /// `root_cause_hint` instead of re-deriving why `x` is untyped; it names
    /// the true upstream cause (`f`, not `x`) rather than the symptom.
    /// Default no-op — synthetic test lookups don't have to opt in.
    fn record_root_cause_hint(
        &self,
        _name: String,
        _cause: crate::indexer::resolve::engine::cause::Cause,
    ) {
    }

    /// The cause recorded for `name` by `record_root_cause_hint`, when the
    /// forward-inference seed for this binding failed and recorded one.
    /// Default `None`.
    fn root_cause_hint(
        &self,
        _name: &str,
    ) -> Option<crate::indexer::resolve::engine::cause::Cause> {
        None
    }
}
