use std::collections::HashSet;
use std::path::PathBuf;

use crate::tree::{self, Row, RowKind};

const MAX_HSCROLL: usize = 500;

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
    pub mode: Mode,
    pub focus: Pane,
    pub files: Vec<FileChange>,
    pub selected: usize,
    pub diff: Vec<DiffLine>,
    pub diff_scroll: usize,
    pub diff_hscroll: usize,
    pub reviewed: HashSet<PathBuf>,
    pub status_msg: Option<String>,
    pub collapsed: HashSet<PathBuf>,
    pub rows: Vec<Row>,
}

impl App {
    pub fn new(files: Vec<FileChange>, repo_root: PathBuf) -> Self {
        let mut app = App {
            repo_root,
            mode: Mode::Unstaged,
            focus: Pane::Files,
            files,
            selected: 0,
            diff: Vec::new(),
            diff_scroll: 0,
            diff_hscroll: 0,
            reviewed: HashSet::new(),
            status_msg: None,
            collapsed: HashSet::new(),
            rows: Vec::new(),
        };
        app.rebuild_rows();
        app
    }

    pub fn rebuild_rows(&mut self) {
        self.rows = tree::build_rows(&self.files, &self.collapsed);
        let max = self.rows.len().saturating_sub(1);
        if self.selected > max {
            self.selected = max;
        }
    }

    pub fn selected_path(&self) -> Option<&PathBuf> {
        match self.rows.get(self.selected) {
            Some(Row { kind: RowKind::File { file_index }, .. }) => {
                self.files.get(*file_index).map(|f| &f.path)
            }
            _ => None,
        }
    }

    pub fn selected_file_index(&self) -> Option<usize> {
        match self.rows.get(self.selected) {
            Some(Row { kind: RowKind::File { file_index }, .. }) => Some(*file_index),
            _ => None,
        }
    }

    pub fn set_diff(&mut self, diff: Vec<DiffLine>) {
        self.diff = diff;
        self.diff_scroll = 0;
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
        // cap at a sane maximum so we don't scroll into the void
        self.diff_hscroll = (next as usize).min(MAX_HSCROLL);
    }

    pub fn scroll_diff(&mut self, delta: isize) {
        if self.diff.is_empty() {
            self.diff_scroll = 0;
            return;
        }
        let max = self.diff.len() as isize - 1;
        let next = (self.diff_scroll as isize + delta).clamp(0, max);
        self.diff_scroll = next as usize;
    }

    pub fn to_top(&mut self) {
        match self.focus {
            Pane::Files => self.selected = 0,
            Pane::Diff => self.diff_scroll = 0,
        }
    }

