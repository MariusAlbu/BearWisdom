// =============================================================================
// languages/typescript/flow_detectors/decorators.rs — decorator-based detectors
//
// Recognises FlowEmission patterns expressed as TypeScript decorators:
// NestJS `@Controller` / route verbs / `@UseGuards` / `@Roles`,
// Angular `@Injectable`, TypeORM `@Entity` / `@Table`, NestJS
// `@GrpcMethod` server handlers, Connect `addService` object-literal
// handler maps. Decorator detection runs against a TypeRef ref whose
// `target_name` is the decorator identifier.
// =============================================================================

use crate::indexer::resolve::engine::{FileContext, ImportEntry};
use crate::indexer::resolve::flow_emit::{
    AuthGuardKind, ChannelRole, FlowEmission, HttpMethod, MigrationDirection, NamedChannelKind,
    StreamKind,
};
use crate::types::CallArg;

use super::db::{file_imports_orm_decorator_package, is_pascal_case_first};
use super::first_arg_string;
use super::messaging::{canonical_rpc_key, file_imports_rpc_library, strip_service_suffix};

/// Reserved synthetic-import key prefix used to thread `@Controller(prefix)`
/// metadata through `FileContext.imports` so that method-level HTTP-verb
/// decorators can recover the class's route prefix from the file context
/// alone. Concatenated with the controller class's qualified name.
pub(crate) const CONTROLLER_PREFIX_KEY: &str = "__ts_controller_prefix__:";

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

