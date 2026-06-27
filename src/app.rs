use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::comments::Comments;
use crate::tree::{Row, RowKind};

/// Which storage scope is currently active for comments and reviewed flags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommentScope {
    /// Working-tree scope: `.turboreview/`
    Worktree,
    /// Per-commit scope: `.turboreview/commits/<sha>/`
    Commit(String),
}

/// Active file-history overlay over the diff pane.
#[derive(Clone, Debug)]
pub struct FileHistory {
    /// The file whose history is being browsed (repo-relative).
    pub file: PathBuf,
    /// Commits touching `file`, newest first. Capped at MAX_FILE_HISTORY.
    pub commits: Vec<crate::git::CommitInfo>,
    /// 0 = baseline (live diff). 1..=commits.len() = commits[idx-1].
    pub idx: usize,
    /// Comment scope active when H was pressed; restored on exit.
    pub baseline_scope: CommentScope,
}

/// In-diff substring search state (committed query with matches).
#[derive(Clone, Debug)]
pub struct SearchState {
    /// Lowercased query (matching is case-insensitive).
    pub query: String,
    /// Indices into `diff` of matching lines, ascending.
    pub matches: Vec<usize>,
    /// Position within `matches` of the focused match.
    pub cur: usize,
}

const MAX_FILE_HISTORY: usize = 50;
/// Largest hunks-only context before `+` switches to full-file diff.
pub const MAX_CONTEXT_LINES: u32 = 50;
const MAX_HSCROLL: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Unstaged,
    Staged,
    Commit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Unstaged,
    Staged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Files,
    Diff,
    /// The right pane. It is tabbed (see `RightTab`): Comments or Debug.
    Comments,
}

/// Which tab the right pane shows. Switched with `[` / `]` when focused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RightTab {
    #[default]
    Comments,
    Debug,
}

/// A row in the comment-list pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommentRow {
    /// A status group header: (status, count)
    Header(crate::comments::CommentStatus, usize),
    /// An item: index into app.comments.items
    Item(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Changes,
    Commits,
}

/// Run-state of a single debug session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// Launching / building, no adapter responses yet.
    Starting,
    /// Debuggee running (not at a breakpoint).
    Running,
    /// Stopped at a breakpoint/step; stack + variables are populated.
    Stopped,
    /// Debuggee exited or adapter terminated.
    Exited,
}

/// One debug session's UI-facing state. The live adapter handle (process +
/// request channel) is attached by the threaded client layer; this struct holds
/// only what the UI renders so it stays `Clone`/testable.
#[derive(Clone, Debug)]
pub struct DebugSession {
    /// Stable id assigned at spawn (also tags incoming events).
    pub id: u64,
    /// Human label, e.g. "worktree" or "commit a1b2c3".
    pub label: String,
    pub state: SessionState,
    /// Thread the adapter last reported stopped on (for follow-up requests).
    pub stopped_thread: Option<i64>,
    /// File + line where it stopped (absolute path), if known.
    pub stopped_at: Option<(PathBuf, u32)>,
    /// Current call stack (innermost first).
    pub stack: Vec<crate::dap::Frame>,
    /// Index of the selected stack frame.
    pub frame_sel: usize,
    /// Locals for the selected frame.
    pub locals: Vec<crate::dap::VarRow>,
}

impl DebugSession {
    pub fn new(id: u64, label: String) -> Self {
        DebugSession {
            id,
            label,
            state: SessionState::Starting,
            stopped_thread: None,
            stopped_at: None,
            stack: Vec::new(),
            frame_sel: 0,
            locals: Vec::new(),
        }
    }
}

/// Top-level debugger overlay state. Present (`Some`) only while debugging.
/// Breakpoints are keyed by absolute source path → set of 1-based line numbers,
/// shared across all sessions.
/// Which tab the right-hand debug pane is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DebugTab {
    /// Call stack + variables of the active session.
    #[default]
    Vars,
    /// The breakpoint list (navigate / enable / delete).
    Breakpoints,
}

#[derive(Clone, Debug, Default)]
pub struct DebugState {
    pub sessions: Vec<DebugSession>,
    /// Index into `sessions` of the active session (panel focus).
    pub active: usize,
    /// Which tab the debug pane shows.
    pub tab: DebugTab,
    /// Breakpoints: absolute source path → (1-based line → enabled). Disabled
    /// breakpoints are kept (greyed in the list) but not sent to adapters.
    pub breakpoints: std::collections::BTreeMap<PathBuf, std::collections::BTreeMap<u32, bool>>,
    /// Selection index within the active session's variables/stack panel.
    pub panel_sel: usize,
    /// Selection index within the breakpoint-list pane.
    pub bp_sel: usize,
    /// Horizontal scroll offset (chars) for the debug pane's content.
    pub hscroll: usize,
}

impl DebugState {
    pub fn active_session(&self) -> Option<&DebugSession> {
        self.sessions.get(self.active)
    }

