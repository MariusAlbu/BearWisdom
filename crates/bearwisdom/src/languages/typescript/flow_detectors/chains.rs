// =============================================================================
// languages/typescript/flow_detectors/chains.rs — chain-based detectors
//
// Recognises FlowEmissions that fall out of TypeScript call chains:
// HTTP clients (axios/fetch/ofetch), WebSocket emitters, Electron IPC,
// Tauri invoke, GraphQL operations, tRPC endpoints, `@nestjs/config` and
// feature-flag reads, plus knex / TypeORM migration call chains and cron /
// CLI registrations recognised inside the same dispatcher.
//
// The dispatcher `detect_chain_flow_emission` is the super-detector;
// every other `detect_*` here handles a more specific chain pattern that
// the dispatcher would otherwise miss.
// =============================================================================

use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::indexer::resolve::flow_emit::{
    ChannelRole, FlowEmission, HttpMethod, MigrationDirection, NamedChannelKind,
};
use crate::types::CallArg;

use super::super::predicates;
use super::db::detect_db_query_emission_with_imports;
use super::first_arg_string;
use super::messaging::{
    detect_bgjob_chain_emission, detect_mailer_chain_emission, detect_mq_chain_emission,
    detect_rpc_chain_emission,
};

pub(crate) fn parse_gql_operation(body: &str) -> Option<String> {
    let trimmed = body.trim();
    let rest = trimmed
        .strip_prefix("query")
        .or_else(|| trimmed.strip_prefix("mutation"))
        .or_else(|| trimmed.strip_prefix("subscription"))?;
    let op = if trimmed.starts_with("query") {
        "query"
    } else if trimmed.starts_with("mutation") {
        "mutation"
    } else {
        "subscription"
    };
    // Skip optional whitespace, then take the operation name (word chars).
    let rest = rest.trim_start();
    // Operation name is optional in GraphQL but always present when named.
    if rest.is_empty() || !rest.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_') {
        // Anonymous operation — use the op kind as the name key.
        return Some(format!("{op}:__anon__"));
    }
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    Some(format!("{op}:{name}"))
}

