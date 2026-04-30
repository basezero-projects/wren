/*!
 * MCP (Model Context Protocol) client.
 *
 * Wren acts as an MCP **client**: it spawns each configured server as a
 * child process, talks JSON-RPC 2.0 over stdio (newline-delimited JSON,
 * one message per line per the MCP spec), and exposes the server's tool
 * catalog to the local Ollama model alongside Wren's built-in tools.
 *
 * Why hand-rolled JSON-RPC and not the `rmcp` crate:
 * - The protocol surface we need is tiny: `initialize`,
 *   `notifications/initialized`, `tools/list`, `tools/call`. That fits in
 *   ~250 LOC of transport with no upstream-churn risk.
 * - Zero new heavy deps. `serde_json` and `tokio` (with `process` +
 *   `io-util`) are already pulled in for `commands.rs` and `voice.rs`.
 * - Mirrors the in-process precedent set by Phase 2's `fetch_url`
 *   (Mozilla Readability) — no daemon, no sidecar, no extra binary.
 *
 * Tool naming and approval gating (the Wren-specific bits):
 * - Every server tool is surfaced to Ollama as `mcp__<server>__<tool>`.
 *   The double-underscore separator avoids collision with built-in tool
 *   names like `read_file` / `fetch_url` and matches the convention
 *   Claude Code itself uses for MCP tools.
 * - Every `mcp__*` dispatch is treated as destructive: the chat loop
 *   surfaces an inline `ToolApprovalRequest` card before the call is
 *   sent. The model cannot reach into a user's secret manager, file
 *   server, or finance database without an explicit click. This is the
 *   same pattern `fetch_url` adopted in Phase 2 — the visible card IS
 *   the user's acknowledgement of the action.
 *
 * The registry lives behind a process-wide `OnceLock<Arc<McpRegistry>>`
 * (initialized in `lib.rs`'s setup hook) so the existing
 * `tools::dispatch(name, args) -> String` signature does not need to be
 * threaded with Tauri state. Same trick `fetch_url` used for its lazy
 * `reqwest::Client`.
 */

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{oneshot, Mutex, RwLock};

use crate::config::defaults::{
    MCP_HANDSHAKE_TIMEOUT_SECS, MCP_TOOL_CALL_TIMEOUT_SECS, MCP_TOOL_NAME_MAX_LEN,
    MCP_TOOL_RESULT_MAX_BYTES,
};
use crate::config::schema::McpServerConfig;

/// JSON-RPC 2.0 protocol-version literal we negotiate as a client.
/// Servers that do not respond to `initialize` get this version back as
/// the requested protocolVersion; modern (2025-and-later) servers
/// echo their own. Wren accepts any version the server returns — we do
/// not depend on capability negotiation beyond `tools`.
const MCP_CLIENT_PROTOCOL_VERSION: &str = "2025-06-18";

/// Prefix used to flatten an MCP server's tool name into Wren's tool
/// catalog. The double underscore avoids collision with built-in single-
/// underscore names. Public so the tools module can route prefixed
/// names back to the registry without re-defining the convention.
pub const MCP_TOOL_NAME_PREFIX: &str = "mcp__";

/// Separator between server name and tool name inside the prefixed
/// form. Same character pair as the prefix; same justification.
pub const MCP_TOOL_NAME_SEPARATOR: &str = "__";

// ─── Tool catalog records ───────────────────────────────────────────────────

/// One MCP tool as the server exposed it via `tools/list`.
///
/// `prefixed_name` is what Ollama sees and what the model emits in a
/// tool_call. `server_name` and `tool_name` are the original components
/// — kept around so dispatch can route the call back to the right
/// client without re-parsing the prefix.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolEntry {
    pub server_name: String,
    pub tool_name: String,
    pub prefixed_name: String,
    pub description: String,
    /// JSON Schema object the model uses to format arguments. Stored
    /// verbatim from the server response so input validation stays the
    /// server's responsibility.
    pub input_schema: Value,
}

/// Status of one configured MCP server, surfaced to the Settings UI.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct McpServerStatus {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// True when there is a live child process and the registry has its
    /// tool list cached. False when the user has not connected, the
    /// child died, or the connect attempt failed.
    pub connected: bool,
    /// Number of tools the server advertised on its last `tools/list`
    /// response. 0 when not connected.
    pub tool_count: usize,
    /// Optional human-readable error from the last connect attempt.
    /// Surfaces in the Settings UI so the user can see WHY a server
    /// is offline (bad path, missing binary, refused handshake, etc.).
    pub last_error: Option<String>,
}

// ─── JSON-RPC framing helpers ───────────────────────────────────────────────

