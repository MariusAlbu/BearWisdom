// Tests for flow_emit.rs — FlowEmission enum and helpers.

use super::*;

// ---------------------------------------------------------------------------
// HttpMethod
// ---------------------------------------------------------------------------

#[test]
fn http_method_from_name_get() {
    assert_eq!(HttpMethod::from_method_name("get"), HttpMethod::Get);
    assert_eq!(HttpMethod::from_method_name("GET"), HttpMethod::Get);
}

#[test]
fn http_method_from_name_post() {
    assert_eq!(HttpMethod::from_method_name("post"), HttpMethod::Post);
}

#[test]
fn http_method_from_name_delete_alias() {
    assert_eq!(HttpMethod::from_method_name("del"), HttpMethod::Delete);
}

#[test]
fn http_method_from_name_unknown_falls_back_to_any() {
    assert_eq!(HttpMethod::from_method_name("request"), HttpMethod::Any);
}

#[test]
fn http_method_as_str_roundtrip() {
    assert_eq!(HttpMethod::Get.as_str(), "GET");
    assert_eq!(HttpMethod::Post.as_str(), "POST");
    assert_eq!(HttpMethod::Put.as_str(), "PUT");
    assert_eq!(HttpMethod::Patch.as_str(), "PATCH");
    assert_eq!(HttpMethod::Delete.as_str(), "DELETE");
    assert_eq!(HttpMethod::Head.as_str(), "HEAD");
    assert_eq!(HttpMethod::Options.as_str(), "OPTIONS");
    assert_eq!(HttpMethod::Any.as_str(), "*");
}

// ---------------------------------------------------------------------------
// NamedChannelKind
// ---------------------------------------------------------------------------

#[test]
fn named_channel_kind_edge_type_strings() {
    assert_eq!(NamedChannelKind::HttpCall.edge_type_str(), "http_call");
    assert_eq!(NamedChannelKind::GraphQLOp.edge_type_str(), "graphql_op");
    assert_eq!(NamedChannelKind::WebSocket.edge_type_str(), "websocket");
    assert_eq!(NamedChannelKind::IpcCall.edge_type_str(), "ipc_call");
    assert_eq!(NamedChannelKind::BgJob.edge_type_str(), "bg_job");
    assert_eq!(NamedChannelKind::Mailer.edge_type_str(), "mailer");
    assert_eq!(
        NamedChannelKind::MessageQueue.edge_type_str(),
        "message_queue"
    );
    assert_eq!(NamedChannelKind::EventBus.edge_type_str(), "event_bus");
}

#[test]
fn named_channel_kind_protocol_strings() {
    assert_eq!(NamedChannelKind::HttpCall.protocol_str(), "rest");
    assert_eq!(NamedChannelKind::GraphQLOp.protocol_str(), "graphql");
    assert_eq!(NamedChannelKind::WebSocket.protocol_str(), "websocket");
    assert_eq!(NamedChannelKind::IpcCall.protocol_str(), "ipc");
    assert_eq!(NamedChannelKind::EventBus.protocol_str(), "event_bus");
}

// ---------------------------------------------------------------------------
// NamedChannelKind::to_flow_edge_kind
// ---------------------------------------------------------------------------

