//! Debug Adapter Protocol (DAP) client over stdio.
//!
//! Phase 1 scope: the wire codec (Content-Length framed JSON) and the typed
//! request/response/event messages. The threaded client that drives a real
//! adapter process is layered on top of this codec.
//!
//! Wire format (one message): `Content-Length: <n>\r\n\r\n<json>` where `<n>` is
//! the byte length of the UTF-8 JSON body. Adapters may send other headers; we
//! only require `Content-Length`.

use std::io::BufRead;

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A decoded DAP message. DAP tags every message with a `type` of `request`,
/// `response`, or `event`; we keep the raw body so callers can pull
/// type-specific fields (`command`, `event`, `body`, `success`, `request_seq`).
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Request {
        seq: i64,
        command: String,
        arguments: Value,
    },
    Response {
        seq: i64,
        request_seq: i64,
        command: String,
        success: bool,
        body: Value,
    },
    Event {
        seq: i64,
        event: String,
        body: Value,
    },
}

/// Encode a single DAP message body (already-serialized JSON `Value`) into a
/// Content-Length framed byte buffer ready to write to the adapter's stdin.
pub fn encode(body: &Value) -> Vec<u8> {
    let json = serde_json::to_vec(body).expect("Value serializes");
    let mut out = format!("Content-Length: {}\r\n\r\n", json.len()).into_bytes();
    out.extend_from_slice(&json);
    out
}

/// Build a `request` message body with the given monotonically-increasing `seq`.
pub fn request(seq: i64, command: &str, arguments: Value) -> Value {
    serde_json::json!({
        "seq": seq,
        "type": "request",
        "command": command,
        "arguments": arguments,
    })
}

/// Read one Content-Length framed message from `r`. Returns `Ok(None)` on a
/// clean EOF before any header bytes (adapter closed its stdout).
pub fn read_message<R: BufRead>(r: &mut R) -> Result<Option<Message>> {
    // Parse headers until the blank line.
    let mut content_length: Option<usize> = None;
    let mut saw_any_header = false;
    loop {
        let mut line = String::new();
        let n = r.read_line(&mut line)?;
        if n == 0 {
            // EOF. Clean only if it happened before any header.
            if !saw_any_header {
                return Ok(None);
            }
            bail!("unexpected EOF in DAP header");
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // end of headers
        }
        saw_any_header = true;
        if let Some(v) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(
                v.trim()
                    .parse::<usize>()
                    .map_err(|_| anyhow!("invalid Content-Length: {v:?}"))?,
            );
        }
        // Other headers (e.g. Content-Type) are ignored.
    }
    let len = content_length.ok_or_else(|| anyhow!("missing Content-Length header"))?;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    let v: Value = serde_json::from_slice(&buf)?;
    parse_message(v)
}

/// Classify a parsed JSON value by its `type` field into a [`Message`].
fn parse_message(v: Value) -> Result<Option<Message>> {
    let seq = v.get("seq").and_then(Value::as_i64).unwrap_or(0);
    match v.get("type").and_then(Value::as_str) {
        Some("request") => Ok(Some(Message::Request {
            seq,
            command: str_field(&v, "command")?,
            arguments: v.get("arguments").cloned().unwrap_or(Value::Null),
        })),
        Some("response") => Ok(Some(Message::Response {
            seq,
            request_seq: v.get("request_seq").and_then(Value::as_i64).unwrap_or(0),
            command: str_field(&v, "command")?,
            success: v.get("success").and_then(Value::as_bool).unwrap_or(false),
            body: v.get("body").cloned().unwrap_or(Value::Null),
        })),
        Some("event") => Ok(Some(Message::Event {
            seq,
            event: str_field(&v, "event")?,
            body: v.get("body").cloned().unwrap_or(Value::Null),
        })),
        other => bail!("unknown DAP message type: {other:?}"),
    }
}

fn str_field(v: &Value, key: &str) -> Result<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("DAP message missing string field {key:?}"))
}

