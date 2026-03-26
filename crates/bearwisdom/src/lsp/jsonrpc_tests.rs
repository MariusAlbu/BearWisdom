use super::*;
use serde_json::json;

#[test]
fn encode_decode_request_roundtrip() {
    let req = Request::new(
        RequestId::Number(1),
        "textDocument/definition",
        Some(json!({ "textDocument": { "uri": "file:///foo.rs" }, "position": { "line": 0, "character": 0 } })),
    );
    let encoded = encode_message(&req);

    // Verify framing header is present
    let header_end = encoded
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("should have \\r\\n\\r\\n separator");
    let header = std::str::from_utf8(&encoded[..header_end]).unwrap();
    assert!(
        header.starts_with("Content-Length: "),
        "header: {header}"
    );

    let body = &encoded[header_end + 4..];
    let msg = decode_message(body).unwrap();
    assert!(matches!(msg, Message::Request(_)));
}

#[test]
fn encode_decode_notification_roundtrip() {
    let notif = Notification::new(
        "initialized",
        Some(json!({})),
    );
    let encoded = encode_message(&notif);
    let sep = encoded
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap();
    let body = &encoded[sep + 4..];
    let msg = decode_message(body).unwrap();
    assert!(matches!(msg, Message::Notification(_)));
}

#[test]
fn content_length_matches_body() {
    let req = Request::new(RequestId::Number(42), "initialize", None);
    let encoded = encode_message(&req);
    let sep = encoded
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap();
    let header = std::str::from_utf8(&encoded[..sep]).unwrap();
    let claimed_len: usize = header
        .trim_start_matches("Content-Length: ")
        .parse()
        .unwrap();
    let actual_body = &encoded[sep + 4..];
    assert_eq!(claimed_len, actual_body.len());
}

#[test]
fn decode_response_with_result() {
    let json = br#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#;
    let msg = decode_message(json).unwrap();
    assert!(matches!(msg, Message::Response(_)));
}

#[test]
fn decode_response_with_error() {
    let json = br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}"#;
    let msg = decode_message(json).unwrap();
    match msg {
        Message::Response(r) => {
            assert!(r.error.is_some());
            assert!(r.into_result().is_err());
        }
        _ => panic!("expected Response"),
    }
}

#[test]
fn decode_notification_no_id() {
    let json = br#"{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{}}"#;
    let msg = decode_message(json).unwrap();
    match msg {
        Message::Notification(n) => {
            assert_eq!(n.method, "textDocument/publishDiagnostics");
        }
        _ => panic!("expected Notification"),
    }
}

#[test]
fn decode_request_with_method_and_id() {
    let json = br#"{"jsonrpc":"2.0","id":7,"method":"workspace/configuration","params":{}}"#;
    let msg = decode_message(json).unwrap();
    assert!(matches!(msg, Message::Request(_)));
}