#[test]
fn named_channel_kind_to_flow_edge_kind_role_invariant() {
    use crate::types::FlowEdgeKind;
    assert_eq!(
        NamedChannelKind::HttpCall.to_flow_edge_kind(ChannelRole::Producer),
        FlowEdgeKind::HttpCall,
    );
    assert_eq!(
        NamedChannelKind::HttpCall.to_flow_edge_kind(ChannelRole::Consumer),
        FlowEdgeKind::HttpCall,
    );
    assert_eq!(
        NamedChannelKind::GraphQLOp.to_flow_edge_kind(ChannelRole::Producer),
        FlowEdgeKind::GraphQLOp,
    );
    assert_eq!(
        NamedChannelKind::WebSocket.to_flow_edge_kind(ChannelRole::Producer),
        FlowEdgeKind::WebSocket,
    );
    assert_eq!(
        NamedChannelKind::RpcCall.to_flow_edge_kind(ChannelRole::Producer),
        FlowEdgeKind::RpcCall,
    );
    assert_eq!(
        NamedChannelKind::IpcCall.to_flow_edge_kind(ChannelRole::Producer),
        FlowEdgeKind::IpcCall,
    );
    assert_eq!(
        NamedChannelKind::BgJob.to_flow_edge_kind(ChannelRole::Producer),
        FlowEdgeKind::BgJob,
    );
    assert_eq!(
        NamedChannelKind::Mailer.to_flow_edge_kind(ChannelRole::Producer),
        FlowEdgeKind::Mailer,
    );
}

#[test]
fn named_channel_kind_to_flow_edge_kind_role_split() {
    use crate::types::FlowEdgeKind;
    assert_eq!(
        NamedChannelKind::MessageQueue.to_flow_edge_kind(ChannelRole::Producer),
        FlowEdgeKind::QueueProduce,
    );
    assert_eq!(
        NamedChannelKind::MessageQueue.to_flow_edge_kind(ChannelRole::Consumer),
        FlowEdgeKind::QueueConsume,
    );
    assert_eq!(
        NamedChannelKind::EventBus.to_flow_edge_kind(ChannelRole::Producer),
        FlowEdgeKind::EventEmit,
    );
    assert_eq!(
        NamedChannelKind::EventBus.to_flow_edge_kind(ChannelRole::Consumer),
        FlowEdgeKind::EventHandle,
    );
}

// ---------------------------------------------------------------------------
// FlowEmission helpers
// ---------------------------------------------------------------------------

#[test]
fn http_call_emission_edge_type_and_protocol() {
    let e = FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: "/api/users".to_string(),
        role: ChannelRole::Producer,
        method: Some(HttpMethod::Get),
        streaming: None,
    };
    assert_eq!(e.edge_type(), "http_call");
    assert_eq!(e.protocol(), Some("rest"));
    assert_eq!(e.http_method_str(), Some("GET"));
    assert_eq!(e.url_pattern(), Some("/api/users"));
}

#[test]
fn http_call_empty_name_yields_no_url_pattern() {
    let e = FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: String::new(),
        role: ChannelRole::Producer,
        method: Some(HttpMethod::Post),
        streaming: None,
    };
    assert_eq!(e.url_pattern(), None);
}

#[test]
fn websocket_emission_no_http_method() {
    let e = FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: "message".to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    };
    assert_eq!(e.edge_type(), "websocket");
    assert_eq!(e.http_method_str(), None);
    assert_eq!(e.url_pattern(), Some("message"));
}

#[test]
fn ipc_call_emission_edge_type() {
    let e = FlowEmission::NamedChannel {
        kind: NamedChannelKind::IpcCall,
        name: "get-settings".to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    };
    assert_eq!(e.edge_type(), "ipc_call");
    assert_eq!(e.protocol(), Some("ipc"));
}

#[test]
fn di_binding_emission() {
    let e = FlowEmission::DiBinding {
        service_symbol_id: 42,
        container: Some("angular".to_string()),
    };
    assert_eq!(e.edge_type(), "di_binding");
    assert_eq!(e.protocol(), None);
    assert_eq!(e.http_method_str(), None);
    assert_eq!(e.url_pattern(), None);
}

#[test]
fn config_lookup_emission_url_pattern_is_key() {
    let e = FlowEmission::ConfigLookup {
        key: "DATABASE_URL".to_string(),
    };
    assert_eq!(e.edge_type(), "config_lookup");
    assert_eq!(e.url_pattern(), Some("DATABASE_URL"));
}

