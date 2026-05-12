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
    assert_eq!(NamedChannelKind::MessageQueue.edge_type_str(), "message_queue");
}

#[test]
fn named_channel_kind_protocol_strings() {
    assert_eq!(NamedChannelKind::HttpCall.protocol_str(), "rest");
    assert_eq!(NamedChannelKind::GraphQLOp.protocol_str(), "graphql");
    assert_eq!(NamedChannelKind::WebSocket.protocol_str(), "websocket");
    assert_eq!(NamedChannelKind::IpcCall.protocol_str(), "ipc");
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
    let e = FlowEmission::ConfigLookup { key: "DATABASE_URL".to_string() };
    assert_eq!(e.edge_type(), "config_lookup");
    assert_eq!(e.url_pattern(), Some("DATABASE_URL"));
}

#[test]
fn feature_flag_emission_url_pattern_is_flag_name() {
    let e = FlowEmission::FeatureFlag { flag_name: "dark_mode".to_string() };
    assert_eq!(e.edge_type(), "feature_flag");
    assert_eq!(e.url_pattern(), Some("dark_mode"));
}
