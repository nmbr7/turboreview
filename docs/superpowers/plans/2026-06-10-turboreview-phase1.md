# turboreview Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A terminal TUI that shows a git repo's working-tree diff (staged or unstaged, togglable) with a left file-list pane (reviewed checkbox + selected highlight) and a right syntax-highlighted diff pane; reviewed state persists to disk.

**Architecture:** Pure-logic core (`git`, `review`, `app`) with no terminal IO, rendered by a `ui` module via ratatui and driven by an event loop in `main`. Diff text is syntax-highlighted (fg) via syntect; diff status / selection set the background (bg) — the two layers compose.

**Tech Stack:** Rust, `git2` 0.21, `ratatui` 0.30 (crossterm default backend), `syntect` + `syntect-tui`, `serde`/`serde_json`, `anyhow` (error plumbing), `tempfile` (test fixtures).

Spec: `docs/superpowers/specs/2026-06-10-turboreview-design.md`

---

## File Structure

- `Cargo.toml` — deps + bin target.
- `src/main.rs` — arg parse, terminal setup/teardown (RAII guard + panic hook), event loop.
- `src/git.rs` — `git2` wrapper: open repo, list changed files, build diff lines. Pure of terminal IO.
- `src/review.rs` — load/save/toggle reviewed-file set (`.turboreview/reviewed.json`).
- `src/app.rs` — central `App` state + pure transitions; `Mode`, `Pane` enums; `FileChange`, `DiffLine`, `LineKind` types.
- `src/highlight.rs` — syntect singletons + `highlight_code(text, ext) -> Vec<Span>` helper.
- `src/ui.rs` — render `&App` to a ratatui `Frame`.
- `tests/` — integration tests use the lib crate; unit tests live inline (`#[cfg(test)]`).

`main.rs` is the binary; the rest is a library (`src/lib.rs`) so tests can import. Set up both targets in Task 1.

---

## Task 1: Project scaffold + dependencies

**Files:**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/main.rs`

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "turboreview"
version = "0.1.0"
edition = "2021"

[lib]
name = "turboreview"
path = "src/lib.rs"

[[bin]]
name = "turboreview"
path = "src/main.rs"

[dependencies]
git2 = "0.21"
ratatui = "0.30"
syntect = "5"
syntect-tui = "3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Write `src/lib.rs`**

```rust
pub mod app;
pub mod git;
pub mod highlight;
pub mod review;
pub mod ui;
```

- [ ] **Step 3: Write a placeholder `src/main.rs`**

```rust
fn main() -> anyhow::Result<()> {
    println!("turboreview");
    Ok(())
}
```

- [ ] **Step 4: Create empty module files so `lib.rs` compiles**

Create `src/app.rs`, `src/git.rs`, `src/highlight.rs`, `src/review.rs`, `src/ui.rs` each containing only:

```rust
// implemented in later tasks
```

- [ ] **Step 5: Verify it builds**

Run: `cargo build`
Expected: compiles (warnings about empty modules OK).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/
git commit -m "chore: scaffold turboreview crate with deps"
```

---

## Task 2: `app` types and pure state transitions

**Files:**
- Modify: `src/app.rs`

These types are used by every later task. No IO here.

- [ ] **Step 1: Write the failing test**

Append to `src/app.rs`:

```rust
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
    fn toggle_reviewed_tracks_selected_file() {
        let mut app = sample();
        assert!(!app.is_reviewed(0));
        app.toggle_reviewed();
        assert!(app.is_reviewed(0));
        app.toggle_reviewed();
        assert!(!app.is_reviewed(0));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib app`
Expected: FAIL — types/methods not defined (does not compile).

- [ ] **Step 3: Write the implementation**

Prepend to `src/app.rs` (above the test module):

