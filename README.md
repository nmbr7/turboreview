# turboreview

Terminal code-review tool for git working trees. The left pane lists changed
files in a directory tree, split into **Unstaged** and **Staged** sections; the
right pane shows the selected file's diff with syntax highlighting, a line-number
gutter, and adjustable context. Stage or unstage whole files, mark files
reviewed, and read diffs full-file or hunk-only.

## Usage

```
turboreview [REPO_PATH]   # defaults to the current directory
```

Run inside (or point at) any git repository.

## Layout

- **Left pane — Files.** Two sections, `▌ Unstaged (N)` and `▌ Staged (N)`, each a
  collapsible directory tree. The selected file's diff loads on the right. A tick
  (`✓` reviewed / `○` not) sits to the left of each filename; file-type icons use
  Nerd Font glyphs.
- **Right pane — Diff.** Syntax-highlighted code with a line-number gutter.
  Added/removed lines are bright with a green/red background; unchanged context
  lines are dimmed so changes stand out. The title shows `(ctx N)` or
  `(full file)`.

## Keys

| Key              | Action                                                       |
|------------------|--------------------------------------------------------------|
| `Tab`            | switch focus (Files / Diff pane)                             |
| `↑`/`↓` `j`/`k`  | move selection (Files) / scroll diff (Diff)                  |
| mouse wheel      | scroll the focused pane                                      |
| `gg` / `G`       | jump to top / bottom of the focused pane                     |
| `Enter`          | on a file: focus the Diff pane · on a directory: fold/unfold |
| `Esc`            | return focus to the Files pane                               |
| `h`/`l` `←`/`→`  | scroll the diff horizontally (Diff pane)                     |
| `+` / `-`        | increase / decrease diff context lines (step 5)              |
| `F`              | toggle full-file view (whole file vs hunks only)             |
| `s`              | stage the selected file (Unstaged) / unstage it (Staged)     |
| `Space`          | toggle the reviewed checkbox on the selected file            |
| `R`              | toggle hiding reviewed files                                 |
| `q` / `Ctrl-C`   | quit                                                         |

## Staging

`s` on a file in the **Unstaged** section stages it; on a file in the **Staged**
section it unstages it. Staging only moves changes into or out of the git index —
your working-tree files are never modified. (Hunk-level staging is not yet
supported.)

## Review state

Reviewed files persist to `<repo>/.turboreview/reviewed.json`. Add
`.turboreview/` to your `.gitignore` if you don't want it tracked. `R` hides
reviewed files to declutter the list.

## Display notes

File-type icons use [Nerd Fonts](https://www.nerdfonts.com/) — install a Nerd
Font in your terminal for the glyphs to render correctly (otherwise you'll see
fallback boxes). Code highlighting uses the bundled Catppuccin Mocha–style dark
theme.
