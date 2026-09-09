// =============================================================================
// indexer/resolve/engine/types.rs — public data contracts of the resolver engine
//
// Pure data: the structs that flow across module boundaries between the resolve
// loop, language resolvers, and the symbol index. No behavior lives here — the
// SymbolLookup trait, the SymbolIndex impl, the chain walker, and the
// ResolutionEngine registry all stay in engine.rs.
// =============================================================================

use crate::type_checker::core::types::{GenericParamId, TypeId};
use crate::types::{ExtractedRef, ExtractedSymbol};
use std::sync::Arc;

use super::SymbolLookup;

// ---------------------------------------------------------------------------
// Public types used by LanguageResolver implementations
// ---------------------------------------------------------------------------

/// Normalized import entry, built from ExtractedRef data.
#[derive(Debug, Clone)]
pub struct ImportEntry {
    /// The module's own declared name for the imported symbol (e.g.,
    /// "CatalogItem", "Foo") — the name its declaring files carry.
    pub imported_name: String,
    /// The module/namespace path (e.g., "eShop.Catalog.API.Model", "./foo").
    pub module_path: Option<String>,
    /// Optional alias (e.g., `import { Foo as Bar }` → alias = "Bar").
    pub alias: Option<String>,
    /// Whether this is a wildcard/namespace import (e.g., `using NS;`).
    pub is_wildcard: bool,
}

impl ImportEntry {
    /// The name this import BINDS in the importing file: the alias when
    /// renamed (`use m::Orig as Bound` brings only `Bound` into scope),
    /// else the imported name. A rename's ORIGINAL name is not in scope —
    /// a rule asking "does an import bind `target`?" must compare this,
    /// while a rule looking the declaration up inside the module keys on
    /// `imported_name`.
    pub fn bound_name(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.imported_name)
    }
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
    /// The source symbol's db row id, when the caller has bound it. Lets
    /// enclosing-scope lookups key on identity instead of the source qname
    /// (which a same-named declaration in another package shares).
    pub source_symbol_id: Option<i64>,
}

/// The confidence every name resolution carries. SymbolInfo is binary — a
/// structural bind succeeds (the import root matched, the qname hit, the
/// inheritance climb reached the member, the `this T` signature matched) or it
/// declines to `None`. There is no graded middle: an ambiguous bind returns
/// `None` rather than a fractional score. Distinct from the reachability
/// trust-band (`DISPATCH_CANDIDATE_CONFIDENCE` = 0.6), which marks speculative
/// fan-out edges the BFS may traverse-but-doubt.
pub const RESOLVED_CONFIDENCE: f64 = 1.0;

/// The result of a successful resolution.
#[derive(Debug)]
pub struct SymbolInfo {
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
pub struct Symbol {
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

// ---------------------------------------------------------------------------
// TypeInfo — unified per-symbol type metadata
// ---------------------------------------------------------------------------

/// Type metadata for one symbol. Source-bound semantics use the canonical
/// declaration-ID map; the name-keyed map remains for unmigrated legacy paths.
///
/// TypeIds are the canonical source of truth: the build pipeline populates
/// `field_type_id` / `return_type_id` first from extractor signals (TypeRef
/// refs, signature parsing, AST-driven extractors). `generic_param_ids` carries
/// interned `GenericParamData` (name + optional bound) per parameter;
/// `generic_param_default_ids` carries interned default TypeIds index-aligned
/// with `generic_param_ids`. Canonical snapshots retain these IDs with the arena;
/// the DB `generic_params` name column is only legacy compatibility metadata.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct TypeInfo {
    /// Source-bound direct base; Some(Unknown) fences missing/unsupported heads.
    #[serde(default)]
    pub base_type_id: Option<TypeId>,
    /// Trait Self is a separate binder, never an explicit generic argument slot.
    #[serde(default)]
    pub trait_self_param: Option<GenericParamId>,
    /// Anonymous input regions: (source byte, omitted-slot index, parameter ID).
    /// Owned by this declaration, separate from explicit generic argument order.
    #[serde(default)]
    pub elided_input_params: Vec<(u32, usize, GenericParamId)>,
    /// Interned generic parameter slots for type declarations, e.g., two
    /// entries for `interface Repository<T, U>`. Each `GenericParamId`
    /// carries the parameter name and optional upper bound in the TypeArena.
    pub generic_param_ids: Vec<GenericParamId>,
    /// Declared defaults for `generic_param_ids`, index-aligned. `None` for
    /// a parameter with no default; `Some(id)` for `<T = string>` or
    /// `<U = T>` where the default type is interned in the workspace arena.
    pub generic_param_default_ids: Vec<Option<TypeId>>,
    /// Canonical TypeId form of `field_type`.
    pub field_type_id: Option<TypeId>,
    /// Canonical TypeId form of `return_type`.
    pub return_type_id: Option<TypeId>,
    /// Ingestion-bound template; runtime substitution uses GenericParamId only.
    pub generic_return: Option<super::generic_return::GenericReturn>,
    pub lexical_alias: Option<super::generic_return::GenericReturn>,
    /// Source-bound parameter types, in declaration order. None is legacy input.
    #[serde(default)]
    pub parameter_type_ids: Option<Vec<TypeId>>,
    /// Source-bound receiver, separate from ordinary argument positions.
    #[serde(default)]
    pub receiver_type_id: Option<TypeId>,
}
