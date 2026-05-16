// =============================================================================
// languages/python/flow_detectors.rs — Python FlowEmission detectors
//
// One detector per framework / protocol surface:
//   - HTTP Consumer (FastAPI / Flask / Django path() / Channels)
//   - HTTP Producer (requests / httpx / aiohttp)
//   - DB query (SQLAlchemy / Django ORM / raw cursor.execute)
//   - RPC (grpc-python stub)
//   - GraphQL (strawberry / graphene)
//   - Queue / mail (background-job + mailer)
//
// All detectors observe a `RefContext` + `FileContext` plus a `&dyn SymbolLookup`
// and emit `FlowEmission` records via the resolver.
// =============================================================================

use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};

pub(crate) fn detect_python_redis_lookup(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::types::CallArg;
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !matches!(root, "redis" | "r" | "cache" | "memcached" | "mc") {
        return None;
    }
    if !matches!(leaf, "get" | "hget" | "mget" | "getex") {
        return None;
    }
    let key = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    })?;
    Some(FlowEmission::ConfigLookup { key: format!("redis:{}", key) })
}

pub(crate) fn detect_python_bgjob_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    // Celery: `task.delay(...)` / `task.apply_async(...)`.
    // RQ: `queue.enqueue(...)`.
    // Dramatiq: `task.send(...)`.
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    let root = segs[0].name.as_str();
    let is_bg = matches!(leaf, "delay" | "apply_async" | "send_with_options")
        || (leaf == "enqueue" && (root == "queue" || root.ends_with("Queue") || root.ends_with("queue")));
    if !is_bg {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: format!("py.{}", root),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

pub(crate) fn detect_python_mailer_emission(
    target_name: &str,
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    // Django: `send_mail(...)`, `EmailMessage(...).send()`, `mail.send_mass_mail`.
    // Flask-Mail: `mail.send(msg)`. Generic SMTP: `smtp.send_message`.
    let leaf = chain.segments.last().map(|s| s.name.as_str()).unwrap_or(target_name);
    if !matches!(leaf, "send" | "send_message" | "send_mail" | "send_mass_mail" | "send_html_mail") {
        return None;
    }
    // Restrict to chains whose root looks mail-related.
    let root = chain.segments.first()?.name.as_str();
    if !matches!(
        root,
        "mail" | "Mail" | "EmailMessage" | "EmailMultiAlternatives" | "smtp" | "smtplib"
    ) {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::Mailer,
        name: format!("py.{}", root),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// HTTP Consumer — route decorators (FastAPI / Flask / Django path())
// ---------------------------------------------------------------------------

/// Recognise route decorators and emit Consumer HttpCall:
/// - FastAPI: `@app.get('/x')`, `@router.post('/x')`, also
///   `@app.put`/`patch`/`delete`/`head`/`options`/`api_route`. The decorator
///   extractor stores the dotted name (`app.get`) in `target_name` so the
///   trailing segment is the HTTP verb.
/// - Flask: `@app.route('/x', methods=['GET'])` — the verb defaults to GET
///   and lives in the optional `methods=` kwarg which we can't see from
///   the decorator first-arg, so emit with method=Any.
pub(crate) fn detect_python_route_decorator_emission(
    target_name: &str,
    first_arg: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    let path = first_arg?.trim();
    if path.is_empty() {
        return None;
    }
    let verb_seg = target_name.rsplit('.').next().unwrap_or(target_name);
    // WebSocket consumer: @app.websocket("/ws") / @router.websocket("/ws").
    if verb_seg == "websocket" {
        let name = crate::connectors::url_pattern::normalize(path);
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::WebSocket,
            name,
            role: ChannelRole::Consumer,
            method: None,
        streaming: None,
        });
    }
    let method = match verb_seg {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        // Flask `@app.route` and FastAPI `@app.api_route` accept any method
        // — the verb list lives in a kwarg we don't currently parse.
        "route" | "api_route" => HttpMethod::Any,
        _ => return None,
    };
    let name = crate::connectors::url_pattern::normalize(path);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Consumer,
        method: Some(method),
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// HTTP Producer — requests / httpx / aiohttp / generic client.get('/x')
// ---------------------------------------------------------------------------

/// Recognise common Python HTTP client call shapes and emit Producer
/// HttpCall. Shapes handled (all leaf-segment-named verb + first
/// string-literal arg):
/// - `requests.get('/x')` / `requests.post(...)` / etc.
/// - `httpx.get(...)`, `httpx.AsyncClient().get(...)`, `client.get(...)`
/// - `aiohttp.ClientSession.get(...)`, `session.get(...)`
///
/// The detector fires when the chain leaf is a known HTTP verb AND the
/// file imports a recognised HTTP client library (so generic
/// `obj.get(key)` shapes like dict accessors don't misfire). The chain
/// root name doesn't need to match the library — long-lived session
/// bindings (`session = httpx.Client(); session.get('/x')`) just work via
/// the file's import set.
pub(crate) fn detect_python_http_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
    file_ctx: &FileContext,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    let leaf = chain.segments.last()?.name.as_str();
    let method = match leaf {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        "request" => HttpMethod::Any,
        // urllib `urlopen(url)` / `urllib.request.urlopen(url)`.
        "urlopen" => HttpMethod::Any,
        _ => return None,
    };
    if !file_imports_python_http_library(file_ctx) {
        return None;
    }
    let url = match call_args.first()? {
        CallArg::StringLit(s) => s.clone(),
        _ => return None,
    };
    if url.is_empty() {
        return None;
    }
    if !(url.starts_with('/') || url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    let name = crate::connectors::url_pattern::normalize(&url);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Producer,
        method: Some(method),
    streaming: None,
    })
}

