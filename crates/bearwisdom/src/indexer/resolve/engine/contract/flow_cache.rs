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

pub trait FlowCacheLookup {
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
    fn root_cause_hint(&self, _name: &str) -> Option<crate::indexer::resolve::engine::cause::Cause> {
        None
    }
}
