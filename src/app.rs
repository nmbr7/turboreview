use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::tree::{Row, RowKind};

const MAX_HSCROLL: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Unstaged,
    Staged,
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

pub struct App {
    pub repo_root: PathBuf,
    pub focus: Pane,
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
        };
        app.rebuild_rows();
        app
    }

    pub fn toggle_full_file(&mut self) {
        self.full_file = !self.full_file;
    }

    pub fn toggle_files(&mut self) {
        self.show_files = !self.show_files;
        // Can't navigate a hidden file list — keep focus on the diff while hidden.
        if !self.show_files {
            self.focus = Pane::Diff;
        } else {
            self.focus = Pane::Files;
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
        self.rows = crate::tree::build_rows(&self.unstaged, &self.staged, &self.collapsed, hidden);
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
        }
    }

    pub fn to_bottom(&mut self) {
        match self.focus {
            Pane::Files => self.selected = self.rows.len().saturating_sub(1),
            Pane::Diff => self.diff_cursor = self.diff.len().saturating_sub(1),
        }
    }

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Pane::Files => Pane::Diff,
            Pane::Diff => Pane::Files,
        };
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

    pub fn inc_context(&mut self) {
        self.context_lines = (self.context_lines + 5).min(50);
    }

    pub fn dec_context(&mut self) {
        self.context_lines = self.context_lines.saturating_sub(5);
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
    fn diff_cursor_clamps_and_gg_g() {
        let mut app = sample();
        app.set_diff(vec![DiffLine::context("x", 1, 1); 5]);
        app.focus = Pane::Diff;
        app.move_diff_cursor(-3);
        assert_eq!(app.diff_cursor, 0);
        app.move_diff_cursor(100);
        assert_eq!(app.diff_cursor, 4); // last index
        app.to_top();
        assert_eq!(app.diff_cursor, 0);
        app.to_bottom();
        assert_eq!(app.diff_cursor, 4);
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
        // toggle off: show_files false + focus Diff
        app.toggle_files();
        assert_eq!(app.show_files, false);
        assert_eq!(app.focus, Pane::Diff);
        // toggle on: show_files true + focus Files
        app.toggle_files();
        assert_eq!(app.show_files, true);
        assert_eq!(app.focus, Pane::Files);
    }
}
