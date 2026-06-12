# Diff File-History & In-Diff Search — Design

**Date:** 2026-06-12
**Status:** Approved (design)

## Goal

Two independent features for the diff pane:

1. **File-history overlay (`H`)** — browse how the currently-viewed file evolved
   across the commits that touched it, stepping older/newer through
   commit-vs-parent diffs without leaving the diff pane.
2. **In-diff search (`/`)** — find text within the currently-rendered diff,
   jumping the line cursor between matches.

The two features are orthogonal: history changes *which* diff is shown; search
operates on *whatever* diff is shown (including a history revision).

---

## Feature 1: File-history overlay

### Behaviour

- Available in **both** views (Changes working-tree diff and Commits-detail diff).
- The diff currently on screen when `H` is pressed is the **baseline** (revision 0).
  It stays addressable; you can always return to it.
- `H` (when Diff is focused and a file is selected) builds the list of commits
  that touched the selected file, walking from HEAD backward, newest first,
  capped at 50. Pressing `H` enters history mode showing the **newest** such
  commit's diff (revision 1) — i.e. one step back from the live baseline.
- `{` (Shift-`[`) steps to an **older** revision (higher index).
- `}` (Shift-`]`) steps to a **newer** revision (lower index). Stepping newer
  past revision 1 returns to the baseline (revision 0) — the live diff.
- `Esc` or `H` again exits history mode and restores the baseline diff.
- If the file has no history (no commits touch it), `H` shows a status message
  (`"no history for <file>"`) and does not enter history mode.

### Diff title

- Baseline (rev 0): unchanged from today's title.
- History revision N of M: `<file> @ <short-sha> (N/M) — <summary>`.

### Comments in history

- Pressing `c` on a history revision stores the comment into **that revision's
  commit scope**: `.turboreview/commits/<sha>/comments.json`, exactly as
  Commits-detail does today (reuses `CommentScope::Commit(sha)`).
- The active `comment_scope` is swapped to the revision's commit while a history
  revision is shown, and restored to the baseline scope when returning to rev 0
  or exiting history.
- Comment relocation runs against each revision's diff the same way
  `refresh_diff` already does.

### State (App)

```rust
/// Active file-history overlay over the diff pane.
pub struct FileHistory {
    /// The file whose history is being browsed (repo-relative).
    pub file: PathBuf,
    /// Commits touching `file`, newest first. Capped at MAX_FILE_HISTORY (50).
    pub commits: Vec<crate::git::CommitInfo>,
    /// 0 = baseline (live diff). 1..=commits.len() = commits[idx-1].
    pub idx: usize,
    /// The baseline comment scope to restore on exit (the scope active when H was pressed).
    pub baseline_scope: CommentScope,
}
```

New `App` field: `pub history: Option<FileHistory>`.

`history.is_some()` means history mode is active. `idx == 0` means showing the
baseline diff but still "in" history mode (so `{`/`}` keep working and Esc exits
cleanly).

### App methods

- `start_history(&mut self, commits: Vec<CommitInfo>)` — if `commits` is empty,
  no-op returning `false`; otherwise set `history = Some(FileHistory { file: selected, commits, idx: 1, baseline_scope: self.comment_scope.clone() })` and return `true`. Caller (main) then sets the per-revision scope and calls `refresh_diff`.
- `history_step(&mut self, delta: isize)` — clamp `idx` to `0..=commits.len()`.
- `exit_history(&mut self)` — restore `comment_scope = baseline_scope`, set
  `history = None`.
- `history_current_commit(&self) -> Option<&CommitInfo>` — `commits.get(idx-1)`
  when `idx >= 1`, else `None`.
- `history_active(&self) -> bool`.

### Git

New method on `Repo`:

```rust
/// Commits that touched `file`, newest first, walking from HEAD. Capped at `limit`.
/// Empty if HEAD is unborn or the file was never touched.
pub fn file_history(&self, file: &Path, limit: usize) -> Result<Vec<CommitInfo>>;
```

Implementation: revwalk from HEAD (TIME sort, like `log`). For each commit, diff
its tree vs its first parent's tree (vs empty tree for root commit) with a
pathspec on `file`; if that diff has any deltas, the commit touched the file —
include it. Build `CommitInfo` exactly as `log` does. Stop at `limit` matches.

Rendering a history revision reuses the existing `commit_diff_for(sha, file, ctx)`.

### main.rs wiring

- `refresh_diff` gains a history branch that runs **before** the existing
  commit-detail / working-tree branches: if `app.history_active()` and
  `idx >= 1`, render via `commit_diff_for(history_current_commit.id, file, ctx)`
  and relocate comments. If `idx == 0`, fall through to the normal baseline
  branches.
- The per-revision comment scope is set in the `H`/`{`/`}` key handlers (not in
  `refresh_diff`), so scope changes are explicit: when entering a revision set
  `app.comment_scope = Commit(sha)` and load that scope's comments; when
  returning to rev 0 restore baseline scope and reload baseline comments.
- Key handlers (only when `focus == Diff`, not in input modal):
  - `H` → if not active: build `file_history`, `start_history`; if it returned
    true, set scope+load, `refresh_diff`. If already active: `exit_history`,
    restore scope+load, `refresh_diff`.
  - `{` → `history_step(+1)` (older), then re-sync scope + `refresh_diff`.
  - `}` → `history_step(-1)` (newer), then re-sync scope + `refresh_diff`.
  - `Esc` while history active and Diff focused → `exit_history` first
    (before the existing Esc nav logic).

