// =============================================================================
// java/flow_detectors.rs — Java FlowEmission detectors
//
// Free helpers invoked by `JavaHooks::detect_flow_emissions` to recognise
// framework-specific call shapes and annotation forms, and turn them into
// `FlowEmission` values for the architecture overview / blast-radius graph.
// Each detector owns one library or one shape:
//
//   - Quartz / Spring `@Scheduled` programmatic registration (BgJob)
//   - JMS / Kafka `producer.send(...)` (MessageQueue Producer)
//   - Spring stereotype annotations (DiBinding)
//   - Spring STOMP / Jakarta WebSocket annotations (WebSocket Consumer)
//   - Spring Data Redis `redisTemplate.opsForX().get(...)` (ConfigLookup)
//   - JavaMailSender / mail service `.send(...)` (Mailer Producer)
//   - HTTP client chains: RestTemplate, WebClient, OkHttp (HttpCall Producer)
//   - JPA EntityManager `.find` / `.persist` / `.merge` / `.remove` (DbQuery)
//   - Spring Data `@Query("JPQL")` annotation (DbQuery)
//   - JdbcTemplate `.query` / `.update` / `.queryForObject` (DbQuery)
//   - Retrofit `@GET` / `@POST` / etc. annotations (HttpCall Producer)
//   - gRPC stub `.fooMethod(req)` (RpcCall Producer)
// =============================================================================

/// Java Quartz `scheduler.scheduleJob(...)`, Spring `@Scheduled`-style
/// programmatic registration. Single-ended BgJob Producer.
pub(crate) fn detect_java_quartz_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !matches!(root, "scheduler" | "jobScheduler" | "quartzScheduler") {
        return None;
    }
    if !matches!(leaf, "scheduleJob" | "scheduleJobs") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: "java.quartz".to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    })
}

/// JMS `producer.send(message)` and Kafka `kafkaTemplate.send(topic, payload)`.
/// Captures the topic (Kafka) when it's a string literal.
pub(crate) fn detect_java_jms_kafka_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use crate::types::CallArg;
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if leaf != "send" {
        return None;
    }
    // Spring KafkaTemplate — topic from first string arg.
    if matches!(root, "kafkaTemplate" | "kafkaProducer") {
        let topic = call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        });
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::MessageQueue,
            name: topic.unwrap_or_else(|| "java.kafka".to_string()),
            role: ChannelRole::Producer,
            method: None,
            streaming: None,
        });
    }
    // JMS — `producer.send(message)`.
    if matches!(root, "jmsTemplate" | "producer" | "messageProducer") {
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::MessageQueue,
            name: "java.jms".to_string(),
            role: ChannelRole::Producer,
            method: None,
            streaming: None,
        });
    }
    None
}

/// Spring STOMP `@MessageMapping("/x")` or `@SubscribeMapping("/x")` on a
/// handler method → WebSocket Consumer keyed on the destination.
/// Match a Spring stereotype annotation (`@Service`, `@Repository`,
/// `@Component`, `@RestController`, `@Controller`) on the class TypeRef.
/// Emits a single-ended `DiBinding` with `container = "spring"` so the
/// architecture overview clusters Spring-managed beans. Pair-keyed
/// matching to the implemented interface (via `implements` edges) needs
/// cross-ref state the resolver doesn't carry; the single-ended marker
/// still clusters the bean as a DI participant.
pub(crate) fn detect_spring_stereotype_emission(
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    if !matches!(
        target,
        "Service" | "Repository" | "Component" | "RestController" | "Controller"
    ) {
        return None;
    }
    Some(FlowEmission::DiBinding {
        service_symbol_id: 0,
        container: Some("spring".to_string()),
    })
}

