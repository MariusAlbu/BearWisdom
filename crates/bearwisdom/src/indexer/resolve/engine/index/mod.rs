// =============================================================================
// indexer/resolve/engine/index/mod.rs — SymbolIndex data layout + thread-local cache
//
// The in-memory symbol index that backs `SymbolLookup` for the resolve loop.
// This file owns the struct definition and the per-thread LocalTypeCache used
// for flow-typing. Behavior splits across siblings:
//
//   * build.rs        — index construction (build / build_with_context)
//   * augment.rs      — post-construction additions (augment_from_parsed,
//                       augment_from_db*, take_chain_misses, set_external_paths)
//   * classify.rs     — external-classification queries (is_ambient_path,
//                       resolve_via_external_reexport, classify_external_name)
//   * lookup_impl.rs  — `impl SymbolLookup for SymbolIndex`
// =============================================================================

use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use crate::type_checker::core::types::TypeArena;
use crate::types::AliasTarget;

use super::{ChainMiss, SymbolInfo, TypeInfo};

// Re-export the engine-level free helpers that submodules of this module need.
// Children of `engine/index/` can see engine's private items via the
// visibility rules; this `pub(super) use` makes the imports in each submodule
// one `super::` hop instead of two.
pub(super) use super::{
    common_prefix_len, find_matching_bracket, is_ambient_global_lib_path, is_type_like_kind,
    merge_where_bounds, parse_generic_param_clause, strip_generic_args,
};

mod augment;
mod build;
mod classify;
mod lookup_impl;

// ---------------------------------------------------------------------------
// SymbolIndex — concrete implementation of SymbolLookup
// ---------------------------------------------------------------------------

