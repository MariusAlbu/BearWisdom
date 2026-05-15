// =============================================================================
// indexer/resolve/engine/lookup.rs — SymbolLookup trait
//
// The read-only contract that decouples the resolve loop and language
// resolvers from the SymbolIndex implementation. Default impls cover the
// common case so synthetic test doubles only have to implement the methods
// they care about.
// =============================================================================

use crate::types::AliasTarget;

use super::{ChainMiss, SymbolInfo};

// ---------------------------------------------------------------------------
// SymbolLookup trait — decouples resolvers from index internals
// ---------------------------------------------------------------------------

/// Read-only access to the global symbol index.
pub trait SymbolLookup {
    /// Find all symbols with the given simple name.
    fn by_name(&self, name: &str) -> &[SymbolInfo];

    /// Find a symbol by exact qualified name.
    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo>;

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
    fn all_by_qualified_name(&self, qname: &str) -> &[SymbolInfo] {
        std::slice::from_ref(match self.by_qualified_name(qname) {
            Some(s) => s,
            None => return &[],
        })
    }

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
    /// Chain-classification filters in `infer_external_from_chain` use this
    /// to tell user source from vendored symbols with matching names.
    fn is_external_file(&self, path: &str) -> bool {
        path.starts_with("ext:")
    }

    /// Is `name` declared as a member (method or property) on any interface
    /// in an ambient-global declaration file (`typescript/lib/lib.*.d.ts`,
    /// `@types/node`)? Those declarations ARE the JS/DOM/ES runtime
    /// surface — a name found there is definitely a runtime API call on
    /// an untyped receiver, regardless of which interface we can't prove.
    ///
    /// Callers use this as an external-classification signal: `x.addEventListener(...)`
    /// where `x`'s type can't be inferred still resolves to "external (DOM)"
    /// because `addEventListener` only exists on `EventTarget`/`HTMLElement`/etc.,
    /// all in lib.dom.d.ts. Replaces the hardcoded `is_common_builtin_method`
    /// list — the index is the source of truth.
    fn is_ambient_global_method(&self, _name: &str) -> bool {
        false
    }

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

    /// Look up the class qualified name for an Angular component selector.
    ///
    /// `raw_selector` is the selector as stored in `@Component({selector:'...'})`,
    /// post-normalization (brackets/dots stripped).  Element selectors look like
    /// `"app-user-card"`; attribute selectors like `"appHighlight"`.
    ///
    /// Returns `Some(qname)` when a `@Component` class with that selector was
    /// indexed.  Default returns `None` — test lookups and non-Angular
    /// projects pay no cost.
    fn angular_selector(&self, _raw_selector: &str) -> Option<&str> {
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

    /// Record a chain walker bail-out for the R3 second-pass reload.
    ///
    /// Called by `crate::type_checker::chain::resolve_via_chain` when it resolved
    /// `current_type` but couldn't continue because the next segment isn't
    /// indexed under it. Default impl is a no-op so test/synthetic lookups
    /// don't have to opt in.
    ///
    /// `SymbolIndex` overrides this with an interior-mutable buffer drained
    /// by `take_chain_misses` after the main resolution loop, so that the
    /// indexer can drive `Ecosystem::resolve_symbol` on demand and re-resolve
    /// only the affected refs.
    fn record_chain_miss(&self, _miss: ChainMiss) {}

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

    /// Install a fresh local-type cache for the next file's resolution pass.
    /// `narrowings` should be pre-sorted innermost-first (smallest range first).
    fn install_local_cache(&self, _narrowings: Vec<crate::types::Narrowing>) {}

    /// Move the cache cursor to the given byte offset. The resolver calls
    /// this before each ref so narrowing lookups see the correct byte range.
    fn set_cursor(&self, _byte: u32) {}

    /// Record a successfully-inferred local-variable type. Called by the
    /// resolver after a flow-binding ref resolves with a non-`None`
    /// `resolved_yield_type`.
    fn record_local_type(&self, _name: String, _type_name: String) {}

    /// Clear the cache at end of file. Keeps leftover bindings from bleeding
    /// into the next file's pass.
    fn clear_local_cache(&self) {}
}