pub(crate) fn detect_java_message_mapping_emission(
    target: &str,
    module: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    // Spring STOMP routing annotations + Jakarta WebSocket / Tyrus
    // lifecycle annotations. `@ServerEndpoint("/ws")` is the most
    // useful one for pairing — it carries the WS path. The lifecycle
    // ones (`@OnOpen`, `@OnMessage`, `@OnClose`, `@OnError`) mark
    // methods on an already-declared endpoint class; they emit a
    // single-ended marker so the class clusters with WS endpoints.
    if !matches!(
        target,
        "MessageMapping"
            | "SubscribeMapping"
            | "OnMessage"
            | "OnOpen"
            | "OnClose"
            | "OnError"
            | "ServerEndpoint"
            | "ClientEndpoint"
    ) {
        return None;
    }
    let dest = module.unwrap_or("");
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: if dest.is_empty() {
            "java.ws".to_string()
        } else {
            dest.to_string()
        },
        role: ChannelRole::Consumer,
        method: None,
        streaming: None,
    })
}

/// Spring Data Redis `redisTemplate.opsForValue().get(key)` → ConfigLookup.
pub(crate) fn detect_java_redis_template_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::types::CallArg;
    let segs = &chain.segments;
    if segs.len() < 3 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !matches!(root, "redisTemplate" | "stringRedisTemplate") {
        return None;
    }
    if leaf != "get" {
        return None;
    }
    // Look for `opsForValue` (or `opsForHash` / `opsForSet`) in the chain.
    if !chain.segments.iter().any(|s| s.name.starts_with("opsFor")) {
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

pub(crate) fn detect_java_mailer_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    // JavaMailSender.send / mailSender.send.
    if !matches!(
        root,
        "mailSender" | "javaMailSender" | "emailService" | "mailService"
    ) {
        return None;
    }
    if !matches!(leaf, "send" | "sendAsync") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::Mailer,
        name: format!("java.{}", root),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    })
}

// ---------------------------------------------------------------------------
// HTTP Producer detection — RestTemplate / WebClient / OkHttp
// ---------------------------------------------------------------------------

/// Recognise Java HTTP-client chain calls and emit Producer
/// `NamedChannel { kind: HttpCall, .. }` keyed on the first string-literal arg.
///
/// Shapes handled:
/// - **RestTemplate**: `<rt>.getForObject("/x", ...)`, `.getForEntity(...)`,
///   `.postForObject(...)`, `.postForEntity(...)`, `.put(...)`, `.delete(...)`,
///   `.patchForObject(...)`, `.headForHeaders(...)`, `.exchange(...)`.
/// - **WebClient (fluent)**: `<wc>.get().uri("/x").retrieve()...` —
///   detect via leaf `uri` AND chain containing one of `get`/`post`/`put`/
///   `patch`/`delete`/`head`/`options`/`method` earlier in the chain.
/// - **OkHttp**: `<builder>.url("...")` — emit Any-method (verb is on a
///   separate `.method(...)` / `.get()` / `.post()` builder call we don't
///   correlate to the url() call statically).
pub(crate) fn detect_java_http_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    let leaf = chain.segments.last()?.name.as_str();

    // First arg as a string literal — required for a pairing key.
    let first_string = match call_args.first()? {
        CallArg::StringLit(s) => Some(s.clone()),
        _ => None,
    };

    // RestTemplate single-call verb methods.
    if let Some(method) = parse_resttemplate_verb(leaf) {
        let url = first_string?;
        if url.is_empty() {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: crate::connectors::url_pattern::normalize(&url),
            role: ChannelRole::Producer,
            method: Some(method),
            streaming: None,
        });
    }

    // WebClient `.uri('/x')` — verb comes from a separate `.get()`/`.post()`
    // segment earlier in the chain. Scan chain segments for the verb token.
    if leaf == "uri" {
        let method = chain
            .segments
            .iter()
            .find_map(|s| parse_webclient_verb(s.name.as_str()))
            .unwrap_or(HttpMethod::Any);
        let url = first_string?;
        if url.is_empty() {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: crate::connectors::url_pattern::normalize(&url),
            role: ChannelRole::Producer,
            method: Some(method),
            streaming: None,
        });
    }

    // OkHttp `.url("https://api.example.com/x")` — verb on a separate
    // builder method, so emit Any here.
    if leaf == "url" {
        let url = first_string?;
        if url.is_empty() {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: crate::connectors::url_pattern::normalize(&url),
            role: ChannelRole::Producer,
            method: Some(HttpMethod::Any),
            streaming: None,
        });
    }

    None
}

