//! Minimal Chrome DevTools Protocol client over a single browser WebSocket.
//!
//! CDP is an id-correlated JSON-RPC dialect, so this mirrors the MCP client in
//! `jcode-base` (`mcp/client.rs`): an atomic request id, a writer task draining
//! an mpsc channel, a reader task resolving a pending map, and a 1:1 request /
//! response correlation. The one addition CDP needs is event routing: messages
//! without an `id` belong to an attached target session and are forwarded to
//! that session's channel.

use anyhow::{Context, Result, anyhow, bail};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;
type Sessions = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Value>>>>;

/// A connected CDP browser endpoint.
pub(crate) struct CdpClient {
    request_id: AtomicU64,
    writer_tx: mpsc::Sender<String>,
    pending: Pending,
    sessions: Sessions,
}

impl CdpClient {
    /// Connect to a `ws://127.0.0.1:<port>/devtools/browser/<id>` endpoint.
    pub(crate) async fn connect(ws_url: &str) -> Result<Self> {
        let (stream, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .with_context(|| format!("failed to connect to CDP endpoint {ws_url}"))?;
        let (mut sink, mut source) = stream.split();

        let (writer_tx, mut writer_rx) = mpsc::channel::<String>(64);
        tokio::spawn(async move {
            while let Some(msg) = writer_rx.recv().await {
                if sink.send(Message::Text(msg)).await.is_err() {
                    break;
                }
            }
        });

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = Arc::clone(&pending);
        let reader_sessions = Arc::clone(&sessions);
        tokio::spawn(async move {
            while let Some(Ok(message)) = source.next().await {
                let text = match message {
                    Message::Text(text) => text,
                    Message::Close(_) => break,
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if let Some(id) = value.get("id").and_then(Value::as_u64) {
                    if let Some(tx) = reader_pending.lock().await.remove(&id) {
                        let _ = tx.send(value);
                    }
                } else if let Some(session) = value
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                {
                    let mut guard = reader_sessions.lock().await;
                    if let Some(tx) = guard.get(&session)
                        && tx.send(value).is_err()
                    {
                        // The receiver is gone (the page finished or its
                        // caller gave up); stop accumulating a channel that
                        // nothing will ever drain.
                        guard.remove(&session);
                    }
                }
            }
            // The socket is gone; wake every caller instead of letting them
            // block until their individual timeouts expire.
            reader_pending.lock().await.clear();
            reader_sessions.lock().await.clear();
        });

        Ok(Self {
            request_id: AtomicU64::new(0),
            writer_tx,
            pending,
            sessions,
        })
    }

    /// Issue a CDP command. `session` targets an attached page session; `None`
    /// addresses the browser itself.
    pub(crate) async fn send(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let mut request = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            request["sessionId"] = json!(session);
        }

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let send_result = self
            .writer_tx
            .send(serde_json::to_string(&request)?)
            .await
            .with_context(|| format!("cdp {method}: connection closed before send"));
        if send_result.is_err() {
            self.pending.lock().await.remove(&id);
            send_result?;
        }

        let response = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                bail!("cdp {method} failed: connection closed before a response arrived")
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                bail!("cdp {method} failed: no response within {timeout:?}");
            }
        };

        if let Some(error) = response.get("error") {
            bail!("cdp {method} failed: {error}");
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Start buffering events for an attached session.
    pub(crate) async fn register_session(&self, session: &str) -> mpsc::UnboundedReceiver<Value> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.sessions.lock().await.insert(session.to_string(), tx);
        rx
    }

    pub(crate) async fn unregister_session(&self, session: &str) {
        self.sessions.lock().await.remove(session);
    }

    /// Wait for one event on a registered session. `Ok(None)` means the wait
    /// elapsed, which several callers treat as a normal outcome rather than a
    /// failure.
    pub(crate) async fn wait_for_event(
        events: &mut mpsc::UnboundedReceiver<Value>,
        method: &str,
        timeout: Duration,
    ) -> Result<Option<Value>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            match tokio::time::timeout(remaining, events.recv()).await {
                Ok(Some(event)) => {
                    if event.get("method").and_then(Value::as_str) == Some(method) {
                        return Ok(Some(event));
                    }
                }
                Ok(None) => return Err(anyhow!("cdp session closed while awaiting {method}")),
                Err(_) => return Ok(None),
            }
        }
    }
}