/// Builds a JSON-RPC 2.0 request envelope. Returns the JSON `Value` so
/// the caller can serialize once and reuse for both wire-write and the
/// pending-map registration.
pub fn build_request(id: i64, method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

/// Builds a JSON-RPC 2.0 notification envelope (no `id`, no response
/// expected). Used for the `notifications/initialized` ack after the
/// `initialize` round-trip completes.
pub fn build_notification(method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

/// Result of decoding one inbound stdout line. `Response` carries an
/// id-keyed result for a previously-sent request; `Notification` is a
/// server-initiated message we ignore (Wren is a passive client for
/// v0 — no resource subscriptions, no sampling). `Ignored` is for
/// blank lines and lines that fail to parse as JSON, which the MCP
/// stdio framing spec says to drop silently rather than abort on.
#[derive(Debug, PartialEq)]
pub enum InboundMessage {
    Response { id: i64, result: Value },
    ResponseError { id: i64, message: String },
    Notification,
    Ignored,
}

/// Decodes one stdout line. Public for direct unit testing — production
/// code calls it through the reader task in `spawn_reader_task`.
pub fn decode_inbound_line(line: &str) -> InboundMessage {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return InboundMessage::Ignored;
    }
    let parsed: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return InboundMessage::Ignored,
    };
    // Notifications have no `id`. Drop them; we do not subscribe to any.
    let id = match parsed.get("id").and_then(Value::as_i64) {
        Some(id) => id,
        None => return InboundMessage::Notification,
    };
    if let Some(err) = parsed.get("error") {
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown JSON-RPC error")
            .to_string();
        return InboundMessage::ResponseError { id, message };
    }
    let result = parsed.get("result").cloned().unwrap_or(Value::Null);
    InboundMessage::Response { id, result }
}

// ─── Client ─────────────────────────────────────────────────────────────────

/// One live connection to a running MCP server.
///
/// Owns the child process (or `None` for in-memory test transports),
/// a writer half (stdin), and a background reader task that demuxes
/// incoming responses by JSON-RPC `id` into oneshot channels. Calls
/// race the timeout from `defaults.rs` so a stuck server cannot wedge
/// the chat loop.
pub struct McpClient {
    name: String,
    next_id: AtomicI64,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<InboundMessage>>>>,
    writer: Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    /// Held to keep the child alive for the lifetime of the client.
    /// Killed explicitly by `disconnect`. None when constructed from
    /// in-memory streams in tests.
    child: Mutex<Option<tokio::process::Child>>,
    reader_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    tools: RwLock<Vec<McpToolEntry>>,
}

impl McpClient {
    /// Spawns a server defined by `cfg`, performs the MCP `initialize`
    /// handshake, and discovers the tool catalog via `tools/list`.
    /// Returns a connected client ready for `call_tool`.
    pub async fn connect(cfg: &McpServerConfig) -> Result<Arc<Self>, String> {
        let (child, stdin, stdout) = spawn_server_child(cfg)?;
        let reader: Box<dyn AsyncRead + Send + Unpin> = Box::new(stdout);
        let writer: Box<dyn AsyncWrite + Send + Unpin> = Box::new(stdin);
        Self::from_streams(cfg.name.clone(), reader, writer, Some(child)).await
    }

    /// Wires a client around arbitrary `AsyncRead` + `AsyncWrite`
    /// halves. Used by `connect` (with child stdio) and by tests (with
    /// `tokio::io::duplex` pipes). Performs the full handshake +
    /// discovery before returning so a half-broken server is reported
    /// as a connect failure rather than as a tool-call failure later.
    pub async fn from_streams(
        name: String,
        reader: Box<dyn AsyncRead + Send + Unpin>,
        writer: Box<dyn AsyncWrite + Send + Unpin>,
        child: Option<tokio::process::Child>,
    ) -> Result<Arc<Self>, String> {
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<InboundMessage>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_task = spawn_reader_task(reader, Arc::clone(&pending));

        let client = Arc::new(Self {
            name: name.clone(),
            next_id: AtomicI64::new(1),
            pending,
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            reader_task: Mutex::new(Some(reader_task)),
            tools: RwLock::new(Vec::new()),
        });

        // Handshake: `initialize` -> response, then send the
        // `notifications/initialized` ack the spec requires before any
        // further requests can be made.
        let init_params = json!({
            "protocolVersion": MCP_CLIENT_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "wren",
                "version": env!("CARGO_PKG_VERSION"),
            },
        });
        let _init_result = client
            .call(
                "initialize",
                init_params,
                Duration::from_secs(MCP_HANDSHAKE_TIMEOUT_SECS),
            )
            .await
            .map_err(|e| format!("initialize failed: {e}"))?;
        client
            .send_notification("notifications/initialized", json!({}))
            .await
            .map_err(|e| format!("notifications/initialized send failed: {e}"))?;

        // Discover tools. A server with no tools at all is legal but
        // useless to Wren; we still keep the connection so the user can
        // see it as connected (with 0 tools) in Settings.
        let list_result = client
            .call(
                "tools/list",
                json!({}),
                Duration::from_secs(MCP_HANDSHAKE_TIMEOUT_SECS),
            )
            .await
            .map_err(|e| format!("tools/list failed: {e}"))?;
        let entries = parse_tools_list_response(&name, &list_result);
        *client.tools.write().await = entries;

