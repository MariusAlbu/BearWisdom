// =============================================================================
// languages/php/hooks.rs — PhpHooks impl plus the concrete PhpResolver (chain
// via PhpChecker, synthetic-global preference for php_stubs / SPL / bundled
// extensions, `$this->` / `this.` stripping with PHP-namespace
// normalization, scope-chain walk, same-namespace, use-statement
// resolution, fully-qualified-name with `\\` ↔ `.` normalization, global
// bare-name lookup for `route()` / `trans()` / `auth()` / `view()` /
// `config()`, inheritance-via-`$this` walk with depth-10 cycle guard,
// `.php`/`.phtml`/`.phpt` bare-name fallback for autoloaded globals and
// PHPUnit assertions) plus 7 flow detectors (Eloquent static + Doctrine
// entity-manager DB queries, Laravel Mail / Notification facade,
// Laravel Bus / Queue / MessageBus dispatch, Ratchet WS interfaces,
// Symfony #[Route] attribute Consumer) plus external classifier with
// composer.json package match (vendor + package segments) and namespace-
// negative structural fallback plus build_file_context with backslash
// normalization.
// =============================================================================

pub(crate) use super::predicates::normalize_php_ns;

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    intern_yield_type, ChainMiss, FileContext, ImportEntry, RefContext, Resolution, SymbolInfo,
    SymbolLookup,
};
use crate::type_checker::chain::{external_type_qname, simple_yield_type};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};

pub struct PhpResolver;

impl PhpResolver {
    pub(crate) fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }

    pub(crate) fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;


        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = walk_php_chain(chain_val, edge_kind, file_ctx, ref_ctx, lookup) {
                return Some(res);
            }
        }

        // Synthetic-global lookup. php_stubs emits real symbols for PHP
        // core, SPL, and bundled extensions (str_*, array_*, json_*,
        // DateTime, Exception hierarchy, …).
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

        let effective_target = target
            .strip_prefix("$this->")
            .or_else(|| target.strip_prefix("this."))
            .unwrap_or(target);

        let normalized_target = predicates::normalize_php_ns(effective_target);
        let effective_target = normalized_target.as_str();

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

        // Global bare-name lookup for PHP helper functions like `route()`,
        // `trans()`, `auth()`, `view()`, `config()` — declared at global
        // scope, indexed from vendor/laravel/framework/.../helpers.php.
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

        // Inheritance-chain walk for `$this->method()` calls.
        let is_this_call = {
            use crate::types::SegmentKind;
            let via_chain = ref_ctx
                .extracted_ref
                .chain
                .as_ref()
                .and_then(|c| c.segments.first())
                .map(|s| s.kind == SegmentKind::SelfRef)
                .unwrap_or(false);
            via_chain || target.starts_with("$this->") || target.starts_with("this.")
        };
        if edge_kind == EdgeKind::Calls
            && is_this_call
            && !effective_target.contains('.')
        {
            let calling_class = ref_ctx
                .scope_chain
                .first()
                .map(|s| s.as_str());

            if let Some(mut class_qname) = calling_class {
                // Depth-10 cycle guard for malformed source.
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

        None
    }

    pub(crate) fn is_visible(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _target: &SymbolInfo,
    ) -> bool {
        // Navigation tool: visibility never gates resolution, so go-to-definition
        // reaches private members. Deliberate divergence from compiler behavior.
        true
    }
}

// composer-manifest match + hardcoded `is_external_php_namespace` set +
// structural fallback via `lookup.has_in_namespace` for PHP runtime classes
// (Closure/Throwable) and vendor packages without a stub on disk.
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

