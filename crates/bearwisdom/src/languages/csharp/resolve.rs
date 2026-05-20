// =============================================================================
// indexer/resolve/rules/csharp/mod.rs — C# resolution rules
//
// Scope rules for C# (all versions through C# 13):
//
//   1. Scope chain walk: innermost scope → outermost, try {scope}.{target}
//   2. Same-namespace: types in the same namespace are visible without `using`
//   3. Using directives: `using Namespace;` makes all public types visible
//   4. Fully qualified: dotted names resolve directly
//   5. Visibility: public/internal/protected/private enforcement
//
// Adding new C# features:
//   - New syntax that introduces scope (e.g., file-scoped namespaces) →
//     update the extractor in parser/extractors/csharp.rs to emit the
//     correct scope_path, then this resolver handles it automatically.
//   - New import forms (e.g., global using) → add to build_file_context.
// =============================================================================


use super::{predicates, type_checker::CSharpChecker};
use crate::type_checker::TypeChecker;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::inheritance;
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// C# language resolver.
///
/// When the C# profile has `engine_primary` set, chain-bearing refs route
/// through `type_checker::Engine::resolve` before reaching this resolver.
/// This impl handles bare-name refs (using directives, same-namespace,
/// qualified names) and any chain refs the engine declines.
pub struct CSharpResolver;

