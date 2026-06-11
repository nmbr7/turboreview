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
    Comments,
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
        DiffLine { kind: LineKind::Context, text: text.into(), old_lineno: Some(old), new_lineno: Some(new) }
    }
}

/// State for the modal comment input box.
pub struct InputState {
    pub buffer: String,        // current text (may contain \n for multi-line)
    pub target_file: PathBuf,
    pub target_line: u32,
    pub target_hunk: String,
    /// Anchor captured at the moment the modal was opened (Fix 4: don't re-derive at Ctrl-S).
    pub anchor_line_text: String,
    pub anchor_before: Vec<String>,
    pub anchor_after: Vec<String>,
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
    pub open_commit: Option<String>,
    pub commit_files: Vec<FileChange>,
    pub show_help: bool,
    pub comment_scope: CommentScope,
    pub show_comments: bool,
    pub comment_selected: usize,
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
            open_commit: None,
            commit_files: Vec::new(),
            show_help: false,
            comment_scope: CommentScope::Worktree,
            show_comments: false,
            comment_selected: 0,
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
        if self.full_file { u32::MAX } else { self.context_lines }
    }

    pub fn rebuild_rows(&mut self) {
        let prev = self.selected_identity();
        let empty = HashSet::new();
        let hidden = if self.hide_reviewed { &self.reviewed } else { &empty };
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
            RowKind::File { section, file_index } => {
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
            (RowKind::File { section: rs, file_index }, RowId::File(s, p)) if rs == s => {
                self.section_files(*rs).get(*file_index).map_or(false, |f| &f.path == p)
            }
            (RowKind::Dir { section: rs, path, .. }, RowId::Dir(s, p)) if rs == s => path == p,
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
            Some(Row { kind: RowKind::File { section, file_index }, .. }) => {
                self.section_files(*section).get(*file_index).map(|f| &f.path)
            }
            _ => None,
        }
    }

    pub fn selected_section(&self) -> Option<Section> {
        match self.rows.get(self.selected) {
            Some(Row { kind: RowKind::File { section, .. }, .. }) => Some(*section),
            _ => None,
        }
    }

    pub fn selected_file_index(&self) -> Option<usize> {
        match self.rows.get(self.selected) {
            Some(Row { kind: RowKind::File { file_index, .. }, .. }) => Some(*file_index),
            _ => None,
        }
    }

    pub fn set_diff(&mut self, diff: Vec<DiffLine>) {
        self.diff = diff;
        self.diff_cursor = 0;
        self.diff_hscroll = 0;
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
        self.diff_cursor = (self.diff_cursor as isize + delta).clamp(0, max) as usize;
    }

    pub fn to_top(&mut self) {
        match self.focus {
            Pane::Files => self.selected = 0,
            Pane::Diff => self.diff_cursor = 0,
            Pane::Comments => self.comment_selected = 0,
        }
    }

    pub fn to_bottom(&mut self) {
        match self.focus {
            Pane::Files => self.selected = self.rows.len().saturating_sub(1),
            Pane::Diff => self.diff_cursor = self.diff.len().saturating_sub(1),
            Pane::Comments => {
                let len = self.comment_rows().len();
                self.comment_selected = len.saturating_sub(1);
            }
        }
    }

    /// Cycle focus through the VISIBLE panes in order: Files -> Diff -> Comments -> Files.
    /// Skips Files when !show_files, skips Comments when !show_comments.
    pub fn toggle_focus(&mut self) {
        let panes: Vec<Pane> = [Pane::Files, Pane::Diff, Pane::Comments]
            .iter()
            .copied()
            .filter(|&p| match p {
                Pane::Files => self.show_files,
                Pane::Diff => true,
                Pane::Comments => self.show_comments,
            })
            .collect();
        if panes.len() <= 1 {
            return; // nothing to cycle
        }
        let current = panes.iter().position(|&p| p == self.focus).unwrap_or(0);
        self.focus = panes[(current + 1) % panes.len()];
    }

    /// Toggle the comment pane. If hiding while Comments has focus, move focus to Diff.
    pub fn toggle_comment_pane(&mut self) {
        self.show_comments = !self.show_comments;
        if !self.show_comments && self.focus == Pane::Comments {
            self.focus = Pane::Diff;
        }
    }

    /// Build the displayable rows for the comment-list pane.
    /// Groups items by status in order: Open, NeedsInfo, Wontfix, Resolved.
    /// Each non-empty group gets a Header(status, count) followed by Item(i) for each match.
    pub fn comment_rows(&self) -> Vec<CommentRow> {
        use crate::comments::CommentStatus;
        let order = [CommentStatus::Open, CommentStatus::NeedsInfo, CommentStatus::Wontfix, CommentStatus::Resolved];
        let mut rows = Vec::new();
        for status in &order {
            let indices: Vec<usize> = self.comments.items.iter().enumerate()
                .filter(|(_, c)| &c.status == status)
                .map(|(i, _)| i)
                .collect();
            if !indices.is_empty() {
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
            if let RowKind::File { section, file_index } = &row.kind {
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

    /// Scan self.diff for the first line whose new_lineno == new_lineno; set diff_cursor to it.
    /// If not found, reset diff_cursor to 0.
    pub fn move_cursor_to_line(&mut self, new_lineno: u32) {
        for (i, dl) in self.diff.iter().enumerate() {
            if dl.new_lineno == Some(new_lineno) {
                self.diff_cursor = i;
                return;
            }
        }
        self.diff_cursor = 0;
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
        self.commits.iter().find(|c| c.id == id).map(|c| c.short.as_str())
    }

    pub fn inc_context(&mut self) {
        self.context_lines = (self.context_lines + 5).min(50);
    }

    pub fn dec_context(&mut self) {
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
        let existing = self.comments.get(&file, line_no).map(|c| c.text.clone()).unwrap_or_default();
        // FIX 4: capture the anchor at the time the modal is opened, not at Ctrl-S time.
        let (anchor_line_text, anchor_before, anchor_after) = self.comment_anchor();
        self.input = Some(InputState {
            buffer: existing,
            target_file: file,
            target_line: line_no,
            target_hunk: hunk,
            anchor_line_text,
            anchor_before,
            anchor_after,
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
        Some(CommittedComment {
            file: s.target_file,
            line: s.target_line,
            hunk: s.target_hunk,
            text: s.buffer,
            line_text: s.anchor_line_text,
            context_before: s.anchor_before,
            context_after: s.anchor_after,
        })
    }

    /// Build the anchor (line_text, context_before, context_after) for the current cursor line.
    /// Returns trimmed text of the cursor line plus up to 2 non-Hunk lines before and after.
    /// FIX 2: if the cursor line's trimmed text is empty, returns "\u{0}" (NUL blank-line marker)
    /// instead of "" (which is the legacy "no anchor" sentinel).
    pub fn comment_anchor(&self) -> (String, Vec<String>, Vec<String>) {
        let raw_trimmed = self.diff.get(self.diff_cursor)
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

        let after: Vec<String> = self.diff.get(self.diff_cursor + 1..)
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
            FileChange { path: PathBuf::from("a.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("b.rs"), status: Status::Added },
            FileChange { path: PathBuf::from("c.rs"), status: Status::Deleted },
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
            FileChange { path: PathBuf::from("src/main.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("src/ui.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("README.md"), status: Status::Modified },
        ];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // rows: Header(U), Dir "src" (1), main.rs (2), ui.rs (3), README.md (4), Header(S) (5)
        assert_eq!(app.rows.len(), 6);

        // Select the Dir row (index 1) and collapse it
        app.selected = 1;
        app.toggle_collapse();
        // Now rows should be: Header(U), Dir "src" (collapsed), README.md, Header(S) → 4 rows
        assert_eq!(app.rows.len(), 4);
        assert!(matches!(app.rows[1].kind, crate::tree::RowKind::Dir { collapsed: true, .. }));

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
    fn selected_path_returns_none_for_dir_row() {
        let files = vec![
            FileChange { path: PathBuf::from("src/main.rs"), status: Status::Modified },
        ];
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
            FileChange { path: PathBuf::from("a.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("b.rs"), status: Status::Added },
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
            FileChange { path: PathBuf::from("a.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("b.rs"), status: Status::Added },
            FileChange { path: PathBuf::from("c.rs"), status: Status::Deleted },
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
        let files = vec![
            FileChange { path: PathBuf::from("src/main.rs"), status: Status::Modified },
        ];
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
            FileChange { path: PathBuf::from("a.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("b.rs"), status: Status::Added },
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
        let unstaged = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let staged = vec![FileChange { path: PathBuf::from("b.rs"), status: Status::Added }];
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
        // inc clamps at 50
        for _ in 0..60 {
            app.inc_context();
        }
        assert_eq!(app.context_lines, 50);
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
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
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
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.focus = Pane::Diff;
        app.selected = 1;
        app.set_diff(vec![
            DiffLine { kind: LineKind::Context, text: "ctx".into(), old_lineno: Some(1), new_lineno: Some(1) },
        ]);
        app.diff_cursor = 0;
        assert_eq!(app.current_hunk_header(), "");
    }

    #[test]
    fn comment_anchor_captures_line_text_and_context() {
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.focus = Pane::Diff;
        app.selected = 1;
        app.set_diff(vec![
            DiffLine { kind: LineKind::Hunk, text: "@@ -1 +1 @@".into(), old_lineno: None, new_lineno: None },
            DiffLine { kind: LineKind::Context, text: "  let a = 1;  ".into(), old_lineno: Some(1), new_lineno: Some(1) },
            DiffLine { kind: LineKind::Add, text: "  fn target()  ".into(), old_lineno: None, new_lineno: Some(2) },
            DiffLine { kind: LineKind::Context, text: "  let b = 2;  ".into(), old_lineno: Some(3), new_lineno: Some(3) },
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
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.focus = Pane::Diff;
        app.selected = 1;
        app.set_diff(vec![
            DiffLine { kind: LineKind::Hunk, text: "@@ -1 +1 @@".into(), old_lineno: None, new_lineno: None },
            DiffLine { kind: LineKind::Add, text: "first".into(), old_lineno: None, new_lineno: Some(1) },
            DiffLine { kind: LineKind::Hunk, text: "@@ -5 +5 @@".into(), old_lineno: None, new_lineno: None },
            DiffLine { kind: LineKind::Add, text: "target".into(), old_lineno: None, new_lineno: Some(5) },
            DiffLine { kind: LineKind::Context, text: "after".into(), old_lineno: Some(6), new_lineno: Some(6) },
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
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine { kind: LineKind::Context, text: "  let a = 1;  ".into(), old_lineno: Some(1), new_lineno: Some(1) },
            DiffLine { kind: LineKind::Add, text: "  fn target()  ".into(), old_lineno: None, new_lineno: Some(2) },
            DiffLine { kind: LineKind::Context, text: "  let b = 2;  ".into(), old_lineno: Some(3), new_lineno: Some(3) },
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
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine { kind: LineKind::Context, text: "before_line".into(), old_lineno: Some(1), new_lineno: Some(1) },
            DiffLine { kind: LineKind::Add, text: "the_target_line".into(), old_lineno: None, new_lineno: Some(2) },
            DiffLine { kind: LineKind::Context, text: "after_line".into(), old_lineno: Some(3), new_lineno: Some(3) },
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
        paths.iter().map(|p| FileChange { path: PathBuf::from(p), status: Status::Modified }).collect()
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
        assert!(app.rows.len() >= 3, "expected at least 3 rows (header + files), got {}", app.rows.len());
        // First row is a Commit header
        assert!(matches!(app.rows[0].kind, crate::tree::RowKind::Header { section: Section::Commit, .. }));
        // At least one File row with Section::Commit
        let has_commit_file = app.rows.iter().any(|r| matches!(&r.kind, crate::tree::RowKind::File { section: Section::Commit, .. }));
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
            crate::tree::RowKind::File { section: Section::Commit, .. }
                | crate::tree::RowKind::Header { section: Section::Commit, .. }
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
            FileChange { path: PathBuf::from("a.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("b.rs"), status: Status::Added },
        ];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // Add comments of various statuses
        app.comments.set(PathBuf::from("a.rs"), 1, "@@".to_string(), "open note".to_string(), "fn a()".to_string(), vec![], vec![]);
        // items[0] is Open by default
        app.comments.set(PathBuf::from("a.rs"), 5, "@@".to_string(), "resolved note".to_string(), "fn b()".to_string(), vec![], vec![]);
        app.comments.items[1].status = crate::comments::CommentStatus::Resolved;
        app.comments.set(PathBuf::from("b.rs"), 3, "@@".to_string(), "wontfix note".to_string(), "fn c()".to_string(), vec![], vec![]);
        app.comments.items[2].status = crate::comments::CommentStatus::Wontfix;
        app.comments.set(PathBuf::from("b.rs"), 7, "@@".to_string(), "needs info note".to_string(), "fn d()".to_string(), vec![], vec![]);
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
        assert!(matches!(rows[0], CommentRow::Header(crate::comments::CommentStatus::Open, 1)));
        // Second row is Item pointing to index 0 (the Open comment)
        assert!(matches!(rows[1], CommentRow::Item(0)));
        // Find Header(NeedsInfo)
        let ni_pos = rows.iter().position(|r| matches!(r, CommentRow::Header(crate::comments::CommentStatus::NeedsInfo, 1)));
        assert!(ni_pos.is_some(), "NeedsInfo header must be present");
        // Find Header(Wontfix)
        let wf_pos = rows.iter().position(|r| matches!(r, CommentRow::Header(crate::comments::CommentStatus::Wontfix, 1)));
        assert!(wf_pos.is_some(), "Wontfix header must be present");
        // Find Header(Resolved)
        let res_pos = rows.iter().position(|r| matches!(r, CommentRow::Header(crate::comments::CommentStatus::Resolved, 1)));
        assert!(res_pos.is_some(), "Resolved header must be present");
        // Order: Open < NeedsInfo < Wontfix < Resolved
        let open_pos = rows.iter().position(|r| matches!(r, CommentRow::Header(crate::comments::CommentStatus::Open, _))).unwrap();
        assert!(open_pos < ni_pos.unwrap(), "Open must come before NeedsInfo");
        assert!(ni_pos.unwrap() < wf_pos.unwrap(), "NeedsInfo must come before Wontfix");
        assert!(wf_pos.unwrap() < res_pos.unwrap(), "Wontfix must come before Resolved");
    }

    #[test]
    fn comment_rows_skips_empty_groups() {
        let mut app = sample();
        // Only add an Open comment — other groups should be absent
        app.comments.set(PathBuf::from("a.rs"), 1, "@@".to_string(), "only open".to_string(), "fn a()".to_string(), vec![], vec![]);
        let rows = app.comment_rows();
        // No Resolved header
        let has_resolved = rows.iter().any(|r| matches!(r, CommentRow::Header(crate::comments::CommentStatus::Resolved, _)));
        assert!(!has_resolved, "Resolved header should not appear when no resolved comments");
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
        assert!(app.selected_comment().is_none(), "selected_comment must be None for a header row");
        // Row 1 is an Item
        app.comment_selected = 1;
        assert!(app.selected_comment().is_some(), "selected_comment must be Some for an item row");
    }

    #[test]
    fn select_row_for_path_finds_file_row() {
        let mut app = sample();
        // rows: Header(U)(0), a.rs(1), b.rs(2), c.rs(3), Header(S)(4)
        let found = app.select_row_for_path(Path::new("b.rs"));
        assert!(found, "select_row_for_path must return true for an existing file path");
        assert_eq!(app.selected_path(), Some(&PathBuf::from("b.rs")));
    }

    #[test]
    fn select_row_for_path_returns_false_for_missing() {
        let mut app = sample();
        let found = app.select_row_for_path(Path::new("nonexistent.rs"));
        assert!(!found, "select_row_for_path must return false for a path not in rows");
    }

    #[test]
    fn move_cursor_to_line_positions_diff_cursor() {
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.set_diff(vec![
            DiffLine { kind: LineKind::Context, text: "x".into(), old_lineno: Some(1), new_lineno: Some(1) },
            DiffLine { kind: LineKind::Context, text: "y".into(), old_lineno: Some(2), new_lineno: Some(2) },
            DiffLine { kind: LineKind::Add, text: "z".into(), old_lineno: None, new_lineno: Some(5) },
        ]);
        app.move_cursor_to_line(5);
        assert_eq!(app.diff_cursor, 2, "diff_cursor must point to the line with new_lineno==5");
        // Non-existent line: cursor stays unchanged
        app.move_cursor_to_line(99);
        assert_eq!(app.diff_cursor, 0, "cursor should reset to 0 when line not found");
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
    fn toggle_comment_pane_moves_focus_when_hiding_comments() {
        let mut app = sample();
        app.show_comments = true;
        app.focus = Pane::Comments;
        // Hiding comments while focused on Comments -> focus moves to Diff
        app.toggle_comment_pane();
        assert!(!app.show_comments);
        assert_eq!(app.focus, Pane::Diff);
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
}
