// =============================================================================
// indexer/resolve/flow_emit.rs — Resolver-emitted flow-edge signals
//
// A `LanguageResolver::resolve` impl returns `Resolution { flow_emit: Some(e) }`
// when the resolved ref's shape matches a cross-tier flow-edge pattern.
//
// The resolve loop accumulates these alongside the edge tuples in
// `FileWriteBuf::flow_emissions`, then bulk-writes them to `flow_edges` after
// the main edge transaction commits — no second scan, no per-framework
// recogniser files, no hardcoded library-symbol lists beyond the small
// `is_http_client_module` predicate in the TS resolver.
//
// All variants are language-agnostic. Per-language resolvers decide WHEN to
// emit (based on what the chain walker resolved), but the SHAPE is universal.
// =============================================================================

/// The HTTP verb for `NamedChannel { kind: HttpCall }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
    /// Sentinel when the method cannot be determined from the call shape.
    Any,
}

impl HttpMethod {
    /// The canonical uppercase string written to `flow_edges.http_method`.
    pub fn as_str(self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
            HttpMethod::Any => "*",
        }
    }

    /// Parse from a lowercase method name as it appears on the chain segment
    /// (e.g. `"get"` from `axios.get(...)`, `"post"` from `fetch(..., {method:"POST"})`).
    pub fn from_method_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "get" => HttpMethod::Get,
            "post" => HttpMethod::Post,
            "put" => HttpMethod::Put,
            "patch" => HttpMethod::Patch,
            "delete" | "del" => HttpMethod::Delete,
            "head" => HttpMethod::Head,
            "options" => HttpMethod::Options,
            _ => HttpMethod::Any,
        }
    }
}

/// The direction of a `NamedChannel` emission — whether this site produces
/// (initiates) or consumes (handles) the channel message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRole {
    /// Call site that initiates the exchange: HTTP client call, `emit`, `invoke`, etc.
    Producer,
    /// Handler that receives the exchange: route handler, `on`, `ipcMain.handle`, etc.
    Consumer,
}

/// The semantic protocol family for `NamedChannel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedChannelKind {
    /// `fetch(url)`, `axios.get(url)`, `ofetch(url)` — HTTP client call.
    HttpCall,
    /// `apolloClient.query({ query: gql`...` })`, `useQuery(DOC)` — GraphQL operation.
    GraphQLOp,
    /// `client.SomeService.Method()` — gRPC / Twirp / Connect client call.
    RpcCall,
    /// `socket.emit('event')` / `socket.on('event')` — WebSocket message.
    WebSocket,
    /// `invoke('cmd', args)` from `@tauri-apps/api`, `ipcRenderer.invoke` — IPC command.
    IpcCall,
    /// `queue.send(topic, payload)`, BullMQ `queue.add(name, data)` — background job.
    BgJob,
    /// Email / push template dispatch.
    Mailer,
    /// Pub/sub topic publish / subscribe.
    MessageQueue,
}

impl NamedChannelKind {
    /// The `edge_type` value written to `flow_edges.edge_type`.
    pub fn edge_type_str(self) -> &'static str {
        match self {
            NamedChannelKind::HttpCall => "http_call",
            NamedChannelKind::GraphQLOp => "graphql_op",
            NamedChannelKind::RpcCall => "rpc_call",
            NamedChannelKind::WebSocket => "websocket",
            NamedChannelKind::IpcCall => "ipc_call",
            NamedChannelKind::BgJob => "bg_job",
            NamedChannelKind::Mailer => "mailer",
            NamedChannelKind::MessageQueue => "message_queue",
        }
    }

    /// The `protocol` value written to `flow_edges.protocol`.
    pub fn protocol_str(self) -> &'static str {
        match self {
            NamedChannelKind::HttpCall => "rest",
            NamedChannelKind::GraphQLOp => "graphql",
            NamedChannelKind::RpcCall => "grpc",
            NamedChannelKind::WebSocket => "websocket",
            NamedChannelKind::IpcCall => "ipc",
            NamedChannelKind::BgJob => "bg_job",
            NamedChannelKind::Mailer => "mailer",
            NamedChannelKind::MessageQueue => "message_queue",
        }
    }
}

/// Emitted by a `LanguageResolver::resolve` impl when the resolved ref's
/// shape matches a cross-tier flow-edge pattern.
///
/// The resolve loop collects these in `FileWriteBuf::flow_emissions` and
/// bulk-writes them to `flow_edges` after the main resolution transaction
/// commits. Single-ended variants (no target_file_id) record call sites;
/// pair-capable variants are eligible for future producer↔consumer matching
/// once handler-side resolvers also emit.
#[derive(Debug, Clone)]
pub enum FlowEmission {
    /// Cross-tier call/handler pair keyed by a string name.
    ///
    /// Covers HTTP calls, GraphQL operations, WebSocket events, IPC commands,
    /// background jobs, mailer templates, and message-queue topics.
    ///
    /// Pairing rule (future): `(kind, name, role=Producer)` ↔
    ///                         `(kind, name, role=Consumer)` across files.
    NamedChannel {
        kind: NamedChannelKind,
        /// Pairing key: URL path, event name, operation name, command name, etc.
        /// Empty string when not statically determinable.
        name: String,
        role: ChannelRole,
        /// HTTP method when `kind == HttpCall`, None for other kinds.
        method: Option<HttpMethod>,
    },

    /// A DI-injected service binding: field or constructor-parameter type
    /// resolves to a service class symbol.
    DiBinding {
        /// DB id of the injected service's symbol.
        service_symbol_id: i64,
        /// Container hint when determinable: "angular", "nestjs", "dotnet",
        /// "spring", "guice", "tauri", "manual".
        container: Option<String>,
    },

    /// Environment variable / config key read.
    ConfigLookup {
        key: String,
    },

    /// Feature flag evaluation.
    FeatureFlag {
        flag_name: String,
    },
}

impl FlowEmission {
    /// The `edge_type` string for this emission.
    pub fn edge_type(&self) -> &str {
        match self {
            FlowEmission::NamedChannel { kind, .. } => kind.edge_type_str(),
            FlowEmission::DiBinding { .. } => "di_binding",
            FlowEmission::ConfigLookup { .. } => "config_lookup",
            FlowEmission::FeatureFlag { .. } => "feature_flag",
        }
    }

    /// The `protocol` string for this emission.
    pub fn protocol(&self) -> Option<&str> {
        match self {
            FlowEmission::NamedChannel { kind, .. } => Some(kind.protocol_str()),
            _ => None,
        }
    }

    /// The `http_method` string, if applicable.
    pub fn http_method_str(&self) -> Option<&str> {
        match self {
            FlowEmission::NamedChannel { kind: NamedChannelKind::HttpCall, method, .. } => {
                method.map(|m| m.as_str())
            }
            _ => None,
        }
    }

    /// The URL pattern / event name / key — written to `flow_edges.url_pattern`.
    pub fn url_pattern(&self) -> Option<&str> {
        match self {
            FlowEmission::NamedChannel { name, .. } if !name.is_empty() => Some(name.as_str()),
            FlowEmission::ConfigLookup { key } => Some(key.as_str()),
            FlowEmission::FeatureFlag { flag_name } => Some(flag_name.as_str()),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "flow_emit_tests.rs"]
mod tests;
