// =============================================================================
// languages/csharp/hooks.rs — CSharpHooks impl plus the concrete
// CSharpResolver (chain via CSharpChecker, `this.` stripping + scope-chain
// walk, same-namespace, wildcard using-directive, fully-qualified-name,
// field-type chain resolution that consults `lookup.field_type_str` for
// `db.SelectFrom`-style refs and tries both bare-type-name and using-
// directive-prefixed candidates, inheritance-via-implicit-`this`) plus
// 11 flow detectors (Hangfire BG, HotChocolate GraphQL, SignalR Hub,
// SmtpClient/MailKit/SendGrid mailer, EF Core LINQ + Dapper SQL parsing
// DB queries, HttpClient + RestSharp HTTP chains, Refit attributes,
// IntegrationEvent / IIntegrationEventHandler eShop event-bus pair,
// .NET DI registration `AddScoped`/`AddTransient`/`AddSingleton`) plus
// classify_external with workspace-project carve-out and longest-prefix
// wildcard using match plus build_file_context with global usings
// injection.
// =============================================================================

use super::predicates;
use super::{type_checker::CSharpChecker};
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::inheritance;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct CSharpResolver;

impl CSharpResolver {
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

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = CSharpChecker.resolve_chain(
                chain_ref, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        let effective_target = target.strip_prefix("this.").unwrap_or(target);

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

        // Field-type chain. `db.SelectFrom` (after `this.` strip) follows
        // the field's type annotation.
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

        // Inheritance walk for implicit `this` calls.
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

        None
    }

    pub(crate) fn is_visible(
        &self,
        file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        target: &SymbolInfo,
    ) -> bool {
        let vis = target.visibility.as_deref().unwrap_or("public");
        match vis {
            "public" => true,
            // Full check requires assembly information; approximate by allowing.
            "internal" => true,
            // Full check would require walking the inheritance chain.
            "protected" => true,
            "private" => &*target.file_path == file_ctx.file_path,
            _ => true,
        }
    }
}

// Hangfire: `BackgroundJob.Enqueue(...)`, `RecurringJob.AddOrUpdate(...)`.
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

// HotChocolate GraphQL: `[QueryType]`, `[MutationType]`, `[Query]`, etc.
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

// `: Hub`, `: Hub<TClient>`, `: DynamicHub` — SignalR base classes.
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

// EF Core LINQ chains + Dapper extension calls with SQL parsing.
pub(crate) fn detect_csharp_db_query_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    let segs = &chain.segments;
    let leaf = segs.last()?.name.as_str();

    // EF Core SaveChanges — entity unknown (commits all queued work).
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
    // with PascalCase middle (DbSet property name) and recognised LINQ op.
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
        "Where" | "FirstOrDefault" | "FirstOrDefaultAsync" | "First" | "FirstAsync"
        | "Single" | "SingleAsync" | "SingleOrDefault" | "SingleOrDefaultAsync"
        | "ToList" | "ToListAsync" | "ToArray" | "ToArrayAsync" | "Find" | "FindAsync"
        | "Count" | "CountAsync" | "LongCount" | "LongCountAsync" | "Any" | "AnyAsync"
        | "All" | "AllAsync" | "Sum" | "SumAsync" | "Min" | "MinAsync" | "Max" | "MaxAsync"
        | "Average" | "AverageAsync" | "Contains" | "ContainsAsync" | "Include"
        | "ThenInclude" | "OrderBy" | "OrderByDescending" | "GroupBy" | "Select"
        | "AsNoTracking" | "AsTracking" | "ToDictionary" | "ToDictionaryAsync"
        | "ToHashSet" | "ToHashSetAsync" => DbQueryOp::Select,
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

// Parse a SQL string to identify the entity (table) name and operation.
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

// HttpClient + RestSharp HTTP chain calls.
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

// Refit `[Get("/x")]` / `[Post("/x")]` Producer. Refit ships with
// `[Get]`/`[Post]`/… attributes that look identical to ASP.NET `[HttpGet]`
// except for the `Http` prefix — the prefix is the disambiguator.
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

