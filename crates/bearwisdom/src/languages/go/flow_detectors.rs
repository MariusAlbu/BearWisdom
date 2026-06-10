// =============================================================================
// go/flow_detectors.rs  —  Per-connector FlowEmission detectors for Go
//
// One detector per producer/consumer family (HTTP, DB query, gRPC, mailer,
// bg-job, MQ, redis-config lookup, gorilla websocket, unix-domain-socket).
// Each `detect_go_*` entry point inspects a chain + call_args (plus
// FileContext where the import surface is load-bearing) and either returns
// a FlowEmission or None.
//
// Lifted from resolve.rs to keep that file scoped to the LanguageResolver
// impl. Helpers private to a single detector (`parse_go_http_pkg_verb`,
// `file_imports_proto_pkg`, etc.) live next to their consumer.
// =============================================================================

use crate::indexer::resolve::engine::FileContext;

// ---------------------------------------------------------------------------
// HTTP Producer detection — net/http + resty
// ---------------------------------------------------------------------------

/// Recognise Go HTTP-client call shapes and emit Producer
/// `NamedChannel { kind: HttpCall, .. }` keyed on the first string-literal arg.
///
/// Shapes handled:
/// - `http.Get(url)` → Get; `http.Post(url, ct, body)` → Post;
///   `http.PostForm(url, ...)` → Post; `http.Head(url)` → Head;
///   `http.NewRequest("METHOD", url, body)` → method from first arg.
/// - resty: `client.R().Get(url)` / `.Post(url)` / etc. — verb leaf,
///   URL as first string arg.
pub(crate) fn detect_go_http_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.is_empty() {
        return None;
    }
    let root = segs.first()?.name.as_str();
    let leaf = segs.last()?.name.as_str();

    // `http.NewRequest("GET", "/x", body)` — method is the first string arg,
    // URL the second. Emit Producer with the parsed verb + URL.
    if root == "http" && leaf == "NewRequest" {
        let (method_arg, url_arg) = match (call_args.first(), call_args.get(1)) {
            (Some(CallArg::StringLit(m)), Some(CallArg::StringLit(u))) => (m.clone(), u.clone()),
            _ => return None,
        };
        if url_arg.is_empty() {
            return None;
        }
        let method = HttpMethod::from_method_name(method_arg.as_str());
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: crate::connectors::url_pattern::normalize(&url_arg),
            role: ChannelRole::Producer,
            method: Some(method),
            streaming: None,
        });
    }

    // `http.Get(url)` / `http.Post(url, ...)` / etc.
    if root == "http" && segs.len() == 2 {
        let method = parse_go_http_pkg_verb(leaf)?;
        let url = match call_args.first()? {
            CallArg::StringLit(s) => s.clone(),
            _ => return None,
        };
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

    // resty `client.R().Get("/x")` — chain has `R` then a verb leaf.
    if segs.iter().any(|s| s.name == "R") {
        let method = parse_resty_verb(leaf)?;
        let url = match call_args.first()? {
            CallArg::StringLit(s) => s.clone(),
            _ => return None,
        };
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

    None
}

fn parse_go_http_pkg_verb(name: &str) -> Option<crate::indexer::resolve::flow_emit::HttpMethod> {
    use crate::indexer::resolve::flow_emit::HttpMethod;
    Some(match name {
        "Get" => HttpMethod::Get,
        "Post" | "PostForm" => HttpMethod::Post,
        "Head" => HttpMethod::Head,
        _ => return None,
    })
}

fn parse_resty_verb(name: &str) -> Option<crate::indexer::resolve::flow_emit::HttpMethod> {
    use crate::indexer::resolve::flow_emit::HttpMethod;
    Some(match name {
        "Get" => HttpMethod::Get,
        "Post" => HttpMethod::Post,
        "Put" => HttpMethod::Put,
        "Patch" => HttpMethod::Patch,
        "Delete" => HttpMethod::Delete,
        "Head" => HttpMethod::Head,
        "Options" => HttpMethod::Options,
        "Execute" => HttpMethod::Any,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// DbQuery — database/sql + gorm
// ---------------------------------------------------------------------------

/// Recognise database/sql and gorm call shapes and emit Producer DbQuery
/// keyed on the entity/table name parsed from the call.
///
/// Shapes handled:
/// - `db.Query("SELECT ... FROM table")` / `db.QueryRow(...)` /
///   `db.Exec(...)` — entity parsed from SQL `FROM`/`UPDATE`/
///   `INSERT INTO`/`DELETE FROM` clause.
/// - gorm `db.First(&user)` / `db.Find(&users)` / `db.Save(&user)` /
///   `db.Create(&user)` / `db.Delete(&user)` — entity from the
///   `&Type{}` composite literal type or the pointer's struct type.
pub(crate) fn detect_go_db_query_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    let leaf = chain.segments.last()?.name.as_str();

    // database/sql — SQL text first arg.
    let sql_op = match leaf {
        "Query" | "QueryRow" | "QueryContext" | "QueryRowContext" => Some(DbQueryOp::Select),
        "Exec" | "ExecContext" => Some(DbQueryOp::Other),
        _ => None,
    };
    if let Some(default_op) = sql_op {
        if let Some(CallArg::StringLit(sql)) = call_args.first() {
            if let Some((entity, op)) = parse_sql_entity(sql, default_op) {
                return Some(FlowEmission::DbQuery {
                    entity_name: format!("go.{}", entity),
                    operation: op,
                });
            }
        }
        return None;
    }

    // gorm — entity is the first non-empty Ident arg (composite-literal
    // address-of or pointer variable).
    let gorm_op = match leaf {
        "First" | "Find" | "Take" | "Last" | "Where" | "Preload" | "Joins" | "Select"
        | "Distinct" | "Pluck" | "Count" | "Scan" | "Scopes" => Some(DbQueryOp::Select),
        "Create" | "CreateInBatches" => Some(DbQueryOp::Insert),
        "Save" | "Update" | "Updates" | "UpdateColumn" | "UpdateColumns" => Some(DbQueryOp::Update),
        "Delete" => Some(DbQueryOp::Delete),
        "FirstOrCreate" | "Upsert" => Some(DbQueryOp::Upsert),
        _ => None,
    };
    if let Some(op) = gorm_op {
        let entity = call_args.iter().find_map(|a| match a {
            CallArg::Ident(name) if is_pascal_case_first_go(name) => Some(name.clone()),
            _ => None,
        })?;
        return Some(FlowEmission::DbQuery {
            entity_name: format!("go.{}", strip_pointer_prefix(&entity)),
            operation: op,
        });
    }

    None
}

fn is_pascal_case_first_go(name: &str) -> bool {
    name.trim_start_matches('*')
        .trim_start_matches('&')
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_uppercase())
}

fn strip_pointer_prefix(name: &str) -> String {
    name.trim_start_matches('*')
        .trim_start_matches('&')
        .trim_start_matches("[]")
        .to_string()
}

/// Parse an SQL string to identify the entity (table) name and operation.
/// Handles the common SELECT / UPDATE / INSERT INTO / DELETE FROM shapes.
fn parse_sql_entity(
    sql: &str,
    fallback_op: crate::indexer::resolve::flow_emit::DbQueryOp,
) -> Option<(String, crate::indexer::resolve::flow_emit::DbQueryOp)> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    let upper = sql.trim().to_ascii_uppercase();
    // Locate the verb + entity-token slot for each common operation.
    let (op, marker, after_marker): (DbQueryOp, &str, usize) =
        if let Some(i) = upper.find("UPDATE ") {
            (DbQueryOp::Update, " UPDATE ", i + 7)
        } else if let Some(i) = upper.find("INSERT INTO ") {
            (DbQueryOp::Insert, " INSERT INTO ", i + 12)
        } else if let Some(i) = upper.find("DELETE FROM ") {
            (DbQueryOp::Delete, " DELETE FROM ", i + 12)
        } else if let Some(i) = upper.find(" FROM ") {
            (DbQueryOp::Select, " FROM ", i + 6)
        } else if upper.starts_with("FROM ") {
            (DbQueryOp::Select, "FROM ", 5)
        } else {
            return None;
        };
    let _ = marker;
    let entity_slice = sql.get(after_marker..)?;
    let entity = entity_slice
        .split_whitespace()
        .next()?
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
        .to_string();
    if entity.is_empty() {
        return None;
    }
    let final_op = match op {
        DbQueryOp::Select => fallback_op_or(op, fallback_op),
        _ => op,
    };
    Some((entity, final_op))
}

fn fallback_op_or(
    primary: crate::indexer::resolve::flow_emit::DbQueryOp,
    fallback: crate::indexer::resolve::flow_emit::DbQueryOp,
) -> crate::indexer::resolve::flow_emit::DbQueryOp {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    // For `Exec` calls the SQL might contain a SELECT (unusual), but the
    // Go-side intent is a write operation. Prefer the SQL-parsed op for
    // explicit verbs (UPDATE/INSERT/DELETE) and fall back to the call
    // shape's hint for SELECTs (which would only land here for Exec).
    match (primary, fallback) {
        (DbQueryOp::Select, DbQueryOp::Other) => DbQueryOp::Select,
        (DbQueryOp::Select, fb) => fb,
        (p, _) => p,
    }
}

// ---------------------------------------------------------------------------
// gRPC Producer — `client.<Service>.<Method>(ctx, req)` chains
// ---------------------------------------------------------------------------

/// Recognise gRPC client chains and emit Producer `NamedChannel { kind: RpcCall, .. }`
/// keyed on `<service>/<method>` (canonical lowercase form). Fires only
/// when the file imports a gRPC-generated package — by convention the
/// import path ends in `pb`/`grpc` or contains `proto`.
pub(crate) fn detect_go_grpc_chain_emission(
    chain: &crate::types::MemberChain,
    file_ctx: &FileContext,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };

    // Require the file to import a generated proto / pb package — the
    // `google.golang.org/grpc` package alone is gRPC plumbing (Dial,
    // ServerOption, etc.), not service calls.
    if !file_imports_proto_pkg(file_ctx) {
        return None;
    }
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs.first()?.name.as_str();
    let leaf = segs.last()?.name.as_str();

    // Drop chains rooted at known stdlib / grpc-setup packages — these are
    // never service calls (fmt.Println, grpc.WithInsecure, context.Background,
    // etc.).
    if is_go_stdlib_or_setup_root(root) {
        return None;
    }
    // Drop lifecycle / factory / generic-leaf names that aren't gRPC
    // service methods. The `New*` factories are detected by prefix —
    // they construct the client, they don't call the service.
    if is_go_grpc_non_method_leaf(leaf) || leaf.starts_with("New") {
        return None;
    }
    // gRPC service method names are conventionally PascalCase with a
    // verb prefix (`Get`, `List`, `Create`, `Update`, `Delete`, `Stream`,
    // `Watch`, `Subscribe`, `Publish`, `Send`). Filtering on these prefixes
    // keeps simple-PascalCase struct-field accesses (`resp.Name`) out of
    // the rpc_call stream.
    if !looks_like_grpc_method_name(leaf) {
        return None;
    }
    // For 2-segment chains the root must look like a gRPC client binding —
    // either a `Client`-suffixed type identifier or a camelCase local
    // variable (`client`, `userClient`). Rejects single-segment-rooted
    // package calls.
    if segs.len() == 2 && !root.ends_with("Client") && !is_camel_case_local(root) {
        return None;
    }

    let service = if segs.len() >= 3 {
        segs[segs.len() - 2].name.as_str()
    } else {
        segs[0].name.as_str()
    };
    let service_norm = strip_go_service_suffix(service);
    if service_norm.is_empty() {
        return None;
    }
    use crate::indexer::resolve::flow_emit::StreamKind;
    let streaming = Some(StreamKind::from_method_name(leaf));
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: format!(
            "{}/{}",
            service_norm.to_ascii_lowercase(),
            leaf.to_ascii_lowercase()
        ),
        role: ChannelRole::Producer,
        method: Some(HttpMethod::Any),
        streaming,
    })
}

