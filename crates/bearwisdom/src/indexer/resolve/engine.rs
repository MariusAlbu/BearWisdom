// =============================================================================
// indexer/resolve/engine.rs — Resolution engine with per-language rule plugins
//
// The engine dispatches reference resolution to language-specific resolvers
// that apply deterministic scope rules (1.0 confidence). When no language
// resolver can resolve a reference, it falls back to the heuristic resolver.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::type_env::TypeEnvironment;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Bracket-matching utility
// ---------------------------------------------------------------------------

/// Count the number of leading dotted-path segments shared between two qualified
/// names (e.g. `"App.Services"` and `"App.Services.Foo"` → 2).  Used to pick
/// the best parent-class candidate when multiple symbols share the same short name.
/// Kinds a chain walker's `is-this-a-type?` probe cares about.
/// Superset across all languages — individual language configs' own
/// `static_type_kinds` still filter to the subset they consider valid as a
/// static-access root.
///
/// Most entries come straight from `SymbolKind::as_str` (strum snake_case);
/// the extras (`trait`, `protocol`, `object`, `mixin`, `extension`,
/// `record`) are the strings language configs reference in
/// `static_type_kinds`/`enclosing_type_kinds`, kept here so the pre-filter
/// stays a superset even when extractors start emitting those strings
/// directly (via `kind_str` overrides in future language plugins).
fn is_type_like_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "struct"
            | "interface"
            | "enum"
            | "type_alias"
            | "namespace"
            | "record"
            | "trait"
            | "protocol"
            | "object"
            | "mixin"
            | "extension"
    )
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    a.split('.').zip(b.split('.')).take_while(|(x, y)| x == y).count()
}