/// In-memory index of all symbols, built once from parsed data.
pub struct SymbolIndex {
    by_name: FxHashMap<String, Vec<SymbolInfo>>,
    /// BTreeMap gives sorted iteration for free — used by `in_namespace` via `.range()`.
    by_qname: BTreeMap<String, SymbolInfo>,
    by_file: FxHashMap<String, Vec<SymbolInfo>>,
    /// Direct-children index keyed on the parent qualified name.
    /// For a symbol `a.b.c.Foo`, the entry sits under `"a.b.c"`. Top-level
    /// symbols (no dot in qname) are keyed on the empty string.
    ///
    /// Exists so chain walkers can jump to members of a specific type in
    /// O(1) hash + small-vec scan, instead of iterating every symbol that
    /// happens to share a simple name across the whole index (tens of
    /// thousands of candidates once external ecosystems are indexed).
    members_by_parent: FxHashMap<String, Vec<SymbolInfo>>,
    /// Type-kind subset of `by_name` — only entries whose `kind` is in
    /// `TYPE_LIKE_KINDS`. Lets chain walkers' `is-this-a-type?` check hit a
    /// much smaller pool than `by_name` once externals are indexed.
    types_by_name: FxHashMap<String, Vec<SymbolInfo>>,
    /// Unified per-symbol type metadata (replaces 4 separate maps).
    type_info: FxHashMap<String, TypeInfo>,
    /// Re-export map: file_path → Vec<(original_name, source_module)>.
    reexport_map: FxHashMap<String, Vec<(String, String)>>,
    /// Package-level re-export map: package_name → Vec<(name_or_*, target_module)>.
    /// Aggregates `reexport_map` entries by the npm package containing each
    /// file. Used by the external re-export chain resolver to follow names
    /// through `export * from 'pkg'` / `export { X } from 'pkg'` chains
    /// across package boundaries. Wildcard `"*"` entries forward every
    /// name; specific-name entries forward just one.
    pkg_reexports: FxHashMap<String, Vec<(String, String)>>,
    /// Module specifier resolution: maps specifiers to actual file paths.
    /// Populated by ecosystem-specific ModuleResolvers during index construction.
    /// Used for bare/aliased specifiers where source-file context doesn't
    /// affect resolution. Relative specifiers are NOT cached here — they
    /// live in `module_to_file_per_source` because `./utils` from one
    /// directory is a different file than from another.
    module_to_file: FxHashMap<String, String>,
    /// Per-source-file resolution map for relative module specifiers.
    /// Keyed by `(source_file_path, module_specifier)`. Necessary because
    /// `./utils` resolves differently for each consumer file — sharing a
    /// global slot causes the first-resolved consumer to win and silently
    /// breaks resolution for every other file.
    module_to_file_per_source: FxHashMap<(String, String), String>,
    /// Language-intrinsic keywords keyed by language id → set of names.
    /// Built once from `keywords::keywords_set_for_language` for all languages
    /// present in the parsed files.
    primitives_by_language: FxHashMap<String, HashSet<&'static str>>,
    /// All symbols grouped by their owning workspace `package_id`. Empty for
    /// single-project layouts or for files whose `ParsedFile::package_id`
    /// is None. Used by language resolvers to scope cross-package import
    /// lookups.
    by_package: FxHashMap<i64, Vec<SymbolInfo>>,
    /// Snapshot of `ProjectContext::workspace_pkg_by_declared_name` taken at
    /// build time — lets `SymbolLookup::workspace_package_id` stand alone
    /// without holding a borrow on the project context.
    workspace_pkg_by_declared_name: FxHashMap<String, i64>,
    /// Per-package path aliases (tsconfig `paths`, jsconfig, framework
    /// configs), snapshot of the NPM manifest's `path_aliases` for each
    /// workspace package. Empty for single-project
    /// layouts — callers fall back to the union.
    path_aliases_by_pkg: FxHashMap<i64, Vec<(String, String)>>,
    /// Project-wide union of path-alias entries; used when no
    /// `package_id` is set or the package has no per-package entry.
    path_aliases_union: Vec<(String, String)>,
    /// Set of package names (as listed in `tsconfig.json`'s
    /// `compilerOptions.types` array) whose external file paths are
    /// treated as ambient-global providers — symbols from these
    /// packages are auto-loaded by TypeScript without an explicit
    /// `import` statement, so the resolver should prefer them when
    /// disambiguating bare-name refs (`expect`, `describe`, `process`,
    /// etc.). Each entry is the raw value as listed (`"vitest/globals"`,
    /// `"node"`, `"@types/jest"`). Used by `is_ambient_path` to test
    /// whether a candidate's external file path falls under any
    /// listed package.
    tsconfig_types_union: Vec<String>,
    /// Class inheritance map: child class qualified_name → parent class qualified_name.
    /// Built from `Inherits` refs at index construction time.  Used by language
    /// resolvers to walk the ancestor chain when `$this->method()` calls cannot
    /// be resolved within the immediate class scope.
    ///
    /// Keyed by child qname (dotted form), value is the direct parent qname.
    /// Transitive ancestors are reached by chaining lookups.
    inherits_map: FxHashMap<String, String>,
    /// Structural shape of every TypeAlias symbol in the project.
    /// Indexed by both qualified name AND simple name so chain walkers can
    /// look up an alias whether or not the encountered name carries its
    /// scope prefix. Populated from `ParsedFile::alias_targets` (which TS
    /// emits) plus a derived `Application` fallback for languages that
    /// only provide a single TypeRef per typedef.
    alias_target: FxHashMap<String, AliasTarget>,
    /// All symbols sharing a qualified name, keyed by qname. Backs
    /// `SymbolLookup::all_by_qualified_name` so callers that care about
    /// kind compatibility can scan past the first-wins match in `by_qname`.
    ///
    /// Only populated for qnames with > 1 symbol — the common case (one
    /// symbol per qname) reads through `by_qname` as before.
    qname_duplicates: FxHashMap<String, Vec<SymbolInfo>>,
    /// Names of methods/properties declared inside ambient-global
    /// declaration files (`lib.dom.d.ts`, `lib.es5.d.ts`, `lib.webworker.d.ts`,
    /// `@types/node/*`). Used as the last-resort external signal for
    /// untyped chain calls: `x.addEventListener(...)` with unknown `x`
    /// still classifies as external because `addEventListener` only lives
    /// on EventTarget in lib.dom.d.ts. Replaces the hardcoded
    /// `is_common_builtin_method` list.
    ambient_global_method_names: HashSet<String>,
    /// File paths explicitly marked `origin='external'` in the DB but stored
    /// without an `ext:` prefix — i.e. script-tag-discovered vendor JS like
    /// `wwwroot/lib/jquery.min.js`. Chain walkers consult this set (via
    /// `SymbolLookup::is_external_file`) so a vendored `$` declaration
    /// doesn't masquerade as project code when classifying jQuery chains.
    ///
    /// Empty when the caller didn't supply a set (tests, synthetic lookups),
    /// in which case the existing `ext:` prefix check is the sole authority.
    external_paths: HashSet<String>,
    /// Project-wide Angular component selector map: raw selector string →
    /// component class qualified name.
    ///
    /// Populated in `build_with_context` from `ParsedFile::component_selectors`
    /// (which the full-index pipeline fills from `@Component({selector:'...'}`
    /// decoration metadata in TypeScript sources).
    ///
    /// Element selectors: `"app-user-card"` → `"src/app/user-card.UserCardComponent"`
    /// Attribute selectors: `"appHighlight"` → `"src/directives.HighlightDirective"`
    ///
    /// Empty for non-Angular projects. Exposed via `SymbolLookup::angular_selector`.
    angular_selectors: FxHashMap<String, String>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
    /// Interior-mutable accumulator for chain walker bail-outs.
    /// `record_chain_miss` pushes; `take_chain_misses` drains.
    ///
    /// `Mutex` is needed because `SymbolIndex` is shared by `&` across
    /// rayon workers in the parallel resolve loop. Contention is bounded —
    /// chain misses are a small fraction of resolves, and locking is fast
    /// compared to the SQL writes the workers are also doing.
    chain_misses: std::sync::Mutex<Vec<ChainMiss>>,
    /// Workspace-wide TypeArena. Holds the canonical `Type` interpretation
    /// of every type expression referenced from any indexed symbol. Used
    /// by chain walkers and language resolvers to switch from string-keyed
    /// type lookups to TypeId-keyed ones. Interior mutability via the
    /// arena's internal RwLock — safe to share via `&self`. Shared via
    /// `Arc` so the same arena flows from the indexer entry point through
    /// extractors and into `SymbolIndex` without per-stage rebuilds.
    pub(crate) type_arena: Arc<TypeArena>,
}

