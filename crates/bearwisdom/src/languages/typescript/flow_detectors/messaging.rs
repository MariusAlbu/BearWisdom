// =============================================================================
// languages/typescript/flow_detectors/messaging.rs — async messaging detectors
//
// Recognises FlowEmissions from asynchronous-messaging call chains:
//
//   * Message queues — RabbitMQ (amqplib), Kafka (kafkajs), NATS, Redis pub/sub
//   * Background jobs — BullMQ, Bull, Agenda, Bee-Queue
//   * RPC clients — gRPC, Connect / Twirp, NestJS microservice clients
//   * Mailer — Nodemailer, NestJS `@nestjs-modules/mailer`, SES, MailerSend
//
// Shared helper: `canonical_rpc_key` produces the lowercase
// `<service>/<method>` pairing key used by both the RPC chain detectors
// here AND the gRPC decorator detectors in `decorators.rs` (re-exported
// via `pub(super)`).
// =============================================================================

use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::indexer::resolve::flow_emit::{
    ChannelRole, FlowEmission, NamedChannelKind, StreamKind,
};
use crate::types::CallArg;

use super::db::is_pascal_case_first;
use super::first_arg_string;

pub(crate) const BGJOB_QUEUE_BINDING_KEY: &str = "__ts_bgjob_queue_binding__:";

pub(crate) fn detect_mq_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[CallArg],
    file_ctx: &FileContext,
) -> Option<FlowEmission> {
    if !file_imports_mq_library(file_ctx) {
        return None;
    }
    let leaf = chain.segments.last()?;
    let role = parse_mq_verb(leaf.name.as_str())?;
    let name = first_arg_string(call_args);
    if name.is_empty() {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::MessageQueue,
        name,
        role,
        method: None,
    streaming: None,
    })
}

fn file_imports_mq_library(file_ctx: &FileContext) -> bool {
    file_ctx
        .imports
        .iter()
        .any(|imp| imp.module_path.as_deref().map_or(false, is_mq_library))
}

fn is_mq_library(pkg: &str) -> bool {
    let root = if pkg.starts_with('@') {
        let mut parts = pkg.splitn(3, '/');
        match (parts.next(), parts.next()) {
            (Some(scope), Some(name)) => &pkg[..scope.len() + 1 + name.len()],
            _ => pkg,
        }
    } else {
        pkg.split('/').next().unwrap_or(pkg)
    };
    matches!(
        root,
        "nats"
            | "ioredis"
            | "redis"
            | "amqplib"
            | "kafkajs"
            | "mqtt"
            | "@aws-sdk/client-sqs"
            | "@google-cloud/pubsub"
            | "@nestjs/microservices"
    )
}

