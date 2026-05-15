use super::*;
use crate::types::*;

#[test]
fn test_elixir_ecto_repo_get_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_elixir_ecto_emission("Repo", "get").unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "ex.*");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_elixir_ecto_repo_insert_emits_db_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_elixir_ecto_emission("MyApp.Repo", "insert").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_elixir_ecto_rejects_non_repo() {
    assert!(detect_elixir_ecto_emission("Logger", "get").is_none());
}

#[test]
fn test_elixir_httpoison_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_elixir_http_emission("HTTPoison", "get", &args).unwrap() {
        FlowEmission::NamedChannel { kind, role, method, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(method, Some(HttpMethod::Get));
            assert_eq!(name, "/api/users");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_tesla_post_emits_producer() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let args = vec![CallArg::Ident("client".to_string()), CallArg::StringLit("/x".to_string())];
    assert!(matches!(
        detect_elixir_http_emission("Tesla", "post", &args).unwrap(),
        FlowEmission::NamedChannel { .. }
    ));
}

#[test]
fn test_elixir_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_elixir_http_emission("Logger", "get", &args).is_none());
}

#[test]
fn test_elixir_grpc_stub_emits_rpc_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    match detect_elixir_grpc_emission("Helloworld.Greeter.Stub", "say_hello").unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::RpcCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "Greeter.say_hello");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_grpc_rejects_non_stub() {
    assert!(detect_elixir_grpc_emission("MyApp.Service", "call").is_none());
}

#[test]
fn test_elixir_oban_insert_emits_bg_job() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    match detect_elixir_oban_emission("Oban", "insert", &[]).unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::BgJob));
            assert_eq!(name, "oban.job");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_bamboo_deliver_now_emits_mailer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    match detect_elixir_mailer_emission("MyApp.Mailer", "deliver_now").unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::Mailer));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "ex.Mailer");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_swoosh_deliver_emits_mailer() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    match detect_elixir_mailer_emission("MyApp.UserMailer", "deliver").unwrap() {
        FlowEmission::NamedChannel { kind, .. } => {
            assert!(matches!(kind, NamedChannelKind::Mailer));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_mailer_rejects_non_mailer_module() {
    assert!(detect_elixir_mailer_emission("Logger", "deliver").is_none());
}

#[test]
fn test_elixir_phoenix_channel_use_emits_ws_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    match detect_elixir_phoenix_channel_use("Phoenix.Channel", None).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::WebSocket));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "ex.Phoenix.Channel");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_phoenix_live_view_recognised() {
    assert!(detect_elixir_phoenix_channel_use("Phoenix.LiveView", None).is_some());
}

#[test]
fn test_elixir_phoenix_channel_rejects_non_phoenix_module() {
    assert!(detect_elixir_phoenix_channel_use("Logger", None).is_none());
    assert!(detect_elixir_phoenix_channel_use("Ecto.Schema", None).is_none());
}