    /// Flat, ordered list of all breakpoints as `(file, line, enabled)`. Order
    /// matches the breakpoint pane (by path, then line).
    pub fn breakpoint_list(&self) -> Vec<(PathBuf, u32, bool)> {
        self.breakpoints
            .iter()
            .flat_map(|(f, lines)| lines.iter().map(move |(l, on)| (f.clone(), *l, *on)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Added,
    Modified,
    Deleted,
    Renamed,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Hunk,
    Add,
    Del,
    Context,
}

#[derive(Clone, Debug)]
pub struct FileChange {
    pub path: PathBuf,
    pub status: Status,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
}

impl DiffLine {
    pub fn context(text: &str, old: u32, new: u32) -> Self {
        DiffLine {
            kind: LineKind::Context,
            text: text.into(),
            old_lineno: Some(old),
            new_lineno: Some(new),
        }
    }
}

/// State for the modal comment input box.
pub struct InputState {
    pub buffer: String, // current text (may contain \n for multi-line)
    pub target_file: PathBuf,
    pub target_line: u32,
    pub target_hunk: String,
    /// Anchor captured at the moment the modal was opened (Fix 4: don't re-derive at Ctrl-S).
    pub anchor_line_text: String,
    pub anchor_before: Vec<String>,
    pub anchor_after: Vec<String>,
    /// Debug snapshot available to attach (set when a session is stopped at this
    /// line). Shown in the modal; only saved onto the comment when `attach_debug`.
    pub debug_snapshot: Option<crate::dap::DebugSnapshot>,
    /// Whether the snapshot will be attached on save (toggled with Ctrl-D).
    pub attach_debug: bool,
}

/// Result of committing a comment input (returned by `input_commit`).
pub struct CommittedComment {
    pub file: PathBuf,
    pub line: u32,
    pub hunk: String,
    pub text: String,
    pub line_text: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
    /// Debug snapshot to attach to the comment (None unless the user kept it on).
    pub debug_snapshot: Option<crate::dap::DebugSnapshot>,
}

pub struct App {
    pub repo_root: PathBuf,
    pub focus: Pane,
    pub view: ViewMode,
    pub unstaged: Vec<FileChange>,
    pub staged: Vec<FileChange>,
    pub selected: usize,
    pub diff: Vec<DiffLine>,
    pub diff_cursor: usize,
    pub diff_hscroll: usize,
    /// Some(anchor) when visual-select is active; anchor is a diff index.
    pub select_anchor: Option<usize>,
    pub reviewed: HashSet<PathBuf>,
    pub status_msg: Option<String>,
    pub collapsed: HashSet<(Section, PathBuf)>,
    pub rows: Vec<Row>,
    pub hide_reviewed: bool,
    pub context_lines: u32,
    pub full_file: bool,
    pub show_files: bool,
    pub file_pane_pct: u16,
    pub comments: Comments,
    pub input: Option<InputState>,
    pub commits: Vec<crate::git::CommitInfo>,
    pub selected_commit: usize,
    /// Number of commits currently requested from git (page size, grows on demand).
    pub commit_limit: usize,
    /// Lazily-computed per-commit diff stats, keyed by full oid. Filled for the
    /// visible window each frame; missing entries render a placeholder.
    pub commit_stats: std::collections::HashMap<String, crate::git::CommitStat>,
    pub open_commit: Option<String>,
    pub commit_files: Vec<FileChange>,
    pub show_help: bool,
    pub comment_scope: CommentScope,
    pub show_comments: bool,
    pub comment_selected: usize,
    pub theme: crate::theme::Theme,
    /// false = unified diff (default), true = side-by-side (split) diff.
    pub split_diff: bool,
    pub history: Option<FileHistory>,
    pub search: Option<SearchState>,
    pub search_input: Option<String>,
    /// Debugger overlay state; `Some` only while debugging.
    pub debug: Option<DebugState>,
    /// Which tab the right pane shows (Comments or Debug).
    pub right_tab: RightTab,
}

enum RowId {
    File(Section, std::path::PathBuf),
    Dir(Section, std::path::PathBuf),
}

impl App {
    pub fn new(unstaged: Vec<FileChange>, staged: Vec<FileChange>, repo_root: PathBuf) -> Self {
        let mut app = App {
            repo_root,
            focus: Pane::Files,
            view: ViewMode::Changes,
            unstaged,
            staged,
            selected: 0,
            diff: Vec::new(),
            diff_cursor: 0,
            diff_hscroll: 0,
            select_anchor: None,
            reviewed: HashSet::new(),
            status_msg: None,
            collapsed: HashSet::new(),
            rows: Vec::new(),
            hide_reviewed: false,
            context_lines: 3,
            full_file: false,
            show_files: true,
            file_pane_pct: 25,
            comments: Comments::default(),
            input: None,
            commits: Vec::new(),
            selected_commit: 0,
            commit_limit: crate::COMMIT_PAGE,
            commit_stats: std::collections::HashMap::new(),
            open_commit: None,
            commit_files: Vec::new(),
            show_help: false,
            comment_scope: CommentScope::Worktree,
            show_comments: false,
            comment_selected: 0,
            theme: crate::theme::Theme::Dark,
            split_diff: false,
            history: None,
            search: None,
            search_input: None,
            debug: None,
            right_tab: RightTab::Comments,
        };
        app.rebuild_rows();
        app
    }

    pub fn toggle_full_file(&mut self) {
        self.full_file = !self.full_file;
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn toggle_theme(&mut self) {
        self.theme = match self.theme {
            crate::theme::Theme::Dark => crate::theme::Theme::Light,
            crate::theme::Theme::Light => crate::theme::Theme::Dark,
        };
    }

    /// Toggle side-by-side (split) diff rendering.
    pub fn toggle_split(&mut self) {
        self.split_diff = !self.split_diff;
    }

    pub fn palette(&self) -> crate::theme::Palette {
        crate::theme::Palette::for_theme(self.theme)
    }

    pub fn toggle_files(&mut self) {
        self.show_files = !self.show_files;
        // Only move focus if the hidden pane was the focused one.
        if !self.show_files && self.focus == Pane::Files {
            self.focus = Pane::Diff;
        }
    }

    pub fn widen_files(&mut self) {
        self.file_pane_pct = (self.file_pane_pct + 5).min(60);
    }

    pub fn narrow_files(&mut self) {
        self.file_pane_pct = self.file_pane_pct.saturating_sub(5).max(10);
    }

    pub fn effective_context(&self) -> u32 {
        if self.full_file {
            u32::MAX
        } else {
            self.context_lines
        }
    }

    pub fn rebuild_rows(&mut self) {
        let prev = self.selected_identity();
        let empty = HashSet::new();
        let hidden = if self.hide_reviewed {
            &self.reviewed
        } else {
            &empty
        };
        self.rows = if self.view == ViewMode::Commits && self.open_commit.is_some() {
            crate::tree::build_commit_rows(&self.commit_files, &self.collapsed, hidden)
        } else {
            crate::tree::build_rows(&self.unstaged, &self.staged, &self.collapsed, hidden)
        };
        self.selected = match prev.and_then(|id| self.find_row(&id)) {
            Some(i) => i,
            None => self.selected.min(self.rows.len().saturating_sub(1)),
        };
    }

    fn selected_identity(&self) -> Option<RowId> {
        let row = self.rows.get(self.selected)?;
        match &row.kind {
            RowKind::File {
                section,
                file_index,
            } => {
                return self
                    .section_files(*section)
                    .get(*file_index)
                    .map(|f| RowId::File(*section, f.path.clone()));
            }
            RowKind::Dir { section, path, .. } => Some(RowId::Dir(*section, path.clone())),
            RowKind::Header { .. } => None,
        }
    }

    fn find_row(&self, id: &RowId) -> Option<usize> {
        self.rows.iter().position(|r| match (&r.kind, id) {
            (
                RowKind::File {
                    section: rs,
                    file_index,
                },
                RowId::File(s, p),
            ) if rs == s => self
                .section_files(*rs)
                .get(*file_index)
                .map_or(false, |f| &f.path == p),
            (
                RowKind::Dir {
                    section: rs, path, ..
                },
                RowId::Dir(s, p),
            ) if rs == s => path == p,
            _ => false,
        })
    }

    pub fn section_files(&self, section: Section) -> &[FileChange] {
        match section {
            Section::Unstaged => &self.unstaged,
            Section::Staged => &self.staged,
            Section::Commit => &self.commit_files,
        }
    }

    pub fn toggle_hide_reviewed(&mut self) {
        self.hide_reviewed = !self.hide_reviewed;
        self.rebuild_rows();
    }

    pub fn selected_path(&self) -> Option<&PathBuf> {
        match self.rows.get(self.selected) {
            Some(Row {
                kind:
                    RowKind::File {
                        section,
                        file_index,
                    },
                ..
            }) => self
                .section_files(*section)
                .get(*file_index)
                .map(|f| &f.path),
            _ => None,
        }
    }

    pub fn selected_section(&self) -> Option<Section> {
        match self.rows.get(self.selected) {
            Some(Row {
                kind: RowKind::File { section, .. },
                ..
            }) => Some(*section),
            _ => None,
        }
    }

    pub fn selected_file_index(&self) -> Option<usize> {
        match self.rows.get(self.selected) {
            Some(Row {
                kind: RowKind::File { file_index, .. },
                ..
            }) => Some(*file_index),
            _ => None,
        }
    }

    pub fn set_diff(&mut self, diff: Vec<DiffLine>) {
        self.diff = diff;
        self.diff_cursor = 0;
        self.diff_hscroll = 0;
        self.search = None;
        self.search_input = None;
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let max = self.rows.len() as isize - 1;
        let next = (self.selected as isize + delta).clamp(0, max);
        self.selected = next as usize;
    }

    pub fn scroll_h(&mut self, delta: isize) {
        let next = (self.diff_hscroll as isize + delta).max(0);
        self.diff_hscroll = (next as usize).min(MAX_HSCROLL);
    }

    pub fn move_diff_cursor(&mut self, delta: isize) {
        if self.diff.is_empty() {
            self.diff_cursor = 0;
            return;
        }
        let max = self.diff.len() as isize - 1;
        let mut pos = (self.diff_cursor as isize + delta).clamp(0, max);
        // Lines the cursor should not rest on, in the direction of travel:
        //  - Hunk headers (can't comment on them; resting makes split view jump).
        //  - In side-by-side mode, Del (old) lines too: each del/add pair shares a
        //    visual row whose new side is the comment target, so landing on the del
        //    would highlight the same row twice. Comments aren't allowed on old
        //    lines anyway, so the cursor tracks only the new side.
        let skip = |k: LineKind| -> bool {
            k == LineKind::Hunk || (self.split_diff && k == LineKind::Del)
        };
        let step = if delta >= 0 { 1 } else { -1 };
        while skip(self.diff[pos as usize].kind) {
            let next = pos + step;
            if next < 0 || next > max {
                // Edge of the diff: reverse to find the nearest acceptable line.
                let mut back = pos - step;
                while (0..=max).contains(&back) && skip(self.diff[back as usize].kind) {
                    back -= step;
                }
                if (0..=max).contains(&back) {
                    pos = back;
                }
                break;
            }
            pos = next;
        }
        self.diff_cursor = pos as usize;
    }

    pub fn select_active(&self) -> bool {
        self.select_anchor.is_some()
    }

    pub fn start_select(&mut self) {
        self.select_anchor = Some(self.diff_cursor);
    }

    pub fn cancel_select(&mut self) {
        self.select_anchor = None;
    }

    /// Inclusive (lo, hi) diff-index range of the current selection, or the
    /// single cursor line when not selecting.
    pub fn select_range(&self) -> (usize, usize) {
        match self.select_anchor {
            Some(a) => (a.min(self.diff_cursor), a.max(self.diff_cursor)),
            None => (self.diff_cursor, self.diff_cursor),
        }
    }

    /// Text of the selected range: each line's clean `text`, '\n'-joined, with a
    /// trailing '\n'. Hunk headers carry no useful content, so they are skipped.
    pub fn selection_text(&self) -> String {
        let (lo, hi) = self.select_range();
        let mut out = String::new();
        for dl in &self.diff[lo..=hi] {
            if dl.kind == LineKind::Hunk {
                continue;
            }
            out.push_str(&dl.text);
            out.push('\n');
        }
        out
    }

    pub fn to_top(&mut self) {
        match self.focus {
            Pane::Files => self.selected = 0,
            Pane::Diff => self.diff_cursor = 0,
            Pane::Comments => match self.right_tab {
                RightTab::Comments => self.comment_selected = 0,
                RightTab::Debug => {
                    if let Some(d) = self.debug.as_mut() {
                        d.panel_sel = 0;
                    }
                }
            },
        }
    }

    pub fn to_bottom(&mut self) {
        match self.focus {
            Pane::Files => self.selected = self.rows.len().saturating_sub(1),
            Pane::Diff => self.diff_cursor = self.diff.len().saturating_sub(1),
            Pane::Comments => match self.right_tab {
                RightTab::Comments => {
                    let len = self.comment_rows().len();
                    self.comment_selected = len.saturating_sub(1);
                }
                RightTab::Debug => {
                    let len = self.debug_panel_len();
                    if let Some(d) = self.debug.as_mut() {
                        d.panel_sel = len.saturating_sub(1);
                    }
                }
            },
        }
    }

    /// Whether the right pane is visible (the Comments tab is enabled, or a
    /// debug session is active so the Debug tab has content).
    pub fn right_pane_visible(&self) -> bool {
        self.show_comments || self.debug_active()
    }

    /// Cycle focus Files -> Diff -> Right -> Files. Skips Files when
    /// !show_files and the Right pane when it isn't visible.
    pub fn toggle_focus(&mut self) {
        let panes: Vec<Pane> = [Pane::Files, Pane::Diff, Pane::Comments]
            .iter()
            .copied()
            .filter(|&p| match p {
                Pane::Files => self.show_files,
                Pane::Diff => true,
                Pane::Comments => self.right_pane_visible(),
            })
            .collect();
        if panes.len() <= 1 {
            return; // nothing to cycle
        }
        let current = panes.iter().position(|&p| p == self.focus).unwrap_or(0);
        self.focus = panes[(current + 1) % panes.len()];
    }

    /// Whether the Debug tab of the right pane is focused.
    pub fn is_debug_focused(&self) -> bool {
        self.focus == Pane::Comments && self.right_tab == RightTab::Debug
    }

    /// Switch the right-pane tab (Comments <-> Debug). Debug only when active.
    pub fn toggle_right_tab(&mut self) {
        self.right_tab = match self.right_tab {
            RightTab::Comments if self.debug_active() => RightTab::Debug,
            RightTab::Debug => RightTab::Comments,
            other => other,
        };
    }

    // ─── Debugger ────────────────────────────────────────────────────────────

    /// Whether a debug session/overlay is active.
    pub fn debug_active(&self) -> bool {
        self.debug.is_some()
    }

    /// Absolute path + 1-based line of the diff line under the cursor, if it maps
    /// to a real source line (skips hunk headers and pure-deletion lines that have
    /// no new-side line number). Used to anchor breakpoints to disk locations.
    pub fn cursor_source_loc(&self) -> Option<(PathBuf, u32)> {
        let file = self.selected_path()?;
        let line = self.cursor_lineno()?;
        Some((self.repo_root.join(file), line))
    }

    /// Whether `(abs_file, line)` currently has a breakpoint (enabled or not).
    pub fn has_breakpoint(&self, file: &Path, line: u32) -> bool {
        self.debug
            .as_ref()
            .and_then(|d| d.breakpoints.get(file))
            .is_some_and(|lines| lines.contains_key(&line))
    }

    /// Whether `(abs_file, line)` has an ENABLED breakpoint (drives the marker).
    pub fn breakpoint_enabled(&self, file: &Path, line: u32) -> bool {
        self.debug
            .as_ref()
            .and_then(|d| d.breakpoints.get(file))
            .and_then(|lines| lines.get(&line))
            .copied()
            .unwrap_or(false)
    }

    /// Toggle a breakpoint on the diff line under the cursor. Lazily creates the
    /// `DebugState` so breakpoints can be set before any session is launched.
    /// Returns `true` if a breakpoint now exists at that line, `false` if it was
    /// removed (or there was no valid source line under the cursor).
    pub fn toggle_breakpoint_at_cursor(&mut self) -> bool {
        let Some((file, line)) = self.cursor_source_loc() else {
            return false;
        };
        let d = self.debug.get_or_insert_with(DebugState::default);
        let lines = d.breakpoints.entry(file.clone()).or_default();
        let now_set = if lines.contains_key(&line) {
            lines.remove(&line);
            false
        } else {
            lines.insert(line, true); // new breakpoints start enabled
            true
        };
        // Drop empty file entries to keep the map tidy.
        if lines.is_empty() {
            d.breakpoints.remove(&file);
        }
        // If we created an empty DebugState only to remove the last breakpoint
        // and there are no sessions, leave it — it's harmless and cheap, and the
        // gutter still needs the (now-empty) map. (debug_active stays true only
        // while breakpoints or sessions exist; tidy that here.)
        if d.breakpoints.is_empty() && d.sessions.is_empty() {
            self.debug = None;
        }
        now_set
    }

    // ─── Breakpoint pane ─────────────────────────────────────────────────────

    /// Number of breakpoints (for clamping the pane selection).
    pub fn breakpoint_count(&self) -> usize {
        self.debug.as_ref().map_or(0, |d| {
            d.breakpoints.values().map(|m| m.len()).sum()
        })
    }

    /// Move the selection within the breakpoint pane, clamped.
    pub fn move_breakpoint_selection(&mut self, delta: isize) {
        let len = self.breakpoint_count();
        if len == 0 {
            return;
        }
        let max = len as isize - 1;
        if let Some(d) = self.debug.as_mut() {
            d.bp_sel = (d.bp_sel as isize + delta).clamp(0, max) as usize;
        }
    }

    /// The selected breakpoint `(abs_file, line, enabled)`, if any.
    pub fn selected_breakpoint(&self) -> Option<(PathBuf, u32, bool)> {
        let d = self.debug.as_ref()?;
        d.breakpoint_list().into_iter().nth(d.bp_sel)
    }

    /// Toggle enabled/disabled for the selected breakpoint. Returns the new
    /// enabled state, or None if there's no selection.
    pub fn toggle_selected_breakpoint(&mut self) -> Option<bool> {
        let (file, line, _) = self.selected_breakpoint()?;
        let d = self.debug.as_mut()?;
        let on = d.breakpoints.get_mut(&file)?.get_mut(&line)?;
        *on = !*on;
        Some(*on)
    }

    /// Delete the selected breakpoint entirely. Returns true if one was removed.
    pub fn delete_selected_breakpoint(&mut self) -> bool {
        let Some((file, line, _)) = self.selected_breakpoint() else {
            return false;
        };
        let Some(d) = self.debug.as_mut() else {
            return false;
        };
        if let Some(lines) = d.breakpoints.get_mut(&file) {
            lines.remove(&line);
            if lines.is_empty() {
                d.breakpoints.remove(&file);
            }
        }
        let len = self.breakpoint_count();
        if let Some(d) = self.debug.as_mut() {
            d.bp_sel = d.bp_sel.min(len.saturating_sub(1));
        }
        true
    }

    /// Jump the diff cursor to the selected breakpoint's file+line. Requires the
    /// file to be the one currently shown in the diff; returns true on success.
    pub fn jump_to_selected_breakpoint(&mut self) -> bool {
        let Some((file, line, _)) = self.selected_breakpoint() else {
            return false;
        };
        // Only jump within the currently-open file's diff.
        let cur_abs = self.selected_path().map(|p| self.repo_root.join(p));
        if cur_abs.as_deref() != Some(file.as_path()) {
            self.status_msg =
                Some("breakpoint is in another file — open it first".into());
            return false;
        }
        if let Some(idx) = self
            .diff
            .iter()
            .position(|dl| dl.new_lineno == Some(line) || dl.old_lineno == Some(line))
        {
            self.diff_cursor = idx;
            self.focus = Pane::Diff;
            true
        } else {
            self.status_msg = Some("breakpoint line not visible in this diff".into());
            false
        }
    }

    /// Whether the debug pane is currently on the Breakpoints tab.
    pub fn debug_tab_is_breakpoints(&self) -> bool {
        self.debug.as_ref().map(|d| d.tab) == Some(DebugTab::Breakpoints)
    }

    /// Switch the debug pane between the Vars and Breakpoints tabs.
    pub fn toggle_debug_tab(&mut self) {
        if let Some(d) = self.debug.as_mut() {
            d.tab = match d.tab {
                DebugTab::Vars => DebugTab::Breakpoints,
                DebugTab::Breakpoints => DebugTab::Vars,
            };
        }
    }

    /// Move selection in whichever debug tab is active.
    pub fn move_debug_selection(&mut self, delta: isize) {
        match self.debug.as_ref().map(|d| d.tab) {
            Some(DebugTab::Breakpoints) => self.move_breakpoint_selection(delta),
            _ => self.move_debug_panel_selection(delta),
        }
    }

    /// Number of selectable rows in the Vars panel = the stack frames. The
    /// selected frame's locals are shown nested beneath it.
    pub fn debug_panel_len(&self) -> usize {
        match self.debug.as_ref().and_then(|d| d.active_session()) {
            Some(s) => s.stack.len(),
            None => 0,
        }
    }

    /// Move the selection within the debug panel, clamped to its row count.
    pub fn move_debug_panel_selection(&mut self, delta: isize) {
        let len = self.debug_panel_len();
        if len == 0 {
            return;
        }
        let max = len as isize - 1;
        if let Some(d) = self.debug.as_mut() {
            let next = ((d.panel_sel as isize + delta).clamp(0, max)) as usize;
            d.panel_sel = next;
            // The Vars panel selection is a stack-frame selection.
            if let Some(s) = d.sessions.get_mut(d.active) {
                s.frame_sel = next;
            }
        }
    }

    /// End all debug sessions, keeping any breakpoints. If no breakpoints
    /// remain, the debug overlay is dropped entirely. Moves focus off the Debug
    /// pane.
    pub fn exit_debug(&mut self) {
        if let Some(d) = self.debug.as_mut() {
            d.sessions.clear();
            d.active = 0;
            d.panel_sel = 0;
            if d.breakpoints.is_empty() {
                self.debug = None;
            }
        }
        // If the Debug tab was showing, fall back to the Comments tab (and move
        // focus off the right pane if it's no longer useful).
        if self.right_tab == RightTab::Debug {
            self.right_tab = RightTab::Comments;
            if self.is_debug_focused() {
                self.focus = Pane::Diff;
            }
        }
        if self.focus == Pane::Comments && !self.right_pane_visible() {
            self.focus = Pane::Diff;
        }
    }

    /// Attach a captured debug snapshot to the comment at its stopped line,
    /// creating a placeholder comment there if none exists. The stopped file is
    /// stored repo-relative to match how comments key their file.
    /// Build a debug snapshot from the active session if it is currently stopped
    /// (call stack + locals at the stop). Used to offer attaching runtime state
    /// to a comment from the comment modal. Caps the stack depth.
    pub fn current_debug_snapshot(&self) -> Option<crate::dap::DebugSnapshot> {
        let sess = self.debug.as_ref()?.active_session()?;
        if sess.state != SessionState::Stopped {
            return None;
        }
        let (file, line) = sess.stopped_at.clone()?;
        const MAX_SNAPSHOT_FRAMES: usize = 8;
        let stack = sess
            .stack
            .iter()
            .take(MAX_SNAPSHOT_FRAMES)
            .cloned()
            .collect();
        Some(crate::dap::DebugSnapshot {
            session_label: sess.label.clone(),
            stopped_file: file.to_string_lossy().into_owned(),
            stopped_line: line,
            stack,
            locals: sess.locals.clone(),
            captured: crate::storage::now_secs(),
        })
    }

    pub fn attach_debug_snapshot(&mut self, snap: crate::dap::DebugSnapshot) {
        // Map the absolute stopped path back to a repo-relative path.
        let abs = PathBuf::from(&snap.stopped_file);
        let rel = abs
            .strip_prefix(&self.repo_root)
            .map(Path::to_path_buf)
            .unwrap_or(abs);
        let line = snap.stopped_line;
        // Find an existing comment on (file,line), else create a minimal one.
        if let Some(c) = self
            .comments
            .items
            .iter_mut()
            .find(|c| c.file == rel && c.line == line)
        {
            c.debug_snapshot = Some(snap);
            c.updated = crate::storage::now_secs();
        } else {
            self.comments.set(
                rel,
                line,
                String::new(),
                String::new(),
                String::new(),
                vec![],
                vec![],
                crate::storage::now_secs(),
            );
            if let Some(c) = self.comments.items.last_mut() {
                c.debug_snapshot = Some(snap);
            }
        }
    }

    /// Toggle the comment pane. If hiding while Comments has focus, move focus to Diff.
    pub fn toggle_comment_pane(&mut self) {
        self.show_comments = !self.show_comments;
        if self.show_comments {
            // Opening the pane focuses it so it can be navigated immediately.
            self.focus = Pane::Comments;
        } else if self.focus == Pane::Comments {
            self.focus = Pane::Diff;
        }
    }

    /// Build the displayable rows for the comment-list pane.
    /// Groups items by status in order: Open, NeedsInfo, Wontfix, Resolved.
    /// Each non-empty group gets a Header(status, count) followed by Item(i) for each match,
    /// sorted by `updated` descending (newest first). Ties are stable by original index.
    pub fn comment_rows(&self) -> Vec<CommentRow> {
        use crate::comments::CommentStatus;
        let order = [
            CommentStatus::Open,
            CommentStatus::NeedsInfo,
            CommentStatus::Wontfix,
            CommentStatus::Resolved,
        ];
        let mut rows = Vec::new();
        for status in &order {
            let mut indices: Vec<usize> = self
                .comments
                .items
                .iter()
                .enumerate()
                .filter(|(_, c)| &c.status == status)
                .map(|(i, _)| i)
                .collect();
            if !indices.is_empty() {
                // Sort by updated descending (newest first); stable by original index for ties.
                indices.sort_by(|&a, &b| {
                    self.comments.items[b]
                        .updated
                        .cmp(&self.comments.items[a].updated)
                });
                rows.push(CommentRow::Header(*status, indices.len()));
                for i in indices {
                    rows.push(CommentRow::Item(i));
                }
            }
        }
        rows
    }

    /// Move the comment pane selection by `delta`, clamping to valid range.
    pub fn move_comment_selection(&mut self, delta: isize) {
        let len = self.comment_rows().len();
        if len == 0 {
            self.comment_selected = 0;
            return;
        }
        let max = len as isize - 1;
        self.comment_selected = (self.comment_selected as isize + delta).clamp(0, max) as usize;
    }

    /// Returns the Comment at the current comment_selected index, or None if it's a header.
    pub fn selected_comment(&self) -> Option<&crate::comments::Comment> {
        match self.comment_rows().get(self.comment_selected) {
            Some(CommentRow::Item(i)) => self.comments.items.get(*i),
            _ => None,
        }
    }

    /// Scan rows for a File row whose path matches `path`. If found, set self.selected and return true.
    pub fn select_row_for_path(&mut self, path: &Path) -> bool {
        for (i, row) in self.rows.iter().enumerate() {
            if let RowKind::File {
                section,
                file_index,
            } = &row.kind
            {
                if let Some(fc) = self.section_files(*section).get(*file_index) {
                    if fc.path == path {
                        self.selected = i;
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Line number under the diff cursor (`new_lineno`, else `old_lineno`).
    pub fn cursor_lineno(&self) -> Option<u32> {
        self.diff
            .get(self.diff_cursor)
            .and_then(|l| l.new_lineno.or(l.old_lineno))
    }

    /// True if any diff line carries `lineno` as its new or old line number.
    pub fn diff_has_lineno(&self, lineno: u32) -> bool {
        self.diff
            .iter()
            .any(|l| l.new_lineno == Some(lineno) || l.old_lineno == Some(lineno))
    }

    /// Scan self.diff for the first line matching `lineno` (new, then old); set diff_cursor.
    /// If not found, reset diff_cursor to 0.
    pub fn move_cursor_to_lineno(&mut self, lineno: u32) {
        for (i, dl) in self.diff.iter().enumerate() {
            if dl.new_lineno == Some(lineno) {
                self.diff_cursor = i;
                return;
            }
        }
        for (i, dl) in self.diff.iter().enumerate() {
            if dl.old_lineno == Some(lineno) {
                self.diff_cursor = i;
                return;
            }
        }
        self.diff_cursor = 0;
    }

    /// Scan self.diff for the first line whose new_lineno == new_lineno; set diff_cursor to it.
    /// If not found, reset diff_cursor to 0.
    pub fn move_cursor_to_line(&mut self, new_lineno: u32) {
        self.move_cursor_to_lineno(new_lineno);
    }

    pub fn toggle_reviewed(&mut self) {
        let Some(path) = self.selected_path().cloned() else {
            return;
        };
        if !self.reviewed.remove(&path) {
            self.reviewed.insert(path);
        }
        self.rebuild_rows();
    }

    pub fn is_reviewed_path(&self, path: &Path) -> bool {
        self.reviewed.contains(path)
    }

    /// Legacy helper used by existing tests — checks by index into the combined
    /// unstaged list (tests that use `sample()` with only unstaged files).
    pub fn is_reviewed(&self, idx: usize) -> bool {
        self.unstaged
            .get(idx)
            .map(|f| self.reviewed.contains(&f.path))
            .unwrap_or(false)
    }

    pub fn toggle_collapse(&mut self) {
        if let Some(row) = self.rows.get(self.selected) {
            if let RowKind::Dir { section, path, .. } = &row.kind {
                let key = (*section, path.clone());
                if self.collapsed.contains(&key) {
                    self.collapsed.remove(&key);
                } else {
                    self.collapsed.insert(key);
                }
                self.rebuild_rows();
            }
        }
    }

    /// Which sections are shown in the current view (for fold-all enumeration).
    fn active_sections(&self) -> Vec<Section> {
        if self.in_commit_detail() {
            vec![Section::Commit]
        } else {
            vec![Section::Unstaged, Section::Staged]
        }
    }

    /// All `(section, dir_path)` keys for every directory ancestor of every file
    /// in the active view's sections. Independent of current collapse state, so
    /// it can both fully expand and fully collapse.
    fn all_dir_keys(&self) -> HashSet<(Section, PathBuf)> {
        let mut keys = HashSet::new();
        for section in self.active_sections() {
            for fc in self.section_files(section) {
                let mut acc = PathBuf::new();
                let comps: Vec<_> = fc.path.components().collect();
                // Every component except the last (the file name) is a directory.
                for comp in comps.iter().take(comps.len().saturating_sub(1)) {
                    acc.push(comp);
                    keys.insert((section, acc.clone()));
                }
            }
        }
        keys
    }

    /// Smart fold-all toggle. If any directory in the active view is currently
    /// expanded, collapse them all; otherwise expand them all. No-op when there
    /// are no directories.
    pub fn toggle_fold_all(&mut self) {
        let dirs = self.all_dir_keys();
        if dirs.is_empty() {
            return;
        }
        let any_expanded = dirs.iter().any(|k| !self.collapsed.contains(k));
        if any_expanded {
            for k in dirs {
                self.collapsed.insert(k);
            }
        } else {
            for k in &dirs {
                self.collapsed.remove(k);
            }
        }
        self.rebuild_rows();
    }

    pub fn next_view(&mut self) {
        let prev = self.view;
        self.view = match self.view {
            ViewMode::Changes => ViewMode::Commits,
            ViewMode::Commits => ViewMode::Changes,
        };
        // When leaving Commits, clear any open commit so returning shows a fresh list.
        if prev == ViewMode::Commits && self.view == ViewMode::Changes {
            self.open_commit = None;
            self.commit_files.clear();
            self.comment_scope = CommentScope::Worktree;
        }
        // Rows reference the previous view's file set; rebuild so they can't point
        // into a now-cleared list (e.g. commit_files after leaving a commit detail).
        self.rebuild_rows();
    }

    pub fn prev_view(&mut self) {
        // With only two modes, prev and next are equivalent toggles
        self.next_view();
    }

    pub fn move_commit_selection(&mut self, delta: isize) {
        if self.commits.is_empty() {
            return;
        }
        let max = self.commits.len() as isize - 1;
        let next = (self.selected_commit as isize + delta).clamp(0, max);
        self.selected_commit = next as usize;
    }

    pub fn selected_commit_info(&self) -> Option<&crate::git::CommitInfo> {
        self.commits.get(self.selected_commit)
    }

    /// Open a commit detail view: set the commit's changed files, record its id,
    /// reset the row selection to the first row, and rebuild rows.
    /// Also sets `comment_scope` to `Commit(id)`.
    pub fn open_commit(&mut self, id: String, files: Vec<FileChange>) {
        self.comment_scope = CommentScope::Commit(id.clone());
        self.commit_files = files;
        self.open_commit = Some(id);
        self.selected = 0;
        self.rebuild_rows();
    }

    /// Close the commit detail view, returning to the commit list.
    /// Also resets `comment_scope` to `Worktree`.
    pub fn close_commit(&mut self) {
        self.open_commit = None;
        self.commit_files.clear();
        self.comment_scope = CommentScope::Worktree;
        self.rebuild_rows();
    }

    /// Return a string label for the current comment scope suitable for the comment log.
    /// `"worktree"` for working-tree scope, `"commit:<sha>"` for per-commit scope.
    pub fn scope_label(&self) -> String {
        match &self.comment_scope {
            CommentScope::Worktree => "worktree".to_string(),
            CommentScope::Commit(sha) => format!("commit:{}", sha),
        }
    }

    /// True when we are in the Commits view AND a commit has been drilled into.
    pub fn in_commit_detail(&self) -> bool {
        self.view == ViewMode::Commits && self.open_commit.is_some()
    }

    /// Return the short id of the open commit (looked up from `self.commits`).
    pub fn open_commit_short(&self) -> Option<&str> {
        let id = self.open_commit.as_deref()?;
        self.commits
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.short.as_str())
    }

    /// Public so callers can cap their `file_history` request consistently.
    pub const MAX_FILE_HISTORY: usize = MAX_FILE_HISTORY;
    pub const MAX_CONTEXT_LINES: u32 = MAX_CONTEXT_LINES;

    /// Enter history mode for the selected file. Returns false (no state change) if
    /// `commits` is empty or no file is selected. On success, idx starts at 1
    /// (newest commit) and the current comment scope is saved as the baseline.
    pub fn start_history(&mut self, commits: Vec<crate::git::CommitInfo>) -> bool {
        if commits.is_empty() {
            return false;
        }
        let Some(file) = self.selected_path().cloned() else {
            return false;
        };
        self.history = Some(FileHistory {
            file,
            commits,
            idx: 1,
            baseline_scope: self.comment_scope.clone(),
        });
        true
    }

    /// Step the history index by `delta` (+1 = older, -1 = newer), clamped to
    /// `0..=commits.len()`. No-op when not in history mode.
    pub fn history_step(&mut self, delta: isize) {
        if let Some(h) = self.history.as_mut() {
            let max = h.commits.len() as isize;
            h.idx = (h.idx as isize + delta).clamp(0, max) as usize;
        }
    }

    /// The commit for the current revision (None at baseline idx 0 or when inactive).
    pub fn history_current_commit(&self) -> Option<&crate::git::CommitInfo> {
        let h = self.history.as_ref()?;
        if h.idx == 0 {
            None
        } else {
            h.commits.get(h.idx - 1)
        }
    }

    /// Exit history mode, restoring the baseline comment scope.
    pub fn exit_history(&mut self) {
        if let Some(h) = self.history.take() {
            self.comment_scope = h.baseline_scope;
        }
    }

    pub fn history_active(&self) -> bool {
        self.history.is_some()
    }

    /// Open the search input line (typing phase). Caller guards on Diff focus.
    pub fn search_start(&mut self) {
        self.search_input = Some(String::new());
    }

    pub fn search_input_active(&self) -> bool {
        self.search_input.is_some()
    }

    pub fn search_active(&self) -> bool {
        self.search.is_some()
    }

    pub fn search_input_push(&mut self, ch: char) {
        if let Some(buf) = self.search_input.as_mut() {
            buf.push(ch);
        }
    }

    pub fn search_input_backspace(&mut self) {
        if let Some(buf) = self.search_input.as_mut() {
            buf.pop();
        }
    }

    pub fn search_input_cancel(&mut self) {
        self.search_input = None;
    }

    pub fn search_clear(&mut self) {
        self.search = None;
        self.search_input = None;
    }

    /// Commit the typed query: compute matches (case-insensitive substring) over the
    /// current diff. If none, leave `search` None and return false. Otherwise set
    /// `search` with `cur` = first match index at/after `diff_cursor` (wrapping),
    /// move `diff_cursor` there, and return true. Clears the input buffer either way.
    pub fn search_commit(&mut self) -> bool {
        let query = match self.search_input.take() {
            Some(q) if !q.is_empty() => q.to_lowercase(),
            _ => return false,
        };
        let matches: Vec<usize> = self
            .diff
            .iter()
            .enumerate()
            .filter(|(_, l)| l.text.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        if matches.is_empty() {
            self.search = None;
            return false;
        }
        // First match at/after the current cursor, else wrap to the first.
        let cur = matches
            .iter()
            .position(|&i| i >= self.diff_cursor)
            .unwrap_or(0);
        self.diff_cursor = matches[cur];
        self.search = Some(SearchState {
            query,
            matches,
            cur,
        });
        true
    }

    /// Move to the next (+1) / previous (-1) match with wraparound. No-op if inactive.
    pub fn search_next(&mut self, delta: isize) {
        if let Some(s) = self.search.as_mut() {
            let len = s.matches.len() as isize;
            if len == 0 {
                return;
            }
            s.cur = ((s.cur as isize + delta) % len + len) as usize % len as usize;
            self.diff_cursor = s.matches[s.cur];
        }
    }

    pub fn inc_context(&mut self) {
        if self.full_file {
            return;
        }
        if self.context_lines >= MAX_CONTEXT_LINES {
            self.full_file = true;
        } else {
            self.context_lines = (self.context_lines + 5).min(MAX_CONTEXT_LINES);
        }
    }

    pub fn dec_context(&mut self) {
        if self.full_file {
            self.full_file = false;
            return;
        }
        self.context_lines = self.context_lines.saturating_sub(5);
    }

    // ── Comment input methods ──────────────────────────────────────────────

    /// Return the diff line currently under the cursor, if any.
    pub fn current_diff_line(&self) -> Option<&DiffLine> {
        self.diff.get(self.diff_cursor)
    }

    /// Scan backwards from `diff_cursor` to find the nearest preceding Hunk
    /// line's text. Returns empty string if none found.
    pub fn current_hunk_header(&self) -> String {
        let end = self.diff_cursor.min(self.diff.len().saturating_sub(1));
        for i in (0..=end).rev() {
            if self.diff[i].kind == LineKind::Hunk {
                return self.diff[i].text.clone();
            }
        }
        String::new()
    }

    /// Open the comment modal for the current diff line.
    /// Only activates when Diff focused, a file is selected, and the current
    /// line has a new_lineno and is not itself a Hunk header.
    pub fn start_comment(&mut self) {
        if self.focus != Pane::Diff {
            return;
        }
        let Some(file) = self.selected_path().cloned() else {
            self.status_msg = Some("comment: no file selected".to_string());
            return;
        };
        let Some(dl) = self.diff.get(self.diff_cursor) else {
            self.status_msg = Some("comment: place cursor on a line".to_string());
            return;
        };
        if dl.kind == LineKind::Hunk {
            self.status_msg = Some("comment: place cursor on a line".to_string());
            return;
        }
        let Some(line_no) = dl.new_lineno else {
            self.status_msg = Some("comment: place cursor on a line".to_string());
            return;
        };
        let hunk = self.current_hunk_header();
        let existing = self
            .comments
            .get(&file, line_no)
            .map(|c| c.text.clone())
            .unwrap_or_default();
        // FIX 4: capture the anchor at the time the modal is opened, not at Ctrl-S time.
        let (anchor_line_text, anchor_before, anchor_after) = self.comment_anchor();
        // If debugging and stopped, offer the current stack/locals for attaching.
        // Prefer a snapshot already on the existing comment so re-editing keeps it.
        let existing_snap = self
            .comments
            .get(&file, line_no)
            .and_then(|c| c.debug_snapshot.clone());
        let live_snap = self.current_debug_snapshot();
        let debug_snapshot = existing_snap.or(live_snap);
        let attach_debug = debug_snapshot.is_some();
        self.input = Some(InputState {
            buffer: existing,
            target_file: file,
            target_line: line_no,
            target_hunk: hunk,
            anchor_line_text,
            anchor_before,
            anchor_after,
            debug_snapshot,
            attach_debug,
        });
    }

    pub fn input_active(&self) -> bool {
        self.input.is_some()
    }

    /// Push a character to the input buffer.
    pub fn input_push(&mut self, ch: char) {
        if let Some(ref mut s) = self.input {
            s.buffer.push(ch);
        }
    }

    /// Remove the last character from the input buffer.
    pub fn input_backspace(&mut self) {
        if let Some(ref mut s) = self.input {
            s.buffer.pop();
        }
    }

    /// Push a newline to the input buffer.
    pub fn input_newline(&mut self) {
        if let Some(ref mut s) = self.input {
            s.buffer.push('\n');
        }
    }

    /// Cancel the input modal without saving.
    pub fn input_cancel(&mut self) {
        self.input = None;
    }

    /// Finalise the input: takes the InputState, clears `self.input`,
    /// and returns a `CommittedComment` so the caller can decide whether to set or remove it.
    /// The anchor fields come from the InputState (captured at `start_comment` time, Fix 4).
    pub fn input_commit(&mut self) -> Option<CommittedComment> {
        let s = self.input.take()?;
        let debug_snapshot = if s.attach_debug { s.debug_snapshot } else { None };
        Some(CommittedComment {
            file: s.target_file,
            line: s.target_line,
            hunk: s.target_hunk,
            text: s.buffer,
            line_text: s.anchor_line_text,
            context_before: s.anchor_before,
            context_after: s.anchor_after,
            debug_snapshot,
        })
    }

    /// Toggle whether the captured debug snapshot will be attached on save.
    /// No-op when there's no snapshot to attach.
    pub fn input_toggle_attach_debug(&mut self) {
        if let Some(s) = self.input.as_mut() {
            if s.debug_snapshot.is_some() {
                s.attach_debug = !s.attach_debug;
            }
        }
    }

    /// Build the anchor (line_text, context_before, context_after) for the current cursor line.
    /// Returns trimmed text of the cursor line plus up to 2 non-Hunk lines before and after.
    /// FIX 2: if the cursor line's trimmed text is empty, returns "\u{0}" (NUL blank-line marker)
    /// instead of "" (which is the legacy "no anchor" sentinel).
    pub fn comment_anchor(&self) -> (String, Vec<String>, Vec<String>) {
        let raw_trimmed = self
            .diff
            .get(self.diff_cursor)
            .map(|l| l.text.trim().to_string())
            .unwrap_or_default();
        // Use NUL sentinel for blank lines so relocate can distinguish them from legacy no-anchor.
        let line_text = if raw_trimmed.is_empty() {
            "\u{0}".to_string()
        } else {
            raw_trimmed
        };

        let cursor = self.diff_cursor.min(self.diff.len());
        let before: Vec<String> = self.diff[..cursor]
            .iter()
            .rev()
            .filter(|l| l.kind != LineKind::Hunk)
            .take(2)
            .map(|l| l.text.trim().to_string())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        let after: Vec<String> = self
            .diff
            .get(self.diff_cursor + 1..)
            .unwrap_or(&[])
            .iter()
            .filter(|l| l.kind != LineKind::Hunk)
            .take(2)
            .map(|l| l.text.trim().to_string())
            .collect();

        (line_text, before, after)
    }

    /// Return the Comment for `line` if one exists, for the currently selected file.
    pub fn comment_for<'a>(&'a self, line: &DiffLine) -> Option<&'a crate::comments::Comment> {
        let n = line.new_lineno?;
        let file = self.selected_path()?;
        self.comments.get(file, n)
    }

    /// Whether `line` has a comment attached.
    pub fn has_comment(&self, line: &DiffLine) -> bool {
        if let Some(n) = line.new_lineno {
            if let Some(file) = self.selected_path() {
                return self.comments.get(file, n).is_some();
            }
        }
        false
    }

    /// Return the comment text for `line`, if any.
    pub fn comment_text_for<'a>(&'a self, line: &DiffLine) -> Option<&'a str> {
        let n = line.new_lineno?;
        let file = self.selected_path()?;
        self.comments.get(file, n).map(|c| c.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Build an App with files in the unstaged list only (mirrors the old `App::new(files, root)` pattern).
    fn sample() -> App {
        let files = vec![
            FileChange {
                path: PathBuf::from("a.rs"),
                status: Status::Modified,
            },
            FileChange {
                path: PathBuf::from("b.rs"),
                status: Status::Added,
            },
            FileChange {
                path: PathBuf::from("c.rs"),
                status: Status::Deleted,
            },
        ];
        App::new(files, vec![], PathBuf::from("/repo"))
    }

    #[test]
    fn selection_moves_and_clamps() {
        let mut app = sample();
        // rows: Header(Unstaged) + a.rs + b.rs + c.rs + Header(Staged) = 5 rows
        // selected starts at 0 (Header row), but move_selection starts at 0
        assert_eq!(app.selected, 0);
        app.move_selection(1);
        assert_eq!(app.selected, 1);
        app.move_selection(-5);
        assert_eq!(app.selected, 0); // clamp low
        app.move_selection(99);
        assert_eq!(app.selected, 4); // clamp high to last row (Header(Staged))
    }

    #[test]
    fn focus_toggle() {
        let mut app = sample();
        assert_eq!(app.focus, Pane::Files);
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Diff);
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Files);
    }

    #[test]
    fn gg_g_on_files_pane_moves_selection() {
        let mut app = sample();
        app.focus = Pane::Files;
        app.to_bottom();
        // 5 rows total; last index = 4
        assert_eq!(app.selected, 4);
        app.to_top();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn set_diff_resets_cursor() {
        let mut app = sample();
        app.set_diff(vec![DiffLine::context("x", 1, 1); 5]);
        app.focus = Pane::Diff;
        app.move_diff_cursor(3);
        assert_eq!(app.diff_cursor, 3);
        app.set_diff(vec![DiffLine::context("y", 1, 1); 2]);
        assert_eq!(app.diff_cursor, 0);
    }

    #[test]
    fn diff_cursor_moves_and_clamps() {
        let mut app = sample();
        app.set_diff(vec![DiffLine::context("x", 1, 1); 5]);
        app.focus = Pane::Diff;
        app.move_diff_cursor(-3);
        assert_eq!(app.diff_cursor, 0);
        app.move_diff_cursor(100);
        assert_eq!(app.diff_cursor, 4);
        app.to_top();
        assert_eq!(app.diff_cursor, 0);
        app.to_bottom();
        assert_eq!(app.diff_cursor, 4);
    }

    #[test]
    fn move_diff_cursor_skips_hunk_headers() {
        let mut app = sample();
        app.focus = Pane::Diff;
        // ctx(0) hunk(1) add(2) hunk(3) ctx(4)
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Context,
                text: "c0".into(),
                old_lineno: Some(1),
                new_lineno: Some(1),
            },
            DiffLine {
                kind: LineKind::Hunk,
                text: "@@ a @@".into(),
                old_lineno: None,
                new_lineno: None,
            },
            DiffLine {
                kind: LineKind::Add,
                text: "a2".into(),
                old_lineno: None,
                new_lineno: Some(2),
            },
            DiffLine {
                kind: LineKind::Hunk,
                text: "@@ b @@".into(),
                old_lineno: None,
                new_lineno: None,
            },
            DiffLine {
                kind: LineKind::Context,
                text: "c4".into(),
                old_lineno: Some(5),
                new_lineno: Some(5),
            },
        ]);
        app.diff_cursor = 0;
        // Down: 0 -> skip hunk(1) -> land add(2)
        app.move_diff_cursor(1);
        assert_eq!(app.diff_cursor, 2);
        // Down again: 2 -> skip hunk(3) -> land ctx(4)
        app.move_diff_cursor(1);
        assert_eq!(app.diff_cursor, 4);
        // Up: 4 -> skip hunk(3) -> land add(2)
        app.move_diff_cursor(-1);
        assert_eq!(app.diff_cursor, 2);
    }

    #[test]
    fn move_diff_cursor_split_skips_del_lines() {
        let mut app = sample();
        app.focus = Pane::Diff;
        app.split_diff = true;
        // ctx(0) del(1) add(2) ctx(3)
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Context,
                text: "c0".into(),
                old_lineno: Some(1),
                new_lineno: Some(1),
            },
            DiffLine {
                kind: LineKind::Del,
                text: "old".into(),
                old_lineno: Some(2),
                new_lineno: None,
            },
            DiffLine {
                kind: LineKind::Add,
                text: "new".into(),
                old_lineno: None,
                new_lineno: Some(2),
            },
            DiffLine {
                kind: LineKind::Context,
                text: "c3".into(),
                old_lineno: Some(3),
                new_lineno: Some(3),
            },
        ]);
        app.diff_cursor = 0;
        // Down: skip del(1) -> land add(2)
        app.move_diff_cursor(1);
        assert_eq!(app.diff_cursor, 2);
        // Down: ctx(3)
        app.move_diff_cursor(1);
        assert_eq!(app.diff_cursor, 3);
        // Up from ctx(3): skip del(1) -> add(2)
        app.move_diff_cursor(-1);
        assert_eq!(app.diff_cursor, 2);
    }