// `using Shared.Foo;` matches workspace project `Shared.csproj` whose
// declared_name is the project filename stem. Handles exact + nested
// namespaces by dot-walking right-to-left.
pub(crate) fn matches_workspace_project(ctx: &ProjectContext, namespace: &str) -> bool {
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

// .NET namespace external classifier via the NuGet manifest.
// Always-external base prefixes (System, Microsoft) are checked first;
// NuGet package names are then checked as namespace prefixes and via
// root-segment matching.
pub(crate) fn is_manifest_external_namespace(ctx: &ProjectContext, ns: &str) -> bool {
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

// IntegrationEvent base class → Producer EventBus keyed on the event
// class name. Pairs with detect_csharp_integration_event_handler_emission.
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

// IIntegrationEventHandler<T> → Consumer EventBus keyed on T. The T is
// recovered (in order) from: the target_name itself if the extractor
// preserved generics, the source symbol's signature, otherwise the
// handler class name as a last-resort placeholder.
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

// `services.AddScoped<I, Impl>()` / AddTransient / AddSingleton.
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

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;

    if r.kind == EdgeKind::TypeRef {
        // Refit `[Get("/x")] Task<Foo> GetFoo();` Producer (the `[Http*]`-
        // prefixed ASP.NET Consumer side is handled by ExtractedRoute).
        if let Some(emission) = detect_refit_attribute_emission(
            r.target_name.as_str(),
            r.module.as_deref(),
        ) {
            return vec![emission];
        }
        if let Some(emission) = detect_csharp_hotchocolate_emission(
            r.target_name.as_str(),
        ) {
            return vec![emission];
        }
        return Vec::new();
    }

    if r.kind == EdgeKind::Inherits {
        if let Some(emission) = detect_csharp_signalr_hub_emission(
            r.target_name.as_str(),
        ) {
            return vec![emission];
        }
        if let Some(emission) = detect_csharp_integration_event_emission(
            r.target_name.as_str(),
            &ref_ctx.source_symbol.name,
        ) {
            return vec![emission];
        }
        if let Some(emission) = detect_csharp_integration_event_handler_emission(
            r.target_name.as_str(),
            ref_ctx.source_symbol.signature.as_deref(),
            &ref_ctx.source_symbol.name,
        ) {
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

    for sym in &file.symbols {
        if sym.kind == crate::types::SymbolKind::Namespace {
            file_namespace = Some(sym.qualified_name.clone());
            break;
        }
    }

    // Inject global usings from the NuGet manifest (SDK implicit +
    // GlobalUsings.cs). These go first so per-file usings can override.
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

    for r in &file.refs {
        if r.kind == EdgeKind::Imports {
            let module = r.module.as_deref().unwrap_or(&r.target_name);
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module.to_string()),
                alias: None,
                // C# `using Namespace;` is a wildcard import — all public
                // types in that namespace become visible.
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

pub struct CSharpHooks;

impl LanguageEngineHooks for CSharpHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            if let Some(ctx) = project_ctx {
                if matches_workspace_project(ctx, target) {
                    return None;
                }
            }
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, target),
                None => predicates::is_external_namespace_fallback(target),
            };
            if external {
                return Some(target.clone());
            }
            return None;
        }

        // Longest matching wildcard using directive.
        let mut best: Option<&str> = None;
        for import in &file_ctx.imports {
            if !import.is_wildcard {
                continue;
            }
            let ns = import.module_path.as_deref().unwrap_or("");
            if ns.is_empty() {
                continue;
            }

            if let Some(ctx) = project_ctx {
                if matches_workspace_project(ctx, ns) {
                    continue;
                }
            }

            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, ns),
                None => predicates::is_external_namespace_fallback(ns),
            };

            if external && (best.is_none() || ns.len() > best.unwrap().len()) {
                best = Some(ns);
            }
        }

        best.map(|s| s.to_string())
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
        CSharpResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static CSHARP_HOOKS: CSharpHooks = CSharpHooks;