/// Find the index of the closing bracket that matches the first opening bracket
/// in `s`.  Uses depth counting so nested brackets are handled correctly.
///
/// Example: `find_matching_bracket("Map<K, List<V>>", '<', '>')` → `Some(14)`.
/// `s.find('>')` would incorrectly return `Some(11)` for this input.
fn find_matching_bracket(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

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

/// The result of a successful resolution.
#[derive(Debug)]
pub struct Resolution {
    /// The DB ID of the resolved target symbol.
    pub target_symbol_id: i64,
    /// Confidence level (1.0 for deterministic, lower for heuristic).
    pub confidence: f64,
    /// Which strategy produced this resolution (for diagnostics).
    pub strategy: &'static str,
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
}

// ---------------------------------------------------------------------------
// SymbolLookup trait — decouples resolvers from index internals
// ---------------------------------------------------------------------------

/// Read-only access to the global symbol index.
pub trait SymbolLookup {
    /// Find all symbols with the given simple name.
    fn by_name(&self, name: &str) -> &[SymbolInfo];

    /// Find a symbol by exact qualified name.
    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo>;

    /// Find the direct children of a type/namespace by exact parent qualified name.
    ///
    /// For `parent_qname = "context.Context"`, returns all symbols whose
    /// qualified_name is `context.Context.X` (one dot deeper) — methods,
    /// fields, nested types. Chain walkers use this to locate the next
    /// segment of a member chain without scanning every candidate that
    /// shares a simple name across the project + externals.
    fn members_of(&self, parent_qname: &str) -> &[SymbolInfo];

    /// Find all type-kind symbols (class, struct, interface, enum, ...) with
    /// the given simple name.
    ///
    /// Exists so `is-this-name-a-type?` checks in chain walkers don't iterate
    /// every non-type symbol that happens to share the name (common words
    /// like `String`, `Error`, `Context` collect thousands of non-type
    /// candidates across an indexed stdlib/externals set).
    fn types_by_name(&self, name: &str) -> &[SymbolInfo];

    /// Find all symbols whose qualified name starts with the given prefix + ".".
    fn in_namespace(&self, namespace: &str) -> Vec<&SymbolInfo>;

    /// Cheap existence check: does any symbol live under this namespace?
    /// O(log N), no allocation. Prefer this over `!in_namespace(x).is_empty()`.
    fn has_in_namespace(&self, namespace: &str) -> bool;

    /// Find all symbols defined in a specific file.
    fn in_file(&self, file_path: &str) -> &[SymbolInfo];

    /// Resolve a module specifier in the context of a specific source file
    /// and return the symbols of the target module.
    ///
    /// Necessary for relative specifiers (`./utils`, `../shared`) where the
    /// resolution depends on the source file's directory — `./utils` from
    /// `apps/web/foo.ts` and `apps/web/bar/baz.ts` are different files.
    /// Default impl falls back to `in_file(spec)` for callers (and indexes)
    /// that don't carry per-source resolution data.
    fn in_module_from(&self, _source_file: &str, spec: &str) -> &[SymbolInfo] {
        self.in_file(spec)
    }

    /// Look up the resolved file path for a module specifier in the context
    /// of a specific source file. Returns `None` when no resolution is
    /// known. Used by re-export following so chain hops can also be
    /// resolved per-source.
    fn resolve_module_from(
        &self,
        _source_file: &str,
        _spec: &str,
    ) -> Option<&str> {
        None
    }

    /// Get the annotated type name for a property/field symbol.
    /// e.g., "AlbumService.db" → Some("DatabaseRepository")
    fn field_type_name(&self, property_qname: &str) -> Option<&str>;

    /// Get the annotated return type for a method/function symbol.
    /// e.g., "UserRepo.findOne" → Some("User")
    fn return_type_name(&self, method_qname: &str) -> Option<&str>;

    /// Get the generic type arguments for a field's type annotation.
    /// e.g., "UserService.repo" → Some(["User"]) for `repo: Repository<User>`
    fn field_type_args(&self, property_qname: &str) -> Option<&[String]>;

    /// Get the generic type parameter names for a type declaration.
    /// e.g., "Repository" → Some(["T"]) for `interface Repository<T>`
    fn generic_params(&self, type_name: &str) -> Option<&[String]>;

    /// Look up re-export chain entries for a barrel file.
    ///
    /// Returns all `(original_name, source_module)` pairs that are re-exported
    /// from `file_path`.  Use this to follow `export { X } from './y'` chains.
    ///
    /// A `original_name` of `"*"` represents an `export * from './y'` wildcard.
    fn reexports_from(&self, file_path: &str) -> &[(String, String)];

    /// Check whether a name is known to be external — either a language primitive,
    /// a test-framework global, or a name from a manifest-declared dependency.
    ///
    /// Used by language resolvers to short-circuit chain classification: if the
    /// root segment of a member chain is externally known, the whole chain is
    /// external and need not be resolved against the project index.
    fn is_external_name(&self, name: &str, language: &str) -> bool;

    /// Return all symbols belonging to a workspace package.
    ///
    /// Used by language resolvers to scope lookups when an import specifier
    /// matches a sibling package's `declared_name`. Returns an empty slice
    /// when the package isn't known (e.g. single-project layouts).
    fn symbols_in_package(&self, _package_id: i64) -> &[SymbolInfo] {
        &[]
    }

    /// Resolve a module specifier to a workspace `package_id`, honoring deep
    /// imports by stripping trailing `/seg` segments until the declared name
    /// matches. Returns `None` when no workspace package declares that name.
    fn workspace_package_id(&self, _specifier: &str) -> Option<i64> {
        None
    }

    /// Exact declared_name match without the deep-import prefix walk.
    /// Returns true when `name` is literally a workspace package's
    /// `declared_name`. Used to tell deep imports apart from bare imports.
    fn is_workspace_declared_name(&self, _name: &str) -> bool {
        false
    }

    /// Rewrite a TS import specifier through the source package's tsconfig
    /// `paths` aliases. Returns the resolved bare path (e.g. `@/utils` →
    /// `src/utils`) or `None` when no alias matches.
    fn resolve_tsconfig_alias(
        &self,
        _package_id: Option<i64>,
        _specifier: &str,
    ) -> Option<String> {
        None
    }

    /// Per-package test-framework global check.
    ///
    /// Returns true when `name` is exported by a test framework declared in
    /// the given package's own manifest — NOT a sibling's. Prefer this over
    /// `is_external_name` when `ref_ctx.file_package_id` is available so a
    /// non-test package doesn't inherit `describe`/`it`/etc. from a sibling
    /// that pulls in vitest or jest.
    fn is_test_global_for(&self, _package_id: Option<i64>, _name: &str) -> bool {
        false
    }

    /// Return the direct parent class qualified name for the given class.
    ///
    /// Built from `Inherits` edges at index construction time.  Returns `None`
    /// when the class has no known parent in the project (e.g., top-level
    /// classes, external base classes, or classes not yet indexed).
    ///
    /// Callers that need transitive ancestors should chain calls:
    /// ```text
    /// let mut cls = my_class;
    /// for _ in 0..MAX_DEPTH {
    ///     match lookup.parent_class_qname(cls) {
    ///         Some(p) => cls = p,
    ///         None => break,
    ///     }
    /// }
    /// ```
    fn parent_class_qname(&self, _class_qname: &str) -> Option<&str> {
        None
    }
}

// ---------------------------------------------------------------------------
// TypeInfo — unified per-symbol type metadata
// ---------------------------------------------------------------------------

/// All type metadata for a single symbol, stored in a single map keyed by
/// the symbol's qualified name (or simple name for generic_params).
#[derive(Debug, Default, Clone)]
pub struct TypeInfo {
    /// Field/property type (e.g., "UserRepository").
    pub field_type: Option<String>,
    /// Generic type arguments (e.g., ["User"] for `Repository<User>`).
    pub type_args: Vec<String>,
    /// Method return type (e.g., "User").
    pub return_type: Option<String>,
    /// Generic parameter names for type declarations (e.g., ["T"] for `interface Repository<T>`).
    pub generic_params: Vec<String>,
}

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
    /// Test-framework globals (e.g., `expect`, `describe`, `it`) computed from
    /// manifest dependencies. Keyed by language-independent name since most test
    /// globals are language-specific sets unioned at build time.
    test_globals: HashSet<String>,
    /// Per-package test-globals set. A package inherits globals only from
    /// test frameworks declared in its own manifest — a consumer package
    /// that never declares vitest/jest/etc. cannot smuggle `describe`/`it`
    /// in through a sibling's dependencies. Resolvers with a package_id in
    /// scope (via `ref_ctx.file_package_id`) should prefer
    /// `is_test_global_for(package_id, name)` to the union-based check.
    test_globals_by_pkg: FxHashMap<i64, HashSet<String>>,
    /// Primitive type names keyed by language id → set of primitive names.
    /// Built once from `primitives::primitives_for_language` for all languages
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
    /// Per-package tsconfig `paths` aliases, snapshot of the NPM manifest's
    /// `tsconfig_paths` for each workspace package. Empty for single-project
    /// layouts — callers fall back to the union.
    tsconfig_paths_by_pkg: FxHashMap<i64, Vec<(String, String)>>,
    /// Project-wide union of tsconfig alias entries; used when no
    /// `package_id` is set or the package has no per-package entry.
    tsconfig_paths_union: Vec<(String, String)>,
    /// Class inheritance map: child class qualified_name → parent class qualified_name.
    /// Built from `Inherits` refs at index construction time.  Used by language
    /// resolvers to walk the ancestor chain when `$this->method()` calls cannot
    /// be resolved within the immediate class scope.
    ///
    /// Keyed by child qname (dotted form), value is the direct parent qname.
    /// Transitive ancestors are reached by chaining lookups.
    inherits_map: FxHashMap<String, String>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl SymbolIndex {
    /// Build the index from parsed files and the symbol-to-ID mapping.
    pub fn build(
        parsed: &[ParsedFile],
        symbol_id_map: &HashMap<(String, String), i64>,
    ) -> Self {
        Self::build_with_context(parsed, symbol_id_map, None)
    }

    /// Build the index, optionally with project context for ecosystem-aware
    /// module resolution (e.g. the Go module path from go.mod).
    pub fn build_with_context(
        parsed: &[ParsedFile],
        symbol_id_map: &HashMap<(String, String), i64>,
        project_ctx: Option<&crate::indexer::project_context::ProjectContext>,
    ) -> Self {
        let mut by_name: FxHashMap<String, Vec<SymbolInfo>> = FxHashMap::default();
        let mut by_qname: BTreeMap<String, SymbolInfo> = BTreeMap::new();
        let mut by_file: FxHashMap<String, Vec<SymbolInfo>> = FxHashMap::default();
        let mut members_by_parent: FxHashMap<String, Vec<SymbolInfo>> =
            FxHashMap::default();
        let mut types_by_name: FxHashMap<String, Vec<SymbolInfo>> = FxHashMap::default();

        for pf in parsed {
            // One Arc<str> per file — all symbols in this file share the same
            // allocation instead of cloning an independent String per symbol.
            let file_path: Arc<str> = Arc::from(pf.path.as_str());

            for sym in &pf.symbols {
                let Some(&id) = symbol_id_map.get(&(pf.path.clone(), sym.qualified_name.clone()))
                else {
                    continue;
                };

                let info = SymbolInfo {
                    id,
                    name: sym.name.clone(),
                    qualified_name: sym.qualified_name.clone(),
                    kind: sym.kind.as_str().to_string(),
                    visibility: sym.visibility.as_ref().map(|v| format!("{v:?}").to_lowercase()),
                    file_path: Arc::clone(&file_path),
                    scope_path: sym.scope_path.clone(),
                    package_id: pf.package_id,
                };

                // Simple name index
                let simple = sym.name.clone();
                by_name.entry(simple).or_default().push(info.clone());

                // Qualified name index (first wins for duplicates)
                by_qname
                    .entry(sym.qualified_name.clone())
                    .or_insert_with(|| info.clone());

                // File index — key stays String (one allocation per file, not per symbol)
                by_file
                    .entry(pf.path.clone())
                    .or_default()
                    .push(info.clone());

                // Direct-children index: everything before the last '.' is the
                // parent qname; top-level symbols go under "".
                let parent_key: &str = match sym.qualified_name.rfind('.') {
                    Some(idx) => &sym.qualified_name[..idx],
                    None => "",
                };
                if is_type_like_kind(&info.kind) {
                    types_by_name
                        .entry(sym.name.clone())
                        .or_default()
                        .push(info.clone());
                }
                members_by_parent
                    .entry(parent_key.to_string())
                    .or_default()
                    .push(info);
            }
        }

        // Build field_type, field_type_args, return_type, and generic_params maps.
        let mut field_type: FxHashMap<String, String> = FxHashMap::default();
        let mut field_type_args: FxHashMap<String, Vec<String>> = FxHashMap::default();
        let mut return_type: FxHashMap<String, String> = FxHashMap::default();
        let mut generic_params: FxHashMap<String, Vec<String>> = FxHashMap::default();

        for pf in parsed {
            // Group TypeRef (non-import) refs by source_symbol_index in one
            // pass over pf.refs — avoids the O(symbols × refs) cost of
            // re-scanning the full ref list for each symbol. At 869k total
            // symbols with 2k-3k refs per external Go stdlib file this
            // inner scan was the dominant term in build_with_context.
            let mut type_refs_by_sym: Vec<Vec<&str>> = vec![Vec::new(); pf.symbols.len()];
            for r in &pf.refs {
                if r.kind != EdgeKind::TypeRef || r.module.is_some() {
                    continue;
                }
                let idx = r.source_symbol_index;
                if idx < type_refs_by_sym.len() {
                    type_refs_by_sym[idx].push(r.target_name.as_str());
                }
            }

            for (sym_idx, sym) in pf.symbols.iter().enumerate() {
                let type_refs = &type_refs_by_sym[sym_idx];
                if type_refs.is_empty() {
                    continue;
                }

                match sym.kind {
                    // Properties/fields: first TypeRef is the field type.
                    // Subsequent TypeRefs from the same symbol may be generic type args.
                    SymbolKind::Property | SymbolKind::Field => {
                        field_type
                            .insert(sym.qualified_name.clone(), type_refs[0].to_string());
                        // If there are additional TypeRefs, they're generic type arguments.
                        // e.g., `repo: Repository<User>` emits ["Repository", "User"]
                        if type_refs.len() > 1 {
                            field_type_args.insert(
                                sym.qualified_name.clone(),
                                type_refs[1..].iter().map(|s| s.to_string()).collect(),
                            );
                        }
                    }
                    // Local variables: first TypeRef is the inferred/annotated type.
                    // Emitted by extractors when the RHS is a constructor, struct
                    // literal, or factory call (e.g. `let pool = DbPool::new(config)`,
                    // `const svc = new UserService()`, `repo = UserRepository(db)`).
                    // Only non-chain TypeRefs land here; chain-bearing ones are handled
                    // by the chain-inference pass below.
                    SymbolKind::Variable => {
                        field_type
                            .insert(sym.qualified_name.clone(), type_refs[0].to_string());
                        if type_refs.len() > 1 {
                            field_type_args.insert(
                                sym.qualified_name.clone(),
                                type_refs[1..].iter().map(|s| s.to_string()).collect(),
                            );
                        }
                    }
                    // Type aliases (typedefs, `using Alias = Type`): first TypeRef
                    // is the aliased type. This populates field_type_name("AliasName")
                    // so chain walkers can dereference typedef aliases.
                    // e.g., `typedef SocketChannel* SocketChannelPtr;`
                    //   → field_type("SocketChannelPtr") = "SocketChannel"
                    // Used by the C/C++ chain walker's dereference_typedef step.
                    SymbolKind::TypeAlias => {
                        field_type
                            .insert(sym.qualified_name.clone(), type_refs[0].to_string());
                        // Also index by simple name for cross-TU lookups where
                        // the typedef may be referenced without its full scope prefix.
                        if sym.name != sym.qualified_name {
                            field_type
                                .entry(sym.name.clone())
                                .or_insert_with(|| type_refs[0].to_string());
                        }
                    }
                    // Methods/functions: last TypeRef is likely the return type.
                    SymbolKind::Method
                    | SymbolKind::Function
                    | SymbolKind::Constructor => {
                        if let Some(&last) = type_refs.last() {
                            return_type
                                .insert(sym.qualified_name.clone(), last.to_string());
                        }
                        // Extra pass: some extractors emit no TypeRef refs
                        // but DO populate a signature string with the return
                        // type at the end (`method(...): ReturnType`). This
                        // fires for synthetic .NET DLL metadata symbols
                        // where there are no tree-sitter refs to mine. Only
                        // fills the slot when a direct TypeRef path above
                        // didn't find one, so languages that already set a
                        // real return_type aren't overwritten.
                        if !return_type.contains_key(&sym.qualified_name) {
                            if let Some(sig) = &sym.signature {
                                if let Some(rt) = parse_return_type_from_signature(sig) {
                                    return_type.insert(sym.qualified_name.clone(), rt);
                                }
                            }
                        }
                    }
                    // Classes/interfaces/structs with TypeRef to themselves may have
                    // generic type parameters in the signature.
                    _ => {}
                }
            }

            // Build generic_params: for class/interface/struct symbols that have
            // type_parameters in their signature (e.g., `interface Repository<T>`,
            // `def process[F[_]]`, `fn compute<T: Clone>`).
            // We detect this by looking at the symbol's signature text.
            for sym in &pf.symbols {
                if !matches!(
                    sym.kind,
                    SymbolKind::Class
                        | SymbolKind::Interface
                        | SymbolKind::Struct
                        | SymbolKind::TypeAlias
                        | SymbolKind::Function
                        | SymbolKind::Method
                ) {
                    continue;
                }
                if let Some(sig) = &sym.signature {
                    // Parse generic params from signature:
                    //   "interface Repository<T>"      → ["T"]      (Java, C#, TS, Kotlin)
                    //   "class Map<K, V>"              → ["K", "V"]
                    //   "trait SnapshotReader[F[_]]"   → ["F"]      (Scala)
                    //   "struct Vec<T>"                → ["T"]      (Rust)
                    //   "class FSM[F[_], S, I, O]"    → ["F", "S", "I", "O"]
                    //
                    // Tries `<>` first (most languages), then `[]` (Scala).
                    // Uses depth-counted bracket matching for nested generics.
                    let bracket_pairs: &[(char, char)] = &[('<', '>'), ('[', ']')];
                    for &(open, close) in bracket_pairs {
                        if let Some(start) = sig.find(open) {
                            if let Some(relative_end) =
                                find_matching_bracket(&sig[start..], open, close)
                            {
                                let end = start + relative_end;
                                let params_str = &sig[start + 1..end];
                                let params: Vec<String> = params_str
                                    .split(',')
                                    .map(|s| {
                                        let trimmed = s.trim();
                                        // Strip higher-kinded markers: "F[_]" → "F"
                                        let name = trimmed
                                            .split(|c: char| c == '[' || c == '<' || c == ':')
                                            .next()
                                            .unwrap_or("")
                                            .split_whitespace()
                                            .next()
                                            .unwrap_or("")
                                            .to_string();
                                        name
                                    })
                                    .filter(|s| !s.is_empty())
                                    .collect();
                                if !params.is_empty() {
                                    generic_params
                                        .insert(sym.name.clone(), params.clone());
                                    generic_params
                                        .insert(sym.qualified_name.clone(), params);
                                    break; // found params, don't try next bracket pair
                                }
                            }
                        }
                    }
                }
            }
        }

        // Merge the four local maps into the unified type_info map.
        let mut type_info: FxHashMap<String, TypeInfo> = FxHashMap::default();
        for (qname, ft) in field_type {
            type_info.entry(qname).or_default().field_type = Some(ft);
        }
        for (qname, args) in field_type_args {
            type_info.entry(qname).or_default().type_args = args;
        }
        for (qname, rt) in return_type {
            type_info.entry(qname).or_default().return_type = Some(rt);
        }
        for (name_or_qname, params) in generic_params {
            type_info.entry(name_or_qname).or_default().generic_params = params;
        }

        // Variable type inference pass: for Variable symbols without an explicit
        // type annotation, try to infer the type from chain-bearing TypeRef refs.
        // These are emitted by the extractor for `const x = this.repo.findOne()`.
        // We resolve the chain to get the method's return type.
        //
        // Per-file, we group the first chain-bearing TypeRef by symbol index in
        // one O(refs) pass so the Variable loop avoids the O(variables × refs)
        // filter that dominated build_with_context on the external Go stdlib
        // (hundreds of top-level `var _ = …` decls × thousands of refs each).
        for pf in parsed {
            let mut first_chain_typeref: Vec<Option<&crate::types::ExtractedRef>> =
                vec![None; pf.symbols.len()];
            for r in &pf.refs {
                if r.kind != EdgeKind::TypeRef || r.chain.is_none() {
                    continue;
                }
                let idx = r.source_symbol_index;
                if idx < first_chain_typeref.len() && first_chain_typeref[idx].is_none() {
                    first_chain_typeref[idx] = Some(r);
                }
            }

            for (sym_idx, sym) in pf.symbols.iter().enumerate() {
                if sym.kind != SymbolKind::Variable {
                    continue;
                }
                // Skip if already has an explicit type.
                if type_info
                    .get(&sym.qualified_name)
                    .and_then(|ti| ti.field_type.as_ref())
                    .is_some()
                {
                    continue;
                }
                let Some(r) = first_chain_typeref[sym_idx] else {
                    continue;
                };
                let chain = r.chain.as_ref().unwrap();
                if let Some(inferred) =
                    infer_type_from_chain(chain, &sym.scope_path, &type_info, &by_name, &by_qname)
                {
                    type_info
                        .entry(sym.qualified_name.clone())
                        .or_default()
                        .field_type = Some(inferred);
                }
            }
        }

        // Build class inheritance map: child_qname → parent_qname.
        //
        // Source: `Inherits` refs emitted by language extractors.  The
        // `source_symbol_index` identifies the child class symbol in its own
        // file; `target_name` is the parent's short/simple name.  We resolve
        // it to a qualified name via the by_name index (already built above).
        //
        // When multiple symbols share the same short name, we prefer the one
        // whose namespace matches the child class's namespace most closely
        // (longest common prefix).  This is a best-effort approximation —
        // the common case (one class per simple name in a project) will always
        // resolve correctly.
        let mut inherits_map: FxHashMap<String, String> = FxHashMap::default();
        for pf in parsed {
            for r in &pf.refs {
                if r.kind != EdgeKind::Inherits {
                    continue;
                }
                // Identify the child class symbol.
                let Some(child_sym) = pf.symbols.get(r.source_symbol_index) else {
                    continue;
                };
                if !matches!(child_sym.kind, SymbolKind::Class | SymbolKind::Interface) {
                    continue;
                }
                let child_qname = &child_sym.qualified_name;
                // Avoid overwriting an existing entry (first Inherits edge wins).
                if inherits_map.contains_key(child_qname) {
                    continue;
                }
                // Resolve parent simple name → qname via by_name.
                let parent_simple = r.target_name.trim_start_matches('\\');
                let candidates = by_name.get(parent_simple).map(|v| v.as_slice()).unwrap_or(&[]);
                if candidates.is_empty() {
                    continue;
                }
                // Pick the candidate whose namespace best matches the child's namespace.
                // "Best" = longest common dotted prefix.
                let child_ns = child_qname.rfind('.').map(|i| &child_qname[..i]).unwrap_or("");
                let best = if candidates.len() == 1 {
                    &candidates[0]
                } else {
                    candidates
                        .iter()
                        .max_by_key(|c| {
                            let cns = c.qualified_name.rfind('.').map(|i| &c.qualified_name[..i]).unwrap_or("");
                            common_prefix_len(child_ns, cns)
                        })
                        .unwrap_or(&candidates[0])
                };
                inherits_map.insert(child_qname.clone(), best.qualified_name.clone());
            }
        }

        // Build re-export map from Imports refs that have a module set.
        // These are emitted by the TS/JS extractor for:
        //   export { X } from './y'   → Imports ref, target_name="X", module="./y"
        //   export * from './y'       → Imports ref, target_name="*", module="./y"
        let mut reexport_map: FxHashMap<String, Vec<(String, String)>> = FxHashMap::default();
        for pf in parsed {
            for r in &pf.refs {
                if r.kind != EdgeKind::Imports {
                    continue;
                }
                let Some(ref mod_path) = r.module else {
                    continue;
                };
                if mod_path.is_empty() {
                    continue;
                }
                reexport_map
                    .entry(pf.path.clone())
                    .or_default()
                    .push((r.target_name.clone(), mod_path.clone()));
            }
        }

        // Build module-to-file mapping using ecosystem-specific ModuleResolvers.
        // For each import ref that carries a module specifier, resolve it to an
        // actual indexed file path and cache the result.
        //
        // Two cache shapes:
        //   - module_to_file: spec → file (for bare/aliased specifiers where
        //     the source file's directory doesn't affect resolution)
        //   - module_to_file_per_source: (source_file, spec) → file (for
        //     relative specifiers like ./utils, ../shared — different
        //     source dirs resolve the same spec to different files)
        //
        // Sharing one global map for relative paths causes the first
        // consumer to "win the slot" for `./utils` and silently breaks
        // resolution for every other file with a same-named neighbour.
        let go_module_path = project_ctx
            .and_then(|ctx| ctx.manifest(crate::indexer::manifest::ManifestKind::GoMod))
            .and_then(|m| m.module_path.as_deref());
        let resolvers =
            crate::indexer::module_resolution::all_resolvers_with_go_module(go_module_path);
        let file_paths: Vec<&str> = parsed.iter().map(|pf| pf.path.as_str()).collect();
        let mut module_to_file: FxHashMap<String, String> = FxHashMap::default();
        let mut module_to_file_per_source: FxHashMap<(String, String), String> =
            FxHashMap::default();

        for pf in parsed {
            let resolver = resolvers
                .iter()
                .find(|r| r.language_ids().contains(&pf.language.as_str()));
            let Some(resolver) = resolver else {
                continue;
            };

            for r in &pf.refs {
                let Some(module) = &r.module else {
                    continue;
                };
                if module.is_empty() {
                    continue;
                }
                // Relative specifiers must be cached per-source because the
                // resolution depends on the importing file's directory.
                let is_relative = module.starts_with('.');
                if is_relative {
                    let key = (pf.path.clone(), module.clone());
                    if module_to_file_per_source.contains_key(&key) {
                        continue;
                    }
                    if let Some(resolved) =
                        resolver.resolve_to_file(module, &pf.path, &file_paths)
                    {
                        module_to_file_per_source.insert(key, resolved);
                    }
                    continue;
                }
                if module_to_file.contains_key(module.as_str()) {
                    continue;
                }
                if let Some(resolved) =
                    resolver.resolve_to_file(module, &pf.path, &file_paths)
                {
                    module_to_file.insert(module.clone(), resolved);
                }
            }
        }

        // Build test-framework globals from manifest dependencies.
        //
        // Union set: the name is a test global if ANY package declared a
        // framework that exports it. Used by the existing
        // `is_external_name` / `classify_external_name` API which has no
        // package_id parameter today.
        //
        // Per-package set: built separately below so resolvers can query
        // `is_test_global_for(package_id, name)` and avoid misclassifying
        // a bare `describe` as external inside a non-test package just
        // because a sibling test package declared vitest.
        let test_globals: HashSet<String> = build_test_globals_union(project_ctx);
        let test_globals_by_pkg: FxHashMap<i64, HashSet<String>> =
            build_test_globals_by_pkg(project_ctx);

        // Build per-language primitive sets for all languages present in parsed files.
        let mut primitives_by_language: FxHashMap<String, HashSet<&'static str>> =
            FxHashMap::default();
        for pf in parsed {
            if !primitives_by_language.contains_key(&pf.language) {
                let set = crate::indexer::primitives::primitives_set_for_language(&pf.language);
                if !set.is_empty() {
                    primitives_by_language.insert(pf.language.clone(), set);
                }
            }
        }

        // Group symbols by workspace package_id so language resolvers can
        // scope lookups when an import specifier matches a sibling package's
        // declared_name. One entry per qname — duplicates filtered via
        // by_qname's first-wins semantics.
        let mut by_package: FxHashMap<i64, Vec<SymbolInfo>> = FxHashMap::default();
        for sym in by_qname.values() {
            if let Some(pkg_id) = sym.package_id {
                by_package.entry(pkg_id).or_default().push(sym.clone());
            }
        }

        let workspace_pkg_by_declared_name: FxHashMap<String, i64> = project_ctx
            .map(|ctx| {
                ctx.workspace_pkg_by_declared_name
                    .iter()
                    .map(|(k, v)| (k.clone(), *v))
                    .collect()
            })
            .unwrap_or_default();

        // Snapshot tsconfig aliases — per-package if available, plus a union
        // derived from the NPM manifest for files with no package_id.
        //
        // tsconfig `paths` targets are relative to each package's own
        // directory, not the workspace root. In a monorepo with
        // `apps/landing/tsconfig.json` declaring `"@/*": ["src/*"]`, a
        // rewritten `@/components/x` must land at
        // `apps/landing/src/components/x` for `in_file()` to find the file.
        // Prepend the package path to each target at snapshot time.
        let mut tsconfig_paths_by_pkg: FxHashMap<i64, Vec<(String, String)>> = FxHashMap::default();
        let mut tsconfig_paths_union: Vec<(String, String)> = Vec::new();
        if let Some(ctx) = project_ctx {
            if let Some(npm) = ctx.manifest(crate::indexer::manifest::ManifestKind::Npm) {
                tsconfig_paths_union = npm.tsconfig_paths.clone();
            }
            for (&pkg_id, manifests) in &ctx.by_package {
                if let Some(npm) = manifests.get(&crate::indexer::manifest::ManifestKind::Npm) {
                    if !npm.tsconfig_paths.is_empty() {
                        let pkg_path = ctx.workspace_pkg_paths.get(&pkg_id);
                        let rewritten: Vec<(String, String)> = npm
                            .tsconfig_paths
                            .iter()
                            .map(|(alias, target)| {
                                let full_target = match pkg_path {
                                    Some(p) if !p.is_empty() => format!("{p}/{target}"),
                                    _ => target.clone(),
                                };
                                (alias.clone(), full_target)
                            })
                            .collect();
                        tsconfig_paths_by_pkg.insert(pkg_id, rewritten);
                    }
                }
            }
        }

        Self {
            by_name,
            by_qname,
            by_file,
            members_by_parent,
            types_by_name,
            type_info,
            reexport_map,
            module_to_file,
            module_to_file_per_source,
            test_globals,
            test_globals_by_pkg,
            primitives_by_language,
            by_package,
            workspace_pkg_by_declared_name,
            tsconfig_paths_by_pkg,
            tsconfig_paths_union,
            inherits_map,
            empty: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }

    /// Load all symbols from the database into the index, filling gaps left by
    /// an incremental build where only changed files were parsed.
    ///
    /// Symbols already present (from parsed files) are NOT overwritten — the
    /// parsed data is richer (has type info, reexports).  This only adds
    /// entries for symbols in unchanged files so the engine resolver can find
    /// them by name during cross-file resolution.
    ///
    /// Call this AFTER `build_with_context` for incremental resolution.
    pub fn augment_from_db(&mut self, conn: &rusqlite::Connection) {
        let mut stmt = match conn.prepare(
            "SELECT s.id, s.name, s.qualified_name, s.kind, f.path,
                    s.scope_path, s.visibility, f.package_id
             FROM symbols s
             JOIN files f ON f.id = s.file_id",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        let rows = match stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<i64>>(7)?,
            ))
        }) {
            Ok(r) => r,
            Err(_) => return,
        };

        for row in rows {
            let Ok((id, name, qname, kind, file_path, scope_path, visibility, package_id)) = row
            else {
                continue;
            };

            // Skip symbols already indexed from parsed files.
            if self.by_qname.contains_key(&qname) {
                continue;
            }

            // Arc<str> for file_path: shared across all symbols in the same file
            // within this batch; BTreeMap insert gets one clone, by_file gets another.
            let file_arc: Arc<str> = Arc::from(file_path.as_str());

            let info = SymbolInfo {
                id,
                name: name.clone(),
                qualified_name: qname.clone(),
                kind,
                visibility,
                file_path: Arc::clone(&file_arc),
                scope_path,
                package_id,
            };

            self.by_name.entry(name.clone()).or_default().push(info.clone());
            self.by_qname.insert(qname.clone(), info.clone());
            if let Some(pkg_id) = info.package_id {
                self.by_package.entry(pkg_id).or_default().push(info.clone());
            }
            let parent_key: String = match qname.rfind('.') {
                Some(idx) => qname[..idx].to_string(),
                None => String::new(),
            };
            if is_type_like_kind(&info.kind) {
                self.types_by_name
                    .entry(name)
                    .or_default()
                    .push(info.clone());
            }
            self.members_by_parent
                .entry(parent_key)
                .or_default()
                .push(info.clone());
            // by_file key stays String (one allocation per file, not per symbol)
            self.by_file.entry(file_path).or_default().push(info);
        }
    }
}

