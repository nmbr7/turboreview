pub const SKILL_DOC: &str = r#"# turboreview review comments — agent guide

`turboreview` is a terminal code-review tool. A human reviewer leaves line
comments while reviewing a git diff. Comments are stored in
`<repo>/.turboreview/comments.json` as a JSON array. As a coding agent, you
read the open comments, make the requested code changes, then write a response
and update each comment's status — all by editing that JSON file.

## File location

`<repo-root>/.turboreview/comments.json`

## Schema

Each element of the array is a comment object:

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

1. Read `.turboreview/comments.json`.
2. For each comment with `status` == `open`:
   - Use `file`, `line`, `line_text`, and `context_before`/`context_after` to locate
     the exact spot in the code (line numbers may have shifted; `line_text` + context
     are the reliable anchor).
   - Read `text` — the requested change or question.
3. Make the code change that addresses the comment.
4. Update that comment object in the JSON:
   - Set `response` to a short explanation of what you did (or why not).
   - Set `status`:
     - `resolved`  — you made the requested change.
     - `wontfix`   — you deliberately did not change it; explain why in `response`.
     - `needs_info` — you need clarification; ask in `response`.
     - leave `open` only if you have not addressed it yet.
5. Write the JSON array back to `.turboreview/comments.json` (preserve all other
   fields and all other comments unchanged; pretty-printed JSON is fine).

## Rules

- NEVER delete a comment object or change `file`, `line`, `text`, `line_text`,
  `context_*`, `orig_line`, or `hunk`. Only set `response` and `status`.
- Preserve the JSON array structure and all comments you are not responding to.
- Keep `response` concise (1–3 sentences). It is shown to the reviewer in the TUI.
- The reviewer re-opens turboreview to see your `response` and `status` inline in
  the diff.
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
    }
}