/// PHP chain walker.
pub(crate) fn walk_php_chain(
    chain_ref: &MemberChain,
    edge_kind: EdgeKind,
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain_ref.segments;
    if segments.len() < 2 {
        return None;
    }

    // Phase 1: root type.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => find_enclosing_class(&ref_ctx.scope_chain, lookup),
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            if let Some(local_type) = lookup.local_type(name) {
                Some(local_type)
            } else {
                let is_type = lookup.types_by_name(name).iter().any(|s| {
                    matches!(
                        s.kind.as_str(),
                        "class" | "interface" | "enum" | "type_alias"
                    )
                });
                if is_type {
                    Some(name.clone())
                } else {
                    let mut found = None;
                    for scope in &ref_ctx.scope_chain {
                        let field_qname = format!("{scope}.{name}");
                        if let Some(type_name) = lookup.field_type_str(&field_qname) {
                            found = Some(type_name.to_string());
                            break;
                        }
                    }
                    found.or_else(|| segments[0].declared_type.clone())
                }
            }
        }
        // PHP static call: `ClassName::method()` — segment is TypeAccess.
        SegmentKind::TypeAccess => {
            let name = &segments[0].name;
            let qualified = lookup
                .types_by_name(name)
                .iter()
                .find(|s| {
                    matches!(s.kind.as_str(), "class" | "interface" | "enum" | "type_alias")
                })
                .map(|s| s.qualified_name.clone())
                .unwrap_or_else(|| name.clone());
            Some(qualified)
        }
        _ => None,
    };

    let mut current_type = root_type?;

    // Phase 2: intermediate segments.
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(next_type) = lookup.field_type_str(&member_qname) {
            current_type = next_type.to_string();
            continue;
        }
        if let Some(next_type) = lookup.return_type_str(&member_qname) {
            current_type = next_type.to_string();
            continue;
        }

        let mut found = false;
        for import in &file_ctx.imports {
            if let Some(module) = &import.module_path {
                let qualified_member = format!("{module}.{member_qname}");
                if let Some(next_type) = lookup.field_type_str(&qualified_member) {
                    current_type = next_type.to_string();
                    found = true;
                    break;
                }
                if let Some(next_type) = lookup.return_type_str(&qualified_member) {
                    current_type = next_type.to_string();
                    found = true;
                    break;
                }
            }
        }
        if found {
            continue;
        }

        if let Some(ext_qname) = external_type_qname(&current_type, lookup) {
            let ext_member = format!("{ext_qname}.{}", seg.name);
            if let Some(next_type) = lookup.field_type_str(&ext_member) {
                current_type = next_type.to_string();
                continue;
            }
            if let Some(next_type) = lookup.return_type_str(&ext_member) {
                current_type = next_type.to_string();
                continue;
            }
            current_type = ext_qname;
            continue;
        }

        let miss_type = external_type_qname(&current_type, lookup)
            .unwrap_or_else(|| current_type.clone());
        lookup.record_chain_miss(ChainMiss {
            current_type: miss_type,
            target_name: seg.name.clone(),
        });
        return None;
    }

    // Phase 3: final segment.
    let last = &segments[segments.len() - 1];
    let effective_type = external_type_qname(&current_type, lookup)
        .unwrap_or_else(|| current_type.clone());
    let candidate = format!("{effective_type}.{}", last.name);

    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if predicates::kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "php_chain_resolution",
                resolved_yield_type: intern_yield_type(simple_yield_type(sym, lookup), lookup),
                flow_emit: None,
            });
        }
    }

    for import in &file_ctx.imports {
        if let Some(module) = &import.module_path {
            let ns_candidate = format!("{module}.{candidate}");
            if let Some(sym) = lookup.by_qualified_name(&ns_candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "php_chain_resolution",
                        resolved_yield_type: intern_yield_type(simple_yield_type(sym, lookup), lookup),
                        flow_emit: None,
                    });
                }
            }
        }
    }

    for sym in lookup.members_of(&effective_type) {
        if sym.name == last.name && predicates::kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "php_chain_resolution",
                resolved_yield_type: intern_yield_type(simple_yield_type(sym, lookup), lookup),
                flow_emit: None,
            });
        }
    }

    // Inheritance walk for Eloquent-style `__callStatic` forwarding.
    let mut cls = effective_type.as_str();
    for _ in 0..10 {
        match lookup.parent_class_qname(cls) {
            None => break,
            Some(parent) => {
                let parent_candidate = format!("{parent}.{}", last.name);
                if let Some(sym) = lookup.by_qualified_name(&parent_candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.85,
                            strategy: "php_chain_inherited",
                            resolved_yield_type: intern_yield_type(
                                simple_yield_type(sym, lookup),
                                lookup,
                            ),
                            flow_emit: None,
                        });
                    }
                }
                for sym in lookup.members_of(parent) {
                    if sym.name == last.name && predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.80,
                            strategy: "php_chain_inherited",
                            resolved_yield_type: intern_yield_type(
                                simple_yield_type(sym, lookup),
                                lookup,
                            ),
                            flow_emit: None,
                        });
                    }
                }
                cls = parent;
            }
        }
    }

    lookup.record_chain_miss(ChainMiss {
        current_type: effective_type,
        target_name: last.name.clone(),
    });
    None
}

