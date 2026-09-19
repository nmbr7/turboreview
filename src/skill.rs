pub const SKILL_DOC: &str = r#"# turboreview review comments — agent guide

`turboreview` is a terminal code-review tool. A human reviewer leaves line
comments while reviewing a git diff. Comments are stored as JSON files under
`<repo>/.turboreview/`. As a coding agent, you read the open comments, make
the requested code changes, then write a response and update each comment's
status — all by editing the appropriate JSON file.

## Setup (in the target repository)

`turboreview` keeps its review state in a `.turboreview/` directory at the repo
root. This is local review metadata, not project source — add it to the
repository's `.gitignore` so it is never committed:

    # .gitignore
    .turboreview/

## File locations

Everything lives under `<repo-root>/.turboreview/`:

    .turboreview/
    ├── comments.json              # worktree review — active comments
    ├── reviewed.json              # worktree review — reviewed-file flags
    ├── comment-log.jsonl          # append-only activity log (newest last)
    ├── lessons.md                 # distilled review lessons (see "Learning from past reviews")
    ├── archive/
    │   └── comments-archive.jsonl # resolved comments, append-only, repo-wide
    └── commits/
        └── <sha>/
            ├── comments.json      # per-commit review — active comments
            └── reviewed.json      # per-commit review — reviewed-file flags