fn file_imports_proto_pkg(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let m = imp
            .module_path
            .as_deref()
            .unwrap_or(imp.imported_name.as_str());
        // Only count generated-proto packages. `google.golang.org/grpc` is
        // intentionally excluded — it's the plumbing, not the service.
        let last_seg = m.rsplit('/').next().unwrap_or(m);
        m.contains("/proto/")
            || m.contains("/pb/")
            || m.contains("/protobuf/")
            || last_seg.ends_with("pb")
            || last_seg.ends_with("_pb")
            || last_seg.ends_with("Pb")
    })
}

fn is_go_stdlib_or_setup_root(name: &str) -> bool {
    matches!(
        name,
        "fmt"
            | "os"
            | "io"
            | "log"
            | "errors"
            | "context"
            | "time"
            | "sync"
            | "strings"
            | "strconv"
            | "bytes"
            | "bufio"
            | "json"
            | "yaml"
            | "http"
            | "net"
            | "url"
            | "grpc"
            | "metadata"
            | "codes"
            | "status"
            | "credentials"
            | "reflection"
            | "health"
    )
}

/// Reject leaves that don't look like gRPC RPC method names. Matches the
/// common verb prefixes for service methods (`Get`, `List`, `Create`,
/// `Update`, `Delete`, `Stream`, `Watch`, `Subscribe`, `Send`, `Publish`,
/// `Search`, `Find`, `Query`, `Mutate`, `Push`, `Pull`, `Insert`, `Remove`,
/// `Add`, `Set`, `Fetch`).
fn looks_like_grpc_method_name(name: &str) -> bool {
    if name.len() < 3 {
        return false;
    }
    let prefixes = [
        "Get",
        "List",
        "Create",
        "Update",
        "Delete",
        "Stream",
        "Watch",
        "Subscribe",
        "Send",
        "Publish",
        "Search",
        "Find",
        "Query",
        "Mutate",
        "Push",
        "Pull",
        "Insert",
        "Remove",
        "Add",
        "Set",
        "Fetch",
        "Run",
        "Exec",
        "Process",
        "Apply",
        "Validate",
        "Authenticate",
        "Authorize",
        "Sync",
        "Replicate",
        "Snapshot",
    ];
    prefixes.iter().any(|p| name.starts_with(p))
}