/// One call-stack frame, as surfaced to the UI and persisted in a comment
/// snapshot. Derived from a DAP `stackTrace` frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub name: String,
    /// Source file path (as reported by the adapter), if any.
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: u32,
    /// DAP frame id, used to request this frame's scopes/variables. Runtime
    /// only — not persisted in comment snapshots.
    #[serde(default, skip)]
    pub id: i64,
    /// Locals for this frame, populated lazily after a stop.
    #[serde(default)]
    pub locals: Vec<VarRow>,
}

/// One variable row (name = value), as surfaced to the UI and persisted in a
/// comment snapshot. `variables_reference` > 0 means the value is structured and
/// can be expanded with a further DAP `variables` request (not persisted).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VarRow {
    pub name: String,
    pub value: String,
    #[serde(default, rename = "type")]
    pub ty: Option<String>,
    /// DAP `variablesReference`: > 0 means the value is structured (e.g. a
    /// String/Vec/struct) and can be expanded with a further `variables`
    /// request. Runtime only — not persisted.
    #[serde(default, skip)]
    pub var_ref: i64,
    /// Memory address of the value, when the adapter reports one (heap-allocated
    /// values like String/Vec expose this). Runtime only — not persisted.
    #[serde(default, skip)]
    pub memory_ref: Option<String>,
    /// Whether this (structured) value is currently expanded in the UI.
    #[serde(default)]
    pub expanded: bool,
    /// Child variables, fetched lazily when expanded (or eagerly before a
    /// snapshot). Persisted so an attached snapshot keeps the resolved tree.
    #[serde(default)]
    pub children: Vec<VarRow>,
}

/// A snapshot of debugger state captured at a stopped point, attachable to a
/// review comment so a reviewer can leave a note *with* the runtime state that
/// prompted it. Stored on `Comment` as an optional, backward-compatible field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DebugSnapshot {
    /// Which session it came from (e.g. "worktree", "commit a1b2c3").
    pub session_label: String,
    /// File + line where execution was stopped.
    pub stopped_file: String,
    pub stopped_line: u32,
    /// Call stack at capture time (innermost first).
    #[serde(default)]
    pub stack: Vec<Frame>,
    /// Locals of the stopped frame at capture time.
    #[serde(default)]
    pub locals: Vec<VarRow>,
    /// Unix epoch seconds when captured.
    #[serde(default)]
    pub captured: i64,
}

/// A `stopped` event payload (the fields we care about). Emitted when the
/// debuggee halts at a breakpoint/step; `thread_id` is needed for follow-up
/// `stackTrace`/`scopes` requests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoppedBody {
    pub reason: String,
    #[serde(default, rename = "threadId")]
    pub thread_id: Option<i64>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Parse the body of a `stopped` event.
pub fn parse_stopped(body: &Value) -> Result<StoppedBody> {
    Ok(serde_json::from_value(body.clone())?)
}

// ─── Threaded client ─────────────────────────────────────────────────────────

