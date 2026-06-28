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
/// Scopes/Variables carry the index of the stack frame they belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Initialize,
    StackTrace,
    Scopes(usize),
    Variables(usize),
    /// Children of an expanded structured var: (frame index, path key).
    VarChildren(usize, u64),
}

/// Whether a session debuggee is launched locally or attached to a remote.
#[derive(Clone, Debug)]
enum LaunchKind {
    /// `launch` a freshly-built program.
    Launch,
    /// `attach` to a remote target via these adapter commands.
    Attach(Vec<String>),
    /// `attach` to a running local process by pid.
    AttachPid(i64),
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
    /// Var-expansion paths, keyed by an id stored in `Pending::VarChildren`
    /// (since `Pending` is `Copy` and can't hold a `Vec`). Value =
    /// (frame index, var path, remaining auto-expand depth; 0 = user-initiated).
    var_paths: HashMap<u64, (usize, Vec<usize>, usize)>,
    next_var_key: u64,
    /// Temp git worktree backing this session (for old-commit debugging), to be
    /// removed when the session ends.
    worktree: Option<std::path::PathBuf>,
}

/// Owns all live debug sessions and the shared event channel.
pub struct DebugManager {
    tx: Sender<SessionMsg>,
    rx: Receiver<SessionMsg>,
    live: HashMap<SessionId, Live>,
    /// Per-session launch config (cfg + source root), used by the `initialized`
    /// handler to send `launch`.
    launch_cfg: HashMap<SessionId, (DebugConfig, std::path::PathBuf, LaunchKind)>,
    /// Worktrees of ended sessions awaiting removal (cleaned by the event loop,
    /// which has the `Repo` handle). See `take_dead_worktrees`.
    dead_worktrees: Vec<std::path::PathBuf>,
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
            dead_worktrees: Vec::new(),
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
        self.launch_inner(app, cfg, source_root, label, None, LaunchKind::Launch)
    }

    /// Attach to a remote target (gdbserver / Docker) per `cfg.remote`. No build;
    /// the adapter runs the remote attach commands. `source_map` should map the
    /// remote source paths to the local repo so frames resolve to the diff.
    pub fn attach_remote(&mut self, app: &mut App, cfg: &DebugConfig, repo_root: &Path) -> Result<()> {
        if !cfg.remote.is_set() {
            anyhow::bail!("no remote target configured (set debug.remote.host/port)");
        }
        let label = if !cfg.remote.host.is_empty() {
            format!("remote {}:{}", cfg.remote.host, cfg.remote.port)
        } else {
            "remote".to_string()
        };
        let commands = cfg.remote.commands();
        self.launch_inner(app, cfg, repo_root, label, None, LaunchKind::Attach(commands))
    }

    /// Attach to a running local process by pid (no build).
    pub fn attach_process(
        &mut self,
        app: &mut App,
        cfg: &DebugConfig,
        repo_root: &Path,
        pid: i64,
        name: &str,
    ) -> Result<()> {
        let label = format!("pid {pid} ({name})");
        self.launch_inner(app, cfg, repo_root, label, None, LaunchKind::AttachPid(pid))
    }

    /// Debug a past commit: create a detached worktree at `sha`, build + launch
    /// there, and map the worktree's source paths back to the repo so the diff
    /// and breakpoints line up. The worktree is cleaned up when the session ends.
    pub fn launch_commit(
        &mut self,
        app: &mut App,
        cfg: &DebugConfig,
        repo: &crate::git::Repo,
        sha: &str,
        short: &str,
    ) -> Result<()> {
        let wt = repo.add_worktree(sha)?;
        // Source map: frames reported under the worktree map to the repo root so
        // they resolve to the open diff. (DAP `sourceReference`/paths; we record
        // it in cfg for adapters that honor source maps, and rely on path
        // basename matching otherwise.)
        let mut cfg = cfg.clone();
        if let Ok(repo_root) = repo.workdir() {
            cfg.source_map.push((
                wt.to_string_lossy().into_owned(),
                repo_root.to_string_lossy().into_owned(),
            ));
        }
        let label = format!("commit {short}");
        let res = self.launch_inner(app, &cfg, &wt, label, Some(wt.clone()), LaunchKind::Launch);
        if res.is_err() {
            // Build/launch failed — don't leak the worktree.
            repo.remove_worktree(&wt);
        }
        res
    }

    fn launch_inner(
        &mut self,
        app: &mut App,
        cfg: &DebugConfig,
        source_root: &Path,
        label: String,
        worktree: Option<std::path::PathBuf>,
        kind: LaunchKind,
    ) -> Result<()> {
        // Build only when launching a local program (attach uses a live target).
        if matches!(kind, LaunchKind::Launch) {
            Self::run_build(cfg, source_root)?;
        }
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
                var_paths: HashMap::new(),
                next_var_key: 1,
                worktree,
            },
        );
        // Stash the launch config so the `initialized` handler can use it.
        self.launch_cfg
            .insert(id, (cfg.clone(), source_root.to_path_buf(), kind));

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
                if let Some(l) = self.live.remove(&id) {
                    if let Some(wt) = l.worktree {
                        self.dead_worktrees.push(wt);
                    }
                }
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
                // initialize succeeded → send `launch` or `attach` by kind. The
                // adapter replies with an `initialized` event when ready for
                // breakpoints.
                if let Some((cfg, root, kind)) = self.launch_cfg.get(&id).cloned() {
                    let Some(l) = self.live.get_mut(&id) else {
                        return;
                    };
                    match kind {
                        LaunchKind::Launch => {
                            let program = root.join(&cfg.program);
                            let cwd = if cfg.cwd.is_empty() {
                                root.clone()
                            } else {
                                root.join(&cfg.cwd)
                            };
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
                        LaunchKind::Attach(commands) => {
                            // lldb-dap: run the remote-connect commands on attach.
                            let _ = l.client.send_request(
                                "attach",
                                json!({
                                    "attachCommands": commands,
                                }),
                            );
                        }
                        LaunchKind::AttachPid(pid) => {
                            let _ = l.client.send_request("attach", json!({ "pid": pid }));
                        }
                    }
                }
            }
            Pending::StackTrace => {
                let mut frames = parse_stack(body);
                // For an old-commit session, rewrite worktree source paths back
                // to the repo root so frames resolve to the open diff + markers.
                if let Some(wt) = self.live.get(&id).and_then(|l| l.worktree.clone()) {
                    let repo_root = app.repo_root.clone();
                    let wt_str = wt.to_string_lossy().into_owned();
                    let root_str = repo_root.to_string_lossy().into_owned();
                    for f in frames.iter_mut() {
                        if let Some(p) = f.file.as_ref() {
                            if let Some(rel) = p.strip_prefix(&wt_str) {
                                f.file = Some(format!("{root_str}{rel}"));
                            }
                        }
                    }
                }
                if let Some(sess) = session_mut(app, id) {
                    sess.stack = frames;
                    sess.frame_sel = 0;
                    sess.locals.clear();
                    sess.stopped_at = sess
                        .stack
                        .first()
                        .and_then(|f| f.file.clone().map(|p| (p.into(), f.line)));
                }
                // Fetch locals for the top frame only. lldb-dap's scope
                // `variablesReference` is stateful (tied to the last-selected
                // frame), so parallel per-frame requests race and return the
                // same locals — we fetch one frame at a time, on demand
                // (see `request_frame_locals`, called when the selection moves).
                let top = session_mut(app, id)
                    .and_then(|s| s.stack.first().map(|f| f.id));
                if let (Some(l), Some(fid)) = (self.live.get_mut(&id), top) {
                    l.top_frame = Some(fid);
                    if let Ok(seq) = l.client.send_request("scopes", json!({"frameId": fid})) {
                        l.pending.insert(seq, Pending::Scopes(0));
                    }
                }
            }
            Pending::Scopes(frame_idx) => {
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
                        l.pending.insert(seq, Pending::Variables(frame_idx));
                    }
                }
            }
            Pending::Variables(frame_idx) => {
                let vars = parse_variables(body);
                let next = session_mut(app, id).map(|sess| {
                    if let Some(f) = sess.stack.get_mut(frame_idx) {
                        f.locals = vars.clone();
                    }
                    // Mirror the top frame's locals into the session-level field
                    // used by the snapshot + the simple variables view.
                    if frame_idx == 0 {
                        sess.locals = vars;
                    }
                    sess.stack.len()
                });
                // Eagerly resolve this frame's structured vars so a captured
                // snapshot includes the full heap contents.
                self.auto_expand_frame(app, id, frame_idx);
                // Chain to the next frame (serially, to avoid lldb-dap's stateful
                // variablesReference race) so all frames' locals are eventually
                // populated and can be captured into a comment snapshot.
                const MAX_FRAMES_WITH_LOCALS: usize = 8;
                if let Some(stack_len) = next {
                    let next_idx = frame_idx + 1;
                    if next_idx < stack_len.min(MAX_FRAMES_WITH_LOCALS) {
                        let fid = session_mut(app, id)
                            .and_then(|s| s.stack.get(next_idx).map(|f| f.id));
                        if let (Some(l), Some(fid)) = (self.live.get_mut(&id), fid) {
                            if let Ok(seq) =
                                l.client.send_request("scopes", json!({"frameId": fid}))
                            {
                                l.pending.insert(seq, Pending::Scopes(next_idx));
                            }
                        }
                    }
                }
            }
            Pending::VarChildren(_frame, key) => {
                let children = parse_variables(body);
                if let Some((frame, path, depth)) =
                    self.live.get_mut(&id).and_then(|l| l.var_paths.remove(&key))
                {
                    // Auto-expanded vars (depth > 0) are marked expanded so the
                    // resolved tree shows + persists.
                    if depth > 0 {
                        app.set_var_expanded(frame, &path, true);
                    }
                    app.set_var_children(frame, &path, children.clone());
                    // Recurse into structured grandchildren while depth remains.
                    if depth > 1 {
                        let sid = id;
                        for (ci, c) in children.iter().enumerate() {
                            if c.var_ref > 0 {
                                let mut cpath = path.clone();
                                cpath.push(ci);
                                self.fire_var_request(sid, frame, c.var_ref, cpath, depth - 1);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Request locals for the active session's frame `idx`, if not already
    /// fetched. lldb-dap resolves a scope's `variablesReference` against the
    /// last-selected frame, so we request scopes for exactly this frame, one at
    /// a time, when the user navigates to it.
    pub fn request_frame_locals(&mut self, app: &App, idx: usize) {
        let Some(d) = app.debug.as_ref() else { return };
        let Some(sess) = d.active_session() else { return };
        let Some(frame) = sess.stack.get(idx) else { return };
        if !frame.locals.is_empty() {
            return; // already have them
        }
        let (sid, fid) = (sess.id, frame.id);
        if let Some(l) = self.live.get_mut(&sid) {
            if let Ok(seq) = l.client.send_request("scopes", json!({"frameId": fid})) {
                l.pending.insert(seq, Pending::Scopes(idx));
            }
        }
    }

    /// Request the children of a structured variable (`var_ref`) so an expanded
    /// String/Vec/struct shows its contents. `path` identifies the var in the
    /// active session's frame `frame_idx`.
    pub fn request_var_children(&mut self, app: &App, frame_idx: usize, var_ref: i64, path: Vec<usize>) {
        let Some(sid) = app.debug.as_ref().and_then(|d| d.active_session()).map(|s| s.id) else {
            return;
        };
        self.fire_var_request(sid, frame_idx, var_ref, path, 0);
    }

    /// Maximum auto-expand depth when eagerly resolving a frame's structured
    /// vars (so a captured snapshot holds the heap contents). Bounds work on
    /// large/cyclic structures.
    const AUTO_EXPAND_DEPTH: usize = 3;

    /// Eagerly request children for every structured variable in `frame_idx`'s
    /// locals so the full value tree is resolved (for snapshot capture). Marks
    /// the vars expanded as their children arrive.
    fn auto_expand_frame(&mut self, app: &App, sid: SessionId, frame_idx: usize) {
        let reqs: Vec<(i64, Vec<usize>)> = app
            .debug
            .as_ref()
            .and_then(|d| d.active_session())
            .and_then(|s| s.stack.get(frame_idx))
            .map(|f| {
                f.locals
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.var_ref > 0 && v.children.is_empty())
                    .map(|(i, v)| (v.var_ref, vec![i]))
                    .collect()
            })
            .unwrap_or_default();
        for (var_ref, path) in reqs {
            self.fire_var_request(sid, frame_idx, var_ref, path, Self::AUTO_EXPAND_DEPTH);
        }
    }

    /// Send a `variables` request for `var_ref`, tagging it with the path +
    /// remaining auto-expand depth so the response can be routed and (if depth
    /// remains) recurse into structured children.
    fn fire_var_request(
        &mut self,
        sid: SessionId,
        frame_idx: usize,
        var_ref: i64,
        path: Vec<usize>,
        depth: usize,
    ) {
        if let Some(l) = self.live.get_mut(&sid) {
            let key = l.next_var_key;
            l.next_var_key += 1;
            l.var_paths.insert(key, (frame_idx, path, depth));
            if let Ok(seq) = l
                .client
                .send_request("variables", json!({"variablesReference": var_ref}))
            {
                l.pending.insert(seq, Pending::VarChildren(frame_idx, key));
            }
        }
    }

    /// Send the current breakpoint set to a session (all files).
    fn send_breakpoints(&mut self, app: &App, id: SessionId) {
        let Some(d) = app.debug.as_ref() else { return };
        let Some(l) = self.live.get_mut(&id) else {
            return;
        };
        let worktree = l.worktree.clone();
        let repo_root = app.repo_root.clone();
        for (file, lines) in &d.breakpoints {
            // For an old-commit (worktree) session, the debuggee's source lives
            // under the worktree, so rewrite repo-rooted breakpoint paths to it.
            let target: std::path::PathBuf = match &worktree {
                Some(wt) => match file.strip_prefix(&repo_root) {
                    Ok(rel) => wt.join(rel),
                    Err(_) => file.clone(),
                },
                None => file.clone(),
            };
            // Only enabled breakpoints are sent; disabled ones stay in the list
            // but are omitted (sending an empty set clears them in the adapter).
            let bps: Vec<Value> = lines
                .iter()
                .filter(|(_, on)| **on)
                .map(|(ln, _)| json!({"line": ln}))
                .collect();
            let _ = l.client.send_request(
                "setBreakpoints",
                json!({
                    "source": {"path": target},
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
            if let Some(wt) = l.worktree {
                self.dead_worktrees.push(wt);
            }
        }
        self.launch_cfg.clear();
    }

    /// Take the list of worktrees from ended sessions so the caller (which holds
    /// the `Repo`) can remove them.
    pub fn take_dead_worktrees(&mut self) -> Vec<std::path::PathBuf> {
        std::mem::take(&mut self.dead_worktrees)
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
                    id: f.get("id").and_then(Value::as_i64).unwrap_or(0),
                    locals: Vec::new(),
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
                // Drop adapter error pseudo-variables (e.g. lldb's "<error>" with
                // "no variable information available") so they don't show as junk.
                .filter(|v| {
                    let name = v.get("name").and_then(Value::as_str).unwrap_or("");
                    let val = v.get("value").and_then(Value::as_str).unwrap_or("");
                    name != "<error>" && !val.contains("no variable information")
                })
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
                    var_ref: v
                        .get("variablesReference")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    memory_ref: v
                        .get("memoryReference")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    expanded: false,
                    children: Vec::new(),
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
    fn parse_variables_captures_ref_and_address() {
        // A heap value (String) typically has a variablesReference (expandable)
        // and a memoryReference (address).
        let body = json!({
            "variables": [
                {"name": "s", "value": "\"hi\"", "type": "alloc::string::String",
                 "variablesReference": 1004, "memoryReference": "0x16fdff2a0"}
            ]
        });
        let vars = parse_variables(&body);
        assert_eq!(vars[0].var_ref, 1004);
        assert_eq!(vars[0].memory_ref.as_deref(), Some("0x16fdff2a0"));
        assert_eq!(vars[0].ty.as_deref(), Some("alloc::string::String"));
    }

    #[test]
    fn parse_variables_drops_error_pseudovars() {
        let body = json!({
            "variables": [
                {"name": "<error>", "value": "no variable information is available", "type": "const char *"},
                {"name": "n", "value": "10", "type": "u32"}
            ]
        });
        let vars = parse_variables(&body);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "n");
    }

    #[test]
    fn parse_stack_sets_frame_id() {
        let body = json!({
            "stackFrames": [
                {"id": 9, "name": "f", "line": 1, "source": {"path": "/a.rs"}}
            ]
        });
        let frames = parse_stack(&body);
        assert_eq!(frames[0].id, 9);
        assert!(frames[0].locals.is_empty());
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