```rust
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
            Pane::Files => self.move_selection(isize::MAX / 2),
            Pane::Diff => self.scroll_diff(isize::MAX / 2),
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib app`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat: add App state and pure transitions"
```

---

## Task 3: `review` persistence

**Files:**
- Modify: `src/review.rs`

- [ ] **Step 1: Write the failing test**

Write `src/review.rs`:

```rust
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// Load the reviewed-file set from `<repo_root>/.turboreview/reviewed.json`.
/// Returns an empty set if the file does not exist.
pub fn load(repo_root: &Path) -> Result<HashSet<PathBuf>> {
    let file = reviewed_path(repo_root);
    if !file.exists() {
        return Ok(HashSet::new());
    }
    let bytes = std::fs::read(&file)?;
    let list: Vec<PathBuf> = serde_json::from_slice(&bytes)?;
    Ok(list.into_iter().collect())
}

/// Persist the reviewed-file set, creating the `.turboreview` dir if needed.
pub fn save(repo_root: &Path, reviewed: &HashSet<PathBuf>) -> Result<()> {
    let dir = repo_root.join(".turboreview");
    std::fs::create_dir_all(&dir)?;
    let mut list: Vec<&PathBuf> = reviewed.iter().collect();
    list.sort();
    let json = serde_json::to_vec_pretty(&list)?;
    std::fs::write(dir.join("reviewed.json"), json)?;
    Ok(())
}

fn reviewed_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".turboreview").join("reviewed.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_missing_returns_empty() {
        let dir = tempdir().unwrap();
        let set = load(dir.path()).unwrap();
        assert!(set.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempdir().unwrap();
        let mut set = HashSet::new();
        set.insert(PathBuf::from("src/a.rs"));
        set.insert(PathBuf::from("src/b.rs"));
        save(dir.path(), &set).unwrap();
        assert!(dir.path().join(".turboreview/reviewed.json").exists());
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded, set);
    }
}
```

- [ ] **Step 2: Run test to verify it fails then passes**

Run: `cargo test --lib review`
Expected: PASS (2 tests) — code and tests written together; if it fails, fix before continuing.

- [ ] **Step 3: Commit**

```bash
git add src/review.rs
git commit -m "feat: persist reviewed-file set to .turboreview/reviewed.json"
```

---

## Task 4: `git` wrapper — changed files

**Files:**
- Modify: `src/git.rs`

- [ ] **Step 1: Write the failing test**

Write `src/git.rs`:

```rust
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::{Diff, DiffOptions, Repository};

use crate::app::{DiffLine, FileChange, LineKind, Mode, Status};

pub struct Repo {
    inner: Repository,
}

impl Repo {
    /// Discover the repository containing `path` (path may be the repo root or a subdir).
    pub fn discover(path: &Path) -> Result<Repo> {
        let inner = Repository::discover(path)
            .with_context(|| format!("no git repository found at {}", path.display()))?;
        Ok(Repo { inner })
    }

    /// Absolute path to the working-tree root.
    pub fn workdir(&self) -> Result<PathBuf> {
        self.inner
            .workdir()
            .map(|p| p.to_path_buf())
            .context("repository has no working directory (bare repo)")
    }

    fn build_diff(&self, mode: Mode) -> Result<Diff<'_>> {
        let mut opts = DiffOptions::new();
        opts.include_untracked(true).recurse_untracked_dirs(true);
        let diff = match mode {
            Mode::Unstaged => self.inner.diff_index_to_workdir(None, Some(&mut opts))?,
            Mode::Staged => {
                let head_tree = match self.inner.head() {
                    Ok(head) => Some(head.peel_to_tree()?),
                    Err(_) => None, // unborn HEAD: compare empty tree -> index
                };
                self.inner
                    .diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts))?
            }
        };
        Ok(diff)
    }

    /// List changed files for the given mode.
    pub fn changed_files(&self, mode: Mode) -> Result<Vec<FileChange>> {
        let diff = self.build_diff(mode)?;
        let mut files = Vec::new();
        for delta in diff.deltas() {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_path_buf());
            if let Some(path) = path {
                files.push(FileChange { path, status: map_status(delta.status()) });
            }
        }
        Ok(files)
    }
}

