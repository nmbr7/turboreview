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
}