    pub fn to_bottom(&mut self) {
        match self.focus {
            Pane::Files => self.selected = self.rows.len().saturating_sub(1),
            Pane::Diff => self.diff_scroll = self.diff.len().saturating_sub(1),
        }
    }

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Pane::Files => Pane::Diff,
            Pane::Diff => Pane::Files,
        };
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Unstaged => Mode::Staged,
            Mode::Staged => Mode::Unstaged,
        };
    }

    pub fn toggle_reviewed(&mut self) {
        if let Some(path) = self.selected_path().cloned() {
            if !self.reviewed.remove(&path) {
                self.reviewed.insert(path);
            }
        }
    }

    pub fn is_reviewed(&self, idx: usize) -> bool {
        self.files
            .get(idx)
            .map(|f| self.reviewed.contains(&f.path))
            .unwrap_or(false)
    }

    pub fn toggle_collapse(&mut self) {
        if let Some(row) = self.rows.get(self.selected) {
            if let RowKind::Dir { path, .. } = &row.kind {
                let path = path.clone();
                if self.collapsed.contains(&path) {
                    self.collapsed.remove(&path);
                } else {
                    self.collapsed.insert(path);
                }
                self.rebuild_rows();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample() -> App {
        let files = vec![
            FileChange { path: PathBuf::from("a.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("b.rs"), status: Status::Added },
            FileChange { path: PathBuf::from("c.rs"), status: Status::Deleted },
        ];
        App::new(files, PathBuf::from("/repo"))
    }

    #[test]
    fn selection_moves_and_clamps() {
        let mut app = sample();
        assert_eq!(app.selected, 0);
        app.move_selection(1);
        assert_eq!(app.selected, 1);
        app.move_selection(-5);
        assert_eq!(app.selected, 0); // clamp low
        app.move_selection(99);
        assert_eq!(app.selected, 2); // clamp high
    }

    #[test]
    fn focus_and_mode_toggle() {
        let mut app = sample();
        assert_eq!(app.focus, Pane::Files);
        app.toggle_focus();
        assert_eq!(app.focus, Pane::Diff);
        assert_eq!(app.mode, Mode::Unstaged);
        app.toggle_mode();
        assert_eq!(app.mode, Mode::Staged);
    }

    #[test]
    fn diff_scroll_clamps_and_gg_g() {
        let mut app = sample();
        app.set_diff(vec![DiffLine::context("x", 1, 1); 5]);
        app.focus = Pane::Diff;
        app.scroll_diff(-3);
        assert_eq!(app.diff_scroll, 0);
        app.scroll_diff(100);
        assert_eq!(app.diff_scroll, 4); // last index
        app.to_top();
        assert_eq!(app.diff_scroll, 0);
        app.to_bottom();
        assert_eq!(app.diff_scroll, 4);
    }

    #[test]
    fn gg_g_on_files_pane_moves_selection() {
        let mut app = sample();
        app.focus = Pane::Files;
        app.to_bottom();
        assert_eq!(app.selected, 2);
        app.to_top();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn set_diff_resets_scroll() {
        let mut app = sample();
        app.set_diff(vec![DiffLine::context("x", 1, 1); 5]);
        app.focus = Pane::Diff;
        app.scroll_diff(3);
        assert_eq!(app.diff_scroll, 3);
        app.set_diff(vec![DiffLine::context("y", 1, 1); 2]);
        assert_eq!(app.diff_scroll, 0);
    }

    #[test]
    fn toggle_reviewed_tracks_selected_file() {
        let mut app = sample();
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
        // Flat files (no dirs) produce File rows; selected_path still returns file path.
        let app = sample();
        assert_eq!(app.selected, 0);
        assert_eq!(app.selected_path(), Some(&PathBuf::from("a.rs")));
    }

    #[test]
    fn selected_file_index_returns_correct_index() {
        let mut app = sample();
        app.selected = 1;
        assert_eq!(app.selected_file_index(), Some(1));
    }

    #[test]
    fn move_selection_clamps_against_rows_length() {
        let app = sample();
        // 3 flat files → 3 rows; move_selection(99) clamps to 2
        let mut app2 = sample();
        app2.move_selection(99);
        assert_eq!(app2.selected, 2);
        let _ = app; // just to show same setup
    }

    #[test]
    fn toggle_collapse_hides_and_shows_dir_children() {
        // Build app with src/main.rs, src/ui.rs, README.md
        let files = vec![
            FileChange { path: PathBuf::from("src/main.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("src/ui.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("README.md"), status: Status::Modified },
        ];
        let mut app = App::new(files, PathBuf::from("/repo"));
        // rows: Dir "src" (0), File "main.rs" (1), File "ui.rs" (2), File "README.md" (3)
        assert_eq!(app.rows.len(), 4);

        // Select the Dir row (index 0) and collapse it
        app.selected = 0;
        app.toggle_collapse();
        // Now rows should be: Dir "src" (collapsed), File "README.md" → 2 rows
        assert_eq!(app.rows.len(), 2);
        assert!(matches!(app.rows[0].kind, crate::tree::RowKind::Dir { collapsed: true, .. }));

        // Toggle again → expand back to 4 rows
        app.selected = 0;
        app.toggle_collapse();
        assert_eq!(app.rows.len(), 4);
    }

    #[test]
    fn toggle_collapse_on_file_row_does_nothing() {
        let mut app = sample(); // flat files → all File rows
        let initial_len = app.rows.len();
        app.selected = 0;
        app.toggle_collapse();
        assert_eq!(app.rows.len(), initial_len);
    }

    #[test]
    fn selected_path_returns_none_for_dir_row() {
        let files = vec![
            FileChange { path: PathBuf::from("src/main.rs"), status: Status::Modified },
        ];
        let mut app = App::new(files, PathBuf::from("/repo"));
        // rows: Dir "src" (0), File "main.rs" (1)
        app.selected = 0; // Dir row
        assert_eq!(app.selected_path(), None);
    }

    #[test]
    fn rebuild_rows_called_after_app_new() {
        let app = sample();
        // rows should be pre-built in App::new
        assert!(!app.rows.is_empty());
        assert_eq!(app.rows.len(), 3); // 3 flat files → 3 rows
    }

    #[test]
    fn to_bottom_uses_rows_length() {
        let mut app = sample();
        app.focus = Pane::Files;
        app.to_bottom();
        assert_eq!(app.selected, app.rows.len() - 1);
    }

    #[test]
    fn toggle_reviewed_on_dir_row_does_nothing() {
        let files = vec![
            FileChange { path: PathBuf::from("src/main.rs"), status: Status::Modified },
        ];
        let mut app = App::new(files, PathBuf::from("/repo"));
        app.selected = 0; // Dir "src" row
        app.toggle_reviewed();
        // reviewed set should remain empty
        assert!(app.reviewed.is_empty());
    }
}