#[test]
fn feature_flag_emission_url_pattern_is_flag_name() {
    let e = FlowEmission::FeatureFlag {
        flag_name: "dark_mode".to_string(),
    };
    assert_eq!(e.edge_type(), "feature_flag");
    assert_eq!(e.url_pattern(), Some("dark_mode"));
}

// ---------------------------------------------------------------------------
// DbQueryOp
// ---------------------------------------------------------------------------

#[test]
fn db_query_op_from_method_name_select_variants() {
    assert_eq!(DbQueryOp::from_method_name("find"), DbQueryOp::Select);
    assert_eq!(DbQueryOp::from_method_name("findOne"), DbQueryOp::Select);
    assert_eq!(DbQueryOp::from_method_name("findUnique"), DbQueryOp::Select);
    assert_eq!(DbQueryOp::from_method_name("select"), DbQueryOp::Select);
    assert_eq!(DbQueryOp::from_method_name("count"), DbQueryOp::Select);
    assert_eq!(DbQueryOp::from_method_name("where"), DbQueryOp::Select);
    assert_eq!(DbQueryOp::from_method_name("query"), DbQueryOp::Select);
}

#[test]
fn db_query_op_from_method_name_insert_variants() {
    assert_eq!(DbQueryOp::from_method_name("create"), DbQueryOp::Insert);
    assert_eq!(DbQueryOp::from_method_name("insert"), DbQueryOp::Insert);
    assert_eq!(DbQueryOp::from_method_name("save"), DbQueryOp::Insert);
    assert_eq!(DbQueryOp::from_method_name("add"), DbQueryOp::Insert);
}

#[test]
fn db_query_op_from_method_name_update_variants() {
    assert_eq!(DbQueryOp::from_method_name("update"), DbQueryOp::Update);
    assert_eq!(DbQueryOp::from_method_name("set"), DbQueryOp::Update);
    assert_eq!(DbQueryOp::from_method_name("patch"), DbQueryOp::Update);
    assert_eq!(DbQueryOp::from_method_name("updateOne"), DbQueryOp::Update);
}

#[test]
fn db_query_op_from_method_name_delete_variants() {
    assert_eq!(DbQueryOp::from_method_name("delete"), DbQueryOp::Delete);
    assert_eq!(DbQueryOp::from_method_name("remove"), DbQueryOp::Delete);
    assert_eq!(DbQueryOp::from_method_name("destroy"), DbQueryOp::Delete);
    assert_eq!(DbQueryOp::from_method_name("deleteMany"), DbQueryOp::Delete);
}

#[test]
fn db_query_op_from_method_name_upsert() {
    assert_eq!(DbQueryOp::from_method_name("upsert"), DbQueryOp::Upsert);
}

#[test]
fn db_query_op_from_method_name_unknown_falls_back_to_other() {
    assert_eq!(DbQueryOp::from_method_name("migrate"), DbQueryOp::Other);
    assert_eq!(DbQueryOp::from_method_name("transaction"), DbQueryOp::Other);
}

#[test]
fn db_query_op_as_str_roundtrip() {
    assert_eq!(DbQueryOp::Select.as_str(), "select");
    assert_eq!(DbQueryOp::Insert.as_str(), "insert");
    assert_eq!(DbQueryOp::Update.as_str(), "update");
    assert_eq!(DbQueryOp::Delete.as_str(), "delete");
    assert_eq!(DbQueryOp::Upsert.as_str(), "upsert");
    assert_eq!(DbQueryOp::Other.as_str(), "other");
}

// ---------------------------------------------------------------------------
// New FlowEmission variants — edge_type / url_pattern / is_single_ended
// ---------------------------------------------------------------------------

#[test]
fn db_entity_with_table_name_hint() {
    let e = FlowEmission::DbEntity {
        base_symbol_id: None,
        base_name_hint: "Entity".to_string(),
        table_name_hint: Some("users".to_string()),
    };
    assert_eq!(e.edge_type(), "db_entity");
    assert_eq!(e.url_pattern(), Some("users"));
    assert!(!e.is_single_ended());
}

