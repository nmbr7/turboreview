# turboreview

Terminal code-review tool. Browse a git repo's working-tree diff (staged or
unstaged), with a collapsible directory tree, syntax-highlighted code, line
numbers, and per-file review checkboxes.

## Usage

```
turboreview [REPO_PATH]   # defaults to the current directory
```

Run inside (or point at) any git repository. The left pane shows changed files
in a directory tree; the right pane shows the diff of the selected file with
syntax highlighting and a line-number gutter.

### Keys

| Key                | Action                                              |
|--------------------|-----------------------------------------------------|
| `Tab`              | switch focus (Files / Diff pane)                    |
| `↑`/`↓` `j`/`k`    | move selection (Files) / scroll diff (Diff)         |
| mouse wheel        | scroll the focused pane                             |
| `gg` / `G`         | jump to top / bottom of the focused pane            |
| `Enter`            | fold/unfold the selected directory (Files pane)     |
| `h`/`l` `←`/`→`    | scroll the diff horizontally (Diff pane)            |
| `s`                | toggle staged / unstaged                             |
| `Space`            | toggle the reviewed checkbox on the selected file   |
| `R`                | toggle hiding reviewed files                         |
| `q` / `Ctrl-C`     | quit                                                |

### Review state

Reviewed files persist to `<repo>/.turboreview/reviewed.json`. Add
`.turboreview/` to your `.gitignore` if you don't want it tracked.

### Display notes

File-type icons use [Nerd Fonts](https://www.nerdfonts.com/) — install a Nerd
Font in your terminal for the glyphs to render correctly (otherwise you'll see
fallback boxes). Code highlighting uses a bundled dark theme.
