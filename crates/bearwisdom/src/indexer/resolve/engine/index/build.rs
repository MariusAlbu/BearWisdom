// =============================================================================
// indexer/resolve/engine/index/build.rs — initial SymbolIndex construction
//
// `build_with_context` is the master constructor: it walks every `ParsedFile`
// and populates every field of SymbolIndex in a fixed sequence of passes
// (basic indexes → type metadata → re-exports → module resolution →
// inheritance → alias targets → workspace + tsconfig → ambient globals).
// One file because the passes share a lot of context; the per-pass
// boundaries are still visible via inline comments.
//
// Augmentation (later adding files / DB symbols to an already-built index)
// lives in `augment.rs`.
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rustc_hash::FxHashMap;
use std::collections::BTreeMap;

use crate::indexer::module_resolution::ModuleResolver as _;
use crate::types::{
    AliasTarget, EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind, Visibility,
};

use super::super::{
    file_belongs_to_npm_package, infer_type_from_chain, npm_package_from_external_path,
    npm_package_from_specifier, parse_return_type_from_signature, resolve_type_name_in_scope,
};
use super::{
    common_prefix_len, find_matching_bracket, is_ambient_global_lib_path, is_type_like_kind,
};
use super::SymbolIndex;
use crate::indexer::resolve::engine::{ChainMiss, ImportEntry, SymbolInfo, TypeInfo};

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
        let mut qname_duplicates: FxHashMap<String, Vec<SymbolInfo>> =
            FxHashMap::default();
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
                    signature: sym.signature.clone(),
                };

                // Simple name index
                let simple = sym.name.clone();
                by_name.entry(simple).or_default().push(info.clone());

                // Qualified name index (first wins for duplicates).
                // When a duplicate arrives, copy the existing winner into
                // `qname_duplicates` (if not already there) and append the
                // new one — `qname_duplicates` stores the FULL set of
                // symbols sharing that qname, in insertion order. The
                // kind-compatible scan reads from there to find the right
                // overload when TypeScript declaration merging makes the
                // first-wins race pick the wrong kind (e.g.
                // `@angular/core.Injectable` exists as both interface and
                // variable — Calls refs need the variable).
                match by_qname.entry(sym.qualified_name.clone()) {
                    std::collections::btree_map::Entry::Vacant(e) => {
                        e.insert(info.clone());
                    }
                    std::collections::btree_map::Entry::Occupied(occ) => {
                        let entry = qname_duplicates
                            .entry(sym.qualified_name.clone())
                            .or_insert_with(|| vec![occ.get().clone()]);
                        entry.push(info.clone());
                    }
                }

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
                        let resolved = resolve_type_name_in_scope(
                            type_refs[0],
                            sym.scope_path.as_deref(),
                            &by_qname,
                        );
                        field_type.insert(sym.qualified_name.clone(), resolved);
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
                        let resolved = resolve_type_name_in_scope(
                            type_refs[0],
                            sym.scope_path.as_deref(),
                            &by_qname,
                        );
                        field_type.insert(sym.qualified_name.clone(), resolved);
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
                            // Scope-resolve the raw type name so chain walking
                            // works across namespace/class boundaries.
                            // `class Dayjs { clone(): Dayjs }` inside
                            // `namespace dayjs` emits a TypeRef with
                            // target_name="Dayjs" — the raw text in the
                            // source. The chain walker needs the fully
                            // qualified "dayjs.Dayjs" to traverse the
                            // symbol graph. We probe candidate FQNs built
                            // from the method's scope_path (innermost
                            // scope first, walking outward) and store the
                            // first one that matches a known qualified
                            // name. Without this, every `.d.ts` file's
                            // namespaced interface chain is invisible and
                            // we end up needing per-library synthetics to
                            // hand-qualify the return types — exactly
                            // what `dayjs_synthetics`, `js_test_chains`,
                            // and friends do.
                            let resolved = resolve_type_name_in_scope(
                                last,
                                sym.scope_path.as_deref(),
                                &by_qname,
                            );
                            return_type
                                .insert(sym.qualified_name.clone(), resolved);
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
                                    let resolved = resolve_type_name_in_scope(
                                        &rt,
                                        sym.scope_path.as_deref(),
                                        &by_qname,
                                    );
                                    return_type.insert(sym.qualified_name.clone(), resolved);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Class symbols are callable — `Foo()` returns an instance of `Foo`.
            // Populate return_type = qualified_name unconditionally so the chain
            // walker can follow `x = Foo(); x.method()`. This pass is separate
            // from the TypeRef loop above because class symbols have no outgoing
            // TypeRefs of their own.
            for sym in &pf.symbols {
                if sym.kind == SymbolKind::Class {
                    return_type
                        .entry(sym.qualified_name.clone())
                        .or_insert_with(|| sym.qualified_name.clone());
                }
            }

            // Build generic_params: for class/interface/struct symbols that have
            // type_parameters in their signature (e.g., `interface Repository<T>`,
            // `def process[F[_]]`, `fn compute<T: Clone>`).
            // We detect this by looking at the symbol's signature text.
            // Includes Namespace because Rust impl blocks emit a Namespace
            // symbol whose signature carries the generic params (`impl Foo<T>`),
            // and methods inside the impl reference those params by simple name.
            for sym in &pf.symbols {
                if !matches!(
                    sym.kind,
                    SymbolKind::Class
                        | SymbolKind::Interface
                        | SymbolKind::Trait
                        | SymbolKind::Struct
                        | SymbolKind::TypeAlias
                        | SymbolKind::Function
                        | SymbolKind::Method
                        | SymbolKind::Namespace
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
                if !matches!(child_sym.kind, SymbolKind::Class | SymbolKind::Interface | SymbolKind::Trait) {
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

        // Build alias_target map. Two sources, in priority order:
        //   1. Structurally-classified shapes from per-file `alias_targets`
        //      (TS extractor populates these — Application / Union /
        //      Intersection / Object / Other).
        //   2. A derived `Application` shape for any TypeAlias symbol the
        //      explicit map didn't cover. Sources: typedefs in C/C++,
        //      Dart/F#/Erlang/Bicep type abbreviations, and TS aliases that
        //      didn't make it through (e.g. parsed under demand filtering
        //      that skipped the symbol body). The fallback uses the
        //      `field_type` already populated by the TypeAlias arm above.
        let mut alias_target_map: FxHashMap<String, AliasTarget> = FxHashMap::default();
        for pf in parsed {
            for (qname, target) in &pf.alias_targets {
                alias_target_map.insert(qname.clone(), target.clone());
                // Mirror by simple name so chain walkers can resolve aliases
                // regardless of whether the encountered current_type is
                // namespaced or bare.
                if let Some(simple) = qname.rsplit('.').next() {
                    if simple != qname {
                        alias_target_map
                            .entry(simple.to_string())
                            .or_insert_with(|| target.clone());
                    }
                }
            }
        }
        // Fallback for languages that don't emit alias_targets explicitly:
        // synthesize Application{root, args} from the type_info maps the
        // TypeAlias arm above already populated. type_args may carry the
        // generic args when the typedef points at a parameterized type.
        for pf in parsed {
            for sym in &pf.symbols {
                if sym.kind != SymbolKind::TypeAlias {
                    continue;
                }
                if alias_target_map.contains_key(&sym.qualified_name) {
                    continue;
                }
                let Some(ti) = type_info.get(&sym.qualified_name) else {
                    continue;
                };
                let Some(root) = ti.field_type.clone() else {
                    continue;
                };
                let target = AliasTarget::Application {
                    root,
                    args: ti.type_args.clone(),
                };
                alias_target_map.insert(sym.qualified_name.clone(), target.clone());
                if sym.name != sym.qualified_name {
                    alias_target_map
                        .entry(sym.name.clone())
                        .or_insert(target);
                }
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

        // Aggregate re-exports by the npm package they belong to. Only
        // bare cross-package re-exports matter (relative ones stay
        // intra-package and are followed via the per-file map). The
        // resulting map answers: "package P forwards which names to which
        // packages?", which the chain resolver walks at lookup time.
        let mut pkg_reexports: FxHashMap<String, Vec<(String, String)>> = FxHashMap::default();
        for (file_path, entries) in &reexport_map {
            let Some(pkg) = npm_package_from_external_path(file_path) else {
                continue;
            };
            for (name, target_module) in entries {
                if target_module.starts_with("./") || target_module.starts_with("../") {
                    continue;
                }
                if target_module.is_empty() {
                    continue;
                }
                let Some(target_pkg) = npm_package_from_specifier(target_module) else {
                    continue;
                };
                if target_pkg == pkg {
                    continue;
                }
                pkg_reexports
                    .entry(pkg.clone())
                    .or_default()
                    .push((name.clone(), target_pkg));
            }
        }
        // Dedup each package's entries — many d.ts files re-export the same
        // names; storing duplicates blows up walk cost without changing
        // outcomes.
        for v in pkg_reexports.values_mut() {
            v.sort();
            v.dedup();
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
        //
        // Performance: the hot path for TypeScript monorepos was an O(N × refs)
        // linear scan inside `NodeModuleResolver::try_resolve` — each unique
        // (source_file, relative_specifier) pair scanned all N file paths with
        // 18 extension probes. On ts-nextjs (~83,000 files, ~100k+ unique pairs)
        // this was ~150 billion comparisons (~22 minutes of wall-clock time).
        //
        // Fix: build a `FilePathIndex` once, pass it to `resolve_to_file_indexed`
        // which uses O(1) hash lookups. The suffix map is keyed by every trailing
        // segment sequence, so `"components/Button.tsx"` resolves in O(1)
        // regardless of project size.
        let go_module_path = project_ctx
            .and_then(|ctx| ctx.manifest(crate::ecosystem::manifest::ManifestKind::GoMod))
            .and_then(|m| m.module_path.as_deref());
        let resolvers =
            crate::indexer::module_resolution::all_resolvers_with_go_module(go_module_path);
        let file_paths: Vec<&str> = parsed.iter().map(|pf| pf.path.as_str()).collect();
        // Pre-build the O(1) path index. Construction is O(N × depth) where
        // depth is the average segment count per path (4-8). Amortised over
        // all subsequent lookups (potentially millions), this is near-free.
        let file_path_index = crate::indexer::module_resolution::FilePathIndex::build(&file_paths);
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
                        resolver.resolve_to_file_indexed(module, &pf.path, &file_path_index)
                    {
                        module_to_file_per_source.insert(key, resolved);
                    }
                    continue;
                }
                if module_to_file.contains_key(module.as_str()) {
                    continue;
                }
                if let Some(resolved) =
                    resolver.resolve_to_file_indexed(module, &pf.path, &file_path_index)
                {
                    module_to_file.insert(module.clone(), resolved);
                }
            }
        }

        // Build the ambient-global method-name set. Any method/property
        // declared in a file whose path looks like a TypeScript ambient
        // declaration (`typescript/lib/lib.*.d.ts`, `@types/node/*`) goes
        // in. Chain walkers hit this when a receiver can't be typed but
        // the called name is a known runtime API — the honest answer is
        // "external (DOM/ES runtime)", not "unresolved".
        let mut ambient_global_method_names: HashSet<String> = HashSet::new();
        for pf in parsed {
            if !is_ambient_global_lib_path(&pf.path) {
                continue;
            }
            for sym in &pf.symbols {
                if matches!(sym.kind, SymbolKind::Method | SymbolKind::Property | SymbolKind::Function) {
                    ambient_global_method_names.insert(sym.name.clone());
                }
            }
        }

        // Build test-framework globals from manifest dependencies.
        // Build per-language primitive sets for all languages present in parsed files.
        let mut primitives_by_language: FxHashMap<String, HashSet<&'static str>> =
            FxHashMap::default();
        for pf in parsed {
            if !primitives_by_language.contains_key(&pf.language) {
                let set = crate::indexer::keywords::keywords_set_for_language(&pf.language);
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
        let mut tsconfig_types_union: Vec<String> = Vec::new();
        if let Some(ctx) = project_ctx {
            if let Some(npm) = ctx.manifest(crate::ecosystem::manifest::ManifestKind::Npm) {
                tsconfig_paths_union = npm.tsconfig_paths.clone();
                tsconfig_types_union = npm.tsconfig_types.clone();
            }
            for (&pkg_id, manifests) in &ctx.by_package {
                if let Some(npm) = manifests.get(&crate::ecosystem::manifest::ManifestKind::Npm) {
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

        // Build the Angular selector map: raw selector → class qualified name.
        // Source: `ParsedFile::component_selectors` populated by the full-index
        // pipeline for TypeScript/Angular files with `@Component({selector:...})`.
        let mut angular_selectors: FxHashMap<String, String> = FxHashMap::default();
        for pf in parsed {
            for (selector, class_qname) in &pf.component_selectors {
                if !selector.is_empty() && !class_qname.is_empty() {
                    angular_selectors.insert(selector.clone(), class_qname.clone());
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
            pkg_reexports,
            module_to_file,
            module_to_file_per_source,
            primitives_by_language,
            by_package,
            workspace_pkg_by_declared_name,
            tsconfig_paths_by_pkg,
            tsconfig_paths_union,
            tsconfig_types_union,
            inherits_map,
            alias_target: alias_target_map,
            qname_duplicates,
            ambient_global_method_names,
            external_paths: HashSet::new(),
            angular_selectors,
            empty: Vec::new(),
            empty_reexports: Vec::new(),
            chain_misses: std::sync::Mutex::new(Vec::new()),
        }
    }
}