Read the directory to see what actually exists — `lessons.md` and `archive/`
are only present once something has written them.

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
{"path":"src/main.rs","line":42,"scope":"worktree","date":"2024-01-15 14:30:00","action":"set"}
{"path":"src/lib.rs","line":10,"scope":"commit:deadbeef1234...","date":"2024-01-15 14:30:05","action":"remove"}
```

Fields:
- `path` — repo-relative file path
- `line` — line number of the comment
- `scope` — `"worktree"` or `"commit:<sha>"` (full sha)
- `date` — date and time in `YYYY-MM-DD HH:MM:SS` format (UTC)
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
| `updated`        | number          | unix epoch seconds of the last edit (0 for legacy comments without it) |
| `debug_snapshot` | object or null  | read-only: call stack + locals captured at a breakpoint (do not edit) |

## Archive

Resolved comments may be moved to `.turboreview/archive/comments-archive.jsonl` to
keep the active `comments.json` small. This happens automatically on startup for
resolved comments older than 14 days, or manually when the reviewer presses `A`.

The archive file is **append-only JSON lines** — one serialized comment object per
line, same schema as the `comments.json` array elements above.

For the routine fix loop you only need the active `comments.json`. But the archive
is the durable record of review feedback that was actually acted on, and it is the
primary input for a lesson pass (see below) — so don't discard it as noise. Two
properties affect how you read it:

- It is **repo-wide and cross-scope**. Archived per-commit comments land in the same
  file as worktree ones, and the originating scope is not recorded on the line. Don't
  assume a line came from the worktree review.
- It is **incomplete by construction**. Only resolved comments are ever archived, and
  editing a comment overwrites its text in place, so earlier wording is not recoverable
  anywhere. Draw lessons from what is present; don't treat the archive as a full history.

## Your workflow

1. Read `lessons.md` if it exists — it records preferences this reviewer has already
   asked for, and applies to the changes you are about to make. Then check
   `comment-log.jsonl` (tail) to identify recent activity and which scope
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
7. **Hand back to the reviewer. Stop here.** Do not `git add` and do not commit.
   Report what you changed, file by file, and which comments you set to which status.
   The reviewer reopens turboreview to see your responses inline in the diff and
   decides whether each fix is right. Stage and commit only after the user explicitly
   confirms — and when they do, run `git add` and `git commit` as separate commands.

## Watching for new comments (live review)

The reviewer may be working in turboreview while you are working in the code. If
you are asked to watch (stay running and respond as comments arrive), rather than
to do a single pass:

1. Poll the active scope's `comments.json` for changes. Compare modification time
   *and* file length — two writes inside one filesystem timestamp tick can share
   an mtime. Every second or two is plenty; this is a human typing.
2. On a change, re-read the file and look for comments with `status` == `open`
   that you have not already answered.
3. Fix, respond, and set the status exactly as in the workflow above.
4. Go back to waiting. Do not exit after the first comment.

**Do not clobber the reviewer's edits.** You and turboreview both write this file,
and turboreview holds the whole array in memory. So:

- Re-read `comments.json` immediately before writing it, every time. Never write
  from a copy you read minutes ago — the reviewer has almost certainly added a
  comment since.
- Only ever change `response` and `status`, on the specific comments you are
  answering. Preserve every other comment and field byte for byte. A whole-file
  rewrite from stale state silently destroys comments the reviewer just wrote.
- Write the file once per batch of fixes, not once per comment. Each write makes
  the reviewer's UI flag an update; a burst of writes is noise.

The reviewer sees a notification when you write, and reloads when they are ready.
They may be mid-read, so the reload happens on their keypress, not yours — which
means a comment you resolved can stay on their screen as `open` for a while. That
is expected; do not write again to try to force it through.

**Waiting on the reviewer.** Set `needs_info` and stop when you need an answer.
Do not guess and do not keep polling for a reply to a question you just asked —
the next comment change will tell you. If the reviewer reopens a comment you
resolved (status back to `open`), treat it as a rejection of your fix: read the
updated `text` for what they actually want before changing anything.

## Learning from past reviews

The same comment should not have to be written twice. When the reviewer keeps asking
for the same thing, that preference belongs in `.turboreview/lessons.md`, where you
read it at the start of every fix loop (step 1 above).

**When to do a lesson pass.** Only when the user explicitly asks — "review my past
comments", "what patterns do you see", "update the lessons file". Never start one
unprompted, never offer one spontaneously, and never fold one into a routine fix loop.

**What to read**, richest source first:

- `archive/comments-archive.jsonl` — resolved comments, the strongest signal, since
  each one is feedback the reviewer gave and you acted on.
- Every `comments.json` (worktree and each `commits/<sha>/`) — any comment that already
  carries a `response` is part of the same record.
- `comment-log.jsonl` — activity shape only. It logs `remove` actions and drifts out of
  sync with `comments.json`, so use it to spot which files draw repeated attention, not
  to reconstruct what a comment said.

**What counts as a lesson.** A preference that recurs across *distinct* comments and
generalises beyond the one site it was raised at — naming, error handling, test
structure, comment density, API shape. Not a lesson: a one-off fix, anything resting on
a single comment, or a restatement of what the code already makes obvious.

**Review every lesson with the user before writing it.** This step is not optional and
does not depend on how confident you are. Nothing goes into `lessons.md` that the user
has not seen and approved first.

Show the complete list of candidate lessons, each with the evidence behind it — which
comments, how many, quoted where it helps. Then stop and wait. Do not write the file in
the same turn you propose them.

Expect the user to edit the list, not just accept it: they may drop a lesson, reword one,
merge two, or narrow one's scope. Take that as the normal outcome. Where a pattern is
ambiguous or thinly evidenced, ask rather than guess — how far it reaches ("everywhere,
or only in the parser?"), and how firm it is ("hard rule or preference?"). Write only
what survives that pass, in the user's words where they gave them.

A wrong lesson is worse than a missing one: it gets applied silently to every future
change, and nothing in the fix loop will ever re-examine it.

**How to write it.** Append to or update `.turboreview/lessons.md`, one lesson per
bullet, each carrying the evidence behind it (which comments, how many). Preserve
lessons already in the file; never rewrite it wholesale. A lesson pass is read-only over
the review corpus — do not touch any `comments.json` while doing one.

## Rules

- NEVER delete a comment object or change `file`, `line`, `text`, `line_text`,
  `context_*`, `orig_line`, `hunk`, or `debug_snapshot`. Only set `response` and
  `status`. (`debug_snapshot`, when present, is captured runtime state — useful
  context for understanding the comment; leave it untouched.)
- Preserve the JSON array structure and all comments you are not responding to.
- Keep `response` concise (1–3 sentences). It is shown to the reviewer in the TUI.
- The reviewer re-opens turboreview to see your `response` and `status` inline in
  the diff.
- When editing a per-commit comment, write back to the commit-specific
  `comments.json` at `.turboreview/commits/<sha>/comments.json` — not the
  worktree file.
- NEVER stage (`git add`) or commit a change you made in response to a comment. The
  point of turboreview is that a human reviews the diff; an agent that commits its own
  fix has decided the review passed on the reviewer's behalf.
- Leave fixes unstaged in the working tree. That is exactly where the Changes view
  shows them, which is where the reviewer will look.
- Wait for explicit user confirmation before staging or committing. You saying "fixed
  it" is not confirmation; the user saying to commit is.
- NEVER write `lessons.md` without reviewing the candidate lessons with the user first
  and getting approval — however obvious a pattern looks to you.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_doc_contains_required_content() {
        assert!(
            SKILL_DOC.contains("comments.json"),
            "SKILL_DOC must reference comments.json"
        );
        assert!(
            SKILL_DOC.contains("status"),
            "SKILL_DOC must mention status field"
        );
        assert!(
            SKILL_DOC.contains("response"),
            "SKILL_DOC must mention response field"
        );
        assert!(
            SKILL_DOC.contains("open"),
            "SKILL_DOC must mention open status"
        );
        assert!(
            SKILL_DOC.contains("resolved"),
            "SKILL_DOC must mention resolved status"
        );
        assert!(
            SKILL_DOC.contains("wontfix"),
            "SKILL_DOC must mention wontfix status"
        );
        assert!(
            SKILL_DOC.contains("needs_info"),
            "SKILL_DOC must mention needs_info status"
        );
        assert!(
            SKILL_DOC.contains("comment-log"),
            "SKILL_DOC must reference comment-log"
        );
        assert!(
            SKILL_DOC.contains("commits/<sha>"),
            "SKILL_DOC must describe per-commit path layout"
        );
        assert!(
            SKILL_DOC.contains("archive"),
            "SKILL_DOC must mention the archive"
        );
        assert!(
            SKILL_DOC.contains("updated"),
            "SKILL_DOC must mention the updated field"
        );
        assert!(
            SKILL_DOC.contains("lessons.md"),
            "SKILL_DOC must document the lessons file"
        );
        assert!(
            SKILL_DOC.contains("## Watching for new comments"),
            "SKILL_DOC must document the live-review watch loop"
        );
        assert!(
            SKILL_DOC.contains("Re-read `comments.json` immediately before writing it"),
            "SKILL_DOC must warn against clobbering the reviewer's edits"
        );
        assert!(
            SKILL_DOC.contains("## Learning from past reviews"),
            "SKILL_DOC must have the learning section"
        );
        assert!(
            SKILL_DOC.contains("comments-archive.jsonl"),
            "SKILL_DOC must name the archive file"
        );
        assert!(
            SKILL_DOC.contains("git add"),
            "SKILL_DOC must state the do-not-stage rule"
        );
        assert!(
            SKILL_DOC.contains("Review every lesson with the user before writing it"),
            "SKILL_DOC must require reviewing lessons with the user before writing"
        );
    }
}
