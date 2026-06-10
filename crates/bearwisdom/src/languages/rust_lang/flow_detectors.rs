// =============================================================================
// languages/rust_lang/flow_detectors.rs — Rust flow-emission detectors
//
// Each `detect_rust_*` helper inspects a `MemberChain` (or an attribute
// name + first arg, for decorator-style routes) and returns a
// `FlowEmission` when the call shape matches a known framework producer
// or consumer. The resolver in `resolve.rs` calls these in sequence and
// uses the first match.
//
// Frameworks covered: Axum, Actix, Rocket, reqwest, SQLx, Diesel, Tonic,
// apalis, rdkafka, redis, Unix domain sockets, lettre, Tauri commands,
// async-graphql.
// =============================================================================

/// Rust axum `ws.on_upgrade(handler)` — WebSocketUpgrade extractor. Emits
/// Consumer WebSocket keyed on a wildcard (the route's path is the natural
/// pair key, captured by the upstream route detector).
pub(crate) fn detect_rust_axum_ws_consumer(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !matches!(root, "ws" | "websocket" | "upgrade" | "WebSocketUpgrade") {
        return None;
    }
    if leaf != "on_upgrade" && leaf != "on_failed_upgrade" {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: "rs.axum.ws".to_string(),
        role: ChannelRole::Consumer,
        method: None,
        streaming: None,
    })
}

/// `Storage::push(job)` / `MemoryStorage::push(job)` from apalis,
/// `dispatcher.push(job)`. Single-ended BgJob Producer.
pub(crate) fn detect_rust_apalis_bgjob(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    let is_storage_root = root.ends_with("Storage")
        || matches!(root, "storage" | "queue" | "dispatcher" | "scheduler");
    if !is_storage_root {
        return None;
    }
    if !matches!(leaf, "push" | "push_raw" | "push_in" | "schedule") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: "rs.apalis".to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    })
}

/// rdkafka `producer.send(...)` and lapin `channel.basic_publish(...)`.
pub(crate) fn detect_rust_rdkafka_mq(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if matches!(leaf, "basic_publish") {
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::MessageQueue,
            name: "rs.amqp".to_string(),
            role: ChannelRole::Producer,
            method: None,
            streaming: None,
        });
    }
    if leaf == "send" && matches!(root, "producer" | "kafka_producer" | "kproducer") {
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::MessageQueue,
            name: "rs.kafka".to_string(),
            role: ChannelRole::Producer,
            method: None,
            streaming: None,
        });
    }
    None
}

/// `redis::AsyncCommands::get(&mut con, key)` and the method-style
/// `con.get(key)` from `redis::Commands`. Emits ConfigLookup keyed on
/// `redis:KEY` when the key is a string literal.
pub(crate) fn detect_rust_redis_config_lookup(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::types::CallArg;
    let segs = &chain.segments;
    let leaf = segs.last()?.name.as_str();
    if leaf != "get" && leaf != "get_ex" {
        return None;
    }
    let root = segs[0].name.as_str();
    if !matches!(
        root,
        "con" | "conn" | "redis" | "rdb" | "client" | "AsyncCommands"
    ) && !chain.segments.iter().any(|s| s.name == "AsyncCommands")
    {
        return None;
    }
    let key = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    })?;
    Some(FlowEmission::ConfigLookup {
        key: format!("redis:{}", key),
    })
}

/// Rust `UnixListener::bind(path)` → IpcCall Consumer.
/// `UnixStream::connect(path)` → IpcCall Producer.
pub(crate) fn detect_rust_uds_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use crate::types::CallArg;
    let segs = &chain.segments;
    if segs.len() != 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    let role = match (root, leaf) {
        ("UnixListener", "bind") => ChannelRole::Consumer,
        ("UnixStream", "connect") => ChannelRole::Producer,
        _ => return None,
    };
    let path = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    })?;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::IpcCall,
        name: path,
        role,
        method: None,
        streaming: None,
    })
}