use std::io::{BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// Stable id for one debug session, used to tag events from multiple adapters
/// multiplexed onto a single channel.
pub type SessionId = u64;

/// A message from a debug session's reader thread to the UI event loop.
#[derive(Clone, Debug)]
pub struct SessionMsg {
    pub session: SessionId,
    pub kind: SessionKind,
}

/// What happened on a session's adapter connection.
#[derive(Clone, Debug)]
pub enum SessionKind {
    /// A decoded DAP message (response or event).
    Message(Message),
    /// The adapter stdout closed (clean EOF) — session ended.
    Closed,
    /// The reader hit an error (e.g. malformed frame).
    Error(String),
}

/// Read DAP messages from `r` until EOF/error, forwarding each as a
/// [`SessionMsg`] tagged with `session`. Generic over the reader so it can be
/// unit-tested with an in-memory buffer (no real adapter process needed).
pub fn run_reader<R: BufRead>(session: SessionId, mut r: R, tx: Sender<SessionMsg>) {
    loop {
        match read_message(&mut r) {
            Ok(Some(msg)) => {
                if tx
                    .send(SessionMsg {
                        session,
                        kind: SessionKind::Message(msg),
                    })
                    .is_err()
                {
                    break; // receiver dropped — stop reading
                }
            }
            Ok(None) => {
                let _ = tx.send(SessionMsg {
                    session,
                    kind: SessionKind::Closed,
                });
                break;
            }
            Err(e) => {
                let _ = tx.send(SessionMsg {
                    session,
                    kind: SessionKind::Error(e.to_string()),
                });
                break;
            }
        }
    }
}

/// A live debug-adapter connection: the spawned process, its piped stdin (for
/// requests), and a monotonic request sequence. The reader thread is spawned at
/// construction and forwards messages on the shared channel.
pub struct DapClient {
    session: SessionId,
    child: Child,
    stdin: std::process::ChildStdin,
    seq: Arc<AtomicI64>,
}

impl DapClient {
    /// Spawn the adapter `command args...`, start its reader thread (forwarding
    /// to `tx` tagged with `session`), and return the client handle.
    pub fn spawn(
        session: SessionId,
        command: &str,
        args: &[String],
        tx: Sender<SessionMsg>,
    ) -> Result<DapClient> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow!("failed to spawn debug adapter {command:?}: {e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no adapter stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("no adapter stdout"))?;
        std::thread::Builder::new()
            .name(format!("dap-reader-{session}"))
            .spawn(move || run_reader(session, BufReader::new(stdout), tx))
            .map_err(|e| anyhow!("failed to spawn reader thread: {e}"))?;
        Ok(DapClient {
            session,
            child,
            stdin,
            seq: Arc::new(AtomicI64::new(0)),
        })
    }

    pub fn session(&self) -> SessionId {
        self.session
    }

    /// Send a DAP request; returns the assigned `seq` so the caller can match the
    /// response (`request_seq`) when it arrives on the channel.
    pub fn send_request(&mut self, command: &str, arguments: Value) -> Result<i64> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let body = request(seq, command, arguments);
        self.stdin.write_all(&encode(&body))?;
        self.stdin.flush()?;
        Ok(seq)
    }

    /// Politely ask the adapter to disconnect, then kill + reap the process.
    pub fn shutdown(&mut self) {
        let _ = self.send_request("disconnect", serde_json::json!({}));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for DapClient {
    fn drop(&mut self) {
        // Best-effort cleanup so a dropped session never leaks an adapter process.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn encode_frames_with_content_length() {
        let body = request(1, "initialize", serde_json::json!({"adapterID": "lldb"}));
        let bytes = encode(&body);
        let text = String::from_utf8(bytes).unwrap();
        // Header then blank line then JSON; length matches the body byte count.
        let (header, json) = text.split_once("\r\n\r\n").unwrap();
        assert_eq!(header, format!("Content-Length: {}", json.len()));
        assert!(json.contains("\"command\":\"initialize\""));
    }

    #[test]
    fn round_trip_request_through_reader() {
        let body = request(7, "next", serde_json::json!({"threadId": 1}));
        let bytes = encode(&body);
        let mut r = BufReader::new(&bytes[..]);
        let msg = read_message(&mut r).unwrap().unwrap();
        assert_eq!(
            msg,
            Message::Request {
                seq: 7,
                command: "next".into(),
                arguments: serde_json::json!({"threadId": 1}),
            }
        );
    }

    #[test]
    fn parses_response_message() {
        let raw = serde_json::json!({
            "seq": 10, "type": "response", "request_seq": 7,
            "command": "stackTrace", "success": true,
            "body": {"stackFrames": []}
        });
        let bytes = encode(&raw);
        let mut r = BufReader::new(&bytes[..]);
        match read_message(&mut r).unwrap().unwrap() {
            Message::Response {
                request_seq,
                command,
                success,
                ..
            } => {
                assert_eq!(request_seq, 7);
                assert_eq!(command, "stackTrace");
                assert!(success);
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn parses_stopped_event() {
        let raw = serde_json::json!({
            "seq": 3, "type": "event", "event": "stopped",
            "body": {"reason": "breakpoint", "threadId": 1}
        });
        let bytes = encode(&raw);
        let mut r = BufReader::new(&bytes[..]);
        let msg = read_message(&mut r).unwrap().unwrap();
        let Message::Event { event, body, .. } = msg else {
            panic!("expected event");
        };
        assert_eq!(event, "stopped");
        let stopped = parse_stopped(&body).unwrap();
        assert_eq!(stopped.reason, "breakpoint");
        assert_eq!(stopped.thread_id, Some(1));
    }

    #[test]
    fn clean_eof_returns_none() {
        let empty: &[u8] = b"";
        let mut r = BufReader::new(empty);
        assert_eq!(read_message(&mut r).unwrap(), None);
    }

    #[test]
    fn ignores_extra_headers() {
        let body = request(1, "threads", Value::Null);
        let json = serde_json::to_vec(&body).unwrap();
        // Inject an extra Content-Type header before Content-Length.
        let mut framed =
            format!("Content-Type: application/vscode-jsonrpc\r\nContent-Length: {}\r\n\r\n", json.len())
                .into_bytes();
        framed.extend_from_slice(&json);
        let mut r = BufReader::new(&framed[..]);
        let msg = read_message(&mut r).unwrap().unwrap();
        assert!(matches!(msg, Message::Request { command, .. } if command == "threads"));
    }

    #[test]
    fn run_reader_forwards_messages_then_closed() {
        use std::sync::mpsc::channel;
        // Two framed events back-to-back, then EOF.
        let mut buf = encode(&serde_json::json!({
            "seq":1,"type":"event","event":"initialized","body":{}
        }));
        buf.extend(encode(&serde_json::json!({
            "seq":2,"type":"event","event":"stopped","body":{"reason":"breakpoint","threadId":1}
        })));
        let (tx, rx) = channel();
        run_reader(42, BufReader::new(&buf[..]), tx);

        let m1 = rx.recv().unwrap();
        assert_eq!(m1.session, 42);
        assert!(matches!(
            m1.kind,
            SessionKind::Message(Message::Event { ref event, .. }) if event == "initialized"
        ));
        let m2 = rx.recv().unwrap();
        assert!(matches!(
            m2.kind,
            SessionKind::Message(Message::Event { ref event, .. }) if event == "stopped"
        ));
        let m3 = rx.recv().unwrap();
        assert!(matches!(m3.kind, SessionKind::Closed));
    }

    #[test]
    fn run_reader_reports_error_on_garbage() {
        use std::sync::mpsc::channel;
        let buf = b"Content-Length: 5\r\n\r\n{bad}".to_vec();
        let (tx, rx) = channel();
        run_reader(1, BufReader::new(&buf[..]), tx);
        let m = rx.recv().unwrap();
        assert!(matches!(m.kind, SessionKind::Error(_)));
    }

    // Integration: spawn a real process (`cat`) as a stand-in adapter. It echoes
    // framed stdin to stdout, so a request we send comes back through the reader.
    #[cfg(unix)]
    #[test]
    fn dap_client_spawn_writes_and_reads_back() {
        use std::sync::mpsc::channel;
        let (tx, rx) = channel();
        let mut client = DapClient::spawn(7, "cat", &[], tx).unwrap();
        let seq = client.send_request("initialize", serde_json::json!({"adapterID":"x"})).unwrap();
        assert_eq!(seq, 1);
        // `cat` echoes our framed request straight back; the reader decodes it.
        let msg = rx.recv().unwrap();
        match msg.kind {
            SessionKind::Message(Message::Request { command, seq: s, .. }) => {
                assert_eq!(command, "initialize");
                assert_eq!(s, 1);
            }
            other => panic!("expected echoed request, got {other:?}"),
        }
        client.shutdown();
    }
}