        Ok(client)
    }

    /// Number of tools this server exposed via `tools/list`. Cached;
    /// no re-query.
    pub async fn tool_count(&self) -> usize {
        self.tools.read().await.len()
    }

    /// Snapshot of the current tool catalog. Returned cloned so the
    /// caller can iterate without holding the lock.
    pub async fn tools_snapshot(&self) -> Vec<McpToolEntry> {
        self.tools.read().await.clone()
    }

    /// Server-assigned name from the user's config. Borrowed view —
    /// callers that need to outlive the client clone the string.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sends a JSON-RPC request and waits for the matching response,
    /// honouring `timeout`. Returns the inner `result` value on success
    /// or a typed error string on transport / protocol / timeout failure.
    pub async fn call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let envelope = build_request(id, method, params);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        if let Err(e) = self.write_envelope(&envelope).await {
            // Make sure we do not leave a zombie sender in the map —
            // future ids would never collide but the leak is real.
            self.pending.lock().await.remove(&id);
            return Err(format!("write failed: {e}"));
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(InboundMessage::Response { result, .. })) => Ok(result),
            Ok(Ok(InboundMessage::ResponseError { message, .. })) => Err(message),
            Ok(Ok(InboundMessage::Notification)) | Ok(Ok(InboundMessage::Ignored)) => {
                // Reader only delivers Response / ResponseError to a
                // pending sender; this branch exists for exhaustiveness.
                Err("unexpected non-response message routed to caller".to_string())
            }
            Ok(Err(_)) => {
                // Sender dropped → reader task died (server exited).
                Err("server connection closed before responding".to_string())
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!("call timed out after {}s", timeout.as_secs()))
            }
        }
    }

    /// Calls `tools/call` and returns the textual result (concatenated
    /// `text` parts of the MCP content array, truncated). Errors are
    /// returned as `Err(String)` so the caller can distinguish a
    /// successful "tool said no" (`isError: true` in the result) from a
    /// transport failure if it wants to.
    pub async fn call_tool(&self, tool_name: &str, args: Value) -> Result<String, String> {
        let result = self
            .call(
                "tools/call",
                json!({
                    "name": tool_name,
                    "arguments": args,
                }),
                Duration::from_secs(MCP_TOOL_CALL_TIMEOUT_SECS),
            )
            .await?;
        Ok(format_tool_call_result(&result))
    }

    /// Closes the connection. Drops every pending sender (callers in
    /// `call` see the channel close and surface "connection closed"),
    /// kills the child if any, and aborts the reader task.
    pub async fn disconnect(&self) {
        // Drop all pending senders first so any awaiting `call` futures
        // resolve immediately instead of waiting for their timeouts.
        self.pending.lock().await.clear();
        if let Some(handle) = self.reader_task.lock().await.take() {
            handle.abort();
        }
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.start_kill();
        }
    }

    async fn write_envelope(&self, envelope: &Value) -> std::io::Result<()> {
        // The MCP spec says messages are newline-delimited and MUST NOT
        // contain embedded newlines. `serde_json::to_string` produces
        // single-line JSON by default (no pretty printing), so a `\n`
        // append is sufficient framing.
        let mut line = serde_json::to_string(envelope).expect("envelope serializes");
        line.push('\n');
        let mut writer = self.writer.lock().await;
        writer.write_all(line.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }

    async fn send_notification(&self, method: &str, params: Value) -> std::io::Result<()> {
        let envelope = build_notification(method, params);
        self.write_envelope(&envelope).await
    }
}

/// Spawns the OS-level child for `cfg` and returns its piped
/// stdin/stdout. Errors are mapped to user-visible strings so the
/// Settings UI can display "could not start `<command>`: <reason>"
/// instead of a Rust Debug print.
#[cfg_attr(coverage_nightly, coverage(off))]
fn spawn_server_child(
    cfg: &McpServerConfig,
) -> Result<(tokio::process::Child, ChildStdin, ChildStdout), String> {
    let mut cmd = tokio::process::Command::new(&cfg.command);
    cmd.args(&cfg.args);
    for (k, v) in &cfg.env {
        cmd.env(k, v);
    }
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());
    cmd.kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start `{}`: {e}", cfg.command))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "child stdin pipe was not captured".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "child stdout pipe was not captured".to_string())?;
    Ok((child, stdin, stdout))
}

/// Spawns the background task that demuxes inbound stdout lines into
/// the `pending` map. Public so tests can audit task behaviour without
/// going through `from_streams`.
pub fn spawn_reader_task(
    reader: Box<dyn AsyncRead + Send + Unpin>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<InboundMessage>>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = String::new();
        let mut reader = BufReader::new(reader);
        loop {
            buf.clear();
            match reader.read_line(&mut buf).await {
                Ok(0) => return, // EOF — server exited.
                Ok(_) => {}
                Err(_) => return,
            }
            let decoded = decode_inbound_line(&buf);
            match decoded {
                InboundMessage::Response { id, .. } | InboundMessage::ResponseError { id, .. } => {
                    if let Some(tx) = pending.lock().await.remove(&id) {
                        // Receiver may have dropped (timeout); ignore.
                        let _ = tx.send(decoded);
                    }
                }
                InboundMessage::Notification | InboundMessage::Ignored => {}
            }
        }
    })
}