/// Inspect `chain_ref` and `file_ctx` to detect whether the chain root resolves
/// to an HTTP-client module, a Socket.IO client, a Tauri/Electron IPC call,
/// a migration call, a scheduled job, or a CLI command registration.
///
/// Recognition is chain-root-import–based, not symbol-name–based:
/// 1. Look at the first chain segment's name.
/// 2. Find the import entry in `file_ctx` that binds that name.
/// 3. Check whether the import source is a well-known package using the
///    `is_http_client_module` et al. predicates.
///
/// `call_args` is used to extract the URL pattern, IPC command name, or cron
/// expression from the first literal argument.
pub(crate) fn detect_chain_flow_emission(
    chain_ref: &crate::types::MemberChain,
    call_args: &[CallArg],
    file_ctx: &FileContext,
) -> Option<FlowEmission> {
    let root_seg = chain_ref.segments.first()?;
    let root_name = &root_seg.name;

    // Express/Hono/Fastify Consumer-side route registration. Checked first
    // because the test fires on the FILE'S imports (not the chain root's
    // identity), so it must run regardless of whether the chain root
    // happens to resolve to a producer-side import later in this function
    // (e.g. when the file does `import fastify from 'fastify'` and then
    // calls `fastify.put('/x', h)` on the same identifier).
    if let Some(emission) = detect_chain_route_consumer(chain_ref, call_args, file_ctx) {
        return Some(emission);
    }

    // Message-queue chain calls (NATS / Redis / amqplib / kafkajs):
    // `nc.publish('subj', ...)`, `redis.subscribe('chan', h)`,
    // `channel.consume('queue', h)`, etc. Like the HTTP route detector,
    // this fires on the FILE'S imports rather than the chain root's
    // identity — the chain root is typically a local bound from the
    // library's connection / channel factory.
    if let Some(emission) = detect_mq_chain_emission(chain_ref, call_args, file_ctx) {
        return Some(emission);
    }

    // Background-job libraries (BullMQ / Bull / Agenda / bee-queue):
    // Producer `queue.add('jobName', data)` / `agenda.now('jobName', ...)`,
    // Consumer constructor `new Worker('queue', processor)` and
    // `queue.process('jobName', h)`. The detector accepts both a single-segment
    // chain (constructor case) and a two-or-more-segment chain (method case)
    // because `new Worker(...)` lands as `[Worker]` while `queue.add(...)`
    // lands as `[queue, add]`. Fires on file imports.
    if let Some(emission) = detect_bgjob_chain_emission(chain_ref, call_args, file_ctx) {
        return Some(emission);
    }

    // gRPC / Connect chains: Producer `client.serviceName.method(req)` (Connect /
    // nice-grpc / ts-proto / @grpc/grpc-js) and Consumer `server.addService(Svc, …)`
    // (@grpc/grpc-js dynamic registration). Fires on file imports.
    if let Some(emission) = detect_rpc_chain_emission(chain_ref, call_args, file_ctx) {
        return Some(emission);
    }

    // Mailer Producer chains: `transport.sendMail({ template: 'name', … })`,
    // `sgMail.send({ templateId: 'd-x', … })`, `mailerService.sendMail({ … })`.
    // Keys on the object-literal `template` / `templateId` value.
    if let Some(emission) = detect_mailer_chain_emission(chain_ref, call_args, file_ctx) {
        return Some(emission);
    }

    // tRPC client chains: `trpc.<group>.<procedure>.useQuery|useMutation|...(args)`.
    // Each emits a Producer HttpCall whose URL is `/api/trpc/<group>.<procedure>`
    // — that pairs against the Next.js file-path Consumer
    // `app/api/trpc/[trpc]/route.ts` → `/api/trpc/{}` via the pairer's
    // segment-wildcard pass.
    if let Some(emission) = detect_trpc_chain_emission(chain_ref, file_ctx) {
        return Some(emission);
    }

    // ConfigLookup chain calls: `configService.get('key')`, `config.get('key')`
    // (NestJS ConfigService), where the first arg is a string-literal key.
    if let Some(emission) = detect_config_call_emission(chain_ref, call_args) {
        return Some(emission);
    }

    // FeatureFlag chain calls: library-gated; `growthbook.isOn('flag')`,
    // `statsig.checkGate('gate')`, `client.variation('flag', …)`,
    // `configcat.getValue('key', …)`, plus bare `useFeatureFlag('x')`.
    if let Some(emission) = detect_feature_flag_chain_emission(chain_ref, call_args, file_ctx) {
        return Some(emission);
    }

    // OpenAPI codegen runtimes (oazapfts and lookalikes): the chain root is a
    // long-lived `runtime(defaults)` instance bound to a local variable, so
    // the import-source path below cannot reach it. Match instead on the
    // leaf method name — `fetchJson`, `fetchText`, `fetchBlob` are distinct
    // to this generator family and don't collide with stdlib JS members.
    if let Some(leaf) = chain_ref.segments.last() {
        if matches!(leaf.name.as_str(), "fetchJson" | "fetchText" | "fetchBlob") {
            let raw = first_arg_string(call_args);
            if raw.is_empty() {
                return None;
            }
            let name = crate::connectors::url_pattern::normalize(&raw);
            if name.is_empty() || name == "/" {
                return None;
            }
            // HTTP verb is buried in the second arg (`{ method: "POST", ... }`)
            // which the call-arg extractor stores as `Other`; without a literal
            // we conservatively report `Any`, which the pairer treats as a
            // wildcard against any concrete consumer method.
            let method = HttpMethod::Any;
            return Some(FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name,
                role: ChannelRole::Producer,
                method: Some(method),
            streaming: None,
            });
        }
    }

    // Global `fetch` / `$fetch` — no import required, detected by name alone.
    if predicates::is_global_fetch(root_name.as_str()) {
        let raw = first_arg_string(call_args);
        // Dynamic-URL calls (`fetch(someVar)`) have no static URL to pair on.
        // Emitting them with an empty name produces a flow_edges row that can
        // never resolve to a consumer — pure noise. Skip.
        if raw.is_empty() {
            return None;
        }
        let name = crate::connectors::url_pattern::normalize(&raw);
        if name.is_empty() || name == "/" {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name,
            role: ChannelRole::Producer,
            method: Some(HttpMethod::Any),
        streaming: None,
        });
    }

    // Electron ipcRenderer — bare identifier, no import lookup needed.
    if predicates::is_electron_ipc_renderer(root_name.as_str()) {
        if let Some(leaf) = chain_ref.segments.last() {
            let role = if leaf.name == "on" { ChannelRole::Consumer } else { ChannelRole::Producer };
            let name = first_arg_string(call_args);
            return Some(FlowEmission::NamedChannel {
                kind: NamedChannelKind::IpcCall,
                name,
                role,
                method: None,
            streaming: None,
            });
        }
    }

    // Electron ipcMain — Consumer-side handler registration. `ipcMain.handle`
    // pairs with `ipcRenderer.invoke`, `ipcMain.on` pairs with
    // `ipcRenderer.send` / `webContents.send`. Both key on the first string
    // argument (the IPC channel name).
    if predicates::is_electron_ipc_main(root_name.as_str()) {
        if let Some(leaf) = chain_ref.segments.last() {
            if matches!(leaf.name.as_str(), "handle" | "on" | "handleOnce" | "once") {
                let name = first_arg_string(call_args);
                if !name.is_empty() {
                    return Some(FlowEmission::NamedChannel {
                        kind: NamedChannelKind::IpcCall,
                        name,
                        role: ChannelRole::Consumer,
                        method: None,
                    streaming: None,
                    });
                }
            }
        }
    }

    // gql`...` / graphql`...` tagged template — GraphQL operation.
    // The call_args carry a TaggedTemplate when the tagged template extractor ran.
    if let Some(CallArg::TaggedTemplate { tag, body }) = call_args.first() {
        if matches!(tag.as_str(), "gql" | "graphql" | "gqlTag" | "GraphQL") {
            let name = parse_gql_operation(body).unwrap_or_else(|| "graphql:__unknown__".to_string());
            return Some(FlowEmission::NamedChannel {
                kind: NamedChannelKind::GraphQLOp,
                name,
                role: ChannelRole::Producer,
                method: None,
            streaming: None,
            });
        }
    }

    // Import-based: look up the package the root name was imported from.
    // When the chain root has no matching import (likely a long-lived
    // constructed instance such as a Prisma client or TypeORM repository),
    // fall through to the ORM-shape detector at the bottom.
    let import_entry = file_ctx.imports.iter()
        .find(|imp| {
            imp.imported_name == *root_name
                || imp.alias.as_deref() == Some(root_name.as_str())
        });
    let import_source = match import_entry.and_then(|imp| imp.module_path.as_deref()) {
        Some(src) => src,
        None => return detect_db_query_emission_with_imports(chain_ref, &file_ctx.imports),
    };

    if predicates::is_http_client_module(import_source) {
        // Derive HTTP method from the chained method name, e.g. `axios.get` → GET.
        let method_seg = chain_ref.segments.get(1);
        let method = method_seg
            .map(|s| HttpMethod::from_method_name(&s.name))
            .unwrap_or(HttpMethod::Any);
        let raw = first_arg_string(call_args);
        if raw.is_empty() {
            return None;
        }
        let name = crate::connectors::url_pattern::normalize(&raw);
        if name.is_empty() || name == "/" {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name,
            role: ChannelRole::Producer,
            method: Some(method),
        streaming: None,
        });
    }

    if predicates::is_socketio_client_module(import_source) {
        if let Some(leaf) = chain_ref.segments.last() {
            let role = if leaf.name == "on" { ChannelRole::Consumer } else { ChannelRole::Producer };
            let name = first_arg_string(call_args);
            return Some(FlowEmission::NamedChannel {
                kind: NamedChannelKind::WebSocket,
                name,
                role,
                method: None,
            streaming: None,
            });
        }
    }

    if predicates::is_tauri_invoke_module(import_source) {
        let name = first_arg_string(call_args);
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::IpcCall,
            name,
            role: ChannelRole::Producer,
            method: None,
        streaming: None,
        });
    }

    // Migration: knex.schema.createTable / queryInterface.createTable / knex.schema.dropTable etc.
    if is_migration_call_chain(chain_ref, import_source) {
        let table_name = first_arg_string(call_args);
        if !table_name.is_empty() {
            let direction = infer_migration_direction_from_chain(chain_ref);
            return Some(FlowEmission::MigrationTarget { table_name, direction });
        }
    }

    // Scheduled job: cron.schedule / node-cron, BullMQ queue.add with repeat
    if is_cron_schedule_call(chain_ref, import_source) {
        let schedule = first_arg_string(call_args);
        if !schedule.is_empty() {
            return Some(FlowEmission::ScheduledJob { schedule });
        }
    }

    // CLI: program.command / yargs.command
    if is_cli_command_call(chain_ref, import_source) {
        let command_name = first_arg_string(call_args);
        if !command_name.is_empty() {
            let framework = infer_cli_framework(import_source);
            return Some(FlowEmission::CliCommand { command_name, framework });
        }
    }

    // ORM DbQuery: chain-shape-driven detection of Prisma / TypeORM /
    // Mongoose / Sequelize call sites. The pairer joins these with
    // already-emitted DbEntity decorators across files. Imports are
    // consulted for the noisier Mongoose/Sequelize PascalCase branch.
    if let Some(emission) = detect_db_query_emission_with_imports(chain_ref, &file_ctx.imports) {
        return Some(emission);
    }

    None
}

