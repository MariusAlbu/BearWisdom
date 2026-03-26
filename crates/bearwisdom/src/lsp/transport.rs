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
#[path = "transport_tests.rs"]
mod tests;