#[test]
fn db_entity_without_table_name_falls_back_to_base_name() {
    let e = FlowEmission::DbEntity {
        base_symbol_id: Some(7),
        base_name_hint: "Model".to_string(),
        table_name_hint: None,
    };
    assert_eq!(e.url_pattern(), Some("Model"));
}

#[test]
fn db_query_emission_fields() {
    let e = FlowEmission::DbQuery {
        entity_name: "User".to_string(),
        operation: DbQueryOp::Select,
    };
    assert_eq!(e.edge_type(), "db_query");
    assert_eq!(e.url_pattern(), Some("User"));
    assert!(!e.is_single_ended());
}

#[test]
fn migration_target_emission() {
    let e = FlowEmission::MigrationTarget {
        table_name: "orders".to_string(),
        direction: MigrationDirection::Up,
    };
    assert_eq!(e.edge_type(), "migration_target");
    assert_eq!(e.url_pattern(), Some("orders"));
    assert!(!e.is_single_ended());
}

#[test]
fn auth_guard_role_emission() {
    let e = FlowEmission::AuthGuard {
        requirement: "admin".to_string(),
        kind: AuthGuardKind::Role,
    };
    assert_eq!(e.edge_type(), "auth_guard");
    assert_eq!(e.url_pattern(), Some("admin"));
    assert_eq!(AuthGuardKind::Role.as_str(), "role");
    assert!(e.is_single_ended());
}

#[test]
fn auth_guard_permission_emission() {
    let e = FlowEmission::AuthGuard {
        requirement: "read:users".to_string(),
        kind: AuthGuardKind::Permission,
    };
    assert_eq!(AuthGuardKind::Permission.as_str(), "permission");
    assert_eq!(e.url_pattern(), Some("read:users"));
    assert!(e.is_single_ended());
}

#[test]
fn auth_guard_policy_emission() {
    let e = FlowEmission::AuthGuard {
        requirement: "IsOwner".to_string(),
        kind: AuthGuardKind::Policy,
    };
    assert_eq!(AuthGuardKind::Policy.as_str(), "policy");
    assert!(e.is_single_ended());
}

#[test]
fn auth_guard_token_emission() {
    let e = FlowEmission::AuthGuard {
        requirement: "JwtAuthGuard".to_string(),
        kind: AuthGuardKind::Token,
    };
    assert_eq!(AuthGuardKind::Token.as_str(), "token");
    assert!(e.is_single_ended());
}

#[test]
fn cli_command_emission() {
    let e = FlowEmission::CliCommand {
        command_name: "build".to_string(),
        framework: Some("commander".to_string()),
    };
    assert_eq!(e.edge_type(), "cli_command");
    assert_eq!(e.url_pattern(), Some("build"));
    assert!(e.is_single_ended());
}

#[test]
fn cli_command_no_framework() {
    let e = FlowEmission::CliCommand {
        command_name: "serve".to_string(),
        framework: None,
    };
    assert_eq!(e.url_pattern(), Some("serve"));
    assert!(e.is_single_ended());
}

#[test]
fn scheduled_job_emission() {
    let e = FlowEmission::ScheduledJob {
        schedule: "0 0 * * *".to_string(),
    };
    assert_eq!(e.edge_type(), "scheduled_job");
    assert_eq!(e.url_pattern(), Some("0 0 * * *"));
    assert!(e.is_single_ended());
}

// ---------------------------------------------------------------------------
// is_single_ended — coverage for paired variants
// ---------------------------------------------------------------------------

#[test]
fn named_channel_is_not_single_ended() {
    let e = FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: "/api".to_string(),
        role: ChannelRole::Producer,
        method: Some(HttpMethod::Get),
        streaming: None,
    };
    assert!(!e.is_single_ended());
}

#[test]
fn db_entity_is_not_single_ended() {
    let e = FlowEmission::DbEntity {
        base_symbol_id: None,
        base_name_hint: "Entity".to_string(),
        table_name_hint: None,
    };
    assert!(!e.is_single_ended());
}