/// Lightweight chain resolution for variable type inference during index building.
/// Uses the already-built type_info map (not the full SymbolLookup trait).
/// Given a type string that looks like a TypeScript tuple literal
/// (`[A, B, C]` where commas are at bracket/angle depth zero), return the
/// Nth element. Returns `None` if `raw` isn't a tuple, if `idx` is out of
/// range, or if the brackets are unbalanced.
///
/// Used by `infer_type_from_chain` to resolve array-pattern destructuring:
/// `const [a, b] = useState<T>()` walks to `useState`'s return type
/// `[T, Dispatch<SetStateAction<T>>]`, then slices index 0 or 1.
///
/// Nested tuples/generics are handled by tracking `<>`/`[]`/`()` depth so
/// commas inside `Dispatch<SetStateAction<T>>` or `[inner, tuple]` aren't
/// split. This is a pure string operation — the walker's TypeEnvironment
/// handles the subsequent generic substitution separately.
fn tuple_element(raw: &str, idx: usize) -> Option<String> {
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    let mut depth_angle = 0i32;
    let mut depth_square = 0i32;
    let mut depth_paren = 0i32;
    let mut current = String::new();
    let mut parts: Vec<String> = Vec::new();
    for ch in inner.chars() {
        match ch {
            '<' => depth_angle += 1,
            '>' => depth_angle -= 1,
            '[' => depth_square += 1,
            ']' => depth_square -= 1,
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            ',' if depth_angle == 0 && depth_square == 0 && depth_paren == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts.into_iter().nth(idx)
}

/// Extract a return type from a method signature string of the shape
/// `{name}{gp}(params): ReturnType`. Returns the substring after the
/// last top-level `):` separator, trimmed. Returns `None` if the shape
/// doesn't match (no parens, no colon after the closing paren, etc).
///
/// Top-level tracking: the colon must be at paren depth 0 so `(): ret`
/// inside a nested function type like `(): () => void` doesn't get
/// captured by mistake.
///
/// Used by the TypeInfo builder to populate return_type for synthetic
/// .NET DLL metadata symbols that have no TypeRef edges but do carry
/// a dotscope-formatted signature string.
fn parse_return_type_from_signature(sig: &str) -> Option<String> {
    // Find the top-level `):` separator. Scan right-to-left tracking
    // paren depth from zero upward — the FIRST `)` at depth 0 (from
    // the end) followed by `:` marks the return type boundary.
    let bytes = sig.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut depth_paren: i32 = 0;
    let mut depth_angle: i32 = 0;
    let mut depth_square: i32 = 0;
    // Scan right-to-left so we get the outermost close paren.
    for (i, &b) in bytes.iter().enumerate().rev() {
        match b {
            b')' => depth_paren += 1,
            b'(' => depth_paren -= 1,
            b'>' => depth_angle += 1,
            b'<' => depth_angle -= 1,
            b']' => depth_square += 1,
            b'[' => depth_square -= 1,
            b':' if depth_paren == 0 && depth_angle == 0 && depth_square == 0 => {
                // Must be preceded by `)` at position i-1 (or earlier
                // with whitespace). Trim whitespace and verify.
                let before = sig[..i].trim_end();
                if before.ends_with(')') {
                    let after = sig[i + 1..].trim();
                    if !after.is_empty() {
                        return Some(after.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn infer_type_from_chain(
    chain: &crate::types::MemberChain,
    scope_path: &Option<String>,
    type_info: &FxHashMap<String, TypeInfo>,
    by_name: &FxHashMap<String, Vec<SymbolInfo>>,
    by_qname: &BTreeMap<String, SymbolInfo>,
) -> Option<String> {
    use crate::types::SegmentKind;

    let segments = &chain.segments;
    if segments.is_empty() {
        return None;
    }

    // Build a minimal scope chain from scope_path.
    let scopes: Vec<String> = if let Some(sp) = scope_path {
        let mut scope_chain = Vec::new();
        let mut current = sp.clone();
        scope_chain.push(current.clone());
        while let Some(dot) = current.rfind('.') {
            current.truncate(dot);
            scope_chain.push(current.clone());
        }
        scope_chain
    } else {
        Vec::new()
    };

    // Phase 1: Root type.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            // Find enclosing class from scope.
            scopes
                .iter()
                .find_map(|s| {
                    by_qname.get(s).and_then(|sym| {
                        if matches!(sym.kind.as_str(), "class" | "struct" | "interface") {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                })
                .or_else(|| scopes.last().cloned())
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;
            scopes
                .iter()
                .find_map(|scope| {
                    let qname = format!("{scope}.{name}");
                    type_info.get(&qname).and_then(|ti| ti.field_type.clone())
                })
                .or_else(|| segments[0].declared_type.clone())
        }
        _ => None,
    }?;

    let mut current_type = root_type;

    // Build a TypeEnvironment for generic substitution.
    let mut env = TypeEnvironment::new();

    // Look up initial generic args for the root field.
    for scope in &scopes {
        let key = format!("{scope}.{}", segments[0].name);
        if let Some(ti) = type_info.get(&key) {
            if !ti.type_args.is_empty() {
                // Enter the generic context for the root type.
                let args = ti.type_args.clone();
                env.enter_generic_context(&current_type, &args, |name| {
                    // Look up by qualified name first, then by simple name.
                    type_info
                        .get(name)
                        .map(|ti| ti.generic_params.clone())
                        .filter(|p| !p.is_empty())
                        .or_else(|| {
                            let simple = name.rsplit('.').next().unwrap_or(name);
                            type_info
                                .get(simple)
                                .map(|ti| ti.generic_params.clone())
                                .filter(|p| !p.is_empty())
                        })
                });
                break;
            }
        }
    }

    // Phase 2: Walk remaining segments.
    for seg in &segments[1..] {
        // Tuple-index selection: produced by array-pattern destructuring.
        // `const [isOpen, setIsOpen] = useState<boolean>(false)` emits a
        // ComputedAccess segment with integer name "0" / "1" so this pass
        // can slice the tuple return type of the preceding call.
        //
        // The incoming `current_type` at this point is the return type of
        // the call (already generic-resolved by a prior iteration), e.g.
        // `[boolean, Dispatch<SetStateAction<boolean>>]`. Splitting by
        // top-level commas and picking the Nth element yields the type
        // bound to the destructured name.
        if seg.kind == SegmentKind::ComputedAccess {
            if let Ok(idx) = seg.name.parse::<usize>() {
                if let Some(element) = tuple_element(&current_type, idx) {
                    current_type = env.resolve(&element);
                    continue;
                }
            }
        }

        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(ti) = type_info.get(&member_qname) {
            if let Some(ft) = &ti.field_type {
                let new_args = ti.type_args.clone();
                let resolved_type = env.resolve(ft);
                env.push_scope();
                if !new_args.is_empty() {
                    env.enter_generic_context(&resolved_type, &new_args, |name| {
                        type_info
                            .get(name)
                            .map(|ti| ti.generic_params.clone())
                            .filter(|p| !p.is_empty())
                            .or_else(|| {
                                let simple = name.rsplit('.').next().unwrap_or(name);
                                type_info
                                    .get(simple)
                                    .map(|ti| ti.generic_params.clone())
                                    .filter(|p| !p.is_empty())
                            })
                    });
                }
                current_type = resolved_type;
                continue;
            }
            if let Some(raw_return) = &ti.return_type {
                // Use TypeEnvironment for generic substitution (T → User, E → Error, etc).
                let resolved = env.resolve(raw_return);
                env.push_scope();
                current_type = resolved;
                continue;
            }
        }

        // Fallback: if the segment corresponds to a call with no matching
        // TypeInfo entry, treat the current_type as the "call return" so
        // chains like `useState(...)[0]` still work when useState is
        // external and its signature is in the index keyed by simple name
        // rather than qualified name. Look up by simple name.
        if let Some(sym_list) = by_name.get(&seg.name) {
            for sym in sym_list {
                if let Some(ti) = type_info.get(&sym.qualified_name) {
                    if let Some(raw_return) = &ti.return_type {
                        let resolved = env.resolve(raw_return);
                        env.push_scope();
                        current_type = resolved;
                        break;
                    }
                }
            }
            // If the loop above didn't `continue`, fall through to the
            // "can't follow" return below only when nothing matched.
            if current_type.contains('[') || current_type.contains('<') {
                // current_type was updated; move to the next segment.
                continue;
            }
        }

        // Can't follow further.
        let _ = by_qname; // retained for future use
        return None;
    }

    // The final current_type is the inferred type of the chain.
    Some(current_type)
}

impl SymbolLookup for SymbolIndex {
    fn by_name(&self, name: &str) -> &[SymbolInfo] {
        self.by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        self.by_qname.get(qname)
    }

    fn members_of(&self, parent_qname: &str) -> &[SymbolInfo] {
        self.members_by_parent
            .get(parent_qname)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn types_by_name(&self, name: &str) -> &[SymbolInfo] {
        self.types_by_name
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// O(log N) prefix search via BTreeMap::range — no extra Vec needed.
    fn in_namespace(&self, namespace: &str) -> Vec<&SymbolInfo> {
        let prefix = format!("{namespace}.");
        // Range lower bound is the owned prefix String so K: Borrow<K> = String: Borrow<String>
        // is satisfied; take_while stops as soon as keys no longer share the prefix.
        self.by_qname
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| v)
            .collect()
    }

    fn has_in_namespace(&self, namespace: &str) -> bool {
        let prefix = format!("{namespace}.");
        self.by_qname
            .range(prefix.clone()..)
            .next()
            .is_some_and(|(k, _)| k.starts_with(&prefix))
    }

    fn in_file(&self, file_path: &str) -> &[SymbolInfo] {
        // Exact match
        if let Some(syms) = self.by_file.get(file_path) {
            return syms.as_slice();
        }
        // Module specifier → resolved file path
        if let Some(resolved) = self.module_to_file.get(file_path) {
            if let Some(syms) = self.by_file.get(resolved) {
                return syms.as_slice();
            }
        }
        &self.empty
    }

    fn in_module_from(&self, source_file: &str, spec: &str) -> &[SymbolInfo] {
        // Per-source resolution wins for relative specifiers — `./utils`
        // from one file is a different file than from another. Falls back
        // to the global lookups (exact path / global module_to_file) when
        // the per-source map has no entry for this (source, spec) pair.
        if spec.starts_with('.') {
            if let Some(resolved) = self
                .module_to_file_per_source
                .get(&(source_file.to_string(), spec.to_string()))
            {
                if let Some(syms) = self.by_file.get(resolved) {
                    return syms.as_slice();
                }
            }
            // Backward-compat: if the spec literally matches an indexed
            // file path (test fixtures often use the spec as the path),
            // surface it. Real-world relative specs like `./utils` won't
            // collide with indexed paths so this is harmless.
        }
        self.in_file(spec)
    }

    fn resolve_module_from(
        &self,
        source_file: &str,
        spec: &str,
    ) -> Option<&str> {
        if spec.starts_with('.') {
            return self
                .module_to_file_per_source
                .get(&(source_file.to_string(), spec.to_string()))
                .map(|s| s.as_str());
        }
        self.module_to_file.get(spec).map(|s| s.as_str())
    }

    fn field_type_name(&self, property_qname: &str) -> Option<&str> {
        self.type_info
            .get(property_qname)
            .and_then(|ti| ti.field_type.as_deref())
    }

    fn return_type_name(&self, method_qname: &str) -> Option<&str> {
        self.type_info
            .get(method_qname)
            .and_then(|ti| ti.return_type.as_deref())
    }

    fn field_type_args(&self, property_qname: &str) -> Option<&[String]> {
        self.type_info.get(property_qname).and_then(|ti| {
            if ti.type_args.is_empty() {
                None
            } else {
                Some(ti.type_args.as_slice())
            }
        })
    }

    fn generic_params(&self, type_name: &str) -> Option<&[String]> {
        self.type_info.get(type_name).and_then(|ti| {
            if ti.generic_params.is_empty() {
                None
            } else {
                Some(ti.generic_params.as_slice())
            }
        })
    }

    fn reexports_from(&self, file_path: &str) -> &[(String, String)] {
        // Exact match
        if let Some(v) = self.reexport_map.get(file_path) {
            return v.as_slice();
        }
        // Module specifier → resolved file path
        if let Some(resolved) = self.module_to_file.get(file_path) {
            if let Some(v) = self.reexport_map.get(resolved) {
                return v.as_slice();
            }
        }
        &self.empty_reexports
    }

    fn is_external_name(&self, name: &str, language: &str) -> bool {
        // 1. Language primitive check.
        if let Some(primitives) = self.primitives_by_language.get(language) {
            if primitives.contains(name) {
                return true;
            }
        }

        // 2. Test-framework global check (language-independent).
        if self.test_globals.contains(name) {
            return true;
        }

        // 3. Name is not in the project index at all and is not a known local name.
        // We intentionally do NOT call by_name here — that is used by the resolution
        // path. This method only covers the three positive-match cases above.
        false
    }

    fn symbols_in_package(&self, package_id: i64) -> &[SymbolInfo] {
        self.by_package
            .get(&package_id)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty)
    }

    fn workspace_package_id(&self, specifier: &str) -> Option<i64> {
        if let Some(&id) = self.workspace_pkg_by_declared_name.get(specifier) {
            return Some(id);
        }
        let mut path = specifier;
        while let Some(slash) = path.rfind('/') {
            path = &path[..slash];
            if let Some(&id) = self.workspace_pkg_by_declared_name.get(path) {
                return Some(id);
            }
        }
        None
    }

    fn is_workspace_declared_name(&self, name: &str) -> bool {
        self.workspace_pkg_by_declared_name.contains_key(name)
    }

    fn resolve_tsconfig_alias(
        &self,
        package_id: Option<i64>,
        specifier: &str,
    ) -> Option<String> {
        let paths = package_id
            .and_then(|id| self.tsconfig_paths_by_pkg.get(&id))
            .map(|v| v.as_slice())
            .unwrap_or(self.tsconfig_paths_union.as_slice());
        if paths.is_empty() {
            return None;
        }
        // Pick the longest matching alias so nested prefixes win.
        let mut best: Option<&(String, String)> = None;
        for entry in paths {
            let (alias, _) = entry;
            if specifier.starts_with(alias.as_str())
                && best.map_or(true, |(b, _)| alias.len() > b.len())
            {
                best = Some(entry);
            }
        }
        let (alias, target) = best?;
        let remainder = &specifier[alias.len()..];
        Some(format!("{target}{remainder}"))
    }

    fn is_test_global_for(&self, package_id: Option<i64>, name: &str) -> bool {
        // Per-package check first; falls back to the union only when there's
        // no package_id at all (root-scoped file). A known package that
        // doesn't declare a test framework correctly answers `false` —
        // sibling frameworks do not leak.
        if let Some(id) = package_id {
            return self
                .test_globals_by_pkg
                .get(&id)
                .map_or(false, |s| s.contains(name));
        }
        self.test_globals.contains(name)
    }

    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.inherits_map.get(class_qname).map(|s| s.as_str())
    }
}

impl SymbolIndex {
    /// Classify an external name with its specific namespace category.
    ///
    /// Returns:
    ///   - `Some("primitive")` for language keyword types (int, string, bool)
    ///   - `Some("builtin")` for runtime globals (console, print, len, Array)
    ///   - `Some("test_framework")` for test globals (describe, it, expect)
    ///   - `None` if the name is not classified as external
    pub fn classify_external_name(&self, name: &str, language: &str) -> Option<&'static str> {
        // Test globals first — most specific classification.
        if self.test_globals.contains(name) {
            return Some("test_framework");
        }

        // Check the merged external set (primitives + externals + query builtins).
        if let Some(all_externals) = self.primitives_by_language.get(language) {
            if all_externals.contains(name) {
                // Distinguish: plugin.primitives() are "primitive", everything
                // else (externals + query builtins) is "builtin".
                let plugin_primitives =
                    crate::indexer::primitives::primitives_for_language(language);
                if plugin_primitives.contains(&name) {
                    return Some("primitive");
                }
                return Some("builtin");
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// LanguageResolver trait
// ---------------------------------------------------------------------------

/// Per-language resolution rules. Each language implements this trait
/// in a separate file under `resolve/rules/`.
pub trait LanguageResolver: Send + Sync {
    /// The language identifier(s) this resolver handles.
    /// Must match the language strings from file detection.
    fn language_ids(&self) -> &[&str];

    /// Build the file context for a parsed file.
    /// `project_ctx` provides global usings and external prefix data.
    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext;

    /// Attempt to resolve a reference using language-specific scope rules.
    ///
    /// Returns `Some(Resolution)` if deterministically resolved.
    /// Returns `None` to fall back to the heuristic resolver.
    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution>;

    /// Check whether a target symbol is visible from the reference site.
    /// Default: always visible (no filtering).
    fn is_visible(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _target: &SymbolInfo,
    ) -> bool {
        true
    }

    /// When a reference can't be resolved (not in the index), try to infer
    /// which external namespace/package it likely comes from based on the
    /// file's import statements.
    ///
    /// Returns `Some("Microsoft.EntityFrameworkCore")` if the target probably
    /// comes from that namespace. Returns `None` if no guess can be made.
    ///
    /// Default: no inference.
    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        None
    }

    /// Same as `infer_external_namespace` but with `lookup` access for
    /// barrel/re-export inspection. Resolvers that need to chase a
    /// passthrough barrel (`@/foo` → `apps/x/src/foo.ts` → `export { Y }
    /// from "external-pkg"`) override this; default delegates to the
    /// lookup-free variant for callers that don't need it.
    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        self.infer_external_namespace(file_ctx, ref_ctx, project_ctx)
    }
}

// ---------------------------------------------------------------------------
// Shared resolution helpers for tier-2 language resolvers
// ---------------------------------------------------------------------------

/// Common resolution logic for languages that follow the standard pattern.
///
/// Steps (highest confidence first):
///   1. Module-qualified lookup: ref has `module` field → try `{module}.{target}`
///   2. Import-based: find target in imported modules via file context
///   3. Scope chain walk: try `{scope}.{target}` for each scope
///   4. Same-file: find target among symbols in the current file
///   5. Qualified name: target contains `.` → direct qname lookup
///
/// Does NOT include a by-name fallback — that's the heuristic resolver's job.
/// This prevents low-confidence false matches from intercepting the heuristic's
/// module-aware matching.
pub fn resolve_common(
    lang_prefix: &'static str,
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
    kind_compatible: fn(EdgeKind, &str) -> bool,
) -> Option<Resolution> {
    let target = &ref_ctx.extracted_ref.target_name;
    let edge_kind = ref_ctx.extracted_ref.kind;

    // Skip import refs — they declare scope, not symbol references.
    if edge_kind == EdgeKind::Imports {
        return None;
    }

    // Step 1: Module-qualified lookup.
    // If ref has module="List" and target="map", try "List.map" as qname.
    if let Some(module) = &ref_ctx.extracted_ref.module {
        // Try direct qualified name: module.target
        let candidates = [
            format!("{module}.{target}"),
            format!("{module}::{target}"),
            format!("{module}/{target}"),
            format!("{module}:{target}"),
        ];
        for candidate in &candidates {
            if let Some(sym) = lookup.by_qualified_name(candidate) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: concat_strategy(lang_prefix, "module_qualified"),
                    });
                }
            }
        }

        // Try finding target in files that match the module name
        let by_name = lookup.by_name(target);
        for sym in by_name {
            let file_lower = sym.file_path.to_lowercase();
            let module_lower = module.to_lowercase();
            // File stem or path segment matches module name
            if file_stem_matches(&file_lower, &module_lower)
                && kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: concat_strategy(lang_prefix, "module_file"),
                });
            }
        }
    }

    // Step 2: Import-based resolution.
    // Check if target matches an imported name, then find it in the imported module.
    for import in &file_ctx.imports {
        let Some(module_path) = &import.module_path else {
            continue;
        };

        // Wildcard import: all names from module are in scope
        if import.is_wildcard {
            let by_name = lookup.by_name(target);
            for sym in by_name {
                let file_lower = sym.file_path.to_lowercase();
                let mod_lower = module_path.to_lowercase();
                let last_seg = mod_lower.rsplit('/').next()
                    .unwrap_or(&mod_lower)
                    .rsplit('.')
                    .next()
                    .unwrap_or(&mod_lower);
                if file_stem_matches(&file_lower, last_seg)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: concat_strategy(lang_prefix, "import"),
                    });
                }
            }
            continue;
        }

        // Named import: target matches the imported name
        let matches_import = import.imported_name == *target
            || import.alias.as_deref() == Some(target);
        if matches_import {
            // Find the symbol in the imported module's file
            let by_name = lookup.by_name(target);
            for sym in by_name {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: concat_strategy(lang_prefix, "import"),
                    });
                }
            }
        }
    }

    // Step 3: Scope chain walk.
    for scope in &ref_ctx.scope_chain {
        let candidate = format!("{scope}.{target}");
        if let Some(sym) = lookup.by_qualified_name(&candidate) {
            if kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: concat_strategy(lang_prefix, "scope_chain"),
                });
            }
        }
    }

    // Step 4: Same-file resolution.
    for sym in lookup.in_file(&file_ctx.file_path) {
        if sym.name == *target && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: concat_strategy(lang_prefix, "same_file"),
            });
        }
    }

    // Step 5: Fully qualified name (target contains dots).
    if target.contains('.') || target.contains("::") || target.contains('/') {
        if let Some(sym) = lookup.by_qualified_name(target) {
            if kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: concat_strategy(lang_prefix, "qualified_name"),
                });
            }
        }
    }

    // No deterministic resolution — let heuristic handle it.
    None
}