fn is_camel_case_local(name: &str) -> bool {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        // Single-letter lowercase identifier (`c`, `s`) — accept.
        (Some(c0), None) => c0.is_ascii_lowercase(),
        // Multi-char camelCase: lowercase first char + at least one more letter.
        (Some(c0), Some(_)) => c0.is_ascii_lowercase() && name.chars().any(|c| c.is_alphanumeric()),
        _ => false,
    }
}

fn strip_go_service_suffix(name: &str) -> String {
    for suffix in ["ServiceClient", "Client", "Service"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            if !stripped.is_empty() {
                return stripped.to_string();
            }
        }
    }
    name.to_string()
}

fn is_go_grpc_non_method_leaf(name: &str) -> bool {
    matches!(
        name,
        "Close" | "Invoke" | "NewClient" | "Dial" | "DialContext"
    )
}

// ---------------------------------------------------------------------------
// Mailer Producer — gomail + stdlib smtp
// ---------------------------------------------------------------------------

/// `gomail.NewDialer(...).DialAndSend(msg)` and `smtp.SendMail(...)`.
pub(crate) fn detect_go_mailer_emission(
    chain: &crate::types::MemberChain,
    _call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.is_empty() {
        return None;
    }
    let root = segs.first()?.name.as_str();
    let leaf = segs.last()?.name.as_str();

    // stdlib `smtp.SendMail`.
    if root == "smtp" && leaf == "SendMail" {
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::Mailer,
            name: "go.smtp".to_string(),
            role: ChannelRole::Producer,
            method: None,
            streaming: None,
        });
    }

    // gomail dialer-style chain. The chain root carries the dialer var (often
    // `d` / `dialer` / `gomail`). Match on the leaf alone — `DialAndSend`,
    // `DialAndSendMessage`, and the v2 `Send` are unambiguous for the lib.
    if matches!(leaf, "DialAndSend" | "DialAndSendContext") {
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::Mailer,
            name: "go.gomail".to_string(),
            role: ChannelRole::Producer,
            method: None,
            streaming: None,
        });
    }
    None
}