fn parse_mq_verb(name: &str) -> Option<ChannelRole> {
    Some(match name {
        "publish" => ChannelRole::Producer,
        "subscribe" | "consume" => ChannelRole::Consumer,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Background-job libraries — Producer / Consumer NamedChannel BgJob
// ---------------------------------------------------------------------------

/// Inspect a chain that may be a background-job Producer or Consumer call and
/// emit a `NamedChannel { kind: BgJob, .. }` keyed on `queueName/jobName` when the
/// queue binding is known, falling back to `jobName` alone otherwise. Fires
/// only when the file imports a recognised background-job library (`bullmq`,
/// `bull`, `agenda`, `bee-queue`).
///
/// Patterns recognised:
/// - Producer (method): `queue.add('jobName', data)` (BullMQ / Bull),
///   `agenda.now('jobName', ...)` / `agenda.schedule(...)` /
///   `agenda.every(...)` (Agenda), `queue.createJob(data)` (bee-queue —
///   data is the job payload; queueName alone is the pairing key).
/// - Producer (constructor inside chain): `new Worker('queueName', ...)` is
///   surfaced through the single-segment constructor branch below.
/// - Consumer (constructor): `new Worker('queueName', processor)` (BullMQ).
///   `new Queue('queueName', ...)` is captured by the file-context pre-pass
///   for binding but does not itself emit a flow edge.
/// - Consumer (method): `queue.process('jobName', h)` (Bull),
///   `agenda.define('jobName', h)` (Agenda).
/// - Consumer (lifecycle): `worker.on('completed' | 'failed' | 'active' |
///   'progress' | 'stalled' | 'drained' | 'waiting', h)` (BullMQ Worker
///   lifecycle event listener — only emitted when the chain root resolves
///   to a `new Worker(...)` binding captured by the pre-pass; without that
///   binding, every `.on(...)` call would match and produce false positives.)
///
/// Pairing key shape:
/// - When the chain root has a known queue binding: `queueName/jobName`.
///   Both `queue.add('jobName', …)` and `queue.process('jobName', …)`
///   produce the same key when `queue` was bound to `new Queue('queueName')`.
/// - When the constructor itself is the entry point (`new Worker('q', …)`):
///   `queueName/*` so the Consumer pairs across all jobs in that queue.
/// - Otherwise: `jobName` alone — best-effort pairing for queue calls whose

pub(crate) fn detect_bgjob_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[CallArg],
    file_ctx: &FileContext,
) -> Option<FlowEmission> {
    if !file_imports_bgjob_library(file_ctx) {
        return None;
    }
    let leaf = chain.segments.last()?;
    let root_name = chain.segments.first()?.name.as_str();

    // Constructor case — single-segment chain whose name is a recognised
    // BgJob Consumer constructor (currently only BullMQ's `Worker`). The
    // queue name is the first string argument; jobName is `*` because the
    // worker handles every job in the queue.
    if chain.segments.len() == 1 {
        let role = parse_bgjob_constructor(leaf.name.as_str())?;
        let queue_name = first_arg_string(call_args);
        if queue_name.is_empty() {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::BgJob,
            name: format!("{}/*", queue_name),
            role,
            method: None,
        streaming: None,
        });
    }

    // Lifecycle-event method case — `worker.on('completed', h)` where the
    // chain root is bound to a `new Worker('q', …)` constructor captured by
    // the file-context pre-pass. The event name itself is not the pairing
    // key; the queue name is, with a `*` job-name wildcard so the listener
    // matches the same shape as the constructor.
    if leaf.name == "on" {
        let event = first_arg_string(call_args);
        if !is_bgjob_lifecycle_event(event.as_str()) {
            return None;
        }
        let queue_name = lookup_bgjob_queue_binding(root_name, file_ctx)?;
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::BgJob,
            name: format!("{}/*", queue_name),
            role: ChannelRole::Consumer,
            method: None,
        streaming: None,
        });
    }

    // Method case — last segment is a recognised BgJob verb.
    let role = parse_bgjob_verb(leaf.name.as_str())?;
    let first_arg = first_arg_string(call_args);
    let queue_binding = lookup_bgjob_queue_binding(root_name, file_ctx);

    // bee-queue: `queue.createJob(data)` carries no jobName — the queue name
    // itself is the only pairing key. The data argument is captured as
    // `CallArg::Other` and we don't bother trying to peek inside.
    if leaf.name == "createJob" {
        let queue_name = queue_binding?;
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::BgJob,
            name: format!("{}/*", queue_name),
            role,
            method: None,
        streaming: None,
        });
    }

    // For every other verb the first string arg must be the job name.
    if first_arg.is_empty() {
        return None;
    }
    let name = match queue_binding {
        Some(queue) => format!("{}/{}", queue, first_arg),
        None => first_arg,
    };
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name,
        role,
        method: None,
    streaming: None,
    })
}

/// Resolve a chain-root variable name to its bound queue name via the
/// synthetic ImportEntry stashed by the file-context pre-pass.
fn lookup_bgjob_queue_binding(var_name: &str, file_ctx: &FileContext) -> Option<String> {
    if var_name.is_empty() {
        return None;
    }
    let key_len = BGJOB_QUEUE_BINDING_KEY.len();
    file_ctx.imports.iter().find_map(|imp| {
        if imp.imported_name.len() > key_len
            && imp.imported_name.starts_with(BGJOB_QUEUE_BINDING_KEY)
            && &imp.imported_name[key_len..] == var_name
        {
            imp.module_path.clone()
        } else {
            None
        }
    })
}