pub(crate) fn detect_rust_lettre_mailer(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !matches!(root, "mailer" | "transport" | "smtp") {
        return None;
    }
    if !matches!(leaf, "send" | "send_async") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::Mailer,
        name: "rs.lettre".to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    })
}

/// HTTP-verb route attribute (`#[get("/x")]`) emitted by the Rust decorator
/// extractor as a TypeRef with `target_name` = verb and `module` = URL.
/// Lift to a Consumer HttpCall.
pub(crate) fn detect_rust_route_attribute_emission(
    attr_name: &str,
    first_arg: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    let method = match attr_name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        // `#[route("/x", method = "POST")]` — actix multi-method form.
        "route" => HttpMethod::Any,
        _ => return None,
    };
    let url = first_arg?;
    if url.is_empty() || !url.starts_with('/') {
        return None;
    }
    let name = crate::connectors::url_pattern::normalize(url);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Consumer,
        method: Some(method),
        streaming: None,
    })
}

/// SQLx macro detection — `sqlx::query!`, `sqlx::query_as!`, etc.
///
/// Encoding from the Rust extractor:
/// - target_name = verb (`"query"`, `"query_as"`, `"query_scalar"`, …)
/// - module      = first crate segment, expected `"sqlx"`
/// - call_args[0] for `query_as!` is the entity `Ident("User")`; the SQL
///   string follows. For `query!` the SQL string is `call_args[0]`.
pub(crate) fn detect_rust_sqlx_macro_emission(
    target_name: &str,
    module: Option<&str>,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    let module = module?;
    let is_sqlx_crate = module == "sqlx" || module.starts_with("sqlx::");
    if !is_sqlx_crate {
        return None;
    }
    let _verb = parse_sqlx_macro_verb(target_name)?;

    // Find the entity identifier (for query_as!) and the SQL string.
    let mut entity: Option<String> = None;
    let mut sql: Option<&str> = None;
    for arg in call_args {
        match arg {
            CallArg::Ident(s) if entity.is_none() && is_pascal_case_first_rust(s) => {
                entity = Some(s.clone());
            }
            CallArg::StringLit(s) | CallArg::TemplateLit(s) if sql.is_none() => {
                sql = Some(s.as_str());
            }
            _ => {}
        }
    }

    let (parsed_entity, op) = sql
        .and_then(parse_rust_sql_entity)
        .unwrap_or((String::new(), DbQueryOp::Other));

    let final_entity = entity.unwrap_or_else(|| {
        if parsed_entity.is_empty() {
            "*".to_string()
        } else {
            parsed_entity.clone()
        }
    });

    Some(FlowEmission::DbQuery {
        entity_name: format!("rs.{}", final_entity),
        operation: op,
    })
}

fn parse_sqlx_macro_verb(name: &str) -> Option<()> {
    match name {
        "query"
        | "query_as"
        | "query_scalar"
        | "query_file"
        | "query_file_as"
        | "query_file_scalar"
        | "query_unchecked"
        | "query_as_unchecked"
        | "query_scalar_unchecked" => Some(()),
        _ => None,
    }
}

/// Parse a SQL string to identify the table and operation.
fn parse_rust_sql_entity(
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
    } else if upper.starts_with("SELECT ") {
        // SELECT without explicit FROM — count(*) etc. Return Select with no
        // entity name so the caller can still produce an emission.
        return Some((String::new(), DbQueryOp::Select));
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
    let final_entity = entity
        .rsplit('.')
        .next()
        .unwrap_or(entity.as_str())
        .to_string();
    Some((final_entity, op))
}