// ---------------------------------------------------------------------------
// BgJob Producer — asynq + machinery
// ---------------------------------------------------------------------------

/// `client.Enqueue(task)` from `hibiken/asynq`, `server.SendTask(...)` from
/// `RichardKnop/machinery`. Leaf-name only; the root is usually a local
/// `client` / `srv` / `enqueuer` ident.
pub(crate) fn detect_go_bgjob_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    let kind_name = match leaf {
        "Enqueue" | "EnqueueContext" | "EnqueueIn" => "asynq",
        "SendTask" | "SendTaskWithContext" => "machinery",
        _ => return None,
    };
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: format!("go.{}", kind_name),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    })
}

// ---------------------------------------------------------------------------
// MessageQueue Producer — Kafka (sarama, kafka-go) + NATS
// ---------------------------------------------------------------------------

/// Recognise Kafka (sarama `SendMessage`, kafka-go `WriteMessages`) and NATS
/// (`nc.Publish(subject, data)`). Emits `NamedChannel { kind: MessageQueue, .. }`.
pub(crate) fn detect_go_mq_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let leaf = segs.last()?.name.as_str();

    // sarama / kafka-go.
    if matches!(leaf, "SendMessage" | "SendMessages" | "WriteMessages") {
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::MessageQueue,
            name: "go.kafka".to_string(),
            role: ChannelRole::Producer,
            method: None,
            streaming: None,
        });
    }
    // NATS — `nc.Publish(subject, data)`. The subject is the first string lit.
    if leaf == "Publish" {
        let subject = call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        })?;
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::MessageQueue,
            name: subject,
            role: ChannelRole::Producer,
            method: None,
            streaming: None,
        });
    }
    None
}