/// Pulls the tool catalog out of a `tools/list` response. Tolerates a
/// missing or non-array `tools` field (returns empty), and per-entry
/// missing-name / oversized-name (skips the entry with a warning so
/// one weird tool does not poison the rest of the catalog).
pub fn parse_tools_list_response(server_name: &str, result: &Value) -> Vec<McpToolEntry> {
    let tools_arr = match result.get("tools").and_then(Value::as_array) {
        Some(arr) => arr,
        None => return Vec::new(),
    };
    let mut out = Vec::with_capacity(tools_arr.len());
    for entry in tools_arr {
        let name = match entry.get("name").and_then(Value::as_str) {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        if name.len() > MCP_TOOL_NAME_MAX_LEN {
            eprintln!(
                "wren: [mcp] dropping {server_name}/{name}: name is {} bytes (max {MCP_TOOL_NAME_MAX_LEN})",
                name.len()
            );
            continue;
        }
        let description = entry
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let input_schema = entry
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        out.push(McpToolEntry {
            server_name: server_name.to_string(),
            tool_name: name.to_string(),
            prefixed_name: format!(
                "{MCP_TOOL_NAME_PREFIX}{server_name}{MCP_TOOL_NAME_SEPARATOR}{name}"
            ),
            description,
            input_schema,
        });
    }
    out
}

/// Flattens an MCP `tools/call` result into a single textual string for
/// the chat loop. Concatenates every `text` part of the `content` array
/// and truncates to `MCP_TOOL_RESULT_MAX_BYTES` so a runaway server
/// response cannot blow the model's context window. Surfaces the
/// `isError: true` flag with an `[error]` prefix so the model knows the
/// tool reported failure even though we returned `Ok`.
pub fn format_tool_call_result(result: &Value) -> String {
    let mut buf = String::new();
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if is_error {
        buf.push_str("[error] ");
    }
    if let Some(arr) = result.get("content").and_then(Value::as_array) {
        for part in arr {
            let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
            if kind == "text" {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    buf.push_str(text);
                    buf.push('\n');
                }
            }
        }
    }
    let trimmed = buf.trim().to_string();
    if trimmed.len() > MCP_TOOL_RESULT_MAX_BYTES {
        let take = MCP_TOOL_RESULT_MAX_BYTES;
        let truncated: String = trimmed.chars().take(take).collect();
        format!(
            "{truncated}\n\n[truncated: showing first {take} of {} chars]",
            trimmed.chars().count()
        )
    } else if trimmed.is_empty() {
        "(no content)".to_string()
    } else {
        trimmed
    }
}

// ─── Registry ───────────────────────────────────────────────────────────────

/// Process-wide registry of connected MCP clients. One instance lives
/// behind a `OnceLock<Arc<McpRegistry>>` initialized at startup.
pub struct McpRegistry {
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
    /// Per-server last-error, surfaced in `McpServerStatus`. Cleared on
    /// successful connect; populated on connect failure / disconnect-
    /// with-error. Survives the client being torn down.
    last_errors: Mutex<HashMap<String, String>>,
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            last_errors: Mutex::new(HashMap::new()),
        }
    }
}

impl McpRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a client (replacing any prior client of the same name,
    /// disconnecting the old one). Used by both `connect_server` and
    /// the in-memory test path.
    pub async fn insert_client(&self, client: Arc<McpClient>) {
        let name = client.name().to_string();
        let mut guard = self.clients.write().await;
        if let Some(prev) = guard.insert(name.clone(), client) {
            // Spawn the disconnect so the lock is not held across an
            // await on a process kill / channel cleanup that could in
            // principle take longer than the lock should be held.
            tokio::spawn(async move {
                prev.disconnect().await;
            });
        }
        self.last_errors.lock().await.remove(&name);
    }

    /// Removes a client by name. Disconnects it before returning so
    /// callers do not race a pending `tools/call`.
    pub async fn remove_client(&self, name: &str) {
        let removed = self.clients.write().await.remove(name);
        if let Some(client) = removed {
            client.disconnect().await;
        }
    }

    /// Records the last connect-time error for `name`. Used by the
    /// Settings UI to surface "why is this server offline".
    pub async fn record_error(&self, name: &str, message: String) {
        self.last_errors
            .lock()
            .await
            .insert(name.to_string(), message);
    }

    /// Returns the cached `last_error` for `name`, if any.
    pub async fn last_error(&self, name: &str) -> Option<String> {
        self.last_errors.lock().await.get(name).cloned()
    }

    /// Snapshot of every connected client's tool list, prefixed.
    /// Iterating the map under a read-lock is safe because connects
    /// rarely race calls in practice; for v0 the cost is negligible.
    pub async fn flat_tool_catalog(&self) -> Vec<McpToolEntry> {
        let mut out = Vec::new();
        let guard = self.clients.read().await;
        for client in guard.values() {
            out.extend(client.tools_snapshot().await);
        }
        out
    }

    /// Looks up a connected client by name. Cloned `Arc` so the caller
    /// can drop the read-lock before awaiting on the client.
    pub async fn get_client(&self, name: &str) -> Option<Arc<McpClient>> {
        self.clients.read().await.get(name).cloned()
    }

    /// True when `name` has a live client. Used by `is_destructive` and
    /// the Settings UI.
    pub async fn is_connected(&self, name: &str) -> bool {
        self.clients.read().await.contains_key(name)
    }

    /// Disconnects every client. Used on app shutdown so child
    /// processes do not outlive Wren.
    pub async fn disconnect_all(&self) {
        let names: Vec<String> = self.clients.read().await.keys().cloned().collect();
        for name in names {
            self.remove_client(&name).await;
        }
    }
}

/// Process-wide registry handle. Initialized once in `lib.rs::run`'s
/// setup hook; accessed from `tools::dispatch` (which has no Tauri
/// state) and from the Tauri commands at the bottom of this file
/// (which prefer the global to threading state through every call).
static REGISTRY: OnceLock<Arc<McpRegistry>> = OnceLock::new();

/// Initializes (or returns) the process-wide registry. Idempotent so
/// double-init in tests or a hot-reload scenario does not panic.
pub fn registry() -> Arc<McpRegistry> {
    REGISTRY
        .get_or_init(|| Arc::new(McpRegistry::new()))
        .clone()
}

