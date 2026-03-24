// =============================================================================
// lsp/server_handle.rs  —  per-server process handle
//
// LspServerHandle owns the Child process, the write half of its stdio,
// a pending-request map (id → oneshot), and a background reader task that
// routes incoming messages to the right waiters or the notification channel.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::timeout;

use crate::lsp::jsonrpc::{Message, Notification, Request, RequestId};
use crate::lsp::transport::{read_message, write_message};
use crate::lsp::types::{
    ClientCapabilities, InitializeParams, InitializeResult, ServerCapabilities, ServerState,
};

// ---------------------------------------------------------------------------
// LspServerHandle
// ---------------------------------------------------------------------------

pub struct LspServerHandle {
    /// The OS process.  We keep it alive so it isn't reaped.
    pub process: Child,

    /// Current lifecycle state.
    pub state: ServerState,

    /// Pending request map.  The reader task sends through these senders when
    /// a matching response arrives.
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>,

    /// Monotonically increasing request ID counter.
    next_id: Arc<AtomicI64>,

    /// Write half — wrapped in Arc<Mutex> so both the handle and the
    /// notification-sender can share it without lifetime issues.
    writer: Arc<Mutex<tokio::process::ChildStdin>>,

    /// Background task — lives as long as the handle.
    _reader_task: tokio::task::JoinHandle<()>,

    /// Channel for server-pushed notifications (diagnostics, etc.).
    pub notification_tx: mpsc::UnboundedSender<(String, Value)>,

    /// Negotiated capabilities from `initialize`.
    pub capabilities: Option<ServerCapabilities>,

    /// Total number of LSP requests sent since startup.
    pub request_count: u64,
}

impl LspServerHandle {
    // -----------------------------------------------------------------------
    // Spawn
    // -----------------------------------------------------------------------

    /// Launch the language-server binary, set up I/O tasks, and return a
    /// handle.  The server is in `Starting` state; call `initialize` next.
    pub async fn spawn(
        command: &str,
        args: &[String],
        workspace_root: &Path,
    ) -> Result<(Self, mpsc::UnboundedReceiver<(String, Value)>)> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .current_dir(workspace_root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        let mut child = cmd.spawn().with_context(|| {
            format!("failed to spawn LSP server '{command}'")
        })?;

        let stdin = child
            .stdin
            .take()
            .context("child stdin was not piped")?;
        let stdout = child
            .stdout
            .take()
            .context("child stdout was not piped")?;

        let writer = Arc::new(Mutex::new(stdin));
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let (notification_tx, notification_rx) = mpsc::unbounded_channel();

        // Clone handles for the reader task.
        let pending_for_task = Arc::clone(&pending);
        let notif_tx_for_task = notification_tx.clone();

        let reader_task = tokio::spawn(async move {
            use tokio::io::BufReader;
            let mut reader = BufReader::new(stdout);

            loop {
                match read_message(&mut reader).await {
                    Ok(msg) => match msg {
                        Message::Response(resp) => {
                            // Extract numeric id — string ids are unusual but handled.
                            let id = match &resp.id {
                                RequestId::Number(n) => *n,
                                RequestId::String(s) => {
                                    tracing::warn!(id = %s, "received response with string id");
                                    continue;
                                }
                            };

                            let result = if let Some(err) = resp.error {
                                Err(anyhow::anyhow!(
                                    "LSP error {}: {}",
                                    err.code,
                                    err.message
                                ))
                            } else {
                                Ok(resp.result.unwrap_or(Value::Null))
                            };

                            let sender = {
                                let mut map = pending_for_task.lock().await;
                                map.remove(&id)
                            };

                            if let Some(tx) = sender {
                                // Receiver may have been dropped (timeout) — ignore send errors.
                                let _ = tx.send(result);
                            } else {
                                tracing::debug!(id, "received response for unknown request id");
                            }
                        }
                        Message::Notification(notif) => {
                            let params = notif.params.unwrap_or(Value::Null);
                            let _ = notif_tx_for_task.send((notif.method, params));
                        }
                        Message::Request(req) => {
                            // Server-to-client requests (e.g. workspace/configuration).
                            // We don't handle them — just log.
                            tracing::debug!(method = %req.method, "ignoring server-to-client request");
                        }
                    },
                    Err(e) => {
                        tracing::debug!(error = %e, "LSP reader task exiting");
                        break;
                    }
                }
            }

            // Drain pending requests with errors on EOF.
            let mut map = pending_for_task.lock().await;
            for (_, tx) in map.drain() {
                let _ = tx.send(Err(anyhow::anyhow!("LSP server closed connection")));
            }
        });

        let handle = Self {
            process: child,
            state: ServerState::Starting,
            pending,
            next_id: Arc::new(AtomicI64::new(1)),
            writer,
            _reader_task: reader_task,
            notification_tx,
            capabilities: None,
            request_count: 0,
        };

        Ok((handle, notification_rx))
    }

