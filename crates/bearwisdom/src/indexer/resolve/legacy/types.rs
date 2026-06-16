// =============================================================================
// indexer/resolve/engine/types.rs — public data contracts of the resolver engine
//
// Pure data: the structs that flow across module boundaries between the resolve
// loop, language resolvers, and the symbol index. No behavior lives here — the
// SymbolLookup trait, the SymbolIndex impl, the chain walker, and the
// ResolutionEngine registry all stay in engine.rs.
// =============================================================================

use crate::type_checker::core::types::TypeId;
use crate::types::{ExtractedRef, ExtractedSymbol};
use std::sync::Arc;

use super::SymbolLookup;

// ---------------------------------------------------------------------------
// Public types used by LanguageResolver implementations
// ---------------------------------------------------------------------------

/// Normalized import entry, built from ExtractedRef data.
#[derive(Debug, Clone)]
pub struct ImportEntry {
    /// The name brought into scope (e.g., "CatalogItem", "Foo").
    pub imported_name: String,
    /// The module/namespace path (e.g., "eShop.Catalog.API.Model", "./foo").
    pub module_path: Option<String>,
    /// Optional alias (e.g., `import { Foo as Bar }` → alias = "Bar").
    pub alias: Option<String>,
    /// Whether this is a wildcard/namespace import (e.g., `using NS;`).
    pub is_wildcard: bool,
}

/// Context for the file being resolved. Built once per file by the resolver.
#[derive(Debug, Clone)]
pub struct FileContext {
    /// The file path (relative to project root).
    pub file_path: String,
    /// The language identifier.
    pub language: String,
    /// Imports in this file.
    pub imports: Vec<ImportEntry>,
    /// The namespace/package this file belongs to.
    pub file_namespace: Option<String>,
}

/// Context for a single reference being resolved.
pub struct RefContext<'a> {
    /// The reference itself.
    pub extracted_ref: &'a ExtractedRef,
    /// The source symbol that contains this reference.
    pub source_symbol: &'a ExtractedSymbol,
    /// The scope chain at the reference site, innermost first.
    /// Built from scope_path: e.g., ["Foo.Bar.Baz", "Foo.Bar", "Foo"].
    pub scope_chain: Vec<String>,
    /// The source file's workspace package id (from `ParsedFile::package_id`).
    /// Used by the external-classification path to scope manifest lookups
    /// to the package that declared the dep — prevents `server/` from
    /// reaching `e2e/`'s devDependencies in a pnpm monorepo.
    pub file_package_id: Option<i64>,
}

/// The confidence every name resolution carries. Resolution is binary — a
/// structural bind succeeds (the import root matched, the qname hit, the
/// inheritance climb reached the member, the `this T` signature matched) or it
/// declines to `None`. There is no graded middle: an ambiguous bind returns
/// `None` rather than a fractional score. Distinct from the reachability
/// trust-band (`DISPATCH_CANDIDATE_CONFIDENCE` = 0.6), which marks speculative
/// fan-out edges the BFS may traverse-but-doubt.
pub const RESOLVED_CONFIDENCE: f64 = 1.0;

/// The result of a successful resolution.
#[derive(Debug)]
pub struct Resolution {
    /// The DB ID of the resolved target symbol.
    pub target_symbol_id: i64,
    /// Always `RESOLVED_CONFIDENCE`. A resolution either binds structurally or
    /// returns `None`; there is no graded score.
    pub confidence: f64,
    /// Which strategy produced this resolution (for diagnostics).
    pub strategy: &'static str,
    /// For chain refs and call refs: the type the resolved target *yields*
    /// (return type for methods, declared type for fields/variables) as a
    /// canonical TypeId in the workspace arena. `None` when not applicable
    /// or when the producer can't intern. Consumed by the resolver loop
    /// to populate the per-file local-type cache for forward flow
    /// inference — see `LocalTypeCache` and `SymbolLookup::record_local_type`.
    pub resolved_yield_type: Option<TypeId>,
    /// Optionally emitted when the resolved ref's shape matches a cross-tier
    /// flow-edge pattern. Accumulated by the resolve loop and bulk-written to
    /// `flow_edges` after the main edge transaction commits.
    ///
    /// `None` for the vast majority of refs. Opt-in per resolver.
    pub flow_emit: Option<crate::indexer::resolve::flow_emit::FlowEmission>,
}

