use super::*;
use std::str::FromStr;

#[test]
fn flow_edge_kind_roundtrip() {
    for (kind, expected_str) in [
        (FlowEdgeKind::HttpCall, "http_call"),
        (FlowEdgeKind::GraphQLOp, "graphql_op"),
        (FlowEdgeKind::RpcCall, "rpc_call"),
        (FlowEdgeKind::RpcHandle, "rpc_handle"),
        (FlowEdgeKind::WebSocket, "websocket"),
        (FlowEdgeKind::IpcCall, "ipc_call"),
        (FlowEdgeKind::BgJob, "bg_job"),
        (FlowEdgeKind::Mailer, "mailer"),
        (FlowEdgeKind::DbEntity, "db_entity"),
        (FlowEdgeKind::DbQuery, "db_query"),
        (FlowEdgeKind::MigrationTarget, "migration_target"),
        (FlowEdgeKind::EventEmit, "event_emit"),
        (FlowEdgeKind::EventHandle, "event_handle"),
        (FlowEdgeKind::QueueProduce, "queue_produce"),
        (FlowEdgeKind::QueueConsume, "queue_consume"),
        (FlowEdgeKind::DiBinding, "di_binding"),
        (FlowEdgeKind::ConfigLookup, "config_lookup"),
        (FlowEdgeKind::FeatureFlag, "feature_flag"),
        (FlowEdgeKind::AuthGuard, "auth_guard"),
        (FlowEdgeKind::CliCommand, "cli_command"),
        (FlowEdgeKind::ScheduledJob, "scheduled_job"),
        (FlowEdgeKind::LspResolved, "lsp_resolved"),
    ] {
        assert_eq!(kind.as_str(), expected_str, "as_str mismatch for {kind:?}");
        let back = FlowEdgeKind::from_str(expected_str).ok();
        assert_eq!(back, Some(kind), "from_str round-trip failed for {kind:?}");
    }
}

#[test]
fn flow_edge_kind_display_matches_as_str() {
    for kind in [
        FlowEdgeKind::HttpCall,
        FlowEdgeKind::GraphQLOp,
        FlowEdgeKind::WebSocket,
        FlowEdgeKind::DiBinding,
    ] {
        assert_eq!(kind.to_string(), kind.as_str());
    }
}

#[test]
fn symbol_kind_roundtrip() {
    for kind in [
        SymbolKind::Class,
        SymbolKind::Struct,
        SymbolKind::Interface,
        SymbolKind::Enum,
        SymbolKind::EnumMember,
        SymbolKind::Method,
        SymbolKind::Constructor,
        SymbolKind::Property,
        SymbolKind::Field,
        SymbolKind::Namespace,
        SymbolKind::Event,
        SymbolKind::Delegate,
        SymbolKind::Function,
        SymbolKind::TypeAlias,
        SymbolKind::Variable,
        SymbolKind::Test,
    ] {
        let s = kind.as_str();
        let back = SymbolKind::from_str(s).ok();
        assert_eq!(back, Some(kind), "round-trip failed for {kind:?}");
    }
}

#[test]
fn symbol_kind_display_matches_as_str() {
    for kind in [
        SymbolKind::Class,
        SymbolKind::EnumMember,
        SymbolKind::TypeAlias,
        SymbolKind::Test,
    ] {
        assert_eq!(kind.to_string(), kind.as_str());
    }
}

#[test]
fn edge_kind_roundtrip() {
    for kind in [
        EdgeKind::Calls,
        EdgeKind::Inherits,
        EdgeKind::Implements,
        EdgeKind::TypeRef,
        EdgeKind::Instantiates,
        EdgeKind::Imports,
        EdgeKind::HttpCall,
        EdgeKind::DbEntity,
        EdgeKind::LspResolved,
    ] {
        let s = kind.as_str();
        let back = EdgeKind::from_str(s).ok();
        assert_eq!(back, Some(kind), "round-trip failed for {kind:?}");
    }
}

#[test]
fn edge_kind_display_matches_as_str() {
    for kind in [
        EdgeKind::HttpCall,
        EdgeKind::LspResolved,
        EdgeKind::DbEntity,
    ] {
        assert_eq!(kind.to_string(), kind.as_str());
    }
}

#[test]
fn visibility_roundtrip() {
    for v in [
        Visibility::Public,
        Visibility::Private,
        Visibility::Protected,
        Visibility::Internal,
    ] {
        let s = v.as_str();
        let back = Visibility::from_str(s).ok();
        assert_eq!(back, Some(v), "round-trip failed for {v:?}");
    }
}

#[test]
fn visibility_display_matches_as_str() {
    for v in [Visibility::Public, Visibility::Protected] {
        assert_eq!(v.to_string(), v.as_str());
    }
}

#[test]
fn unknown_strings_return_none() {
    assert!(SymbolKind::from_str("Class").is_err()); // PascalCase rejected
    assert!(SymbolKind::from_str("").is_err());
    assert!(EdgeKind::from_str("http-call").is_err()); // wrong separator
    assert!(Visibility::from_str("Public").is_err()); // PascalCase rejected
}
