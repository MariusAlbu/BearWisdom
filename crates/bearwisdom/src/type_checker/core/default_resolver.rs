// =============================================================================
// type_checker/core/default_resolver.rs — language-agnostic engine resolver tower
//
// The deterministic strategies a language plugin can compose to resolve a
// chain-less ref without falling through to the tier-2 heuristic.
//
// Every strategy returns Option<Resolution> at confidence 1.0. Each is keyed
// on data the project itself supplies — the extractor's import table, the
// extractor-set `r.module`, the symbol index's qname/by_name/scope_chain
// surface — and never guesses. The one strategy that bare-name guesses
// (`heuristic_name_kind`) is intentionally absent: bare-name + kind-match
// is not a strategy we ship in the engine.
//
// Hook authors call individual methods in whatever order makes sense for
// the language. The canonical ordering (most specific evidence first) is
// documented per method. Strategies are pure functions on the resolver's
// context — same inputs always produce the same output.
// =============================================================================

use std::borrow::Cow;
use std::str::FromStr;

use super::reexport::follow_reexports;
use crate::indexer::resolve::engine::{
    FileContext, RefContext, Resolution, SymbolInfo, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::type_checker::profile::language_profile::{
    AliasDecode, AmbientGlobals, CandidateDirs, ChainQualification, ExtMatch, ExternalByImport,
    FileScopedImports, HeadAliasBind, ImportResolution, KindCompatibility, KindTable, ModuleAnchor,
    ModuleAnchorBind, ModulePrefixRewrites, ModuleScope, NameNormalization, NameTransform, NormSpec,
    RelativeMarker, SelectorResolution, StemMatch, StemSource, WildcardMatch,
};
use crate::types::{EdgeKind, SymbolKind};

/// Language-agnostic engine resolver. Composes deterministic strategies
/// out of the data the project ships: extractor-produced imports,
/// extractor-set module prefixes, the symbol index's qname / by_name /
/// scope_chain surface.
///
/// Construct fresh per ref; methods are pure functions of the context.
pub struct DefaultResolver<'a> {
    /// File-level context: imports, file namespace, file path.
    pub file_ctx: &'a FileContext,
    /// The ref being resolved.
    pub ref_ctx: &'a RefContext<'a>,
    /// Read-only symbol index access.
    pub lookup: &'a dyn SymbolLookup,
    /// Predicate the language uses to decide whether a candidate's `kind`
    /// is a plausible target for the ref's edge kind. Pass `|_, _| true`
    /// to accept any kind.
    pub kind_compatible: fn(EdgeKind, &str) -> bool,
}

/// The pure-data profile deltas the ladder reads beyond the kind predicate,
/// separators, and import-resolution. Bundled so `run_ladder` keeps one
/// argument for "everything the profile contributes to module-anchored and
/// external binding plus the self-keyword strip". The fn-pointer `resolve_all`
/// path passes `INERT`, leaving every delta off (byte-identical to the prior
/// ladder).
#[derive(Clone, Copy)]
struct LadderProfileData<'p> {
    module_skip: Option<fn(&str) -> bool>,
    module_anchor: ModuleAnchor,
    module_anchor_terminal: bool,
    relative_marker: RelativeMarker,
    external_by_import: Option<&'p ExternalByImport>,
    ext_match: ExtMatch,
    self_keywords: &'p [&'p str],
    ambient_namespace_prefixes: &'p [&'p str],
    name_normalization: NameNormalization,
    module_scope: ModuleScope,
    wildcard_match: WildcardMatch,
    head_alias: HeadAliasBind,
    file_scoped_imports: FileScopedImports,
    alias_module_qname: bool,
    module_prefix_rewrites: ModulePrefixRewrites,
    workspace_packages: bool,
    overload_pick_all: bool,
    ambient_globals: AmbientGlobals,
    namespaceless_global_type_lookup: bool,
    explicit_member_import: bool,
    selector_resolution: Option<&'p SelectorResolution>,
}

impl LadderProfileData<'static> {
    /// All deltas off — the fn-pointer ladder and any language whose profile
    /// opts into none of them.
    const INERT: LadderProfileData<'static> = LadderProfileData {
        module_skip: None,
        module_anchor: ModuleAnchor::Off,
        module_anchor_terminal: false,
        relative_marker: RelativeMarker::None,
        external_by_import: None,
        ext_match: ExtMatch::PkgSegment,
        self_keywords: &[],
        ambient_namespace_prefixes: &[],
        name_normalization: NameNormalization::None,
        module_scope: ModuleScope::Off,
        wildcard_match: WildcardMatch::QnameUnder,
        head_alias: HeadAliasBind::Off,
        file_scoped_imports: FileScopedImports::Off,
        alias_module_qname: false,
        module_prefix_rewrites: ModulePrefixRewrites::Off,
        workspace_packages: false,
        overload_pick_all: false,
        ambient_globals: AmbientGlobals::Off,
        namespaceless_global_type_lookup: false,
        explicit_member_import: false,
        selector_resolution: None,
    };
}

impl<'a> DefaultResolver<'a> {
    /// Strategy 1 — module-qualified resolution via `r.module`.
    ///
    /// The extractor recorded an explicit module prefix on the ref —
    /// Erlang `lists:map`, OCaml `List.map`, R `dplyr::mutate`. Tries:
    /// (a) `module.target`, `module::target`, `module/target`, `module:target`
    ///     as exact qualified names, then (b) any `target` candidate whose
    ///     file stem matches the module name.
    ///
    /// Returns None when no module is set or no candidate matches.
    pub fn resolve_via_ref_module(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let module = self.ref_ctx.extracted_ref.module.as_deref()?;

        for sep in [".", "::", "/", ":"] {
            let qname = format!("{module}{sep}{target}");
            if let Some(sym) = self.lookup.by_qualified_name(&qname) {
                if kind(edge_kind, &sym.kind) {
                    return Some(self.resolution(sym.id, "default_ref_module"));
                }
            }
        }

        let module_lower = module.to_lowercase();
        let last_seg_lower = module.rsplit('.').next().unwrap_or(module).to_lowercase();
        for sym in self.lookup.by_name(target) {
            if !kind(edge_kind, &sym.kind) {
                continue;
            }
            let file_lower = sym.file_path.to_lowercase();
            if path_stem_matches(&file_lower, &module_lower)
                || path_stem_matches(&file_lower, &last_seg_lower)
            {
                return Some(self.resolution(sym.id, "default_ref_module"));
            }
        }

        None
    }