/// Axum `Router::new().route("/x", get(handler))` — Consumer HttpCall.
///
/// Detection: chain root segment is "Router" (Identifier) AND any segment
/// in the chain is named "route" AND the leaf segment is "route" or
/// "nest" AND `call_args[0]` is a string literal starting with `/`.
pub(crate) fn detect_rust_axum_route_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    let segs = &chain.segments;
    let root = segs.first()?.name.as_str();
    if root != "Router" {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    if !matches!(leaf, "route" | "nest" | "merge") {
        return None;
    }
    let url = match call_args.first()? {
        CallArg::StringLit(s) | CallArg::TemplateLit(s) => s.as_str(),
        _ => return None,
    };
    if !url.starts_with('/') {
        return None;
    }
    // Try to read the HTTP verb from the second arg (`get(handler)` etc.).
    let method = call_args
        .get(1)
        .and_then(|a| match a {
            CallArg::Ident(s) => parse_http_verb_word(s.as_str()),
            _ => None,
        })
        .unwrap_or(HttpMethod::Any);
    let name = crate::connectors::url_pattern::normalize(url);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Consumer,
        method: Some(method),
        streaming: None,
    })
}

fn parse_http_verb_word(s: &str) -> Option<crate::indexer::resolve::flow_emit::HttpMethod> {
    use crate::indexer::resolve::flow_emit::HttpMethod;
    Some(match s {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        _ => return None,
    })
}

/// Actix `web::resource("/x").route(...)` / `web::scope("/api")` — Consumer
/// HttpCall.
pub(crate) fn detect_rust_actix_resource_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let second = segs[1].name.as_str();
    if root != "web" {
        return None;
    }
    if !matches!(second, "resource" | "scope") {
        return None;
    }
    // The URL is the first arg of the `web::resource(...)` call. When the
    // ref we're examining is for `web::resource("/x")` itself, args carry
    // the URL. Chained `.route(...)` calls have their own chain — the URL
    // sits on the root call's args. We accept either: if `call_args` has a
    // string starting with `/`, use it.
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) | CallArg::TemplateLit(s) if s.starts_with('/') => Some(s.as_str()),
        _ => None,
    })?;
    let name = crate::connectors::url_pattern::normalize(url);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Consumer,
        method: Some(HttpMethod::Any),
        streaming: None,
    })
}

/// reqwest: `client.get("/x")`, `client.post("/x").json(...)`,
/// `Client::new().get("/x")` — Producer HttpCall.
///
/// Detection: chain's leaf is one of the canonical HTTP verbs AND
/// `call_args[0]` is a string literal that looks like a URL (starts with
/// `/` or `http`).
pub(crate) fn detect_rust_reqwest_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    let method = match leaf {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        _ => return None,
    };
    // Skip if root looks like a router/server bound rather than a client.
    // `Router::new().get(...)` exists in some frameworks; the route-call
    // detector handles that. Here we want the chain root to be an
    // Identifier (a `client` variable) or PascalCase "Client" type.
    let root = segs.first()?;
    let root_name = root.name.as_str();
    if root_name == "Router" || root_name == "App" || root_name == "Server" {
        return None;
    }
    // Match either:
    //   `client.get("/x")` — root is Identifier, leaf is verb
    //   `Client::new().get("/x")` — chain has segments[Client, new, get]
    let looks_like_client = matches!(
        root.kind,
        crate::types::SegmentKind::Identifier | crate::types::SegmentKind::SelfRef
    ) || root_name.ends_with("Client")
        || root_name == "reqwest";
    if !looks_like_client {
        return None;
    }
    let url = match call_args.first()? {
        CallArg::StringLit(s) | CallArg::TemplateLit(s) => s.as_str(),
        _ => return None,
    };
    if !(url.starts_with('/') || url.starts_with("http://") || url.starts_with("https://")) {
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

/// Diesel: `<table>::table.filter(...).first(&conn)` / `.load(...)` /
/// `.get_result(...)` / `.execute(...)` — DbQuery keyed on the table name.
pub(crate) fn detect_rust_diesel_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};

    let segs = &chain.segments;
    if segs.len() < 3 {
        return None;
    }
    // Pattern: ["users"(Identifier or scoped), "table"(Property), …, leaf].
    // The `table` segment is the Diesel convention.
    let has_table = segs.iter().any(|s| s.name == "table");
    if !has_table {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    let op = parse_diesel_op(leaf)?;
    // Entity is the segment immediately before "table".
    let table_idx = segs.iter().position(|s| s.name == "table")?;
    if table_idx == 0 {
        return None;
    }
    let entity = segs[table_idx - 1].name.as_str();
    if entity.is_empty() {
        return None;
    }
    Some(FlowEmission::DbQuery {
        entity_name: format!("rs.{}", entity),
        operation: op,
    })
}