    // -----------------------------------------------------------------------
    // Initialize handshake
    // -----------------------------------------------------------------------

    /// Perform the LSP `initialize` / `initialized` handshake.
    ///
    /// Sets `state` to `Ready` and stores the negotiated `ServerCapabilities`.
    pub async fn initialize(&mut self, root_uri: &str) -> Result<ServerCapabilities> {
        self.state = ServerState::Initializing;

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri.to_string()),
            capabilities: ClientCapabilities::default(),
        };

        let result = self
            .send_request("initialize", serde_json::to_value(params)?)
            .await?;

        let init: InitializeResult =
            serde_json::from_value(result).context("parsing InitializeResult")?;

        // Send `initialized` notification (no params required).
        self.send_notification("initialized", serde_json::json!({}))
            .await?;

        self.capabilities = Some(init.capabilities.clone());
        self.state = ServerState::Ready;

        Ok(init.capabilities)
    }

    // -----------------------------------------------------------------------
    // Request / notification helpers
    // -----------------------------------------------------------------------

    /// Assign an ID, send the request, and wait up to 30 s for the response.
    pub async fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.request_count += 1;

        let (tx, rx) = oneshot::channel::<Result<Value>>();
        {
            let mut map = self.pending.lock().await;
            map.insert(id, tx);
        }

        let req = Request::new(RequestId::Number(id), method, Some(params));
        {
            let mut w = self.writer.lock().await;
            write_message(&mut w, &req)
                .await
                .with_context(|| format!("sending LSP request '{method}'"))?;
        }

        match timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                // Sender was dropped — server died.
                bail!("LSP server closed before responding to '{method}'");
            }
            Err(_elapsed) => {
                // Timed out — remove the pending entry to avoid a leak.
                let mut map = self.pending.lock().await;
                map.remove(&id);
                bail!("LSP request '{method}' timed out after 30 s");
            }
        }
    }

    /// Send a one-way notification (no response expected).
    pub async fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let notif = Notification::new(method, Some(params));
        let mut w = self.writer.lock().await;
        write_message(&mut w, &notif)
            .await
            .with_context(|| format!("sending LSP notification '{method}'"))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Shutdown
    // -----------------------------------------------------------------------

    /// Send `shutdown`, wait for ack, send `exit`, then kill after 5 s if
    /// the process hasn't exited on its own.
    pub async fn shutdown(&mut self) -> Result<()> {
        self.state = ServerState::ShuttingDown;

        // `shutdown` is a request — server must respond before we send `exit`.
        if let Err(e) = self
            .send_request("shutdown", Value::Null)
            .await
        {
            tracing::warn!(error = %e, "LSP shutdown request failed; proceeding to exit");
        }

        // `exit` is a notification.
        let _ = self
            .send_notification("exit", Value::Null)
            .await;

        // Give the process 5 s to exit cleanly.
        match timeout(Duration::from_secs(5), self.process.wait()).await {
            Ok(Ok(status)) => {
                tracing::debug!(status = ?status, "LSP server exited cleanly");
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "error waiting for LSP server exit");
            }
            Err(_) => {
                tracing::warn!("LSP server did not exit within 5 s, killing");
                let _ = self.process.kill().await;
            }
        }

        self.state = ServerState::Stopped;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // State inspection
    // -----------------------------------------------------------------------

    pub fn is_ready(&self) -> bool {
        self.state == ServerState::Ready
    }
}