/// Inspect `chain` for an ORM query call. Returns `FlowEmission::DbQuery` when
/// the chain shape matches a Prisma, TypeORM, Mongoose, or Sequelize call.
///
/// Recognition is intentionally chain-shape-based rather than import-based:
/// the ORM client is usually a long-lived constructed instance whose import
/// trail is one or two hops removed from the call site. Distinguishing the
/// four ORMs by call-shape works because their public APIs are distinct:
///
///   * **Prisma** — three-segment `<client>.<model>.<op>` where `<model>` is
///     a camelCase property of the generated client and `<op>` is from a
///     Prisma-specific method set (`findUnique`, `findMany`, `upsert`, ...).
///   * **TypeORM** — two-segment `<repo>.<op>` where `<repo>` either declares
///     a `Repository<Entity>` type (or `TreeRepository<>`, `MongoRepository<>`)
///     or carries a `Repository`/`Repo` name suffix; `<op>` is a TypeORM op.
///   * **Mongoose / Sequelize** — two-segment `<Model>.<op>` where `<Model>`
///     is a PascalCase identifier and `<op>` is one of the ORM's static
///     methods (`findOne`, `findByPk`, `findAndCountAll`, ...).
///
/// The detector is conservative on shared method names (`find`, `create`,
/// `update`, `delete`) by anchoring on the casing / suffix signal of the
/// chain root.  False positives are bounded by the pairer: a DbQuery only

