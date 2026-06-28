<h1 align="center">TurboReview</h1>

[![Rust](https://img.shields.io/badge/Rust-stable-orange)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-blue)](#)
[![UI](https://img.shields.io/badge/UI-Terminal%20TUI-6f42c1)](#)

`turboreview` is a terminal code-review tool for git repositories. Review the
working tree or browse the branch's commit history, with a file tree on the left
and the selected diff (syntax-highlighted, adjustable context) on the right.
Stage files, mark them reviewed, leave line comments — and let an AI coding agent
read those comments, fix the code, and respond. It can also **debug** straight
from the diff (breakpoints, stepping, variable inspection, even debugging a past
commit) and overlay **test coverage**.

<p align="center">
  <img width="800" alt="turboreview screenshot" src="https://github.com/user-attachments/assets/69ca499b-052b-47e0-b567-72e123ac4568" />
  <br>
  <em>TurboReview diff</em>
</p>


## Install

**Prerequisites:**

- A recent stable Rust toolchain (`cargo`) and `git`. Install Rust via
  [rustup](https://rustup.rs/): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`.
- A C compiler / build toolchain — `git2` (libgit2) and `libz` are built from
  source. macOS: Xcode Command Line Tools (`xcode-select --install`). Linux:
  `build-essential` and `pkg-config`.
- **Linux only:** X11 clipboard libraries for copy support (`arboard`), e.g. on
  Debian/Ubuntu: `libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.
- A [Nerd Font](https://www.nerdfonts.com/) set in your terminal for the
  file-type glyphs.

### From crates.io

```sh
cargo install turboreview
```

The `turboreview` binary lands in `~/.cargo/bin` — make sure that's on your
`PATH`.

### From source

```sh
git clone https://github.com/nmbr7/turboreview.git
cd turboreview
cargo install --path .          # installs the release binary to ~/.cargo/bin
```

### Developing

```sh
cargo run -- [REPO_PATH]        # run against a repo (defaults to .)
cargo test                      # run the test suite
cargo fmt && cargo clippy       # format + lint
```

Updating later: re-run `cargo install turboreview` (add `--force` to overwrite
an older build).

## Usage

```
turboreview [REPO_PATH]   # defaults to the current directory
turboreview --skill       # print the AI-agent guide and exit
```

Run inside (or point at) any git repository.

## Layout

Two views, toggled with `[` / `]`:

- **Changes** — the working tree. Left pane has two sections, `▌ Unstaged (N)`
  and `▌ Staged (N)`, each a collapsible directory tree.
- **Commits** — the branch history. Left pane lists commits (short hash, summary,
  author, date, and per-commit diff stats `N files +ins -del`). Diff stats are
  computed lazily for the rows in view. The list loads 50 commits at a time;
  scroll to the bottom and press `L` to load the next page. Press `Enter` on a
  commit to drill into its changed files (diff vs the commit's first parent);
  `Esc` steps back out.

In both views:

- **Left pane — Files.** Each file row shows a colored status letter
  (`A` added, `M` modified, `D` deleted, `R` renamed), a review tick
  (`✓` reviewed / `○` not), and a Nerd Font file-type icon. The pane can be
  resized or hidden entirely (see keys).
- **Right pane — Diff.** Syntax-highlighted code with a line-number gutter.
  Added/removed lines are bright with a green/red background; unchanged context
  lines are dimmed so changes stand out. A line cursor (highlighted row) marks
  where comments attach. The title shows `(ctx N)` or `(full file)`. Press `v`
  for a side-by-side (split) view — old on the left, new on the right — which is
  remembered across sessions.
- **Comments pane (toggle with `C`).** An optional third column listing the
  current scope's comments grouped by status (Open / NeedsInfo / Wontfix /
  Resolved). Press `Enter` on a comment to jump to its file and line.

Press `?` at any time for an in-app keybinding overlay.

## Keys

| Key              | Action                                                          |
|------------------|-----------------------------------------------------------------|
| `[` / `]`        | switch view (Changes ⇆ Commits)                                 |
| `Tab`            | switch focus across the visible panes                           |
| `↑`/`↓` `j`/`k`  | move selection / commit / line cursor in the focused pane       |
| `⇧↑`/`⇧↓` `J`/`K`| jump (fast scroll) in the focused pane                          |
| mouse wheel      | move the focused pane's cursor                                  |
| `gg` / `G`       | jump to top / bottom of the focused pane                        |
| `L`              | load more commits (Commits view, when more history exists)       |
| `Enter`          | open commit · focus a file's Diff · fold dir · jump to comment  |
| `C`              | toggle the comment-list pane                                    |
| `Esc`            | step back (Diff → files → commit list) / focus the Files pane   |
| `h`/`l` `←`/`→`  | scroll the diff horizontally (Diff pane)                        |
| `+` / `-`        | increase / decrease diff context (step 5; `+` at max → full file) |
| `F`              | toggle full-file view (shortcut; also reachable via `+`)          |
| `v`              | toggle side-by-side (split) vs unified diff (persisted)          |
| `H`              | file-history overlay for the selected file's diff               |
| `{` / `}`        | step to older / newer revision (in the history overlay)         |
| `/`              | search within the current diff                                  |
| `n` / `N`        | jump to next / previous search match                            |
| `a`              | fold / unfold all directories in the file tree                 |
| `O`              | view all tracked files / changed files only                     |
| `z`              | hide / show the file pane (diff goes full-width)               |
| `<` / `>`        | narrow / widen the focused pane (files or the right pane)        |
| `c`              | comment on the cursor line (opens a modal input box)            |
| `s`              | stage the selected file (Unstaged) / unstage it (Staged)        |
| `Space`          | toggle the reviewed checkbox on the selected file               |
| `R`              | toggle hiding reviewed files                                    |
| `r`              | refresh everything from disk / git                              |
| `%`              | toggle test-coverage highlight (LCOV)                            |
| `M`              | run the configured coverage command, then show it               |
| `?`              | show the keybinding help overlay                                |
| `qq` / `Ctrl-C`  | quit (press `q` twice to avoid accidental exits)                |

**Debugging** (right pane Debug tab; see [Debugging](#debugging)):

| Key              | Action                                                          |
|------------------|-----------------------------------------------------------------|
| `b`              | toggle a breakpoint on the cursor line (Diff pane)              |
| `D`              | launch a debug session (a selected commit in Commits, else worktree) |
| `[` / `]`        | right pane: switch the Comments / Debug tab (when focused)       |
| `t`              | Debug tab: switch the Vars / Breakpoints view                   |
| `c`/`n`/`i`/`o`  | continue / step over / step in / step out (Debug focused)       |
| `Enter`          | Vars: expand a variable · Breakpoints: jump to it                |
| `Space` / `d`    | Breakpoints tab: enable-disable / delete a breakpoint           |
| `h` / `l`        | scroll the Debug pane horizontally                              |
| `Ctrl-D`         | in the comment box: attach the stopped call stack to the comment |
| `X`              | end all debug sessions                                          |

In the comment input box: type freely, **Enter** for a newline, **Ctrl-S** to
save, **Esc** to cancel. Saving an empty comment deletes it.

## Staging

`s` on a file in the **Unstaged** section stages it; on a file in the **Staged**
section it unstages it. Staging only moves changes into or out of the git index —
your working-tree files are never modified. (Hunk-level staging is not yet
supported.)

## Comments

Press `c` on a diff line to attach a comment. Comments render in a bordered box
directly under their line.

Each comment is **anchored** to its line's content plus the surrounding lines, so
it follows the line when the file shifts (e.g. lines inserted above). If a comment
can no longer be confidently placed, it is shown as `⚠ outdated` rather than lost.

### AI-agent review loop

Comments carry a `status` (`open`, `resolved`, `wontfix`, `needs_info`) and an
optional `response`. An AI coding agent can close the loop:

1. Run `turboreview --skill` to print a guide describing the on-disk schema and
   workflow.
2. The agent reads the open comments, makes the requested changes, then writes a
   `response` and sets the `status` in the relevant `comments.json`.
3. Reopen turboreview — each comment box shows the agent's status badge
   (`✓ resolved`, `✗ wontfix`, `? needs-info`) and its response inline.

## File history & search

Press `H` on a file's diff to enter the **history overlay** — it walks the
commits that touched that file (newest first). `{` steps to an older revision,
`}` to a newer one; `}` past the newest returns to the live diff, and `Esc`
exits. While stepping, the cursor stays on the same line number, the view
scrolls to keep it centered, and context is expanded (up to full file) if needed
to show that line. Comments made on a past revision are stored in that commit's
scope (`.turboreview/commits/<sha>/`).

Press `/` to **search** within the current diff (case-insensitive substring).
`n` and `N` jump to the next and previous match.

## Debugging

turboreview can drive a debugger straight from the diff, using the [Debug Adapter
Protocol](https://microsoft.github.io/debug-adapter-protocol/) (any DAP adapter:
`lldb-dap`, `codelldb`, `debugpy`, …). Configure it under `debug` in
`.turboreview/config.json` (see [docs/debug-config-example.md](docs/debug-config-example.md)):

```jsonc
"debug": {
  "adapter": { "command": "lldb-dap", "args": [] },
  "build":   "cargo build",                 // run before launch
  "program": "target/debug/your-binary",    // built binary (relative to source root)
  "args": [], "cwd": ".",
  "source_map": []
}
```

- Press `b` on a diff line to set a breakpoint (a `●` shows in the gutter), then
  `D` to build + launch. The **right pane's Debug tab** shows the call stack and
  the selected frame's variables; `▶` marks the stopped line.
- `Tab` to focus the Debug pane, then step with `c` / `n` / `i` / `o`. `Enter`
  expands a structured value (String/Vec/struct/map) to its contents; each
  variable shows its type and memory address.
- `t` switches the Debug pane between **Vars** and **Breakpoints** (the
  breakpoint list: `Enter` jumps, `Space` enables/disables, `d` deletes).
- Press `D` to open the **launch picker**: debug the working tree, a selected
  commit, attach to a running process, or attach to a remote target.
- **Attach to a process:** pick *process* to open a filterable list of running
  processes (type to filter by name or pid, `Enter` to attach).
- **Debug a past commit:** pick *commit* in the Commits view — turboreview checks
  it out in a throwaway `git worktree`, builds it there, and debugs that
  historical binary (cleaned up when the session ends).
- **Remote attach (gdbserver / Docker):** pick *remote* to attach to a running
  target. Configure it under `debug.remote` (and use `source_map` to map the
  remote source paths to your local checkout):

  ```jsonc
  "debug": {
    "remote": { "host": "localhost", "port": 1234 },
    "source_map": [["/build/src", "/local/repo/src"]]
  }
  ```

  `host`/`port` run `gdb-remote host:port`; for other setups set
  `remote.attach_commands` to raw adapter commands instead.
- **Attach a snapshot to a comment:** while stopped, press `c` to comment, then
  `Ctrl-D` to attach the current call stack + locals. It's saved with the comment
  and shown inline, so a reviewer keeps the exact runtime state.
- `O` toggles **view all files** (every tracked file, not just changed ones) so
  you can open and breakpoint anywhere in the codebase. `X` ends the session.

## Test coverage

Press `%` to overlay **coverage** from an LCOV file: a `▌` bar in the gutter is
green for covered lines, red for uncovered. The footer shows overall and
current-file percentages. Configure the file (and, optionally, a command that
generates it) in `.turboreview/config.json`:

```jsonc
"coverage_file": "coverage/lcov.info",
"coverage_command": "cargo llvm-cov --lcov --output-path coverage/lcov.info"
```

Press `M` to run the command and reload. Any tool that emits LCOV works
(`cargo llvm-cov`, `tarpaulin`, `nyc`, `gcov`, …).

## Macro-expanded view

Press `e` to replace the diff with the **macro-expanded source** of the selected
file (read-only, syntax-highlighted); `e` again restores the diff. It runs a
configurable command and shows its stdout. The `{module}` and `{file}`
placeholders are substituted (module path derived from the `src/`-relative path):

```jsonc
// crate with both a lib and a bin needs an explicit target:
"expand_command": "cargo expand --lib {module}"
```

Default is `cargo expand {module}` (needs nightly + `cargo-expand`). Switching
files leaves the expanded view.

Configure **several** named commands to get a picker on `e`:

```jsonc
"expand_commands": [
  { "name": "lib",     "command": "cargo expand --lib {module}" },
  { "name": "example", "command": "cargo expand --example $(basename {file} .rs)" }
]
```

With one (or none) configured, `e` runs it directly; with several, it opens a
selection popup.

## Review state & storage

Reviewed flags and comments persist under `<repo>/.turboreview/`:

| Path                                              | Holds                                   |
|---------------------------------------------------|-----------------------------------------|
| `.turboreview/comments.json` · `reviewed.json`    | working-tree review (Changes view)      |
| `.turboreview/commits/<sha>/comments.json` · `…`  | per-commit review (Commits view)        |
| `.turboreview/comment-log.jsonl`                  | append-only activity log (newest last)  |

The comment log records one JSON object per comment add/edit
(`{path, line, scope, date, action}`, where `date` is a `YYYY-MM-DD HH:MM:SS`
timestamp), so an agent can read the tail to find the latest review activity. Add `.turboreview/` to your `.gitignore` if you don't
want it tracked. `R` hides reviewed files to declutter the list.

## Display notes

File-type icons use [Nerd Fonts](https://www.nerdfonts.com/) — install a Nerd
Font in your terminal for the glyphs to render correctly (otherwise you'll see
fallback boxes). Code highlighting uses the bundled Catppuccin Mocha–style dark
theme.
