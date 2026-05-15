// =============================================================================
// languages/typescript/flow_detectors.rs — FlowEmission detectors for TS/JS
//
// All the `detect_*` helpers that recognise a cross-tier flow-edge pattern in
// a TypeScript or JavaScript call site and return a `FlowEmission` for the
// pairer. Grouped by detection shape:
//
//   * Decorator-based — NestJS @Controller / @Get / TypeORM @Entity / Angular @Injectable
//   * Chain-based — HTTP clients (fetch/axios), WebSocket, IPC, tRPC,
//     gRPC clients, message-queue libraries, background-job libraries,
//     mailer libraries, route handlers (Express/Fastify/Hono/Koa)
//   * Member-access — config/env reads, feature-flag boolean shorthand
//   * DB query — ORM operation classification (Prisma / TypeORM /
//     Mongoose / Sequelize) and entity-name inference
//
// The detectors are independent of the resolver's tier-1 logic — they run
// against the same RefContext regardless of whether resolution succeeded.
// =============================================================================

use tracing::debug;

use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::indexer::resolve::flow_emit::{
    AuthGuardKind, ChannelRole, DbQueryOp, FlowEmission, HttpMethod, MigrationDirection,
    NamedChannelKind,
};
use crate::types::CallArg;

use super::predicates;

/// Reserved synthetic-import key prefix used to thread `@Controller(prefix)`
/// metadata through `FileContext.imports` so that method-level HTTP-verb
/// decorators can recover the class's route prefix from the file context
/// alone. Concatenated with the controller class's qualified name.
pub(super) const CONTROLLER_PREFIX_KEY: &str = "__ts_controller_prefix__:";
pub(super) const BGJOB_QUEUE_BINDING_KEY: &str = "__ts_bgjob_queue_binding__:";

/// Inspect a TypeRef ref whose `target_name` is a decorator name and return a
/// `FlowEmission` when it matches a well-known cross-tier pattern decorator.
///
/// This covers:
/// - NestJS `@UseGuards(...)`, `@Roles(...)`, `@Permissions(...)`, `@AuthGuard(...)`
/// - TypeORM `@Entity(...)`, `@Table(...)` — DbEntity marker
/// - Sequelize `@Table(...)`, `@Column(...)` applied to a model class
///
/// `first_arg` is the first string argument from the decorator call, stored in
/// `ExtractedRef::module` by the decorator extractor.
///
/// `class_context` is the name of the symbol carrying the decorator (the
/// class for class-level decorators). DbEntity decorators without an explicit
/// table-name argument fall back to this so the cross-file pairer has a
/// meaningful entity key.
pub(crate) fn detect_decorator_flow_emission(
    decorator_name: &str,
    first_arg: Option<&str>,
    class_context: Option<&str>,
) -> Option<FlowEmission> {
    detect_decorator_flow_emission_inner(decorator_name, first_arg, class_context)
}

/// Imports-aware variant. Entity-marker decorators (`@Entity`, `@Table`,
/// `@Schema`, `@Document`, `@Collection`) share their names with type imports
/// from unrelated packages — `import { Entity } from '@elysiajs/core'`,
/// `import { Document } from 'next/document'`, `import { Schema } from 'zod'`.
/// Those produce TypeRef refs that fire this detector and emit bogus DbEntity
/// rows whose url_pattern is whatever class happens to use the type. Gating
/// on the file importing a known ORM package eliminates the false positives.
pub(crate) fn detect_decorator_flow_emission_with_imports(
    decorator_name: &str,
    first_arg: Option<&str>,
    class_context: Option<&str>,
    file_imports: &[ImportEntry],
) -> Option<FlowEmission> {
    let is_entity_decorator = matches!(
        decorator_name,
        "Entity" | "Table" | "Schema" | "Document" | "Collection"
    );
    if is_entity_decorator && !file_imports_orm_decorator_package(file_imports) {
        return None;
    }
    // Skip when `first_arg` matches a file import path — that's a type
    // import like `import { Entity } from 'typeorm'`, not an `@Entity('name')`
    // decorator usage. The extractor reuses `r.module` for both semantics
    // and they're otherwise indistinguishable from this layer.
    if let Some(arg) = first_arg {
        if file_imports.iter().any(|imp| imp.module_path.as_deref() == Some(arg)) {
            return None;
        }
    }
    detect_decorator_flow_emission_inner(decorator_name, first_arg, class_context)
}