/// True when the chain looks like a knex/queryInterface migration table call:
/// `queryInterface.createTable` / `queryInterface.dropTable` /
/// `knex.schema.createTable` / `knex.schema.dropTableIfExists`.
fn is_migration_call_chain(chain: &crate::types::MemberChain, import_source: &str) -> bool {
    if predicates::is_knex_module(import_source) {
        return chain.segments.windows(2).any(|w| {
            w[0].name == "schema"
                && matches!(
                    w[1].name.to_ascii_lowercase().as_str(),
                    "createtable" | "droptable" | "droptableifexists" | "renametable"
                        | "altertable" | "hastable"
                )
        });
    }
    // Sequelize queryInterface (first segment name)
    let root = chain.segments.first().map(|s| s.name.as_str()).unwrap_or("");
    if root == "queryInterface" || root == "queryRunner" {
        return chain.segments.last().map_or(false, |s| {
            matches!(
                s.name.to_ascii_lowercase().as_str(),
                "createtable" | "droptable" | "addcolumn" | "removecolumn"
                    | "renametable" | "addindex" | "removeindex" | "createqueryinterface"
            )
        });
    }
    false
}

fn infer_migration_direction_from_chain(chain: &crate::types::MemberChain) -> MigrationDirection {
    // Direction can't be determined from the call site alone — the containing
    // function name (up/down) is at a higher AST level. Default to Up.
    let _ = chain;
    MigrationDirection::Up
}

