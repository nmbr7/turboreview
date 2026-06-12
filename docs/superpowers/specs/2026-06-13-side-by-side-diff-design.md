# Side-by-Side Diff View — Design

**Date:** 2026-06-13
**Status:** Approved (design)

## Goal

Add a side-by-side (split) diff rendering mode to the diff pane: old/left vs
new/right, toggled with `v`, persisted in `config.json`. Unified mode stays the
default. The underlying diff data model (`diff: Vec<DiffLine>`) is unchanged —
side-by-side is a pure render mode, so comments, search, the line cursor, and
anchoring all keep working exactly as today.

## Approach: render-only over `Vec<DiffLine>`

The cursor still indexes a single `DiffLine` (`diff_cursor`). Side-by-side only
changes how `render_diff` lays lines out. We derive paired rows at render time
from the existing `diff`.

### Pairing algorithm

Walk `diff` in order, grouping into visual rows:

- **Hunk** line → a full-width header row (spans both columns), same as unified.
- **Context** line → one row: same text on left and right (with its
  `old_lineno` on the left gutter, `new_lineno` on the right).
- A maximal run of consecutive **Del** lines immediately followed by a run of
  **Add** lines → pair them: for `k` in `0..max(dels, adds)`, emit a row with
  `dels.get(k)` on the left (blank if absent) and `adds.get(k)` on the right
  (blank if absent). A del with no matching add → left only, right blank; an add
  with no matching del → right only, left blank.
- A run of **Del** with no following Add → each on the left, right blank.
- A run of **Add** with no preceding Del → each on the right, left blank.

Each paired row records which `DiffLine` index sits on the left and which on the
right (either may be `None` for a blank cell). This index is what the cursor and
comment logic key off.

### Cursor in side-by-side

`diff_cursor` still points at a `DiffLine` index. A paired row is "the cursor
row" when either its left or right cell holds `diff_cursor`. The whole row is
highlighted (both cells get the selected background) so the user sees the
current line regardless of side. Up/down still moves `diff_cursor` by one
`DiffLine` (unchanged `move_diff_cursor`); visually the cursor advances through
left/right cells in diff order, which is acceptable and predictable.

### Comments in side-by-side

`c` comments on the `DiffLine` under `diff_cursor` exactly as today (no change to
`start_comment`/anchoring). The inline comment box renders **full width** under
the paired row that contains the commented line, reusing the existing box
drawing. The `rendered_height`/scroll math counts the row height (1) plus the
comment box height, same as unified.

### Search highlighting

Unchanged: a matching `DiffLine` tints its cell background. In side-by-side the
tint lands on whichever side that line occupies.

## State

`App` gains:

```rust
pub split_diff: bool, // false = unified (default), true = side-by-side
```

Method `toggle_split(&mut self)` flips it. Initialized from persisted config in
`main`.

## Persistence

`storage::Config` currently has only `theme`, and `save_theme` writes the whole
struct (so a future `save_split` would clobber `theme`). Refactor to a single
read-modify-write config:

```rust
#[derive(Serialize, Deserialize, Default)]
struct Config {
    theme: String,           // "dark" | "light"
    #[serde(default)]
    split_diff: bool,        // side-by-side toggle
}

fn load_config(repo_root: &Path) -> Config;          // private helper
fn save_config(repo_root: &Path, cfg: &Config) -> Result<()>;
```

`load_theme`/`save_theme` become thin wrappers over `load_config`/`save_config`
that only touch the `theme` field (save = load, set theme, write). New
`load_split(repo_root) -> bool` and `save_split(repo_root, bool)` do the same for
`split_diff`. This keeps both fields when either is saved. `#[serde(default)]`
on `split_diff` keeps old config.json files (theme-only) loading cleanly.

## Keys

- `v` (when not in an input modal): `app.toggle_split()` then
  `storage::save_split(&app.repo_root, app.split_diff)` (best-effort, like `T`).
  No diff reload needed — same data, different render.

## Rendering (`ui.rs`)

`render_diff` branches on `app.split_diff`:

- **Unified** (false): the current code path, untouched.
- **Side-by-side** (true): split the inner diff area into two equal columns with
  a 1-char vertical separator. Build paired rows via a helper
  `pair_diff_rows(&app.diff) -> Vec<RowPair>` (pure, unit-testable, lives in
  `ui.rs` or a small `splitdiff` module). For each visible paired row:
  - Hunk header row: render across the full width (hunk color), as unified.
  - Otherwise: render the left cell (gutter + old line text, del background /
    dim context) and the right cell (gutter + new line text, add background /
    dim context); a blank cell renders as empty with the pane background.
  - Cursor row: both cells get `selected_bg`.
  - Search match: the matching cell gets `accent_dim` background.
  - Inline comment box: full-width under the row, reusing the unified box code.

`RowPair`:

```rust
struct RowPair {
    /// Full-width header (hunk) line; when Some, left/right are ignored.
    header: Option<usize>,  // diff index of the Hunk line
    left: Option<usize>,    // diff index shown on the left (old side)
    right: Option<usize>,   // diff index shown on the right (new side)
}
```

The scroll/height math operates on paired rows (each row height = 1 + its
comment box), mirroring the unified `rendered_height` but in paired-row space.
The cursor row is found as the paired row whose `left` or `right` == `diff_cursor`.

## Help & docs

- Help overlay "Diff view" section gains: `v  side-by-side / unified diff`.
- README Keys table + a sentence in the diff section.

## Testing

### `pair_diff_rows` (pure, in ui.rs tests)
- Context line → one row, left==right==that index.
- Equal del/add run (2 del, 2 add) → 2 rows, pairing del[i]/add[i].
- Unequal run (2 del, 1 add) → 2 rows; second row has right == None.
- Add-only run → rows with left == None.
- Del-only run → rows with right == None.
- Hunk line → header row with left/right None.

### `App`
- `toggle_split` flips `split_diff`.

### storage
- `save_split` then `load_split` round-trips true.
- Saving theme after split keeps split (no clobber): set split=true, save_theme,
  load_split still true.
- Old theme-only config.json still loads (split defaults false).

### ui render (smoke)
- With `split_diff = true` and a small diff, render does not panic and the dump
  contains both an old-side and new-side token.

## Out of scope (YAGNI)

- Intra-line (word-level) highlighting of changes.
- Independent per-side vertical scrolling (both sides scroll together by row).
- Syncing `v` with the file pane auto-hide (user can `z` to reclaim width).