/// BullMQ Worker lifecycle events: the small set of identifiers passed to
/// `worker.on(event, handler)` for status reporting. Other `.on(...)` calls
/// (DOM, EventEmitter, etc.) are intentionally rejected by this whitelist.
fn is_bgjob_lifecycle_event(name: &str) -> bool {
    matches!(
        name,
        "completed" | "failed" | "active" | "progress" | "stalled" | "drained" | "waiting"
    )
}

fn file_imports_bgjob_library(file_ctx: &FileContext) -> bool {
    file_ctx
        .imports
        .iter()
        .any(|imp| imp.module_path.as_deref().map_or(false, is_bgjob_library))
}

fn is_bgjob_library(pkg: &str) -> bool {
    let root = if pkg.starts_with('@') {
        let mut parts = pkg.splitn(3, '/');
        match (parts.next(), parts.next()) {
            (Some(scope), Some(name)) => &pkg[..scope.len() + 1 + name.len()],
            _ => pkg,
        }
    } else {
        pkg.split('/').next().unwrap_or(pkg)
    };
    matches!(root, "bullmq" | "bull" | "agenda" | "bee-queue")
}

/// Recognised constructor names that bind a Consumer to a queue. The first
/// argument is the queue name. BullMQ's `Worker` is the canonical case.
fn parse_bgjob_constructor(name: &str) -> Option<ChannelRole> {
    match name {
        "Worker" => Some(ChannelRole::Consumer),
        _ => None,
    }
}