/// True when the chain looks like a cron schedule registration.
fn is_cron_schedule_call(chain: &crate::types::MemberChain, import_source: &str) -> bool {
    predicates::is_cron_module(import_source)
        && chain.segments.last().map_or(false, |s| {
            matches!(s.name.to_ascii_lowercase().as_str(), "schedule" | "create" | "scheduleJob")
        })
}

/// True when the chain looks like a CLI command registration.
fn is_cli_command_call(chain: &crate::types::MemberChain, import_source: &str) -> bool {
    predicates::is_cli_module(import_source)
        && chain.segments.last().map_or(false, |s| s.name == "command")
}

fn infer_cli_framework(import_source: &str) -> Option<String> {
    if import_source.contains("commander") {
        Some("commander".to_string())
    } else if import_source.contains("yargs") {
        Some("yargs".to_string())
    } else if import_source.contains("@oclif") {
        Some("oclif".to_string())
    } else if import_source.contains("meow") {
        Some("meow".to_string())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Express / Hono / Fastify chain-route Consumer — HttpCall emission
// ---------------------------------------------------------------------------

/// Inspect a non-import-rooted chain (`app.get('/x', h)`,
/// `router.post('/y', h)`, etc.) and emit a `Consumer`-role `HttpCall` when
/// the file's import set contains an Express-family framework and the
/// chain's terminal segment names an HTTP verb.
///
/// Recognised frameworks: `express`, `hono`, `fastify`, `fastify-plugin`
/// (and any sub-path of those). Recognised verbs: `get` / `post` / `put` /
/// `patch` / `delete` / `head` / `options` / `all`.
///
/// The path is captured from the first string-literal call argument and
/// normalised via `connectors::url_pattern::normalize`. Sites whose first
/// arg isn't a literal (handler-only `router.all(h)`, etc.) are not

pub(crate) fn detect_chain_route_consumer(
    chain: &crate::types::MemberChain,
    call_args: &[CallArg],
    file_ctx: &FileContext,
) -> Option<FlowEmission> {
    if !file_imports_chain_router_framework(file_ctx) {
        return None;
    }
    let leaf = chain.segments.last()?;
    let method = parse_http_verb(leaf.name.as_str())?;
    let path = first_arg_string(call_args);
    if path.is_empty() {
        return None;
    }
    let normalized = crate::connectors::url_pattern::normalize(&path);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: normalized,
        role: ChannelRole::Consumer,
        method: Some(method),
    streaming: None,
    })
}

fn file_imports_chain_router_framework(file_ctx: &FileContext) -> bool {
    file_ctx
        .imports
        .iter()
        .any(|imp| imp.module_path.as_deref().map_or(false, is_chain_router_module))
}

fn is_chain_router_module(pkg: &str) -> bool {
    let root = pkg.split('/').next().unwrap_or(pkg);
    matches!(root, "express" | "hono" | "fastify" | "fastify-plugin")
}