// ─── Tool-catalog integration (called by tools.rs) ──────────────────────────

/// Returns true when `name` follows the `mcp__<server>__<tool>` shape.
/// Used by `tools::is_destructive` to gate every MCP call as
/// destructive (the approval card surfaces the server + tool + args)
/// and by `tools::dispatch` to route the call.
pub fn is_mcp_tool_name(name: &str) -> bool {
    name.starts_with(MCP_TOOL_NAME_PREFIX)
        && name[MCP_TOOL_NAME_PREFIX.len()..].contains(MCP_TOOL_NAME_SEPARATOR)
}

/// Splits a `mcp__<server>__<tool>` name into its components, or
/// `None` if the shape does not match. Used by `dispatch_mcp_tool`.
pub fn split_mcp_tool_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix(MCP_TOOL_NAME_PREFIX)?;
    let idx = rest.find(MCP_TOOL_NAME_SEPARATOR)?;
    let server = &rest[..idx];
    let tool = &rest[idx + MCP_TOOL_NAME_SEPARATOR.len()..];
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server, tool))
}

/// Builds the JSON-Schema tool definitions for every connected MCP
/// tool. Appended to `tools::tool_definitions()` so Ollama sees one
/// flat catalog of built-ins + MCP tools per request.
pub async fn extra_tool_definitions() -> Vec<Value> {
    let entries = registry().flat_tool_catalog().await;
    entries
        .into_iter()
        .map(|e| {
            json!({
                "type": "function",
                "function": {
                    "name": e.prefixed_name,
                    "description": e.description,
                    "parameters": e.input_schema,
                }
            })
        })
        .collect()
}

/// Routes a `mcp__*` tool call from `tools::dispatch` to the right
/// connected client. Returns the textual result on success or an
/// `Error: ...` string on every failure mode (unknown server, no
/// connection, transport error). The leading `Error:` keeps the
/// existing chat-loop's `ok = !result.starts_with("Error:")` heuristic
/// working without touching `commands.rs`.
pub async fn dispatch_mcp_tool(name: &str, args: &Value) -> String {
    let (server, tool) = match split_mcp_tool_name(name) {
        Some(parts) => parts,
        None => return format!("Error: malformed MCP tool name `{name}`"),
    };
    let client = match registry().get_client(server).await {
        Some(c) => c,
        None => return format!("Error: MCP server `{server}` is not connected"),
    };
    match client.call_tool(tool, args.clone()).await {
        Ok(text) => text,
        Err(e) => format!("Error: MCP `{server}` tool `{tool}` failed: {e}"),
    }
}

// ─── Tauri command surface ──────────────────────────────────────────────────

/// Returns the connection / tool-count status for every server defined
/// in `[mcp].servers`. Servers in the config but not connected appear
/// with `connected: false` and `tool_count: 0`. Used by the Settings
/// UI to render the per-server connect/disconnect controls.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn mcp_list_servers(
    config: tauri::State<'_, parking_lot::RwLock<crate::config::AppConfig>>,
) -> Result<Vec<McpServerStatus>, String> {
    let configured = config.read().mcp.servers.clone();
    let registry = registry();
    let mut out = Vec::with_capacity(configured.len());
    for cfg in configured {
        let last_error = registry.last_error(&cfg.name).await;
        let (connected, tool_count) = match registry.get_client(&cfg.name).await {
            Some(client) => (true, client.tool_count().await),
            None => (false, 0),
        };
        out.push(McpServerStatus {
            name: cfg.name,
            command: cfg.command,
            args: cfg.args,
            connected,
            tool_count,
            last_error,
        });
    }
    Ok(out)
}

/// Connects to a configured server by name, replacing any prior
/// connection. On failure the registry's `last_errors` map is updated
/// so a follow-up `mcp_list_servers` shows the reason.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn mcp_connect_server(
    name: String,
    config: tauri::State<'_, parking_lot::RwLock<crate::config::AppConfig>>,
) -> Result<McpServerStatus, String> {
    let cfg = config
        .read()
        .mcp
        .servers
        .iter()
        .find(|s| s.name == name)
        .cloned()
        .ok_or_else(|| format!("no MCP server named `{name}` in config"))?;

    let registry = registry();
    match McpClient::connect(&cfg).await {
        Ok(client) => {
            let tool_count = client.tool_count().await;
            registry.insert_client(client).await;
            Ok(McpServerStatus {
                name: cfg.name,
                command: cfg.command,
                args: cfg.args,
                connected: true,
                tool_count,
                last_error: None,
            })
        }
        Err(e) => {
            registry.record_error(&name, e.clone()).await;
            Err(e)
        }
    }
}

