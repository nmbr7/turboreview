//! Debug session orchestration: owns the live DAP adapter connections and the
//! shared event channel, drives the DAP handshake/step requests, and applies
//! incoming messages to the UI-facing [`DebugState`] on `App`.
//!
//! The live [`DapClient`]s and the `mpsc::Receiver` cannot live on `App` (which
//! is `Clone` and holds no OS handles), so they live here in a `DebugManager`
//! owned by the main event loop.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};

use anyhow::Result;
use serde_json::{json, Value};

use crate::app::{App, DebugSession, DebugState, SessionState};
use crate::dap::{DapClient, Message, SessionId, SessionKind, SessionMsg};
use crate::storage::DebugConfig;

/// What a pending request was for, so the matching response can be routed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Initialize,
    StackTrace,
    Scopes,
    Variables,
}

/// One live adapter connection plus its in-flight request bookkeeping.
struct Live {
    client: DapClient,
    /// seq → what the request was for.
    pending: HashMap<i64, Pending>,
    /// Source breakpoints already sent (so we only re-send on change).
    bp_sent: bool,
    /// Frame id of the top frame at the last stop (for scopes/variables).
    top_frame: Option<i64>,
}

/// Owns all live debug sessions and the shared event channel.
pub struct DebugManager {
    tx: Sender<SessionMsg>,
    rx: Receiver<SessionMsg>,
    live: HashMap<SessionId, Live>,
    /// Per-session launch config (cfg + source root), used by the `initialized`
    /// handler to send `launch`.
    launch_cfg: HashMap<SessionId, (DebugConfig, std::path::PathBuf)>,
    next_id: SessionId,
}

impl Default for DebugManager {
    fn default() -> Self {
        let (tx, rx) = channel();
        DebugManager {
            tx,
            rx,
            live: HashMap::new(),
            launch_cfg: HashMap::new(),
            next_id: 1,
        }
    }
}