/// Verbs that designate a queue- or job-keyed BgJob call when the file
/// imports a recognised broker. Producer verbs enqueue; Consumer verbs
/// register a handler.
fn parse_bgjob_verb(name: &str) -> Option<ChannelRole> {
    Some(match name {
        // Producer enqueues — BullMQ/Bull `queue.add('jobName', data)`,
        // Agenda `agenda.now('jobName', ...)` / `.schedule(...)` / `.every(...)`,
        // bee-queue `queue.createJob(data)` (queue-keyed, no jobName).
        "add" | "now" | "schedule" | "every" | "createJob" => ChannelRole::Producer,
        // Consumer registers — Bull `queue.process('jobName', h)`,
        // Agenda `agenda.define('jobName', h)`.
        "process" | "define" => ChannelRole::Consumer,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// gRPC / Connect chains — Producer / Consumer NamedChannel RpcCall
// ---------------------------------------------------------------------------

/// Inspect a chain that may be a gRPC / Connect / nice-grpc / ts-proto RPC
/// call site and emit a `NamedChannel { kind: RpcCall, .. }` keyed on
/// `serviceName/methodName`. Fires only when the file imports a recognised
/// gRPC library.
///
/// Recognised patterns:
/// - Producer (3+ segments): `client.serviceName.methodName(req)` — Connect
///   and nice-grpc clients expose nested service routers; service = the
///   second-to-last segment, method = the leaf.
/// - Producer (2 segments): `serviceClient.methodName(req)` — ts-proto and
///   `@grpc/grpc-js` generated clients flatten the service into the root
///   identifier; service = the root segment, method = the leaf.
/// - Consumer (2 segments): `server.addService(SvcDef, { method: handler })`
///   — `@grpc/grpc-js` dynamic registration. The first arg is a service
///   definition identifier (PascalCase). Emitted as `<service>/*` so the

pub(crate) fn detect_rpc_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[CallArg],
    file_ctx: &FileContext,
) -> Option<FlowEmission> {
    if !file_imports_rpc_library(file_ctx) {
        return None;
    }
    if chain.segments.len() < 2 {
        return None;
    }
    let leaf = chain.segments.last()?;
    let leaf_name = leaf.name.as_str();

    // Consumer: `server.addService(SvcDef, { method1: h, method2: h })`.
    // When the second arg's object-literal keys are captured we emit one
    // Consumer per registered method (`<service>/<method>`); when only the
    // PascalCase service definition is recoverable we fall back to a single
    // `<service>/*` wildcard Consumer that the pairer matches against every
    // method of the same service.
    if leaf_name == "addService" {
        let Some(CallArg::Ident(ident)) = call_args.first() else {
            return None;
        };
        if !is_pascal_case_first(ident) {
            return None;
        }
        let service = strip_service_suffix(ident);
        // The chain detector returns a single emission, so multi-method
        // expansion is performed at the trait dispatch site via
        // `detect_addservice_object_keys`. Here we always emit the wildcard
        // form; the multi-emission path is engaged when object keys are
        // available.
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::RpcCall,
            name: canonical_rpc_key(&service, "*"),
            role: ChannelRole::Consumer,
            method: None,
        streaming: None,
        });
    }

    // Producer: client method call. Reject members whose leaf name is a
    // promise/observable lifecycle method or a stream control method —
    // gRPC clients return streams/promises and callers chain `.then`,
    // `.catch`, `.subscribe`, etc. against them, which must not look like
    // RPC method calls.
    if is_rpc_non_method_leaf(leaf_name) {
        return None;
    }

    let segs = &chain.segments;
    let (service, method) = if segs.len() >= 3 {
        // `client.serviceName.methodName(...)` → (serviceName, methodName)
        (segs[segs.len() - 2].name.as_str(), leaf_name)
    } else {
        // `serviceClient.methodName(...)` → strip a trailing `Client` /
        // `Service` suffix from the root for a cleaner pairing key.
        (segs[0].name.as_str(), leaf_name)
    };
    let service_norm = strip_service_suffix(service);
    if service_norm.is_empty() || method.is_empty() {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: canonical_rpc_key(&service_norm, method),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

pub(super) fn file_imports_rpc_library(file_ctx: &FileContext) -> bool {
    file_ctx
        .imports
        .iter()
        .any(|imp| imp.module_path.as_deref().map_or(false, is_rpc_library))
}

fn is_rpc_library(pkg: &str) -> bool {
    let root = if pkg.starts_with('@') {
        let mut parts = pkg.splitn(3, '/');
        match (parts.next(), parts.next()) {
            (Some(scope), Some(name)) => &pkg[..scope.len() + 1 + name.len()],
            _ => pkg,
        }
    } else {
        pkg.split('/').next().unwrap_or(pkg)
    };
    matches!(
        root,
        "@connectrpc/connect"
            | "@connectrpc/connect-web"
            | "@connectrpc/connect-node"
            | "@bufbuild/connect"
            | "@bufbuild/connect-web"
            | "@bufbuild/connect-node"
            | "nice-grpc"
            | "nice-grpc-web"
            | "@grpc/grpc-js"
            | "@grpc/proto-loader"
            | "ts-proto"
    )
}

/// Strip a trailing `Client`, `Service`, or `ServiceClient` suffix so a
/// generated `UserServiceClient.getUser` chain pairs with a `@GrpcMethod(
/// 'UserService', 'getUser')` decorator. Returns the input unchanged when
/// no suffix matches.
pub(super) fn strip_service_suffix(name: &str) -> String {
    for suffix in ["ServiceClient", "Client", "Service"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            if !stripped.is_empty() {
                return stripped.to_string();
            }
        }
    }
    name.to_string()
}

/// Leaf names that are common JS/TS lifecycle methods, not gRPC RPC methods.
/// Reject them so `client.users.then(...)` doesn't get treated as an RPC call.
fn is_rpc_non_method_leaf(name: &str) -> bool {
    matches!(
        name,
        "then"
            | "catch"
            | "finally"
            | "subscribe"
            | "pipe"
            | "toPromise"
            | "cancel"
            | "abort"
            | "close"
            | "end"
            | "on"
            | "off"
            | "once"
            | "addListener"
            | "removeListener"
            | "emit"
    )
}

// ---------------------------------------------------------------------------
// Mailer / templated-email Producer chains — NamedChannel Mailer
// ---------------------------------------------------------------------------

/// Inspect a chain that may be a mailer Producer call and emit a Consumer-
/// pairing-keyed `NamedChannel { kind: Mailer, .. }`. The key is the template
/// name extracted from the call's first object-literal argument:
/// `transport.sendMail({ template: 'welcome', ... })` keys on `welcome`;
/// `sgMail.send({ templateId: 'd-12345', ... })` keys on `d-12345`.
///
/// Recognised verbs: `sendMail`, `send`, `sendEmail`, `sendTemplate`. The
/// detector only fires when (a) the leaf segment matches one of those AND
/// (b) the first argument is an `ObjectKeys` literal containing a
/// `template` or `templateId` key with a string-literal value. Calls whose

pub(crate) fn detect_mailer_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[CallArg],
    _file_ctx: &FileContext,
) -> Option<FlowEmission> {
    let leaf = chain.segments.last()?;
    if !matches!(
        leaf.name.as_str(),
        "sendMail" | "send" | "sendEmail" | "sendTemplate"
    ) {
        return None;
    }
    // Preferred: explicit template name via `template` / `templateId` /
    // `template_id` object key. Pair key is the template name.
    if let Some(template) = call_args
        .first()
        .and_then(|a| {
            object_string_value(a, "template")
                .or_else(|| object_string_value(a, "templateId"))
                .or_else(|| object_string_value(a, "template_id"))
        })
        .filter(|t| !t.is_empty())
    {
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::Mailer,
            name: template,
            role: ChannelRole::Producer,
            method: None,
        streaming: None,
        });
    }
    // Fallback: the chain root identifies a library-named mailer client
    // (Resend, SendGrid, Postmark, Mailgun) that does not require a static
    // template key — `resend.emails.send({...})`, `sgMail.send({...})`.
    // Generic local var names (`transport`, `mailer`, `mailService`) are
    // deliberately excluded so the strict-template-key behaviour is
    // preserved for nodemailer / Mailer-service shapes; pairing those
    // through a wildcard would cluster every unrelated mailer call into
    // one bucket.
    let root_name = chain.segments.first().map(|s| s.name.as_str()).unwrap_or("");
    let is_library_root = matches!(
        root_name,
        "resend" | "sgMail" | "sendgrid" | "postmark" | "mailgun" | "Resend"
    );
    if !is_library_root {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::Mailer,
        name: format!("ts.{}", root_name),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

/// Return the string-literal value bound to `key` inside a `CallArg::ObjectKeys`
/// argument. Returns `None` when the arg isn't an object literal, the key is
/// absent, or the value isn't a captured string literal.
fn object_string_value(arg: &CallArg, key: &str) -> Option<String> {
    let CallArg::ObjectKeys(pairs) = arg else {
        return None;
    };
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.clone())
}

