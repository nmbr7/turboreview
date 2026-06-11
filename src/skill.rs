pub const SKILL_DOC: &str = r#"# turboreview review comments — agent guide

`turboreview` is a terminal code-review tool. A human reviewer leaves line
comments while reviewing a git diff. Comments are stored as JSON files under
`<repo>/.turboreview/`. As a coding agent, you read the open comments, make
the requested code changes, then write a response and update each comment's
status — all by editing the appropriate JSON file.

## File locations

### Working-tree review (Changes view)

Comments: `<repo-root>/.turboreview/comments.json`
Reviewed flags: `<repo-root>/.turboreview/reviewed.json`

These cover changes in the working tree (unstaged/staged files).

### Per-commit review (Commits-detail view)

Comments: `<repo-root>/.turboreview/commits/<sha>/comments.json`
Reviewed flags: `<repo-root>/.turboreview/commits/<sha>/reviewed.json`

Where `<sha>` is the full commit hash (40 hex characters) of the commit being
reviewed. Each commit gets its own isolated store.

### Activity log

`<repo-root>/.turboreview/comment-log.jsonl`

An append-only JSON-lines file. Each line is a JSON object written whenever a
comment is added or edited (not on relocation or load):

```json
{"path":"src/main.rs","line":42,"scope":"worktree","date":"2024-01-15","action":"set"}
{"path":"src/lib.rs","line":10,"scope":"commit:deadbeef1234...","date":"2024-01-15","action":"remove"}
```

Fields:
- `path` — repo-relative file path
- `line` — line number of the comment
- `scope` — `"worktree"` or `"commit:<sha>"` (full sha)
- `date` — date in `YYYY-MM-DD` format
- `action` — `"set"` (added/edited) or `"remove"` (deleted)

**To find the latest review activity**, read `comment-log.jsonl` from the end
(tail). Each scope's actual comment data lives in the corresponding
`comments.json` (worktree or the commit's directory).

## Schema

Each element of the `comments.json` array is a comment object:

| field            | type            | meaning                                                        |
|------------------|-----------------|----------------------------------------------------------------|
| `file`           | string (path)   | repo-relative path of the file the comment is on               |
| `line`           | number          | current line number (post-image / new-side line) of the comment|
| `hunk`           | string          | the diff hunk header for context, e.g. `@@ -1,4 +1,8 @@`        |
| `text`           | string          | the reviewer's comment                                         |
| `line_text`      | string          | the exact (trimmed) source line the comment was anchored to    |
| `context_before` | array of string | up to 2 source lines immediately before (for relocation)       |
| `context_after`  | array of string | up to 2 source lines immediately after                         |
| `orig_line`      | number          | line number when the comment was first created                 |
| `stale`          | bool            | true if turboreview could not confidently relocate the comment |
| `status`         | string          | one of `open`, `resolved`, `wontfix`, `needs_info`             |
| `response`       | string or null  | your reply to the reviewer                                     |

## Your workflow

1. Check `comment-log.jsonl` (tail) to identify recent activity and which scope
   (`worktree` or `commit:<sha>`) has open comments.
2. Read the appropriate `comments.json` (worktree or commit-specific).
3. For each comment with `status` == `open`:
   - Use `file`, `line`, `line_text`, and `context_before`/`context_after` to locate
     the exact spot in the code (line numbers may have shifted; `line_text` + context
     are the reliable anchor).
   - Read `text` — the requested change or question.
4. Make the code change that addresses the comment.
5. Update that comment object in the JSON:
   - Set `response` to a short explanation of what you did (or why not).
   - Set `status`:
     - `resolved`  — you made the requested change.
     - `wontfix`   — you deliberately did not change it; explain why in `response`.
     - `needs_info` — you need clarification; ask in `response`.
     - leave `open` only if you have not addressed it yet.
6. Write the JSON array back to the correct `comments.json` (preserve all other
   fields and all other comments unchanged; pretty-printed JSON is fine).

## Rules

- NEVER delete a comment object or change `file`, `line`, `text`, `line_text`,
  `context_*`, `orig_line`, or `hunk`. Only set `response` and `status`.
- Preserve the JSON array structure and all comments you are not responding to.
- Keep `response` concise (1–3 sentences). It is shown to the reviewer in the TUI.
- The reviewer re-opens turboreview to see your `response` and `status` inline in
  the diff.
- When editing a per-commit comment, write back to the commit-specific
  `comments.json` at `.turboreview/commits/<sha>/comments.json` — not the
  worktree file.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_doc_contains_required_content() {
        assert!(SKILL_DOC.contains("comments.json"), "SKILL_DOC must reference comments.json");
        assert!(SKILL_DOC.contains("status"), "SKILL_DOC must mention status field");
        assert!(SKILL_DOC.contains("response"), "SKILL_DOC must mention response field");
        assert!(SKILL_DOC.contains("open"), "SKILL_DOC must mention open status");
        assert!(SKILL_DOC.contains("resolved"), "SKILL_DOC must mention resolved status");
        assert!(SKILL_DOC.contains("wontfix"), "SKILL_DOC must mention wontfix status");
        assert!(SKILL_DOC.contains("needs_info"), "SKILL_DOC must mention needs_info status");
        assert!(SKILL_DOC.contains("comment-log"), "SKILL_DOC must reference comment-log");
        assert!(SKILL_DOC.contains("commits/<sha>"), "SKILL_DOC must describe per-commit path layout");
    }
}