/// Common external namespace inference for tier-2 languages.
///
/// Classifies refs as external when:
///   1. The ref is an import (the import path IS the namespace)
///   2. The target name is a known builtin/external
///   3. The target was imported from a non-relative module (bare-name walk)
///   4. A module-qualified ref matches an import path
///   5. Chain-root propagation: if the root of a MemberChain was imported
///      from a non-relative module, the entire chain is external
pub fn infer_external_common(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    is_builtin: fn(&str) -> bool,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;

    // Import refs: the import path is the external namespace.
    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let ns = ref_ctx
            .extracted_ref
            .module
            .as_deref()
            .unwrap_or(target);
        return Some(ns.to_string());
    }

    // Known builtin/stdlib function or type.
    if is_builtin(target) {
        return Some("builtin".to_string());
    }

    // Module-qualified ref where the module matches an import path:
    // the ref comes from an external dependency.
    if let Some(module) = &ref_ctx.extracted_ref.module {
        for import in &file_ctx.imports {
            let Some(module_path) = &import.module_path else {
                continue;
            };
            if import.imported_name == *module
                || module_path.contains(module.as_str())
            {
                return Some(module_path.clone());
            }
        }
    }

    // Bare-name import walk: if the target was imported from a non-relative
    // module, it's external. This handles `import Test.Hspec` → `it` is
    // external, `from django.db import models` → `models` is external, etc.
    //
    // For wildcard imports (`import Module` with is_wildcard=true), any
    // unresolved name *could* come from that module. We classify it as
    // external since resolve() already tried and failed to find it locally.
    let simple = target.split('.').next().unwrap_or(target);
    for import in &file_ctx.imports {
        let Some(module_path) = &import.module_path else {
            continue;
        };
        // Skip relative imports — those are project-internal.
        if module_path.starts_with('.') || module_path.starts_with("crate") {
            continue;
        }

        // Determine if we have a meaningful manifest — an empty manifests map
        // means no ecosystem parser exists for this language (Haskell, OCaml, etc.).
        // In that case, treat non-relative imports as external since we have no
        // way to distinguish project-local from third-party.
        //
        // For per-package isolation (M2), `manifests_for(package_id)` returns
        // only the source file's own package's manifests — so `server/` files
        // don't see deps that only `e2e/` declares.
        let pkg_id = ref_ctx.file_package_id;
        let pkg_manifests = project_ctx.map(|ctx| ctx.manifests_for(pkg_id));
        let has_manifest = pkg_manifests
            .map(|m| !m.is_empty())
            .unwrap_or(false);

        // Named import match: `from foo import Bar` → target "Bar" matches.
        if !import.is_wildcard && import.imported_name == simple {
            if has_manifest {
                if is_manifest_dependency(project_ctx.unwrap(), pkg_id, module_path) {
                    return Some(module_path.clone());
                }
                // Manifest exists but dep not in it — project-internal.
                continue;
            }
            // No manifest at all — conservatively treat non-relative as external.
            return Some(module_path.clone());
        }

        // Alias match: `import qualified Data.Map as Map` → target "Map" matches alias.
        if let Some(alias) = &import.alias {
            if alias == simple {
                if has_manifest {
                    if is_manifest_dependency(project_ctx.unwrap(), pkg_id, module_path) {
                        return Some(module_path.clone());
                    }
                    continue;
                }
                return Some(module_path.clone());
            }
        }

        // Wildcard import: `import Module` (no selective list) — any unresolved
        // bare name could come from this module. Fire when:
        //   (a) manifest confirms the module is a dependency, OR
        //   (b) no manifest parser exists for this language at all
        if import.is_wildcard {
            if has_manifest {
                if is_manifest_dependency(project_ctx.unwrap(), pkg_id, module_path) {
                    return Some(module_path.clone());
                }
            } else {
                // No manifest parser → can't distinguish, but resolve() already
                // failed to find it locally, so classify as external.
                return Some(module_path.clone());
            }
        }
    }

    // Chain-root propagation: if the ref has a MemberChain and the root
    // segment was imported from a non-relative module, classify the whole
    // chain as external.
    if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
        if chain_ref.segments.len() >= 2 {
            let root = &chain_ref.segments[0].name;
            for import in &file_ctx.imports {
                if import.imported_name != root.as_str() {
                    continue;
                }
                let Some(module_path) = &import.module_path else {
                    continue;
                };
                if module_path.starts_with('.') || module_path.starts_with("crate") {
                    continue;
                }
                let is_ext = match project_ctx {
                    Some(ctx) => {
                        is_manifest_dependency(ctx, ref_ctx.file_package_id, module_path)
                    }
                    None => true,
                };
                if is_ext {
                    return Some(format!("{}.*", module_path));
                }
            }
        }
    }

    None
}