// ---------------------------------------------------------------------------
// Redis ConfigLookup — go-redis Get
// ---------------------------------------------------------------------------

/// `rdb.Get(ctx, "key")` / `rdb.Get(ctx, "key").Result()` from `go-redis`.
/// Emits `ConfigLookup { key: "redis:KEY" }` so cache lookups cluster with
/// config-key reads.
pub(crate) fn detect_go_redis_config_lookup(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::types::CallArg;

    let segs = &chain.segments;
    let leaf = segs.last()?.name.as_str();
    if leaf != "Get" && leaf != "GetEx" {
        return None;
    }
    // go-redis convention: first arg is `ctx`, second is the key string.
    let key = match (call_args.first(), call_args.get(1)) {
        (Some(CallArg::Ident(c)), Some(CallArg::StringLit(k)))
            if matches!(c.as_str(), "ctx" | "context" | "c" | "rctx") =>
        {
            k.clone()
        }
        _ => return None,
    };
    if key.is_empty() {
        return None;
    }
    Some(FlowEmission::ConfigLookup {
        key: format!("redis:{}", key),
    })
}

// ---------------------------------------------------------------------------
// IPC — Unix domain socket Listen / Dial
// ---------------------------------------------------------------------------

/// gorilla/websocket `upgrader.Upgrade(w, r, nil)` and
/// nhooyr/websocket `websocket.Accept(w, r, opts)`. Single-ended Consumer.
pub(crate) fn detect_go_gorilla_ws_consumer(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    let matches_gorilla = matches!(root, "upgrader" | "Upgrader") && leaf == "Upgrade";
    let matches_nhooyr = root == "websocket" && leaf == "Accept";
    if !matches_gorilla && !matches_nhooyr {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: if matches_gorilla {
            "go.gorilla.ws"
        } else {
            "go.nhooyr.ws"
        }
        .to_string(),
        role: ChannelRole::Consumer,
        method: None,
        streaming: None,
    })
}

/// `net.Listen("unix", path)` → IpcCall Consumer keyed on the path.
/// `net.Dial("unix", path)` → IpcCall Producer keyed on the path.
pub(crate) fn detect_go_uds_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.len() != 2 {
        return None;
    }
    let root = segs.first()?.name.as_str();
    let leaf = segs.last()?.name.as_str();
    if root != "net" {
        return None;
    }
    let role = match leaf {
        "Listen" | "ListenUnix" => ChannelRole::Consumer,
        "Dial" | "DialUnix" => ChannelRole::Producer,
        _ => return None,
    };
    let (network_arg, path_arg) = match (call_args.first(), call_args.get(1)) {
        (Some(CallArg::StringLit(n)), Some(CallArg::StringLit(p))) => (n.as_str(), p.as_str()),
        _ => return None,
    };
    if !network_arg.starts_with("unix") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::IpcCall,
        name: path_arg.to_string(),
        role,
        method: None,
        streaming: None,
    })
}