A small helper `sync_history_scope(repo, app)` centralizes
"set scope from current idx, load comments, refresh_diff" so the three handlers
share it.

### Interaction with existing features

- Switching view (`[`/`]`) or selecting a different file exits history first
  (call `exit_history` at the top of those handlers if active), so history never
  outlives its file.
- `F` (full-file), `+`/`-` (context) work in history — they just re-render the
  current revision via `commit_diff_for`.

---

## Feature 2: In-diff search

### Behaviour

- `/` (when Diff focused, not in comment modal) opens a one-line search input at
  the bottom (reuses the status-bar row, or a thin overlay line).
- Typing edits the query; `Enter` commits the search; `Esc` cancels (clears
  query and matches).
- On commit: compute `matches` = indices into `app.diff` of lines whose `text`
  contains the query, **case-insensitive**. Move `diff_cursor` to the first
  match at or after the current cursor (wrapping to the first overall if none
  after). If no matches, status message `"no match for <query>"` and clear.
- `n` → next match (wrap). `N` → previous match (wrap). These keys are only
  bound to search while a committed query with matches is live; otherwise they
  fall through to any existing binding (none today).
- Matched substrings in the diff are highlighted (accent background /
  reverse video) wherever the query appears in a rendered line.
- Search state is cleared when the diff is replaced (`set_diff`), so a stale
  query never points at lines from a different file/revision.

### State (App)

```rust
/// In-diff search state.
pub struct SearchState {
    /// Lowercased query (matching is case-insensitive).
    pub query: String,
    /// Indices into `app.diff` of matching lines, ascending.
    pub matches: Vec<usize>,
    /// Position within `matches` of the currently-focused match.
    pub cur: usize,
}
```

New `App` fields:
- `pub search: Option<SearchState>` — `Some` only while a committed query is
  live (has matches). Cleared on cancel, on no-match, and in `set_diff`.
- `pub search_input: Option<String>` — `Some(buffer)` while the `/` input line
  is open and being typed. Mutually distinct from `search` (input is the typing
  phase; `search` is the committed-result phase).

### App methods

- `search_start(&mut self)` — set `search_input = Some(String::new())` (only if
  Diff focused; caller guards).
- `search_input_push/backspace/cancel` — edit/clear `search_input`.
- `search_commit(&mut self) -> bool` — take `search_input`, lowercase it,
  compute matches over `self.diff`; if empty, leave `search = None` and return
  false; else set `search = Some(...)` with `cur` = first match index ≥
  `diff_cursor` (wrapping), move `diff_cursor` there, return true.
- `search_next(&mut self, delta: isize)` — advance `cur` with wraparound, move
  `diff_cursor` to `matches[cur]`. No-op if `search` is None.
- `search_clear(&mut self)` — `search = None; search_input = None`.
- `search_input_active(&self) -> bool`, `search_active(&self) -> bool`.
- `set_diff` is extended to call `self.search_clear()` (alongside the existing
  cursor/hscroll reset).

### main.rs wiring

- A dedicated input-handling block (mirroring the existing comment-modal block)
  runs when `app.search_input_active()`: `Enter` → `search_commit` +
  `refresh_diff`-independent (no diff reload; just cursor move); `Esc` →
  `search_input_cancel`; `Backspace`/char → edit buffer. This block intercepts
  keys before the normal dispatch, like the comment modal does.
- Normal dispatch (Diff focus): `/` → `search_start`. `n` → `search_next(+1)`
  if `search_active`. `N` → `search_next(-1)` if `search_active`.

### ui.rs wiring

- `render_diff`: when `search_active`, highlight occurrences of `search.query`
  (case-insensitive) within each line's rendered text using the palette accent
  background. The currently-focused match line may additionally use a stronger
  emphasis (optional; minimum: highlight all occurrences).
- When `search_input_active`, render the input line: `/<buffer>` in the status
  row area with a cursor block, replacing the `? for help` hint while typing.

---

## Testing

### git.rs (`file_history`)
- Repo with 3 commits, file touched in commits 1 and 3 only → `file_history`
  returns 2 commits, newest first.
- Unborn HEAD → empty.
- File never touched → empty.
- `limit` caps the result length.

### app.rs (history)
- `start_history` with empty commits → false, no state.
- `start_history` with commits → `history` set, `idx == 1`, baseline_scope
  captured.
- `history_step` clamps `idx` to `0..=len` (older past oldest stays, newer past
  rev 1 reaches 0, never negative).
- `exit_history` restores baseline scope and clears `history`.
- `history_current_commit` returns None at idx 0, the right commit at idx ≥ 1.

### app.rs (search)
- `search_commit` over a known diff finds the right line indices
  (case-insensitive).
- `search_commit` with no match → returns false, `search` stays None.
- `search_next` wraps forward and backward.
- `set_diff` clears an active search.
- Input push/backspace/cancel edit/clear the input buffer.

### Manual / integration
- History overlay: enter, step `{`/`}`, comment into a revision scope, exit,
  confirm baseline restored.
- Search: `/` term, `n`/`N` cycle, highlight renders, Esc clears.

---

## Out of scope (YAGNI)

- Regex search (plain case-insensitive substring only).
- Searching across files or the whole repo (current diff only).
- Persisting history position or last search across sessions.
- Editing/staging from the history overlay (read of past commits; only
  commenting is allowed, into that commit's scope).
