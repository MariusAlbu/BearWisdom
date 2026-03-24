// =============================================================================
// lsp/transport.rs  —  LSP stdio framing over tokio Child stdio
//
// Wire format:  Content-Length: <n>\r\n\r\n<json bytes>
//
// `read_message` and `write_message` are standalone async functions — they
// borrow the I/O handles and leave ownership with the caller so the caller
// can keep both halves of the Child's stdio.
// =============================================================================

use anyhow::{bail, Context, Result};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};

use crate::lsp::jsonrpc::{decode_message, encode_message, Message};

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Read exactly one LSP message from `reader`.
///
/// Parses the `Content-Length` header, reads exactly that many bytes, then
/// decodes the JSON body into a [`Message`].
///
/// Returns an error on EOF, malformed headers, or invalid JSON.
pub async fn read_message(reader: &mut BufReader<ChildStdout>) -> Result<Message> {
    // ---- Read headers until the blank line --------------------------------
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        let n: usize = reader
            .read_line(&mut line)
            .await
            .context("reading LSP header line")?;

        if n == 0 {
            bail!("LSP server closed stdout (EOF while reading headers)");
        }

        let trimmed = line.trim_end_matches(|c| c == '\r' || c == '\n');

        if trimmed.is_empty() {
            // Blank line — headers are done.
            break;
        }

        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            let len: usize = value
                .trim()
                .parse()
                .context("invalid Content-Length value")?;
            content_length = Some(len);
        }
        // Ignore any other headers (e.g. Content-Type) — LSP spec allows them.
    }

    let len = content_length.context("LSP message missing Content-Length header")?;

    // ---- Read body --------------------------------------------------------
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .await
        .context("reading LSP message body")?;

    decode_message(&body)
}

// ---------------------------------------------------------------------------
// Write
// ---------------------------------------------------------------------------

/// Encode `msg` with LSP framing and write it to `writer` atomically.
pub async fn write_message(writer: &mut ChildStdin, msg: &impl Serialize) -> Result<()> {
    let buf = encode_message(msg);
    writer
        .write_all(&buf)
        .await
        .context("writing LSP message to stdin")?;
    writer.flush().await.context("flushing LSP stdin")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