    /// Strategy 2 — exact qualified-name match on a dotted target.
    ///
    /// Target like `Catalog.CatalogService.List` or `tokio::runtime::spawn`
    /// → look it up directly. The extractor produced the dotted form
    /// because that's the syntactic shape of the call — if it matches a
    /// symbol's qname exactly, that's the answer.
    pub fn resolve_via_qname_exact(
        &self,
        overload_pick_all: bool,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
        norm: NameNormalization,
    ) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        if !target.contains('.') && !target.contains("::") && !target.contains('/') {
            return None;
        }
        // Declaration-merging: when a language exposes interface + variable
        // under the same qname, scan every overload for the first kind-
        // compatible one rather than `by_qualified_name`'s first-wins pick.
        if overload_pick_all {
            for sym in self.lookup.all_by_qualified_name(target) {
                if kind(edge_kind, &sym.kind) {
                    return Some(self.resolution(sym.id, "default_qname_exact"));
                }
            }
            return None;
        }
        if let Some(sym) = self.lookup.by_qualified_name(target) {
            if kind(edge_kind, &sym.kind) {
                return Some(self.resolution(sym.id, "default_qname_exact"));
            }
        }
        // Case-folding fallback: a folding language whose call site differs only
        // in surface case from the keyed qname misses the byte-exact probe
        // above. Scan the leaf's `by_name` candidates and accept one whose whole
        // qname folds equal to the target. `NameNormalization::None` borrows
        // identically on both sides, so this never fires for a case-sensitive
        // language (the exact probe is the whole strategy).
        if !matches!(norm, NameNormalization::None) {
            let leaf = target.rsplit(['.', ':', '/']).next().unwrap_or(target);
            let target_norm = normalize_name(norm, target);
            for sym in self.lookup.by_name(leaf) {
                if normalize_name(norm, &sym.qualified_name) == target_norm
                    && kind(edge_kind, &sym.kind)
                {
                    return Some(self.resolution(sym.id, "default_qname_exact"));
                }
            }
        }
        None
    }

    /// Strategy — explicit-member submodule import binds a unique internal
    /// symbol of the imported name.
    ///
    /// An explicit-member import names the symbol AND its enclosing module in
    /// one statement (`import struct MyModule.Bar`): the local binding is `Bar`
    /// and the module path is the dotted `MyModule.Bar`. The discriminator is a
    /// DOTTED `module_path` whose last segment equals both `imported_name` and
    /// the bare `target` — that exact shape is the import-scope evidence, so the
    /// bind is import-directed, not whole-program. Binds the UNIQUE internal
    /// `by_name(target)` symbol of compatible kind; zero or multiple candidates
    /// decline (never guess).
    ///
    /// Gated on `explicit_member_import` (default off). A non-dotted
    /// `module_path` (a plain whole-module `import Foundation`) leaves the
    /// strategy inert — that form carries no project-symbol scope and stays for
    /// external classification.
    pub fn resolve_via_explicit_member_import(
        &self,
        enabled: bool,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        if !enabled {
            return None;
        }
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.contains('.') || target.contains("::") || target.contains('/') {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;

        // The import scope is genuine only when the import statement explicitly
        // names this symbol: a dotted module path whose last segment is the
        // imported name (== the bare target). A plain `import Foundation`
        // (no dot) never satisfies this.
        let armed = self.file_ctx.imports.iter().any(|import| {
            if import.imported_name != target {
                return false;
            }
            let Some(module) = import.module_path.as_deref() else {
                return false;
            };
            module.contains('.') && module.rsplit('.').next() == Some(target)
        });
        if !armed {
            return None;
        }

        let mut compatible: Vec<&SymbolInfo> = self
            .lookup
            .by_name(target)
            .iter()
            .filter(|sym| !self.lookup.is_external_file(&sym.file_path))
            .filter(|sym| kind(edge_kind, &sym.kind))
            .collect();
        compatible.sort_by(|a, b| {
            a.qualified_name
                .cmp(&b.qualified_name)
                .then(a.kind.cmp(&b.kind))
        });
        compatible.dedup_by(|a, b| a.qualified_name == b.qualified_name && a.kind == b.kind);
        if compatible.len() != 1 {
            return None;
        }
        Some(self.resolution(compatible[0].id, "default_explicit_member_import"))
    }

    /// Strategy 3 — bare target matches a name in the file's import list.
    ///
    /// `import { Foo } from './foo'` then `Foo(...)`: the import brings
    /// `Foo` into local scope. Find a `target`-named symbol in any file
    /// whose path matches the import's module specifier.
    pub fn resolve_via_file_import(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;

        for import in &self.file_ctx.imports {
            let matches_direct = import.imported_name == target;
            let matches_alias = import.alias.as_deref() == Some(target);
            if !matches_direct && !matches_alias {
                continue;
            }
            let lookup_name = if matches_alias {
                &import.imported_name
            } else {
                target
            };
            let module_path = import.module_path.as_deref().unwrap_or("");

            for sym in self.lookup.by_name(lookup_name) {
                if !kind(edge_kind, &sym.kind) {
                    continue;
                }
                if file_path_matches_module(&sym.file_path, module_path) {
                    return Some(self.resolution(sym.id, "default_file_import"));
                }
            }
        }
        None
    }

    /// Strategy 4 — dotted namespace import + bare target → expand to qname.
    ///
    /// C# `using eShop.Catalog.API.Model;` then `CatalogItem`: the import
    /// brings the WHOLE namespace into scope, not the type. Form
    /// `{ns}.{target}` for each dotted import and look it up.
    pub fn resolve_via_namespace_import(
        &self,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
        norm: NameNormalization,
    ) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;

        for import in &self.file_ctx.imports {
            for prefix in candidate_namespace_prefixes(import) {
                if !prefix.contains('.') {
                    continue;
                }
                let qname = format!("{prefix}.{target}");
                if let Some(sym) = self.lookup.by_qualified_name(&qname) {
                    if kind(edge_kind, &sym.kind) {
                        return Some(self.resolution(sym.id, "default_namespace_import"));
                    }
                }
                // Case-folding fallback: the `{prefix}.{target}` form may differ
                // in case from the keyed qname (folding language). Scan the
                // prefix's members and accept one whose qname folds equal. Gated
                // on a non-identity spec — case-sensitive languages skip it.
                if !matches!(norm, NameNormalization::None) {
                    let expected_norm = normalize_name(norm, &qname);
                    for sym in self.lookup.in_namespace(prefix) {
                        if normalize_name(norm, &sym.qualified_name) == expected_norm
                            && kind(edge_kind, &sym.kind)
                        {
                            return Some(self.resolution(sym.id, "default_namespace_import"));
                        }
                    }
                }
            }
        }
        None
    }

    /// Strategy — bare member keyed under an import's package short name.
    ///
    /// An import that names a package (`import "github.com/gin-gonic/gin"`)
    /// brings the short name `gin` into scope; the package's members are keyed
    /// `gin{sep}{Name}`. When the extractor dropped the qualifier off a member
    /// ref, the bare `target` is `NewRouter` and resolves under
    /// `{import.imported_name}{sep}{target}`. For an aliased import
    /// (`import mygin "github.com/gin-gonic/gin"`) the symbol stays keyed under
    /// the path's last segment, so try `{last_path_segment}{sep}{target}` too.
    ///
    /// `sep` is the profile's qname separator (`.` for Go, `::` for Hare). The
    /// last-path-segment probe splits the module path on `/` first (slash-pathed
    /// imports like Go's) and then on `sep` (a `::`-pathed import like Hare's
    /// `crypto::sha256`), so the trailing component is the package's short name
    /// under either path syntax.
    ///
    /// Probes both forms by exact qname, dotless prefixes included (the
    /// dotted-prefix guard in `resolve_via_namespace_import` skips exactly these
    /// short names). Gated by `ChainQualification::PackageShortName` in the
    /// ladder so languages whose imports name the type itself stay untouched.
    pub fn resolve_via_package_short_name(
        &self,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
        sep: &str,
    ) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() || target.contains('.') || target.contains("::") || target.contains('/') {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        for import in &self.file_ctx.imports {
            let Some(module) = import.module_path.as_deref() else {
                continue;
            };
            let last_seg = module
                .rsplit('/')
                .next()
                .unwrap_or(module)
                .rsplit(sep)
                .next()
                .unwrap_or(module);
            for prefix in [import.imported_name.as_str(), last_seg] {
                if prefix.is_empty() {
                    continue;
                }
                let qname = format!("{prefix}{sep}{target}");
                if let Some(sym) = self.lookup.by_qualified_name(&qname) {
                    if kind(edge_kind, &sym.kind) {
                        return Some(self.resolution(sym.id, "default_package_short_name"));
                    }
                }
            }
        }
        None
    }

    /// Strategy 5 — dotted target whose leaf resolves under an ambient-namespace qname.
    ///
    /// TS `declare global { namespace Express { interface Multer { ... } } }`
    /// in `@types/multer` indexes members under qnames like
    /// `@types/multer.Express.Multer.File`. A ref written as
    /// `Express.Multer.File` doesn't match `qname_exact` because of the
    /// package prefix. Look up the leaf via by_name and accept any
    /// candidate whose qname ENDS WITH `.{target}` — the package prefix
    /// is absorbed.
    pub fn resolve_via_ambient_namespace_path(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        if !target.contains('.') {
            return None;
        }
        let leaf = target.rsplit('.').next().unwrap_or(target);
        let suffix = format!(".{target}");
        let candidates = self.lookup.by_name(leaf);
        let best = candidates
            .iter()
            .filter(|sym| sym.qualified_name.ends_with(&suffix))
            .filter(|sym| kind(edge_kind, &sym.kind))
            .min_by_key(|sym| sym.file_path.matches('/').count())?;
        Some(self.resolution(best.id, "default_ambient_namespace_path"))
    }

    /// Strategy 6 — same-file-namespace lookup.
    ///
    /// In C#, types in the same namespace are visible without a `using`.
    /// If the source file declares namespace X, candidates whose qname is
    /// `X.target` are in scope.
    pub fn resolve_via_same_namespace(
        &self,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
        norm: NameNormalization,
    ) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let ns = self.file_ctx.file_namespace.as_deref()?;
        if ns.is_empty() {
            return None;
        }
        let expected = format!("{ns}.{target}");
        for sym in self.lookup.by_name(target) {
            if sym.qualified_name == expected && kind(edge_kind, &sym.kind) {
                return Some(self.resolution(sym.id, "default_same_namespace"));
            }
        }
        // Case-folding fallback: a folding language whose call site differs in
        // case from the declared member name misses the byte-exact probe above
        // (`by_name` is case-sensitive). Scan the namespace's members and accept
        // one whose qname folds equal to `{ns}.{target}`. Gated on a non-identity
        // spec, so this never runs for a case-sensitive language.
        if !matches!(norm, NameNormalization::None) {
            let expected_norm = normalize_name(norm, &expected);
            for sym in self.lookup.in_namespace(ns) {
                if normalize_name(norm, &sym.qualified_name) == expected_norm
                    && kind(edge_kind, &sym.kind)
                {
                    return Some(self.resolution(sym.id, "default_same_namespace"));
                }
            }
        }
        None
    }

    /// Strategy 7 — qname is prefixed by an imported namespace.
    ///
    /// `using FamilyBudget.Api.Entities;` then `Transaction`: the
    /// candidate's qname `FamilyBudget.Api.Entities.Transaction` starts
    /// with the imported namespace. Boundary check ensures
    /// `FamilyBudget.Api.Entities` doesn't accidentally match
    /// `FamilyBudget.Api.EntitiesOther`.
    pub fn resolve_via_imported_namespace(
        &self,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
        norm: NameNormalization,
    ) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        for sym in self.lookup.by_name(target) {
            if !kind(edge_kind, &sym.kind) {
                continue;
            }
            for import in &self.file_ctx.imports {
                let Some(module) = &import.module_path else {
                    continue;
                };
                if sym.qualified_name.starts_with(module.as_str()) {
                    let rest = &sym.qualified_name[module.len()..];
                    if rest.is_empty() || rest.starts_with('.') {
                        return Some(self.resolution(sym.id, "default_imported_namespace"));
                    }
                }
                if file_path_matches_module(&sym.file_path, module) {
                    return Some(self.resolution(sym.id, "default_imported_namespace"));
                }
            }
        }
        // Case-folding fallback: under a folding spec the candidate's declared
        // leaf may differ in case from `target`, so `by_name(target)` above
        // misses. For each imported module, scan its members and accept one
        // whose qname folds equal to `{module}.{target}`. Gated on a non-identity
        // spec — a case-sensitive language never reaches this scan.
        if !matches!(norm, NameNormalization::None) {
            for import in &self.file_ctx.imports {
                let Some(module) = &import.module_path else {
                    continue;
                };
                let expected = format!("{module}.{target}");
                let expected_norm = normalize_name(norm, &expected);
                for sym in self.lookup.in_namespace(module) {
                    if normalize_name(norm, &sym.qualified_name) == expected_norm
                        && kind(edge_kind, &sym.kind)
                    {
                        return Some(self.resolution(sym.id, "default_imported_namespace"));
                    }
                }
            }
        }
        None
    }

    /// Strategy 8 — chain prefix (second-to-last segment) matches an import.
    ///
    /// `resolve::resolve_and_write()` with `use crate::indexer::resolve`:
    /// the chain prefix `resolve` matches the import; the import's
    /// module path tells us which directory the target lives in.
    pub fn resolve_via_chain_prefix(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let chain = self.ref_ctx.extracted_ref.chain.as_ref()?;
        if chain.segments.len() < 2 {
            return None;
        }
        let prefix = chain.segments[chain.segments.len() - 2].name.as_str();

        let matching_import = self
            .file_ctx
            .imports
            .iter()
            .find(|imp| imp.imported_name == prefix || imp.alias.as_deref() == Some(prefix))?;
        let module_path = matching_import.module_path.as_deref().unwrap_or("");

        let candidates = self.lookup.by_name(target);

        for sym in candidates {
            if !kind(edge_kind, &sym.kind) {
                continue;
            }
            if file_path_matches_module(&sym.file_path, module_path) {
                return Some(self.resolution(sym.id, "default_chain_prefix"));
            }
        }

        for sym in candidates {
            if !kind(edge_kind, &sym.kind) {
                continue;
            }
            if sym.file_path.split('/').any(|seg| seg == prefix) {
                return Some(self.resolution(sym.id, "default_chain_prefix"));
            }
        }

        None
    }

    /// Strategy 9 — cross-package re-export chain.
    ///
    /// User imports `Foo` from `pkg-a`, but `Foo`'s definition lives in
    /// `pkg-b` and `pkg-a` re-exports it. The plain `file_import` strategy
    /// fails because the candidate's file is inside `pkg-b` (not the
    /// imported `pkg-a`). The lookup walks the package re-export graph
    /// to find the actual owner.
    ///
    /// Two shapes covered:
    ///   (a) Direct: `import { Foo } from 'pkg-a'` and target is `Foo`.
    ///   (b) Dotted: `import { Ns } from 'pkg-a'` and target is `Ns.Inner`
    ///       — first segment is the import alias, last segment is the
    ///       symbol to resolve.
    pub fn resolve_via_reexport_chain(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;

        // Shape (a): bare target matches a non-relative import.
        if let Some(matching) = self
            .file_ctx
            .imports
            .iter()
            .find(|imp| imp.imported_name == target)
        {
            if let Some(module) = matching.module_path.as_deref() {
                if !module.is_empty() && !is_relative_specifier(module) {
                    if let Some(id) =
                        self.lookup.resolve_external_reexport(target, target, module)
                    {
                        return self
                            .candidate_with_compatible_kind(target, id, edge_kind, kind)
                            .map(|sid| self.resolution(sid, "default_reexport_chain"));
                    }
                }
            }
        }

        // Shape (b): dotted target. First segment may be an import alias;
        // last segment is the actual symbol.
        if let Some(dot) = target.find('.') {
            let prefix = &target[..dot];
            let suffix = target.rsplit('.').next().unwrap_or(target);
            if !prefix.is_empty() && !suffix.is_empty() && suffix != target {
                if let Some(matching) = self
                    .file_ctx
                    .imports
                    .iter()
                    .find(|imp| imp.imported_name == prefix)
                {
                    if let Some(module) = matching.module_path.as_deref() {
                        if !module.is_empty() && !is_relative_specifier(module) {
                            if let Some(id) = self
                                .lookup
                                .resolve_external_reexport(suffix, prefix, module)
                            {
                                return self
                                    .candidate_with_compatible_kind(suffix, id, edge_kind, kind)
                                    .map(|sid| {
                                        self.resolution(sid, "default_reexport_chain")
                                    });
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Strategy — follow re-export chains from an imported module.
    ///
    /// When the file imports `target` from a module that does not itself
    /// define it but RE-EXPORTS it (`export { X } from './y'`, Rust
    /// `pub use crate::bar::X`), walk the re-export chain to the module that
    /// actually defines `target`.
    ///
    /// The walk consults `reexports_from`, which carries ONLY refs the
    /// extractor tagged `is_reexport=true`. A private import
    /// (`use crate::bar::X;` without `pub`, `import { X } from 'pkg'`) is
    /// `is_reexport=false` and never enters that map, so a private import
    /// cannot forward a name here (Invariant #2). Imports whose module
    /// resolves to an external (`ext:`) file — or doesn't resolve to a
    /// project file at all — are skipped: cross-package re-exports are the
    /// externals stage's job, handled by `resolve_via_reexport_chain`.
    pub fn resolve_via_reexport_following(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let from_file = self.file_ctx.file_path.as_str();

        for import in &self.file_ctx.imports {
            if import.is_wildcard || import.imported_name != target {
                continue;
            }
            let Some(module) = import.module_path.as_deref() else {
                continue;
            };
            if module.is_empty() {
                continue;
            }
            // Resolve the imported module to a project file. An `ext:` hit is
            // external — no project-internal re-export hop possible. A relative
            // specifier is project-internal by construction; when per-source
            // resolution has no entry (`reexports_from` is keyed by file path and
            // `resolve_module_from` populated lazily), fall back to the raw
            // specifier — `reexports_from` resolves it via its own exact-path /
            // `module_to_file` fallback.
            let resolved = match self.lookup.resolve_module_from(from_file, module) {
                Some(p) if p.starts_with("ext:") => continue,
                Some(p) => p.to_string(),
                None if is_relative_specifier(module) => module.to_string(),
                None => continue,
            };
            if let Some(res) = follow_reexports(
                &resolved,
                target,
                edge_kind,
                self.kind_compatible,
                self.lookup,
                0,
            ) {
                return Some(res);
            }
        }
        None
    }

    /// Strategy 10 — ambient-package preference.
    ///
    /// Bare target whose only candidate that's kind-compatible lives in a
    /// project-declared ambient package (TS `tsconfig.json#types`, `@types/*`,
    /// `globals.d.ts`). The project opted into those packages as ambient
    /// providers, so an unimported reference to one of their members is
    /// the intended target — not an ambiguous bare-name guess.
    pub fn resolve_via_ambient_package(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        self.resolve_via_ambient_package_named(self.ref_ctx.extracted_ref.target_name.as_str(), kind)
    }

    /// `resolve_via_ambient_package` against an explicit `name` rather than the
    /// ref's raw target. Lets the ladder strip a profile-declared namespace-
    /// alias prefix (`sys.concat` → `concat`) before the ambient-package probe
    /// without rewriting the ref or affecting the bare-name strategies above.
    fn resolve_via_ambient_package_named(
        &self,
        name: &str,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let candidates = self.lookup.by_name(name);
        let ambient: Vec<&SymbolInfo> = candidates
            .iter()
            .filter(|sym| self.lookup.is_ambient_path(&sym.file_path))
            .filter(|sym| kind(edge_kind, &sym.kind))
            .collect();
        let best = ambient
            .iter()
            .min_by_key(|sym| sym.file_path.matches('/').count())?;
        Some(self.resolution(best.id, "default_ambient_package"))
    }

    /// Walk the by-name candidates and return the id of the first one whose
    /// kind matches the edge kind. Used after a lookup returned a single id
    /// to confirm the candidate is kind-compatible (the lookup itself can't
    /// know the edge kind).
    fn candidate_with_compatible_kind(
        &self,
        name: &str,
        id: i64,
        edge_kind: EdgeKind,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<i64> {
        let candidate = self
            .lookup
            .by_name(name)
            .iter()
            .find(|sym| sym.id == id)?;
        if kind(edge_kind, &candidate.kind) {
            Some(id)
        } else {
            None
        }
    }

    /// Strategy 11 — bare target uniquely matches one project-internal symbol.
    ///
    /// When `lookup.by_name(target)` returns EXACTLY ONE kind-compatible,
    /// project-internal candidate, that's the answer — there is no other
    /// possible target. Two or more candidates → return None (genuine
    /// ambiguity, do not guess). Zero candidates → return None.
    ///
    /// "Project-internal" excludes `ext:`-prefixed external files so that
    /// e.g. a Fortran call doesn't get bound to a same-named stdlib symbol
    /// when the project also defines it. The strict gates above
    /// (`scope_visible`, `same_file`, `file_import`, `ref_module`) catch
    /// the explicit-evidence cases first; this is the residual that
    /// catches "the project has one `foo` and the caller didn't qualify".
    pub fn resolve_via_unique_internal_name(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        if target.contains('.') || target.contains("::") || target.contains('/') {
            return None;
        }
        let candidates = self.lookup.by_name(target);
        let mut compatible: Vec<&SymbolInfo> = candidates
            .iter()
            .filter(|sym| !self.lookup.is_external_file(&sym.file_path))
            .filter(|sym| kind(edge_kind, &sym.kind))
            .collect();
        // Dedup on (qualified_name, kind): the same logical symbol indexed
        // twice (header + impl, pnpm-monorepo duplicates) is one candidate
        // for ambiguity purposes.
        compatible.sort_by(|a, b| {
            a.qualified_name
                .cmp(&b.qualified_name)
                .then(a.kind.cmp(&b.kind))
        });
        compatible.dedup_by(|a, b| {
            a.qualified_name == b.qualified_name && a.kind == b.kind
        });
        if compatible.len() != 1 {
            return None;
        }
        Some(self.resolution(compatible[0].id, "default_unique_internal_name"))
    }

    /// Strategy 12 — bare target matches a sibling symbol in the same file.
    ///
    /// For languages without explicit scope rules (Lua, Bash, OCaml at root,
    /// most scripting languages), a bare call to a function defined later
    /// or earlier in the same file is the canonical resolution. Walk every
    /// symbol the lookup knows about in this file and accept the first
    /// kind-compatible match.
    pub fn resolve_via_same_file(
        &self,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
        self_keywords: &[&str],
        norm: NameNormalization,
    ) -> Option<Resolution> {
        let target = strip_self_keyword(
            self.ref_ctx.extracted_ref.target_name.as_str(),
            self_keywords,
        );
        // Yield to an explicit (non-wildcard) import that binds this name. True
        // lexical locals are already handled by resolve_via_scope_visible (which
        // runs first); a file-level sibling that isn't in the scope chain must
        // not shadow an import that names the same thing.
        if !target.is_empty()
            && self.file_ctx.imports.iter().any(|imp| {
                !imp.is_wildcard
                    && (imp.imported_name == target || imp.alias.as_deref() == Some(target))
            })
        {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let target_norm = normalize_name(norm, target);
        for sym in self.lookup.in_file(&self.file_ctx.file_path) {
            if normalize_name(norm, &sym.name) == target_norm && kind(edge_kind, &sym.kind) {
                return Some(self.resolution(sym.id, "default_same_file"));
            }
        }
        None
    }

    /// Strategy 12 — scope-chain walk.
    ///
    /// Try `{scope}{sep}{target}` for each scope in `ref_ctx.scope_chain`
    /// (innermost first) and each `sep` in `separators`. Catches local symbols
    /// defined in the source symbol's own enclosing type / namespace / module
    /// before we widen to file or project scope.
    ///
    /// `separators` always contains `"."` (the universal index join) and, for a
    /// language whose `qname_separator` differs, that separator too — so a
    /// `::`-keyed index resolves the same scope-visible members a `.`-keyed one
    /// does. The list is deduped by the caller, so a `.`-separator language tries
    /// `"."` exactly once.
    pub fn resolve_via_scope_visible(
        &self,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
        separators: &[&str],
        self_keywords: &[&str],
        norm: NameNormalization,
    ) -> Option<Resolution> {
        let target = strip_self_keyword(
            self.ref_ctx.extracted_ref.target_name.as_str(),
            self_keywords,
        );
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let target_norm = normalize_name(norm, target);
        for scope in &self.ref_ctx.scope_chain {
            // Exact qname probe — byte-identical under every separator. With
            // `NameNormalization::None` this is the whole strategy, so a
            // case-sensitive language is unaffected.
            for sep in separators {
                let qname = format!("{scope}{sep}{target}");
                if let Some(sym) = self.lookup.by_qualified_name(&qname) {
                    if kind(edge_kind, &sym.kind) {
                        return Some(self.resolution(sym.id, "default_scope_visible"));
                    }
                }
            }
            // Normalized-name fallback: only when the language folds names, and
            // only after the exact probe missed. Compares the scope's members
            // by normalized name so a reference written in a different surface
            // form (case-folded, sigil-stripped) binds to the scope member.
            if !matches!(norm, NameNormalization::None) {
                for member in self.lookup.members_of(scope) {
                    if normalize_name(norm, &member.name) == target_norm
                        && kind(edge_kind, &member.kind)
                    {
                        return Some(self.resolution(member.id, "default_scope_visible"));
                    }
                }
            }
        }
        None
    }

    /// Strategy — generic-parameter-in-scope.
    ///
    /// A bare identifier whose name matches a generic parameter declared on
    /// the source symbol or any enclosing scope (impl/class/function) is the
    /// generic parameter itself, not an external type. Resolve it to the
    /// declaring symbol so it counts as an edge into that scope rather than
    /// polluting `unresolved_refs`. Catches `I`, `F`, `TDocSet`,
    /// `TSortKey`, `TScoreCombiner`, etc. across every language with
    /// declared-generic syntax.
    pub fn resolve_via_generic_param(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() || target.contains('.') || target.contains("::") {
            return None;
        }
        // Source symbol's own generics. The extractor stores them as
        // interned GenericParamId; resolve the names through the workspace
        // arena (when present). Synthetic test lookups without an arena
        // just skip this branch and the enclosing-scope path below picks
        // up the same information through the string-typed `generic_params`
        // trait method.
        if let Some(arena) = self.lookup.type_arena() {
            if self
                .ref_ctx
                .source_symbol
                .generic_params
                .iter()
                .any(|id| arena.generic_param(*id).name == target)
            {
                if let Some(sym) =
                    self.lookup.by_qualified_name(&self.ref_ctx.source_symbol.qualified_name)
                {
                    return Some(self.resolution(sym.id, "engine_generic_param"));
                }
            }
        }
        // Source symbol's OWN declared params via the string map (populated
        // from signatures at index build). The arena path above only fires for
        // extractors that intern GenericParamIds; this covers the rest — e.g.
        // a template function whose params (`OutputIt fill_n(OutputIt ...)`)
        // appear only in its own signature, never in an enclosing scope. The
        // scope-chain loop below deliberately skips the source symbol's qname,
        // so without this those self-declared params stay unresolved.
        if let Some(params) =
            self.lookup.generic_params(&self.ref_ctx.source_symbol.qualified_name)
        {
            if params.iter().any(|p| p == target) {
                if let Some(sym) =
                    self.lookup.by_qualified_name(&self.ref_ctx.source_symbol.qualified_name)
                {
                    return Some(self.resolution(sym.id, "engine_generic_param"));
                }
            }
        }
        // Enclosing scopes (impl block around a method, class around a
        // method on a generic class, etc.). scope_chain holds qnames
        // outermost-last by convention, but the iteration order doesn't
        // matter — the first match wins.
        for scope_qname in self.ref_ctx.scope_chain.iter() {
            if scope_qname == &self.ref_ctx.source_symbol.qualified_name {
                continue;
            }
            let Some(params) = self.lookup.generic_params(scope_qname) else { continue };
            if params.iter().any(|p| p == target) {
                if let Some(sym) = self.lookup.by_qualified_name(scope_qname) {
                    return Some(self.resolution(sym.id, "engine_generic_param"));
                }
            }
        }
        None
    }

    /// The nearest enclosing type symbol (class/struct/interface/…) for the
    /// current ref. Walks `scope_chain` innermost-first, then falls back to
    /// the source symbol's `scope_path`. `None` when the ref isn't inside a
    /// type (free function, file scope).
    fn enclosing_type(&self) -> Option<&'a SymbolInfo> {
        let lk = self.lookup;
        // Structured first: the source symbol's containment chain names its
        // enclosing type by kind, derived from the `parent_index` chain rather
        // than assembled from `scope_path` strings.
        if let Some(type_qname) = lk
            .containing_scope(&self.ref_ctx.source_symbol.qualified_name)
            .and_then(|s| s.containing_type_qname())
        {
            if let Some(sym) = lk.by_qualified_name(type_qname) {
                if is_type_kind(&sym.kind) {
                    return Some(sym);
                }
            }
        }
        // Fallback for lookups with no containment chain (synthetic test
        // doubles): scope_chain innermost-first, then the source's scope_path.
        for scope in &self.ref_ctx.scope_chain {
            if let Some(sym) = lk.by_qualified_name(scope) {
                if is_type_kind(&sym.kind) {
                    return Some(sym);
                }
            }
        }
        let sp = self.ref_ctx.source_symbol.scope_path.as_deref()?;
        let sym = lk.by_qualified_name(sp)?;
        is_type_kind(&sym.kind).then_some(sym)
    }

    /// Strategy — `this`/`self`/`Self`/`super` keyword refs.
    ///
    /// `this`/`self`/`Self` resolve to the enclosing type; `super`/`base`
    /// resolve to its direct parent via `parent_class_qname`. Catches
    /// `super(...)` constructor delegation and bare-keyword refs that carry
    /// no member chain for the chain walker to follow.
    pub fn resolve_via_self_keyword(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let enclosing = self.enclosing_type()?;
        match target {
            "this" | "self" | "Self" => kind(edge_kind, &enclosing.kind)
                .then(|| self.resolution(enclosing.id, "engine_self_keyword")),
            "super" | "base" => {
                let parent_qname = self.lookup.parent_class_qname(&enclosing.qualified_name)?;
                let parent = self.lookup.by_qualified_name(parent_qname)?;
                kind(edge_kind, &parent.kind)
                    .then(|| self.resolution(parent.id, "engine_self_keyword"))
            }
            _ => None,
        }
    }

    /// Strategy — member declared on an ANCESTOR of the enclosing type.
    ///
    /// `resolve_via_scope_visible` already resolves members of the immediate
    /// enclosing scope via `{scope}.{target}`. This climbs the inheritance
    /// chain with `parent_class_qname` and accepts a member whose simple name
    /// matches — reaching inherited fields/methods declared on a base class.
    /// Climb is bounded at `MAX_INHERITANCE_DEPTH`.
    pub fn resolve_via_enclosing_member(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() || target.contains('.') || target.contains("::") {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let mut current = self.enclosing_type()?.qualified_name.clone();
        for _ in 0..MAX_INHERITANCE_DEPTH {
            for member in self.lookup.members_of(&current) {
                if member.name == target && kind(edge_kind, &member.kind) {
                    return Some(self.resolution(member.id, "engine_enclosing_member"));
                }
            }
            match self.lookup.parent_class_qname(&current) {
                Some(parent) => current = parent.to_string(),
                None => break,
            }
        }
        None
    }

    /// Strategy — import specifier resolved through a path alias.
    ///
    /// `resolve_via_file_import` matches an import's `module_path` directly
    /// against candidate file paths. This handles specifiers that are path
    /// aliases (`@/utils`, `$lib/...`) which must first be rewritten to a
    /// real path. Fires only when the rewrite changes the specifier — the
    /// raw-path case already ran in `resolve_via_file_import`.
    pub fn resolve_via_aliased_import(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        for import in &self.file_ctx.imports {
            let matches_direct = import.imported_name == target;
            let matches_alias = import.alias.as_deref() == Some(target);
            if !matches_direct && !matches_alias {
                continue;
            }
            let Some(raw_module) = import.module_path.as_deref() else { continue };
            let Some(rewritten) = self
                .lookup
                .resolve_path_alias(self.ref_ctx.file_package_id, raw_module)
            else {
                continue;
            };
            if rewritten == raw_module {
                continue;
            }
            let lookup_name = if matches_alias {
                import.imported_name.as_str()
            } else {
                target
            };
            for sym in self.lookup.by_name(lookup_name) {
                if kind(edge_kind, &sym.kind)
                    && file_path_matches_module(&sym.file_path, &rewritten)
                {
                    return Some(self.resolution(sym.id, "engine_aliased_import"));
                }
            }
        }
        None
    }

    /// Strategy — template-include import resolution.
    ///
    /// Fires only for `Imports` refs whose `target_name` is a relative-path /
    /// stem reference to another template FILE (not a symbol): handlebars
    /// partials, EJS / Pug / Nunjucks includes, GSP `<g:render template>`,
    /// markdown relative links, GitHub-Actions YAML `uses`. From the raw
    /// target and the `ImportResolution` data, generate candidate project file
    /// paths (extensions, parent-dir walk, directory-index entries, underscore
    /// and kebab variants) and bind the first candidate whose in-file symbol
    /// matches `bind_kind` under the configured `stem_match` rule.
    ///
    /// All per-language behavior is in `ir` — the algorithm is one shape. A
    /// non-`Imports` ref, an empty target, or a declined leading slash short-
    /// circuits to `None` so the regular ladder is unaffected.
    pub fn resolve_via_import_path(
        &self,
        ir: &ImportResolution,
    ) -> Option<Resolution> {
        if self.ref_ctx.extracted_ref.kind != EdgeKind::Imports {
            return None;
        }
        let target = self.ref_ctx.extracted_ref.target_name.trim();
        if target.is_empty() {
            return None;
        }
        if ir.decline_leading_slash && target.starts_with('/') {
            return None;
        }
        let source_dir = std::path::Path::new(self.file_ctx.file_path.as_str()).parent()?;

        for candidate in import_path_candidates(source_dir, target, ir) {
            let path_str = candidate.to_string_lossy().replace('\\', "/");
            let file_stem = candidate
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let file_name = candidate
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            for sym in self.lookup.in_file(&path_str) {
                if sym.kind != ir.bind_kind {
                    continue;
                }
                let name_ok = match ir.stem_match {
                    StemMatch::StemExact => sym.name == file_stem,
                    StemMatch::StemOrUnderscoreStripped => {
                        sym.name == file_stem
                            || sym.name == file_stem.trim_start_matches('_')
                    }
                    StemMatch::BasenameWithExt => sym.name == file_name,
                    StemMatch::AnyClassInFile => true,
                };
                if name_ok {
                    return Some(self.resolution(sym.id, ir.strategy_tag));
                }
            }
        }
        None
    }

    /// Strategy — module-anchored bind off the extractor-set `r.module`.
    ///
    /// When the ref carries a `module` (a `package:`/relative URI, a `require`
    /// path, a `from X import` source, or a post-pass-attached module path) the
    /// `module` is the most specific evidence for where the target lives. Bound
    /// by the profile's `ModuleAnchorBind`:
    ///   - `NameExactKind` — `in_module_from(file, module)` then the symbol whose
    ///     simple name equals `target` and whose kind matches.
    ///   - `PreferNamedElseFirst` — `in_module_from` then the same-named symbol
    ///     (case-insensitive) when present, else the first symbol in the module.
    ///   - `ByNameUnderModuleDir` — `by_name(target)` whose `file_path` contains
    ///     `module.replace('.', "/")`, plus the `{module}.{target}` qname probe.
    ///   - `ByFileStem` — `by_name(target)`, kind-compatible, whose file
    ///     basename-stem OR a path dir-segment equals the module's leaf (its
    ///     last `.`-segment, lowercased) per `path_stem_matches`.
    ///   - `MemberOfModuleType` — `members_of(module)` where `module` names a
    ///     TYPE: the member whose name equals `target` under `norm` and whose
    ///     kind is compatible.
    ///
    /// `relative_marker` splits which modules take the `in_module_from` bind: with
    /// a marker set, only a prefixed (relative) module runs the configured bind and
    /// every other (absolute) module routes to `ByNameUnderModuleDir`. With
    /// `RelativeMarker::None` every module runs the configured bind. `norm` folds
    /// the name comparison for `MemberOfModuleType` (the same `NameNormalization`
    /// the scope / same-file probes use). Returns `None` when no module is set,
    /// the anchor is `Off`, or no candidate matches.
    pub fn resolve_via_module_anchor(
        &self,
        anchor: ModuleAnchor,
        relative_marker: RelativeMarker,
        norm: NameNormalization,
        sep: &str,
        rewrites: ModulePrefixRewrites,
        overload_pick_all: bool,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        let ModuleAnchor::On(bind) = anchor else {
            return None;
        };
        let module = self.ref_ctx.extracted_ref.module.as_deref()?;
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;

        let is_relative = match relative_marker {
            RelativeMarker::None => true,
            RelativeMarker::DotPrefix => module.starts_with('.'),
            RelativeMarker::DotSlashPrefix => {
                module.starts_with("./") || module.starts_with("../")
            }
        };
        // A relative module takes the in_module_from bind; an absolute module
        // (only possible when a marker is set) takes the directory-containment
        // bind regardless of the profile's configured rule.
        let effective_bind = if is_relative {
            bind
        } else {
            ModuleAnchorBind::ByNameUnderModuleDir
        };

        match effective_bind {
            ModuleAnchorBind::NameExactKind => {
                for sym in self.lookup.in_module_from(&self.file_ctx.file_path, module) {
                    if sym.name == target && kind(edge_kind, &sym.kind) {
                        return Some(self.resolution(sym.id, "default_module_anchor"));
                    }
                }
            }
            ModuleAnchorBind::PreferNamedElseFirst => {
                let syms = self.lookup.in_module_from(&self.file_ctx.file_path, module);
                let pick = syms
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(target))
                    .or_else(|| syms.first());
                if let Some(sym) = pick {
                    return Some(self.resolution(sym.id, "default_module_anchor"));
                }
            }
            ModuleAnchorBind::ByNameUnderModuleDir => {
                // qname probe: `{prefix}{sep}{target}` for each candidate module
                // prefix, under the universal `.` join and, when the profile
                // separator differs, that separator too — a `::`-keyed module
                // (`crate::db`) probes both `crate::db.new` (index join) and
                // `crate::db::new`. `module_prefix_candidates` yields the literal
                // module first, then any profile-configured DefinitelyTyped /
                // deep-import-peel rewrites for a bare specifier.
                for prefix in module_prefix_candidates(module, rewrites) {
                    for s in module_anchor_separators(sep) {
                        let qname = format!("{prefix}{s}{target}");
                        if overload_pick_all {
                            for sym in self.lookup.all_by_qualified_name(&qname) {
                                if kind(edge_kind, &sym.kind) {
                                    return Some(
                                        self.resolution(sym.id, "default_module_anchor"),
                                    );
                                }
                            }
                        } else if let Some(sym) = self.lookup.by_qualified_name(&qname) {
                            if kind(edge_kind, &sym.kind) {
                                return Some(self.resolution(sym.id, "default_module_anchor"));
                            }
                        }
                    }
                }
                // Directory-containment probe: turn the module into a path
                // fragment by mapping every separator (`.` and the profile's,
                // e.g. `::`) to `/`. A multi-segment crate-rooted module
                // (`crate::db`) rarely matches the full fragment (`crate/db`),
                // so fall back to the module's trailing segment — the file-like
                // leaf (`db` → `db.rs`). Declined for a bare specifier when the
                // rewrites axis demands it: a bare package name (`react`) must
                // resolve through the qname rewrites above or stay unresolved,
                // never directory-match a same-named project file.
                if declines_bare_directory_match(module, rewrites) {
                    return None;
                }
                let module_as_path = separators_to_slash(module, sep);
                let leaf = module_leaf(module, sep).to_lowercase();
                for sym in self.lookup.by_name(target) {
                    if !kind(edge_kind, &sym.kind) {
                        continue;
                    }
                    let path = sym.file_path.replace('\\', "/");
                    if path.contains(&module_as_path) {
                        return Some(self.resolution(sym.id, "default_module_anchor"));
                    }
                    if leaf != module_as_path && path_stem_matches(&path.to_lowercase(), &leaf) {
                        return Some(self.resolution(sym.id, "default_module_anchor"));
                    }
                }
            }
            ModuleAnchorBind::ByFileStem { against } => {
                let leaf = match against {
                    StemSource::ModuleLeaf => {
                        module.rsplit('.').next().unwrap_or(module).to_lowercase()
                    }
                };
                for sym in self.lookup.by_name(target) {
                    if !kind(edge_kind, &sym.kind) {
                        continue;
                    }
                    if path_stem_matches(&sym.file_path.to_lowercase(), &leaf) {
                        return Some(self.resolution(sym.id, "default_module_anchor"));
                    }
                }
            }
            ModuleAnchorBind::MemberOfModuleType => {
                let target_norm = normalize_name(norm, target);
                for member in self.lookup.members_of(module) {
                    if normalize_name(norm, &member.name) == target_norm
                        && kind(edge_kind, &member.kind)
                    {
                        return Some(self.resolution(member.id, "default_module_anchor"));
                    }
                }
            }
        }
        None
    }

    /// Strategy — import-scoped bind of a bare target to an EXTERNAL symbol.
    ///
    /// The regular ladder deliberately excludes externals
    /// (`resolve_via_unique_internal_name` filters `is_external_file`;
    /// `resolve_via_ranked_candidates` admits externals but only on a score
    /// margin, ungated by the import set). This binds a bare `target` to an
    /// external symbol whose FILE is named by one of the resolving file's
    /// non-relative imports.
    ///
    /// `ext_match` selects how the external file is matched:
    ///   - `PkgSegment` — the file's `ext:<lang>:<pkg>` package segment equals
    ///     an import root, OR starts with `{root}-` for package families
    ///     (`aws-sdk-s3` under gem `aws`).
    ///   - `FileStemOrDir` — the file's basename-stem / a dir-segment equals an
    ///     import LEAF (last path segment, `std/` / `pkg/` prefix stripped) or
    ///     an import PACKAGE (first path segment) under `path_stem_matches`, for
    ///     ecosystems whose externals are named by file rather than an
    ///     `ext:`-package boundary.
    ///
    /// Gated by `external_by_import` so non-opted-in languages never touch
    /// externals here.
    pub fn resolve_via_external_by_import(
        &self,
        _cfg: &ExternalByImport,
        ext_match: ExtMatch,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() || target.contains('.') || target.contains("::") {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let matcher = match ext_match {
            ExtMatch::PkgSegment => {
                let import_roots: Vec<&str> = self
                    .file_ctx
                    .imports
                    .iter()
                    .filter_map(|imp| {
                        let m = imp.module_path.as_deref()?;
                        if m.starts_with('.') {
                            return None;
                        }
                        Some(m.split('/').next().unwrap_or(m))
                    })
                    .collect();
                if import_roots.is_empty() {
                    return None;
                }
                ExtFileMatcher::PkgSegment(import_roots)
            }
            ExtMatch::FileStemOrDir => {
                // Leaf = last path segment (with `std/` / `pkg/` prefix dropped);
                // package = first path segment (skipping the `std` / `pkg` root).
                let mut needles: Vec<String> = Vec::new();
                for imp in &self.file_ctx.imports {
                    let Some(m) = imp.module_path.as_deref() else {
                        continue;
                    };
                    if m.starts_with('.') {
                        continue;
                    }
                    let stripped =
                        m.strip_prefix("std/").or_else(|| m.strip_prefix("pkg/")).unwrap_or(m);
                    let leaf = stripped.rsplit('/').next().unwrap_or(stripped);
                    if !leaf.is_empty() {
                        needles.push(leaf.to_lowercase());
                    }
                    let pkg = m.split('/').next().unwrap_or(m);
                    if pkg != "std" && pkg != "pkg" && !pkg.is_empty() {
                        needles.push(pkg.to_lowercase());
                    }
                }
                if needles.is_empty() {
                    return None;
                }
                ExtFileMatcher::FileStemOrDir(needles)
            }
        };
        for sym in self.lookup.by_name(target) {
            if !self.lookup.is_external_file(&sym.file_path) {
                continue;
            }
            if !kind(edge_kind, &sym.kind) {
                continue;
            }
            let matched = match &matcher {
                ExtFileMatcher::PkgSegment(roots) => {
                    let pkg_seg = external_package_segment(&sym.file_path);
                    !pkg_seg.is_empty()
                        && roots
                            .iter()
                            .any(|root| pkg_seg == *root || pkg_seg.starts_with(&format!("{root}-")))
                }
                ExtFileMatcher::FileStemOrDir(needles) => {
                    let file_lower = sym.file_path.to_lowercase();
                    needles.iter().any(|n| path_stem_matches(&file_lower, n))
                }
            };
            if matched {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: RESOLVED_CONFIDENCE,
                    strategy: "default_external_by_import",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        None
    }

    /// Strategy — bare target in a sibling file of the SAME directory.
    ///
    /// For a language whose package IS its directory (Odin), a bare reference
    /// to a symbol declared in another file of the same directory is the
    /// canonical resolution — there is no `module` to anchor on. Bind any
    /// kind-compatible `by_name(target)` candidate whose immediate parent-dir
    /// basename equals the source file's immediate parent-dir basename.
    ///
    /// Module-independent: it does NOT consult `r.module`, so it runs even when
    /// the ref carries none (unlike `resolve_via_module_anchor`, which early-
    /// returns on a missing module). Selected by `ModuleScope::SameDir` via
    /// `resolve_via_module_scope`.
    pub fn resolve_via_same_dir(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() || target.contains('.') || target.contains("::") || target.contains('/')
        {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let src_parent = parent_dir_basename(&self.file_ctx.file_path)?;
        for sym in self.lookup.by_name(target) {
            if !kind(edge_kind, &sym.kind) {
                continue;
            }
            if parent_dir_basename(&sym.file_path).as_deref() == Some(src_parent.as_str()) {
                return Some(self.resolution(sym.id, "default_same_dir"));
            }
        }
        None
    }

    /// Strategy — bare target declared elsewhere in the SAME module, with no
    /// import and no chain to root it.
    ///
    /// The module boundary is parameterized by `ModuleScope`:
    /// - `Off` — inert, the rung never fires.
    /// - `SameDir` — delegates to `resolve_via_same_dir` (the parent dir IS the
    ///   module: Odin/MATLAB same-package references), strategy `default_same_dir`.
    /// - `SourcesTargetSubtree` — the SwiftPM whole-module case: every file under
    ///   one `Sources/<Target>/` (or `Tests/<Target>/`) subtree compiles into one
    ///   module and sees the others without import. A candidate is in-module iff
    ///   it shares the source's `module_subtree_prefix`. Over the in-module +
    ///   internal + kind-compatible candidates the unique-internal-name dedup
    ///   convention applies: bind iff EXACTLY ONE survives, else decline. Off the
    ///   `Sources/<Target>/` layout the prefix is `None` and the rung is inert —
    ///   never a same-dir fallback.
    ///
    /// Declines on a dotted / `::` / `/`-bearing or empty target (mirrors
    /// `resolve_via_same_dir` / `resolve_via_unique_internal_name`). Runs late in
    /// the ladder, so any real import / structural rung wins first;
    /// decline-over-guess is preserved.
    pub fn resolve_via_module_scope(
        &self,
        scope: ModuleScope,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        match scope {
            ModuleScope::Off => None,
            ModuleScope::SameDir => self.resolve_via_same_dir(kind),
            ModuleScope::SourcesTargetSubtree => self.resolve_via_sources_target_subtree(kind),
        }
    }

    /// The `SourcesTargetSubtree` arm of `resolve_via_module_scope`.
    fn resolve_via_sources_target_subtree(
        &self,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty()
            || target.contains('.')
            || target.contains("::")
            || target.contains('/')
        {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let src_prefix = module_subtree_prefix(&self.file_ctx.file_path)?;
        let mut compatible: Vec<&SymbolInfo> = self
            .lookup
            .by_name(target)
            .iter()
            .filter(|sym| !self.lookup.is_external_file(&sym.file_path))
            .filter(|sym| kind(edge_kind, &sym.kind))
            .filter(|sym| {
                module_subtree_prefix(&sym.file_path).as_deref() == Some(src_prefix.as_str())
            })
            .collect();
        // Dedup on (qualified_name, kind): one logical symbol indexed twice is
        // a single candidate for ambiguity purposes (mirrors
        // `resolve_via_unique_internal_name`).
        compatible.sort_by(|a, b| {
            a.qualified_name
                .cmp(&b.qualified_name)
                .then(a.kind.cmp(&b.kind))
        });
        compatible.dedup_by(|a, b| a.qualified_name == b.qualified_name && a.kind == b.kind);
        if compatible.len() != 1 {
            return None;
        }
        Some(self.resolution(compatible[0].id, "default_module_scope"))
    }

    /// Strategy — dotted target whose HEAD names an in-file alias declaration.
    ///
    /// A dotted target `{head}.{rest}` whose `{head}` is a declaration in the
    /// resolving file binds to that declaration: the head is an alias block the
    /// rest of the target reads against (an HCL provider alias —
    /// `google.compute_instance` where `google` is a `provider` block in the
    /// file). Truncate at the first `.`, bind the head.
    ///
    /// The head is declined when empty or when it carries a `_`: a `_`-bearing
    /// head is a provider RESOURCE TYPE (`aws_instance.web`), not an alias, and
    /// must not bind to an in-file declaration. `require_kind`, when `Some`,
    /// restricts the in-file candidate to that kind; `None` accepts any
    /// kind-compatible one. Gated by `HeadAliasBind::OnSameFile` — `Off` (every
    /// other language) never reaches the in-file scan.
    pub fn resolve_via_head_alias(
        &self,
        cfg: HeadAliasBind,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        let HeadAliasBind::OnSameFile { require_kind } = cfg else {
            return None;
        };
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let dot = target.find('.')?;
        let head = &target[..dot];
        if head.is_empty() || head.contains('_') {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        for sym in self.lookup.in_file(&self.file_ctx.file_path) {
            if sym.name != head {
                continue;
            }
            if let Some(req) = require_kind {
                if sym.kind != req {
                    continue;
                }
            }
            if kind(edge_kind, &sym.kind) {
                return Some(self.resolution(sym.id, "default_head_alias"));
            }
        }
        None
    }

    /// Strategy — bare target bound to the MODULE symbol whose qname IS an
    /// import's full module path.
    ///
    /// A namespace-qualified import (`alias MyApp.Foo`) brings the bare name
    /// `Foo` into scope bound to the module `MyApp.Foo` itself, not to a member
    /// under it. When an import's `imported_name` equals the bare `target`, the
    /// answer is the symbol looked up by the import's `module_path` qname. The
    /// file-scoped/file-import rungs key on a FILE PATH and a member name, so
    /// they cannot bind a bare alias to a module qname; this rung does the
    /// by-qname-equals-import-full-path lookup the others skip. Gated by
    /// `enabled`; off (every non-opted language) returns immediately.
    pub fn resolve_via_alias_module_qname(
        &self,
        enabled: bool,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        if !enabled {
            return None;
        }
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty()
            || target.contains('.')
            || target.contains("::")
            || target.contains('/')
        {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        for import in &self.file_ctx.imports {
            if import.imported_name != target {
                continue;
            }
            let Some(full_module) = import.module_path.as_deref() else {
                continue;
            };
            if let Some(sym) = self.lookup.by_qualified_name(full_module) {
                if kind(edge_kind, &sym.kind) {
                    return Some(self.resolution(sym.id, "default_alias_module_qname"));
                }
            }
        }
        None
    }

    /// Strategy — bare target bound to a symbol in a file-naming import.
    ///
    /// An import whose `module_path` names a FILE (a Robot `.robot` / `.resource`
    /// resource import, a Python-library file) brings that file's members into
    /// bare-name scope. Two passes, most-specific first:
    ///
    ///   1. SYMBOL-NAME pass — scan each import's file for a kind-compatible
    ///      symbol whose name matches `target` under `norm`. Covers resource
    ///      keywords and Python-library methods callable by their own name.
    ///   2. ALIAS-DECODE pass (only when `alias_decode` is set) — match `target`
    ///      against an import entry's `imported_name` and bind the symbol named
    ///      by that entry's decoded `alias`. The import table carries the
    ///      target-name → owning-symbol mapping for names that are not
    ///      themselves symbol identifiers (`@keyword("Add ${n} items")` aliases,
    ///      `KEYWORDS`-dict / `get_keyword_names` entries).
    ///
    /// `wildcard_only` restricts both passes to imports flagged `is_wildcard`
    /// (the file-import flag for languages that mark member-bearing file imports
    /// wildcard). `module_path` is treated as the imported file path directly —
    /// these imports already carry the resolved on-disk path (the language's
    /// `build_file_context` resolves resource basenames before the ladder runs),
    /// so each scan is a single `in_file` lookup per import. Gated by
    /// `FileScopedImports::On`; `Off` (every other language) never scans.
    pub fn resolve_via_file_scoped_import(
        &self,
        cfg: FileScopedImports,
        norm: NameNormalization,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        let FileScopedImports::On {
            wildcard_only,
            alias_decode,
        } = cfg
        else {
            return None;
        };
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let target_norm = normalize_name(norm, target);
        // Pass 1 — match the target against a symbol NAME in the imported file.
        for import in &self.file_ctx.imports {
            if wildcard_only && !import.is_wildcard {
                continue;
            }
            let Some(path) = import.module_path.as_deref() else {
                continue;
            };
            for sym in self.lookup.in_file(path) {
                if normalize_name(norm, &sym.name) == target_norm && kind(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: RESOLVED_CONFIDENCE,
                        strategy: "default_file_scoped_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        // Pass 2 — match the target against an import entry's `imported_name`
        // and bind the symbol named by its decoded `alias`.
        if let Some(decode) = alias_decode {
            return self.resolve_via_alias_decoded_import(
                decode,
                wildcard_only,
                norm,
                &target_norm,
                edge_kind,
                kind,
            );
        }
        None
    }

    /// Alias-decode pass of `resolve_via_file_scoped_import`. For each scanned
    /// import entry whose `imported_name` matches the target under `norm`,
    /// decode the entry's `alias` as `{type}{separator}{member}` and bind:
    ///   - the symbol named `member` (most specific), else
    ///   - the symbol named `type`, else
    ///   - the first `fallback_kind` symbol in the file (the dispatch class).
    /// An entry with no `alias` only participates through the fallback, so a
    /// plain (non-aliased) import never binds here.
    fn resolve_via_alias_decoded_import(
        &self,
        decode: AliasDecode,
        wildcard_only: bool,
        norm: NameNormalization,
        target_norm: &str,
        edge_kind: EdgeKind,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        for import in &self.file_ctx.imports {
            if wildcard_only && !import.is_wildcard {
                continue;
            }
            if normalize_name(norm, &import.imported_name) != target_norm {
                continue;
            }
            let Some(path) = import.module_path.as_deref() else {
                continue;
            };
            let (type_name, member_name) = match import.alias.as_deref() {
                Some(alias) => match alias.split_once(decode.separator) {
                    Some((t, m)) => (
                        (!t.is_empty()).then_some(t),
                        (!m.is_empty()).then_some(m),
                    ),
                    None => ((!alias.is_empty()).then_some(alias), None),
                },
                None => (None, None),
            };
            // Most specific: a named member.
            if let Some(member) = member_name {
                for sym in self.lookup.in_file(path) {
                    if sym.name == member && kind(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: RESOLVED_CONFIDENCE,
                            strategy: "default_alias_decoded_import",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
            // Next: the named owning type.
            if let Some(ty) = type_name {
                for sym in self.lookup.in_file(path) {
                    if sym.name == ty && kind(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: RESOLVED_CONFIDENCE,
                            strategy: "default_alias_decoded_import",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
            // Fallback: the dispatch type the file owns.
            if let Some(fallback_kind) = decode.fallback_kind {
                for sym in self.lookup.in_file(path) {
                    if sym.kind == fallback_kind && kind(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: RESOLVED_CONFIDENCE,
                            strategy: "default_alias_decoded_import",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
        }
        None
    }

    /// Strategy — bare target scoped to a sibling WORKSPACE package.
    ///
    /// A bare import specifier (`@org/utils`, or its deep form
    /// `@org/utils/sub/mod`) that matches a sibling workspace package's declared
    /// name scopes the lookup to that package's symbol set: the
    /// kind-compatible `target`-named symbol whose file path contains the deep
    /// import's sub-path (when present), else the first kind-compatible
    /// same-named symbol in the package. The specifier is taken from the ref's
    /// own `module`, else from the file import that binds `target`.
    ///
    /// Reads `SymbolLookup::workspace_package_id` / `symbols_in_package` /
    /// `is_workspace_declared_name`, all of which default to "no workspace" for
    /// synthetic lookups. Gated by `workspace_packages`.
    pub fn resolve_via_workspace_package(
        &self,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        // Specifier source: the ref's own module, else the import that binds
        // this target by name.
        let specifier: Option<&str> = match self.ref_ctx.extracted_ref.module.as_deref() {
            Some(m) => Some(m),
            None => self
                .file_ctx
                .imports
                .iter()
                .find(|imp| imp.imported_name == target)
                .and_then(|imp| imp.module_path.as_deref()),
        };
        let specifier = specifier?;
        if !is_bare_module_specifier(specifier) {
            return None;
        }
        let pkg_id = self.lookup.workspace_package_id(specifier)?;
        let sub_path = workspace_sub_path(specifier, self.lookup);

        let mut fallback: Option<&SymbolInfo> = None;
        for sym in self.lookup.symbols_in_package(pkg_id) {
            if sym.name != target || !kind(edge_kind, &sym.kind) {
                continue;
            }
            if let Some(sub) = sub_path.as_deref() {
                if sym.file_path.contains(sub) {
                    return Some(self.resolution(sym.id, "default_workspace_package"));
                }
            }
            if fallback.is_none() {
                fallback = Some(sym);
            }
        }
        fallback.map(|sym| self.resolution(sym.id, "default_workspace_package"))
    }

    /// Strategy — bare single-identifier call bound to an ambient global.
    ///
    /// A bare `target` (no `.`) that no import binds may be an unimported
    /// ambient global: a test-framework / npm global recorded under the
    /// synthetic `__npm_globals__.<name>` namespace (jest `describe`, jQuery
    /// `$`), or a runtime/core-lib symbol whose `declare global` form lives in
    /// an ambient-global lib file (`Record`, `HTMLElement`, `setTimeout`).
    ///
    /// Probes `__npm_globals__.{target}` first, then every bare-qname candidate
    /// whose defining file is an ambient-global lib file. `instantiate_accepts_
    /// variable` relaxes kind-compat for an `Instantiates` ref against a lib
    /// `variable` — the core lib encodes constructors as `declare var X: { new():
    /// Y }`. Gated by `AmbientGlobals::On`.
    pub fn resolve_via_npm_globals(
        &self,
        cfg: AmbientGlobals,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        let AmbientGlobals::On {
            instantiate_accepts_variable,
        } = cfg
        else {
            return None;
        };
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        if target.is_empty() || target.contains('.') {
            return None;
        }
        if !matches!(
            edge_kind,
            EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates
        ) {
            return None;
        }
        // npm-globals namespace probe.
        let globals_candidate =
            format!("{}.{target}", crate::ecosystem::npm::NPM_GLOBALS_MODULE);
        if let Some(sym) = self.lookup.by_qualified_name(&globals_candidate) {
            if kind(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: RESOLVED_CONFIDENCE,
                    strategy: "default_npm_globals",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        // Ambient-global lib-file bare-qname probe.
        for candidate in self.lookup.all_by_qualified_name(target) {
            if !crate::indexer::resolve::engine::is_ambient_global_lib_path(&candidate.file_path) {
                continue;
            }
            let kind_ok = kind(edge_kind, &candidate.kind)
                || (instantiate_accepts_variable
                    && matches!(edge_kind, EdgeKind::Instantiates)
                    && candidate.kind == "variable");
            if !kind_ok {
                continue;
            }
            return Some(Resolution {
                target_symbol_id: candidate.id,
                confidence: RESOLVED_CONFIDENCE,
                strategy: "default_lib_globals",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
        None
    }

    /// Strategy — bare target first-match-bound in a flat global namespace.
    ///
    /// For a language with NO imports, namespace, or scope structure (SQL and
    /// other namespaceless DDL/config languages), a bare `target` binds to the
    /// FIRST kind-compatible, project-internal symbol of the same name. Unlike
    /// `resolve_via_unique_internal_name`, which declines on more than one
    /// candidate, this first-match-binds: duplicate names across files are
    /// common and there is no structure to disambiguate. External candidates are
    /// excluded so a project symbol always wins over a same-named external.
    /// Gated by `namespaceless_global_type_lookup`; off (every other language)
    /// returns immediately. Runs LAST in the ladder so any structural evidence
    /// wins first.
    pub fn resolve_via_namespaceless_global(
        &self,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        for sym in self.lookup.by_name(target) {
            if self.lookup.is_external_file(&sym.file_path) {
                continue;
            }
            if kind(edge_kind, &sym.kind) {
                return Some(self.resolution(sym.id, "default_namespaceless_global"));
            }
        }
        None
    }

    /// Strategy — template ref bound to a component/directive via its selector.
    ///
    /// A template `Calls` ref whose target names a component tag or attribute
    /// directive binds to the decorated class registered under that selector.
    /// The raw target is tried first, then each configured `NameTransform`
    /// (`<app-user-card>` arrives as `AppUserCard`; `PascalToKebab` yields the
    /// `app-user-card` map key). `SymbolLookup::selector_qname` answers the
    /// selector → class-qname map. Gated by `selector_resolution`.
    pub fn resolve_via_selector_map(
        &self,
        cfg: &SelectorResolution,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
    ) -> Option<Resolution> {
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        if !cfg.edge_kinds.contains(&edge_kind) {
            return None;
        }
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() {
            return None;
        }
        let mut candidates: Vec<Cow<'_, str>> = vec![Cow::Borrowed(target)];
        for transform in cfg.name_transforms {
            candidates.push(apply_name_transform(*transform, target));
        }
        for candidate in &candidates {
            let Some(class_qname) = self.lookup.selector_qname(candidate) else {
                continue;
            };
            let class_qname = class_qname.to_string();
            if let Some(sym) = self.lookup.by_qualified_name(&class_qname) {
                if kind(edge_kind, &sym.kind) {
                    return Some(self.resolution(sym.id, "default_selector_map"));
                }
            }
            // The class symbol may not be qname-keyed under the map's value
            // (export-wrapper qnames); fall back to a by-name scan that pins
            // the exact qname.
            let short = class_qname.rsplit('.').next().unwrap_or(&class_qname);
            for sym in self.lookup.by_name(short) {
                if sym.qualified_name == class_qname && kind(edge_kind, &sym.kind) {
                    return Some(self.resolution(sym.id, "default_selector_map"));
                }
            }
        }
        None
    }

    /// Run every strategy in canonical order, returning the first hit.
    ///
    /// Canonical order, most-specific evidence first. The two profile-gated
    /// strategies (module anchor, external-by-import) are off for this
    /// fn-pointer path — `resolve_all` passes inert profile data:
    ///   0.  import path   — template-include FILE binding (gated on
    ///       `import_resolution`; fires only for `Imports` refs)
    ///   0b. module anchor — `r.module` bound to project symbols (gated on
    ///       `module_anchor`); a missed terminal anchor ends the ladder
    ///   1.  scope chain  — innermost enclosing scope wins
    ///   2.  same file    — sibling symbol in the source file
    ///   2b. file-scoped import — bare target in a file-naming import's file
    ///        (gated on `file_scoped_imports`)
    ///   3.  self keyword — `this`/`self`/`super` against the enclosing type
    ///   4.  enclosing member — inherited member of the enclosing type
    ///   5.  r.module     — extractor-set module prefix
    ///   6.  qname exact  — dotted target matches a stored qname
    ///   6b. head alias   — dotted target's head names an in-file declaration
    ///        (gated on `head_alias`)
    ///   7.  chain prefix — second-to-last chain segment matches an import
    ///   8.  reexport chain — import points at a re-exporting package
    ///   9.  file import  — bare target matches an imported name
    ///   10. reexport follow — import points at a module that re-exports the name
    ///   11. aliased import — import specifier rewritten through a path alias
    ///   12. namespace import — dotted import expanded against the target
    ///   13. ambient namespace path — qname ends with the dotted target
    ///   14. same namespace — file's declared namespace + target
    ///   15. imported namespace — candidate qname is prefixed by an import
    ///   16. ambient package — candidate lives in a declared ambient pkg
    ///   16b. external-by-import — bare target bound to an import-scoped
    ///        external symbol (gated on `external_by_import`)
    ///   16c. module scope — bare same-module target with no import: a sibling
    ///        in the same dir (`SameDir`) or the same `Sources/<Target>/`
    ///        subtree (`SourcesTargetSubtree`); gated on `module_scope`,
    ///        module-independent
    ///   17. wildcard import — bare target under a wildcard import's namespace
    ///   18. generic param — bare target matches a declared generic parameter
    ///   19. namespaceless global — bare target first-match-bound to an internal
    ///        symbol (gated on `namespaceless_global_type_lookup`; dead last)
    ///
    /// Every structural strategy binds through scope or import structure. The
    /// only non-structural binder is the gated, dead-last namespaceless-global
    /// rung for flat-global languages with no structure to bind through; every
    /// other language stays structure-only. The by-name method
    /// `resolve_via_unique_internal_name` is not part of the default binder.
    ///
    /// Language hooks that want a different order or additional language-
    /// specific strategies call individual methods themselves.
    pub fn resolve_all(&self) -> Option<Resolution> {
        let kind = self.kind_compatible;
        self.run_ladder(
            &move |edge, sym_kind| kind(edge, sym_kind),
            ChainQualification::None,
            &["."],
            None,
            LadderProfileData::INERT,
        )
    }

    /// Engine entry: run the same ladder but gate candidate kinds against a
    /// `LanguageProfile`'s `kind_compatible_table` rather than a fn-pointer.
    /// The engine cannot build a fn-pointer that closes over the profile's
    /// table, so it threads a table-driven closure through the shared ladder.
    pub fn resolve_all_with_profile(
        &self,
        profile: &crate::type_checker::profile::language_profile::LanguageProfile,
    ) -> Option<Resolution> {
        let table = profile.kind_compatible_table;
        // The index join is always `.`; add the profile separator only when it
        // differs, so a `.`-separator language probes `.` exactly once.
        let both = [".", profile.qname_separator];
        let separators: &[&str] = if profile.qname_separator == "." {
            &both[..1]
        } else {
            &both[..]
        };
        self.run_ladder(
            &move |edge, sym_kind| kind_ok_table(table, edge, sym_kind),
            profile.chain_qualification,
            separators,
            profile.import_resolution.as_ref(),
            LadderProfileData {
                module_skip: profile.module_skip,
                module_anchor: profile.module_anchor,
                module_anchor_terminal: profile.module_anchor_terminal,
                relative_marker: profile.relative_marker,
                external_by_import: profile.external_by_import.as_ref(),
                ext_match: profile.ext_match,
                self_keywords: profile.self_keywords,
                ambient_namespace_prefixes: profile.ambient_namespace_prefixes,
                name_normalization: profile.name_normalization,
                module_scope: profile.module_scope,
                wildcard_match: profile.wildcard_match,
                head_alias: profile.head_alias,
                file_scoped_imports: profile.file_scoped_imports,
                alias_module_qname: profile.alias_module_qname,
                module_prefix_rewrites: profile.module_prefix_rewrites,
                workspace_packages: profile.workspace_packages,
                overload_pick_all: profile.overload_pick_all,
                ambient_globals: profile.ambient_globals,
                namespaceless_global_type_lookup: profile.namespaceless_global_type_lookup,
                explicit_member_import: profile.explicit_member_import,
                selector_resolution: profile.selector_resolution.as_ref(),
            },
        )
    }

    /// The strategy ladder, parameterised on the kind-compatibility predicate.
    /// Canonical order, most-specific evidence first (see the per-method docs).
    /// `chain_qual` gates the profile-specific import-shape strategies; the
    /// fn-pointer `resolve_all` path passes `None` so those stay off.
    /// `separators` is the set of qname joins the scope-visible probe tries
    /// (always `"."`, plus the profile separator when it differs).
    /// `import_resolution`, when present, runs the template-include strategy
    /// FIRST — it is the most specific evidence for an `Imports` ref and the
    /// only strategy that binds a path-stem target to a file symbol; for every
    /// other ref shape it short-circuits to `None`.
    ///
    /// `pd` carries the pure-data profile deltas: the module-string decline off
    /// `r.module` (runs FIRST, before any binding strategy, so a module that
    /// names a non-project provider leaves the ref for external classification),
    /// the module-anchor bind off `r.module` (runs just after `import_resolution`,
    /// the most specific evidence for a module-carrying ref), its terminal guard
    /// (a missed anchor on a non-`Imports` module-carrying ref ends the ladder so
    /// an external prefix isn't hijacked by a same-named local), the import-scoped
    /// external bind (near the end, around ambient), and the self-keyword strip
    /// plus name normalization threaded into the scope / same-file probes (both
    /// identity by default, so a case-sensitive language is byte-identical).
    fn run_ladder(
        &self,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
        chain_qual: ChainQualification,
        separators: &[&str],
        import_resolution: Option<&ImportResolution>,
        pd: LadderProfileData,
    ) -> Option<Resolution> {
        // Module-string decline: a ref whose extractor-set `module` names a
        // non-project provider declines before any binding strategy, so no
        // same-named project symbol binds and external classification brands it
        // after. The module-keyed sibling of the target-keyed `builtin_skip`.
        if let (Some(skip), Some(module)) =
            (pd.module_skip, self.ref_ctx.extracted_ref.module.as_deref())
        {
            if skip(module) {
                return None;
            }
        }
        // Most-specific evidence: a template include, a workspace-package
        // scope, a component selector, or a module anchor.
        if let Some(res) = import_resolution.and_then(|ir| self.resolve_via_import_path(ir)) {
            return Some(res);
        }
        if pd.workspace_packages {
            if let Some(res) = self.resolve_via_workspace_package(kind) {
                return Some(res);
            }
        }
        if let Some(res) = pd
            .selector_resolution
            .and_then(|cfg| self.resolve_via_selector_map(cfg, kind))
        {
            return Some(res);
        }
        if let Some(res) = self.resolve_via_module_anchor(
            pd.module_anchor,
            pd.relative_marker,
            pd.name_normalization,
            separators.last().copied().unwrap_or("."),
            pd.module_prefix_rewrites,
            pd.overload_pick_all,
            kind,
        ) {
            return Some(res);
        }
        // Terminal guard: a non-`Imports` ref that carries a module and opted
        // into the anchor must not fall to the bare-name strategies when the
        // anchor missed — a same-named local would hijack an external prefix.
        if pd.module_anchor_terminal
            && matches!(pd.module_anchor, ModuleAnchor::On(_))
            && self.ref_ctx.extracted_ref.module.is_some()
            && self.ref_ctx.extracted_ref.kind != EdgeKind::Imports
        {
            return None;
        }

        let result = self
            .resolve_via_scope_visible(kind, separators, pd.self_keywords, pd.name_normalization)
            .or_else(|| {
                self.resolve_via_same_file(kind, pd.self_keywords, pd.name_normalization)
            })
            .or_else(|| {
                self.resolve_via_file_scoped_import(
                    pd.file_scoped_imports,
                    pd.name_normalization,
                    kind,
                )
            })
            .or_else(|| self.resolve_via_self_keyword(kind))
            .or_else(|| self.resolve_via_enclosing_member(kind))
            .or_else(|| self.resolve_via_ref_module(kind))
            .or_else(|| {
                self.resolve_via_qname_exact(pd.overload_pick_all, kind, pd.name_normalization)
            })
            .or_else(|| self.resolve_via_head_alias(pd.head_alias, kind))
            .or_else(|| self.resolve_via_alias_module_qname(pd.alias_module_qname, kind))
            .or_else(|| self.resolve_via_chain_prefix(kind))
            .or_else(|| self.resolve_via_reexport_chain(kind))
            .or_else(|| self.resolve_via_explicit_member_import(pd.explicit_member_import, kind))
            .or_else(|| self.resolve_via_file_import(kind))
            .or_else(|| self.resolve_via_reexport_following())
            .or_else(|| self.resolve_via_aliased_import(kind))
            .or_else(|| self.resolve_via_namespace_import(kind, pd.name_normalization))
            .or_else(|| {
                (chain_qual == ChainQualification::PackageShortName)
                    .then(|| {
                        let sep = separators.last().copied().unwrap_or(".");
                        self.resolve_via_package_short_name(kind, sep)
                    })
                    .flatten()
            })
            .or_else(|| self.resolve_via_ambient_namespace_path(kind))
            .or_else(|| self.resolve_via_same_namespace(kind, pd.name_normalization))
            .or_else(|| self.resolve_via_imported_namespace(kind, pd.name_normalization))
            .or_else(|| self.resolve_via_ambient_package(kind))
            .or_else(|| {
                // A target under a profile-declared namespace alias
                // (`sys.concat`, `az.resourceId`) names a bare ambient symbol;
                // strip the prefix and retry the ambient-package probe.
                strip_ambient_prefix(
                    self.ref_ctx.extracted_ref.target_name.as_str(),
                    pd.ambient_namespace_prefixes,
                )
                .and_then(|leaf| self.resolve_via_ambient_package_named(leaf, kind))
            })
            .or_else(|| {
                pd.external_by_import
                    .and_then(|cfg| self.resolve_via_external_by_import(cfg, pd.ext_match, kind))
            })
            .or_else(|| self.resolve_via_module_scope(pd.module_scope, kind))
            .or_else(|| {
                self.resolve_via_wildcard_import(kind, pd.wildcard_match, pd.name_normalization)
            })
            .or_else(|| self.resolve_via_generic_param())
            // Last resort: an unimported ambient global (jest/jQuery/DOM/core
            // lib). Below the structural strategies so a project symbol always
            // wins; gated on `ambient_globals`.
            .or_else(|| self.resolve_via_npm_globals(pd.ambient_globals, kind))
            // Terminal first-match for a flat-global language (SQL and other
            // namespaceless DDL/config). Dead last so every structural rung
            // above wins first; gated on `namespaceless_global_type_lookup`.
            .or_else(|| {
                pd.namespaceless_global_type_lookup
                    .then(|| self.resolve_via_namespaceless_global(kind))
                    .flatten()
            });
        if result.is_none() {
            self.record_bare_name_chain_miss();
        }
        result
    }

    /// Record a chain miss with an empty `current_type` for non-trivial
    /// bare-name refs that failed every strategy. The externals stage's
    /// `locate_via_symbol_index` consults `SymbolLocationIndex` for the
    /// target name; if an external file owns a matching symbol it gets
    /// demand-pulled and a second resolve pass picks it up via the same
    /// strategies above.
    ///
    /// Generalises the rust-specific recording in `RustResolver::resolve`
    /// to every language consuming `DefaultResolver::resolve_all`. The
    /// SymbolLookup default `record_chain_miss` is a no-op so synthetic
    /// test lookups pay nothing.
    fn record_bare_name_chain_miss(&self) {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if self.ref_ctx.extracted_ref.module.is_some()
            || target.contains("::")
            || target.contains('.')
            || target.contains('/')
        {
            return;
        }
        let trivial = target.len() < 2
            || target.chars().next().map_or(true, |c| c == '_')
            || !target.chars().any(|c| c.is_alphabetic());
        if trivial {
            return;
        }
        self.lookup.record_chain_miss(
            crate::indexer::resolve::engine::ChainMiss {
                current_type: String::new(),
                target_name: target.to_string(),
                module: None,
            },
        );
    }

    /// Strategy — bare target brought into scope by a wildcard / static
    /// wildcard import.
    ///
    /// `import static org.junit.jupiter.api.Assertions.*;` (Java),
    /// `use foo::*;` (Rust), `from utils import *` (Python),
    /// `using namespace std;` (C++) bring every exported member of the
    /// imported namespace into bare-name scope.
    ///
    /// `mode` decides how a candidate is matched to a wildcard's module:
    ///   - `QnameUnder` — the candidate's qualified name lives directly under
    ///     the module path (`qname_directly_under`); the member is keyed under
    ///     the imported namespace.
    ///   - `FileStem` — the candidate's FILE names the module: its basename-stem
    ///     OR a path dir-segment equals the module name, under `norm` for the
    ///     name comparison and, when `underscore_prefix`, accepting a `{module}_…`
    ///     include-file stem. For languages whose unit import brings a FILE into
    ///     scope rather than a namespace (Pascal units / FPC includes).
    ///
    /// Returns None when no wildcard import matches OR when multiple candidates
    /// from different wildcards tie.
    pub fn resolve_via_wildcard_import(
        &self,
        kind: &dyn Fn(EdgeKind, &str) -> bool,
        mode: WildcardMatch,
        norm: NameNormalization,
    ) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() || target.contains('.') || target.contains("::") {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let wildcards: Vec<&str> = self
            .file_ctx
            .imports
            .iter()
            .filter(|imp| imp.is_wildcard)
            .filter_map(|imp| imp.module_path.as_deref())
            .filter(|m| !m.is_empty())
            .collect();
        if wildcards.is_empty() {
            return None;
        }
        let target_norm = normalize_name(norm, target);
        let mut hits: Vec<&SymbolInfo> = Vec::new();
        for sym in self.lookup.by_name(target) {
            if !kind(edge_kind, &sym.kind) {
                continue;
            }
            let under_a_wildcard = match mode {
                WildcardMatch::QnameUnder => {
                    wildcards.iter().any(|ns| qname_directly_under(&sym.qualified_name, ns))
                }
                WildcardMatch::FileStem { underscore_prefix } => {
                    // The candidate's surface name must match the ref under the
                    // profile's normalization before its file is checked — a
                    // case-insensitive language binds a differently-cased ref.
                    if normalize_name(norm, &sym.name) != target_norm {
                        false
                    } else {
                        let file_lower = sym.file_path.to_lowercase();
                        wildcards.iter().any(|ns| {
                            let ns_lower = ns.to_lowercase();
                            wildcard_file_stem_matches(&file_lower, &ns_lower, underscore_prefix)
                        })
                    }
                }
            };
            if under_a_wildcard {
                hits.push(sym);
            }
        }
        // Single hit — accept. Multiple — let the candidate-ranking
        // strategy below break the tie via richer signals.
        if hits.len() == 1 {
            return Some(self.resolution(hits[0].id, "default_wildcard_import"));
        }
        None
    }

    /// Strategy — multi-candidate disambiguation by ranking.
    ///
    /// When `by_name(target)` returns multiple kind-compatible candidates and
    /// none of the strict single-candidate strategies above could pick one,
    /// rank by deterministic signals derived from the data the resolver
    /// already has: same workspace package, manifest-declared dep, ambient
    /// path, file-path proximity, candidate visibility. Pick the top when it
    /// dominates the runner-up by `RANK_MARGIN`. Otherwise stay None — never
    /// guess.
    ///
    /// Catches the 88% of bucket A unresolved refs where the symbol exists
    /// in N external packages (e.g. `description` declared in hundreds of
    /// ARM templates, `expect` declared in every `@types/jest` variant) but
    /// the older strict path bailed on ambiguity.
    pub fn resolve_via_ranked_candidates(&self, kind: &dyn Fn(EdgeKind, &str) -> bool) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() || target.contains('.') || target.contains("::") || target.contains('/') {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let candidates: Vec<&SymbolInfo> = self
            .lookup
            .by_name(target)
            .iter()
            .filter(|sym| kind(edge_kind, &sym.kind))
            .collect();
        if candidates.len() < 2 {
            // Zero — nothing to resolve to. Exactly one — the earlier strict
            // `resolve_via_unique_internal_name` / `resolve_via_ambient_package`
            // strategies already had their chance. Either case: stay out.
            return None;
        }
        let mut scored: Vec<(i32, &SymbolInfo)> = candidates
            .iter()
            .map(|sym| (self.score_candidate(sym), *sym))
            .collect();
        // Highest score first. Tie-break by id for determinism (same input →
        // same output across runs, even on hash-randomised lookups).
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
        let (top_score, top) = scored[0];
        let (runner_score, _) = scored[1];
        if top_score - runner_score < RANK_MARGIN {
            return None;
        }
        Some(self.resolution(top.id, "default_ranked_candidate"))
    }

    /// Score a candidate against the current ref context. Higher = better.
    /// Built from signals already on the lookup / file_ctx / ref_ctx — no
    /// new state, no hardcoded names.
    fn score_candidate(&self, sym: &SymbolInfo) -> i32 {
        let mut s: i32 = 0;

        // Same workspace package as caller. Strongest signal — a workspace
        // boundary is a strong intent marker, and same-package resolution
        // beats almost everything else.
        if let (Some(caller_pkg), Some(sym_pkg)) =
            (self.ref_ctx.file_package_id, sym.package_id)
        {
            if caller_pkg == sym_pkg {
                s += 1000;
            }
        }

        // Caller has an import statement that points at this candidate's
        // package or namespace. Two shapes:
        //   (a) workspace package id match — `import { X } from 'pkg-a'`
        //       resolves `pkg-a` to a workspace package id, and the
        //       candidate's package_id matches.
        //   (b) qname-prefix match — candidate's qualified_name starts with
        //       the import's module path (dot/slash/double-colon normalised).
        let mut matched_workspace_import = false;
        for import in &self.file_ctx.imports {
            let Some(mod_path) = import.module_path.as_deref() else { continue };
            if let Some(wp_id) = self.lookup.workspace_package_id(mod_path) {
                if Some(wp_id) == sym.package_id {
                    s += 500;
                    matched_workspace_import = true;
                }
            }
            if qname_under_module(&sym.qualified_name, mod_path) {
                s += 300;
            }
        }
        let _ = matched_workspace_import;

        // Ambient candidates (TS `declare global`, @types/* packages, lib.dom)
        // are project-declared in-scope providers.
        if self.lookup.is_ambient_path(&sym.file_path) {
            s += 200;
        }

        // File-path proximity. Each shared directory segment is +10. Internal
        // candidates near the caller usually win; external candidates score 0
        // here because their `ext:` prefix shares nothing with the caller.
        s += path_proximity_score(&self.file_ctx.file_path, &sym.file_path);

        // External candidates take a small penalty proportional to nesting
        // depth — `ext:idx:.../lib/X.d.ts` beats `ext:idx:.../deep/nested/.../X.d.ts`.
        // This lets the most canonical version of a duplicated external
        // symbol win when no other signal disambiguates.
        if self.lookup.is_external_file(&sym.file_path) {
            let depth = sym.file_path.matches('/').count() as i32;
            s -= depth.min(20);
        }

        // Visibility hint. Public is preferable; private external symbols
        // are almost never the intended target of a cross-file ref.
        match sym.visibility.as_deref() {
            Some("public") => s += 50,
            Some("private") => s -= 200,
            _ => {}
        }

        s
    }

    fn resolution(&self, target_symbol_id: i64, strategy: &'static str) -> Resolution {
        Resolution {
            target_symbol_id,
            confidence: 1.0,
            strategy,
            resolved_yield_type: None,
            flow_emit: None,
        }
    }
}

/// The precomputed match data for `resolve_via_external_by_import`, built once
/// from the file's imports before the candidate loop. `PkgSegment` carries the
/// import roots; `FileStemOrDir` carries the lowercased import leaves and
/// packages probed against each candidate's file path.
enum ExtFileMatcher<'m> {
    PkgSegment(Vec<&'m str>),
    FileStemOrDir(Vec<String>),
}

/// Profile-table-driven kind compatibility check. Unrecognised symbol-kind
/// strings default to permissive so an extractor typo doesn't silently hide a
/// real symbol. Mirrors `core::members::kind_matches` / the bare-name check.
fn kind_ok_table(table: KindTable, edge: EdgeKind, sym_kind: &str) -> bool {
    match SymbolKind::from_str(sym_kind) {
        Ok(parsed) => KindCompatibility::check(table, edge, parsed),
        Err(_) => true,
    }
}

/// Minimum score margin the top candidate must beat the runner-up by to
/// claim the resolution. Set conservatively — any closer than this and the
/// strategy stays out and lets the ref land as honestly unresolved.
const RANK_MARGIN: i32 = 100;

/// Upper bound on inheritance-chain climbing in `resolve_via_enclosing_member`.
/// Matches the chain walker's inheritance-walk bound.
const MAX_INHERITANCE_DEPTH: usize = 8;

/// `true` when `kind` names a type a `this`/`self` keyword or an inherited
/// member can attach to — a class-like declaration, not a namespace,
/// function, or value.
fn is_type_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "struct"
            | "interface"
            | "enum"
            | "trait"
            | "object"
            | "record"
            | "protocol"
            | "actor"
            | "mixin"
            | "annotation"
    )
}

/// `true` when `qualified_name` reads as `module_path` (slash / colon /
/// dot-separated) prefix followed by `.` and one or more segments.
fn qname_under_module(qualified_name: &str, module_path: &str) -> bool {
    let dotted = module_path.replace("::", ".").replace('/', ".");
    if dotted.is_empty() { return false }
    let needle = format!("{dotted}.");
    qualified_name.starts_with(&needle) || qualified_name == dotted
}

/// Stricter form of `qname_under_module`: candidate must sit DIRECTLY
/// under the module — exactly one segment deeper. `Assertions.assertTrue`
/// matches `Assertions`; `Assertions.Nested.foo` does not.
fn qname_directly_under(qualified_name: &str, module_path: &str) -> bool {
    let dotted = module_path.replace("::", ".").replace('/', ".");
    if dotted.is_empty() { return false }
    let needle = format!("{dotted}.");
    let Some(rest) = qualified_name.strip_prefix(needle.as_str()) else { return false };
    !rest.contains('.')
}

/// Shared-directory-prefix score between two file paths. Returns 10 ×
/// number of shared leading directory segments. Path separators are
/// normalised to `/`. Caller's filename is dropped before comparing —
/// a sibling file in the same dir counts, the same file does not.
fn path_proximity_score(caller_path: &str, candidate_path: &str) -> i32 {
    let caller_norm = caller_path.replace('\\', "/");
    let candidate_norm = candidate_path.replace('\\', "/");
    let caller_dir = caller_norm
        .rsplit_once('/')
        .map(|(d, _)| d)
        .unwrap_or("");
    let candidate_dir = candidate_norm
        .rsplit_once('/')
        .map(|(d, _)| d)
        .unwrap_or("");
    let caller_segs: Vec<&str> = caller_dir.split('/').filter(|s| !s.is_empty()).collect();
    let candidate_segs: Vec<&str> = candidate_dir
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let shared = caller_segs
        .iter()
        .zip(candidate_segs.iter())
        .take_while(|(a, b)| a == b)
        .count() as i32;
    shared * 10
}

/// Yield every namespace prefix worth trying for an import: the module
/// path (when dotted) and the imported name (when dotted and distinct).
fn candidate_namespace_prefixes(
    import: &crate::indexer::resolve::engine::ImportEntry,
) -> impl Iterator<Item = &str> {
    let mut prefixes: Vec<&str> = Vec::with_capacity(2);
    if let Some(m) = import.module_path.as_deref() {
        if !m.is_empty() {
            prefixes.push(m);
        }
    }
    let name = import.imported_name.as_str();
    if !name.is_empty() && !prefixes.iter().any(|&p| p == name) {
        prefixes.push(name);
    }
    prefixes.into_iter()
}

/// File path's basename stem or any path segment matches the module
/// (case-insensitive on both inputs). External `ext:<lang>:<pkg>` paths
/// match on the trailing colon-delimited component.
fn path_stem_matches(file_path_lower: &str, module_lower: &str) -> bool {
    if module_lower.is_empty() {
        return false;
    }
    let normalized = file_path_lower.replace('\\', "/");
    if let Some(basename) = normalized.rsplit('/').next() {
        if let Some((stem, _ext)) = basename.rsplit_once('.') {
            if stem == module_lower {
                return true;
            }
        } else if basename == module_lower {
            return true;
        }
    }
    normalized.split('/').any(|seg| {
        seg == module_lower
            || seg.split(':').next_back().map_or(false, |tail| tail == module_lower)
    })
}

/// The qname-probe separators a module anchor tries: always the universal `.`
/// index join, plus the profile separator when it differs. So a `::`-keyed
/// module probes both `crate::db.new` and `crate::db::new`; a `.`-keyed one
/// probes `.` exactly once.
fn module_anchor_separators(sep: &str) -> impl Iterator<Item = &str> {
    [".", sep].into_iter().take(if sep == "." { 1 } else { 2 })
}

/// Map every module-path separator (the universal `.` and the profile's, e.g.
/// Rust's `::`) to a forward slash so the result can be matched against a file
/// path. `crate::db` → `crate/db`, `a.b.c` → `a/b/c`.
fn separators_to_slash(module: &str, sep: &str) -> String {
    let dotted = if sep == "." {
        module.to_string()
    } else {
        module.replace(sep, ".")
    };
    dotted.replace('.', "/")
}

/// The trailing path-segment of a module under either separator — the file-like
/// leaf used as a containment fallback. `crate::db` → `db`, `a.b.c` → `c`.
fn module_leaf<'a>(module: &'a str, sep: &str) -> &'a str {
    let after_profile = if sep != "." {
        module.rsplit(sep).next().unwrap_or(module)
    } else {
        module
    };
    after_profile.rsplit('.').next().unwrap_or(after_profile)
}

/// `path_stem_matches` extended with the include-file underscore-prefix probe.
/// True when the file's basename-stem / a dir-segment equals `module_lower`
/// (the shared `path_stem_matches` rule) OR, when `underscore_prefix`, the
/// basename-stem is `{module_lower}_…` — a unit's symbols split across
/// `{unit}_part.inc` siblings. Both inputs are already lowercased.
fn wildcard_file_stem_matches(
    file_path_lower: &str,
    module_lower: &str,
    underscore_prefix: bool,
) -> bool {
    if path_stem_matches(file_path_lower, module_lower) {
        return true;
    }
    if !underscore_prefix || module_lower.is_empty() {
        return false;
    }
    let normalized = file_path_lower.replace('\\', "/");
    let basename = normalized.rsplit('/').next().unwrap_or(&normalized);
    let stem = basename.rsplit_once('.').map(|(s, _)| s).unwrap_or(basename);
    stem.starts_with(&format!("{module_lower}_"))
}

/// The basename of a file path's immediate parent directory. Path separators
/// are normalized to `/`. Returns `None` when the path has no parent directory
/// (a bare filename). For `pkg/foo/bar.odin` returns `Some("foo")`.
fn parent_dir_basename(file_path: &str) -> Option<String> {
    let normalized = file_path.replace('\\', "/");
    let (dir, _file) = normalized.rsplit_once('/')?;
    Some(dir.rsplit('/').next().unwrap_or(dir).to_string())
}

/// The SwiftPM module-subtree prefix of a file path: the substring up to and
/// including `Sources/<seg>/` (or `Tests/<seg>/`). Every file under one such
/// subtree compiles into module `<seg>`, so two files share a module iff their
/// prefixes are equal. Path separators are normalized to `/`; the `Sources`/
/// `Tests` segment names are matched case-sensitively (SwiftPM uses exactly
/// those). Returns `None` when the path has no such prefix (a flat / Xcode
/// layout), which leaves the `SourcesTargetSubtree` rung inert.
fn module_subtree_prefix(file_path: &str) -> Option<String> {
    let normalized = file_path.replace('\\', "/");
    let segs: Vec<&str> = normalized.split('/').collect();
    // Need a root segment ("Sources"/"Tests"), a target segment, and at least
    // one more (the file) so the prefix is `<root>/<target>/`.
    segs.iter().enumerate().find_map(|(i, seg)| {
        if (*seg == "Sources" || *seg == "Tests") && i + 2 < segs.len() {
            Some(segs[..=i + 1].join("/") + "/")
        } else {
            None
        }
    })
}

/// Handles relative TS specifiers (`./catalog` → `src/catalog.ts`),
/// dotted namespaces (C# `System.Linq` → file in namespace `Linq`), and
/// last-segment fallback (file path contains the rightmost module
/// segment).
/// True for `./foo`, `../shared`, etc. — the kinds of specifiers that
/// resolve inside the project and don't cross a package boundary.
fn is_relative_specifier(spec: &str) -> bool {
    spec.starts_with("./") || spec.starts_with("../")
}

/// Normalize a name for the bare-name binding comparison. Applied identically
/// to a candidate's name and the ref's target before they are compared in
/// `resolve_via_same_file` / `resolve_via_scope_visible`.
///
/// `NameNormalization::None` is the identity transform and returns the input
/// borrowed unchanged — no allocation, byte-for-byte. A `Spec` whose deltas are
/// all off (no sigils, no prefixes, no chars, case-sensitive) is also identity
/// and borrows. Otherwise the transform runs in a fixed order so the same input
/// always normalizes the same way: strip a wrapping sigil pair, strip the first
/// matching leading prefix, remove the configured characters anywhere, then fold
/// ASCII case.
fn normalize_name(norm: NameNormalization, s: &str) -> Cow<'_, str> {
    let spec = match norm {
        NameNormalization::None => return Cow::Borrowed(s),
        NameNormalization::Spec(spec) => spec,
    };
    if is_identity_spec(&spec) {
        return Cow::Borrowed(s);
    }

    let mut cur = s;

    // 1. Sigil wrapper: when the name both starts with `prefix` and ends with
    //    `suffix`, drop both. The first matching pair wins.
    for (prefix, suffix) in spec.strip_sigils {
        if let Some(inner) = cur.strip_prefix(*prefix) {
            if let Some(inner) = inner.strip_suffix(*suffix) {
                cur = inner;
                break;
            }
        }
    }

    // 2. Leading prefix: drop the first declared prefix that matches. When the
    //    spec folds case, the prefix match folds too — a prefix runs before the
    //    case-fold step below, so a differently-cased call site (`when foo` vs a
    //    `When ` prefix) must still strip. Case-sensitive specs keep the exact
    //    byte-prefix match.
    for prefix in spec.strip_prefixes {
        let matched_len = if spec.case_insensitive {
            cur.get(..prefix.len())
                .filter(|head| head.eq_ignore_ascii_case(prefix))
                .map(|_| prefix.len())
        } else {
            cur.starts_with(*prefix).then_some(prefix.len())
        };
        if let Some(len) = matched_len {
            cur = &cur[len..];
            break;
        }
    }

    // 3 & 4. Remove the configured characters anywhere and fold case. Both need
    //        an owned buffer; build it once.
    let needs_char_strip = !spec.strip_chars.is_empty();
    if !needs_char_strip && !spec.case_insensitive {
        return Cow::Borrowed(cur);
    }
    let mut out = String::with_capacity(cur.len());
    for ch in cur.chars() {
        if needs_char_strip && spec.strip_chars.contains(&ch) {
            continue;
        }
        if spec.case_insensitive {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    Cow::Owned(out)
}

/// A `NormSpec` whose every field is the default (no sigils, no prefixes, no
/// chars, case-sensitive) is the identity transform — `normalize_name` borrows
/// rather than allocating for it.
fn is_identity_spec(spec: &NormSpec) -> bool {
    !spec.case_insensitive
        && spec.strip_chars.is_empty()
        && spec.strip_prefixes.is_empty()
        && spec.strip_sigils.is_empty()
}

/// Strip a leading `{kw}.` from `target` when `kw` is one of `self_keywords`
/// (Python `self.method` → `method`). Only the first matching keyword strips,
/// and only when followed by `.`. An empty `self_keywords` slice — the default
/// for languages with no self keyword — returns `target` unchanged.
fn strip_self_keyword<'t>(target: &'t str, self_keywords: &[&str]) -> &'t str {
    for kw in self_keywords {
        if let Some(rest) = target.strip_prefix(kw) {
            if let Some(after) = rest.strip_prefix('.') {
                return after;
            }
        }
    }
    target
}

/// Strip a leading `{prefix}.` when `prefix` is one of `ambient_prefixes`.
/// Returns the stripped leaf, or `None` when no prefix matches (so the caller
/// only retries the ambient probe for genuinely aliased targets). Mirrors
/// `strip_self_keyword`'s shape but signals a match via `Option`.
fn strip_ambient_prefix<'t>(target: &'t str, ambient_prefixes: &[&str]) -> Option<&'t str> {
    for prefix in ambient_prefixes {
        if let Some(rest) = target.strip_prefix(prefix) {
            if let Some(after) = rest.strip_prefix('.') {
                if !after.is_empty() {
                    return Some(after);
                }
            }
        }
    }
    None
}

/// The package segment of an external file path under the
/// `ext:<lang>:<pkg>/...` convention. For `ext:ruby:aws-sdk-s3/lib/x.rb`
/// returns `aws-sdk-s3`; for paths that don't match the three-colon shape
/// returns `""`. The `<pkg>` segment may itself contain `/`-separated path
/// parts — only the first is the package name.
fn external_package_segment(path: &str) -> &str {
    let Some(rest) = path.strip_prefix("ext:") else {
        return "";
    };
    let Some((_lang, after_lang)) = rest.split_once(':') else {
        return "";
    };
    after_lang.split('/').next().unwrap_or("")
}

/// Generate the ordered candidate file paths for a template-include target.
///
/// Data-driven by `ir`: name variants (`target`, plus a kebab form when
/// `kebab_variant`), then for each variant the source-dir base candidate set
/// (the base itself; its `.ext` forms; directory-index `entry.ext` forms
/// under the base; and the `_{stem}` sibling with its `.ext` forms when
/// `underscore_variant`). When `candidate_dirs` is `WalkUp`, the same base
/// candidate set is generated at `{ancestor}/{dir}/{variant}` for each named
/// `dir` across the source dir and up to `depth` ancestors.
fn import_path_candidates(
    source_dir: &std::path::Path,
    target: &str,
    ir: &ImportResolution,
) -> Vec<std::path::PathBuf> {
    use crate::indexer::resolve::engine::{camel_to_kebab, lexical_normalize};
    use std::path::PathBuf;

    let mut out: Vec<PathBuf> = Vec::with_capacity(32);

    // Append the full base candidate set for one base path. `base` is the
    // already-`source_dir`-joined, lexically-normalized variant path.
    let push_base_set = |out: &mut Vec<PathBuf>, base: PathBuf| {
        let already_ext = base
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| ir.extensions.contains(&e))
            .unwrap_or(false);
        out.push(base.clone());
        if !already_ext {
            let base_str = base.to_string_lossy().to_string();
            for ext in ir.extensions {
                out.push(PathBuf::from(format!("{base_str}.{ext}")));
            }
            for entry in ir.index_files {
                for ext in ir.extensions {
                    out.push(base.join(format!("{entry}.{ext}")));
                }
            }
        }
        if ir.underscore_variant {
            if let (Some(parent), Some(stem)) =
                (base.parent(), base.file_name().and_then(|n| n.to_str()))
            {
                let underscored = parent.join(format!("_{stem}"));
                out.push(underscored.clone());
                if !already_ext {
                    let und_str = underscored.to_string_lossy().to_string();
                    for ext in ir.extensions {
                        out.push(PathBuf::from(format!("{und_str}.{ext}")));
                    }
                }
            }
        }
    };

    let mut name_variants: Vec<String> = vec![target.to_string()];
    if ir.kebab_variant {
        if let Some(kebab) = camel_to_kebab(target) {
            name_variants.push(kebab);
        }
    }

    for variant in &name_variants {
        let direct = lexical_normalize(&source_dir.join(variant));
        push_base_set(&mut out, direct);

        if let CandidateDirs::WalkUp { dirs, depth } = ir.candidate_dirs {
            let mut current = Some(source_dir);
            let mut level = 0usize;
            while let Some(dir) = current {
                for d in dirs {
                    let base = lexical_normalize(&dir.join(d).join(variant));
                    push_base_set(&mut out, base);
                }
                level += 1;
                if level > depth {
                    break;
                }
                current = dir.parent();
            }
        }
    }

    out
}

/// A bare module specifier names a package, not a project-relative path:
/// it does not start with `.` or `/`, and is not a Windows drive path
/// (`C:/...`). The generic mirror of the TS `is_bare_specifier` predicate so
/// the engine can tell a package specifier from a relative one without the TS
/// plugin.
fn is_bare_module_specifier(spec: &str) -> bool {
    !spec.starts_with('.')
        && !spec.starts_with('/')
        && !(spec.len() >= 2 && spec.as_bytes()[1] == b':')
}

/// The ordered module-prefix candidates the `ByNameUnderModuleDir` anchor
/// probes for one ref: the literal `module` first, then — only for a bare
/// specifier under `ModulePrefixRewrites::On` — the DefinitelyTyped `@types/`
/// rewrites and the deep-import `/seg` peels. A relative specifier or
/// `Off` yields just the literal module.
fn module_prefix_candidates(module: &str, rewrites: ModulePrefixRewrites) -> Vec<String> {
    let mut out = vec![module.to_string()];
    let ModulePrefixRewrites::On {
        definitely_typed,
        deep_import_peel,
        ..
    } = rewrites
    else {
        return out;
    };
    if !is_bare_module_specifier(module) {
        return out;
    }
    if definitely_typed && !module.starts_with("@types/") {
        // `@scope/pkg` → `@types/scope__pkg`; `pkg` → `@types/pkg`.
        if let Some(rest) = module.strip_prefix('@') {
            if let Some(slash) = rest.find('/') {
                let scope = &rest[..slash];
                let pkg = &rest[slash + 1..];
                out.push(format!("@types/{scope}__{pkg}"));
            }
        } else {
            out.push(format!("@types/{module}"));
        }
    }
    if deep_import_peel && module.contains('/') {
        // Strip trailing `/seg` segments, stopping before a bare `@scope`.
        let mut path = module;
        while let Some(slash) = path.rfind('/') {
            let parent = &path[..slash];
            if parent.starts_with('@') && !parent.contains('/') {
                break;
            }
            path = parent;
            out.push(path.to_string());
        }
    }
    out
}

/// Whether the directory-containment fallback of `ByNameUnderModuleDir` is
/// declined for `module` — true only for a bare specifier when the rewrites
/// axis sets `decline_bare_directory_match`.
fn declines_bare_directory_match(module: &str, rewrites: ModulePrefixRewrites) -> bool {
    matches!(
        rewrites,
        ModulePrefixRewrites::On {
            decline_bare_directory_match: true,
            ..
        }
    ) && is_bare_module_specifier(module)
}

/// The sub-path portion of a deep workspace import — the remainder after the
/// longest declared-name prefix. `None` when `specifier` is itself a declared
/// workspace package name (no deep path) or no workspace package matches.
/// Mirrors the TS `sub_path_for_deep_import` shape over the `SymbolLookup`
/// trait so the engine needs no TS plugin.
fn workspace_sub_path(specifier: &str, lookup: &dyn SymbolLookup) -> Option<String> {
    if lookup.is_workspace_declared_name(specifier) {
        return None;
    }
    let mut path = specifier;
    while let Some(slash) = path.rfind('/') {
        path = &path[..slash];
        if lookup.is_workspace_declared_name(path) {
            return Some(specifier[path.len() + 1..].to_string());
        }
    }
    None
}

/// Apply a `NameTransform` to a ref target, yielding one selector-key
/// candidate. Borrows the input when the transform is identity for it.
fn apply_name_transform(transform: NameTransform, name: &str) -> Cow<'_, str> {
    match transform {
        NameTransform::PascalToKebab => pascal_to_kebab(name),
    }
}

/// `AppUserCard` → `app-user-card`: insert `-` at each interior uppercase
/// boundary and lowercase. A single-segment input with no interior uppercase
/// boundary (already lowercase or camelCase) is returned borrowed unchanged.
fn pascal_to_kebab(name: &str) -> Cow<'_, str> {
    let needs_split = name
        .char_indices()
        .any(|(i, c)| i > 0 && c.is_ascii_uppercase());
    if !needs_split && name.chars().all(|c| !c.is_ascii_uppercase()) {
        return Cow::Borrowed(name);
    }
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('-');
        }
        out.extend(ch.to_lowercase());
    }
    Cow::Owned(out)
}

fn file_path_matches_module(file_path: &str, module: &str) -> bool {
    if module.is_empty() {
        return false;
    }
    let normalized = file_path.replace('\\', "/");
    let cleaned = module.trim_start_matches("./").trim_start_matches("../");
    let stem = normalized
        .trim_end_matches(".ts")
        .trim_end_matches(".tsx")
        .trim_end_matches(".js")
        .trim_end_matches(".jsx")
        .trim_end_matches(".mts")
        .trim_end_matches(".cts")
        .trim_end_matches(".cs");
    if stem.ends_with(cleaned) || stem.ends_with(&cleaned.replace('.', "/")) {
        return true;
    }
    // Package-directory match: the module's full slash-form must appear as a
    // contiguous, segment-bounded run inside the path — `posthog.models`
    // (→ `posthog/models`) matches `posthog/models/person.py` because the run
    // sits between a leading `/` (or string start) and a trailing `/`. Matching
    // only the module's bare LEAF would be unsound: `org.assertj.core.api`
    // (leaf `api`) would bind a `…/junit/jupiter/api/Assertions.java` symbol by
    // coincidence, a false 1.0 bind. Requiring the whole dotted path keeps the
    // junit/assertj leaves from colliding while still resolving the package
    // (`__init__.py` re-export) shape the `stem.ends_with` branch misses.
    let dotted = cleaned.replace('.', "/");
    if dotted.is_empty() {
        return false;
    }
    path_contains_segment_run(&normalized, &dotted)
}

/// True when `run` (a `/`-joined path fragment) appears in `path` aligned to
/// path-segment boundaries on both sides — bounded by `/` or a string edge.
/// Distinguishes a real directory-prefix hit (`a/b` in `a/b/c.py`) from an
/// incidental substring (`api` in `capi/x.py`, or a leaf landing mid-segment).
fn path_contains_segment_run(path: &str, run: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = path[from..].find(run) {
        let start = from + rel;
        let end = start + run.len();
        let left_ok = start == 0 || path.as_bytes()[start - 1] == b'/';
        let right_ok = end == path.len() || path.as_bytes()[end] == b'/';
        if left_ok && right_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

#[cfg(test)]
#[path = "default_resolver_tests.rs"]
mod tests;