/// Check if a module path corresponds to a known dependency in the manifest
/// visible to `package_id`.
///
/// Extracts the root package name from the module path and checks the
/// per-package manifests when `package_id` is known — falls back to the
/// whole-project union for files without a package (root configs, shared
/// scripts) or for legacy single-unit contexts.
///
/// Handles common naming conventions:
///   - Python: `django.db` → root "django", also checks "django" with hyphen swap
///   - Rust: `tokio::runtime` → root "tokio"
///   - Haskell: `Test.Hspec` → root "hspec" (lowercase)
///   - Elixir: `Phoenix.Controller` → root "phoenix" (lowercase)
fn is_manifest_dependency(
    ctx: &ProjectContext,
    package_id: Option<i64>,
    module_path: &str,
) -> bool {
    // Extract the root segment — the part before the first separator.
    let root = module_path
        .split('.')
        .next()
        .and_then(|s| s.split("::").next())
        .unwrap_or(module_path);

    let root_lower = root.to_lowercase();
    let manifests = ctx.manifests_for(package_id);
    for manifest in manifests.values() {
        if manifest.dependencies.contains(root)
            || manifest.dependencies.contains(&root_lower)
            || manifest.dependencies.contains(&root_lower.replace('_', "-"))
            || manifest.dependencies.contains(&root_lower.replace('-', "_"))
        {
            return true;
        }
        // Also check if any dep starts with the root as a prefix
        // (handles scoped packages like `ecto_sql` matching root `Ecto`).
        for dep in &manifest.dependencies {
            if dep.split('_').next() == Some(&root_lower) {
                return true;
            }
        }
    }
    false
}

