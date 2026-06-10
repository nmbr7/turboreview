use std::collections::HashSet;
use std::path::PathBuf;

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
    pub reviewed: HashSet<PathBuf>,
    pub status_msg: Option<String>,
}

impl App {
    pub fn new(files: Vec<FileChange>, repo_root: PathBuf) -> Self {
        App {
            repo_root,
            mode: Mode::Unstaged,
            focus: Pane::Files,
            files,
            selected: 0,
            diff: Vec::new(),
            diff_scroll: 0,
            reviewed: HashSet::new(),
            status_msg: None,
        }
    }

    pub fn selected_path(&self) -> Option<&PathBuf> {
        self.files.get(self.selected).map(|f| &f.path)
    }

    pub fn set_diff(&mut self, diff: Vec<DiffLine>) {
        self.diff = diff;
        self.diff_scroll = 0;
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.files.is_empty() {
            return;
        }
        let max = self.files.len() as isize - 1;
        let next = (self.selected as isize + delta).clamp(0, max);
        self.selected = next as usize;
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
            Pane::Files => self.selected = self.files.len().saturating_sub(1),
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
}