fn file_imports_python_http_library(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let m = imp.module_path.as_deref().unwrap_or(imp.imported_name.as_str());
        matches!(
            m.split('.').next().unwrap_or(m),
            "requests" | "httpx" | "aiohttp" | "urllib3" | "urllib" | "fastapi"
        )
    })
}

fn file_imports_python_django(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let m = imp.module_path.as_deref().unwrap_or(imp.imported_name.as_str());
        m.split('.').next().unwrap_or(m) == "django"
    })
}

pub(crate) fn file_imports_python_channels(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let m = imp.module_path.as_deref().unwrap_or(imp.imported_name.as_str());
        m.split('.').next().unwrap_or(m) == "channels"
    })
}

fn file_imports_python_sqlalchemy(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let m = imp.module_path.as_deref().unwrap_or(imp.imported_name.as_str());
        let root = m.split('.').next().unwrap_or(m);
        root == "sqlalchemy" || root == "sqlmodel"
    })
}

// ---------------------------------------------------------------------------
// Django `path()` / `re_path()` routes → Consumer HttpCall
// ---------------------------------------------------------------------------

/// Detect Django URL conf entries:
///   `path("users/<int:id>/", views.detail)`
///   `re_path(r"^users/(?P<id>\d+)/$", views.detail)`
/// Both emit a Consumer HttpCall with method=Any (Django path entries
/// match any verb; per-method filtering lives inside the view).
/// Strawberry / Graphene GraphQL decorators. Recognises:
/// - `@strawberry.field` / `@strawberry.mutation` / `@strawberry.subscription`
///   on a resolver method → Consumer GraphQLOp keyed on the method name.
/// - `@strawberry.type` / `@strawberry.input` / `@strawberry.interface` on
///   a class → emits a DbEntity-style marker via the existing decorator
///   path. Schema-typed entities aren't paired today, so we limit
///   emission to the operation decorators where we have a method name.
pub(crate) fn detect_python_graphql_decorator_emission(
    target_name: &str,
    source_symbol_name: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let last = target_name.rsplit('.').next().unwrap_or(target_name);
    if !matches!(last, "field" | "mutation" | "subscription") {
        return None;
    }
    // The root of the dotted decorator must be a graphql library hint.
    let root = target_name.split('.').next().unwrap_or(target_name);
    if !matches!(root, "strawberry" | "graphene") {
        return None;
    }
    if source_symbol_name.is_empty() {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::GraphQLOp,
        name: format!("{}:{}", last, source_symbol_name),
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

/// Django Channels consumer inheritance: any of the WebSocket consumer
/// base classes triggers a single-ended Consumer WebSocket emission for
/// the subclass.
pub(crate) fn detect_python_channels_consumer_inheritance(
    target_name: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let base = target_name.rsplit('.').next().unwrap_or(target_name);
    if !matches!(
        base,
        "WebsocketConsumer"
            | "AsyncWebsocketConsumer"
            | "JsonWebsocketConsumer"
            | "AsyncJsonWebsocketConsumer"
    ) {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: "py.channels".to_string(),
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

/// Django Channels routing.py: `path("ws/x", X.as_asgi())` declares a
/// WebSocket route. Only fires when the file imports `channels.*` —
/// otherwise the call routes through the regular HTTP `path` detector.
pub(crate) fn detect_python_channels_path_emission(
    target_name: &str,
    call_args: &[crate::types::CallArg],
    file_ctx: &FileContext,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use crate::types::CallArg;
    if !matches!(target_name, "path" | "re_path") {
        return None;
    }
    if !file_imports_python_channels(file_ctx) {
        return None;
    }
    let url = match call_args.first()? {
        CallArg::StringLit(s) => s.clone(),
        _ => return None,
    };
    if url.is_empty() {
        return None;
    }
    let cleaned: String = url
        .trim_start_matches('^')
        .trim_end_matches('$')
        .to_string();
    let name = crate::connectors::url_pattern::normalize(&cleaned);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name,
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

pub(crate) fn detect_python_django_path_emission(
    target_name: &str,
    call_args: &[crate::types::CallArg],
    file_ctx: &FileContext,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    if !matches!(target_name, "path" | "re_path") {
        return None;
    }
    if !file_imports_python_django(file_ctx) {
        return None;
    }
    let url = match call_args.first()? {
        CallArg::StringLit(s) => s.clone(),
        _ => return None,
    };
    if url.is_empty() {
        return None;
    }
    // Normalise leading `^` and trailing `$` from re_path regex anchors so
    // the pattern compares cleanly against Producer-side URLs.
    let cleaned: String = url
        .trim_start_matches('^')
        .trim_end_matches('$')
        .to_string();
    let name = crate::connectors::url_pattern::normalize(&cleaned);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Consumer,
        method: Some(HttpMethod::Any),
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// SQLAlchemy 2.x `select(Entity)` — bare-call DbQuery
// ---------------------------------------------------------------------------

/// Detect `select(User)` / `delete(User)` / `update(User)` / `insert(User)`
/// from SQLAlchemy 2.x where the entity name is the first positional arg.
/// Fires only when the file imports sqlalchemy or sqlmodel to avoid
/// matching unrelated `select` / `update` helper functions.
pub(crate) fn detect_python_sqlalchemy_select_call(
    target_name: &str,
    call_args: &[crate::types::CallArg],
    file_ctx: &FileContext,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    let op = match target_name {
        "select" => DbQueryOp::Select,
        "insert" => DbQueryOp::Insert,
        "update" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        _ => return None,
    };
    if !file_imports_python_sqlalchemy(file_ctx) {
        return None;
    }
    let entity = call_args.iter().find_map(|a| match a {
        CallArg::Ident(s) if is_pascal_case_first(s) => Some(s.clone()),
        _ => None,
    })?;
    Some(FlowEmission::DbQuery {
        entity_name: namespaced_python_entity(&entity),
        operation: op,
    })
}

// ---------------------------------------------------------------------------
// Raw `cursor.execute("SELECT ...")` → DbQuery
// ---------------------------------------------------------------------------

/// Detect raw DB-API `cursor.execute("SELECT ...")` / `connection.execute(...)`
/// where the first arg is a SQL string. Entity is parsed from the FROM /
/// UPDATE / INSERT INTO / DELETE FROM clause.
pub(crate) fn detect_python_cursor_execute_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::types::CallArg;

    let leaf = chain.segments.last()?.name.as_str();
    if !matches!(leaf, "execute" | "executemany" | "executescript") {
        return None;
    }
    // Avoid matching SQLAlchemy `session.execute(select(User))` which is
    // detected on the inner `select` call instead — require the first arg
    // to be a StringLit for the raw DB-API path.
    let sql = match call_args.first()? {
        CallArg::StringLit(s) => s.as_str(),
        _ => return None,
    };
    let (entity, op) = parse_python_sql_entity(sql)?;
    Some(FlowEmission::DbQuery {
        entity_name: namespaced_python_entity(&entity),
        operation: op,
    })
}

fn parse_python_sql_entity(
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
    Some((
        entity
            .rsplit('.')
            .next()
            .unwrap_or(entity.as_str())
            .to_string(),
        op,
    ))
}

// ---------------------------------------------------------------------------
// grpc-python Stub → RpcCall Producer
// ---------------------------------------------------------------------------

/// Detect `<Service>Stub(channel).Method(req)` — chain root is a
/// PascalCase identifier ending in `Stub` followed by the rpc method.
pub(crate) fn detect_python_grpc_stub_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    if !root.ends_with("Stub") || root == "Stub" {
        return None;
    }
    if !is_pascal_case_first(root) {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    // Skip when leaf is itself the constructor — `Stub(channel)` alone
    // shouldn't emit; only `Stub(channel).Method(...)` patterns do.
    if leaf == root {
        return None;
    }
    let service = root.strip_suffix("Stub").unwrap_or(root);
    let name = format!("{}.{}", service, leaf);
    use crate::indexer::resolve::flow_emit::StreamKind;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name,
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::from_method_name(leaf)),
    })
}

// ---------------------------------------------------------------------------
// DbQuery — SQLAlchemy / Django ORM
// ---------------------------------------------------------------------------

/// Recognise DbQuery shapes and emit a single-ended DbQuery emission.
/// Shapes handled:
/// - SQLAlchemy `session.query(Entity)` → entity from the first chain
///   segment after `query` (none recorded — the query subject is the
///   call arg, not part of the chain in this AST).
/// - SQLAlchemy `Entity.query.filter(...)` (Flask-SQLAlchemy style) →
///   entity is the chain root.
/// - SQLAlchemy 2.x `select(Entity).where(...)` — entity is the call arg
///   of `select`; not recoverable from chain alone, skipped.
/// - Django `Entity.objects.filter(...)` / `Entity.objects.get(...)` /
///   `Entity.objects.create(...)` → entity is the chain root.
pub(crate) fn detect_python_db_query_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;

    let segs = &chain.segments;
    // Require exactly `[Entity, manager, op]` — three segments, with the
    // ORM op as the immediate leaf. Longer chains
    // (`Entity.objects.filter(x).order_by(y)`) emit their own
    // shorter-prefix Calls ref via the per-call_expression visitor; the
    // outermost ref carries the full chain and re-emitting on it would
    // multiply by chain depth. Capping at len==3 ensures one DbQuery
    // emission per Django/SQLAlchemy entity-access site.
    if segs.len() != 3 {
        return None;
    }
    let root = segs[0].name.as_str();
    let mid = segs[1].name.as_str();
    let leaf = segs[2].name.as_str();
    if !is_pascal_case_first(root) {
        return None;
    }

    // Flask-SQLAlchemy `Entity.query.<op>` — `Entity` is the model class.
    if mid == "query" {
        let op = sqlalchemy_op_from_leaf(leaf)?;
        return Some(FlowEmission::DbQuery {
            entity_name: namespaced_python_entity(root),
            operation: op,
        });
    }

    // Django ORM `Entity.objects.<op>` — `Entity` is the model class.
    if mid == "objects" {
        let op = django_op_from_leaf(leaf)?;
        return Some(FlowEmission::DbQuery {
            entity_name: namespaced_python_entity(root),
            operation: op,
        });
    }

    None
}

/// Prefix the entity name with `py.` so the pairer's loose
/// `entity_names_match` (case-insensitive + pluralization tolerance)
/// doesn't cross-pair Python `Document.objects.X` queries with TypeScript
/// `@Document` decorators (Mongoose, NestJS) that produce a `DbEntity` for
/// the same bare name. The cost is that until a Python-side DbEntity
/// emission ships, these DbQuery rows stay single-ended — which is the
/// correct behaviour given the lack of a Python ORM-model FlowEmission
/// today.
fn namespaced_python_entity(name: &str) -> String {
    format!("py.{}", name)
}

fn sqlalchemy_op_from_leaf(
    name: &str,
) -> Option<crate::indexer::resolve::flow_emit::DbQueryOp> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    Some(match name {
        "filter" | "filter_by" | "first" | "all" | "one" | "one_or_none" | "scalar" | "get"
        | "count" | "exists" => DbQueryOp::Select,
        "update" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        "add" | "insert" => DbQueryOp::Insert,
        _ => return None,
    })
}

fn django_op_from_leaf(
    name: &str,
) -> Option<crate::indexer::resolve::flow_emit::DbQueryOp> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    Some(match name {
        "filter" | "all" | "get" | "exclude" | "first" | "last" | "exists" | "count" | "values"
        | "values_list" => DbQueryOp::Select,
        "create" | "bulk_create" => DbQueryOp::Insert,
        "update" | "update_or_create" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        "get_or_create" => DbQueryOp::Upsert,
        _ => return None,
    })
}

fn is_pascal_case_first(name: &str) -> bool {
    name.chars().next().map_or(false, |c| c.is_ascii_uppercase())
}

// ---------------------------------------------------------------------------