fn parse_diesel_op(name: &str) -> Option<crate::indexer::resolve::flow_emit::DbQueryOp> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    Some(match name {
        // Read operations.
        "first" | "first_async" | "load" | "load_async" | "get_result" | "get_result_async"
        | "get_results" | "get_results_async" | "select" | "filter" | "find" | "order"
        | "order_by" | "limit" | "offset" | "count" | "count_async" | "single_value"
        | "execute_returning" | "filter_by" | "distinct" => DbQueryOp::Select,
        // Mutations.
        "insert_into" | "values" | "do_update" | "do_nothing" => DbQueryOp::Insert,
        "set" | "update" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        // execute() collapses to Other because operation isn't recoverable
        // without parsing the surrounding builder; pairing on entity is
        // still useful.
        "execute" | "execute_async" => DbQueryOp::Other,
        _ => return None,
    })
}

/// Tonic generated client: `<Service>Client::new(channel).method(req)` —
/// Producer RpcCall keyed on the service.method name.
///
/// Detection: chain has at least 3 segments, root ends in "Client",
/// segment[1] is a constructor (`new` / `connect` / `with_origin` /
/// `with_interceptor`), and the leaf is the RPC method.
pub(crate) fn detect_rust_tonic_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};

    let segs = &chain.segments;
    if segs.len() < 3 {
        return None;
    }
    let root = segs[0].name.as_str();
    if !root.ends_with("Client") || root == "Client" {
        return None;
    }
    let ctor = segs[1].name.as_str();
    if !matches!(
        ctor,
        "new" | "connect" | "with_origin" | "with_interceptor" | "with_channel"
    ) {
        return None;
    }
    let method = segs.last()?.name.as_str();
    if matches!(
        method,
        "new" | "connect" | "with_origin" | "with_interceptor" | "with_channel"
    ) {
        return None;
    }
    // Service name = root with "Client" suffix stripped.
    let service = root.strip_suffix("Client").unwrap_or(root);
    let name = format!("{}.{}", service, method);
    use crate::indexer::resolve::flow_emit::StreamKind;
    let streaming = Some(StreamKind::from_method_name(method));
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name,
        role: ChannelRole::Producer,
        method: None,
        streaming,
    })
}

fn is_pascal_case_first_rust(name: &str) -> bool {
    name.chars()
        .next()
        .map_or(false, |c| c.is_ascii_uppercase())
}

/// `#[tauri::command]` on a Rust function — Consumer IpcCall. Emits with
/// a wildcard channel name; pairing relies on the TS invoke("name") side
/// matching the function's own name (handled by the symbol-resolution
/// step).
pub(crate) fn detect_rust_tauri_command_attribute(
    attr_name: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    if attr_name != "command" {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::IpcCall,
        name: "tauri.*".to_string(),
        role: ChannelRole::Consumer,
        method: None,
        streaming: None,
    })
}

/// async-graphql / juniper procedural-macro attributes on schema root
/// impls. Emits Consumer GraphQLOp keyed on the marker so the
/// architecture overview clusters GraphQL roots across languages.
pub(crate) fn detect_rust_async_graphql_attribute(
    attr_name: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let kind = match attr_name {
        // async-graphql.
        "Object" | "ComplexObject" | "SimpleObject" | "MergedObject" => "query",
        "Subscription" => "subscription",
        // juniper.
        "graphql_object" | "graphql_subscription" | "graphql_interface" => "query",
        _ => return None,
    };
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::GraphQLOp,
        name: format!("rs.graphql.{}", kind),
        role: ChannelRole::Consumer,
        method: None,
        streaming: None,
    })
}
