# turboreview — Design

**Date:** 2026-06-10
**Status:** Phase 1 spec (Phases 2 & 3 outlined, specced later)

## Context

`turboreview` is a terminal code-review tool for git working trees. The reviewer
sees the current diff (staged or unstaged, togglable), overlays llvm-cov coverage
on the changed lines, marks files as reviewed via a checkbox, and — for a selected
changed line — inspects live variables via a DAP debugger.

The work is **phased**. This document fully specs **Phase 1** and outlines Phases 2
and 3 (each gets its own spec + plan cycle later). Build Phase 1 end-to-end first.

## Stack

- `git2` (0.21) — repo access, diffs
- `ratatui` (0.30) + `crossterm` (default backend) — TUI + input
- `serde` / `serde_json` — config + (Phase 2) coverage JSON
- Phase 2: llvm-cov JSON export (`cargo llvm-cov --json`)
- Phase 3: `dap` crate (DAP client) + external `codelldb` adapter (>=1.11, stdio)
- `tempfile` — test fixtures (temp git repos)

---

## Phase 1 — Diff viewer + review checkboxes (THIS PHASE)

### Goal

Two-pane TUI. Left: changed-file list with a reviewed checkbox and selected-file
highlight. Right: diff of the selected file. Toggle staged/unstaged. Focus-based
navigation with scroll + `gg`/`G`. Reviewed state persists to a file.

### Layout

```
┌─ Files [UNSTAGED] ──┬─ Diff: src/main.rs ──────────────┐
│ [x] src/main.rs   ◄ │ @@ -1,4 +1,8 @@                   │
│ [ ] src/ui.rs       │ -old line                        │
│ [x] cov.rs          │ +new line                        │
│                     │  context line                    │
├─────────────────────┴──────────────────────────────────┤
│ Tab:focus  s:staged  Space:review  ↑↓/jk:move  gg/G  q  │
└─────────────────────────────────────────────────────────┘
```

- Header on left pane shows current mode: `[STAGED]` / `[UNSTAGED]`.
- Selected file: highlighted row (bg color). `◄` marker optional.
- Checkbox: `[x]` reviewed, `[ ]` not.

### Navigation (focus-based)

- `Tab` — switch focused pane (Files ⇄ Diff).
- `↑`/`↓` or `k`/`j` — act on focused pane: move file selection (Files) or scroll
  diff one line (Diff).
- `gg` — jump to top of focused pane (first file / diff top).
- `G` — jump to bottom of focused pane (last file / diff bottom).
- `s` — toggle staged/unstaged mode (refreshes file list + diff).
- `Space` — toggle reviewed checkbox on the selected file.
- `q` / `Ctrl-C` — quit.

(`s` chosen for mode toggle so `Tab` is free for pane focus.)

### Modules

1. **`git`** — wraps `git2`.
   - `open(path) -> Repo` via `Repository::discover` (path arg, default CWD).
   - `changed_files(mode) -> Vec<FileChange>` where `FileChange { path, status }`.
     - Unstaged: `diff_index_to_workdir`.
     - Staged: `diff_tree_to_index` (HEAD tree → index).
   - `diff_for(path, mode) -> Vec<DiffLine>` — `DiffLine { kind: Add|Del|Context|Hunk,
     text, old_lineno, new_lineno }`. Absolute line numbers retained (needed Phase 2).

2. **`review`** — persisted reviewed set.
   - State: `HashSet<PathBuf>`.
   - `load(repo_root)` / `save(repo_root)` → `<repo_root>/.turboreview/reviewed.json`
     (serde_json). Create dir if missing. Add `.turboreview/` to nothing — it lives
     in the target repo; document that user may gitignore it.
   - `toggle(path)`, `is_reviewed(path)`.

3. **`config`** — minimal in Phase 1 (program/adapter fields are Phase 3). Define the
   struct now with optional fields; Phase 1 only needs repo path resolution. May be a
   stub returning defaults.

4. **`app`** — central state, pure transitions (no IO, no render).
   - Fields: `mode: Mode`, `files: Vec<FileChange>`, `selected: usize`,
     `focus: Pane`, `diff: Vec<DiffLine>`, `diff_scroll: usize`,
     `reviewed: HashSet<PathBuf>`.
   - Transitions: `move_selection(±1)`, `set_selection(idx)`, `scroll_diff(±1)`,
     `to_top()`, `to_bottom()`, `toggle_focus()`, `toggle_mode()`, `toggle_reviewed()`.
   - On selection/mode change: caller refreshes `diff` from `git` and persists review
     set. `app` exposes what changed; IO done in `main`.

5. **`ui`** — render `&App` → frame. Two panes (`Layout` horizontal split) + status
   bar (vertical split). File rows: checkbox glyph + path, selected row gets bg via
   `Style::bg`. Diff lines colored by kind (add=green fg, del=red fg, hunk=cyan).
   Focused pane gets a highlighted border. No git, no mutation.

6. **`main`** — parse args (repo path, optional `--config`), `crossterm` raw mode +
   alt screen, load review set, build `App`, event loop: read key → `app` transition →
   refresh diff if needed → `ui::render` → on review change `review::save`. Restore
   terminal on exit (including panic hook).

### Data flow

`key → app transition → (if file/mode changed) git::diff_for → app.diff → ui::render`.
Review toggle → `app.reviewed` → `review::save`.

### Error handling

- Not a git repo → print clean message, exit non-zero. No panic.
- No changes → empty file list, right pane shows "No changes" placeholder.
- `git2` / IO errors → surface to status bar; never panic the TUI. Terminal always
  restored via guard + panic hook.

### Testing

- `git` module: unit tests against temp repos (`tempfile` + `git2` init; write/stage
  files) — assert `changed_files` and `diff_for` for staged vs unstaged.
- `review` module: round-trip `load`/`save`, toggle behavior, dir creation.
- `app` module: pure transition tests (selection clamping, focus toggle, mode toggle,
  scroll bounds, gg/G) — no terminal.
- `ui`: smoke render via ratatui `TestBackend` — asserts no panic + key glyphs present.

### Out of scope (Phase 1)

Coverage overlay, debugger, variables pane, search, editing/staging actions.

---

## Phase 2 — Coverage overlay (outline)

- **`coverage`** module: parse `cargo llvm-cov --json` export with serde. Build
  `file -> {line -> hit_count}`. NOTE: exact JSON shape (`data[].files[].segments`
  vs summary) must be confirmed against real output before coding this phase.
- `ui` diff rendering: covered changed line → green bg highlight; uncovered → default;
  **selected diff line → stronger bg** (selected wins over coverage color).
- Coverage file path from `--config` / CLI arg. App reads existing file; does not run
  llvm-cov.

## Phase 3 — DAP debug pane (outline)

- Add 3rd right pane: variables at the selected changed line.
- **`debug`** module: `dap` crate client driving `codelldb` over stdio.
  - Launch debuggee from config (program/args/adapter path) **or** attach by PID —
    both supported, chosen via config.
  - `set_breakpoint(file, line)` on selected changed line → `continue` → on stop,
    `scopes` → `variables` → `Vec<Var { name, value, type }>`.
- `config` gains: adapter path, launch program/args, attach PID.
- `dap` crate is pre-1.0 — expect churn; isolate behind the `debug` module interface.