impl DebugManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn any_sessions(&self) -> bool {
        !self.live.is_empty()
    }

    /// Run the configured build command (blocking). Returns Ok(()) on success or
    /// a descriptive error including stderr. Skipped when `cfg.build` is empty.
    pub fn run_build(cfg: &DebugConfig, cwd: &Path) -> Result<()> {
        if cfg.build.trim().is_empty() {
            return Ok(());
        }
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cfg.build)
            .current_dir(cwd)
            .output()?;
        if !out.status.success() {
            anyhow::bail!(
                "build failed ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    /// Launch a new session: build, spawn the adapter, register a `DebugSession`
    /// on `app`, and kick off the DAP handshake (`initialize`). Breakpoints and
    /// `launch` are sent once the adapter reports `initialized` (see [`drain`]).
    /// `source_root` is the directory whose build/program the session debugs.
    pub fn launch(
        &mut self,
        app: &mut App,
        cfg: &DebugConfig,
        source_root: &Path,
        label: String,
    ) -> Result<()> {
        Self::run_build(cfg, source_root)?;
        if cfg.adapter.command.trim().is_empty() {
            anyhow::bail!("no debug adapter configured (set debug.adapter.command)");
        }
        let id = self.next_id;
        self.next_id += 1;
        let mut client =
            DapClient::spawn(id, &cfg.adapter.command, &cfg.adapter.args, self.tx.clone())?;
        // DAP handshake: initialize first. On its response we send `launch`;
        // the adapter then emits `initialized`, on which we send breakpoints +
        // configurationDone. (Order matters: lldb-dap emits `initialized` only
        // after it accepts `launch`.)
        let seq = client.send_request(
            "initialize",
            json!({
                "clientID": "turboreview",
                "adapterID": cfg.adapter.command,
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path",
            }),
        )?;
        let mut pending = HashMap::new();
        pending.insert(seq, Pending::Initialize);
        self.live.insert(
            id,
            Live {
                client,
                pending,
                bp_sent: false,
                top_frame: None,
            },
        );
        // Stash the launch config so the `initialized` handler can use it.
        self.launch_cfg.insert(id, (cfg.clone(), source_root.to_path_buf()));

        let d = app.debug.get_or_insert_with(DebugState::default);
        d.sessions.push(DebugSession::new(id, label));
        d.active = d.sessions.len() - 1;
        Ok(())
    }

    /// Drain all queued adapter messages, updating `app` state and issuing any
    /// follow-up requests. Call once per UI tick.
    pub fn drain(&mut self, app: &mut App) {
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(app, msg);
        }
    }

    fn handle(&mut self, app: &mut App, msg: SessionMsg) {
        let id = msg.session;
        match msg.kind {
            SessionKind::Closed | SessionKind::Error(_) => {
                set_state(app, id, SessionState::Exited);
                self.live.remove(&id);
                self.launch_cfg.remove(&id);
            }
            SessionKind::Message(m) => self.handle_message(app, id, m),
        }
    }

    fn handle_message(&mut self, app: &mut App, id: SessionId, m: Message) {
        match m {
            Message::Event { event, body, .. } => self.handle_event(app, id, &event, &body),
            Message::Response {
                request_seq,
                success,
                body,
                ..
            } => {
                let kind = self
                    .live
                    .get_mut(&id)
                    .and_then(|l| l.pending.remove(&request_seq));
                if let Some(kind) = kind {
                    if success {
                        self.handle_response(app, id, kind, &body);
                    }
                }
            }
            Message::Request { .. } => { /* reverse requests unhandled in P1 */ }
        }
    }

    fn handle_event(&mut self, app: &mut App, id: SessionId, event: &str, body: &Value) {
        match event {
            "initialized" => {
                // The adapter emits `initialized` AFTER it accepts `launch`. Now
                // is the time to register breakpoints and finish configuration.
                self.send_breakpoints(app, id);
                if let Some(l) = self.live.get_mut(&id) {
                    let _ = l.client.send_request("configurationDone", json!({}));
                }
                set_state(app, id, SessionState::Running);
            }
            "stopped" => {
                let thread = body.get("threadId").and_then(Value::as_i64);
                set_state(app, id, SessionState::Stopped);
                if let Some(sess) = session_mut(app, id) {
                    sess.stopped_thread = thread;
                }
                // Ask for the call stack of the stopped thread.
                if let (Some(l), Some(t)) = (self.live.get_mut(&id), thread) {
                    if let Ok(seq) = l
                        .client
                        .send_request("stackTrace", json!({"threadId": t, "levels": 50}))
                    {
                        l.pending.insert(seq, Pending::StackTrace);
                    }
                }
            }
            "terminated" | "exited" => {
                // Debuggee finished: clear the stop so the ▶ marker and stale
                // stack/locals disappear.
                if let Some(sess) = session_mut(app, id) {
                    sess.state = SessionState::Exited;
                    sess.stopped_thread = None;
                    sess.stopped_at = None;
                    sess.stack.clear();
                    sess.locals.clear();
                }
            }
            _ => {}
        }
    }

    fn handle_response(&mut self, app: &mut App, id: SessionId, kind: Pending, body: &Value) {
        match kind {
            Pending::Initialize => {
                // initialize succeeded → send `launch`. The adapter replies with
                // an `initialized` event once it's ready for breakpoints.
                if let Some((cfg, root)) = self.launch_cfg.get(&id).cloned() {
                    let program = root.join(&cfg.program);
                    let cwd = if cfg.cwd.is_empty() {
                        root.clone()
                    } else {
                        root.join(&cfg.cwd)
                    };
                    if let Some(l) = self.live.get_mut(&id) {
                        let _ = l.client.send_request(
                            "launch",
                            json!({
                                "program": program,
                                "args": cfg.args,
                                "cwd": cwd,
                                "stopOnEntry": false,
                            }),
                        );
                    }
                }
            }
            Pending::StackTrace => {
                let frames = parse_stack(body);
                let top_id = body
                    .get("stackFrames")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(|f| f.get("id"))
                    .and_then(Value::as_i64);
                if let Some(sess) = session_mut(app, id) {
                    sess.stack = frames;
                    sess.frame_sel = 0;
                    sess.stopped_at = sess
                        .stack
                        .first()
                        .and_then(|f| f.file.clone().map(|p| (p.into(), f.line)));
                }
                if let (Some(l), Some(fid)) = (self.live.get_mut(&id), top_id) {
                    l.top_frame = Some(fid);
                    if let Ok(seq) = l.client.send_request("scopes", json!({"frameId": fid})) {
                        l.pending.insert(seq, Pending::Scopes);
                    }
                }
            }
            Pending::Scopes => {
                // Use the first scope's variablesReference (usually "Locals").
                let var_ref = body
                    .get("scopes")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(|s| s.get("variablesReference"))
                    .and_then(Value::as_i64);
                if let (Some(l), Some(vr)) = (self.live.get_mut(&id), var_ref) {
                    if let Ok(seq) = l
                        .client
                        .send_request("variables", json!({"variablesReference": vr}))
                    {
                        l.pending.insert(seq, Pending::Variables);
                    }
                }
            }
            Pending::Variables => {
                let vars = parse_variables(body);
                if let Some(sess) = session_mut(app, id) {
                    sess.locals = vars;
                }
            }
        }
    }

    /// Send the current breakpoint set to a session (all files).
    fn send_breakpoints(&mut self, app: &App, id: SessionId) {
        let Some(d) = app.debug.as_ref() else { return };
        let Some(l) = self.live.get_mut(&id) else {
            return;
        };
        for (file, lines) in &d.breakpoints {
            let bps: Vec<Value> = lines.iter().map(|ln| json!({"line": ln})).collect();
            let _ = l.client.send_request(
                "setBreakpoints",
                json!({
                    "source": {"path": file},
                    "breakpoints": bps,
                }),
            );
        }
        l.bp_sent = true;
    }

    /// Resend the current breakpoint set to every live session (after the user
    /// toggles a breakpoint). No-op when there are no sessions.
    pub fn sync_breakpoints(&mut self, app: &App) {
        let ids: Vec<SessionId> = self.live.keys().copied().collect();
        for id in ids {
            self.send_breakpoints(app, id);
        }
    }

    /// Send a control request (`continue`/`next`/`stepIn`/`stepOut`) to the
    /// active session using its stopped thread.
    pub fn control(&mut self, app: &mut App, command: &str) {
        let Some(d) = app.debug.as_ref() else { return };
        let Some(sess) = d.active_session() else {
            return;
        };
        let (id, thread) = (sess.id, sess.stopped_thread);
        if let (Some(l), Some(t)) = (self.live.get_mut(&id), thread) {
            let _ = l.client.send_request(command, json!({"threadId": t}));
            set_state(app, id, SessionState::Running);
        }
    }

    /// Build a [`crate::dap::DebugSnapshot`] from the active session's current
    /// stop, if it is stopped with a stack.
    pub fn snapshot(&self, app: &App) -> Option<crate::dap::DebugSnapshot> {
        let sess = app.debug.as_ref()?.active_session()?;
        if sess.state != SessionState::Stopped {
            return None;
        }
        let (file, line) = sess.stopped_at.clone()?;
        // Cap the captured stack so runtime/startup frames don't bloat the
        // comment; the innermost frames are what the reviewer cares about.
        const MAX_SNAPSHOT_FRAMES: usize = 8;
        let stack: Vec<_> = sess.stack.iter().take(MAX_SNAPSHOT_FRAMES).cloned().collect();
        Some(crate::dap::DebugSnapshot {
            session_label: sess.label.clone(),
            stopped_file: file.to_string_lossy().into_owned(),
            stopped_line: line,
            stack,
            locals: sess.locals.clone(),
            captured: crate::storage::now_secs(),
        })
    }

    /// Disconnect and reap every live adapter. Call on exiting debug mode / app.
    pub fn shutdown(&mut self) {
        for (_, mut l) in self.live.drain() {
            l.client.shutdown();
        }
        self.launch_cfg.clear();
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn session_mut(app: &mut App, id: SessionId) -> Option<&mut DebugSession> {
    app.debug
        .as_mut()?
        .sessions
        .iter_mut()
        .find(|s| s.id == id)
}

fn set_state(app: &mut App, id: SessionId, state: SessionState) {
    if let Some(s) = session_mut(app, id) {
        s.state = state;
    }
}

fn parse_stack(body: &Value) -> Vec<crate::dap::Frame> {
    body.get("stackFrames")
        .and_then(Value::as_array)
        .map(|frames| {
            frames
                .iter()
                .map(|f| crate::dap::Frame {
                    name: f
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    file: f
                        .get("source")
                        .and_then(|s| s.get("path"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    line: f.get("line").and_then(Value::as_i64).unwrap_or(0) as u32,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_variables(body: &Value) -> Vec<crate::dap::VarRow> {
    body.get("variables")
        .and_then(Value::as_array)
        .map(|vars| {
            vars.iter()
                .map(|v| crate::dap::VarRow {
                    name: v
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    value: v
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    ty: v.get("type").and_then(Value::as_str).map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stack_extracts_frames() {
        let body = json!({
            "stackFrames": [
                {"id": 1, "name": "main", "line": 42, "source": {"path": "/repo/src/main.rs"}},
                {"id": 2, "name": "helper", "line": 7, "source": {"path": "/repo/src/lib.rs"}}
            ]
        });
        let frames = parse_stack(&body);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].name, "main");
        assert_eq!(frames[0].line, 42);
        assert_eq!(frames[0].file.as_deref(), Some("/repo/src/main.rs"));
    }

    #[test]
    fn parse_variables_extracts_rows() {
        let body = json!({
            "variables": [
                {"name": "x", "value": "1", "type": "i32"},
                {"name": "s", "value": "\"hi\""}
            ]
        });
        let vars = parse_variables(&body);
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].name, "x");
        assert_eq!(vars[0].ty.as_deref(), Some("i32"));
        assert_eq!(vars[1].ty, None);
    }

    #[test]
    fn run_build_skips_when_empty() {
        let cfg = DebugConfig::default();
        assert!(DebugManager::run_build(&cfg, Path::new(".")).is_ok());
    }

    #[test]
    fn run_build_reports_failure() {
        let mut cfg = DebugConfig::default();
        cfg.build = "exit 3".into();
        assert!(DebugManager::run_build(&cfg, Path::new(".")).is_err());
    }
}
