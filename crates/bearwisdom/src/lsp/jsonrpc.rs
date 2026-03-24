// =============================================================================
// lsp/jsonrpc.rs  —  JSON-RPC 2.0 message types + framing
//
// Wire format:  Content-Length: <n>\r\n\r\n<json bytes>
// =============================================================================

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Primitive wire types
// ---------------------------------------------------------------------------

/// A JSON-RPC 2.0 request message (client → server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A JSON-RPC 2.0 response message (server → client).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

/// A JSON-RPC 2.0 notification message (either direction, no id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// The id field of a JSON-RPC request or response.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

/// The error object in a JSON-RPC error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Request {
    pub fn new(id: RequestId, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params,
        }
    }
}

impl Notification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

// ---------------------------------------------------------------------------
// Message — unified enum for dispatch
//
// Disambiguation rules (per JSON-RPC 2.0 + LSP spec):
//   - Has "id" field  + has "method"             → Request
//   - Has "id" field  + has "result" or "error"  → Response
//   - No  "id" field  + has "method"             → Notification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = Value::deserialize(deserializer)?;
        let obj = raw
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("JSON-RPC message must be an object"))?;

        let has_id = obj.contains_key("id");
        let has_method = obj.contains_key("method");
        let has_result = obj.contains_key("result");
        let has_error = obj.contains_key("error");

        if has_id && has_method {
            let req = Request::deserialize(raw).map_err(serde::de::Error::custom)?;
            Ok(Message::Request(req))
        } else if has_id && (has_result || has_error) {
            let resp = Response::deserialize(raw).map_err(serde::de::Error::custom)?;
            Ok(Message::Response(resp))
        } else if !has_id && has_method {
            let notif = Notification::deserialize(raw).map_err(serde::de::Error::custom)?;
            Ok(Message::Notification(notif))
        } else {
            Err(serde::de::Error::custom(
                "unrecognised JSON-RPC message shape",
            ))
        }
    }
}

impl Serialize for Message {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Message::Request(r) => r.serialize(serializer),
            Message::Response(r) => r.serialize(serializer),
            Message::Notification(n) => n.serialize(serializer),
        }
    }
}

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

/// Serializes `msg` to JSON and wraps it in an LSP `Content-Length` frame.
///
/// Output format:  `Content-Length: <n>\r\n\r\n<json bytes>`
pub fn encode_message(msg: &impl Serialize) -> Vec<u8> {
    let json = serde_json::to_vec(msg).expect("LSP message serialization must not fail");
    let header = format!("Content-Length: {}\r\n\r\n", json.len());
    let mut buf = Vec::with_capacity(header.len() + json.len());
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(&json);
    buf
}

/// Parses a raw JSON body (without headers) into a `Message`.
pub fn decode_message(json: &[u8]) -> Result<Message> {
    serde_json::from_slice(json).context("failed to decode JSON-RPC message")
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

impl Response {
    /// Extract the result value, returning an error if the response contained
    /// a JSON-RPC error object.
    pub fn into_result(self) -> Result<Value> {
        if let Some(err) = self.error {
            bail!("LSP error {}: {}", err.code, err.message);
        }
        Ok(self.result.unwrap_or(Value::Null))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