fn detect_decorator_flow_emission_inner(
    decorator_name: &str,
    first_arg: Option<&str>,
    class_context: Option<&str>,
) -> Option<FlowEmission> {
    let entity_table_hint = || -> Option<String> {
        first_arg
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| class_context.filter(|c| !c.is_empty()).map(|c| c.to_string()))
    };
    match decorator_name {
        // NestJS authorization decorators
        "UseGuards" | "Guard" => Some(FlowEmission::AuthGuard {
            requirement: first_arg.unwrap_or("").to_string(),
            kind: AuthGuardKind::Custom,
        }),
        "Roles" => Some(FlowEmission::AuthGuard {
            requirement: first_arg.unwrap_or("").to_string(),
            kind: AuthGuardKind::Role,
        }),
        "Permissions" | "RequirePermissions" => Some(FlowEmission::AuthGuard {
            requirement: first_arg.unwrap_or("").to_string(),
            kind: AuthGuardKind::Permission,
        }),
        "Policy" | "CheckPolicy" => Some(FlowEmission::AuthGuard {
            requirement: first_arg.unwrap_or("").to_string(),
            kind: AuthGuardKind::Policy,
        }),
        "AuthGuard" | "JwtAuthGuard" | "BearerAuth" => Some(FlowEmission::AuthGuard {
            requirement: first_arg.unwrap_or(decorator_name).to_string(),
            kind: AuthGuardKind::Token,
        }),
        // TypeORM / Sequelize entity markers
        "Entity" => Some(FlowEmission::DbEntity {
            base_symbol_id: None,
            base_name_hint: "Entity".to_string(),
            table_name_hint: entity_table_hint(),
        }),
        "Table" => Some(FlowEmission::DbEntity {
            base_symbol_id: None,
            base_name_hint: "Model".to_string(),
            table_name_hint: entity_table_hint(),
        }),
        "Schema" | "Document" | "Collection" => Some(FlowEmission::DbEntity {
            base_symbol_id: None,
            base_name_hint: decorator_name.to_string(),
            table_name_hint: entity_table_hint(),
        }),
        // NestJS microservices Consumer markers — `@MessagePattern('topic')`
        // for request/response and `@EventPattern('topic')` for fire-and-
        // forget event handlers. Both pair to producer-side `publish`/`send`
        // calls keyed on the same topic string.
        "MessagePattern" | "EventPattern" => Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::MessageQueue,
            name: first_arg.unwrap_or("").to_string(),
            role: ChannelRole::Consumer,
            method: None,
        streaming: None,
        }),
        // NestJS WebSocket Consumer — `@SubscribeMessage('event')` on a
        // gateway method handles inbound WS messages keyed on the event name.
        // `@WebSocketGateway()` marks a class as a WS endpoint; we tag it
        // single-ended with the class name so the architecture overview
        // still surfaces the gateway when no SubscribeMessage is present.
        "SubscribeMessage" => Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::WebSocket,
            name: first_arg.unwrap_or("").to_string(),
            role: ChannelRole::Consumer,
            method: None,
        streaming: None,
        }),
        "WebSocketGateway" => Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::WebSocket,
            name: class_context.unwrap_or("ts.nestjs.gateway").to_string(),
            role: ChannelRole::Consumer,
            method: None,
        streaming: None,
        }),
        _ => None,
    }
}