fn map_status(s: git2::Delta) -> Status {
    use git2::Delta::*;
    match s {
        Added | Untracked => Status::Added,
        Deleted => Status::Deleted,
        Modified => Status::Modified,
        Renamed => Status::Renamed,
        _ => Status::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Init a repo, return (tempdir, Repo). Caller writes files.
    fn init_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "t").unwrap();
        cfg.set_str("user.email", "t@t").unwrap();
        (dir, repo)
    }

    #[test]
    fn untracked_file_shows_as_unstaged_added() {
        let (dir, _repo) = init_repo();
        fs::write(dir.path().join("new.txt"), "hello\n").unwrap();
        let repo = Repo::discover(dir.path()).unwrap();
        let files = repo.changed_files(Mode::Unstaged).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("new.txt"));
        assert_eq!(files[0].status, Status::Added);
    }

    #[test]
    fn staged_file_shows_in_staged_mode_only() {
        let (dir, repo) = init_repo();
        fs::write(dir.path().join("a.txt"), "x\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.txt")).unwrap();
        index.write().unwrap();

        let r = Repo::discover(dir.path()).unwrap();
        let staged = r.changed_files(Mode::Staged).unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].path, PathBuf::from("a.txt"));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib git`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**

```bash
git add src/git.rs
git commit -m "feat: list changed files via git2 (staged/unstaged)"
```

---

## Task 5: `git` wrapper — diff lines for a file

**Files:**
- Modify: `src/git.rs`

- [ ] **Step 1: Write the failing test**

Add inside the `tests` module in `src/git.rs`:

```rust
    #[test]
    fn diff_for_untracked_file_yields_added_lines() {
        let (dir, _repo) = init_repo();
        fs::write(dir.path().join("new.txt"), "line1\nline2\n").unwrap();
        let repo = Repo::discover(dir.path()).unwrap();
        let lines = repo.diff_for(Path::new("new.txt"), Mode::Unstaged).unwrap();
        let added: Vec<_> = lines.iter().filter(|l| l.kind == LineKind::Add).collect();
        assert_eq!(added.len(), 2);
        assert!(added.iter().any(|l| l.text.contains("line1")));
    }

    #[test]
    fn diff_for_unchanged_file_is_empty() {
        let (dir, _repo) = init_repo();
        let repo = Repo::discover(dir.path()).unwrap();
        let lines = repo.diff_for(Path::new("missing.txt"), Mode::Unstaged).unwrap();
        assert!(lines.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib git::tests::diff_for_untracked_file_yields_added_lines`
Expected: FAIL — `diff_for` not defined.

- [ ] **Step 3: Implement `diff_for`**

Add this method inside `impl Repo` in `src/git.rs`:

```rust
    /// Build the diff lines for a single file path (relative to the repo root).
    pub fn diff_for(&self, file: &Path, mode: Mode) -> Result<Vec<DiffLine>> {
        let mut opts = DiffOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .pathspec(file);
        let diff = match mode {
            Mode::Unstaged => self.inner.diff_index_to_workdir(None, Some(&mut opts))?,
            Mode::Staged => {
                let head_tree = match self.inner.head() {
                    Ok(head) => Some(head.peel_to_tree()?),
                    Err(_) => None,
                };
                self.inner
                    .diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts))?
            }
        };

        let mut lines = Vec::new();
        diff.print(git2::DiffFormat::Patch, |_delta, hunk, line| {
            // Skip file headers (origin 'F' / 'H' handled below); we only want hunk + body.
            let origin = line.origin();
            let kind = match origin {
                '+' => LineKind::Add,
                '-' => LineKind::Del,
                ' ' => LineKind::Context,
                'H' => LineKind::Hunk,
                _ => return true, // 'F' file header, binary, etc: skip
            };
            // For hunk-context lines git2 may invoke with hunk header text.
            let text = if kind == LineKind::Hunk {
                hunk.map(|h| String::from_utf8_lossy(h.header()).trim_end().to_string())
                    .unwrap_or_default()
            } else {
                String::from_utf8_lossy(line.content()).trim_end_matches('\n').to_string()
            };
            lines.push(DiffLine {
                kind,
                text,
                old_lineno: line.old_lineno(),
                new_lineno: line.new_lineno(),
            });
            true
        })?;
        Ok(lines)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib git`
Expected: PASS (4 tests total).

- [ ] **Step 5: Commit**

```bash
git add src/git.rs
git commit -m "feat: build per-file diff lines via git2 print"
```

---

## Task 6: `highlight` — syntect → ratatui spans

**Files:**
- Modify: `src/highlight.rs`

- [ ] **Step 1: Write the failing test**

Write `src/highlight.rs`:

```rust
use std::sync::OnceLock;

use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

struct Assets {
    syntaxes: SyntaxSet,
    theme: Theme,
}

fn assets() -> &'static Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let mut themes = ThemeSet::load_defaults();
        // base16-ocean.dark is bundled in syntect's default ThemeSet.
        let theme = themes
            .themes
            .remove("base16-ocean.dark")
            .expect("bundled theme present");
        Assets { syntaxes, theme }
    })
}