#[test]
fn db_query_is_not_single_ended() {
    let e = FlowEmission::DbQuery {
        entity_name: "Post".to_string(),
        operation: DbQueryOp::Insert,
    };
    assert!(!e.is_single_ended());
}

#[test]
fn migration_target_is_not_single_ended() {
    let e = FlowEmission::MigrationTarget {
        table_name: "events".to_string(),
        direction: MigrationDirection::Down,
    };
    assert!(!e.is_single_ended());
}

// ---------------------------------------------------------------------------
// StreamKind
// ---------------------------------------------------------------------------

#[test]
fn stream_kind_as_str() {
    assert_eq!(StreamKind::Unary.as_str(), "unary");
    assert_eq!(StreamKind::ServerStreaming.as_str(), "server_streaming");
    assert_eq!(StreamKind::ClientStreaming.as_str(), "client_streaming");
    assert_eq!(StreamKind::BidiStreaming.as_str(), "bidi_streaming");
}

#[test]
fn stream_kind_from_method_name_recognises_server_streaming_prefix() {
    assert_eq!(
        StreamKind::from_method_name("stream_events"),
        StreamKind::ServerStreaming
    );
    assert_eq!(
        StreamKind::from_method_name("subscribe_orders"),
        StreamKind::ServerStreaming
    );
    assert_eq!(
        StreamKind::from_method_name("watch_pods"),
        StreamKind::ServerStreaming
    );
}

#[test]
fn stream_kind_from_method_name_recognises_server_streaming_suffix() {
    assert_eq!(
        StreamKind::from_method_name("get_events_stream"),
        StreamKind::ServerStreaming
    );
}

#[test]
fn stream_kind_from_method_name_recognises_client_streaming() {
    assert_eq!(
        StreamKind::from_method_name("upload_chunks"),
        StreamKind::ClientStreaming
    );
    assert_eq!(
        StreamKind::from_method_name("record_route"),
        StreamKind::ClientStreaming
    );
    assert_eq!(
        StreamKind::from_method_name("collect_metrics"),
        StreamKind::ClientStreaming
    );
}

#[test]
fn stream_kind_from_method_name_recognises_bidi() {
    assert_eq!(
        StreamKind::from_method_name("chat"),
        StreamKind::BidiStreaming
    );
    assert_eq!(
        StreamKind::from_method_name("dialog_session"),
        StreamKind::BidiStreaming
    );
    assert_eq!(
        StreamKind::from_method_name("route_bidi"),
        StreamKind::BidiStreaming
    );
}

#[test]
fn stream_kind_from_method_name_defaults_to_unary() {
    assert_eq!(StreamKind::from_method_name("get_user"), StreamKind::Unary);
    assert_eq!(
        StreamKind::from_method_name("create_order"),
        StreamKind::Unary
    );
    assert_eq!(
        StreamKind::from_method_name("HelloWorld"),
        StreamKind::Unary
    );
}

#[test]
fn rpc_call_emission_carries_streaming_kind() {
    let e = FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: "User.GetUser".to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::ServerStreaming),
    };
    assert_eq!(e.streaming_str(), Some("server_streaming"));
}

#[test]
fn rpc_call_unary_yields_no_streaming_metadata_string() {
    let e = FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: "User.GetUser".to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::Unary),
    };
    assert_eq!(e.streaming_str(), None);
}

#[test]
fn http_call_streaming_str_is_always_none() {
    let e = FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: "/api".to_string(),
        role: ChannelRole::Producer,
        method: Some(HttpMethod::Get),
        streaming: Some(StreamKind::ServerStreaming),
    };
    // streaming_str gates on kind == RpcCall.
    assert_eq!(e.streaming_str(), None);
}

// ---------------------------------------------------------------------------
// FlowEmission::flow_edge_kind
// ---------------------------------------------------------------------------