fn parse_resttemplate_verb(name: &str) -> Option<crate::indexer::resolve::flow_emit::HttpMethod> {
    use crate::indexer::resolve::flow_emit::HttpMethod;
    Some(match name {
        "getForObject" | "getForEntity" => HttpMethod::Get,
        "postForObject" | "postForEntity" | "postForLocation" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patchForObject" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "headForHeaders" => HttpMethod::Head,
        "optionsForAllow" => HttpMethod::Options,
        "exchange" | "execute" => HttpMethod::Any,
        _ => return None,
    })
}

fn parse_webclient_verb(name: &str) -> Option<crate::indexer::resolve::flow_emit::HttpMethod> {
    use crate::indexer::resolve::flow_emit::HttpMethod;
    Some(match name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        "method" => HttpMethod::Any,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// JPA / Spring Data — DbQuery emission
// ---------------------------------------------------------------------------

/// Recognise JPA EntityManager chain calls and emit a `DbQuery` keyed on
/// the entity class from the first arg (`Entity.class`). Shapes handled:
/// - `entityManager.find(Entity.class, id)`
/// - `entityManager.getReference(Entity.class, id)`
/// - `entityManager.persist(entityInstance)` — entity name unknown, skipped.
/// - `entityManager.remove(...)` / `merge(...)` — same, skipped.
/// - `entityManager.createQuery("FROM Entity", Entity.class)` — emit on
///   the second class-literal arg when present.
///
/// Spring Data derived methods (`findByName`, `existsByEmail`) and
/// generic-typed repository calls are deferred — they require resolving
/// the repository's `<T>` type parameter to a model class, which is not
/// statically recoverable from the chain alone.
pub(crate) fn detect_java_db_query_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();

    // EntityManager method names that take Entity.class as the first arg.
    let op = match leaf {
        "find" | "getReference" => DbQueryOp::Select,
        "createQuery" | "createNativeQuery" => DbQueryOp::Select,
        _ => return None,
    };
    if !is_entity_manager_root(root) {
        return None;
    }

    // First-arg shape varies. `find(Entity.class, id)` → class literal first.
    // `createQuery("FROM ...", Entity.class)` → string first, class second.
    // Pick the class-literal arg.
    let entity = call_args.iter().find_map(|a| match a {
        CallArg::Ident(name) if is_pascal_case_first_java(name) => Some(name.clone()),
        _ => None,
    })?;

    Some(FlowEmission::DbQuery {
        entity_name: format!("java.{}", entity),
        operation: op,
    })
}

fn is_entity_manager_root(name: &str) -> bool {
    matches!(
        name,
        "entityManager"
            | "em"
            | "entityManagerFactory"
            | "session" // Hibernate Session has similar API
            | "sessionFactory"
    )
}

fn is_pascal_case_first_java(name: &str) -> bool {
    name.chars()
        .next()
        .map_or(false, |c| c.is_ascii_uppercase())
}

// ---------------------------------------------------------------------------
// `@Query("...")` annotation → DbQuery emission
// ---------------------------------------------------------------------------

/// Spring Data `@Query("SELECT u FROM User u WHERE ...")` annotation on a
/// repository interface method. Emit `DbQuery` keyed on the entity name
/// parsed from the JPQL `FROM <Entity>` clause. Native queries use a SQL
/// table name we can't reliably map to a Java class, so for `@Query` we
/// only handle JPQL where the FROM clause is a class name (PascalCase).
pub(crate) fn detect_jpa_query_annotation_emission(
    annotation_name: &str,
    first_arg: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if annotation_name != "Query" {
        return None;
    }
    let sql = first_arg?.trim();
    if sql.is_empty() {
        return None;
    }
    let upper = sql.to_ascii_uppercase();
    let from_idx = upper.find(" FROM ").or_else(|| upper.find("FROM "))?;
    let after_from = &sql[from_idx + 5..];
    let entity = after_from
        .split_whitespace()
        .next()?
        .trim_end_matches(',')
        .trim_end_matches(';');
    if !is_pascal_case_first_java(entity) {
        return None;
    }
    let op = if upper.starts_with("SELECT") || upper.contains(" SELECT ") {
        DbQueryOp::Select
    } else if upper.starts_with("UPDATE") || upper.contains(" UPDATE ") {
        DbQueryOp::Update
    } else if upper.starts_with("DELETE") || upper.contains(" DELETE ") {
        DbQueryOp::Delete
    } else if upper.starts_with("INSERT") || upper.contains(" INSERT ") {
        DbQueryOp::Insert
    } else {
        DbQueryOp::Select
    };
    Some(FlowEmission::DbQuery {
        entity_name: format!("java.{}", entity),
        operation: op,
    })
}

// ---------------------------------------------------------------------------
// JdbcTemplate — DbQuery emission
// ---------------------------------------------------------------------------

/// Recognise JdbcTemplate / NamedParameterJdbcTemplate chains:
/// - `jdbcTemplate.query("SELECT ...", rowMapper)`,
///   `queryForObject(...)`, `queryForList(...)`, `queryForMap(...)`,
///   `queryForRowSet(...)`.
/// - `jdbcTemplate.update(...)`, `batchUpdate(...)`, `execute(...)`.
///
/// First arg as a SQL string yields the table; the verb is the chain leaf.
pub(crate) fn detect_java_jdbc_template_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !is_jdbc_template_root(root) {
        return None;
    }
    let leaf_op = match leaf {
        "query" | "queryForObject" | "queryForList" | "queryForMap" | "queryForRowSet"
        | "queryForStream" => DbQueryOp::Select,
        "update" | "batchUpdate" => DbQueryOp::Update,
        "execute" => DbQueryOp::Other,
        _ => return None,
    };
    let sql = match call_args.first()? {
        CallArg::StringLit(s) | CallArg::TemplateLit(s) => s.clone(),
        _ => return None,
    };
    let (entity, sql_op) = match parse_java_sql_entity(&sql) {
        Some(pair) => pair,
        None => return None,
    };
    let op = match (leaf_op, sql_op) {
        (DbQueryOp::Update, _) => DbQueryOp::Update,
        (DbQueryOp::Select, _) => DbQueryOp::Select,
        (DbQueryOp::Other, sql) => sql,
        _ => leaf_op,
    };
    Some(FlowEmission::DbQuery {
        entity_name: format!("java.{}", entity),
        operation: op,
    })
}