/// Find the enclosing class/interface from the scope chain.
fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}

pub(crate) fn infer_external_inner(
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

// composer.json `"vendor/package"` packages map to PHP namespace roots like
// "Intervention" (vendor) or "Image" (package). Match against either
// segment, hyphen-stripped, lowercase-normalized.
fn is_composer_package_match(
    ns_root: &str,
    deps: &std::collections::HashSet<String>,
) -> bool {
    let ns_lower = ns_root.to_lowercase();
    for dep in deps {
        let (vendor, package) = if let Some(slash) = dep.find('/') {
            (&dep[..slash], &dep[slash + 1..])
        } else {
            (dep.as_str(), dep.as_str())
        };
        let vendor_lower = vendor.to_lowercase().replace('-', "");
        let package_lower = package.to_lowercase().replace('-', "");
        let ns_lower_nohyphen = ns_lower.replace('-', "");
        if vendor_lower == ns_lower_nohyphen || package_lower == ns_lower_nohyphen {
            return true;
        }
    }
    false
}

// Eloquent static `<Model>::<op>(...)` + Doctrine entity-manager `find` /
// `findBy` / `findOneBy` / `persist` / `flush`. Entity name from a Pascal-
// Case TypeAccess root (Eloquent) or the first `Ident`-shaped CallArg (the
// `::class` constant on Doctrine).
pub(crate) fn detect_php_db_query_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::{CallArg, SegmentKind};

    let segs = &chain.segments;
    let leaf = segs.last()?.name.as_str();

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
    }

    let root_seg = segs.first()?;
    if root_seg.kind != SegmentKind::TypeAccess {
        return None;
    }
    let root = root_seg.name.as_str();
    if !is_pascal_case_first_php(root) {
        return None;
    }
    // Ignore facade/utility statics — `Route`, `Auth`, `Log`, `Cache` etc.
    // are call-site facades, not Eloquent models.
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
        "create" | "createMany" | "make" | "insert" | "insertGetId" | "insertOrIgnore"
        | "forceCreate" => DbQueryOp::Insert,
        "update" | "updateOrCreate" | "save" | "push" | "touch" | "increment"
        | "decrement" | "fill" | "forceFill" => DbQueryOp::Update,
        "firstOrCreate" | "firstOrNew" | "upsert" => DbQueryOp::Upsert,
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

// Common Laravel facades that look like static-call entry points but are
// not Eloquent models.
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

// Laravel `Mail::to($user)->send(new WelcomeEmail)` / Notification facade.
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

// Laravel `Bus::dispatch(new Job(...))` / `Queue::push(...)` / MessageBus.
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

// Ratchet WS server: `implements MessageComponentInterface` /
// `WampServerInterface`. Emit Consumer WebSocket.
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

// Symfony `#[Route('/api/users', methods: ['GET'])]`. The decorator extractor
// captures attr name as target_name and first string arg (URL) as module.
// HTTP method isn't preserved by the decorator extractor today — record Any.
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

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;

    if r.kind == EdgeKind::TypeRef {
        if let Some(emission) = detect_symfony_route_attribute_emission(
            r.target_name.as_str(),
            r.module.as_deref(),
        ) {
            return vec![emission];
        }
        return Vec::new();
    }

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

    let file_namespace = file.symbols.iter().find_map(|sym| {
        if sym.kind == crate::types::SymbolKind::Namespace {
            Some(sym.qualified_name.clone())
        } else {
            None
        }
    });

    // PHP extractor emits:
    //   use App\Models\User;       → target_name = "User",  module = "App\Models\User"
    //   use App\Models\User as U;  → target_name = "U",     module = "App\Models\User"
    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }
        let module = r.module.as_deref().unwrap_or(&r.target_name);

        // Normalize backslash → dot for index lookup consistency.
        let normalized_module = predicates::normalize_php_ns(module);

        imports.push(ImportEntry {
            imported_name: r.target_name.clone(),
            module_path: Some(normalized_module),
            alias: None,
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

pub struct PhpHooks;

impl LanguageEngineHooks for PhpHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let _ = lookup;
        detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(build_file_context_inner(file, project_ctx))
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if let Some(res) = PhpResolver.resolve(file_ctx, ref_ctx, lookup) {
            return Some(res);
        }
        (crate::type_checker::core::DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all()
    }
}

pub static PHP_HOOKS: PhpHooks = PhpHooks;
