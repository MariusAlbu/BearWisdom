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

use crate::indexer::resolve::engine::{
    FileContext, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::types::EdgeKind;

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
    pub fn resolve_via_ref_module(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let module = self.ref_ctx.extracted_ref.module.as_deref()?;

        for sep in [".", "::", "/", ":"] {
            let qname = format!("{module}{sep}{target}");
            if let Some(sym) = self.lookup.by_qualified_name(&qname) {
                if (self.kind_compatible)(edge_kind, &sym.kind) {
                    return Some(self.resolution(sym.id, "default_ref_module"));
                }
            }
        }

        let module_lower = module.to_lowercase();
        let last_seg_lower = module.rsplit('.').next().unwrap_or(module).to_lowercase();
        for sym in self.lookup.by_name(target) {
            if !(self.kind_compatible)(edge_kind, &sym.kind) {
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
    pub fn resolve_via_qname_exact(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        if !target.contains('.') && !target.contains("::") && !target.contains('/') {
            return None;
        }
        let sym = self.lookup.by_qualified_name(target)?;
        if !(self.kind_compatible)(edge_kind, &sym.kind) {
            return None;
        }
        Some(self.resolution(sym.id, "default_qname_exact"))
    }

    /// Strategy 3 — bare target matches a name in the file's import list.
    ///
    /// `import { Foo } from './foo'` then `Foo(...)`: the import brings
    /// `Foo` into local scope. Find a `target`-named symbol in any file
    /// whose path matches the import's module specifier.
    pub fn resolve_via_file_import(&self) -> Option<Resolution> {
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
                if !(self.kind_compatible)(edge_kind, &sym.kind) {
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
    pub fn resolve_via_namespace_import(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;

        for import in &self.file_ctx.imports {
            for prefix in candidate_namespace_prefixes(import) {
                if !prefix.contains('.') {
                    continue;
                }
                let qname = format!("{prefix}.{target}");
                if let Some(sym) = self.lookup.by_qualified_name(&qname) {
                    if (self.kind_compatible)(edge_kind, &sym.kind) {
                        return Some(self.resolution(sym.id, "default_namespace_import"));
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
    pub fn resolve_via_ambient_namespace_path(&self) -> Option<Resolution> {
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
            .filter(|sym| (self.kind_compatible)(edge_kind, &sym.kind))
            .min_by_key(|sym| sym.file_path.matches('/').count())?;
        Some(self.resolution(best.id, "default_ambient_namespace_path"))
    }

    /// Strategy 6 — same-file-namespace lookup.
    ///
    /// In C#, types in the same namespace are visible without a `using`.
    /// If the source file declares namespace X, candidates whose qname is
    /// `X.target` are in scope.
    pub fn resolve_via_same_namespace(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let ns = self.file_ctx.file_namespace.as_deref()?;
        if ns.is_empty() {
            return None;
        }
        let expected = format!("{ns}.{target}");
        for sym in self.lookup.by_name(target) {
            if sym.qualified_name == expected && (self.kind_compatible)(edge_kind, &sym.kind) {
                return Some(self.resolution(sym.id, "default_same_namespace"));
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
    pub fn resolve_via_imported_namespace(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        for sym in self.lookup.by_name(target) {
            if !(self.kind_compatible)(edge_kind, &sym.kind) {
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
        None
    }

    /// Strategy 8 — chain prefix (second-to-last segment) matches an import.
    ///
    /// `resolve::resolve_and_write()` with `use crate::indexer::resolve`:
    /// the chain prefix `resolve` matches the import; the import's
    /// module path tells us which directory the target lives in.
    pub fn resolve_via_chain_prefix(&self) -> Option<Resolution> {
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
            if !(self.kind_compatible)(edge_kind, &sym.kind) {
                continue;
            }
            if file_path_matches_module(&sym.file_path, module_path) {
                return Some(self.resolution(sym.id, "default_chain_prefix"));
            }
        }

        for sym in candidates {
            if !(self.kind_compatible)(edge_kind, &sym.kind) {
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
    pub fn resolve_via_reexport_chain(&self) -> Option<Resolution> {
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
                            .candidate_with_compatible_kind(target, id, edge_kind)
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
                                    .candidate_with_compatible_kind(suffix, id, edge_kind)
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

    /// Strategy 10 — ambient-package preference.
    ///
    /// Bare target whose only candidate that's kind-compatible lives in a
    /// project-declared ambient package (TS `tsconfig.json#types`, `@types/*`,
    /// `globals.d.ts`). The project opted into those packages as ambient
    /// providers, so an unimported reference to one of their members is
    /// the intended target — not an ambiguous bare-name guess.
    pub fn resolve_via_ambient_package(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let candidates = self.lookup.by_name(target);
        let ambient: Vec<&SymbolInfo> = candidates
            .iter()
            .filter(|sym| self.lookup.is_ambient_path(&sym.file_path))
            .filter(|sym| (self.kind_compatible)(edge_kind, &sym.kind))
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
    ) -> Option<i64> {
        let candidate = self
            .lookup
            .by_name(name)
            .iter()
            .find(|sym| sym.id == id)?;
        if (self.kind_compatible)(edge_kind, &candidate.kind) {
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
    pub fn resolve_via_unique_internal_name(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        if target.contains('.') || target.contains("::") || target.contains('/') {
            return None;
        }
        let candidates = self.lookup.by_name(target);
        let mut compatible: Vec<&SymbolInfo> = candidates
            .iter()
            .filter(|sym| !self.lookup.is_external_file(&sym.file_path))
            .filter(|sym| (self.kind_compatible)(edge_kind, &sym.kind))
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
    pub fn resolve_via_same_file(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        for sym in self.lookup.in_file(&self.file_ctx.file_path) {
            if sym.name == target && (self.kind_compatible)(edge_kind, &sym.kind) {
                return Some(self.resolution(sym.id, "default_same_file"));
            }
        }
        None
    }

    /// Strategy 12 — scope-chain walk.
    ///
    /// Try `{scope}.{target}` for each scope in `ref_ctx.scope_chain`
    /// (innermost first). Catches local symbols defined in the source
    /// symbol's own enclosing type / namespace / module before we widen
    /// to file or project scope.
    pub fn resolve_via_scope_visible(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        for scope in &self.ref_ctx.scope_chain {
            let qname = format!("{scope}.{target}");
            if let Some(sym) = self.lookup.by_qualified_name(&qname) {
                if (self.kind_compatible)(edge_kind, &sym.kind) {
                    return Some(self.resolution(sym.id, "default_scope_visible"));
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
    pub fn resolve_via_self_keyword(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let enclosing = self.enclosing_type()?;
        match target {
            "this" | "self" | "Self" => (self.kind_compatible)(edge_kind, &enclosing.kind)
                .then(|| self.resolution(enclosing.id, "engine_self_keyword")),
            "super" | "base" => {
                let parent_qname = self.lookup.parent_class_qname(&enclosing.qualified_name)?;
                let parent = self.lookup.by_qualified_name(parent_qname)?;
                (self.kind_compatible)(edge_kind, &parent.kind)
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
    pub fn resolve_via_enclosing_member(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() || target.contains('.') || target.contains("::") {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let mut current = self.enclosing_type()?.qualified_name.clone();
        for _ in 0..MAX_INHERITANCE_DEPTH {
            for member in self.lookup.members_of(&current) {
                if member.name == target && (self.kind_compatible)(edge_kind, &member.kind) {
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
    pub fn resolve_via_aliased_import(&self) -> Option<Resolution> {
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
                if (self.kind_compatible)(edge_kind, &sym.kind)
                    && file_path_matches_module(&sym.file_path, &rewritten)
                {
                    return Some(self.resolution(sym.id, "engine_aliased_import"));
                }
            }
        }
        None
    }

    /// Run every strategy in canonical order, returning the first hit.
    ///
    /// Canonical order, most-specific evidence first:
    ///   1.  scope chain  — innermost enclosing scope wins
    ///   2.  same file    — sibling symbol in the source file
    ///   3.  self keyword — `this`/`self`/`super` against the enclosing type
    ///   4.  enclosing member — inherited member of the enclosing type
    ///   5.  r.module     — extractor-set module prefix
    ///   6.  qname exact  — dotted target matches a stored qname
    ///   7.  chain prefix — second-to-last chain segment matches an import
    ///   8.  reexport chain — import points at a re-exporting package
    ///   9.  file import  — bare target matches an imported name
    ///   10. aliased import — import specifier rewritten through a path alias
    ///   11. namespace import — dotted import expanded against the target
    ///   12. ambient namespace path — qname ends with the dotted target
    ///   13. same namespace — file's declared namespace + target
    ///   14. imported namespace — candidate qname is prefixed by an import
    ///   15. ambient package — candidate lives in a declared ambient pkg
    ///   16. wildcard import — bare target under a wildcard import's namespace
    ///   17. generic param — bare target matches a declared generic parameter
    ///
    /// Every strategy binds through scope or import structure. There is no
    /// global `by_name` search in this ladder — a name binds through that
    /// structure or stays unresolved. The by-name methods
    /// `resolve_via_unique_internal_name` and `resolve_via_ranked_candidates`
    /// are not part of the default binder.
    ///
    /// Language hooks that want a different order or additional language-
    /// specific strategies call individual methods themselves.
    pub fn resolve_all(&self) -> Option<Resolution> {
        let result = self.resolve_via_scope_visible()
            .or_else(|| self.resolve_via_same_file())
            .or_else(|| self.resolve_via_self_keyword())
            .or_else(|| self.resolve_via_enclosing_member())
            .or_else(|| self.resolve_via_ref_module())
            .or_else(|| self.resolve_via_qname_exact())
            .or_else(|| self.resolve_via_chain_prefix())
            .or_else(|| self.resolve_via_reexport_chain())
            .or_else(|| self.resolve_via_file_import())
            .or_else(|| self.resolve_via_aliased_import())
            .or_else(|| self.resolve_via_namespace_import())
            .or_else(|| self.resolve_via_ambient_namespace_path())
            .or_else(|| self.resolve_via_same_namespace())
            .or_else(|| self.resolve_via_imported_namespace())
            .or_else(|| self.resolve_via_ambient_package())
            .or_else(|| self.resolve_via_wildcard_import())
            .or_else(|| self.resolve_via_generic_param());
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
            },
        );
    }

    /// Strategy — bare target brought into scope by a wildcard / static
    /// wildcard import.
    ///
    /// `import static org.junit.jupiter.api.Assertions.*;` (Java),
    /// `use foo::*;` (Rust), `from utils import *` (Python),
    /// `using namespace std;` (C++) bring every exported member of the
    /// imported namespace into bare-name scope. When a candidate's
    /// qualified name lives directly under any wildcard import's
    /// module_path, resolve to it. Returns None when no wildcard import
    /// matches OR when multiple candidates from different wildcards tie.
    pub fn resolve_via_wildcard_import(&self) -> Option<Resolution> {
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
        let mut hits: Vec<&SymbolInfo> = Vec::new();
        for sym in self.lookup.by_name(target) {
            if !(self.kind_compatible)(edge_kind, &sym.kind) {
                continue;
            }
            for ns in &wildcards {
                if qname_directly_under(&sym.qualified_name, ns) {
                    hits.push(sym);
                    break;
                }
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
    pub fn resolve_via_ranked_candidates(&self) -> Option<Resolution> {
        let target = self.ref_ctx.extracted_ref.target_name.as_str();
        if target.is_empty() || target.contains('.') || target.contains("::") || target.contains('/') {
            return None;
        }
        let edge_kind = self.ref_ctx.extracted_ref.kind;
        let candidates: Vec<&SymbolInfo> = self
            .lookup
            .by_name(target)
            .iter()
            .filter(|sym| (self.kind_compatible)(edge_kind, &sym.kind))
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

/// Handles relative TS specifiers (`./catalog` → `src/catalog.ts`),
/// dotted namespaces (C# `System.Linq` → file in namespace `Linq`), and
/// last-segment fallback (file path contains the rightmost module
/// segment).
/// True for `./foo`, `../shared`, etc. — the kinds of specifiers that
/// resolve inside the project and don't cross a package boundary.
fn is_relative_specifier(spec: &str) -> bool {
    spec.starts_with("./") || spec.starts_with("../")
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
    let last_segment = module.rsplit('.').next().unwrap_or(module);
    normalized.contains(last_segment)
}

#[cfg(test)]
#[path = "default_resolver_tests.rs"]
mod tests;