/// Flattened symbol info used during resolution lookups.
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub id: i64,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub visibility: Option<String>,
    pub file_path: Arc<str>,
    pub scope_path: Option<String>,
    /// The package this symbol belongs to, if the project is a monorepo.
    /// Derived from `ParsedFile::package_id` at index build time,
    /// or from the `files.package_id` column when augmenting from DB.
    pub package_id: Option<i64>,
    /// Symbol signature when available — function arrow type, parameter
    /// type annotation, etc. Populated for symbols whose downstream
    /// resolution depends on inspecting the type (e.g. callable variables
    /// with `impl Fn(...)` signatures bound at function-parameter scope).
    /// `None` for symbols where the signature is unhelpful or absent
    /// (struct fields, enum variants, etc.).
    pub signature: Option<String>,
}

/// Intern a yield-type qname into the lookup's TypeArena. Returns `None`
/// when either the input qname or the arena is absent. Provides the
/// migration path for legacy `LanguageResolver`/`TypeChecker` impls that
/// still produce yield types as strings — they wrap their string output
/// in this helper to populate `Resolution::resolved_yield_type` with a
/// canonical TypeId until they're rewritten to produce TypeId directly.
pub fn intern_yield_type(qname: Option<String>, lookup: &dyn SymbolLookup) -> Option<TypeId> {
    let qname = qname?;
    let arena = lookup.type_arena()?;
    Some(arena.class(&qname))
}

// ---------------------------------------------------------------------------
// TypeInfo — unified per-symbol type metadata
// ---------------------------------------------------------------------------

/// All type metadata for a single symbol, stored in a single map keyed by
/// the symbol's qualified name (or simple name for generic_params).
///
/// TypeIds are the canonical source of truth: the build pipeline populates
/// `field_type_id` / `return_type_id` / `type_arg_ids` first from extractor
/// signals (TypeRef refs, signature parsing, AST-driven extractors), and
/// the string fields below are formatted from those TypeIds for the
/// legacy string-typed `SymbolLookup` accessors. Removing the parallel
/// string-population path closes the gap that used to let strings and
/// TypeIds drift out of sync.
#[derive(Debug, Default, Clone)]
pub struct TypeInfo {
    /// Field/property type rendered from `field_type_id`.
    pub field_type: Option<String>,
    /// Generic type arguments of `field_type` (e.g. `["User"]` for a field
    /// `Repository<User>`). Kept distinct from `return_type_args` so a symbol
    /// that is both a field and a method of the same qname (legal in Java:
    /// `List<X> size; List<Y> size()`) does not clobber one with the other.
    pub type_args: Vec<String>,
    /// Method return type rendered from `return_type_id`.
    pub return_type: Option<String>,
    /// Generic type arguments of `return_type` (e.g. `["User"]` for a method
    /// returning `Repository<User>`). Consulted by the chain walker on a
    /// method-call yield to bind the element type parameter.
    pub return_type_args: Vec<String>,
    /// Generic parameter names for type declarations (e.g., ["T"] for `interface Repository<T>`).
    pub generic_params: Vec<String>,
    /// Declared upper bounds for `generic_params`, index-aligned. `None` for an
    /// unbounded parameter; `Some("Animal")` for `<T extends Animal>` / `<T: Animal>`.
    /// Resolved to `GenericParamData.bound` when the param's `Type::Generic` is interned.
    pub generic_param_bounds: Vec<Option<String>>,
    /// Canonical TypeId form of `field_type`.
    pub field_type_id: Option<TypeId>,
    /// Canonical TypeId form of `return_type`.
    pub return_type_id: Option<TypeId>,
    /// Canonical TypeIds of `type_args`, in declaration order.
    pub type_arg_ids: Vec<TypeId>,
    /// Canonical TypeIds of `return_type_args`, in declaration order. Kept in
    /// sync with the string form through the same intern/format round-trip as
    /// `type_arg_ids`, so a method-return arg canonicalizes the same way a
    /// field arg does.
    pub return_type_arg_ids: Vec<TypeId>,
    /// Canonical TypeIds of declared generic parameters — each one a
    /// `Type::Generic { param }` interned through the workspace arena.
    /// Populated alongside `generic_params` so consumers that drive
    /// substitution can resolve `T` / `K` / `V` symbols by id instead of
    /// by name. `owner_symbol_index` is currently a placeholder (0); a
    /// future wave will wire real owner indices from the extractor.
    pub generic_param_type_ids: Vec<TypeId>,
}