#[test]
fn flow_edge_kind_named_channel_delegates_to_channel_kind() {
    use crate::types::FlowEdgeKind;
    let e = FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: "/api".to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    };
    assert_eq!(e.flow_edge_kind(), FlowEdgeKind::HttpCall);
}

#[test]
fn flow_edge_kind_event_bus_producer_maps_to_event_emit() {
    use crate::types::FlowEdgeKind;
    let e = FlowEmission::NamedChannel {
        kind: NamedChannelKind::EventBus,
        name: "user.created".to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    };
    assert_eq!(e.flow_edge_kind(), FlowEdgeKind::EventEmit);
}

#[test]
fn flow_edge_kind_event_bus_consumer_maps_to_event_handle() {
    use crate::types::FlowEdgeKind;
    let e = FlowEmission::NamedChannel {
        kind: NamedChannelKind::EventBus,
        name: "user.created".to_string(),
        role: ChannelRole::Consumer,
        method: None,
        streaming: None,
    };
    assert_eq!(e.flow_edge_kind(), FlowEdgeKind::EventHandle);
}

#[test]
fn flow_edge_kind_message_queue_role_split() {
    use crate::types::FlowEdgeKind;
    let prod = FlowEmission::NamedChannel {
        kind: NamedChannelKind::MessageQueue,
        name: "orders".to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    };
    let cons = FlowEmission::NamedChannel {
        kind: NamedChannelKind::MessageQueue,
        name: "orders".to_string(),
        role: ChannelRole::Consumer,
        method: None,
        streaming: None,
    };
    assert_eq!(prod.flow_edge_kind(), FlowEdgeKind::QueueProduce);
    assert_eq!(cons.flow_edge_kind(), FlowEdgeKind::QueueConsume);
}

#[test]
fn flow_edge_kind_non_channel_variants() {
    use crate::types::FlowEdgeKind;
    assert_eq!(
        FlowEmission::DbEntity {
            base_symbol_id: None,
            base_name_hint: "Model".to_string(),
            table_name_hint: None,
        }
        .flow_edge_kind(),
        FlowEdgeKind::DbEntity,
    );
    assert_eq!(
        FlowEmission::DbQuery {
            entity_name: "User".to_string(),
            operation: DbQueryOp::Select,
        }
        .flow_edge_kind(),
        FlowEdgeKind::DbQuery,
    );
    assert_eq!(
        FlowEmission::MigrationTarget {
            table_name: "users".to_string(),
            direction: MigrationDirection::Up,
        }
        .flow_edge_kind(),
        FlowEdgeKind::MigrationTarget,
    );
    assert_eq!(
        FlowEmission::DiBinding {
            service_symbol_id: 1,
            container: None,
        }
        .flow_edge_kind(),
        FlowEdgeKind::DiBinding,
    );
    assert_eq!(
        FlowEmission::ConfigLookup {
            key: "DATABASE_URL".to_string()
        }
        .flow_edge_kind(),
        FlowEdgeKind::ConfigLookup,
    );
    assert_eq!(
        FlowEmission::FeatureFlag {
            flag_name: "dark_mode".to_string()
        }
        .flow_edge_kind(),
        FlowEdgeKind::FeatureFlag,
    );
    assert_eq!(
        FlowEmission::AuthGuard {
            requirement: "admin".to_string(),
            kind: AuthGuardKind::Role,
        }
        .flow_edge_kind(),
        FlowEdgeKind::AuthGuard,
    );
    assert_eq!(
        FlowEmission::CliCommand {
            command_name: "deploy".to_string(),
            framework: None,
        }
        .flow_edge_kind(),
        FlowEdgeKind::CliCommand,
    );
    assert_eq!(
        FlowEmission::ScheduledJob {
            schedule: "0 * * * *".to_string()
        }
        .flow_edge_kind(),
        FlowEdgeKind::ScheduledJob,
    );
}
