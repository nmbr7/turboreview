<h1 align="center">TurboReview</h1>

[![Rust](https://img.shields.io/badge/Rust-stable-orange)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-blue)](#)
[![UI](https://img.shields.io/badge/UI-Terminal%20TUI-6f42c1)](#)

`turboreview` is a terminal code-review tool for git repositories. Review the
working tree (unstaged/staged changes) **or** browse the branch's commit history
and review each commit's diff. The left pane lists files in a directory tree; the
right pane shows the selected file's diff with syntax highlighting, a line-number
gutter, and adjustable context. Stage or unstage whole files, mark files
reviewed, leave line comments — and let an AI coding agent read those comments,
fix the code, and respond.

<p align="center">
  <img width="800" alt="turbohex screenshot" src="https://github.com/user-attachments/assets/5fad1538-43b3-4a15-99ae-6b376a6224fc" />
  <br>
  <em>TurboReview diff</em>
</p>


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
  author, date). Press `Enter` on a commit to drill into its changed files (diff
  vs the commit's first parent); `Esc` steps back out.

In both views:

- **Left pane — Files.** Each file row shows a colored status letter
  (`A` added, `M` modified, `D` deleted, `R` renamed), a review tick
  (`✓` reviewed / `○` not), and a Nerd Font file-type icon. The pane can be
  resized or hidden entirely (see keys).
- **Right pane — Diff.** Syntax-highlighted code with a line-number gutter.
  Added/removed lines are bright with a green/red background; unchanged context
  lines are dimmed so changes stand out. A line cursor (highlighted row) marks
  where comments attach. The title shows `(ctx N)` or `(full file)`.

Press `?` at any time for an in-app keybinding overlay.

## Keys

| Key              | Action                                                          |
|------------------|-----------------------------------------------------------------|
| `[` / `]`        | switch view (Changes ⇆ Commits)                                 |
| `Tab`            | switch focus (Files / Diff pane)                                |
| `↑`/`↓` `j`/`k`  | move selection / commit / line cursor in the focused pane       |
| mouse wheel      | move the focused pane's cursor                                  |
| `gg` / `G`       | jump to top / bottom of the focused pane                        |
| `Enter`          | open a commit · focus a file's Diff · fold/unfold a directory   |
| `Esc`            | step back (Diff → files → commit list) / focus the Files pane   |
| `h`/`l` `←`/`→`  | scroll the diff horizontally (Diff pane)                        |
| `+` / `-`        | increase / decrease diff context lines (step 5)                 |
| `F`              | toggle full-file view (whole file vs hunks only)               |
| `z`              | hide / show the file pane (diff goes full-width)               |
| `<` / `>`        | narrow / widen the file pane                                    |
| `c`              | comment on the cursor line (opens a modal input box)            |
| `s`              | stage the selected file (Unstaged) / unstage it (Staged)        |
| `Space`          | toggle the reviewed checkbox on the selected file               |
| `R`              | toggle hiding reviewed files                                    |
| `r`              | refresh everything from disk / git                              |
| `?`              | show the keybinding help overlay                                |
| `q` / `Ctrl-C`   | quit                                                            |

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

## Review state & storage

Reviewed flags and comments persist under `<repo>/.turboreview/`:

| Path                                              | Holds                                   |
|---------------------------------------------------|-----------------------------------------|
| `.turboreview/comments.json` · `reviewed.json`    | working-tree review (Changes view)      |
| `.turboreview/commits/<sha>/comments.json` · `…`  | per-commit review (Commits view)        |
| `.turboreview/comment-log.jsonl`                  | append-only activity log (newest last)  |

The comment log records one JSON object per comment add/edit
(`{path, line, scope, date, action}`), so an agent can read the tail to find the
latest review activity. Add `.turboreview/` to your `.gitignore` if you don't
want it tracked. `R` hides reviewed files to declutter the list.

## Display notes

File-type icons use [Nerd Fonts](https://www.nerdfonts.com/) — install a Nerd
Font in your terminal for the glyphs to render correctly (otherwise you'll see
fallback boxes). Code highlighting uses the bundled Catppuccin Mocha–style dark
theme.