/// Check if a file path's stem matches a module name.
fn file_stem_matches(file_path_lower: &str, module_lower: &str) -> bool {
    let normalized = file_path_lower.replace('\\', "/");
    // Check file stem: "src/lists.erl" stem is "lists"
    if let Some(basename) = normalized.rsplit('/').next() {
        if let Some(stem) = basename.rsplit_once('.').map(|(s, _)| s) {
            if stem == module_lower {
                return true;
            }
        }
    }
    // Check path segment: "src/lists/mod.rs"
    normalized.split('/').any(|seg| seg == module_lower)
}

/// Strategy name helper — returns a leaked &'static str for diagnostics.
/// Uses a fixed set of known suffixes to avoid allocation.
fn concat_strategy(prefix: &'static str, suffix: &str) -> &'static str {
    // For diagnostics only — use the prefix as a fallback.
    // The full "{prefix}_{suffix}" string can't be &'static without leaking,
    // so we return just the suffix which is always a literal.
    match suffix {
        "module_qualified" => match prefix {
            "erlang" => "erlang_module_qualified",
            "ocaml" => "ocaml_module_qualified",
            "haskell" => "haskell_module_qualified",
            "r" => "r_module_qualified",
            "clojure" => "clojure_module_qualified",
            "pascal" => "pascal_module_qualified",
            "fortran" => "fortran_module_qualified",
            "matlab" => "matlab_module_qualified",
            "powershell" => "powershell_module_qualified",
            "fsharp" => "fsharp_module_qualified",
            _ => "common_module_qualified",
        },
        "module_file" => "common_module_file",
        "import" => "common_import",
        "scope_chain" => "common_scope_chain",
        "same_file" => "common_same_file",
        "qualified_name" => "common_qualified_name",
        _ => "common_resolved",
    }
}

// ---------------------------------------------------------------------------
// ResolutionEngine
// ---------------------------------------------------------------------------

/// The engine that dispatches resolution to language-specific resolvers.
pub struct ResolutionEngine {
    resolvers: FxHashMap<String, Arc<dyn LanguageResolver>>,
}

impl ResolutionEngine {
    /// Create a new engine with the default set of language resolvers.
    pub fn new() -> Self {
        let mut engine = Self {
            resolvers: FxHashMap::default(),
        };
        for resolver in crate::languages::default_resolvers() {
            for &lang_id in resolver.language_ids() {
                engine
                    .resolvers
                    .insert(lang_id.to_string(), Arc::clone(&resolver));
            }
        }
        engine
    }

    /// Get the resolver for a language, if one is registered.
    pub fn resolver_for(&self, language: &str) -> Option<&dyn LanguageResolver> {
        self.resolvers.get(language).map(|r| r.as_ref())
    }
}

// ---------------------------------------------------------------------------
// Chain-aware external inference (shared by all resolvers)
// ---------------------------------------------------------------------------

/// If a ref has a MemberChain, walk it to see if we can determine a type
/// that isn't in our index — meaning the chain leads to an external type.
///
/// For `this.repo.findMany()` where repo has type `PrismaClient`:
/// 1. `this` → `UserService` (from scope chain)
/// 2. `repo` → field_type = `PrismaClient`
/// 3. `PrismaClient` not in index → return Some("PrismaClient")
///
/// This classifies the entire chain call as external with the unresolved
/// type name as the namespace.
pub fn infer_external_from_chain(
    chain: &crate::types::MemberChain,
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    use crate::types::SegmentKind;

    let segments = &chain.segments;
    if segments.len() < 2 {
        return None;
    }

    // Fast path: if the root segment is a known external name (primitive, test
    // framework global), classify the whole chain as external immediately.
    // We use an empty string as the language hint here since `infer_external_from_chain`
    // is language-agnostic; the language-specific primitives are checked inside
    // `is_external_name` only when the language is known.  The test-global check
    // is language-independent, so it still fires correctly.
    if segments[0].kind == SegmentKind::Identifier {
        let root = &segments[0].name;
        // Use an empty language string — this still catches test globals.
        // Primitive checks per language happen in the language-specific resolvers.
        if lookup.is_external_name(root, "") {
            return Some(format!("{}.*", root));
        }
    }

    // Phase 1: Determine root type.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            // Find enclosing class.
            let mut found = None;
            for scope in scope_chain {
                if let Some(sym) = lookup.by_qualified_name(scope) {
                    if matches!(sym.kind.as_str(), "class" | "struct" | "interface") {
                        found = Some(scope.clone());
                        break;
                    }
                }
            }
            found.or_else(|| scope_chain.last().cloned())
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;
            // Field on enclosing class?
            let mut found = None;
            for scope in scope_chain {
                let field_qname = format!("{scope}.{name}");
                if let Some(type_name) = lookup.field_type_name(&field_qname) {
                    found = Some(type_name.to_string());
                    break;
                }
            }
            found.or_else(|| segments[0].declared_type.clone())
        }
        _ => None,
    };

    // Phase 2: Walk the chain checking if the current type is external.
    // If no root type was determined, check if the root identifier itself
    // is external (not in the index, or a variable with no resolvable type).
    //
    // IMPORTANT: all `by_name` lookups in this function must filter out
    // external-origin symbols (file_path starts with "ext:"). Otherwise a
    // Python external type like `sqlalchemy.Table` would match a TS `table`
    // root and convince the walker the chain is "internal", suppressing the
    // external classification for entirely unrelated code.
    let is_internal = |s: &SymbolInfo| -> bool { !s.file_path.starts_with("ext:") };

    let mut current_type = match root_type {
        Some(t) => t,
        None => {
            if segments[0].kind == SegmentKind::Identifier {
                let name = &segments[0].name;
                // Type-like kinds (class/struct/interface/enum/type_alias) are
                // pre-indexed in types_by_name — avoids scanning the full
                // by_name candidate pool (externals can collide by the tens of
                // thousands on common names like "Context"/"Error"/"Request").
                let has_type = lookup.types_by_name(name).iter().filter(|s| is_internal(s)).any(|s| {
                    matches!(
                        s.kind.as_str(),
                        "class" | "struct" | "interface" | "enum" | "type_alias"
                    )
                });
                if has_type {
                    return None;
                }
                // Function/method presence still needs the full by_name pool
                // since those aren't type-kinds — but here we only care whether
                // any exists, and `.any()` short-circuits. `has_in_namespace`
                // would be even cheaper but a bare-name function doesn't live
                // under a namespace prefix. Fall through to by_name but gate
                // behind the cheaper type check above so the fast path wins
                // on hot type names.
                let has_func = lookup.by_name(name).iter().filter(|s| is_internal(s)).any(|s| {
                    matches!(s.kind.as_str(), "function" | "method")
                });
                if has_func {
                    return None;
                }
                return Some(name.clone());
            }
            return None;
        }
    };

    for seg in &segments[1..] {
        // If the current type isn't in the index → it's external.
        // Use types_by_name (pre-filtered type-kind pool) so common external
        // type names like "Context"/"Request"/"Error" don't drag in tens of
        // thousands of non-type candidates on every ref.
        let type_in_index = lookup
            .by_qualified_name(&current_type)
            .filter(|s| is_internal(s))
            .is_some()
            || lookup.types_by_name(&current_type).iter().filter(|s| is_internal(s)).any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "struct" | "interface" | "enum" | "type_alias"
                        | "trait" | "module" | "namespace"
                )
            });

        if !type_in_index {
            return Some(current_type);
        }

        // Try to follow to the next type.
        let member_qname = format!("{current_type}.{}", seg.name);
        if let Some(next) = lookup.field_type_name(&member_qname) {
            current_type = next.to_string();
            continue;
        }
        if let Some(next) = lookup.return_type_name(&member_qname) {
            current_type = next.to_string();
            continue;
        }

        // Can't follow further — the member exists on a known type but
        // we don't know its return type. Not enough info to classify.
        break;
    }

    None
}