fn parse_http_verb(name: &str) -> Option<HttpMethod> {
    Some(match name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        "all" => HttpMethod::Any,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Message-queue chain calls — Producer / Consumer NamedChannel MessageQueue
// ---------------------------------------------------------------------------

/// Inspect a non-import-rooted chain (`nc.publish('subj', ...)`,
/// `redis.subscribe('chan')`, `channel.consume('queue', h)`, etc.) and
/// emit a `NamedChannel { kind: MessageQueue, .. }` keyed on the first string
/// argument when the file's import set contains a known broker client.
///
/// Recognised libraries: `nats`, `ioredis`, `redis`, `amqplib`, `kafkajs`,
/// `mqtt`, `@aws-sdk/client-sqs`, `@google-cloud/pubsub`,
/// `@nestjs/microservices`.
///
/// Recognised chain leaves: `publish` (Producer); `subscribe` and
/// `consume` (Consumer). Producer verbs that take an object literal
/// argument (kafkajs `producer.send({ topic, ... })`, sqs
/// `sendMessage({ QueueUrl, ... })`) are not emitted here — the call-arg
/// extractor stores their object literals as `CallArg::Other`, leaving
/// no pairing key available. The decorator path
/// (`@MessagePattern`/`@EventPattern`) covers the NestJS microservices

pub(crate) fn detect_trpc_chain_emission(
    chain: &crate::types::MemberChain,
    file_ctx: &FileContext,
) -> Option<FlowEmission> {
    if !file_imports_trpc(file_ctx) {
        return None;
    }
    let segs = &chain.segments;
    if segs.len() < 3 {
        return None;
    }
    let root = segs[0].name.as_str();
    if !matches!(root, "trpc" | "api" | "client") {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    if !is_trpc_verb(leaf) {
        return None;
    }
    // Intermediate segments between root and verb form the procedure path.
    let procedure_parts: Vec<&str> = segs[1..segs.len() - 1]
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    if procedure_parts.is_empty() {
        return None;
    }
    let procedure = procedure_parts.join(".");
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: format!("/api/trpc/{}", procedure),
        role: ChannelRole::Producer,
        method: Some(HttpMethod::Any),
    streaming: None,
    })
}

fn file_imports_trpc(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let Some(m) = imp.module_path.as_deref() else { return false; };
        m.starts_with("@trpc/")
            || m == "@trpc/client"
            || m.ends_with("/trpc/client")
            || m.ends_with("/trpc")
            || imp.imported_name == "trpc"
    })
}

fn is_trpc_verb(name: &str) -> bool {
    matches!(
        name,
        "useQuery"
            | "useMutation"
            | "useSubscription"
            | "useInfiniteQuery"
            | "useSuspenseQuery"
            | "query"
            | "mutate"
            | "fetch"
            | "prefetch"
            | "prefetchQuery"
    )
}

// ---------------------------------------------------------------------------
// ConfigLookup — `process.env.X`, `import.meta.env.X`, `config.get('key')`
// ---------------------------------------------------------------------------

/// Inspect a TypeRef chain like `process.env.NODE_ENV` or
/// `import.meta.env.VITE_API_BASE` and emit a `ConfigLookup` keyed on the

pub(crate) fn detect_member_access_config_emission(
    chain: &crate::types::MemberChain,
) -> Option<FlowEmission> {
    let segs = &chain.segments;
    // `process.env.<KEY>` — three segments, identifier root.
    if segs.len() == 3 && segs[0].name == "process" && segs[1].name == "env" {
        let key = segs[2].name.clone();
        if !key.is_empty() {
            return Some(FlowEmission::ConfigLookup { key });
        }
    }
    // `import.meta.env.<KEY>` — four segments, `import` keyword root.
    if segs.len() == 4 && segs[0].name == "import" && segs[1].name == "meta" && segs[2].name == "env" {
        let key = segs[3].name.clone();
        if !key.is_empty() {
            return Some(FlowEmission::ConfigLookup { key });
        }
    }
    None
}

/// Inspect a TypeRef chain whose root identifier is feature-flag-shaped
/// (`featureFlags`, `features`, `flags`, or any name containing
/// `featureFlag`) and emit a `FeatureFlag` keyed on the leaf identifier.
/// Recognises two shapes:
/// - `[<root>, <flag>]` — two-segment plain property access.
/// - `[<root>, value, <flag>]` — three-segment Svelte runes / Manager
///   singleton pattern (`featureFlagsManager.value.X`).
pub(crate) fn detect_member_access_feature_flag_emission(
    chain: &crate::types::MemberChain,
) -> Option<FlowEmission> {
    let segs = &chain.segments;
    let (root, flag) = match segs.len() {
        2 => (segs[0].name.as_str(), segs[1].name.as_str()),
        3 if segs[1].name == "value" => (segs[0].name.as_str(), segs[2].name.as_str()),
        _ => return None,
    };
    if !is_feature_flag_identifier(root) {
        return None;
    }
    if flag.is_empty() {
        return None;
    }
    Some(FlowEmission::FeatureFlag { flag_name: flag.to_string() })
}

fn is_feature_flag_identifier(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), "featureflags" | "features" | "flags")
        || lower.contains("featureflag")
}