// ---------------------------------------------------------------------------
// tRPC client Producer detection — NamedChannel HttpCall
// ---------------------------------------------------------------------------

/// Recognise tRPC client chains and emit a Producer HttpCall keyed on the
/// implicit HTTP URL `/api/trpc/<group>.<procedure>`.
///
/// Chain shape: `trpc.<group>.<procedure>.<verb>(...)` where:
/// - root segment is `trpc` (or `api`, `client` when imported from `@trpc/...`)
/// - leaf segment is one of `useQuery`, `useMutation`, `useSubscription`,
///   `useInfiniteQuery`, `query`, `mutate`, `fetch`, `prefetch`,
///   `prefetchQuery`, `useSuspenseQuery` (covers React Query, vanilla
///   client, SSR helper, and TanStack Query integrations).
/// - intermediate segments form the dotted procedure path.
///
/// Files must import a tRPC client module (`@trpc/client`, `@trpc/react-query`,
/// `@trpc/next`, or a local `trpc` module). The file-import test catches
/// both the canonical `import { trpc } from '@/trpc/client'` shape and
/// the deeper `@trpc/...` package imports.

/// Canonical pairing key for RPC names: lowercase `<service>/<method>`.
/// Lowercasing is the only way to pair a `client.userService.getUser` Connect
/// chain (camelCase service) with a `@GrpcMethod('UserService', 'GetUser')`
/// decorator (PascalCase, proto-declared names) without relying on side-band
/// schema knowledge. The wildcard suffix `*` is preserved as-is.
pub(crate) fn canonical_rpc_key(service: &str, method: &str) -> String {
    let svc = service.to_ascii_lowercase();
    let m = if method == "*" {
        "*".to_string()
    } else {
        method.to_ascii_lowercase()
    };
    format!("{}/{}", svc, m)
}