/// Extract a string name from the first call argument, if it is a string literal
/// or a template literal. Returns an empty string when the argument is an identifier
/// or otherwise not statically determinable.
fn first_arg_string(call_args: &[CallArg]) -> String {
    match call_args.first() {
        Some(CallArg::StringLit(s)) => s.clone(),
        Some(CallArg::TemplateLit(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Parse a GraphQL operation type + name from a tagged template body.
///
/// Looks for the pattern `(query|mutation|subscription)\s+(\w+)` at the start
/// of the body (ignoring leading whitespace). Returns `"<op>:<Name>"` when
/// found, e.g. `"query:GetUsers"`.
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
/// becomes a cross-file `flow_edges` row if a matching DbEntity exists.
pub(crate) fn detect_db_query_emission(
    chain: &crate::types::MemberChain,
) -> Option<FlowEmission> {
    // Production path runs through the imports-aware variant; this no-imports
    // wrapper exists for unit tests that don't need the gating semantics.
    detect_db_query_emission_inner(chain, true)
}

/// Imports-aware variant of the ORM-call detector.
///
/// The Mongoose/Sequelize PascalCase branch is the noisiest — `Object.create`,
/// `Headers.find`, `URL.create`, `Array.from`-like shapes all match. Gating
/// on the file actually importing one of those ORMs eliminates the bulk of
/// false positives without losing any real query. The Prisma + TypeORM
/// branches already have stronger structural signals (3-segment chain with
/// Prisma-specific op set; declared `Repository<>` type or `Repository`
/// suffix) and stay open even without an explicit import — Prisma clients
/// are commonly long-lived re-exported instances whose import trail can't be
/// recovered from the call site alone.
pub(crate) fn detect_db_query_emission_with_imports(
    chain: &crate::types::MemberChain,
    file_imports: &[ImportEntry],
) -> Option<FlowEmission> {
    detect_db_query_emission_inner(chain, file_imports_mongoose_or_sequelize(file_imports))
}

fn detect_db_query_emission_inner(
    chain: &crate::types::MemberChain,
    mongoose_seq_branch_enabled: bool,
) -> Option<FlowEmission> {
    let segs = &chain.segments;
    if segs.is_empty() {
        return None;
    }

    // Prisma: <client>.<camelModel>.<prismaOp>
    if segs.len() >= 3 {
        let model_seg = &segs[1];
        let op_seg = &segs[2];
        if is_camel_case_first(&model_seg.name) && is_prisma_op(&op_seg.name) {
            return Some(FlowEmission::DbQuery {
                entity_name: capitalize_first(&model_seg.name),
                operation: classify_orm_op(&op_seg.name),
            });
        }
    }

    // TypeORM repository — declared-type or name-suffix signal on the root.
    if segs.len() >= 2 {
        let root = &segs[0];
        let op_seg = &segs[1];
        if is_typeorm_op(&op_seg.name) {
            if let Some(dt) = root.declared_type.as_deref() {
                let dt_root = dt.split('<').next().unwrap_or(dt);
                if matches!(dt_root, "Repository" | "TreeRepository" | "MongoRepository") {
                    if let Some(entity) = root.type_args.first() {
                        return Some(FlowEmission::DbQuery {
                            entity_name: entity.clone(),
                            operation: classify_orm_op(&op_seg.name),
                        });
                    }
                }
            }
            if let Some(entity) = repository_suffix_entity(&root.name) {
                return Some(FlowEmission::DbQuery {
                    entity_name: entity,
                    operation: classify_orm_op(&op_seg.name),
                });
            }
        }
    }

    // Mongoose / Sequelize: <PascalModel>.<staticOp> — gated by the caller.
    // Without the gate, `Object.create`, `Headers.find`, `URL.create`, and
    // similar JS-builtin call shapes all match. The imports-aware variant
    // enables this branch only when mongoose/sequelize is imported.
    if mongoose_seq_branch_enabled && segs.len() >= 2 {
        let root = &segs[0];
        let op_seg = &segs[1];
        if is_pascal_case_first(&root.name) && is_mongoose_or_sequelize_op(&op_seg.name) {
            return Some(FlowEmission::DbQuery {
                entity_name: root.name.clone(),
                operation: classify_orm_op(&op_seg.name),
            });
        }
    }

    None
}

/// True when the file imports from a package that defines ORM entity-marker
/// decorators (`@Entity`, `@Table`, `@Schema`, `@Document`, `@Collection`).
/// Used to gate the entity-marker branch of `detect_decorator_flow_emission`
/// against type imports of the same names from unrelated packages.
fn file_imports_orm_decorator_package(file_imports: &[ImportEntry]) -> bool {
    file_imports.iter().any(|imp| {
        let Some(m) = imp.module_path.as_deref() else { return false; };
        m == "typeorm"
            || m.starts_with("typeorm/")
            || m == "sequelize-typescript"
            || m == "@nestjs/mongoose"
            || m == "@mikro-orm/core"
            || m.starts_with("@mikro-orm/")
            || m == "@nestjs/typeorm"
    })
}

fn file_imports_mongoose_or_sequelize(file_imports: &[ImportEntry]) -> bool {
    file_imports.iter().any(|imp| {
        let Some(m) = imp.module_path.as_deref() else { return false; };
        m == "mongoose"
            || m.starts_with("mongoose/")
            || m == "sequelize"
            || m.starts_with("sequelize/")
            || m == "@sequelize/core"
            || m.starts_with("@sequelize/")
            || m == "sequelize-typescript"
    })
}

/// Classify an ORM method name into a `DbQueryOp`.
///
/// Wraps `DbQueryOp::from_method_name` (which folds case but matches only
/// against a fixed set of generic verbs) with ORM-specific compound forms
/// that the generic mapper would otherwise return `Other` for —
/// `findByIdAndUpdate`, `findByPk`, `createMany`, `bulkCreate`, and so on.
fn classify_orm_op(name: &str) -> DbQueryOp {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        // Compound Select forms not covered by from_method_name's literal set.
        "findbyid" | "findbypk" | "findunique" | "findfirst" | "findany"
        | "findfirstorthrow" | "finduniqueorthrow" | "findoneorfail"
        | "findoneby" | "findonebyorfail" | "findandcount" | "findandcountby"
        | "findby" | "findandcountall" | "estimateddocumentcount"
        | "countdocuments" | "countby" | "distinct" | "aggregate" | "groupby"
        | "has" | "exist" => DbQueryOp::Select,

        // Compound Insert forms.
        "createmany" | "createmanyandreturn" | "bulkcreate" | "insertmany"
        | "bulkbuild" => DbQueryOp::Insert,

        // Compound Update forms.
        "updatemany" | "updatemanyandreturn" | "updateone" | "replaceone"
        | "findbyidandupdate" | "findoneandupdate" | "findoneandreplace"
        | "softrestore" => DbQueryOp::Update,

        // Compound Delete forms.
        "deletemany" | "deleteone" | "destroyall" | "truncate"
        | "softdelete" | "softremove" | "findbyidanddelete"
        | "findbyidandremove" | "findoneanddelete" | "findoneandremove" => {
            DbQueryOp::Delete
        }

        // Compound Upsert forms.
        "createorupdate" | "saveorupdate" => DbQueryOp::Upsert,

        // Fall through to the shared classifier for the simple verbs it
        // already handles (find / findOne / findAll / create / save /
        // update / delete / remove / upsert / count / ...).
        _ => DbQueryOp::from_method_name(name),
    }
}

fn is_pascal_case_first(s: &str) -> bool {
    s.chars().next().map_or(false, |c| c.is_ascii_uppercase())
}

fn is_camel_case_first(s: &str) -> bool {
    s.chars().next().map_or(false, |c| c.is_ascii_lowercase())
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Strip a `Repository` or `Repo` suffix (case-insensitive on the suffix
/// itself, preserving the prefix's casing) and PascalCase the remainder.
/// Returns `None` if no suffix is present or the prefix is empty.
fn repository_suffix_entity(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let prefix_len = if lower.ends_with("repository") {
        name.len() - "repository".len()
    } else if lower.ends_with("repo") {
        name.len() - "repo".len()
    } else {
        return None;
    };
    if prefix_len == 0 {
        return None;
    }
    Some(capitalize_first(&name[..prefix_len]))
}

/// Prisma-distinctive op set. `find` is intentionally excluded — it's a JS
/// Array method and would false-positive on three-segment chains rooted at
/// any object with a `find` property. Prisma's own catalogue uses the more
/// specific `findUnique` / `findFirst` / `findMany` forms.
fn is_prisma_op(name: &str) -> bool {
    matches!(
        name,
        "findUnique"
            | "findUniqueOrThrow"
            | "findFirst"
            | "findFirstOrThrow"
            | "findMany"
            | "create"
            | "createMany"
            | "createManyAndReturn"
            | "update"
            | "updateMany"
            | "updateManyAndReturn"
            | "upsert"
            | "delete"
            | "deleteMany"
            | "count"
            | "aggregate"
            | "groupBy"
    )
}

fn is_typeorm_op(name: &str) -> bool {
    matches!(
        name,
        "find"
            | "findOne"
            | "findOneBy"
            | "findOneOrFail"
            | "findOneByOrFail"
            | "findBy"
            | "findAndCount"
            | "findAndCountBy"
            | "save"
            | "insert"
            | "update"
            | "upsert"
            | "delete"
            | "remove"
            | "softDelete"
            | "softRemove"
            | "restore"
            | "count"
            | "countBy"
            | "exist"
            | "exists"
    )
}

fn is_mongoose_or_sequelize_op(name: &str) -> bool {
    matches!(
        name,
        // Mongoose statics
        "find"
            | "findOne"
            | "findById"
            | "findByIdAndUpdate"
            | "findByIdAndDelete"
            | "findByIdAndRemove"
            | "findOneAndUpdate"
            | "findOneAndDelete"
            | "findOneAndReplace"
            | "findOneAndRemove"
            | "create"
            | "insertMany"
            | "updateOne"
            | "updateMany"
            | "replaceOne"
            | "deleteOne"
            | "deleteMany"
            | "remove"
            | "countDocuments"
            | "estimatedDocumentCount"
            | "distinct"
            | "aggregate"
            | "exists"
            // Sequelize statics
            | "findAll"
            | "findByPk"
            | "findOrCreate"
            | "findOrBuild"
            | "findAndCountAll"
            | "bulkCreate"
            | "upsert"
            | "destroy"
            | "truncate"
            | "increment"
            | "decrement"
            // Shared / generic
            | "count"
            | "save"
    )
}

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
/// emitted — they can't be paired without a static path.
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
/// consumer side without needing chain detection.
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
///   originating `new Queue(...)` declaration is in another file.
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
///   wildcard pairer can match every client call against this server.
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

fn file_imports_rpc_library(file_ctx: &FileContext) -> bool {
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
fn strip_service_suffix(name: &str) -> String {
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
/// template name lives in a variable produce no emission (no static key).
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
/// trailing identifier. Returns `None` for unrelated chain shapes.
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

// ---------------------------------------------------------------------------
// NestJS gRPC method decorators — Consumer-role RpcCall emission
// ---------------------------------------------------------------------------

/// Inspect a TypeRef ref whose `target_name` is a gRPC method decorator
/// (`@GrpcMethod`, `@GrpcStreamMethod`) and emit a Consumer
/// `NamedChannel { kind: RpcCall, .. }`.
///
/// Argument shapes recognised:
/// - `@GrpcMethod('ServiceName', 'methodName')` — both args captured;
///   pairing key is `ServiceName/methodName` (service-suffix-stripped).
/// - `@GrpcMethod('ServiceName')` — falls back to the enclosing method's
///   declared name.
/// - `@GrpcMethod()` — emits with empty service and the enclosing method
///   name, which won't pair but keeps the consumer visible as a single-
///   ended row.
pub(crate) fn detect_grpc_decorator_flow_emission(
    decorator_name: &str,
    call_args: &[CallArg],
    enclosing_method: &str,
) -> Option<FlowEmission> {
    if !matches!(decorator_name, "GrpcMethod" | "GrpcStreamMethod") {
        return None;
    }
    // Require a string-literal first argument (the service name). Import-
    // statement refs for the `GrpcMethod` identifier also land here with the
    // same `target_name` but empty `call_args`; rejecting them here keeps
    // them out of the flow-edge stream.
    let service = first_arg_string(call_args);
    if service.is_empty() {
        return None;
    }
    let method = match call_args.get(1) {
        Some(CallArg::StringLit(s)) | Some(CallArg::TemplateLit(s)) => s.clone(),
        _ => enclosing_method.to_string(),
    };
    if method.is_empty() {
        return None;
    }
    let service_norm = strip_service_suffix(&service);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: canonical_rpc_key(&service_norm, &method),
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

/// Expand `server.addService(SvcDef, { m1: h, m2: h })` into one Consumer
/// emission per registered method when the call-arg extractor captured the
/// object-literal keys. Returns `None` when the call shape doesn't match
/// (lets the regular wildcard branch take over). Lowercases the service name
/// per the RPC canonical-key convention so PascalCase server registrations
/// pair with camelCase client chains.
pub(crate) fn detect_addservice_object_keys(
    chain: &crate::types::MemberChain,
    call_args: &[CallArg],
    file_ctx: &FileContext,
) -> Option<Vec<FlowEmission>> {
    if !file_imports_rpc_library(file_ctx) {
        return None;
    }
    if chain.segments.len() != 2 {
        return None;
    }
    let leaf = chain.segments.last()?;
    if leaf.name != "addService" {
        return None;
    }
    let Some(CallArg::Ident(ident)) = call_args.first() else {
        return None;
    };
    if !is_pascal_case_first(ident) {
        return None;
    }
    let Some(CallArg::ObjectKeys(pairs)) = call_args.get(1) else {
        return None;
    };
    if pairs.is_empty() {
        return None;
    }
    let service = strip_service_suffix(ident);
    let emissions = pairs
        .iter()
        .map(|(method, _value)| FlowEmission::NamedChannel {
            kind: NamedChannelKind::RpcCall,
            name: canonical_rpc_key(&service, method),
            role: ChannelRole::Consumer,
            method: None,
        streaming: None,
        })
        .collect();
    Some(emissions)
}

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

// ---------------------------------------------------------------------------
// NestJS HTTP route decorators — Consumer-role HttpCall emission
// ---------------------------------------------------------------------------

/// Inspect a TypeRef ref whose `target_name` is an HTTP-verb decorator
/// (`@Get`, `@Post`, `@Put`, `@Patch`, `@Delete`, `@Head`, `@Options`,
/// `@All`) and emit a `NamedChannel { kind: HttpCall, role: Consumer, .. }`.
///
/// The URL pattern is assembled by joining the enclosing class's
/// `@Controller(prefix)` value with this decorator's path argument, then
/// normalised via `connectors::url_pattern::normalize` so that `:id`,
/// `<id>`, `{id}` and `{}` all collapse to the canonical `{}`.
///
/// The controller prefix is looked up in `file_ctx.imports` under the
/// reserved `__ts_controller_prefix__:` synthetic key, which the
/// resolver's `build_file_context` populated during the class-decorator
/// pre-pass. When the prefix arg is not a string literal (`@Controller(RouteKey.User)`)
/// the lookup returns an empty string and the route is emitted using the
/// method-level path alone.
/// Match Angular's `@Injectable()` class decorator. Emits a single-ended
/// `DiBinding` with `container = "angular"` so the Angular service clusters
/// alongside NestJS providers in the architecture overview. Constructor
/// injection sites are not paired here — the resolver lacks cross-symbol
/// state to match a parameter type to its provider class.
pub(crate) fn detect_angular_injectable_emission(target: &str) -> Option<FlowEmission> {
    if target != "Injectable" {
        return None;
    }
    Some(FlowEmission::DiBinding {
        service_symbol_id: 0,
        container: Some("angular".to_string()),
    })
}

pub(crate) fn detect_route_decorator_flow_emission(
    decorator_name: &str,
    first_arg: Option<&str>,
    method_qname: &str,
    file_ctx: &FileContext,
) -> Option<FlowEmission> {
    let method = match decorator_name {
        "Get" => HttpMethod::Get,
        "Post" => HttpMethod::Post,
        "Put" => HttpMethod::Put,
        "Patch" => HttpMethod::Patch,
        "Delete" => HttpMethod::Delete,
        "Head" => HttpMethod::Head,
        "Options" => HttpMethod::Options,
        "All" => HttpMethod::Any,
        _ => return None,
    };

    // Gate: the file imports a NestJS-style decorator package. Without
    // this, `import type { Options as SWCOptions } from '@swc/core'` produces
    // a TypeRef ref with target_name="Options" that fires this detector and
    // emits a bogus `Options /@swc/core` consumer route.
    if !file_imports_nest_decorator_package(file_ctx) {
        return None;
    }

    // Two distinct ref shapes carry target_name="Get"/"Post"/etc.:
    //   1. The import statement `import { Get, Post, ... } from '@nestjs/common'`
    //      — extractor sets r.module to the import source. This is NOT a
    //      decorator usage; it should not emit anything.
    //   2. The decorator usage `@Get('path')` or `@Get()` — extractor sets
    //      r.module to the decorator's first string argument (or to the
    //      import source when the decorator has no argument, since both
    //      shapes share the same ref-construction path).
    //
    // Distinguishing case 1 from a no-arg case-2 isn't possible from this
    // layer alone. We sniff the `first_arg`: if it matches any file import
    // module_path it's never a real URL path — treat it as no argument and
    // let the controller prefix carry the route.
    let arg_is_import_source = first_arg
        .map(|a| file_ctx.imports.iter().any(|imp| imp.module_path.as_deref() == Some(a)))
        .unwrap_or(false);
    let path_arg: Option<&str> = if arg_is_import_source { None } else { first_arg };

    let class_qname = method_qname
        .rsplit_once('.')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let prefix = lookup_controller_prefix(class_qname, file_ctx).unwrap_or("");
    // No controller prefix AND no method-level path = an import statement
    // misread as a decorator. Real decorators usage either has a method path
    // or sits inside a @Controller(prefix)-annotated class.
    if prefix.is_empty() && path_arg.unwrap_or("").is_empty() {
        return None;
    }
    let path = path_arg.unwrap_or("");
    let joined = join_route_segments(prefix, path);
    let normalized = crate::connectors::url_pattern::normalize(&joined);

    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: normalized,
        role: ChannelRole::Consumer,
        method: Some(method),
    streaming: None,
    })
}

/// True when the file imports from a package that defines the NestJS-style
/// route decorators (`@Get`, `@Post`, …). Other packages may export types or
/// utilities sharing those names (`Options` from `@swc/core`, `Get<T>` from
/// generic libraries) and must not be misread as routing decorators.
fn file_imports_nest_decorator_package(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let Some(m) = imp.module_path.as_deref() else { return false; };
        m == "@nestjs/common"
            || m.starts_with("@nestjs/common/")
            || m == "@nestjs/microservices"
            || m == "@nestjs/websockets"
            || m == "@nestjs/graphql"
            // routing-controllers and tsoa use the same decorator names; gate
            // them in too rather than emitting bogus rows when they're not
            // imported.
            || m == "routing-controllers"
            || m == "tsoa"
    })
}

/// Resolve a controller class's `@Controller(prefix)` value from the file
/// context. Returns `None` when no class with that qualified name was
/// decorated, or an empty `Some("")` when the prefix arg was a non-literal
/// expression that the decorator extractor could not capture.
pub(crate) fn lookup_controller_prefix<'a>(
    class_qname: &str,
    file_ctx: &'a FileContext,
) -> Option<&'a str> {
    if class_qname.is_empty() {
        return None;
    }
    let key_prefix_len = CONTROLLER_PREFIX_KEY.len();
    file_ctx.imports.iter().find_map(|imp| {
        if imp.imported_name.len() > key_prefix_len
            && imp.imported_name.starts_with(CONTROLLER_PREFIX_KEY)
            && &imp.imported_name[key_prefix_len..] == class_qname
        {
            imp.module_path.as_deref()
        } else {
            None
        }
    })
}

/// Join a controller-level prefix and a method-level path into a single
/// URL pattern with exactly one `/` between them. Leading-slash and
/// trailing-slash handling mirrors NestJS's own router: empty parts are
/// dropped, and a single leading `/` is enforced.
pub(crate) fn join_route_segments(prefix: &str, path: &str) -> String {
    let p = prefix.trim().trim_end_matches('/');
    let q = path.trim().trim_start_matches('/');
    match (p.is_empty(), q.is_empty()) {
        (true, true) => String::from("/"),
        (true, false) => format!("/{}", q),
        (false, true) => {
            if p.starts_with('/') {
                p.to_string()
            } else {
                format!("/{}", p)
            }
        }
        (false, false) => {
            if p.starts_with('/') {
                format!("{}/{}", p, q)
            } else {
                format!("/{}/{}", p, q)
            }
        }
    }
}