/// Highlight one line of code, choosing the grammar by file extension.
/// Falls back to a single plain span if the extension is unknown or highlighting fails.
pub fn highlight_code(text: &str, extension: &str) -> Vec<Span<'static>> {
    let assets = assets();
    let syntax = assets
        .syntaxes
        .find_syntax_by_extension(extension)
        .unwrap_or_else(|| assets.syntaxes.find_syntax_plain_text());
    let mut hl = HighlightLines::new(syntax, &assets.theme);
    match hl.highlight_line(text, &assets.syntaxes) {
        Ok(ranges) => ranges
            .into_iter()
            .filter_map(|seg| syntect_tui::into_span(seg).ok())
            .map(|s| Span::styled(s.content.into_owned(), s.style))
            .collect(),
        Err(_) => vec![Span::raw(text.to_string())],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_extension_produces_spans() {
        let spans = highlight_code("let x = 42;", "rs");
        assert!(!spans.is_empty());
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(joined.contains("let"));
    }

    #[test]
    fn unknown_extension_falls_back_without_panic() {
        let spans = highlight_code("anything goes", "zzz-unknown");
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(joined.contains("anything goes"));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib highlight`
Expected: PASS (2 tests). If `into_span` signature differs, adjust the `.map` line — the goal is `Vec<Span<'static>>` with owned content.

- [ ] **Step 3: Commit**

```bash
git add src/highlight.rs
git commit -m "feat: syntect-backed syntax highlighting helper"
```

---

## Task 7: `ui` — render the two panes + status bar

**Files:**
- Modify: `src/ui.rs`

- [ ] **Step 1: Write the failing test**

Write `src/ui.rs`:

```rust
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, LineKind, Mode, Pane};
use crate::highlight::highlight_code;

const SELECTED_BG: Color = Color::Rgb(60, 60, 90);
const ADD_BG: Color = Color::Rgb(20, 50, 20);
const DEL_BG: Color = Color::Rgb(55, 20, 20);

pub fn render(frame: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(outer[0]);

    render_files(frame, app, panes[0]);
    render_diff(frame, app, panes[1]);
    render_status(frame, app, outer[1]);
}

fn focused_border(app: &App, pane: Pane) -> Style {
    if app.focus == pane {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn render_files(frame: &mut Frame, app: &App, area: Rect) {
    let mode = match app.mode {
        Mode::Staged => "STAGED",
        Mode::Unstaged => "UNSTAGED",
    };
    let items: Vec<ListItem> = app
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let check = if app.is_reviewed(i) { "[x] " } else { "[ ] " };
            let mut style = Style::default();
            if i == app.selected {
                style = style.bg(SELECTED_BG).add_modifier(Modifier::BOLD);
            }
            ListItem::new(Line::from(format!("{}{}", check, f.path.display()))).style(style)
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(focused_border(app, Pane::Files))
            .title(format!(" Files [{}] ", mode)),
    );
    frame.render_widget(list, area);
}

fn render_diff(frame: &mut Frame, app: &App, area: Rect) {
    let ext = app
        .selected_path()
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();

    let title = app
        .selected_path()
        .map(|p| format!(" Diff: {} ", p.display()))
        .unwrap_or_else(|| " Diff ".to_string());

    let lines: Vec<Line> = if app.diff.is_empty() {
        vec![Line::from(Span::styled(
            "No changes",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.diff
            .iter()
            .skip(app.diff_scroll)
            .map(|dl| {
                let bg = match dl.kind {
                    LineKind::Add => Some(ADD_BG),
                    LineKind::Del => Some(DEL_BG),
                    _ => None,
                };
                if dl.kind == LineKind::Hunk {
                    return Line::from(Span::styled(
                        dl.text.clone(),
                        Style::default().fg(Color::Cyan),
                    ));
                }
                // syntax-highlighted fg spans, then overlay bg per diff kind
                let mut spans: Vec<Span> = highlight_code(&dl.text, &ext);
                if let Some(bg) = bg {
                    for s in spans.iter_mut() {
                        s.style = s.style.bg(bg);
                    }
                }
                Line::from(spans)
            })
            .collect()
    };

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(focused_border(app, Pane::Diff))
            .title(title),
    );
    frame.render_widget(para, area);
}

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let base = "Tab:focus  s:staged  Space:review  ↑↓/jk:move  gg/G  q:quit";
    let text = match &app.status_msg {
        Some(msg) => format!("{}   |   {}", base, msg),
        None => base.to_string(),
    };
    let para = Paragraph::new(text).style(Style::default().fg(Color::Gray));
    frame.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, DiffLine, FileChange, Status};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn app_with_diff() -> App {
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, PathBuf::from("/repo"));
        app.set_diff(vec![
            DiffLine { kind: LineKind::Hunk, text: "@@ -1 +1 @@".into(), old_lineno: None, new_lineno: None },
            DiffLine { kind: LineKind::Add, text: "let x = 1;".into(), old_lineno: None, new_lineno: Some(1) },
        ]);
        app
    }

    #[test]
    fn render_does_not_panic_and_shows_file() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = app_with_diff();
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let dump: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("a.rs"));
        assert!(dump.contains("[ ]"));
    }

    #[test]
    fn empty_diff_shows_placeholder() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let app = App::new(files, PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("No changes"));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib ui`
Expected: PASS (2 tests). If `frame.area()` errors on the ratatui version, use `frame.size()` — adjust to whichever the installed 0.30.x exposes.

- [ ] **Step 3: Commit**

```bash
git add src/ui.rs
git commit -m "feat: render file list + syntax-highlighted diff + status bar"
```

---

## Task 8: `main` — terminal lifecycle + event loop

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Write `src/main.rs`**

```rust
use std::io::{self, Stdout};
use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use turboreview::app::{App, Mode, Pane};
use turboreview::git::Repo;
use turboreview::{review, ui};

// ratatui re-exports crossterm; use that single copy to avoid version mismatch.
use ratatui::crossterm;

fn main() -> Result<()> {
    let repo_arg = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let repo = Repo::discover(&PathBuf::from(&repo_arg))?;
    let root = repo.workdir()?;

    let files = repo.changed_files(Mode::Unstaged)?;
    let mut app = App::new(files, root.clone());
    app.reviewed = review::load(&root)?;
    refresh_diff(&repo, &mut app);

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, &repo, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

fn refresh_diff(repo: &Repo, app: &mut App) {
    match app.selected_path() {
        Some(path) => match repo.diff_for(path, app.mode) {
            Ok(lines) => {
                app.status_msg = None;
                app.set_diff(lines);
            }
            Err(e) => {
                app.status_msg = Some(format!("diff error: {e}"));
                app.set_diff(Vec::new());
            }
        },
        None => app.set_diff(Vec::new()),
    }
}

fn reload_files(repo: &Repo, app: &mut App) {
    match repo.changed_files(app.mode) {
        Ok(files) => {
            app.files = files;
            if app.selected >= app.files.len() {
                app.selected = app.files.len().saturating_sub(1);
            }
        }
        Err(e) => app.status_msg = Some(format!("list error: {e}")),
    }
    refresh_diff(repo, app);
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    repo: &Repo,
    app: &mut App,
) -> Result<()> {
    let mut pending_g = false; // for the `gg` chord
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        // `gg` chord: first g arms, second g fires to_top.
        if matches!(key.code, KeyCode::Char('g')) && key.modifiers.is_empty() {
            if pending_g {
                app.to_top();
                pending_g = false;
            } else {
                pending_g = true;
            }
            continue;
        }
        pending_g = false;

        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) => return Ok(()),
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(()),
            (KeyCode::Tab, _) => app.toggle_focus(),
            (KeyCode::Char('s'), _) => {
                app.toggle_mode();
                reload_files(repo, app);
            }
            (KeyCode::Char(' '), _) => {
                app.toggle_reviewed();
                review::save(&app.repo_root, &app.reviewed)?;
            }
            (KeyCode::Char('G'), _) => app.to_bottom(),
            (KeyCode::Up, _) | (KeyCode::Char('k'), _) => move_in_focus(repo, app, -1),
            (KeyCode::Down, _) | (KeyCode::Char('j'), _) => move_in_focus(repo, app, 1),
            _ => {}
        }
    }
}

fn move_in_focus(repo: &Repo, app: &mut App, delta: isize) {
    match app.focus {
        Pane::Files => {
            app.move_selection(delta);
            refresh_diff(repo, app);
        }
        Pane::Diff => app.scroll_diff(delta),
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    // Restore the terminal even on panic so the user's shell isn't left broken.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        hook(info);
    }));
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

// Silence unused-import lint if KeyEvent ends up unreferenced after edits.
#[allow(unused_imports)]
use KeyEvent as _KeyEvent;
```

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: compiles. If `ratatui::crossterm` re-export path differs in the installed 0.30.x, drop that `use` line and add `crossterm = "0.28"` to `Cargo.toml` instead — but prefer the re-export to guarantee one crossterm version.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test`
Expected: all tests PASS (app 5, review 2, git 4, highlight 2, ui 2).

- [ ] **Step 4: Manual smoke test**

Run: `cargo run -- .`
Expected: TUI opens on this repo. Verify: file list left with `[ ]` checkboxes, selected row highlighted; `Tab` switches focus (border color moves); arrows move selection (Files) / scroll diff (Diff); `s` toggles `[STAGED]`/`[UNSTAGED]` title; `Space` toggles `[x]`; `gg`/`G` jump; `q` quits and the shell is intact. (If the working tree is clean, `touch scratch.txt` first to get an entry.)

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: terminal lifecycle and key-driven event loop"
```

---

## Task 9: Wire-up review + docs polish

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update `README.md`**

```markdown
# turboreview

Terminal code-review tool. Phase 1: browse a git repo's working-tree diff
(staged or unstaged), with syntax-highlighted code and per-file review checkboxes.

## Usage

```
turboreview [REPO_PATH]   # defaults to current directory
```

### Keys

| Key            | Action                                   |
|----------------|------------------------------------------|
| `Tab`          | switch focus (Files / Diff)              |
| `↑`/`↓` `j`/`k`| move selection (Files) / scroll (Diff)   |
| `gg` / `G`     | jump to top / bottom of focused pane     |
| `s`            | toggle staged / unstaged                 |
| `Space`        | toggle reviewed checkbox                  |
| `q` / `Ctrl-C` | quit                                     |

Reviewed files persist to `<repo>/.turboreview/reviewed.json`.
```

- [ ] **Step 2: Verify build + tests once more**

Run: `cargo test && cargo build`
Expected: all PASS, clean build.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document Phase 1 usage and keybindings"
```

---

## Verification (end-to-end)

1. `cargo test` — all unit tests green.
2. `cargo run -- .` in this repo (with at least one changed/untracked file) — confirm
   panes, navigation, mode toggle, syntax colors, checkbox persistence (toggle a file,
   quit, relaunch → `[x]` retained, `.turboreview/reviewed.json` present).
3. `cargo run -- /path/to/some/other/repo` — confirms the repo path arg works.
4. Run in a non-repo dir — confirm clean error message, no panic, terminal intact.

## Notes for the implementer

- API drift: `git2` 0.21, `ratatui` 0.30, `syntect-tui` 3 — a few call sites flag
  likely-variant names (`frame.area()` vs `size()`, `into_span` return shape,
  `ratatui::crossterm` re-export). Each is called out at the step. Resolve by checking
  `cargo doc --open` for the installed version; do not guess silently.
- Keep the three pure modules (`app`, `git`, `review`) free of terminal IO so their
  tests stay fast and deterministic.
- Coverage overlay (Phase 2) and DAP debug pane (Phase 3) are intentionally absent.