/// Disconnects a server by name. Idempotent — disconnecting a server
/// that was never connected is a no-op.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn mcp_disconnect_server(name: String) -> Result<(), String> {
    registry().remove_client(&name).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    // ── decode_inbound_line ───────────────────────────────────────────────

    #[test]
    fn decode_inbound_line_blank_is_ignored() {
        assert_eq!(decode_inbound_line(""), InboundMessage::Ignored);
        assert_eq!(decode_inbound_line("   \n"), InboundMessage::Ignored);
    }

    #[test]
    fn decode_inbound_line_invalid_json_is_ignored() {
        assert_eq!(
            decode_inbound_line("not even close to json"),
            InboundMessage::Ignored
        );
    }

    #[test]
    fn decode_inbound_line_response_returns_id_and_result() {
        let line = r#"{"jsonrpc":"2.0","id":42,"result":{"x":1}}"#;
        match decode_inbound_line(line) {
            InboundMessage::Response { id, result } => {
                assert_eq!(id, 42);
                assert_eq!(result["x"], 1);
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn decode_inbound_line_response_missing_result_yields_null() {
        let line = r#"{"jsonrpc":"2.0","id":7}"#;
        match decode_inbound_line(line) {
            InboundMessage::Response { id, result } => {
                assert_eq!(id, 7);
                assert!(result.is_null());
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn decode_inbound_line_error_returns_message() {
        let line = r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"no such method"}}"#;
        match decode_inbound_line(line) {
            InboundMessage::ResponseError { id, message } => {
                assert_eq!(id, 3);
                assert_eq!(message, "no such method");
            }
            other => panic!("expected ResponseError, got {other:?}"),
        }
    }

    #[test]
    fn decode_inbound_line_error_missing_message_uses_fallback() {
        let line = r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32601}}"#;
        match decode_inbound_line(line) {
            InboundMessage::ResponseError { id, message } => {
                assert_eq!(id, 3);
                assert_eq!(message, "unknown JSON-RPC error");
            }
            other => panic!("expected ResponseError, got {other:?}"),
        }
    }

    #[test]
    fn decode_inbound_line_no_id_is_notification() {
        let line = r#"{"jsonrpc":"2.0","method":"foo","params":{}}"#;
        assert_eq!(decode_inbound_line(line), InboundMessage::Notification);
    }

    // ── tool name helpers ─────────────────────────────────────────────────

    #[test]
    fn is_mcp_tool_name_recognises_prefixed_form() {
        assert!(is_mcp_tool_name("mcp__svr__tool"));
        assert!(is_mcp_tool_name("mcp__a__b__c")); // tool name may itself contain __
    }

    #[test]
    fn is_mcp_tool_name_rejects_built_ins() {
        assert!(!is_mcp_tool_name("read_file"));
        assert!(!is_mcp_tool_name("fetch_url"));
        // No separator after the prefix → not an MCP tool.
        assert!(!is_mcp_tool_name("mcp__nosep"));
        // Prefix-only is not an MCP tool either.
        assert!(!is_mcp_tool_name("mcp__"));
    }

    #[test]
    fn split_mcp_tool_name_extracts_components() {
        assert_eq!(split_mcp_tool_name("mcp__svr__tool"), Some(("svr", "tool")));
        // Tools with embedded __ keep everything after the first __.
        assert_eq!(
            split_mcp_tool_name("mcp__svr__sub__tool"),
            Some(("svr", "sub__tool"))
        );
    }

    #[test]
    fn split_mcp_tool_name_rejects_invalid() {
        assert!(split_mcp_tool_name("read_file").is_none());
        assert!(split_mcp_tool_name("mcp__nosep").is_none());
        assert!(split_mcp_tool_name("mcp____tool").is_none()); // empty server
        assert!(split_mcp_tool_name("mcp__svr__").is_none()); // empty tool
    }

    // ── parse_tools_list_response ─────────────────────────────────────────

    #[test]
    fn parse_tools_list_handles_empty_or_missing_array() {
        assert!(parse_tools_list_response("svr", &json!({})).is_empty());
        assert!(parse_tools_list_response("svr", &json!({"tools": null})).is_empty());
        assert!(parse_tools_list_response("svr", &json!({"tools": "not an array"})).is_empty());
        assert!(parse_tools_list_response("svr", &json!({"tools": []})).is_empty());
    }

    #[test]
    fn parse_tools_list_yields_prefixed_entries() {
        let result = json!({
            "tools": [
                {"name": "read", "description": "reads stuff", "inputSchema": {"type": "object"}},
                {"name": "write"},
            ]
        });
        let entries = parse_tools_list_response("svr", &result);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].prefixed_name, "mcp__svr__read");
        assert_eq!(entries[0].description, "reads stuff");
        assert_eq!(entries[0].input_schema["type"], "object");
        assert_eq!(entries[1].prefixed_name, "mcp__svr__write");
        // Missing description / inputSchema fall back to sensible defaults.
        assert_eq!(entries[1].description, "");
        assert_eq!(entries[1].input_schema["type"], "object");
    }

    #[test]
    fn parse_tools_list_drops_oversized_or_nameless_entries() {
        let oversized = "x".repeat(MCP_TOOL_NAME_MAX_LEN + 1);
        let result = json!({
            "tools": [
                {"name": ""},
                {"description": "no name"},
                {"name": oversized},
                {"name": "ok"},
            ]
        });
        let entries = parse_tools_list_response("svr", &result);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tool_name, "ok");
    }

    // ── format_tool_call_result ───────────────────────────────────────────

    #[test]
    fn format_tool_call_result_concats_text_parts() {
        let r = json!({
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "text", "text": "world"},
                {"type": "image", "data": "base64..."} // ignored
            ]
        });
        assert_eq!(format_tool_call_result(&r), "hello\nworld");
    }

    #[test]
    fn format_tool_call_result_empty_content_uses_placeholder() {
        assert_eq!(format_tool_call_result(&json!({})), "(no content)");
        assert_eq!(
            format_tool_call_result(&json!({"content": []})),
            "(no content)"
        );
    }

    #[test]
    fn format_tool_call_result_marks_is_error() {
        let r = json!({
            "isError": true,
            "content": [{"type": "text", "text": "boom"}]
        });
        assert_eq!(format_tool_call_result(&r), "[error] boom");
    }

    #[test]
    fn format_tool_call_result_truncates_oversized_text() {
        let long = "a".repeat(MCP_TOOL_RESULT_MAX_BYTES + 100);
        let r = json!({"content": [{"type": "text", "text": long}]});
        let out = format_tool_call_result(&r);
        assert!(out.contains("[truncated:"));
        assert!(out.starts_with("aaaa"));
    }

    // ── build_request / build_notification ────────────────────────────────

    #[test]
    fn build_request_carries_id_and_method() {
        let v = build_request(7, "foo", json!({"x": 1}));
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 7);
        assert_eq!(v["method"], "foo");
        assert_eq!(v["params"]["x"], 1);
    }

    #[test]
    fn build_notification_omits_id() {
        let v = build_notification("foo", json!({"y": 2}));
        assert_eq!(v["jsonrpc"], "2.0");
        assert!(v.get("id").is_none());
        assert_eq!(v["method"], "foo");
    }

    // ── McpClient end-to-end via in-memory duplex pipes ───────────────────
    //
    // These tests stand up a fake "server" task that reads requests from
    // one side of a tokio::io::duplex pipe and writes scripted responses
    // back. This exercises the real handshake, the reader task, the
    // pending-id demux, and the timeout path without needing a child
    // process — keeping the test deterministic across CI matrices.

    /// Spawns a fake MCP server task on the given pipe halves. It reads
    /// every newline-delimited request, looks up a scripted response in
    /// `responses` keyed by method, fills the response's `id` from the
    /// request, and writes it back. Methods not in the map are ignored
    /// (so notifications like `notifications/initialized` are dropped).
    fn spawn_fake_server<R, W>(
        mut reader: R,
        mut writer: W,
        responses: HashMap<String, Value>,
    ) -> tokio::task::JoinHandle<()>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        tokio::spawn(async move {
            let mut buf = String::new();
            let mut br = BufReader::new(&mut reader);
            loop {
                buf.clear();
                match br.read_line(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                let req: Value = match serde_json::from_str(buf.trim()) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let method = req
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let id = req.get("id").and_then(Value::as_i64);
                if let Some(template) = responses.get(&method) {
                    let mut resp = template.clone();
                    if let (Some(id), Some(obj)) = (id, resp.as_object_mut()) {
                        obj.insert("id".to_string(), json!(id));
                    }
                    let mut line = serde_json::to_string(&resp).unwrap();
                    line.push('\n');
                    let _ = writer.write_all(line.as_bytes()).await;
                    let _ = writer.flush().await;
                }
            }
        })
    }

    type BoxedReader = Box<dyn AsyncRead + Send + Unpin>;
    type BoxedWriter = Box<dyn AsyncWrite + Send + Unpin>;

    fn make_pipes() -> (BoxedReader, BoxedWriter, BoxedReader, BoxedWriter) {
        // client_writer  ──>  server_reader
        let (client_writer, server_reader) = duplex(8192);
        // server_writer  ──>  client_reader
        let (server_writer, client_reader) = duplex(8192);
        let (cr, _) = tokio::io::split(client_reader);
        let (_, cw) = tokio::io::split(client_writer);
        let (sr, _) = tokio::io::split(server_reader);
        let (_, sw) = tokio::io::split(server_writer);
        (Box::new(cr), Box::new(cw), Box::new(sr), Box::new(sw))
    }

    fn responses_with_two_tools() -> HashMap<String, Value> {
        let mut r = HashMap::new();
        r.insert(
            "initialize".to_string(),
            json!({"jsonrpc":"2.0","result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"fake","version":"0.0.1"}}}),
        );
        r.insert(
            "tools/list".to_string(),
            json!({"jsonrpc":"2.0","result":{"tools":[
                {"name":"echo","description":"echoes","inputSchema":{"type":"object","properties":{"msg":{"type":"string"}}}},
                {"name":"add","description":"adds","inputSchema":{"type":"object"}}
            ]}}),
        );
        r.insert(
            "tools/call".to_string(),
            json!({"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"called!"}]}}),
        );
        r
    }

    #[tokio::test]
    async fn mcp_client_handshake_and_tool_listing() {
        let (cr, cw, sr, sw) = make_pipes();
        let _server = spawn_fake_server(sr, sw, responses_with_two_tools());

        let client = McpClient::from_streams("fake".to_string(), cr, cw, None)
            .await
            .expect("connects");
        assert_eq!(client.tool_count().await, 2);
        let tools = client.tools_snapshot().await;
        assert_eq!(tools[0].prefixed_name, "mcp__fake__echo");
        assert_eq!(tools[1].prefixed_name, "mcp__fake__add");

        let result = client
            .call_tool("echo", json!({"msg": "hi"}))
            .await
            .unwrap();
        assert_eq!(result, "called!");

        client.disconnect().await;
    }

    #[tokio::test]
    async fn mcp_client_initialize_failure_is_surfaced() {
        let (cr, cw, sr, sw) = make_pipes();
        let mut responses = HashMap::new();
        responses.insert(
            "initialize".to_string(),
            json!({"jsonrpc":"2.0","error":{"code":-32601,"message":"server says no"}}),
        );
        let _server = spawn_fake_server(sr, sw, responses);

        // McpClient doesn't `derive(Debug)` (it holds non-Debug streams),
        // so `expect_err` is not available — match the Result by hand.
        let err = match McpClient::from_streams("fake".to_string(), cr, cw, None).await {
            Ok(_) => panic!("initialize should fail"),
            Err(e) => e,
        };
        assert!(err.contains("initialize failed"), "got: {err}");
        assert!(err.contains("server says no"), "got: {err}");
    }

    #[tokio::test]
    async fn mcp_client_call_times_out_when_server_silent() {
        let (cr, cw, sr, sw) = make_pipes();
        // Server replies to initialize + tools/list but never to a custom
        // method, so the next call times out.
        let _server = spawn_fake_server(sr, sw, responses_with_two_tools());

        let client = McpClient::from_streams("fake".to_string(), cr, cw, None)
            .await
            .expect("connects");
        let err = client
            .call("never/answers", json!({}), Duration::from_millis(50))
            .await
            .expect_err("times out");
        assert!(err.contains("timed out"), "got: {err}");
    }

    #[tokio::test]
    async fn mcp_client_call_after_server_drop_returns_closed() {
        let (cr, cw, sr, sw) = make_pipes();
        let server = spawn_fake_server(sr, sw, responses_with_two_tools());

        let client = McpClient::from_streams("fake".to_string(), cr, cw, None)
            .await
            .expect("connects");
        // Kill the fake server, then issue a call. The reader task hits
        // EOF and the pending sender drops; the call sees "closed".
        server.abort();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let err = client
            .call("anything", json!({}), Duration::from_secs(2))
            .await
            .expect_err("server is gone");
        // Three legitimate races on a sudden server drop: the reader hits
        // EOF first ("closed"), the write side fails first ("write failed:
        // broken pipe"), or the timeout fires before either signal reaches
        // the awaiting call. All three are correct shutdown paths.
        assert!(
            err.contains("closed") || err.contains("timed out") || err.contains("write failed"),
            "got: {err}"
        );
    }

    // ── Registry ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn registry_insert_replace_remove_and_flat_catalog() {
        let registry = McpRegistry::new();
        let (cr, cw, sr, sw) = make_pipes();
        let _server = spawn_fake_server(sr, sw, responses_with_two_tools());
        let client = McpClient::from_streams("fake".to_string(), cr, cw, None)
            .await
            .unwrap();

        registry.insert_client(client).await;
        assert!(registry.is_connected("fake").await);
        let catalog = registry.flat_tool_catalog().await;
        assert_eq!(catalog.len(), 2);

        // Replacing inserts a fresh client and disconnects the old one.
        let (cr, cw, sr, sw) = make_pipes();
        let _server2 = spawn_fake_server(sr, sw, responses_with_two_tools());
        let client2 = McpClient::from_streams("fake".to_string(), cr, cw, None)
            .await
            .unwrap();
        registry.insert_client(client2).await;
        assert_eq!(registry.flat_tool_catalog().await.len(), 2);

        registry.remove_client("fake").await;
        assert!(!registry.is_connected("fake").await);
        assert!(registry.flat_tool_catalog().await.is_empty());

        // Removing a server that was never connected is a no-op.
        registry.remove_client("never-existed").await;
    }

    #[tokio::test]
    async fn registry_record_and_recall_last_error() {
        let r = McpRegistry::new();
        assert!(r.last_error("svr").await.is_none());
        r.record_error("svr", "boom".to_string()).await;
        assert_eq!(r.last_error("svr").await.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn registry_disconnect_all_clears_clients() {
        let registry = McpRegistry::new();
        let (cr, cw, sr, sw) = make_pipes();
        let _server = spawn_fake_server(sr, sw, responses_with_two_tools());
        let client = McpClient::from_streams("fake".to_string(), cr, cw, None)
            .await
            .unwrap();
        registry.insert_client(client).await;
        assert!(registry.is_connected("fake").await);
        registry.disconnect_all().await;
        assert!(!registry.is_connected("fake").await);
    }

    // ── dispatch_mcp_tool routing ─────────────────────────────────────────
    //
    // dispatch_mcp_tool reads from the process-wide REGISTRY, so these
    // tests cover the routing + error branches without touching it.

    #[tokio::test]
    async fn dispatch_mcp_tool_rejects_malformed_name() {
        // A name without the separator is not a valid MCP tool; the
        // dispatcher reports it explicitly so a bug elsewhere surfaces
        // instead of silently dropping the call.
        let out = dispatch_mcp_tool("not_mcp_at_all", &json!({})).await;
        assert!(out.starts_with("Error:"));
        assert!(out.contains("malformed MCP tool name"));
    }

    #[tokio::test]
    async fn dispatch_mcp_tool_unknown_server_is_error() {
        // Use a server name that is extremely unlikely to be registered
        // even if other tests run in parallel against the global registry.
        let unique = format!("test-not-registered-{}", uuid::Uuid::new_v4());
        let tool_name = format!("mcp__{unique}__whatever");
        let out = dispatch_mcp_tool(&tool_name, &json!({})).await;
        assert!(out.starts_with("Error:"));
        assert!(out.contains("not connected"));
    }
}