impl LanguageResolver for CSharpResolver {
    fn language_ids(&self) -> &[&str] {
        &["csharp", "vbnet"]
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

        // Chain-aware resolution: dispatch to CSharpChecker.
        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = CSharpChecker.resolve_chain(
                chain_ref, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        // Normalize: strip `this.` prefix for member access on the current class.
        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        // Step 1: Scope chain walk (innermost → outermost).
        // e.g., scope_chain = ["NS.Cls.Method", "NS.Cls", "NS"]
        // Try "NS.Cls.Method.Target", "NS.Cls.Target", "NS.Target"
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_scope_chain",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 2: Same-namespace resolution.
        // In C#, types in the same namespace are visible without a `using` directive.
        if let Some(ns) = &file_ctx.file_namespace {
            let candidate = format!("{ns}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_same_namespace",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 3: Using directive resolution.
        // `using eShop.Catalog.API.Model;` → try "eShop.Catalog.API.Model.{target}"
        for import in &file_ctx.imports {
            if import.is_wildcard {
                if let Some(module) = &import.module_path {
                    let candidate = format!("{module}.{effective_target}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if self.is_visible(file_ctx, ref_ctx, sym)
                            && predicates::kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "csharp_using_directive",
                                resolved_yield_type: None,
                                flow_emit: None,
                            });
                        }
                    }
                }
            }
        }

        // Step 4: Fully qualified name (target contains dots).
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_qualified_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 5: Field type chain resolution.
        // For `db.SelectFrom` (after stripping `this.`), follow the field's type annotation.
        if effective_target.contains('.') {
            if let Some(dot) = effective_target.find('.') {
                let field_name = &effective_target[..dot];
                let rest = &effective_target[dot + 1..];

                for scope in &ref_ctx.scope_chain {
                    let field_qname = format!("{scope}.{field_name}");
                    if let Some(type_name) = lookup.field_type_str(&field_qname) {
                        let candidate = format!("{type_name}.{rest}");
                        if let Some(sym) = lookup.by_qualified_name(&candidate) {
                            if predicates::kind_compatible(edge_kind, &sym.kind) {
                                return Some(Resolution {
                                    target_symbol_id: sym.id,
                                    confidence: 0.95,
                                    strategy: "csharp_field_type_chain",
                                    resolved_yield_type: None,
                                    flow_emit: None,
                                });
                            }
                        }
                        // Try using directives: {namespace}.{TypeName}.{rest}
                        for import in &file_ctx.imports {
                            if import.is_wildcard {
                                if let Some(module) = &import.module_path {
                                    let candidate = format!("{module}.{type_name}.{rest}");
                                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                                            return Some(Resolution {
                                                target_symbol_id: sym.id,
                                                confidence: 0.90,
                                                strategy: "csharp_field_type_chain",
                                                resolved_yield_type: None,
                                                flow_emit: None,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        // Step 6: Inheritance-chain walk for implicit `this` calls.
        //
        // C# allows bare method calls inside a class body — `MyMethod()` is
        // implicitly `this.MyMethod()` and may target a protected/public method
        // on a base class.  When Steps 1–5 all miss, walk the inherits_map
        // upward from the enclosing class (scope_chain[1]) trying
        // `{ancestor}.{target}`.
        //
        // Only fires for Calls edges on simple (no-dot) names inside a class.
        if edge_kind == EdgeKind::Calls && !effective_target.contains('.') {
            if let Some(calling_class) =
                inheritance::enclosing_class_from_scope(&ref_ctx.scope_chain)
            {
                if let Some(res) = inheritance::resolve_via_inheritance(
                    calling_class,
                    effective_target,
                    edge_kind,
                    file_ctx,
                    ref_ctx,
                    lookup,
                    predicates::kind_compatible,
                    |fc, rc, sym| self.is_visible(fc, rc, sym),
                    "csharp_inherited_method",
                ) {
                    return Some(res);
                }
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
            "internal" => {
                // Approximate: visible if in the same project (same top-level directory).
                // For a proper check we'd need assembly information.
                true
            }
            "protected" => {
                // Approximate: visible if in the same class hierarchy.
                // Full check would require walking the inheritance chain.
                true
            }
            "private" => {
                // Private: only visible within the same file.
                &*target.file_path == file_ctx.file_path
            }
            _ => true,
        }
    }

}

/// Hangfire: `BackgroundJob.Enqueue(...)`, `RecurringJob.AddOrUpdate(...)`.
pub(crate) fn detect_csharp_hangfire_bg_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !matches!(root, "BackgroundJob" | "RecurringJob" | "BatchJob") {
        return None;
    }
    if !matches!(leaf, "Enqueue" | "Schedule" | "AddOrUpdate" | "ContinueWith") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: format!("cs.{}", root),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

/// HotChocolate GraphQL attribute markers. Annotates class/method as
/// GraphQL Query / Mutation / Subscription roots. Emits Consumer
/// GraphQLOp keyed on the marker so the architecture overview clusters
/// HotChocolate types alongside other GraphQL emissions.
pub(crate) fn detect_csharp_hotchocolate_emission(
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let kind = match target {
        "QueryType" | "Query" | "ExtendObjectType" => "query",
        "MutationType" | "Mutation" => "mutation",
        "SubscriptionType" | "Subscription" => "subscription",
        _ => return None,
    };
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::GraphQLOp,
        name: format!("cs.hotchocolate.{}", kind),
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

/// Match `: Hub`, `: Hub<TClient>`, `: DynamicHub` — SignalR base classes.
/// Emits Consumer WebSocket so the hub clusters as a WS endpoint.
pub(crate) fn detect_csharp_signalr_hub_emission(
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let base = target.split('<').next().unwrap_or(target).trim();
    if !matches!(base, "Hub" | "DynamicHub") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: "cs.signalr".to_string(),
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

pub(crate) fn detect_csharp_mailer_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    // SmtpClient / MailKit SmtpClient / SendGrid Client.
    if !matches!(
        root,
        "smtp" | "smtpClient" | "_smtpClient" | "_emailService" | "emailService" | "mailService" | "_mailService" | "_sendGridClient" | "sendGridClient"
    ) {
        return None;
    }
    if !matches!(
        leaf,
        "Send" | "SendAsync" | "SendMailAsync" | "SendEmailAsync"
    ) {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::Mailer,
        name: format!("cs.{}", root),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// DbQuery detection — EF Core + Dapper
// ---------------------------------------------------------------------------

/// Recognise EF Core LINQ chains and Dapper extension calls. Emits a
/// `DbQuery` keyed on the discovered entity name and operation.
///
/// Shapes handled:
/// - **EF Core LINQ**: `<context>.<EntitySet>.<op>(...)` where
///   `EntitySet` is PascalCase and `op` is one of the recognised
///   query / mutation methods (`Where`, `FirstOrDefault`, `ToListAsync`,
///   `Add`, `Update`, `Remove`, etc.).
/// - **EF Core SaveChanges**: `<context>.SaveChanges` / `SaveChangesAsync`
///   emits a single DbQuery with a `*` entity name (commits all queued
///   work — entity is unknown statically).
/// - **EF Core Set<T>()**: chain whose leaf is `Set` (generic method) is
///   too type-info-dependent to recover the entity without type-arg
///   capture; skipped.
/// - **Dapper**: `<connection>.Query` / `QueryAsync` / `QueryFirstOrDefault`
///   / `QuerySingle` / `Execute` / `ExecuteAsync` / `ExecuteScalar`. SQL
///   text parsed from the first string-literal arg for `FROM <table>` /
///   `UPDATE <table>` / `INSERT INTO <table>` / `DELETE FROM <table>`
///   clauses.
pub(crate) fn detect_csharp_db_query_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    let segs = &chain.segments;
    let leaf = segs.last()?.name.as_str();

    // EF Core SaveChanges — entity unknown.
    if matches!(leaf, "SaveChanges" | "SaveChangesAsync") {
        return Some(FlowEmission::DbQuery {
            entity_name: "cs.*".to_string(),
            operation: DbQueryOp::Other,
        });
    }

    // Dapper extensions. First positional arg is the SQL string.
    if let Some(op) = parse_dapper_verb(leaf) {
        let sql = match call_args.first()? {
            CallArg::StringLit(s) | CallArg::TemplateLit(s) => s.clone(),
            _ => return None,
        };
        if let Some((entity, sql_op)) = parse_csharp_sql_entity(&sql) {
            return Some(FlowEmission::DbQuery {
                entity_name: format!("cs.{}", entity),
                operation: match (op, sql_op) {
                    (DbQueryOp::Other, sql) => sql,
                    (forced, _) => forced,
                },
            });
        }
        return None;
    }

    // EF Core LINQ chain: `<context>.<EntitySet>.<op>(...)` — 3+ segments
    // with a PascalCase middle (DbSet property name) and a recognised
    // LINQ-ish op leaf.
    if segs.len() >= 3 {
        let entity = segs[segs.len() - 2].name.as_str();
        if !is_pascal_case_first_cs(entity) {
            return None;
        }
        if let Some(op) = parse_efcore_linq_op(leaf) {
            return Some(FlowEmission::DbQuery {
                entity_name: format!("cs.{}", entity),
                operation: op,
            });
        }
    }

    None
}

fn parse_dapper_verb(name: &str) -> Option<crate::indexer::resolve::flow_emit::DbQueryOp> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    Some(match name {
        "Query" | "QueryAsync" | "QueryFirstOrDefault" | "QueryFirstOrDefaultAsync"
        | "QuerySingle" | "QuerySingleAsync" | "QuerySingleOrDefault"
        | "QuerySingleOrDefaultAsync" | "QueryMultiple" | "QueryMultipleAsync" => {
            DbQueryOp::Select
        }
        "Execute" | "ExecuteAsync" | "ExecuteScalar" | "ExecuteScalarAsync" => DbQueryOp::Other,
        _ => return None,
    })
}

fn parse_efcore_linq_op(name: &str) -> Option<crate::indexer::resolve::flow_emit::DbQueryOp> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    Some(match name {
        // Read operations.
        "Where" | "FirstOrDefault" | "FirstOrDefaultAsync" | "First" | "FirstAsync"
        | "Single" | "SingleAsync" | "SingleOrDefault" | "SingleOrDefaultAsync"
        | "ToList" | "ToListAsync" | "ToArray" | "ToArrayAsync" | "Find" | "FindAsync"
        | "Count" | "CountAsync" | "LongCount" | "LongCountAsync" | "Any" | "AnyAsync"
        | "All" | "AllAsync" | "Sum" | "SumAsync" | "Min" | "MinAsync" | "Max" | "MaxAsync"
        | "Average" | "AverageAsync" | "Contains" | "ContainsAsync" | "Include"
        | "ThenInclude" | "OrderBy" | "OrderByDescending" | "GroupBy" | "Select"
        | "AsNoTracking" | "AsTracking" | "ToDictionary" | "ToDictionaryAsync"
        | "ToHashSet" | "ToHashSetAsync" => DbQueryOp::Select,
        // Write operations on DbSet.
        "Add" | "AddAsync" | "AddRange" | "AddRangeAsync" => DbQueryOp::Insert,
        "Update" | "UpdateRange" => DbQueryOp::Update,
        "Remove" | "RemoveRange" => DbQueryOp::Delete,
        "Attach" | "AttachRange" => DbQueryOp::Other,
        _ => return None,
    })
}

fn is_pascal_case_first_cs(name: &str) -> bool {
    name.chars().next().map_or(false, |c| c.is_ascii_uppercase())
}

/// Parse a SQL string to identify the entity (table) name and operation.
/// Returns `(table_name, op)` when a recognised clause is found.
fn parse_csharp_sql_entity(
    sql: &str,
) -> Option<(String, crate::indexer::resolve::flow_emit::DbQueryOp)> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    let upper = sql.trim().to_ascii_uppercase();
    let (op, after) = if let Some(i) = upper.find("INSERT INTO ") {
        (DbQueryOp::Insert, i + 12)
    } else if let Some(i) = upper.find("UPDATE ") {
        (DbQueryOp::Update, i + 7)
    } else if let Some(i) = upper.find("DELETE FROM ") {
        (DbQueryOp::Delete, i + 12)
    } else if let Some(i) = upper.find(" FROM ") {
        (DbQueryOp::Select, i + 6)
    } else if upper.starts_with("FROM ") {
        (DbQueryOp::Select, 5)
    } else {
        return None;
    };
    let slice = sql.get(after..)?;
    let token = slice.split_whitespace().next()?;
    let entity: String = token
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .to_string();
    if entity.is_empty() {
        return None;
    }
    // Drop schema qualifier (`dbo.Users` → `Users`).
    let final_entity = entity.rsplit('.').next().unwrap_or(entity.as_str()).to_string();
    Some((final_entity, op))
}

// ---------------------------------------------------------------------------
// HTTP Producer detection — HttpClient + RestSharp chain-call shapes
// ---------------------------------------------------------------------------

/// Recognise `<httpClient>.GetAsync("/x")` / `PostAsync` / `PutAsync` /
/// `DeleteAsync` / `PatchAsync` / `SendAsync` chain calls (HttpClient) and
/// `<restClient>.ExecuteAsync` / `<restClient>.<Verb>Async` /
/// `<restClient>.ExecuteAsync<T>` (RestSharp). Emits a Producer
/// `NamedChannel { kind: HttpCall, .. }` keyed on the first string-literal
/// argument, normalized via `url_pattern::normalize`.
///
/// The detector requires the chain leaf to be a recognised verb-suffixed
/// method AND the first argument to be a captured string/template literal
/// — calls whose URL is in a variable don't get a static pairing key.
pub(crate) fn detect_csharp_http_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    let leaf = chain.segments.last()?.name.as_str();
    let method = parse_csharp_http_verb(leaf)?;
    let url_raw = match call_args.first()? {
        CallArg::StringLit(s) | CallArg::TemplateLit(s) => s.clone(),
        _ => return None,
    };
    if url_raw.is_empty() {
        return None;
    }
    let name = crate::connectors::url_pattern::normalize(&url_raw);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Producer,
        method: Some(method),
    streaming: None,
    })
}

/// Parse the trailing HTTP-verb token from a HttpClient/RestSharp method
/// name. Returns `Any` for non-verb methods like `ExecuteAsync` (RestSharp
/// dispatches internally on the request's method, so the static call
/// shape doesn't carry a verb).
fn parse_csharp_http_verb(name: &str) -> Option<crate::indexer::resolve::flow_emit::HttpMethod> {
    use crate::indexer::resolve::flow_emit::HttpMethod;
    Some(match name {
        "GetAsync" | "GetStringAsync" | "GetByteArrayAsync" | "GetStreamAsync" => HttpMethod::Get,
        "PostAsync" | "PostAsJsonAsync" => HttpMethod::Post,
        "PutAsync" | "PutAsJsonAsync" => HttpMethod::Put,
        "PatchAsync" | "PatchAsJsonAsync" => HttpMethod::Patch,
        "DeleteAsync" | "DeleteFromJsonAsync" => HttpMethod::Delete,
        "SendAsync" | "ExecuteAsync" | "ExecuteGetAsync" | "ExecutePostAsync" => HttpMethod::Any,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Refit attribute emission — `[Get("/x")]` / `[Post("/x")]` on an interface method
// ---------------------------------------------------------------------------

/// Emit Producer HttpCall when an attribute ref's `target_name` is a Refit
/// HTTP-verb attribute. The decorator's first string arg (`r.module` in
/// the C# extractor's encoding) carries the route URL. Refit ships with
/// `[Get]`/`[Post]`/etc. attributes that look identical to ASP.NET's
/// `[HttpGet]` family except for the `Http` prefix — the prefix is the
/// disambiguator that lets us emit Producer for one and Consumer for the
/// other (the ExtractedRoute adapter handles the `[Http*]` Consumer side).
pub(crate) fn detect_refit_attribute_emission(
    attr_name: &str,
    first_arg: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    let method = match attr_name {
        "Get" => HttpMethod::Get,
        "Post" => HttpMethod::Post,
        "Put" => HttpMethod::Put,
        "Patch" => HttpMethod::Patch,
        "Delete" => HttpMethod::Delete,
        "Head" => HttpMethod::Head,
        "Options" => HttpMethod::Options,
        _ => return None,
    };
    let url = first_arg?;
    if url.is_empty() {
        return None;
    }
    let name = crate::connectors::url_pattern::normalize(url);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Producer,
        method: Some(method),
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Check whether a namespace refers to a sibling workspace project.
///
/// The `declared_name` for a .NET workspace package is the .csproj / .fsproj /
/// .vbproj filename stem (A2). MSBuild convention maps that stem to the
/// project's root namespace, so a `using Shared.Foo;` in a consumer project
/// targeting `Shared.csproj` resolves via `declared_name = "Shared"`.
///
/// Handles exact matches and nested namespaces: `Shared`, `Shared.Models`,
/// `Shared.Models.Users` all collapse to the `Shared` workspace package by
/// dot-walking from right to left. (`workspace_package_id` on ProjectContext
/// walks `/` separators for TypeScript-style deep imports — .NET namespaces
/// use `.` so we reimplement the walk locally.)
pub(super) fn matches_workspace_project(ctx: &ProjectContext, namespace: &str) -> bool {
    if ctx.workspace_pkg_by_declared_name.contains_key(namespace) {
        return true;
    }
    let mut path = namespace;
    while let Some(dot) = path.rfind('.') {
        path = &path[..dot];
        if ctx.workspace_pkg_by_declared_name.contains_key(path) {
            return true;
        }
    }
    false
}

/// Check whether a .NET namespace is external, using the NuGet manifest directly.
///
/// Always-external base prefixes (`System`, `Microsoft`) are checked first.
/// NuGet package names are then checked as namespace prefixes and via root-segment
/// matching (e.g., a "Newtonsoft.Json" dep makes any "Newtonsoft.*" namespace external).
pub(super) fn is_manifest_external_namespace(ctx: &ProjectContext, ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    if matches!(root, "System" | "Microsoft") {
        return true;
    }
    if let Some(m) = ctx.manifest(ManifestKind::NuGet) {
        if m.dependencies.contains(ns) {
            return true;
        }
        for dep in &m.dependencies {
            if ns.starts_with(dep.as_str())
                && ns.len() > dep.len()
                && ns.as_bytes()[dep.len()] == b'.'
            {
                return true;
            }
            if let Some(dep_root) = dep.split('.').next() {
                if root == dep_root {
                    return true;
                }
            }
        }
        return false;
    }
    false
}

// ---------------------------------------------------------------------------
// Integration event bus — `IntegrationEvent` base class +
// `IIntegrationEventHandler<T>` handler interface inheritance shapes.
// ---------------------------------------------------------------------------

/// Match a class whose base type is `IntegrationEvent`. Emits Producer
/// EventBus keyed on the event class name; the matching Consumer is
/// emitted by `detect_csharp_integration_event_handler_emission` when the
/// same `T` appears in `IIntegrationEventHandler<T>`.
pub(crate) fn detect_csharp_integration_event_emission(
    target: &str,
    source_symbol_name: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let base = target.split('<').next().unwrap_or(target).trim();
    if base != "IntegrationEvent" {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::EventBus,
        name: source_symbol_name.to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    })
}

/// Match a class whose base type is `IIntegrationEventHandler<T>`. Emits
/// Consumer EventBus keyed on `T`. The `T` is recovered (in order) from:
/// the target_name itself if the extractor preserved generics, the source
/// symbol's signature when present, otherwise the handler class name as a
/// last-resort placeholder.
pub(crate) fn detect_csharp_integration_event_handler_emission(
    target: &str,
    source_symbol_signature: Option<&str>,
    source_symbol_name: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let base = target.split('<').next().unwrap_or(target).trim();
    if base != "IIntegrationEventHandler" {
        return None;
    }
    let event_name = extract_event_handler_t(target)
        .or_else(|| source_symbol_signature.and_then(extract_event_handler_t))
        .unwrap_or_else(|| source_symbol_name.to_string());
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::EventBus,
        name: event_name,
        role: ChannelRole::Consumer,
        method: None,
        streaming: None,
    })
}

/// Extract `T` from a string containing `IIntegrationEventHandler<T>`.
fn extract_event_handler_t(s: &str) -> Option<String> {
    let start = s.find("IIntegrationEventHandler")?;
    let after = &s[start + "IIntegrationEventHandler".len()..];
    let lt = after.find('<')?;
    let gt = after.find('>')?;
    if gt <= lt {
        return None;
    }
    Some(after[lt + 1..gt].trim().to_string())
}

// ---------------------------------------------------------------------------
// .NET DI registration — `services.AddScoped<I, Impl>()` style.
// ---------------------------------------------------------------------------

/// Match `AddScoped` / `AddTransient` / `AddSingleton` chain leaves and
/// emit a `DiBinding` with `container = "dotnet"`. When the chain segment
/// carries generic `type_args`, the first arg becomes the service symbol
/// hint via name lookup at flow-write time; otherwise the binding is
/// emitted with `service_symbol_id = 0` (single-ended marker that still
/// clusters in the architecture overview).
pub(crate) fn detect_dotnet_di_chain_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let leaf = chain.segments.last()?;
    if !matches!(
        leaf.name.as_str(),
        "AddScoped" | "AddTransient" | "AddSingleton"
    ) {
        return None;
    }
    Some(FlowEmission::DiBinding {
        service_symbol_id: 0,
        container: Some("dotnet".to_string()),
    })
}

// ---------------------------------------------------------------------------
// Tests are in resolve_tests.rs, declared in mod.rs

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;

    // Refit interface methods: `[Get("/x")] Task<Foo> GetFoo();` lands
    // as a TypeRef attribute ref whose `target_name` is the HTTP verb
    // and `module` carries the route URL. Emit a Producer HttpCall
    // keyed on the normalized URL with `method` parsed from the
    // attribute name. The same `Get`/`Post`/… attribute names are
    // also used by ASP.NET (`[HttpGet]`) but those are
    // `[Http*]`-prefixed — the per-file ExtractedRoute adapter
    // handles those on the Consumer side.
    if r.kind == EdgeKind::TypeRef {
        if let Some(emission) = detect_refit_attribute_emission(
            r.target_name.as_str(),
            r.module.as_deref(),
        ) {
            return vec![emission];
        }
        // HotChocolate GraphQL: `[QueryType]`, `[MutationType]`,
        // `[Query]`, `[Mutation]`, `[Subscription]`, `[GraphQLName(...)]`.
        // Annotates a class or method as a GraphQL operation root.
        if let Some(emission) = detect_csharp_hotchocolate_emission(
            r.target_name.as_str(),
        ) {
            return vec![emission];
        }
        return Vec::new();
    }

    // SignalR Hub inheritance — `public class ChatHub : Hub` /
    // `: Hub<IClient>`. Emit single-ended Consumer WebSocket so the
    // architecture overview clusters SignalR hubs with the ws_call edges.
    if r.kind == EdgeKind::Inherits {
        if let Some(emission) = detect_csharp_signalr_hub_emission(
            r.target_name.as_str(),
        ) {
            return vec![emission];
        }
        // Integration event class — `class FooEvent : IntegrationEvent`.
        // Emits Producer EventBus keyed on the event class name.
        if let Some(emission) = detect_csharp_integration_event_emission(
            r.target_name.as_str(),
            &ref_ctx.source_symbol.name,
        ) {
            return vec![emission];
        }
        // Integration event handler — `class FooHandler :
        // IIntegrationEventHandler<FooEvent>`. Emits Consumer EventBus
        // keyed on `T` (the event type) so it pairs with the matching
        // Producer emitted by the event class above.
        if let Some(emission) = detect_csharp_integration_event_handler_emission(
            r.target_name.as_str(),
            ref_ctx.source_symbol.signature.as_deref(),
            &ref_ctx.source_symbol.name,
        ) {
            return vec![emission];
        }
        return Vec::new();
    }

    // Chain-call detection: HttpClient + RestSharp + Dapper + EF Core.
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let Some(chain_ref) = r.chain.as_ref() else {
        return Vec::new();
    };
    if let Some(emission) = detect_csharp_http_chain_emission(chain_ref, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_csharp_db_query_emission(chain_ref, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_csharp_mailer_emission(chain_ref) {
        return vec![emission];
    }
    if let Some(emission) = detect_csharp_hangfire_bg_emission(chain_ref) {
        return vec![emission];
    }
    if let Some(emission) = detect_dotnet_di_chain_emission(chain_ref) {
        return vec![emission];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();
    let mut file_namespace = None;

    // Extract namespace from the first Namespace symbol.
    for sym in &file.symbols {
        if sym.kind == crate::types::SymbolKind::Namespace {
            file_namespace = Some(sym.qualified_name.clone());
            break;
        }
    }

    // Inject global usings from the NuGet manifest (SDK implicit + GlobalUsings.cs).
    // These go first so per-file usings can override.
    if let Some(ctx) = project_ctx {
        let global_usings: &[String] = ctx
            .manifest(ManifestKind::NuGet)
            .map(|m| m.global_usings.as_slice())
            .unwrap_or(&[]);
        for ns in global_usings {
            imports.push(ImportEntry {
                imported_name: ns.clone(),
                module_path: Some(ns.clone()),
                alias: None,
                is_wildcard: true,
            });
        }
    }

    // Extract per-file using directives from refs with EdgeKind::Imports.
    for r in &file.refs {
        if r.kind == EdgeKind::Imports {
            let module = r.module.as_deref().unwrap_or(&r.target_name);
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module.to_string()),
                alias: None,
                // C# `using Namespace;` is a wildcard import — all public types
                // in that namespace become visible.
                is_wildcard: module.contains('.'),
            });
        }
    }

    FileContext {
        file_path: file.path.clone(),
        language: "csharp".to_string(),
        imports,
        file_namespace,
    }
}
