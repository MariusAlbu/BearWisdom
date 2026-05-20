// =============================================================================
// indexer/resolve/rules/php/mod.rs — PHP resolution rules
//
// Scope rules for PHP (7.4+, 8.x):
//
//   1. Chain-aware resolution: walk MemberChain following field/return types.
//   2. Scope chain walk: innermost scope → outermost, try {scope}.{target}
//   3. Same-namespace resolution: types in the same namespace are visible
//      without `use` (mirrors C# same-namespace visibility).
//   4. Use statement resolution: `use App\Models\User;` makes `User` visible.
//   5. Fully qualified names: backslash-separated names resolve directly
//      (normalized to dotted form in the index).
//
// PHP import model:
//   The PHP extractor emits EdgeKind::Imports refs for `use` declarations:
//     use App\Models\User;         → target_name = "User",  module = "App\Models\User"
//     use App\Models\User as U;    → target_name = "U",     module = "App\Models\User"
//
//   PHP namespaces use backslash as separator. The index normalizes these
//   to dotted form (e.g., "App\Models\User" → "App.Models.User") to be
//   consistent with the rest of the resolvers. We accept both forms in lookups.
//
// Adding new PHP features:
//   - Trait use → add to build_file_context (already EdgeKind::Imports in extractor).
//   - Enum backed types → extractor emits TypeRef; scope chain handles them.
// =============================================================================


// Re-export for test visibility (php_tests.rs uses `use super::*`).
pub(crate) use super::predicates::normalize_php_ns;

use super::{predicates, type_checker::PhpChecker};
use crate::type_checker::TypeChecker;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// PHP language resolver.
pub struct PhpResolver;

impl LanguageResolver for PhpResolver {
    fn language_ids(&self) -> &[&str] {
        &["php"]
    }

    
    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }
fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Skip import refs themselves — they're not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Chain-aware resolution: if we have a structured MemberChain, walk it
        // step-by-step following field types. Chain context (receiver type +
        // inheritance walk) is more accurate than bare-name lookup, so it
        // runs before the walker fallback below.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = PhpChecker.resolve_chain(
                chain_val, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        // Bare-name walker lookup for refs with no chain context. php_stubs
        // emits real symbols for the PHP core, SPL, and bundled extensions
        // (str_*, array_*, json_*, DateTime, Exception hierarchy, ...).
        // ext:-only filter so namespace / scope / same-file paths still win
        // for project symbols.
        if !target.contains('\\') && !target.contains('.') {
            for sym in lookup.by_name(target) {
                if !sym.file_path.starts_with("ext:") {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "php_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Normalize: strip `$this->` or `this.` prefix for member access.
        let effective_target = target
            .strip_prefix("$this->")
            .or_else(|| target.strip_prefix("this."))
            .unwrap_or(target);

        // Also normalize any backslash separators in the target itself.
        let normalized_target = predicates::normalize_php_ns(effective_target);
        let effective_target = normalized_target.as_str();

        // Step 1: Scope chain walk (innermost → outermost).
        // e.g., scope_chain = ["App.Controllers.UserController.store",
        //                       "App.Controllers.UserController",
        //                       "App.Controllers"]
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "php_scope_chain",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 2: Same-namespace resolution.
        // In PHP, classes in the same namespace are visible without `use`.
        if let Some(ns) = &file_ctx.file_namespace {
            let candidate = format!("{ns}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "php_same_namespace",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 3: Use statement resolution.
        // `use App\Models\User;` → target "User" resolves to "App.Models.User"
        for import in &file_ctx.imports {
            if import.imported_name == effective_target {
                if let Some(module) = &import.module_path {
                    if let Some(sym) = lookup.by_qualified_name(module) {
                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "php_use_statement",
                                resolved_yield_type: None,
                                flow_emit: None,
                            });
                        }
                    }
                }
            }
        }

        // Step 4: Fully qualified name (target contains "\" or ".").
        if effective_target.contains('.') || effective_target.contains('\\') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "php_qualified_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 5: Global bare-name lookup for PHP global helper functions.
        //
        // PHP helper functions like `route()`, `trans()`, `auth()`, `view()`,
        // `config()` are declared at global scope — no namespace, so their
        // qualified name in the index is just their simple name (e.g. `route`).
        // These functions are called without `use` statements anywhere in the
        // project, so Steps 1-4 all miss them. We look them up directly via
        // `by_qualified_name(bare_name)` which finds external symbols indexed
        // from vendor/laravel/framework/src/Illuminate/Foundation/helpers.php
        // and similar global-helper files.
        //
        // Only triggers for Calls edges on simple (no-dot) names to avoid
        // matching class method names (those always carry a scope prefix).
        if edge_kind == EdgeKind::Calls && !effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if sym.kind == "function" {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.9,
                        strategy: "php_global_function",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 6: Inheritance-chain walk for `$this->method()` calls.
        //
        // When a method call on `$this` could not be resolved by the scope
        // chain (Step 1), the method is likely defined on a parent class.
        // We walk the `inherits_map` upward from the calling class, trying
        // `{ancestor}.{method_name}` at each level (depth ≤ 10 to guard
        // against inheritance cycles in malformed source).
        //
        // Only fires for:
        //   - EdgeKind::Calls on a simple (no-dot) name
        //   - the call is on `$this` — detected via the chain's first segment
        //     being SelfRef, OR the original target had a `$this->` prefix
        //   - the scope chain has at least one class-level entry
        let is_this_call = {
            use crate::types::SegmentKind;
            // Check chain for SelfRef first segment (the normal PHP `$this->method()` pattern).
            let via_chain = ref_ctx
                .extracted_ref
                .chain
                .as_ref()
                .and_then(|c| c.segments.first())
                .map(|s| s.kind == SegmentKind::SelfRef)
                .unwrap_or(false);
            // Fallback: target still has the `$this->` prefix (emitted by some code paths).
            via_chain || target.starts_with("$this->") || target.starts_with("this.")
        };
        if edge_kind == EdgeKind::Calls
            && is_this_call
            && !effective_target.contains('.')
        {
            // The calling class is the first entry in the scope chain —
            // scope_chain is built from the source symbol's scope_path, which
            // is the enclosing class qname (e.g. "App.Services.SetupAccount").
            // scope_chain[0] is thus the class; scope_chain[1] is the namespace.
            let calling_class = ref_ctx
                .scope_chain
                .first()
                .map(|s| s.as_str());

            if let Some(mut class_qname) = calling_class {
                // Walk up at most 10 ancestors so cycles don't spin forever.
                for _ in 0..10 {
                    match lookup.parent_class_qname(class_qname) {
                        None => break,
                        Some(parent_qname) => {
                            let candidate = format!("{parent_qname}.{effective_target}");
                            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                                if self.is_visible(file_ctx, ref_ctx, sym)
                                    && predicates::kind_compatible(edge_kind, &sym.kind)
                                {
                                    return Some(Resolution {
                                        target_symbol_id: sym.id,
                                        confidence: 0.85,
                                        strategy: "php_inherited_method",
                                        resolved_yield_type: None,
                                        flow_emit: None,
                                    });
                                }
                            }
                            class_qname = parent_qname;
                        }
                    }
                }
            }
        }

        // PHP bare-name fallback. 7th language using the
        // `<lang>_bare_name` template (PRs 31, 35, 36, 37, 38, 39).
        // PHP's namespace resolution + autoloader registers global
        // classes (Closure, RuntimeException, ArrayAccess, …) and
        // PHPUnit assertion methods (`assertEquals`, `assertSame`,
        // `assertFileContains`) as ambient runtime symbols. Engine's
        // module/import/scope path can't bind them without a `use`
        // statement.
        //
        // Gated by `.php`/`.phtml`/`.phpt` file-extension and
        // `kind_compatible` + `is_visible`. PHP visibility is enforced
        // through is_visible to keep private/protected cross-file refs
        // unbound.
        let edge_kind = ref_ctx.extracted_ref.kind;
        let target = &ref_ctx.extracted_ref.target_name;
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains('\\')
            && !target.contains("::")
        {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_php = path.ends_with(".php")
                    || path.ends_with(".phtml")
                    || path.ends_with(".phpt")
                    || path.starts_with("ext:php:");
                if !is_php {
                    continue;
                }
                if !self.is_visible(file_ctx, ref_ctx, sym) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "php_bare_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Could not resolve deterministically — fall back to heuristic.
        None
    }

    fn is_visible(
        &self,
        file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        target: &SymbolInfo,
    ) -> bool {
        let vis = target.visibility.as_deref().unwrap_or("public");
        match vis {
            "public" => true,
            "protected" => {
                // Accessible from same class or subclasses — approximate: allow.
                true
            }
            "private" => {
                // Only visible within the same file (same class).
                &*target.file_path == file_ctx.file_path
            }
            _ => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Test whether `normalized` should be classified as external. Combines the
/// composer-manifest match, the hardcoded `is_external_php_namespace` set,
/// and a structural fallback that uses `lookup.has_in_namespace` to flag
/// any imported namespace the project's own symbols don't cover (PHP runtime
/// classes like `Closure`/`Throwable`, vendor packages without a stub on
/// disk, etc.).
fn ns_is_external(
    project_ctx: Option<&ProjectContext>,
    pkg_id: Option<i64>,
    lookup: Option<&dyn SymbolLookup>,
    normalized: &str,
) -> bool {
    let ns_root = normalized.split('.').next().unwrap_or(normalized);
    if let Some(ctx) = project_ctx {
        if let Some(manifest) = ctx
            .manifests_for(pkg_id)
            .get(&ManifestKind::Composer)
        {
            if is_composer_package_match(ns_root, &manifest.dependencies) {
                return true;
            }
        }
    }
    if predicates::is_external_php_namespace(normalized, project_ctx) {
        return true;
    }
    if let Some(lookup) = lookup {
        if !lookup.has_in_namespace(normalized) && !lookup.has_in_namespace(ns_root) {
            return true;
        }
    }
    false
}

pub(super) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: Option<&dyn SymbolLookup>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;
    let pkg_id = ref_ctx.file_package_id;

    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let import_path = ref_ctx
            .extracted_ref
            .module
            .as_deref()
            .unwrap_or(target);
        let normalized = predicates::normalize_php_ns(import_path);
        if ns_is_external(project_ctx, pkg_id, lookup, &normalized) {
            return Some(normalized);
        }
        return None;
    }

    let mut best: Option<String> = None;
    for import in &file_ctx.imports {
        let ns = import.module_path.as_deref().unwrap_or("");
        if ns.is_empty() {
            continue;
        }
        if ns_is_external(project_ctx, pkg_id, lookup, ns) {
            if best.as_deref().is_none() || ns.len() > best.as_deref().unwrap().len() {
                best = Some(ns.to_string());
            }
        }
    }
    best
}

/// Check whether a PHP namespace root matches any composer.json package dependency.
///
/// Composer packages use `"vendor/package"` format (e.g., `"intervention/image"`).
/// PHP namespace roots are CamelCase (e.g., `"Intervention"`).
///
/// Matching strategy:
/// 1. Exact case-insensitive match of the namespace root against the package part
///    (after the `/`): `"Intervention"` matches `"intervention/image"` package part `"image"` — no.
///    Actually match against the last segment after `/`: vendor/package → package.
/// 2. Also try matching against the vendor segment before `/`.
///
/// For well-known mappings like `laravel/framework` → `Illuminate`, the
/// ALWAYS_EXTERNAL list in builtins handles them. This function catches packages
/// not in that list where the namespace root matches the composer package name.
fn is_composer_package_match(
    ns_root: &str,
    deps: &std::collections::HashSet<String>,
) -> bool {
    let ns_lower = ns_root.to_lowercase();
    for dep in deps {
        // `"vendor/package"` — check both vendor and package segments.
        let (vendor, package) = if let Some(slash) = dep.find('/') {
            (&dep[..slash], &dep[slash + 1..])
        } else {
            (dep.as_str(), dep.as_str())
        };
        // Normalize: replace hyphens with nothing for comparison (e.g., "my-package" → "mypackage").
        let vendor_lower = vendor.to_lowercase().replace('-', "");
        let package_lower = package.to_lowercase().replace('-', "");
        let ns_lower_nohyphen = ns_lower.replace('-', "");
        if vendor_lower == ns_lower_nohyphen || package_lower == ns_lower_nohyphen {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// DbQuery detection — Eloquent (Laravel) + Doctrine ORM
// ---------------------------------------------------------------------------

/// Recognise Eloquent and Doctrine query chains and emit a DbQuery keyed
/// on the discovered entity name and operation.
///
/// Shapes handled:
/// - **Eloquent static**: `<Model>::<op>(...)` where `Model` is PascalCase
///   and `op` is a recognised query/mutation method (`where`, `find`,
///   `findOrFail`, `first`, `create`, `update`, `delete`, `destroy`, …).
///   Chained queries fire on the leaf:
///   `User::where(...)->orderBy(...)->get()` emits with `entity_name=User`
///   and the leaf op.
/// - **Eloquent instance**: `$model-><op>(...)` cannot be tied to a model
///   without type info — skipped.
/// - **Doctrine entity-manager**: `$em->find(SomeEntity::class, $id)` /
///   `$em->getRepository(SomeEntity::class)->findBy(...)`. The entity name
///   comes from the first `Ident`-shaped argument (the `::class`
///   constant). `findBy`, `find`, `findOneBy`, `findAll`, `count`,
///   `persist`, `remove`, `flush` are recognised verbs.
pub(crate) fn detect_php_db_query_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::{CallArg, SegmentKind};

    let segs = &chain.segments;
    let leaf = segs.last()?.name.as_str();

    // Doctrine flush — entity unknown.
    if leaf == "flush" {
        // Only fire when the chain root is a $-style identifier (entity manager
        // variable), not arbitrary `Foo->flush()` calls.
        if let Some(first) = segs.first() {
            if matches!(first.kind, SegmentKind::Identifier) {
                return Some(FlowEmission::DbQuery {
                    entity_name: "php.*".to_string(),
                    operation: DbQueryOp::Other,
                });
            }
        }
        return None;
    }

    // Doctrine repository/entity-manager methods. Entity name comes from the
    // first `Ident` arg (the `::class` constant captured by extract_call_args).
    if let Some(op) = parse_doctrine_op(leaf) {
        let entity = call_args.iter().find_map(|a| match a {
            CallArg::Ident(s) if is_pascal_case_first_php(s) => Some(s.clone()),
            _ => None,
        });
        if let Some(entity) = entity {
            return Some(FlowEmission::DbQuery {
                entity_name: format!("php.{}", entity),
                operation: op,
            });
        }
        // Doctrine repository chain: `getRepository(User::class)->findBy(...)`.
        // The leaf is `findBy`; the entity was captured in a *previous* segment's
        // call_args, not the current one. We don't have that here, so fall
        // through to the static-Eloquent path below for any chain whose root is
        // PascalCase.
    }

    // Eloquent static chain: `<Model>::<op>(...)` — chain root is a
    // PascalCase TypeAccess segment.
    let root_seg = segs.first()?;
    if root_seg.kind != SegmentKind::TypeAccess {
        return None;
    }
    let root = root_seg.name.as_str();
    if !is_pascal_case_first_php(root) {
        return None;
    }
    // Ignore facade/utility statics — `Route`, `Auth`, `Log`, `Cache`, etc.
    // are call-site facades, not Eloquent models. Conservative deny-list:
    // accept any PascalCase root that isn't a known facade. Resolution is
    // a hint, not a hard guarantee — pairing on the entity name covers
    // mistakes by simply not matching anything.
    if is_php_facade(root) {
        return None;
    }

    let op = parse_eloquent_op(leaf)?;
    Some(FlowEmission::DbQuery {
        entity_name: format!("php.{}", root),
        operation: op,
    })
}

fn parse_eloquent_op(name: &str) -> Option<crate::indexer::resolve::flow_emit::DbQueryOp> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    Some(match name {
        // Read operations.
        "where" | "whereIn" | "whereNotIn" | "whereNull" | "whereNotNull"
        | "whereBetween" | "whereDate" | "whereTime" | "whereYear" | "whereMonth"
        | "whereDay" | "whereColumn" | "whereExists" | "whereHas" | "whereDoesntHave"
        | "orWhere" | "orWhereIn" | "orWhereNull" | "orWhereNotNull" | "orWhereHas"
        | "find" | "findOrFail" | "findOrNew" | "findMany"
        | "first" | "firstOrFail" | "firstOr" | "firstWhere" | "sole"
        | "get" | "all" | "pluck" | "value" | "count" | "exists" | "doesntExist"
        | "min" | "max" | "sum" | "avg" | "average"
        | "with" | "withCount" | "withTrashed" | "without" | "withoutGlobalScopes"
        | "has" | "doesntHave" | "select" | "selectRaw" | "distinct"
        | "orderBy" | "orderByDesc" | "orderByRaw" | "latest" | "oldest" | "inRandomOrder"
        | "groupBy" | "groupByRaw" | "having" | "havingRaw"
        | "limit" | "take" | "skip" | "offset" | "paginate" | "simplePaginate"
        | "cursorPaginate" | "chunk" | "chunkById" | "lazy" | "lazyById"
        | "join" | "leftJoin" | "rightJoin" | "crossJoin"
        | "scope" | "newQuery" | "query" | "toBase" | "toSql"
        | "pluckArr" | "values" | "keys" => DbQueryOp::Select,
        // Insert operations.
        "create" | "createMany" | "make" | "insert" | "insertGetId" | "insertOrIgnore"
        | "forceCreate" => DbQueryOp::Insert,
        // Update operations.
        "update" | "updateOrCreate" | "save" | "push" | "touch" | "increment"
        | "decrement" | "fill" | "forceFill" => DbQueryOp::Update,
        // Upsert.
        "firstOrCreate" | "firstOrNew" | "upsert" => DbQueryOp::Upsert,
        // Delete operations.
        "delete" | "destroy" | "forceDelete" | "truncate" | "restore" => DbQueryOp::Delete,
        _ => return None,
    })
}

fn parse_doctrine_op(name: &str) -> Option<crate::indexer::resolve::flow_emit::DbQueryOp> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    Some(match name {
        "find" | "findBy" | "findOneBy" | "findAll" | "count" => DbQueryOp::Select,
        "persist" => DbQueryOp::Insert,
        "merge" => DbQueryOp::Update,
        "remove" => DbQueryOp::Delete,
        _ => return None,
    })
}

fn is_pascal_case_first_php(name: &str) -> bool {
    name.chars().next().map_or(false, |c| c.is_ascii_uppercase())
}

/// Common Laravel facades that look like static-call entry points but are
/// not Eloquent models. Static calls on these are call-site helpers
/// rather than DB queries — skip them.
fn is_php_facade(name: &str) -> bool {
    matches!(
        name,
        "Route" | "Auth" | "Log" | "Cache" | "Config" | "DB" | "Schema" | "Storage"
            | "Mail" | "Queue" | "Event" | "Hash" | "Session" | "Cookie" | "Crypt"
            | "Lang" | "Notification" | "URL" | "Validator" | "View" | "Request"
            | "Response" | "Redirect" | "App" | "Artisan" | "Bus" | "Broadcast"
            | "Date" | "File" | "Gate" | "Http" | "Lottery" | "Password" | "Pipeline"
            | "Process" | "RateLimiter" | "Reminder" | "Sleep" | "Vite"
            | "Carbon" | "Str" | "Arr" | "Collection" | "Builder"
            | "self" | "static" | "parent"
    )
}

// ---------------------------------------------------------------------------
// Laravel Mail / Symfony Mailer Producer
// ---------------------------------------------------------------------------

/// Detect Laravel `Mail::to($user)->send(new WelcomeEmail)` style.
/// Chain root is "Mail" facade.
pub(crate) fn detect_php_mailer_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !matches!(root, "Mail" | "Notification") {
        return None;
    }
    if !matches!(leaf, "send" | "queue" | "later" | "raw" | "html") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::Mailer,
        name: format!("php.{}", root),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// Laravel Bus / Queue Producer
// ---------------------------------------------------------------------------

/// Detect Laravel `Bus::dispatch(new Job(...))`, `Queue::push(...)` style.
pub(crate) fn detect_php_bgjob_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !matches!(root, "Bus" | "Queue" | "MessageBus") {
        return None;
    }
    if !matches!(
        leaf,
        "dispatch" | "dispatchSync" | "dispatchNow" | "push" | "later" | "bulk"
    ) {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: format!("php.{}", root),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

/// Ratchet WS server class — `implements MessageComponentInterface` /
/// `WampServerInterface`. Emit Consumer WebSocket so the handler clusters
/// with the WS edges.
pub(crate) fn detect_php_ratchet_emission(
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let last = target.rsplit('\\').next().unwrap_or(target);
    if !matches!(
        last,
        "MessageComponentInterface"
            | "WampServerInterface"
            | "WsServerInterface"
            | "MessageInterface"
    ) {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: "php.ratchet".to_string(),
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// Symfony Route attribute → Consumer HttpCall
// ---------------------------------------------------------------------------

/// Emit a Consumer HttpCall flow emission for Symfony / `attributes` style
/// route declarations: `#[Route('/api/users', methods: ['GET'])]`. The
/// decorator extractor captures the attribute name as the ref's
/// `target_name` and the first string arg (the URL) as `module`. HTTP
/// method isn't preserved by the decorator extractor today so we record
/// `Any`; pairing matches against any-method consumers when the producer
/// side specifies a concrete verb.
pub(crate) fn detect_symfony_route_attribute_emission(
    attr_name: &str,
    first_arg: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    if attr_name != "Route" {
        return None;
    }
    let url = first_arg?;
    if url.is_empty() || !url.starts_with('/') {
        return None;
    }
    let name = crate::connectors::url_pattern::normalize(url);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Consumer,
        method: Some(HttpMethod::Any),
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------


pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;

    // Symfony `#[Route('/x', methods: ['GET'])]` on a controller method
    // lands as a TypeRef whose `target_name` is "Route" and `module` is
    // the first string arg (the URL). HTTP method isn't currently
    // captured by the decorator extractor — fall back to Any.
    if r.kind == EdgeKind::TypeRef {
        if let Some(emission) = detect_symfony_route_attribute_emission(
            r.target_name.as_str(),
            r.module.as_deref(),
        ) {
            return vec![emission];
        }
        return Vec::new();
    }

    // Ratchet `class XHandler implements MessageComponentInterface` —
    // also `Wamp` and `WampServerInterface`. Emit single-ended Consumer
    // WebSocket so the handler clusters with WS endpoints.
    if r.kind == EdgeKind::Implements || r.kind == EdgeKind::Inherits {
        if let Some(emission) = detect_php_ratchet_emission(r.target_name.as_str()) {
            return vec![emission];
        }
        return Vec::new();
    }

    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let Some(chain_ref) = r.chain.as_ref() else {
        return Vec::new();
    };

    if let Some(emission) = detect_php_db_query_emission(chain_ref, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_php_mailer_emission(chain_ref) {
        return vec![emission];
    }
    if let Some(emission) = detect_php_bgjob_emission(chain_ref) {
        return vec![emission];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    // Extract the current namespace from the first Namespace symbol.
    let file_namespace = file.symbols.iter().find_map(|sym| {
        if sym.kind == crate::types::SymbolKind::Namespace {
            Some(sym.qualified_name.clone())
        } else {
            None
        }
    });

    // Extract `use` declarations from EdgeKind::Imports refs.
    // PHP extractor emits:
    //   use App\Models\User;       → target_name = "User",  module = "App\Models\User"
    //   use App\Models\User as U;  → target_name = "U",     module = "App\Models\User"
    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }
        let module = r.module.as_deref().unwrap_or(&r.target_name);

        // Normalize backslash separators to dots for index lookup consistency.
        let normalized_module = predicates::normalize_php_ns(module);

        imports.push(ImportEntry {
            imported_name: r.target_name.clone(),
            module_path: Some(normalized_module),
            alias: None,
            // PHP `use` is always an exact type import, not a wildcard.
            is_wildcard: false,
        });
    }

    FileContext {
        file_path: file.path.clone(),
        language: "php".to_string(),
        imports,
        file_namespace,
    }
}