impl SymbolIndex {
    /// Hand out a fresh `Arc` to the workspace TypeArena so consumers (the
    /// resolver's type-checker Engine, language plugins that opt into
    /// arena-aware extraction during augmentation, etc.) can share the
    /// same canonical arena as the index itself.
    pub fn type_arena_arc(&self) -> Arc<TypeArena> {
        Arc::clone(&self.type_arena)
    }
}

// Per-worker forward-inference cache for local variables (R5). Each rayon
// worker gets its own thread-local copy, so parallel resolve workers don't
// trample each other's per-file scopes. Reset at the start of each file's
// resolution by `install_local_cache`; reusing the same TLS across files
// (rayon worker threads are persistent) is safe because of that reset.
thread_local! {
    pub(crate) static LOCAL_TYPE_CACHE: RefCell<LocalTypeCache> = RefCell::new(LocalTypeCache::default());
}

// ---------------------------------------------------------------------------
// LocalTypeCache — per-file flow-typing cache
// ---------------------------------------------------------------------------

/// Per-file local-variable type cache.
///
/// Populated by the resolver loop: each time a flow-binding ref (RHS of
/// `<lhs> = <expr>`) resolves with a non-`None` `resolved_yield_type`,
/// the LHS name is recorded here. Subsequent chain walkers for the same
/// file consult this cache first in Phase 1 so a same-named global
/// doesn't shadow the local's inferred type.
///
/// Three sources of truth, in priority order:
///   1. Conditional narrowings — `if (x is Bar) { x.barMethod(); }`.
///      The cursor is set by the resolver to the current ref's byte
///      offset; whichever narrowing's `[byte_start, byte_end)` contains
///      the cursor wins. Overlapping ranges are resolved innermost-first
///      (vec is pre-sorted by descending specificity).
///   2. Forward inference — `let x = foo(); x.something()`.
///      Reassignment overwrites the earlier binding (last write wins,
///      which is safe because refs are resolved in source order).
///   3. Miss — returns `None`; chain walker falls back to existing logic.
pub struct LocalTypeCache {
    /// Forward-propagated types: name → most recent inferred type.
    /// Reassignment simply overwrites (resolver walks refs in line order).
    forward: FxHashMap<String, String>,
    /// Conditional-narrowing scopes, pre-sorted innermost-first so the
    /// first matching entry wins naturally.
    narrowings: Vec<crate::types::Narrowing>,
    /// Discriminated-union guard scopes (`if (x.kind === "circle")`). Kept
    /// separate from `narrowings` because the narrowed type is a union branch
    /// the chain walker resolves at lookup time, not a fixed type name.
    discriminants: Vec<crate::types::DiscriminantNarrowing>,
    /// Current ref's byte position. Set by the resolver before each
    /// chain-walker call via `SymbolLookup::set_cursor`.
    cursor: u32,
}

impl Default for LocalTypeCache {
    fn default() -> Self {
        Self {
            forward: FxHashMap::default(),
            narrowings: Vec::new(),
            discriminants: Vec::new(),
            cursor: 0,
        }
    }
}

impl LocalTypeCache {
    /// Look up the active type for `name` at the current cursor position.
    /// Narrowings take precedence — innermost (smallest) wins because the
    /// `narrowings` vec is pre-sorted by ascending range size.
    pub fn lookup(&self, name: &str) -> Option<&str> {
        for n in &self.narrowings {
            if n.name == name
                && n.byte_start <= self.cursor
                && self.cursor < n.byte_end
            {
                return Some(&n.narrowed_type);
            }
        }
        self.forward.get(name).map(|s| s.as_str())
    }

    /// The active discriminant guard for `name` at the cursor — `(prop,
    /// literal)`. The chain walker uses it to pick a union branch when the
    /// receiver resolves to a `Type::Union`.
    pub fn discriminant(&self, name: &str) -> Option<(&str, &str)> {
        for d in &self.discriminants {
            if d.name == name && d.byte_start <= self.cursor && self.cursor < d.byte_end {
                return Some((d.prop.as_str(), d.literal.as_str()));
            }
        }
        None
    }
}