    #[test]
    fn move_diff_cursor_unified_does_not_skip_del() {
        let mut app = sample();
        app.focus = Pane::Diff;
        app.split_diff = false; // unified
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Context,
                text: "c0".into(),
                old_lineno: Some(1),
                new_lineno: Some(1),
            },
            DiffLine {
                kind: LineKind::Del,
                text: "old".into(),
                old_lineno: Some(2),
                new_lineno: None,
            },
        ]);
        app.diff_cursor = 0;
        // Unified: del IS a valid cursor target.
        app.move_diff_cursor(1);
        assert_eq!(app.diff_cursor, 1);
    }

    #[test]
    fn move_diff_cursor_all_hunks_does_not_hang() {
        let mut app = sample();
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Hunk,
                text: "@@ a @@".into(),
                old_lineno: None,
                new_lineno: None,
            },
            DiffLine {
                kind: LineKind::Hunk,
                text: "@@ b @@".into(),
                old_lineno: None,
                new_lineno: None,
            },
        ]);
        app.diff_cursor = 0;
        // No non-hunk line exists; must terminate (lands somewhere in range).
        app.move_diff_cursor(1);
        assert!(app.diff_cursor <= 1);
    }

    fn three_line_diff() -> Vec<DiffLine> {
        vec![
            DiffLine {
                kind: LineKind::Hunk,
                text: "@@ a @@".into(),
                old_lineno: None,
                new_lineno: None,
            },
            DiffLine {
                kind: LineKind::Add,
                text: "alpha".into(),
                old_lineno: None,
                new_lineno: Some(1),
            },
            DiffLine {
                kind: LineKind::Add,
                text: "beta".into(),
                old_lineno: None,
                new_lineno: Some(2),
            },
        ]
    }

    #[test]
    fn select_range_single_when_no_anchor() {
        let mut app = sample();
        app.set_diff(three_line_diff());
        app.diff_cursor = 2;
        assert!(!app.select_active());
        assert_eq!(app.select_range(), (2, 2));
    }

    #[test]
    fn select_range_orders_anchor_and_cursor() {
        let mut app = sample();
        app.set_diff(three_line_diff());
        // Anchor below cursor: anchor=2, cursor=1 -> (1, 2)
        app.diff_cursor = 2;
        app.start_select();
        app.diff_cursor = 1;
        assert!(app.select_active());
        assert_eq!(app.select_range(), (1, 2));
    }

    #[test]
    fn selection_text_joins_with_trailing_newline() {
        let mut app = sample();
        app.set_diff(three_line_diff());
        app.diff_cursor = 1;
        app.start_select();
        app.diff_cursor = 2;
        assert_eq!(app.selection_text(), "alpha\nbeta\n");
    }

    #[test]
    fn selection_text_skips_hunk_lines() {
        let mut app = sample();
        app.set_diff(three_line_diff());
        // Range covers hunk(0)..add(2); hunk text must be omitted.
        app.diff_cursor = 0;
        app.start_select();
        app.diff_cursor = 2;
        assert_eq!(app.selection_text(), "alpha\nbeta\n");
    }

    #[test]
    fn cancel_select_clears_anchor() {
        let mut app = sample();
        app.set_diff(three_line_diff());
        app.start_select();
        assert!(app.select_active());
        app.cancel_select();
        assert!(!app.select_active());
        assert_eq!(app.select_range(), (app.diff_cursor, app.diff_cursor));
    }

    #[test]
    fn toggle_reviewed_tracks_selected_file() {
        let mut app = sample();
        // selected=0 is Header row; move to row 1 (a.rs)
        app.selected = 1;
        assert!(!app.is_reviewed(0));
        app.toggle_reviewed();
        assert!(app.is_reviewed(0));
        app.toggle_reviewed();
        assert!(!app.is_reviewed(0));
    }

    #[test]
    fn hscroll_clamps_at_zero_and_resets_on_set_diff() {
        let mut app = sample();
        app.scroll_h(-5);
        assert_eq!(app.diff_hscroll, 0); // clamp low
        app.scroll_h(3);
        assert_eq!(app.diff_hscroll, 3);
        app.set_diff(vec![DiffLine::context("x", 1, 1); 2]);
        assert_eq!(app.diff_hscroll, 0); // reset on new diff
    }

    // --- NEW TREE-BASED TESTS ---

    #[test]
    fn flat_files_selected_path_resolves_via_rows() {
        // With the two-section model, row 0 is Header(Unstaged), row 1 is a.rs
        let mut app = sample();
        app.selected = 1; // a.rs row
        assert_eq!(app.selected_path(), Some(&PathBuf::from("a.rs")));
    }

    #[test]
    fn selected_file_index_returns_correct_index() {
        let mut app = sample();
        // row 2 is b.rs (index 1 in unstaged)
        app.selected = 2;
        assert_eq!(app.selected_file_index(), Some(1));
    }

    #[test]
    fn move_selection_clamps_against_rows_length() {
        let mut app = sample();
        // 3 flat files in unstaged: rows = Header(U) + a + b + c + Header(S) = 5
        app.move_selection(99);
        assert_eq!(app.selected, 4); // clamped to last row
    }

    #[test]
    fn toggle_collapse_hides_and_shows_dir_children() {
        // Build app with src/main.rs, src/ui.rs, README.md in unstaged
        let files = vec![
            FileChange {
                path: PathBuf::from("src/main.rs"),
                status: Status::Modified,
            },
            FileChange {
                path: PathBuf::from("src/ui.rs"),
                status: Status::Modified,
            },
            FileChange {
                path: PathBuf::from("README.md"),
                status: Status::Modified,
            },
        ];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // rows: Header(U), Dir "src" (1), main.rs (2), ui.rs (3), README.md (4), Header(S) (5)
        assert_eq!(app.rows.len(), 6);

        // Select the Dir row (index 1) and collapse it
        app.selected = 1;
        app.toggle_collapse();
        // Now rows should be: Header(U), Dir "src" (collapsed), README.md, Header(S) → 4 rows
        assert_eq!(app.rows.len(), 4);
        assert!(matches!(
            app.rows[1].kind,
            crate::tree::RowKind::Dir {
                collapsed: true,
                ..
            }
        ));

        // Toggle again → expand back to 6 rows
        app.selected = 1;
        app.toggle_collapse();
        assert_eq!(app.rows.len(), 6);
    }

    #[test]
    fn toggle_collapse_on_file_row_does_nothing() {
        let mut app = sample(); // flat files → all File rows (plus 2 Headers)
        let initial_len = app.rows.len();
        app.selected = 1; // a.rs File row
        app.toggle_collapse();
        assert_eq!(app.rows.len(), initial_len);
    }

    #[test]
    fn fold_all_collapses_then_expands_every_dir() {
        let files = vec![
            FileChange {
                path: PathBuf::from("src/a/x.rs"),
                status: Status::Modified,
            },
            FileChange {
                path: PathBuf::from("src/b/y.rs"),
                status: Status::Modified,
            },
            FileChange {
                path: PathBuf::from("top.rs"),
                status: Status::Modified,
            },
        ];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // Fully expanded baseline: Header(U) + src + a + x.rs + b + y.rs + top.rs + Header(S) = 8
        assert_eq!(app.rows.len(), 8);

        // First press: collapse all. Nested dirs hidden; only top-level src + top.rs show.
        app.toggle_fold_all();
        // Header(U) + src(collapsed) + top.rs + Header(S) = 4
        assert_eq!(app.rows.len(), 4);
        // src, src/a, src/b all collapsed
        assert!(app
            .collapsed
            .contains(&(Section::Unstaged, PathBuf::from("src"))));
        assert!(app
            .collapsed
            .contains(&(Section::Unstaged, PathBuf::from("src/a"))));
        assert!(app
            .collapsed
            .contains(&(Section::Unstaged, PathBuf::from("src/b"))));

        // Second press: expand all → back to 8 rows.
        app.toggle_fold_all();
        assert_eq!(app.rows.len(), 8);
        assert!(app.collapsed.is_empty());
    }

    #[test]
    fn fold_all_with_partial_collapse_collapses_remaining() {
        let files = vec![
            FileChange {
                path: PathBuf::from("src/a/x.rs"),
                status: Status::Modified,
            },
            FileChange {
                path: PathBuf::from("lib/b.rs"),
                status: Status::Modified,
            },
        ];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // Collapse only "lib" manually; "src"/"src/a" still expanded → fold-all should collapse all.
        app.collapsed
            .insert((Section::Unstaged, PathBuf::from("lib")));
        app.toggle_fold_all();
        assert!(app
            .collapsed
            .contains(&(Section::Unstaged, PathBuf::from("src"))));
        assert!(app
            .collapsed
            .contains(&(Section::Unstaged, PathBuf::from("src/a"))));
        assert!(app
            .collapsed
            .contains(&(Section::Unstaged, PathBuf::from("lib"))));
    }

    #[test]
    fn fold_all_no_dirs_is_noop() {
        let mut app = sample(); // flat files, no dirs
        let before = app.collapsed.len();
        app.toggle_fold_all();
        assert_eq!(app.collapsed.len(), before);
    }

    #[test]
    fn selected_path_returns_none_for_dir_row() {
        let files = vec![FileChange {
            path: PathBuf::from("src/main.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // rows: Header(U)(0), Dir "src" (1), File "main.rs" (2), Header(S) (3)
        app.selected = 1; // Dir row
        assert_eq!(app.selected_path(), None);
    }

    #[test]
    fn rebuild_rows_called_after_app_new() {
        let app = sample();
        // rows should be pre-built in App::new
        assert!(!app.rows.is_empty());
        // 3 flat files → Header(U) + 3 files + Header(S) = 5 rows
        assert_eq!(app.rows.len(), 5);
    }

    #[test]
    fn to_bottom_uses_rows_length() {
        let mut app = sample();
        app.focus = Pane::Files;
        app.to_bottom();
        assert_eq!(app.selected, app.rows.len() - 1);
    }

    #[test]
    fn hide_reviewed_hides_reviewed_file() {
        let files = vec![
            FileChange {
                path: PathBuf::from("a.rs"),
                status: Status::Modified,
            },
            FileChange {
                path: PathBuf::from("b.rs"),
                status: Status::Added,
            },
        ];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // Select a.rs (row 1) and review it
        app.selected = 1;
        app.toggle_reviewed();
        assert!(app.is_reviewed(0));
        // before hiding: Header(U) + a.rs + b.rs + Header(S) = 4 rows
        assert_eq!(app.rows.len(), 4);
        app.toggle_hide_reviewed();
        // a.rs hidden -> Header(U) + b.rs + Header(S) = 3 rows
        assert_eq!(app.rows.len(), 3);
        app.toggle_hide_reviewed();
        assert_eq!(app.rows.len(), 4); // shown again
    }

    #[test]
    fn rebuild_preserves_selection_identity() {
        let files = vec![
            FileChange {
                path: PathBuf::from("a.rs"),
                status: Status::Modified,
            },
            FileChange {
                path: PathBuf::from("b.rs"),
                status: Status::Added,
            },
            FileChange {
                path: PathBuf::from("c.rs"),
                status: Status::Deleted,
            },
        ];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // row 3 is c.rs (index 2 in unstaged); rows: H(U)(0) a(1) b(2) c(3) H(S)(4)
        app.selected = 3;
        app.rebuild_rows();
        // c.rs still selected (identity preserved)
        assert_eq!(app.selected_path().unwrap(), &PathBuf::from("c.rs"));
    }

    #[test]
    fn toggle_reviewed_on_dir_row_does_nothing() {
        let files = vec![FileChange {
            path: PathBuf::from("src/main.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // rows: Header(U)(0), Dir "src"(1), File "main.rs"(2), Header(S)(3)
        app.selected = 1; // Dir "src" row
        app.toggle_reviewed();
        // reviewed set should remain empty
        assert!(app.reviewed.is_empty());
    }

    #[test]
    fn reviewing_in_hide_mode_drops_file_and_keeps_valid_selection() {
        let files = vec![
            FileChange {
                path: PathBuf::from("a.rs"),
                status: Status::Modified,
            },
            FileChange {
                path: PathBuf::from("b.rs"),
                status: Status::Added,
            },
        ];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.toggle_hide_reviewed(); // hide-mode on, nothing reviewed yet
                                    // rows: Header(U)(0) + a.rs(1) + b.rs(2) + Header(S)(3) = 4 rows
        assert_eq!(app.rows.len(), 4);
        // select a.rs and review it
        app.selected = 1;
        app.toggle_reviewed();
        // rows: Header(U)(0) + b.rs(1) + Header(S)(2) = 3 rows
        assert_eq!(app.rows.len(), 3);
        // selection must still be valid; after clamping, row 1 = b.rs
        assert_eq!(app.selected_path().unwrap(), &PathBuf::from("b.rs"));
    }

    #[test]
    fn selected_section_returns_correct_section() {
        let unstaged = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let staged = vec![FileChange {
            path: PathBuf::from("b.rs"),
            status: Status::Added,
        }];
        let mut app = App::new(unstaged, staged, PathBuf::from("/repo"));
        // rows: Header(U)(0), a.rs(1), Header(S)(2), b.rs(3)
        app.selected = 1;
        assert_eq!(app.selected_section(), Some(Section::Unstaged));
        app.selected = 3;
        assert_eq!(app.selected_section(), Some(Section::Staged));
        app.selected = 0; // Header row
        assert_eq!(app.selected_section(), None);
    }

    #[test]
    fn effective_context_returns_context_lines_or_max() {
        let mut app = sample();
        // default: full_file is false -> effective_context == context_lines
        assert_eq!(app.effective_context(), app.context_lines);
        // toggle full_file on -> effective_context == u32::MAX
        app.toggle_full_file();
        assert_eq!(app.effective_context(), u32::MAX);
        // toggle back off -> effective_context == context_lines again
        app.toggle_full_file();
        assert_eq!(app.effective_context(), app.context_lines);
    }

    #[test]
    fn context_lines_inc_dec_clamp() {
        let mut app = sample();
        assert_eq!(app.context_lines, 3);
        // step is 5
        app.inc_context();
        assert_eq!(app.context_lines, 8);
        app.dec_context();
        assert_eq!(app.context_lines, 3);
        // inc clamps at MAX_CONTEXT_LINES then + switches to full file
        for _ in 0..60 {
            app.inc_context();
        }
        assert_eq!(app.context_lines, MAX_CONTEXT_LINES);
        assert!(app.full_file);
        // dec from full file restores hunks-only at max context
        app.dec_context();
        assert!(!app.full_file);
        assert_eq!(app.context_lines, MAX_CONTEXT_LINES);
        // dec clamps at 0
        for _ in 0..60 {
            app.dec_context();
        }
        assert_eq!(app.context_lines, 0);
        // one more dec stays at 0
        app.dec_context();
        assert_eq!(app.context_lines, 0);
    }

    #[test]
    fn inc_context_at_max_context_enables_full_file() {
        let mut app = sample();
        app.context_lines = MAX_CONTEXT_LINES;
        app.inc_context();
        assert!(app.full_file);
        assert_eq!(app.context_lines, MAX_CONTEXT_LINES);
        app.inc_context(); // no-op while full file
        assert!(app.full_file);
    }

    #[test]
    fn dec_context_from_full_file_via_f_keeps_context_lines() {
        let mut app = sample();
        app.context_lines = 8;
        app.toggle_full_file();
        app.dec_context();
        assert!(!app.full_file);
        assert_eq!(app.context_lines, 8);
    }

    #[test]
    fn file_pane_resize_clamps() {
        let mut app = sample();
        assert_eq!(app.file_pane_pct, 25);
        // widen to clamp at 60
        for _ in 0..20 {
            app.widen_files();
        }
        assert_eq!(app.file_pane_pct, 60);
        // narrow to clamp at 10
        for _ in 0..20 {
            app.narrow_files();
        }
        assert_eq!(app.file_pane_pct, 10);
    }

    #[test]
    fn toggle_files_sets_focus() {
        let mut app = sample();
        assert_eq!(app.show_files, true);
        assert_eq!(app.focus, Pane::Files);
        // toggle off while focused on Files -> show_files false, focus moves to Diff
        app.toggle_files();
        assert_eq!(app.show_files, false);
        assert_eq!(app.focus, Pane::Diff);
        // toggle on -> show_files true, but focus stays on Diff (no auto-steal back to Files)
        app.toggle_files();
        assert_eq!(app.show_files, true);
        assert_eq!(app.focus, Pane::Diff);
    }

    // ── Comment input tests ───────────────────────────────────────────────

    /// Build an App focused on Diff with a hunk + add line diff loaded.
    fn app_with_add_diff() -> App {
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1; // select a.rs row
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Hunk,
                text: "@@ -1,4 +1,8 @@".into(),
                old_lineno: None,
                new_lineno: None,
            },
            DiffLine {
                kind: LineKind::Add,
                text: "let x = 1;".into(),
                old_lineno: None,
                new_lineno: Some(2),
            },
        ]);
        app.diff_cursor = 1; // cursor on the Add line
        app
    }

    #[test]
    fn start_comment_sets_input_with_correct_target() {
        let mut app = app_with_add_diff();
        app.start_comment();
        assert!(app.input.is_some());
        let input = app.input.as_ref().unwrap();
        assert_eq!(input.target_file, PathBuf::from("a.rs"));
        assert_eq!(input.target_line, 2);
        assert_eq!(input.target_hunk, "@@ -1,4 +1,8 @@");
        assert_eq!(input.buffer, "");
    }

    #[test]
    fn start_comment_prefills_buffer_with_existing_comment() {
        let mut app = app_with_add_diff();
        // pre-load a comment for a.rs line 2
        app.comments.set(
            PathBuf::from("a.rs"),
            2,
            "@@ -1,4 +1,8 @@".to_string(),
            "existing note".to_string(),
            "let x = 1;".to_string(),
            vec![],
            vec![],
            0,
        );
        app.start_comment();
        let input = app.input.as_ref().unwrap();
        assert_eq!(input.buffer, "existing note");
    }

    #[test]
    fn start_comment_does_nothing_on_hunk_line() {
        let mut app = app_with_add_diff();
        app.diff_cursor = 0; // cursor on Hunk line
        app.start_comment();
        assert!(app.input.is_none());
    }

    #[test]
    fn start_comment_does_nothing_when_not_diff_focused() {
        let mut app = app_with_add_diff();
        app.focus = Pane::Files;
        app.start_comment();
        assert!(app.input.is_none());
    }

    #[test]
    fn input_push_backspace_newline_mutate_buffer() {
        let mut app = app_with_add_diff();
        app.start_comment();
        app.input_push('h');
        app.input_push('i');
        assert_eq!(app.input.as_ref().unwrap().buffer, "hi");
        app.input_newline();
        assert_eq!(app.input.as_ref().unwrap().buffer, "hi\n");
        app.input_push('!');
        assert_eq!(app.input.as_ref().unwrap().buffer, "hi\n!");
        app.input_backspace();
        assert_eq!(app.input.as_ref().unwrap().buffer, "hi\n");
        app.input_backspace();
        assert_eq!(app.input.as_ref().unwrap().buffer, "hi");
    }

    #[test]
    fn input_commit_returns_committed_comment_and_clears_input() {
        let mut app = app_with_add_diff();
        app.start_comment();
        app.input_push('g');
        app.input_push('o');
        let result = app.input_commit();
        assert!(result.is_some());
        let committed = result.unwrap();
        assert_eq!(committed.file, PathBuf::from("a.rs"));
        assert_eq!(committed.line, 2);
        assert_eq!(committed.hunk, "@@ -1,4 +1,8 @@");
        assert_eq!(committed.text, "go");
        // Anchor from the diff: cursor on "let x = 1;" at new_lineno 2
        assert_eq!(committed.line_text, "let x = 1;");
        // input cleared
        assert!(app.input.is_none());
    }

    #[test]
    fn current_hunk_header_finds_preceding_hunk() {
        let mut app = app_with_add_diff();
        app.diff_cursor = 1; // past the Hunk line at index 0
        let hunk = app.current_hunk_header();
        assert_eq!(hunk, "@@ -1,4 +1,8 @@");
    }

    #[test]
    fn current_hunk_header_returns_empty_when_no_hunk() {
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.focus = Pane::Diff;
        app.selected = 1;
        app.set_diff(vec![DiffLine {
            kind: LineKind::Context,
            text: "ctx".into(),
            old_lineno: Some(1),
            new_lineno: Some(1),
        }]);
        app.diff_cursor = 0;
        assert_eq!(app.current_hunk_header(), "");
    }

    #[test]
    fn comment_anchor_captures_line_text_and_context() {
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.focus = Pane::Diff;
        app.selected = 1;
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Hunk,
                text: "@@ -1 +1 @@".into(),
                old_lineno: None,
                new_lineno: None,
            },
            DiffLine {
                kind: LineKind::Context,
                text: "  let a = 1;  ".into(),
                old_lineno: Some(1),
                new_lineno: Some(1),
            },
            DiffLine {
                kind: LineKind::Add,
                text: "  fn target()  ".into(),
                old_lineno: None,
                new_lineno: Some(2),
            },
            DiffLine {
                kind: LineKind::Context,
                text: "  let b = 2;  ".into(),
                old_lineno: Some(3),
                new_lineno: Some(3),
            },
        ]);
        app.diff_cursor = 2; // cursor on the Add line

        let (line_text, before, after) = app.comment_anchor();
        assert_eq!(line_text, "fn target()"); // trimmed
                                              // context_before: last non-hunk line before index 2 = index 1 (Context)
        assert_eq!(before, vec!["let a = 1;"]);
        // context_after: next non-hunk line after index 2 = index 3 (Context)
        assert_eq!(after, vec!["let b = 2;"]);
    }

    #[test]
    fn comment_anchor_skips_hunk_lines_in_context() {
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.focus = Pane::Diff;
        app.selected = 1;
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Hunk,
                text: "@@ -1 +1 @@".into(),
                old_lineno: None,
                new_lineno: None,
            },
            DiffLine {
                kind: LineKind::Add,
                text: "first".into(),
                old_lineno: None,
                new_lineno: Some(1),
            },
            DiffLine {
                kind: LineKind::Hunk,
                text: "@@ -5 +5 @@".into(),
                old_lineno: None,
                new_lineno: None,
            },
            DiffLine {
                kind: LineKind::Add,
                text: "target".into(),
                old_lineno: None,
                new_lineno: Some(5),
            },
            DiffLine {
                kind: LineKind::Context,
                text: "after".into(),
                old_lineno: Some(6),
                new_lineno: Some(6),
            },
        ]);
        app.diff_cursor = 3; // cursor on "target" Add line

        let (line_text, before, after) = app.comment_anchor();
        assert_eq!(line_text, "target");
        // Hunk line at index 2 is skipped; next non-hunk before is "first" at index 1
        assert_eq!(before, vec!["first"]);
        assert_eq!(after, vec!["after"]);
    }

    // FIX 4 TDD: anchor captured at start_comment time, not at commit time
    #[test]
    fn start_comment_captures_anchor_in_input_state() {
        // Build an app with context lines so comment_anchor() returns real data
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Context,
                text: "  let a = 1;  ".into(),
                old_lineno: Some(1),
                new_lineno: Some(1),
            },
            DiffLine {
                kind: LineKind::Add,
                text: "  fn target()  ".into(),
                old_lineno: None,
                new_lineno: Some(2),
            },
            DiffLine {
                kind: LineKind::Context,
                text: "  let b = 2;  ".into(),
                old_lineno: Some(3),
                new_lineno: Some(3),
            },
        ]);
        app.diff_cursor = 1; // cursor on Add line "fn target()"
        app.start_comment();

        let input = app.input.as_ref().expect("input should be active");
        // Anchor captured at start_comment time
        assert_eq!(input.anchor_line_text, "fn target()");
        assert_eq!(input.anchor_before, vec!["let a = 1;"]);
        assert_eq!(input.anchor_after, vec!["let b = 2;"]);
    }

    // ── Help overlay tests ───────────────────────────────────────────────────

    #[test]
    fn toggle_help_flips_show_help() {
        let mut app = sample();
        assert!(!app.show_help);
        app.toggle_help();
        assert!(app.show_help);
        app.toggle_help();
        assert!(!app.show_help);
    }

    #[test]
    fn toggle_theme_flips_dark_light() {
        let mut app = sample();
        assert_eq!(app.theme, crate::theme::Theme::Dark);
        app.toggle_theme();
        assert_eq!(app.theme, crate::theme::Theme::Light);
        app.toggle_theme();
        assert_eq!(app.theme, crate::theme::Theme::Dark);
    }

    #[test]
    fn toggle_split_flips_flag() {
        let mut app = sample();
        assert!(!app.split_diff);
        app.toggle_split();
        assert!(app.split_diff);
        app.toggle_split();
        assert!(!app.split_diff);
    }

    #[test]
    fn palette_returns_matching_palette_for_theme() {
        let mut app = sample();
        // Dark theme
        let dark_pal = app.palette();
        assert_eq!(
            dark_pal.accent,
            crate::theme::Palette::for_theme(crate::theme::Theme::Dark).accent
        );
        // Switch to light
        app.toggle_theme();
        let light_pal = app.palette();
        assert_eq!(
            light_pal.accent,
            crate::theme::Palette::for_theme(crate::theme::Theme::Light).accent
        );
        assert_ne!(dark_pal.accent, light_pal.accent);
    }

    // ── NEW: ViewMode / commit list state tests ──────────────────────────────

    fn make_commit_info(summary: &str) -> crate::git::CommitInfo {
        crate::git::CommitInfo {
            id: "abcdef1234567890".to_string(),
            short: "abcdef12".to_string(),
            summary: summary.to_string(),
            author: "test".to_string(),
            time: "2024-01-01".to_string(),
        }
    }

    #[test]
    fn start_history_empty_commits_returns_false() {
        let mut app = sample();
        app.selected = 1; // a.rs
        let ok = app.start_history(vec![]);
        assert!(!ok);
        assert!(app.history.is_none());
    }

    #[test]
    fn start_history_sets_state_at_rev_one() {
        let mut app = sample();
        app.selected = 1; // a.rs
        let ok = app.start_history(vec![make_commit_info("c1"), make_commit_info("c2")]);
        assert!(ok);
        let h = app.history.as_ref().unwrap();
        assert_eq!(h.file, std::path::PathBuf::from("a.rs"));
        assert_eq!(h.commits.len(), 2);
        assert_eq!(h.idx, 1); // newest commit, one step back from baseline
        assert_eq!(h.baseline_scope, CommentScope::Worktree);
    }

    #[test]
    fn history_step_clamps_to_zero_and_len() {
        let mut app = sample();
        app.selected = 1;
        app.start_history(vec![make_commit_info("c1"), make_commit_info("c2")]);
        // idx starts at 1
        app.history_step(1); // older -> 2
        assert_eq!(app.history.as_ref().unwrap().idx, 2);
        app.history_step(1); // older past oldest -> stays 2
        assert_eq!(app.history.as_ref().unwrap().idx, 2);
        app.history_step(-1); // newer -> 1
        app.history_step(-1); // newer -> 0 (baseline)
        assert_eq!(app.history.as_ref().unwrap().idx, 0);
        app.history_step(-1); // never negative -> stays 0
        assert_eq!(app.history.as_ref().unwrap().idx, 0);
    }

    #[test]
    fn history_current_commit_none_at_baseline() {
        let mut app = sample();
        app.selected = 1;
        app.start_history(vec![make_commit_info("c1")]);
        assert!(app.history_current_commit().is_some()); // idx 1
        app.history_step(-1); // idx 0
        assert!(app.history_current_commit().is_none());
    }

    #[test]
    fn exit_history_restores_scope_and_clears() {
        let mut app = sample();
        app.selected = 1;
        app.comment_scope = CommentScope::Worktree;
        app.start_history(vec![make_commit_info("c1")]);
        // Simulate the main loop swapping scope into the revision's commit.
        app.comment_scope = CommentScope::Commit("abcdef1234567890".into());
        app.exit_history();
        assert!(app.history.is_none());
        assert_eq!(app.comment_scope, CommentScope::Worktree);
    }

    fn app_with_search_diff() -> App {
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Context,
                text: "fn alpha() {".into(),
                old_lineno: Some(1),
                new_lineno: Some(1),
            },
            DiffLine {
                kind: LineKind::Add,
                text: "let Beta = 2;".into(),
                old_lineno: None,
                new_lineno: Some(2),
            },
            DiffLine {
                kind: LineKind::Context,
                text: "let gamma = 3;".into(),
                old_lineno: Some(3),
                new_lineno: Some(3),
            },
            DiffLine {
                kind: LineKind::Add,
                text: "return beta;".into(),
                old_lineno: None,
                new_lineno: Some(4),
            },
        ]);
        app
    }

    #[test]
    fn search_commit_finds_case_insensitive_matches() {
        let mut app = app_with_search_diff();
        app.search_start();
        for ch in "beta".chars() {
            app.search_input_push(ch);
        }
        let ok = app.search_commit();
        assert!(ok);
        let s = app.search.as_ref().unwrap();
        // "let Beta = 2;" (idx 1) and "return beta;" (idx 3)
        assert_eq!(s.matches, vec![1, 3]);
        // cursor jumped to first match at/after current cursor (cursor was 0) -> idx 1
        assert_eq!(app.diff_cursor, 1);
    }

    #[test]
    fn search_commit_no_match_returns_false() {
        let mut app = app_with_search_diff();
        app.search_start();
        for ch in "zzz".chars() {
            app.search_input_push(ch);
        }
        let ok = app.search_commit();
        assert!(!ok);
        assert!(app.search.is_none());
    }

    #[test]
    fn search_next_wraps_forward_and_back() {
        let mut app = app_with_search_diff();
        app.search_start();
        for ch in "beta".chars() {
            app.search_input_push(ch);
        }
        app.search_commit(); // cursor at idx 1 (cur=0)
        app.search_next(1); // -> idx 3 (cur=1)
        assert_eq!(app.diff_cursor, 3);
        app.search_next(1); // wrap -> idx 1 (cur=0)
        assert_eq!(app.diff_cursor, 1);
        app.search_next(-1); // wrap back -> idx 3 (cur=1)
        assert_eq!(app.diff_cursor, 3);
    }

    #[test]
    fn set_diff_clears_active_search() {
        let mut app = app_with_search_diff();
        app.search_start();
        for ch in "beta".chars() {
            app.search_input_push(ch);
        }
        app.search_commit();
        assert!(app.search.is_some());
        app.set_diff(vec![DiffLine::context("x", 1, 1)]);
        assert!(app.search.is_none());
        assert!(app.search_input.is_none());
    }

    #[test]
    fn search_input_push_backspace_cancel() {
        let mut app = app_with_search_diff();
        app.search_start();
        app.search_input_push('a');
        app.search_input_push('b');
        assert_eq!(app.search_input.as_deref(), Some("ab"));
        app.search_input_backspace();
        assert_eq!(app.search_input.as_deref(), Some("a"));
        app.search_input_cancel();
        assert!(app.search_input.is_none());
        assert!(app.search.is_none());
    }

    #[test]
    fn view_mode_toggles_with_next_and_prev_view() {
        let mut app = sample();
        assert_eq!(app.view, ViewMode::Changes);
        app.next_view();
        assert_eq!(app.view, ViewMode::Commits);
        app.next_view();
        assert_eq!(app.view, ViewMode::Changes);
        app.prev_view();
        assert_eq!(app.view, ViewMode::Commits);
        app.prev_view();
        assert_eq!(app.view, ViewMode::Changes);
    }

    #[test]
    fn move_commit_selection_clamps() {
        let mut app = sample();
        app.commits = vec![
            make_commit_info("first"),
            make_commit_info("second"),
            make_commit_info("third"),
        ];
        // start at 0
        assert_eq!(app.selected_commit, 0);
        app.move_commit_selection(1);
        assert_eq!(app.selected_commit, 1);
        app.move_commit_selection(99);
        assert_eq!(app.selected_commit, 2); // clamped to max index
        app.move_commit_selection(-99);
        assert_eq!(app.selected_commit, 0); // clamped to 0
    }

    #[test]
    fn input_commit_includes_anchor_fields() {
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Context,
                text: "before_line".into(),
                old_lineno: Some(1),
                new_lineno: Some(1),
            },
            DiffLine {
                kind: LineKind::Add,
                text: "the_target_line".into(),
                old_lineno: None,
                new_lineno: Some(2),
            },
            DiffLine {
                kind: LineKind::Context,
                text: "after_line".into(),
                old_lineno: Some(3),
                new_lineno: Some(3),
            },
        ]);
        app.diff_cursor = 1;
        app.start_comment();
        app.input_push('n');
        app.input_push('o');
        app.input_push('t');
        app.input_push('e');
        let result = app.input_commit();
        assert!(result.is_some());
        let committed = result.unwrap();
        assert_eq!(committed.file, PathBuf::from("a.rs"));
        assert_eq!(committed.line, 2);
        assert_eq!(committed.hunk, ""); // no hunk header in this diff
        assert_eq!(committed.text, "note");
        assert_eq!(committed.line_text, "the_target_line");
        assert_eq!(committed.context_before, vec!["before_line"]);
        assert_eq!(committed.context_after, vec!["after_line"]);
        assert!(app.input.is_none());
    }

    // ── Part 2 TDD: commit-detail open/close/section_files ─────────────────

    fn make_commit_files(paths: &[&str]) -> Vec<FileChange> {
        paths
            .iter()
            .map(|p| FileChange {
                path: PathBuf::from(p),
                status: Status::Modified,
            })
            .collect()
    }

    #[test]
    fn open_commit_sets_state_and_builds_commit_rows() {
        let mut app = sample();
        app.view = ViewMode::Commits;
        // Give it a commit info so open_commit_short can resolve.
        app.commits = vec![crate::git::CommitInfo {
            id: "aaaa1111bbbb2222".to_string(),
            short: "aaaa1111".to_string(),
            summary: "test commit".to_string(),
            author: "tester".to_string(),
            time: "2024-01-01".to_string(),
        }];
        let files = make_commit_files(&["src/main.rs", "lib.rs"]);
        app.open_commit("aaaa1111bbbb2222".to_string(), files);

        // open_commit field set, commit_files populated
        assert_eq!(app.open_commit, Some("aaaa1111bbbb2222".to_string()));
        assert_eq!(app.commit_files.len(), 2);
        // selected reset
        assert_eq!(app.selected, 0);
        // in_commit_detail() is true
        assert!(app.in_commit_detail());
        // rows should have: Header(Commit) + Dir(src) + File(main.rs) + File(lib.rs)
        // = 1 header + 1 dir + 1 file under dir + 1 flat file = 4 rows
        assert!(
            app.rows.len() >= 3,
            "expected at least 3 rows (header + files), got {}",
            app.rows.len()
        );
        // First row is a Commit header
        assert!(matches!(
            app.rows[0].kind,
            crate::tree::RowKind::Header {
                section: Section::Commit,
                ..
            }
        ));
        // At least one File row with Section::Commit
        let has_commit_file = app.rows.iter().any(|r| {
            matches!(
                &r.kind,
                crate::tree::RowKind::File {
                    section: Section::Commit,
                    ..
                }
            )
        });
        assert!(has_commit_file, "expected File rows with Section::Commit");
    }

    #[test]
    fn close_commit_clears_state_and_rebuilds() {
        let mut app = sample();
        app.view = ViewMode::Commits;
        let files = make_commit_files(&["a.rs"]);
        app.open_commit("deadbeef12345678".to_string(), files);
        assert!(app.in_commit_detail());

        app.close_commit();

        assert_eq!(app.open_commit, None);
        assert!(app.commit_files.is_empty());
        assert!(!app.in_commit_detail());
        // rows rebuilt (no open_commit means back to normal unstaged/staged rows from sample)
        // sample() has 3 unstaged files -> Header(U) + 3 files + Header(S) = 5 rows
        assert_eq!(app.rows.len(), 5);
    }

    #[test]
    fn switching_view_from_commit_detail_rebuilds_rows() {
        // Regression: leaving a commit detail via [ / ] cleared commit_files but
        // left rows pointing into them, causing an out-of-bounds panic on next access.
        let mut app = sample();
        app.view = ViewMode::Commits;
        let files = make_commit_files(&["a.rs", "b.rs"]);
        app.open_commit("deadbeef12345678".to_string(), files);
        assert!(app.in_commit_detail());

        app.next_view(); // leave Commits -> Changes; must clear AND rebuild

        assert_eq!(app.open_commit, None);
        assert!(app.commit_files.is_empty());
        // No row may reference Section::Commit anymore.
        assert!(!app.rows.iter().any(|r| matches!(
            &r.kind,
            crate::tree::RowKind::File {
                section: Section::Commit,
                ..
            } | crate::tree::RowKind::Header {
                section: Section::Commit,
                ..
            }
        )));
        // selected must be in bounds for the rebuilt rows.
        assert!(app.selected < app.rows.len().max(1));
    }

    #[test]
    fn section_files_commit_returns_commit_files() {
        let mut app = sample();
        let files = make_commit_files(&["x.rs", "y.rs", "z.rs"]);
        app.commit_files = files.clone();
        let result = app.section_files(Section::Commit);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].path, PathBuf::from("x.rs"));
        assert_eq!(result[1].path, PathBuf::from("y.rs"));
        assert_eq!(result[2].path, PathBuf::from("z.rs"));
    }

    // ── Part 2 TDD: comment pane state, Pane::Comments, comment_rows, jump ─────

    fn sample_with_comments() -> App {
        let files = vec![
            FileChange {
                path: PathBuf::from("a.rs"),
                status: Status::Modified,
            },
            FileChange {
                path: PathBuf::from("b.rs"),
                status: Status::Added,
            },
        ];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // Add comments of various statuses
        app.comments.set(
            PathBuf::from("a.rs"),
            1,
            "@@".to_string(),
            "open note".to_string(),
            "fn a()".to_string(),
            vec![],
            vec![],
            0,
        );
        // items[0] is Open by default
        app.comments.set(
            PathBuf::from("a.rs"),
            5,
            "@@".to_string(),
            "resolved note".to_string(),
            "fn b()".to_string(),
            vec![],
            vec![],
            0,
        );
        app.comments.items[1].status = crate::comments::CommentStatus::Resolved;
        app.comments.set(
            PathBuf::from("b.rs"),
            3,
            "@@".to_string(),
            "wontfix note".to_string(),
            "fn c()".to_string(),
            vec![],
            vec![],
            0,
        );
        app.comments.items[2].status = crate::comments::CommentStatus::Wontfix;
        app.comments.set(
            PathBuf::from("b.rs"),
            7,
            "@@".to_string(),
            "needs info note".to_string(),
            "fn d()".to_string(),
            vec![],
            vec![],
            0,
        );
        app.comments.items[3].status = crate::comments::CommentStatus::NeedsInfo;
        app
    }

    #[test]
    fn comment_rows_groups_by_status_open_first() {
        let app = sample_with_comments();
        let rows = app.comment_rows();
        // Expected: Header(Open,1) Item(0) Header(NeedsInfo,1) Item(3) Header(Wontfix,1) Item(2) Header(Resolved,1) Item(1)
        // Order: Open, NeedsInfo, Wontfix, Resolved
        assert!(!rows.is_empty(), "comment_rows must not be empty");
        // First row is Header(Open)
        assert!(matches!(
            rows[0],
            CommentRow::Header(crate::comments::CommentStatus::Open, 1)
        ));
        // Second row is Item pointing to index 0 (the Open comment)
        assert!(matches!(rows[1], CommentRow::Item(0)));
        // Find Header(NeedsInfo)
        let ni_pos = rows.iter().position(|r| {
            matches!(
                r,
                CommentRow::Header(crate::comments::CommentStatus::NeedsInfo, 1)
            )
        });
        assert!(ni_pos.is_some(), "NeedsInfo header must be present");
        // Find Header(Wontfix)
        let wf_pos = rows.iter().position(|r| {
            matches!(
                r,
                CommentRow::Header(crate::comments::CommentStatus::Wontfix, 1)
            )
        });
        assert!(wf_pos.is_some(), "Wontfix header must be present");
        // Find Header(Resolved)
        let res_pos = rows.iter().position(|r| {
            matches!(
                r,
                CommentRow::Header(crate::comments::CommentStatus::Resolved, 1)
            )
        });
        assert!(res_pos.is_some(), "Resolved header must be present");
        // Order: Open < NeedsInfo < Wontfix < Resolved
        let open_pos = rows
            .iter()
            .position(|r| {
                matches!(
                    r,
                    CommentRow::Header(crate::comments::CommentStatus::Open, _)
                )
            })
            .unwrap();
        assert!(
            open_pos < ni_pos.unwrap(),
            "Open must come before NeedsInfo"
        );
        assert!(
            ni_pos.unwrap() < wf_pos.unwrap(),
            "NeedsInfo must come before Wontfix"
        );
        assert!(
            wf_pos.unwrap() < res_pos.unwrap(),
            "Wontfix must come before Resolved"
        );
    }

    #[test]
    fn comment_rows_skips_empty_groups() {
        let mut app = sample();
        // Only add an Open comment — other groups should be absent
        app.comments.set(
            PathBuf::from("a.rs"),
            1,
            "@@".to_string(),
            "only open".to_string(),
            "fn a()".to_string(),
            vec![],
            vec![],
            0,
        );
        let rows = app.comment_rows();
        // No Resolved header
        let has_resolved = rows.iter().any(|r| {
            matches!(
                r,
                CommentRow::Header(crate::comments::CommentStatus::Resolved, _)
            )
        });
        assert!(
            !has_resolved,
            "Resolved header should not appear when no resolved comments"
        );
        // Total: Header(Open) + Item(0) = 2
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn move_comment_selection_clamps() {
        let mut app = sample_with_comments();
        // 4 comments, 4 groups => 8 rows (4 headers + 4 items)
        let row_count = app.comment_rows().len();
        assert_eq!(row_count, 8);
        // Move beyond end clamps
        app.move_comment_selection(100);
        assert_eq!(app.comment_selected, row_count - 1);
        // Move below 0 clamps
        app.move_comment_selection(-100);
        assert_eq!(app.comment_selected, 0);
        // Normal step
        app.move_comment_selection(1);
        assert_eq!(app.comment_selected, 1);
    }

    #[test]
    fn selected_comment_returns_none_on_header_some_on_item() {
        let mut app = sample_with_comments();
        // Row 0 is a Header
        app.comment_selected = 0;
        assert!(
            app.selected_comment().is_none(),
            "selected_comment must be None for a header row"
        );
        // Row 1 is an Item
        app.comment_selected = 1;
        assert!(
            app.selected_comment().is_some(),
            "selected_comment must be Some for an item row"
        );
    }

    #[test]
    fn select_row_for_path_finds_file_row() {
        let mut app = sample();
        // rows: Header(U)(0), a.rs(1), b.rs(2), c.rs(3), Header(S)(4)
        let found = app.select_row_for_path(Path::new("b.rs"));
        assert!(
            found,
            "select_row_for_path must return true for an existing file path"
        );
        assert_eq!(app.selected_path(), Some(&PathBuf::from("b.rs")));
    }

    #[test]
    fn select_row_for_path_returns_false_for_missing() {
        let mut app = sample();
        let found = app.select_row_for_path(Path::new("nonexistent.rs"));
        assert!(
            !found,
            "select_row_for_path must return false for a path not in rows"
        );
    }

    #[test]
    fn cursor_lineno_prefers_new_over_old() {
        let mut app = sample();
        app.set_diff(vec![DiffLine {
            kind: LineKind::Del,
            text: "gone".into(),
            old_lineno: Some(7),
            new_lineno: None,
        }]);
        app.diff_cursor = 0;
        assert_eq!(app.cursor_lineno(), Some(7));
    }

    #[test]
    fn diff_has_lineno_matches_new_or_old() {
        let mut app = sample();
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Del,
                text: "gone".into(),
                old_lineno: Some(7),
                new_lineno: None,
            },
            DiffLine {
                kind: LineKind::Add,
                text: "new".into(),
                old_lineno: None,
                new_lineno: Some(9),
            },
        ]);
        assert!(app.diff_has_lineno(7));
        assert!(app.diff_has_lineno(9));
        assert!(!app.diff_has_lineno(1));
    }

    #[test]
    fn move_cursor_to_lineno_falls_back_to_old_lineno() {
        let mut app = sample();
        app.set_diff(vec![DiffLine {
            kind: LineKind::Del,
            text: "gone".into(),
            old_lineno: Some(7),
            new_lineno: None,
        }]);
        app.move_cursor_to_lineno(7);
        assert_eq!(app.diff_cursor, 0);
    }

    #[test]
    fn move_cursor_to_line_positions_diff_cursor() {
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Context,
                text: "x".into(),
                old_lineno: Some(1),
                new_lineno: Some(1),
            },
            DiffLine {
                kind: LineKind::Context,
                text: "y".into(),
                old_lineno: Some(2),
                new_lineno: Some(2),
            },
            DiffLine {
                kind: LineKind::Add,
                text: "z".into(),
                old_lineno: None,
                new_lineno: Some(5),
            },
        ]);
        app.move_cursor_to_line(5);
        assert_eq!(
            app.diff_cursor, 2,
            "diff_cursor must point to the line with new_lineno==5"
        );
        // Non-existent line: cursor stays unchanged
        app.move_cursor_to_line(99);
        assert_eq!(
            app.diff_cursor, 0,
            "cursor should reset to 0 when line not found"
        );
    }

    #[test]
    fn toggle_focus_cycles_through_visible_panes() {
        let mut app = sample();
        // Default: show_files=true, show_comments=false
        // Cycle: Files -> Diff -> Files (Comments skipped since show_comments=false)
        assert_eq!(app.focus, Pane::Files);
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Diff);
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Files);

        // Enable comments pane
        app.show_comments = true;
        // Cycle: Files -> Diff -> Comments -> Files
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Diff);
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Comments);
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Files);
    }

    #[test]
    fn toggle_focus_only_diff_visible_stays_diff() {
        let mut app = sample();
        app.show_files = false;
        app.show_comments = false;
        app.focus = Pane::Diff;
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Diff);
    }

    #[test]
    fn toggle_comment_pane_flips_show_comments() {
        let mut app = sample();
        assert!(!app.show_comments);
        app.toggle_comment_pane();
        assert!(app.show_comments);
        app.toggle_comment_pane();
        assert!(!app.show_comments);
    }

    #[test]
    fn toggle_comment_pane_focuses_comments_when_opening() {
        let mut app = sample();
        app.focus = Pane::Files;
        app.toggle_comment_pane(); // opens
        assert!(app.show_comments);
        assert_eq!(app.focus, Pane::Comments);
    }

    #[test]
    fn toggle_comment_pane_moves_focus_when_hiding_comments() {
        let mut app = sample();
        app.show_comments = true;
        app.focus = Pane::Comments;
        // Hiding comments while focused on Comments -> focus moves to Diff
        app.toggle_comment_pane();
        assert!(!app.show_comments);
        assert_eq!(app.focus, Pane::Diff);
    }

    // ── Part 2 TDD: sort within group by updated desc ────────────────────────

    #[test]
    fn comment_rows_within_open_group_sorted_by_updated_desc() {
        let mut app = sample();
        // Add two Open comments: one with updated=500 (older), one with updated=1500 (newer)
        app.comments.set(
            PathBuf::from("a.rs"),
            1,
            "@@".to_string(),
            "older comment".to_string(),
            "fn a()".to_string(),
            vec![],
            vec![],
            500,
        );
        // items[0].updated = 500
        app.comments.set(
            PathBuf::from("a.rs"),
            2,
            "@@".to_string(),
            "newer comment".to_string(),
            "fn b()".to_string(),
            vec![],
            vec![],
            1500,
        );
        // items[1].updated = 1500
        let rows = app.comment_rows();
        // Expected: Header(Open,2), Item(?newer), Item(?older)
        // The newer-updated comment (items[1], updated=1500) should come first
        assert_eq!(rows.len(), 3, "Header + 2 items");
        assert!(matches!(
            rows[0],
            CommentRow::Header(crate::comments::CommentStatus::Open, 2)
        ));
        // First Item should be the one with updated=1500 (items index 1)
        match rows[1] {
            CommentRow::Item(idx) => {
                assert_eq!(
                    app.comments.items[idx].text, "newer comment",
                    "first item in group must be the newer-updated comment"
                );
            }
            _ => panic!("expected Item at rows[1]"),
        }
        match rows[2] {
            CommentRow::Item(idx) => {
                assert_eq!(
                    app.comments.items[idx].text, "older comment",
                    "second item in group must be the older-updated comment"
                );
            }
            _ => panic!("expected Item at rows[2]"),
        }
    }

    // ── CommentScope tests ─────────────────────────────────────────────────────

    #[test]
    fn open_commit_sets_comment_scope_to_commit() {
        let mut app = sample();
        app.view = ViewMode::Commits;
        assert_eq!(app.comment_scope, CommentScope::Worktree);

        let files = make_commit_files(&["a.rs"]);
        app.open_commit("deadbeef1234567890abcdef".to_string(), files);

        assert_eq!(
            app.comment_scope,
            CommentScope::Commit("deadbeef1234567890abcdef".to_string())
        );
    }

    #[test]
    fn close_commit_resets_comment_scope_to_worktree() {
        let mut app = sample();
        app.view = ViewMode::Commits;
        let files = make_commit_files(&["a.rs"]);
        app.open_commit("deadbeef1234567890abcdef".to_string(), files);
        assert_eq!(
            app.comment_scope,
            CommentScope::Commit("deadbeef1234567890abcdef".to_string())
        );

        app.close_commit();

        assert_eq!(app.comment_scope, CommentScope::Worktree);
    }

    #[test]
    fn next_view_from_commits_resets_scope_to_worktree() {
        let mut app = sample();
        app.view = ViewMode::Commits;
        let files = make_commit_files(&["a.rs"]);
        app.open_commit("aabbccdd11223344".to_string(), files);
        assert_eq!(
            app.comment_scope,
            CommentScope::Commit("aabbccdd11223344".to_string())
        );

        // next_view from Commits -> Changes clears open_commit and resets scope
        app.next_view();

        assert_eq!(app.view, ViewMode::Changes);
        assert_eq!(app.comment_scope, CommentScope::Worktree);
    }

    #[test]
    fn scope_label_worktree() {
        let app = sample();
        assert_eq!(app.scope_label(), "worktree");
    }

    #[test]
    fn scope_label_commit() {
        let mut app = sample();
        app.view = ViewMode::Commits;
        let files = make_commit_files(&["a.rs"]);
        app.open_commit("abc123def456".to_string(), files);
        assert_eq!(app.scope_label(), "commit:abc123def456");
    }

    // ─── Debugger: breakpoint toggle + panel ─────────────────────────────────

    /// App with file `a.rs` selected and a diff whose cursor sits on new line 2.
    fn app_on_diff_line() -> App {
        let mut app = sample();
        app.selected = 1; // a.rs (row 0 is the Unstaged header)
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Add,
                text: "let x = 1;".into(),
                old_lineno: None,
                new_lineno: Some(2),
            },
            DiffLine::context("ctx", 3, 3),
        ]);
        app.diff_cursor = 0; // on new line 2
        app
    }

    #[test]
    fn toggle_breakpoint_adds_then_removes() {
        let mut app = app_on_diff_line();
        let abs = PathBuf::from("/repo").join("a.rs");
        assert!(!app.has_breakpoint(&abs, 2));

        // First toggle sets it.
        assert!(app.toggle_breakpoint_at_cursor());
        assert!(app.has_breakpoint(&abs, 2));
        assert!(app.debug_active());

        // Second toggle clears it; with no sessions/bps left, debug state drops.
        assert!(!app.toggle_breakpoint_at_cursor());
        assert!(!app.has_breakpoint(&abs, 2));
        assert!(!app.debug_active());
    }

    #[test]
    fn toggle_breakpoint_noop_without_source_line() {
        let mut app = sample();
        app.focus = Pane::Diff;
        // No file selected / no diff → no source loc.
        assert!(!app.toggle_breakpoint_at_cursor());
        assert!(!app.debug_active());
    }

    #[test]
    fn debug_panel_selection_clamps_to_rows() {
        let mut app = app_on_diff_line();
        let mut st = DebugState::default();
        let mut sess = DebugSession::new(1, "worktree".into());
        sess.stack = vec![crate::dap::Frame {
            name: "main".into(),
            file: None,
            line: 0,
            id: 0,
            locals: vec![],
        }];
        sess.locals = vec![
            crate::dap::VarRow {
                name: "x".into(),
                value: "1".into(),
                ty: None,
                var_ref: 0,
                memory_ref: None,
            },
            crate::dap::VarRow {
                name: "y".into(),
                value: "2".into(),
                ty: None,
                var_ref: 0,
                memory_ref: None,
            },
        ];
        // Three frames; selection is over frames (locals nest under the selected).
        let frame = |n: &str| crate::dap::Frame {
            name: n.into(),
            file: None,
            line: 0,
            id: 0,
            locals: vec![],
        };
        sess.stack = vec![frame("a"), frame("b"), frame("c")];
        st.sessions.push(sess);
        app.debug = Some(st);

        assert_eq!(app.debug_panel_len(), 3); // 3 frames
        app.focus = Pane::Comments;
        app.right_tab = RightTab::Debug;
        app.move_debug_panel_selection(99);
        assert_eq!(app.debug.as_ref().unwrap().panel_sel, 2); // clamped to last frame
        // Frame selection mirrors panel selection.
        assert_eq!(app.debug.as_ref().unwrap().sessions[0].frame_sel, 2);
        app.move_debug_panel_selection(-99);
        assert_eq!(app.debug.as_ref().unwrap().panel_sel, 0);
    }

    #[test]
    fn attach_snapshot_creates_comment_with_stack() {
        let mut app = app_on_diff_line();
        let snap = crate::dap::DebugSnapshot {
            session_label: "worktree".into(),
            stopped_file: "/repo/a.rs".into(), // abs; repo_root is /repo
            stopped_line: 2,
            stack: vec![crate::dap::Frame {
                name: "main".into(),
                file: Some("/repo/a.rs".into()),
                line: 2,
                id: 0,
                locals: vec![],
            }],
            locals: vec![],
            captured: 1,
        };
        app.attach_debug_snapshot(snap);
        let c = app
            .comments
            .items
            .iter()
            .find(|c| c.file == PathBuf::from("a.rs") && c.line == 2)
            .expect("comment created at stopped line");
        assert!(c.debug_snapshot.is_some());
        assert_eq!(c.debug_snapshot.as_ref().unwrap().stack[0].name, "main");
    }

    #[test]
    fn breakpoint_list_toggle_and_delete() {
        let mut app = app_on_diff_line();
        app.toggle_breakpoint_at_cursor(); // /repo/a.rs:2, enabled
        let abs = PathBuf::from("/repo").join("a.rs");
        assert!(app.breakpoint_enabled(&abs, 2));
        assert_eq!(app.breakpoint_count(), 1);

        // Select it and disable.
        let on = app.toggle_selected_breakpoint();
        assert_eq!(on, Some(false));
        assert!(app.has_breakpoint(&abs, 2)); // still present
        assert!(!app.breakpoint_enabled(&abs, 2)); // but disabled

        // Re-enable.
        assert_eq!(app.toggle_selected_breakpoint(), Some(true));
        assert!(app.breakpoint_enabled(&abs, 2));

        // Delete removes it entirely.
        assert!(app.delete_selected_breakpoint());
        assert_eq!(app.breakpoint_count(), 0);
        assert!(!app.has_breakpoint(&abs, 2));
    }

    #[test]
    fn debug_tab_toggles() {
        let mut app = app_on_diff_line();
        app.toggle_breakpoint_at_cursor();
        assert!(!app.debug_tab_is_breakpoints());
        app.toggle_debug_tab();
        assert!(app.debug_tab_is_breakpoints());
        app.toggle_debug_tab();
        assert!(!app.debug_tab_is_breakpoints());
    }

    #[test]
    fn exit_debug_keeps_breakpoints_drops_sessions() {
        let mut app = app_on_diff_line();
        app.toggle_breakpoint_at_cursor(); // a breakpoint exists
        let mut st = app.debug.take().unwrap();
        st.sessions.push(DebugSession::new(1, "worktree".into()));
        app.debug = Some(st);
        app.focus = Pane::Comments;
        app.right_tab = RightTab::Debug;

        app.exit_debug();
        // Sessions gone, breakpoints kept, debug overlay still present, the
        // right tab fell back to Comments and focus moved off the Debug tab.
        let d = app.debug.as_ref().unwrap();
        assert!(d.sessions.is_empty());
        assert!(!d.breakpoints.is_empty());
        assert_eq!(app.right_tab, RightTab::Comments);
        assert!(!app.is_debug_focused());
    }

    #[test]
    fn right_pane_joins_focus_cycle_when_active() {
        let mut app = sample();
        app.show_comments = false; // right pane hidden unless debugging
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Diff);
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Files); // no right pane in cycle yet

        // Activate debug → the right pane (Debug tab) joins the cycle.
        app.debug = Some(DebugState::default());
        app.right_tab = RightTab::Debug;
        app.focus = Pane::Diff;
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Comments);
        assert!(app.is_debug_focused());
    }

    #[test]
    fn toggle_right_tab_only_to_debug_when_active() {
        let mut app = sample();
        assert_eq!(app.right_tab, RightTab::Comments);
        app.toggle_right_tab(); // no debug → stays on Comments
        assert_eq!(app.right_tab, RightTab::Comments);
        app.debug = Some(DebugState::default());
        app.toggle_right_tab();
        assert_eq!(app.right_tab, RightTab::Debug);
        app.toggle_right_tab();
        assert_eq!(app.right_tab, RightTab::Comments);
    }
}