/// Inspect a Calls chain like `config.get('PORT')` or
/// `configService.get('DATABASE_URL', 'fallback')` and emit a `ConfigLookup`
/// keyed on the first string-literal argument. The chain root identifier
/// must be a recognised config-service binding name; this keeps the
/// detector from misfiring on unrelated `.get(string)` shapes (e.g. Map's
/// `.get`).
pub(crate) fn detect_config_call_emission(
    chain: &crate::types::MemberChain,
    call_args: &[CallArg],
) -> Option<FlowEmission> {
    let leaf = chain.segments.last()?;
    if leaf.name != "get" {
        return None;
    }
    let root = chain.segments.first()?;
    if !is_config_service_root(root.name.as_str()) {
        return None;
    }
    let key = first_arg_string(call_args);
    if key.is_empty() {
        return None;
    }
    Some(FlowEmission::ConfigLookup { key })
}

fn is_config_service_root(name: &str) -> bool {
    matches!(
        name,
        "config"
            | "configService"
            | "configs"
            | "appConfig"
            | "envConfig"
            | "settings"
            | "configManager"
    )
}

// ---------------------------------------------------------------------------
// FeatureFlag — library-gated chain calls
// ---------------------------------------------------------------------------

/// Recognise feature-flag client calls and emit `FeatureFlag` keyed on the
/// flag name (first string-literal arg). Library gating: the chain detector
/// only fires when the file imports a recognised feature-flag SDK or when
/// the leaf is a globally-named hook (`useFeatureFlag`).
///
/// Recognised shapes:
/// - `<gb>.isOn('flag')`, `<gb>.feature('flag')`, `<gb>.getValue('flag', d)`
///   — GrowthBook
/// - `<ld>.variation('flag', user, default)` — LaunchDarkly
/// - `<statsig>.checkGate('gate')` / `<statsig>.getConfig('cfg')` — Statsig
/// - `<configcat>.getValue('key', default)` — ConfigCat
/// - `useFeatureFlag('flag')`, `useFlag('flag')` — generic React hooks
pub(crate) fn detect_feature_flag_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[CallArg],
    file_ctx: &FileContext,
) -> Option<FlowEmission> {
    let leaf = chain.segments.last()?;
    // Hook shape: 1-segment chain whose leaf is a generic feature-flag hook.
    if chain.segments.len() == 1
        && matches!(leaf.name.as_str(), "useFeatureFlag" | "useFlag" | "useGate")
    {
        let flag = first_arg_string(call_args);
        if !flag.is_empty() {
            return Some(FlowEmission::FeatureFlag { flag_name: flag });
        }
        return None;
    }
    if !is_feature_flag_verb(leaf.name.as_str()) {
        return None;
    }
    if !file_imports_feature_flag_library(file_ctx) {
        return None;
    }
    let flag = first_arg_string(call_args);
    if flag.is_empty() {
        return None;
    }
    Some(FlowEmission::FeatureFlag { flag_name: flag })
}

fn is_feature_flag_verb(name: &str) -> bool {
    matches!(
        name,
        "isOn"
            | "isOff"
            | "feature"
            | "getValue"
            | "variation"
            | "variationDetail"
            | "boolVariation"
            | "stringVariation"
            | "checkGate"
            | "getConfig"
            | "getExperiment"
    )
}

fn file_imports_feature_flag_library(file_ctx: &FileContext) -> bool {
    file_ctx
        .imports
        .iter()
        .any(|imp| imp.module_path.as_deref().map_or(false, is_feature_flag_library))
}

fn is_feature_flag_library(pkg: &str) -> bool {
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
        "@growthbook/growthbook"
            | "@growthbook/growthbook-react"
            | "launchdarkly-js-client-sdk"
            | "launchdarkly-node-server-sdk"
            | "@launchdarkly/node-server-sdk"
            | "@launchdarkly/js-client-sdk"
            | "statsig-js"
            | "statsig-node"
            | "@statsig/js-client"
            | "configcat-js"
            | "configcat-node"
            | "@unleash/proxy-client-react"
            | "unleash-client"
            | "@vercel/flags"
            | "@openfeature/web-sdk"
            | "@openfeature/server-sdk"
    )
}