// ---------------------------------------------------------------------------
// Helpers for building RefContext
// ---------------------------------------------------------------------------

/// Build the scope chain from a symbol's scope_path.
///
/// scope_path = "A.B.C" → ["A.B.C", "A.B", "A"]
pub fn build_scope_chain(scope_path: Option<&str>) -> Vec<String> {
    let Some(path) = scope_path else {
        return Vec::new();
    };
    if path.is_empty() {
        return Vec::new();
    }

    let mut chain = Vec::new();
    let mut current = path.to_string();
    chain.push(current.clone());

    while let Some(dot_pos) = current.rfind('.') {
        current.truncate(dot_pos);
        chain.push(current.clone());
    }

    chain
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_chain_from_path() {
        let chain = build_scope_chain(Some("A.B.C"));
        assert_eq!(chain, vec!["A.B.C", "A.B", "A"]);
    }

    #[test]
    fn parse_return_type_from_dotnet_signature() {
        assert_eq!(
            parse_return_type_from_signature("Greet(string): string"),
            Some("string".to_string())
        );
        assert_eq!(
            parse_return_type_from_signature("Get<T>(int): Task<T>"),
            Some("Task<T>".to_string())
        );
        assert_eq!(
            parse_return_type_from_signature(
                "Add<K, V>(K, V): Dictionary<K, V>"
            ),
            Some("Dictionary<K, V>".to_string())
        );
        // No trailing colon — Java style leading-return-type. Nothing to extract.
        assert_eq!(parse_return_type_from_signature("String foo()"), None);
        // Constructor — no colon suffix. Parser must not claim `Foo` or `()`.
        assert_eq!(parse_return_type_from_signature("Foo()"), None);
        // Generic with nested angle brackets — colons INSIDE must not fire.
        assert_eq!(
            parse_return_type_from_signature(
                "Map<K: Ord, V>(K): V"
            ),
            Some("V".to_string())
        );
        assert_eq!(parse_return_type_from_signature(""), None);
    }

    #[test]
    fn tuple_element_simple_pair() {
        assert_eq!(
            tuple_element("[boolean, Dispatch<SetStateAction<boolean>>]", 0),
            Some("boolean".to_string())
        );
        assert_eq!(
            tuple_element("[boolean, Dispatch<SetStateAction<boolean>>]", 1),
            Some("Dispatch<SetStateAction<boolean>>".to_string())
        );
    }

    #[test]
    fn tuple_element_handles_whitespace_and_commas_in_generics() {
        assert_eq!(
            tuple_element(" [ A , B<X, Y> , C ] ", 0),
            Some("A".to_string())
        );
        assert_eq!(
            tuple_element(" [ A , B<X, Y> , C ] ", 1),
            Some("B<X, Y>".to_string())
        );
        assert_eq!(
            tuple_element(" [ A , B<X, Y> , C ] ", 2),
            Some("C".to_string())
        );
    }

    #[test]
    fn tuple_element_handles_nested_tuples() {
        assert_eq!(
            tuple_element("[[inner, tuple], outer]", 0),
            Some("[inner, tuple]".to_string())
        );
        assert_eq!(
            tuple_element("[[inner, tuple], outer]", 1),
            Some("outer".to_string())
        );
    }

    #[test]
    fn tuple_element_out_of_range_returns_none() {
        assert_eq!(tuple_element("[A, B]", 2), None);
    }

    #[test]
    fn tuple_element_rejects_non_tuple() {
        assert_eq!(tuple_element("Foo<Bar>", 0), None);
        assert_eq!(tuple_element("string", 0), None);
        assert_eq!(tuple_element("", 0), None);
    }

    #[test]
    fn test_scope_chain_single() {
        let chain = build_scope_chain(Some("Namespace"));
        assert_eq!(chain, vec!["Namespace"]);
    }

    #[test]
    fn test_scope_chain_empty() {
        assert!(build_scope_chain(None).is_empty());
        assert!(build_scope_chain(Some("")).is_empty());
    }

    #[test]
    fn test_symbol_index_by_name() {
        // Build a minimal ParsedFile + symbol_id_map
        let pf = ParsedFile {
            path: "src/foo.cs".to_string(),
            language: "csharp".to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            content: None,
            has_errors: false,
            symbols: vec![
                ExtractedSymbol {
                    name: "Foo".to_string(),
                    qualified_name: "NS.Foo".to_string(),
                    kind: SymbolKind::Class,
                    visibility: Some(Visibility::Public),
                    start_line: 1,
                    end_line: 10,
                    start_col: 0,
                    end_col: 0,
                    signature: None,
                    doc_comment: None,
                    scope_path: Some("NS".to_string()),
                    parent_index: None,
                },
            ],
            refs: vec![],
            routes: vec![],
            db_sets: vec![],
            symbol_origin_languages: vec![],
            ref_origin_languages: vec![],
            symbol_from_snippet: vec![],
        };

        let mut id_map = HashMap::new();
        id_map.insert(("src/foo.cs".to_string(), "NS.Foo".to_string()), 42);

        let index = SymbolIndex::build(&[pf], &id_map);

        // by_name
        let results = index.by_name("Foo");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, 42);
        assert_eq!(results[0].qualified_name, "NS.Foo");

        // by_qualified_name
        let result = index.by_qualified_name("NS.Foo");
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, 42);

        // in_namespace
        let ns_results = index.in_namespace("NS");
        assert_eq!(ns_results.len(), 1);

        // in_file
        let file_results = index.in_file("src/foo.cs");
        assert_eq!(file_results.len(), 1);

        // missing
        assert!(index.by_name("Bar").is_empty());
        assert!(index.by_qualified_name("NS.Bar").is_none());
    }

    fn make_class_sym(name: &str, qname: &str) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind: SymbolKind::Class,
            visibility: None,
            start_line: 1,
            end_line: 2,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        }
    }

    fn make_pf(path: &str, syms: Vec<ExtractedSymbol>) -> ParsedFile {
        ParsedFile {
            path: path.to_string(),
            language: "csharp".to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            content: None,
            has_errors: false,
            symbols: syms,
            refs: vec![],
            routes: vec![],
            db_sets: vec![],
            symbol_origin_languages: vec![],
            ref_origin_languages: vec![],
            symbol_from_snippet: vec![],
        }
    }

    #[test]
    fn test_in_namespace_sorted_multiple() {
        let pf = make_pf(
            "src/a.cs",
            vec![
                make_class_sym("Foo", "NS.Foo"),
                make_class_sym("Bar", "NS.Bar"),
                make_class_sym("Baz", "Other.Baz"),
            ],
        );

        let mut id_map = HashMap::new();
        id_map.insert(("src/a.cs".to_string(), "NS.Foo".to_string()), 1);
        id_map.insert(("src/a.cs".to_string(), "NS.Bar".to_string()), 2);
        id_map.insert(("src/a.cs".to_string(), "Other.Baz".to_string()), 3);

        let index = SymbolIndex::build(&[pf], &id_map);

        let ns_results = index.in_namespace("NS");
        assert_eq!(ns_results.len(), 2);
        let mut ids: Vec<i64> = ns_results.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2]);

        let other_results = index.in_namespace("Other");
        assert_eq!(other_results.len(), 1);
        assert_eq!(other_results[0].id, 3);
    }

    #[test]
    fn test_in_namespace_no_prefix_bleed() {
        // "NS" must not match "NSX.Thing"
        let pf = make_pf(
            "src/a.cs",
            vec![
                make_class_sym("Foo", "NS.Foo"),
                make_class_sym("Thing", "NSX.Thing"),
            ],
        );

        let mut id_map = HashMap::new();
        id_map.insert(("src/a.cs".to_string(), "NS.Foo".to_string()), 1);
        id_map.insert(("src/a.cs".to_string(), "NSX.Thing".to_string()), 2);

        let index = SymbolIndex::build(&[pf], &id_map);

        let ns_results = index.in_namespace("NS");
        assert_eq!(ns_results.len(), 1);
        assert_eq!(ns_results[0].qualified_name, "NS.Foo");
    }

    #[test]
    fn test_in_namespace_empty() {
        let pf = make_pf("src/a.cs", vec![make_class_sym("Foo", "NS.Foo")]);
        let mut id_map = HashMap::new();
        id_map.insert(("src/a.cs".to_string(), "NS.Foo".to_string()), 1);
        let index = SymbolIndex::build(&[pf], &id_map);

        assert!(index.in_namespace("Missing").is_empty());
        assert!(index.in_namespace("N").is_empty()); // "N" is a prefix of "NS" but not "NS."
    }

    #[test]
    fn test_globals_scope_to_declaring_package() {
        use crate::indexer::manifest::{ManifestData, ManifestKind};
        use crate::indexer::project_context::ProjectContext;

        // Two packages: `e2e` declares vitest; `server` declares nothing.
        let mut ctx = ProjectContext::default();
        let mut e2e_npm = ManifestData::default();
        e2e_npm.dependencies.insert("vitest".to_string());
        let mut server_npm = ManifestData::default();
        server_npm.dependencies.insert("express".to_string());
        ctx.by_package.insert(1, [(ManifestKind::Npm, e2e_npm)].into());
        ctx.by_package
            .insert(2, [(ManifestKind::Npm, server_npm)].into());

        // Union must also be populated so union-based callers (no pkg_id
        // context) still classify describe/it correctly.
        let mut union = ManifestData::default();
        union.dependencies.insert("vitest".to_string());
        union.dependencies.insert("express".to_string());
        ctx.manifests.insert(ManifestKind::Npm, union);

        let index = SymbolIndex::build_with_context(&[], &HashMap::new(), Some(&ctx));

        // e2e (pkg 1) sees vitest globals.
        assert!(index.is_test_global_for(Some(1), "describe"));
        assert!(index.is_test_global_for(Some(1), "expect"));
        // server (pkg 2) does NOT see them — vitest wasn't declared there.
        assert!(!index.is_test_global_for(Some(2), "describe"));
        assert!(!index.is_test_global_for(Some(2), "expect"));
        // Root-scoped file (no package_id) falls back to union.
        assert!(index.is_test_global_for(None, "describe"));
    }

    #[test]
    fn test_globals_empty_when_no_framework_declared() {
        let index = SymbolIndex::build(&[], &HashMap::new());
        assert!(!index.is_test_global_for(None, "describe"));
        assert!(!index.is_test_global_for(Some(1), "describe"));
    }
}

