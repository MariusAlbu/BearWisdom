use super::*;

fn name_of(emission: &FlowEmission) -> &str {
    match emission {
        FlowEmission::NamedChannel { name, .. } => name.as_str(),
        _ => "",
    }
}

#[test]
fn emits_one_consumer_per_rpc() {
    let src = r#"
syntax = "proto3";

service UserService {
  rpc GetUser(GetUserRequest) returns (GetUserResponse);
  rpc ListUsers(ListUsersRequest) returns (ListUsersResponse);
}

service BillingService {
  rpc Charge(ChargeRequest) returns (ChargeResponse);
}
"#;
    let points = extract_proto_grpc_starts(src);
    assert_eq!(points.len(), 3);
    assert_eq!(name_of(&points[0].1), "UserService.GetUser");
    assert_eq!(name_of(&points[2].1), "BillingService.Charge");
    for (_, e) in &points {
        match e {
            FlowEmission::NamedChannel { kind, role, streaming, .. } => {
                assert_eq!(*kind, NamedChannelKind::RpcCall);
                assert_eq!(*role, ChannelRole::Consumer);
                assert!(streaming.is_none(), "unary expected, got {streaming:?}");
            }
            _ => panic!("expected NamedChannel"),
        }
    }
}

#[test]
fn detects_streaming_variants() {
    let src = r#"
service S {
  rpc Sub(Req) returns (stream Resp);
  rpc Upload(stream Req) returns (Resp);
  rpc Chat(stream Req) returns (stream Resp);
}
"#;
    let points = extract_proto_grpc_starts(src);
    assert_eq!(points.len(), 3);
    let streamings: Vec<_> = points
        .iter()
        .map(|(_, e)| match e {
            FlowEmission::NamedChannel { streaming, .. } => *streaming,
            _ => None,
        })
        .collect();
    assert_eq!(streamings[0], Some(StreamKind::ServerStreaming));
    assert_eq!(streamings[1], Some(StreamKind::ClientStreaming));
    assert_eq!(streamings[2], Some(StreamKind::BidiStreaming));
}

#[test]
fn empty_source_produces_no_points() {
    assert!(extract_proto_grpc_starts("").is_empty());
}
