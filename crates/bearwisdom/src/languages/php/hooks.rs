// =============================================================================
// languages/php/hooks.rs — PhpHooks impl of LanguageEngineHooks. Carries 7 flow
// detectors (Eloquent static + Doctrine entity-manager DB queries, Laravel
// Mail / Notification facade, Laravel Bus / Queue / MessageBus dispatch,
// Ratchet WS interfaces, Symfony #[Route] attribute Consumer), the external
// classifier with composer.json package match (vendor + package segments) and
// namespace-negative structural fallback, and build_file_context with
// backslash normalization. Resolution runs through the generic engine; the
// chain walker qualifies bare receivers per
// `ChainQualification::SamePackageAndImports`.
// =============================================================================

pub(crate) use super::predicates::normalize_php_ns;

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

// composer-manifest match (`is_external_php_namespace` consults composer.json
// only) + structural fallback via `lookup.has_in_namespace` for PHP runtime
// classes (Closure/Throwable) and vendor packages without a stub on disk.
fn ns_is_external(
    project_ctx: Option<&ProjectContext>,
    pkg_id: Option<i64>,
    lookup: Option<&dyn SymbolLookup>,
    normalized: &str,
) -> bool {
    let ns_root = normalized.split('.').next().unwrap_or(normalized);
    if let Some(ctx) = project_ctx {
        if let Some(manifest) = ctx.manifests_for(pkg_id).get(&ManifestKind::Composer) {
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

pub(crate) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: Option<&dyn SymbolLookup>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;
    let pkg_id = ref_ctx.file_package_id;

    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
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
fn is_composer_package_match(ns_root: &str, deps: &std::collections::HashSet<String>) -> bool {
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
        "where"
        | "whereIn"
        | "whereNotIn"
        | "whereNull"
        | "whereNotNull"
        | "whereBetween"
        | "whereDate"
        | "whereTime"
        | "whereYear"
        | "whereMonth"
        | "whereDay"
        | "whereColumn"
        | "whereExists"
        | "whereHas"
        | "whereDoesntHave"
        | "orWhere"
        | "orWhereIn"
        | "orWhereNull"
        | "orWhereNotNull"
        | "orWhereHas"
        | "find"
        | "findOrFail"
        | "findOrNew"
        | "findMany"
        | "first"
        | "firstOrFail"
        | "firstOr"
        | "firstWhere"
        | "sole"
        | "get"
        | "all"
        | "pluck"
        | "value"
        | "count"
        | "exists"
        | "doesntExist"
        | "min"
        | "max"
        | "sum"
        | "avg"
        | "average"
        | "with"
        | "withCount"
        | "withTrashed"
        | "without"
        | "withoutGlobalScopes"
        | "has"
        | "doesntHave"
        | "select"
        | "selectRaw"
        | "distinct"
        | "orderBy"
        | "orderByDesc"
        | "orderByRaw"
        | "latest"
        | "oldest"
        | "inRandomOrder"
        | "groupBy"
        | "groupByRaw"
        | "having"
        | "havingRaw"
        | "limit"
        | "take"
        | "skip"
        | "offset"
        | "paginate"
        | "simplePaginate"
        | "cursorPaginate"
        | "chunk"
        | "chunkById"
        | "lazy"
        | "lazyById"
        | "join"
        | "leftJoin"
        | "rightJoin"
        | "crossJoin"
        | "scope"
        | "newQuery"
        | "query"
        | "toBase"
        | "toSql"
        | "pluckArr"
        | "values"
        | "keys" => DbQueryOp::Select,
        "create" | "createMany" | "make" | "insert" | "insertGetId" | "insertOrIgnore"
        | "forceCreate" => DbQueryOp::Insert,
        "update" | "updateOrCreate" | "save" | "push" | "touch" | "increment" | "decrement"
        | "fill" | "forceFill" => DbQueryOp::Update,
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
    name.chars()
        .next()
        .map_or(false, |c| c.is_ascii_uppercase())
}

// Common Laravel facades that look like static-call entry points but are
// not Eloquent models.
fn is_php_facade(name: &str) -> bool {
    matches!(
        name,
        "Route"
            | "Auth"
            | "Log"
            | "Cache"
            | "Config"
            | "DB"
            | "Schema"
            | "Storage"
            | "Mail"
            | "Queue"
            | "Event"
            | "Hash"
            | "Session"
            | "Cookie"
            | "Crypt"
            | "Lang"
            | "Notification"
            | "URL"
            | "Validator"
            | "View"
            | "Request"
            | "Response"
            | "Redirect"
            | "App"
            | "Artisan"
            | "Bus"
            | "Broadcast"
            | "Date"
            | "File"
            | "Gate"
            | "Http"
            | "Lottery"
            | "Password"
            | "Pipeline"
            | "Process"
            | "RateLimiter"
            | "Reminder"
            | "Sleep"
            | "Vite"
            | "Carbon"
            | "Str"
            | "Arr"
            | "Collection"
            | "Builder"
            | "self"
            | "static"
            | "parent"
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
        if let Some(emission) =
            detect_symfony_route_attribute_emission(r.target_name.as_str(), r.module.as_deref())
        {
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
}

pub static PHP_HOOKS: PhpHooks = PhpHooks;
