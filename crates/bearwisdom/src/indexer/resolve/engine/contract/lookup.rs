// =============================================================================
// indexer/resolve/engine/lookup.rs — SymbolLookup trait
//
// The read-only contract that decouples the resolve loop and language
// resolvers from the SymbolIndex implementation. Default impls cover the
// common case so synthetic test doubles only have to implement the methods
// they care about.
// =============================================================================

use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::types::AliasTarget;

use super::{Symbol, SymbolSet};

// ---------------------------------------------------------------------------
// SymbolLookup trait — decouples resolvers from index internals
// ---------------------------------------------------------------------------

/// Read-only access to the global symbol index.
pub trait SymbolLookup {
    /// Find all symbols with the given simple name.
    fn by_name(&self, name: &str) -> SymbolSet<'_>;

    /// Find a symbol by exact qualified name.
    fn by_qualified_name(&self, qname: &str) -> Option<&Symbol>;

    /// Find every symbol sharing the exact qualified name, in insertion
    /// order. Default returns the one-winner slice of `by_qualified_name`.
    ///
    /// TypeScript declaration merging exports interface + variable under the
    /// same qname (e.g. `@angular/core.Injectable` is both the decorator
    /// function AND the options-type interface). `by_qualified_name` picks
    /// whichever lost the first-wins race; when the caller needs a specific
    /// kind (e.g. a `Calls` ref against a `variable`/`function`/`class`),
    /// the interface overload is useless. This lookup exposes all
    /// overloads so the caller can scan for a kind-compatible target.
    fn all_by_qualified_name(&self, qname: &str) -> SymbolSet<'_> {
        match self.by_qualified_name(qname) {
            Some(s) => SymbolSet::Borrowed(std::slice::from_ref(s)),
            None => SymbolSet::empty(),
        }
    }

    /// Find the direct children of a type/namespace by exact parent qualified name.
    ///
    /// For `parent_qname = "context.Context"`, returns all symbols whose
    /// qualified_name is `context.Context.X` (one dot deeper) — methods,
    /// fields, nested types. Chain walkers use this to locate the next
    /// segment of a member chain without scanning every candidate that
    /// shares a simple name across the project + externals.
    fn members_of(&self, parent_qname: &str) -> SymbolSet<'_>;

    /// Direct children of a type/namespace by its symbol id — the id-keyed
    /// counterpart to `members_of`. A chain walker that has typed a receiver to
    /// a symbol id walks its members by identity instead of re-matching qname
    /// strings. Default returns empty; the real store overrides it.
    fn members_of_id(&self, _parent_id: i64) -> SymbolSet<'_> {
        SymbolSet::empty()
    }

    /// Find all type-kind symbols (class, struct, interface, enum, ...) with
    /// the given simple name.
    ///
    /// Exists so `is-this-name-a-type?` checks in chain walkers don't iterate
    /// every non-type symbol that happens to share the name (common words
    /// like `String`, `Error`, `Context` collect thousands of non-type
    /// candidates across an indexed stdlib/externals set).
    fn types_by_name(&self, name: &str) -> SymbolSet<'_>;

    /// Find all symbols whose qualified name starts with the given prefix + ".".
    fn in_namespace(&self, namespace: &str) -> Vec<&Symbol>;

    /// Cheap existence check: does any symbol live under this namespace?
    /// O(log N), no allocation. Prefer this over `!in_namespace(x).is_empty()`.
    fn has_in_namespace(&self, namespace: &str) -> bool;

    /// Find all symbols defined in a specific file.
    fn in_file(&self, file_path: &str) -> SymbolSet<'_>;

    /// Resolve a module specifier in the context of a specific source file
    /// and return the symbols of the target module.
    ///
    /// Necessary for relative specifiers (`./utils`, `../shared`) where the
    /// resolution depends on the source file's directory — `./utils` from
    /// `apps/web/foo.ts` and `apps/web/bar/baz.ts` are different files.
    /// Default impl falls back to `in_file(spec)` for callers (and indexes)
    /// that don't carry per-source resolution data.
    fn in_module_from(&self, _source_file: &str, spec: &str) -> SymbolSet<'_> {
        self.in_file(spec)
    }

    /// Look up the resolved file path for a module specifier in the context
    /// of a specific source file. Returns `None` when no resolution is
    /// known. Used by re-export following so chain hops can also be
    /// resolved per-source.
    fn resolve_module_from(&self, _source_file: &str, _spec: &str) -> Option<&str> {
        None
    }

    /// Get the annotated type name for a property/field symbol.
    /// e.g., "AlbumService.db" → Some("DatabaseRepository").
    fn field_type_name(&self, property_qname: &str) -> Option<&str>;

    /// Get the annotated return type for a method/function symbol.
    /// e.g., "UserRepo.findOne" → Some("User").
    fn return_type_name(&self, method_qname: &str) -> Option<&str>;

    /// Get the generic type parameter names for a type declaration.
    /// e.g., "Repository" → Some(["T"]) for `interface Repository<T>`
    fn generic_params(&self, type_name: &str) -> Option<&[String]>;

    /// Canonical TypeId form of `field_type_name`. Returns `Some(id)` when
    /// the property's field type has been interned into the workspace arena.
    /// Default returns `None` so synthetic test lookups don't have to opt in.
    fn field_type_id(&self, _property_qname: &str) -> Option<TypeId> {
        None
    }

    /// Canonical TypeId form of `return_type_name`. Returns `Some(id)` when
    /// the method's return type has been interned into the workspace arena.
    /// Default returns `None` so synthetic test lookups don't have to opt in.
    fn return_type_id(&self, _method_qname: &str) -> Option<TypeId> {
        None
    }

    /// Return type keyed by the callable's SYMBOL ID rather than its qualified
    /// name. A public API duplicated across packages shares one qname, so
    /// `return_type_id` (qname-keyed) returns the first-winner's type for every
    /// copy; this id-keyed form lets a caller that has resolved the import-scoped
    /// callee read THAT declaration's return. Default `None` so synthetic test
    /// lookups need not opt in.
    fn return_type_id_of(&self, _symbol_id: i64) -> Option<TypeId> {
        None
    }

    /// Borrow the workspace TypeArena that owns every TypeId returned by
    /// `field_type_id` / `return_type_id`. Returns
    /// `None` for synthetic test lookups that haven't opted into the
    /// TypeId surface.
    fn type_arena(&self) -> Option<&TypeArena> {
        None
    }

    /// Render the field type for `qname` from the canonical TypeArena.
    /// Replaces `field_type_name` for consumers that want to drive their
    /// chain walks through the TypeId surface. Falls back to the legacy
    /// string accessor for synthetic lookups that haven't bound an arena.
    fn field_type_str(&self, qname: &str) -> Option<String> {
        if let (Some(id), Some(arena)) = (self.field_type_id(qname), self.type_arena()) {
            return Some(arena.format_type(id));
        }
        self.field_type_name(qname).map(|s| s.to_string())
    }

    /// Render the return type for `qname` from the canonical TypeArena.
    fn return_type_str(&self, qname: &str) -> Option<String> {
        if let (Some(id), Some(arena)) = (self.return_type_id(qname), self.type_arena()) {
            return Some(arena.format_type(id));
        }
        self.return_type_name(qname).map(|s| s.to_string())
    }

    /// Look up the structural shape of a type alias.
    ///
    /// Returns `Some(&AliasTarget)` when `name` is a registered alias
    /// (currently only the TypeScript extractor classifies its aliases
    /// this finely; other languages get a derived `Application` shape
    /// or `None`). Used by `crate::type_checker::alias::expand_alias`
    /// at the top of every chain segment iteration so the walker can
    /// follow `type UserMap = Map<string, User>` aliases through to
    /// the underlying concrete head and type args.
    ///
    /// Default returns `None` so synthetic test lookups don't have to
    /// opt in.
    fn alias_target(&self, _name: &str) -> Option<&crate::types::AliasTarget> {
        None
    }

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

    /// Check whether a file path is known external-origin.
    ///
    /// Two signals combine: the historical `ext:` path-prefix convention
    /// (used by the npm / nuget / go / maven / pypi external pipelines)
    /// AND the explicit external-paths set (seeded from the DB for files
    /// written with `origin='external'` that kept a project-relative path,
    /// e.g. script-tag-discovered vendor JS like `wwwroot/lib/jquery.min.js`).
    ///
    /// Chain-classification filters use this
    /// to tell user source from vendored symbols with matching names.
    fn is_external_file(&self, path: &str) -> bool {
        path.starts_with("ext:")
    }

    /// The symbols a package contributes to *ambient scope* under `name` —
    /// global declarations referenceable without an import.
    ///
    /// Ambient membership is a structural fact the ecosystem layer records at
    /// materialization: a top-level declaration in a `lib.*.d.ts`, a `declare
    /// global` block, an `@types` global; a language's prelude / builtins /
    /// universe. The ambient-scope rung binds a bare reference to one of these
    /// when no more specific rule bound it, so resolution works for any
    /// language whose ecosystem populated the scope.
    ///
    /// Default empty — a lookup with no flagged ambient symbols contributes
    /// nothing here.
    fn ambient_symbols(&self, _name: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }

    /// Does `path` lie inside a package declared as ambient by the project's
    /// own configuration (TypeScript `tsconfig.json#compilerOptions.types`,
    /// `@types/*` packages, or `globals.d.ts` files)?
    ///
    /// Ambient packages contribute symbols the user can reference without an
    /// `import` statement. The DefaultResolver's `ambient_package` strategy
    /// uses this to prefer candidates inside an ambient-declared package
    /// when a bare-name ref could otherwise match thousands of identically-
    /// named symbols across the project.
    ///
    /// Default returns `false` — synthetic test lookups and non-TS projects
    /// pay nothing.
    fn is_ambient_path(&self, _path: &str) -> bool {
        false
    }

    /// Walk the cross-package re-export chain starting from `module_path` (a
    /// bare specifier the user imported `target_name` from), looking for the
    /// package that actually owns the symbol.
    ///
    /// User imports `TSESTree` from `@typescript-eslint/utils`, but the
    /// definition lives in `@typescript-eslint/types` (`utils` re-exports
    /// from `types`). The plain import strategy fails because the candidate
    /// file is in a different package than the import specifier; this hop
    /// walks the re-export graph and accepts a candidate whose file lives
    /// in any reachable package.
    ///
    /// `target_name` is the symbol to resolve, `chain_prefix` is the dotted
    /// prefix (or equal to `target_name` for the direct shape). Returns the
    /// symbol id when found in any reachable package, `None` otherwise.
    /// Default returns `None`.
    fn resolve_external_reexport(
        &self,
        _target_name: &str,
        _chain_prefix: &str,
        _module_path: &str,
    ) -> Option<i64> {
        None
    }

    /// Return all symbols belonging to a workspace package.
    ///
    /// Used by language resolvers to scope lookups when an import specifier
    /// matches a sibling package's `declared_name`. Returns an empty slice
    /// when the package isn't known (e.g. single-project layouts).
    fn symbols_in_package(&self, _package_id: i64) -> SymbolSet<'_> {
        SymbolSet::empty()
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

    /// Resolve an import specifier through the project's declared path
    /// aliases. Returns the rewritten bare path (e.g. `@/utils` → `src/utils`,
    /// `$lib/x` → `src/lib/x`) or `None` when no alias matches.
    ///
    /// The alias table is populated per ecosystem from whatever config
    /// declares it — TS `tsconfig.json#paths`, `jsconfig.json`, framework
    /// configs — so the resolver tower can rewrite aliased specifiers without
    /// baking any one config format into the language-agnostic path.
    fn resolve_path_alias(&self, _package_id: Option<i64>, _specifier: &str) -> Option<String> {
        None
    }

    /// Look up the class qualified name for a component/directive selector.
    ///
    /// `raw_selector` is the selector as stored in `@Component({selector:'...'})`
    /// / `@Directive({selector:'...'})`, post-normalization (brackets/dots
    /// stripped). Element selectors look like `"app-user-card"`; attribute
    /// selectors like `"appHighlight"`.
    ///
    /// Returns `Some(qname)` when a decorated class with that selector was
    /// indexed. Default returns `None` — test lookups and projects with no
    /// selector map pay no cost.
    fn selector_qname(&self, _raw_selector: &str) -> Option<&str> {
        None
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

    /// ALL direct parent heads for a class — the multi-parent, qname-keyed
    /// counterpart of `parent_class_qname` (which yields only the first). A type
    /// extending several supertypes (`interface A extends X, Y, Z`) exposes each,
    /// so a member walk that must inspect a SPECIFIC supertype (e.g. a mapped-type
    /// alias parent whose members come from its source) can find it regardless of
    /// declaration order. Default empty; the real store overrides it.
    fn parent_class_qnames(&self, _class_qname: &str) -> &[String] {
        &[]
    }

    /// The direct parent class's symbol id for a child symbol id — the id-keyed
    /// counterpart of `parent_class_qname`. A chain walker that has typed a
    /// receiver to a symbol id climbs its supertype chain by identity, so a
    /// class extending a base whose qname is shared by an unrelated type in
    /// another package climbs to the SPECIFIC base recorded for this child, not
    /// whichever same-named type won a first-wins qname race. Default returns
    /// `None`; the real store overrides it.
    fn parent_class_id(&self, _child_id: i64) -> Option<i64> {
        None
    }

    /// ALL direct parent symbol ids for a child symbol id — the multi-parent
    /// counterpart of `parent_class_id`. An interface or class can extend /
    /// implement several supertypes, and a member may be declared on any of them,
    /// so the chain walker climbs the supertype DAG breadth-first over this set.
    /// Default derives the single `parent_class_id` (0-or-1 parent); the real
    /// store overrides it with every recorded direct parent.
    fn parent_class_ids(&self, child_id: i64) -> Vec<i64> {
        self.parent_class_id(child_id).into_iter().collect()
    }

    /// The generic type arguments on the `extends`/`implements` edge from
    /// `child_head` to its direct supertype `parent_head`: `["User"]` for
    /// `class Child extends Base<User>`. Empty when the edge carries no
    /// arguments or no such edge is recorded. Lets a member found on a generic
    /// supertype bind that supertype's parameters (`Base.m: T` → `User`), since
    /// the arguments live on the edge, not on the receiver. Default empty; the
    /// real store overrides it.
    fn parent_class_args(&self, _child_head: &str, _parent_head: &str) -> &[String] {
        &[]
    }

    /// The qualified name of the nearest type (class/struct/interface/trait/enum)
    /// that structurally encloses the symbol named `source_qname`, excluding the
    /// symbol itself — Roslyn's `ContainingType`.
    ///
    /// Derived from the symbol's `parent_index` chain at build time, so it is
    /// immune to qname-construction bugs and selects the enclosing type by
    /// *kind* rather than by position in a flattened scope chain. Returns `None`
    /// for top-level symbols, files with no enclosing type, and synthetic test
    /// lookups that don't opt in.
    fn enclosing_type_qname(&self, _source_qname: &str) -> Option<&str> {
        None
    }

    /// The qualified name of the nearest namespace/module that structurally
    /// encloses the symbol named `source_qname`, excluding the symbol itself —
    /// Roslyn's `ContainingNamespace`. Same construction as
    /// `enclosing_type_qname`; default `None`.
    fn enclosing_namespace_qname(&self, _source_qname: &str) -> Option<&str> {
        None
    }

    /// Record a chain walker bail-out for the R3 second-pass reload.
    ///
    /// Called by `crate::type_checker::chain::resolve_via_chain` when it resolved
    /// the receiver type but couldn't continue because the next segment isn't
    /// indexed under it. `target_name` is that next segment. Default impl is a
    /// no-op so test/synthetic lookups don't have to opt in.
    ///
    /// `SymbolIndex` overrides this to mark the file currently being resolved
    /// as part of the next pass's frontier — the only files whose resolution
    /// can change once inline externals materialization adds members.
    fn record_chain_miss(&self, _target_name: &str) {}

    // -------------------------------------------------------------------
    // Per-file flow-typing cache (R5).
    //
    // The resolver calls `install_local_cache` at the start of each file,
    // moves `set_cursor` before resolving each ref, and calls
    // `record_local_type` after a resolve succeeds with a yield type. Chain
    // walkers consult `local_type` first in Phase 1 so a local variable's
    // inferred type takes precedence over same-named globals.
    //
    // All methods default to no-ops — synthetic test lookups and
    // non-SymbolIndex impls don't have to opt in.
    // -------------------------------------------------------------------

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

    /// Clear the cache at end of file. Keeps leftover bindings from bleeding
    /// into the next file's pass.
    fn clear_local_cache(&self) {}
}