fn is_jdbc_template_root(name: &str) -> bool {
    matches!(
        name,
        "jdbcTemplate"
            | "jdbc"
            | "namedJdbcTemplate"
            | "namedParameterJdbcTemplate"
            | "jdbcOperations"
    )
}

fn parse_java_sql_entity(
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
// Retrofit `@GET("/x")` / `@POST(...)` etc. Producer HttpCall
// ---------------------------------------------------------------------------

pub(crate) fn detect_retrofit_attribute_emission(
    attr_name: &str,
    first_arg: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    let method = match attr_name {
        "GET" => HttpMethod::Get,
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "PATCH" => HttpMethod::Patch,
        "DELETE" => HttpMethod::Delete,
        "HEAD" => HttpMethod::Head,
        "OPTIONS" => HttpMethod::Options,
        "HTTP" => HttpMethod::Any,
        _ => return None,
    };
    let url = first_arg?.trim();
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
// gRPC-java Stub → RpcCall Producer
// ---------------------------------------------------------------------------

/// `<Service>Grpc.newBlockingStub(channel).method(req)` /
/// `newFutureStub` / `newStub`. Detection: chain root ends with "Grpc",
/// second segment is one of the stub constructors, leaf is the rpc method.
pub(crate) fn detect_java_grpc_stub_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};

    let segs = &chain.segments;
    if segs.len() < 3 {
        return None;
    }
    let root = segs[0].name.as_str();
    if !root.ends_with("Grpc") || root == "Grpc" {
        return None;
    }
    let ctor = segs[1].name.as_str();
    if !matches!(ctor, "newBlockingStub" | "newFutureStub" | "newStub") {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    if matches!(leaf, "newBlockingStub" | "newFutureStub" | "newStub") {
        return None;
    }
    let service = root.strip_suffix("Grpc").unwrap_or(root);
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
// Tests
// ---------------------------------------------------------------------------
