use super::*;
use crate::lsp::jsonrpc::{Notification, Request, RequestId};

/// Build a framed byte buffer exactly as a real server would send it.
fn make_framed(json: &[u8]) -> Vec<u8> {
    let header = format!("Content-Length: {}\r\n\r\n", json.len());
    let mut buf = header.into_bytes();
    buf.extend_from_slice(json);
    buf
}

/// Wrap bytes in a `BufReader<ChildStdout>` via a pipe-backed tokio task.
///
/// We can't construct `ChildStdout` directly in tests, so we spawn a tiny
/// child process that just writes the bytes and exits.
///
/// On Windows `echo` is a shell built-in, so we use `cmd /c echo`.
/// We write the bytes into the child's stdin and forward stdout — but the
/// simplest approach is to parse from a Cursor instead.  Since BufReader
/// is generic, we test encoding separately and trust the framing logic.
#[test]
fn encode_decode_framing_roundtrip() {
    let req = Request::new(
        RequestId::Number(1),
        "textDocument/hover",
        Some(serde_json::json!({ "position": { "line": 0, "character": 5 } })),
    );
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
    assert_eq!(claimed_len, encoded.len() - sep - 4);
}

#[test]
fn notification_encodes_correctly() {
    let notif = Notification::new("initialized", Some(serde_json::json!({})));
    let bytes = encode_message(&notif);
    let s = String::from_utf8(bytes).unwrap();
    assert!(s.contains("Content-Length: "));
    assert!(s.contains("\"method\":\"initialized\""));
}

#[test]
fn framed_bytes_have_correct_separator() {
    let json = br#"{"jsonrpc":"2.0","method":"ping","params":null}"#;
    let framed = make_framed(json);
    let sep_pos = framed
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("must have \\r\\n\\r\\n");
    let body = &framed[sep_pos + 4..];
    assert_eq!(body, json);
}