// ---------------------------------------------------------------------------
// Test-framework globals — manifest-declared → exported global set
// ---------------------------------------------------------------------------

/// Map a manifest-declared test-framework dep name to the set of globals it
/// exposes to source files at test time.
///
/// Only covers the runtime-global case: frameworks whose helpers are
/// injected as free identifiers rather than imported. Python's unittest
/// / pytest do NOT qualify here — their helpers come through explicit
/// imports and are resolved by the normal import-aware path.
fn test_framework_globals(dep: &str) -> &'static [&'static str] {
    match dep {
        // JS/TS — vitest + jest overlap in the BDD surface.
        // Includes assertion/mock methods that appear as bare calls after fluent chains
        // (e.g., `expect(x).toEqual(y)` emits `toEqual` as a standalone calls ref).
        "vitest" | "jest" | "@jest/globals" => &[
            "describe", "xdescribe", "fdescribe", "it", "xit", "fit", "test", "expect",
            "beforeEach", "afterEach", "beforeAll", "afterAll",
            "vi", "jest",
            // jest/vitest assertion methods accessed via chain
            "toEqual", "toStrictEqual", "toBe", "toBeTrue", "toBeFalse",
            "toBeNull", "toBeUndefined", "toBeTruthy", "toBeFalsy",
            "toBeDefined", "toBeNaN", "toBeGreaterThan",
            "toBeGreaterThanOrEqual", "toBeLessThan", "toBeLessThanOrEqual",
            "toBeCloseTo", "toContain", "toContainEqual", "toHaveLength",
            "toHaveProperty", "toMatch", "toMatchObject", "toMatchSnapshot",
            "toMatchInlineSnapshot", "toThrow", "toThrowError",
            "toThrowErrorMatchingSnapshot", "toThrowErrorMatchingInlineSnapshot",
            "toBeInstanceOf",
            "toHaveBeenCalled", "toHaveBeenCalledTimes", "toHaveBeenCalledWith",
            "toHaveBeenCalledOnce", "toHaveBeenLastCalledWith",
            "toHaveBeenNthCalledWith", "toHaveReturned", "toHaveReturnedTimes",
            "toHaveReturnedWith", "toHaveLastReturnedWith", "toHaveNthReturnedWith",
            // DOM matchers (jest-dom / @testing-library)
            "toHaveClass", "toHaveAttr", "toHaveText", "toContainText",
            "toBeVisible", "toBeDisabled", "toBeEnabled", "toBeInTheDocument",
            "toHaveValue", "toHaveStyle", "toHaveFocus",
            // asymmetric matchers
            "anything", "any", "objectContaining", "arrayContaining",
            "stringContaining", "stringMatching",
            // spy/mock methods
            "spyOn", "mockClear", "mockReset", "mockRestore",
            "mockImplementation", "mockImplementationOnce",
            "mockReturnValue", "mockReturnValueOnce",
            "mockResolvedValue", "mockResolvedValueOnce",
            "mockRejectedValue", "mockRejectedValueOnce",
            "mockFn", "fn",
        ],
        "mocha" => &["describe", "it", "before", "after", "beforeEach", "afterEach"],
        "chai" => &[
            "expect", "assert", "should",
            // chai BDD assertion methods — emitted as bare calls from fluent chains
            // (e.g., `expect(x).to.equal(y)` emits `equal` as a standalone calls ref)
            "equal", "eql", "deep", "include", "contain", "members", "keys",
            "property", "match", "satisfy", "closeTo", "approximately",
            "above", "below", "least", "most", "within", "instanceof",
            "an", "ok", "true", "false", "null", "undefined", "NaN",
            "exist", "empty", "arguments", "throw", "respondTo", "itself",
            "change", "increase", "decrease", "lengthOf", "oneOf",
            "equalNode", "equalDom", "equalHtml",
        ],
        "ava" => &["test"],
        "jasmine" | "jasmine-core" | "karma-jasmine" | "@angular-devkit/build-angular" => &[
            // lifecycle
            "describe", "xdescribe", "fdescribe", "it", "xit", "fit",
            "expect", "fail", "pending",
            "beforeEach", "afterEach", "beforeAll", "afterAll",
            // namespace object (jasmine.createSpy etc.)
            "jasmine",
            // matchers — emitted as bare chain calls
            "toEqual", "toStrictEqual", "toBe", "toBeTrue", "toBeFalse",
            "toBeTruthy", "toBeFalsy", "toBeDefined", "toBeUndefined",
            "toBeNull", "toBeNaN", "toBeGreaterThan", "toBeGreaterThanOrEqual",
            "toBeLessThan", "toBeLessThanOrEqual", "toBeCloseTo",
            "toContain", "toContainEqual", "toMatch", "toMatchObject",
            "toHaveLength", "toHaveProperty",
            "toHaveBeenCalled", "toHaveBeenCalledTimes", "toHaveBeenCalledWith",
            "toHaveBeenCalledOnce", "toHaveBeenLastCalledWith",
            "toHaveBeenNthCalledWith",
            "toBeInstanceOf", "toThrow", "toThrowError",
            // DOM/Angular-testing-library matchers
            "toHaveClass", "toHaveAttr", "toHaveText", "toContainText",
            "toBeVisible", "toBeDisabled", "toBeEnabled",
            // spy / mock helpers
            "spyOn", "spyOnProperty", "createSpy", "createSpyObj",
            "callThrough", "callFake", "returnValue", "returnValues",
            "stub", "restore",
            // fluent chain nodes emitted as bare refs
            "and", "calls",
            // asymmetric matchers
            "anything", "any", "objectContaining", "arrayContaining",
            "stringContaining", "stringMatching",
        ],
        // Bun's built-in test runner — same shape as vitest.
        "bun-types" => &[
            "describe", "it", "test", "expect",
            "beforeEach", "afterEach", "beforeAll", "afterAll",
        ],
        _ => &[],
    }
}

/// Build the project-wide union of test globals from every ecosystem manifest.
fn build_test_globals_union(
    project_ctx: Option<&crate::indexer::project_context::ProjectContext>,
) -> HashSet<String> {
    let mut globals = HashSet::new();
    let Some(ctx) = project_ctx else { return globals };
    for manifest in ctx.manifests.values() {
        for dep in &manifest.dependencies {
            for g in test_framework_globals(dep) {
                globals.insert((*g).to_string());
            }
        }
    }
    globals
}

/// Build per-package test globals keyed by `packages.id`.
///
/// Only packages that directly declare a test framework in their own manifest
/// get entries — sibling packages stay clean.
fn build_test_globals_by_pkg(
    project_ctx: Option<&crate::indexer::project_context::ProjectContext>,
) -> FxHashMap<i64, HashSet<String>> {
    let mut out: FxHashMap<i64, HashSet<String>> = FxHashMap::default();
    let Some(ctx) = project_ctx else { return out };
    for (&pkg_id, manifests) in &ctx.by_package {
        let mut set = HashSet::new();
        for manifest in manifests.values() {
            for dep in &manifest.dependencies {
                for g in test_framework_globals(dep) {
                    set.insert((*g).to_string());
                }
            }
        }
        if !set.is_empty() {
            out.insert(pkg_id, set);
        }
    }
    out
}
