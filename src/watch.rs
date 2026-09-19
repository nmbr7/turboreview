//! `turboreview --watch`: stream review comments as they are written, for an
//! agent that stays running alongside the reviewer.
//!
//! turboreview and the agent both poll the same `comments.json` files — there is
//! no daemon and no socket, and this module is the agent's half of that. It
//! prints one JSON object per line to stdout as comments appear and change, so
//! the consumer can read a line at a time without parsing prose.
//!
//! What is emitted (see [`Event`]):
//!   - `new`     — a comment appeared with status `open`
//!   - `edited`  — the reviewer changed the text of a comment
//!   - `reopened`— a comment we answered went back to `open`
//!
//! Comments the *agent itself* resolves are deliberately not emitted: the point
//! is to surface work to do, and echoing back your own writes is noise.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use serde::Serialize;

use crate::app::CommentScope;
use crate::comments::{Comment, CommentStatus, Comments};

/// How often to stat the comment files. The reviewer is typing by hand, so
/// sub-second latency buys nothing and a tighter loop just burns syscalls.
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Why a comment is being emitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// First time we have seen this comment, and it is open.
    New,
    /// The reviewer changed the comment's text.
    Edited,
    /// A comment that had been answered is open again — treat the previous
    /// response as rejected.
    Reopened,
}

/// One line of `--watch` output.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Event {
    pub kind: EventKind,
    /// `"worktree"` or `"commit:<sha>"` — which store this comment lives in.
    /// The agent must write its reply back to this same scope.
    pub scope: String,
    /// Path of the `comments.json` to write the reply into.
    pub path: PathBuf,
    pub file: PathBuf,
    /// Current line number; may have moved since the comment was created.
    pub line: u32,
    /// Line number at creation — the stable key for this comment.
    pub orig_line: u32,
    pub text: String,
    /// The source line the comment is anchored to, trimmed.
    pub line_text: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
    pub hunk: String,
    /// True when turboreview could not confidently relocate the comment, so
    /// `line` is a guess and `line_text` + context are the reliable anchor.
    pub stale: bool,
    /// Previous response, when this is a `reopened` event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response: Option<String>,
}

/// The stable identity of a comment across polls: its scope, the file it is on,
/// and where it was created.
///
/// `line` moves when the code above it changes, so it cannot be part of the key.
/// Scope must be, because the worktree and each commit are independent stores —
/// without it, `a.rs:1` reviewed on a commit collides with `a.rs:1` in the
/// worktree and each poll reports the other one as an edit.
type Key = (String, PathBuf, u32);

/// What we last saw for one comment, enough to classify the next change.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    text: String,
    status: CommentStatus,
}

/// Tracks comments across polls and decides what to emit.
///
/// Split out from the I/O so the transition rules can be tested directly.
#[derive(Default)]
pub struct Watcher {
    seen: HashMap<Key, Seen>,
}

impl Watcher {
    pub fn new() -> Watcher {
        Watcher::default()
    }

    /// Record the current state without emitting anything.
    ///
    /// Called once at startup so a watcher attaching to a repo with existing
    /// open comments does not replay the whole backlog as if it were new. Use
    /// `--replay` to emit them instead.
    pub fn prime(&mut self, scope: &str, comments: &[Comment]) {
        for c in comments {
            self.seen.insert(
                (scope.to_string(), c.file.clone(), c.orig_line),
                Seen {
                    text: c.text.clone(),
                    status: c.status,
                },
            );
        }
    }

    /// Classify one scope's comments against what we saw last time.
    ///
    /// Emits at most one event per comment per poll. A comment that is not open
    /// is still recorded, so a later reopen is recognised.
    pub fn step(&mut self, scope: &str, path: &Path, comments: &[Comment]) -> Vec<Event> {
        let mut out = Vec::new();
        for c in comments {
            let key = (scope.to_string(), c.file.clone(), c.orig_line);
            let now = Seen {
                text: c.text.clone(),
                status: c.status,
            };
            let kind = match self.seen.get(&key) {
                // Never seen: only interesting if it wants an answer.
                None => (c.status == CommentStatus::Open).then_some(EventKind::New),
                Some(prev) => {
                    if prev.text != c.text {
                        // Reworded. Emit whatever the new status is, because the
                        // reviewer changed what they are asking for.
                        Some(EventKind::Edited)
                    } else if prev.status != CommentStatus::Open && c.status == CommentStatus::Open
                    {
                        Some(EventKind::Reopened)
                    } else {
                        // Includes the agent's own resolve: status moved away
                        // from open, which is not work to do.
                        None
                    }
                }
            };
            self.seen.insert(key, now);
            if let Some(kind) = kind {
                out.push(Event {
                    kind,
                    scope: scope.to_string(),
                    path: path.to_path_buf(),
                    file: c.file.clone(),
                    line: c.line,
                    orig_line: c.orig_line,
                    text: c.text.clone(),
                    line_text: c.line_text.clone(),
                    context_before: c.context_before.clone(),
                    context_after: c.context_after.clone(),
                    hunk: c.hunk.clone(),
                    stale: c.stale,
                    previous_response: (kind == EventKind::Reopened)
                        .then(|| c.response.clone())
                        .flatten(),
                });
            }
        }
        out
    }
}

/// Every comment store under `.turboreview/`: the worktree scope plus one per
/// reviewed commit.
///
/// Rescanned on each poll rather than listed once, so a commit scope created
/// while the watcher is running is picked up without a restart.
pub fn scopes(repo_root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = vec![(
        "worktree".to_string(),
        crate::storage::scope_dir(repo_root, &CommentScope::Worktree).join("comments.json"),
    )];
    let commits = repo_root.join(".turboreview").join("commits");
    let Ok(entries) = std::fs::read_dir(&commits) else {
        return out;
    };
    let mut shas: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    // Stable order so output is reproducible across runs.
    shas.sort();
    for sha in shas {
        let path = commits.join(&sha).join("comments.json");
        if path.exists() {
            out.push((format!("commit:{sha}"), path));
        }
    }
    out
}

/// Load one scope's comments, or `None` if the file is missing or mid-write.
///
/// A partial write is a normal occurrence here — the reviewer's save and our
/// read are not synchronised — so a parse failure is not an error, just a
/// "try again next poll".
fn load_scope(path: &Path) -> Option<Vec<Comment>> {
    let dir = path.parent()?;
    Comments::load(dir).ok().map(|c| c.items)
}

/// Run the watch loop until stdout closes (the consumer went away) or the
/// process is killed.
///
/// `replay` emits currently-open comments on startup instead of priming past
/// them, for an agent picking up a review already in progress.
pub fn run(repo_root: &Path, replay: bool, mut out: impl Write) -> Result<()> {
    let mut watcher = Watcher::new();
    let mut fingerprints: HashMap<PathBuf, Option<(i64, u32, u64)>> = HashMap::new();

    if !replay {
        // Adopt the current state silently so we only report what happens next.
        for (scope, path) in scopes(repo_root) {
            if let Some(items) = load_scope(&path) {
                watcher.prime(&scope, &items);
            }
            fingerprints.insert(path.clone(), fingerprint(&path));
        }
    }

    loop {
        for (scope, path) in scopes(repo_root) {
            let current = fingerprint(&path);
            // `get` returns None for a path we have never stat'd, which is
            // distinct from a stored None (file absent) — so compare the
            // Option<Option<_>> directly rather than flattening.
            if fingerprints.get(&path) == Some(&current) {
                continue;
            }
            let Some(items) = load_scope(&path) else {
                // Mid-write: leave the fingerprint so we retry next tick.
                continue;
            };
            fingerprints.insert(path.clone(), current);
            for event in watcher.step(&scope, &path, &items) {
                // A serialisation failure here would be a bug in our own types,
                // not bad input, so surface it rather than silently dropping.
                writeln!(out, "{}", serde_json::to_string(&event)?)?;
            }
            // Flush per scope so a consumer reading line-by-line sees events as
            // they happen rather than when a block buffer happens to fill.
            out.flush()?;
        }
        // Note: a closed consumer is only noticed on the next real write, since
        // an empty write returns Ok(0) without touching the fd and a flush with
        // an empty buffer is a no-op. So `--watch | head -1` keeps running until
        // the next comment change rather than exiting at once. Harmless for the
        // intended use (an agent reads continuously and exits on its own), and
        // the alternative — emitting filler bytes to probe the pipe — would
        // corrupt the JSON-lines contract.
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// `(mtime_secs, mtime_nanos, len)` for a comments file, or `None` if absent.
fn fingerprint(path: &Path) -> Option<(i64, u32, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let (secs, nanos) = match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_nanos()),
        Err(e) => {
            let d = e.duration();
            (-(d.as_secs() as i64), d.subsec_nanos())
        }
    };
    Some((secs, nanos, meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmt(file: &str, orig_line: u32, text: &str, status: CommentStatus) -> Comment {
        Comment {
            file: PathBuf::from(file),
            line: orig_line,
            hunk: "@@ -1,2 +1,2 @@".into(),
            text: text.into(),
            line_text: "let x = 1;".into(),
            context_before: vec![],
            context_after: vec![],
            orig_line,
            stale: false,
            status,
            response: None,
            updated: 0,
            debug_snapshot: None,
        }
    }

    #[test]
    fn emits_a_new_open_comment_once() {
        let mut w = Watcher::new();
        let items = vec![cmt("a.rs", 1, "fix this", CommentStatus::Open)];
        let first = w.step("worktree", Path::new("p"), &items);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind, EventKind::New);
        assert_eq!(first[0].text, "fix this");
        // Unchanged on the next poll: nothing more to say.
        assert!(w.step("worktree", Path::new("p"), &items).is_empty());
    }

    #[test]
    fn does_not_emit_a_comment_that_is_already_answered() {
        let mut w = Watcher::new();
        let items = vec![cmt("a.rs", 1, "fix this", CommentStatus::Resolved)];
        assert!(w.step("worktree", Path::new("p"), &items).is_empty());
    }

    #[test]
    fn does_not_echo_the_agents_own_resolve() {
        let mut w = Watcher::new();
        let open = vec![cmt("a.rs", 1, "fix this", CommentStatus::Open)];
        assert_eq!(w.step("worktree", Path::new("p"), &open).len(), 1);
        // The agent answers it; that write must not come back as work to do.
        let resolved = vec![cmt("a.rs", 1, "fix this", CommentStatus::Resolved)];
        assert!(w.step("worktree", Path::new("p"), &resolved).is_empty());
    }

    #[test]
    fn emits_an_edit_to_an_existing_comment() {
        let mut w = Watcher::new();
        let before = vec![cmt("a.rs", 1, "fix this", CommentStatus::Open)];
        w.step("worktree", Path::new("p"), &before);
        let after = vec![cmt(
            "a.rs",
            1,
            "actually, fix it this way",
            CommentStatus::Open,
        )];
        let ev = w.step("worktree", Path::new("p"), &after);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, EventKind::Edited);
        assert_eq!(ev[0].text, "actually, fix it this way");
    }

    #[test]
    fn emits_an_edit_even_after_the_agent_resolved_it() {
        let mut w = Watcher::new();
        let open = vec![cmt("a.rs", 1, "fix this", CommentStatus::Open)];
        w.step("worktree", Path::new("p"), &open);
        let resolved = vec![cmt("a.rs", 1, "fix this", CommentStatus::Resolved)];
        w.step("worktree", Path::new("p"), &resolved);
        // Reviewer rewords it while it is still marked resolved.
        let reworded = vec![cmt(
            "a.rs",
            1,
            "no, do it differently",
            CommentStatus::Resolved,
        )];
        let ev = w.step("worktree", Path::new("p"), &reworded);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, EventKind::Edited);
    }

    #[test]
    fn emits_a_reopen_with_the_previous_response() {
        let mut w = Watcher::new();
        let open = vec![cmt("a.rs", 1, "fix this", CommentStatus::Open)];
        w.step("worktree", Path::new("p"), &open);
        let mut answered = cmt("a.rs", 1, "fix this", CommentStatus::Resolved);
        answered.response = Some("did it".into());
        w.step("worktree", Path::new("p"), &[answered]);
        // Reviewer rejects the fix by reopening.
        let mut reopened = cmt("a.rs", 1, "fix this", CommentStatus::Open);
        reopened.response = Some("did it".into());
        let ev = w.step("worktree", Path::new("p"), &[reopened]);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, EventKind::Reopened);
        assert_eq!(ev[0].previous_response.as_deref(), Some("did it"));
    }

    #[test]
    fn a_relocated_comment_is_not_a_new_one() {
        let mut w = Watcher::new();
        let before = vec![cmt("a.rs", 5, "fix this", CommentStatus::Open)];
        w.step("worktree", Path::new("p"), &before);
        // The agent's edit shifted the code; `line` moved but orig_line anchors it.
        let mut moved = cmt("a.rs", 5, "fix this", CommentStatus::Open);
        moved.line = 42;
        assert!(
            w.step("worktree", Path::new("p"), &[moved]).is_empty(),
            "a comment that merely moved must not be reported as new"
        );
    }

    /// Regression: the worktree and each commit are independent stores, so the
    /// same file:line in two scopes is two different comments. Keying without
    /// the scope made each poll report the other scope's comment as an edit,
    /// and made the agent's own resolve echo back as work to do.
    #[test]
    fn the_same_line_in_two_scopes_does_not_collide() {
        let mut w = Watcher::new();
        let wt = vec![cmt("m.rs", 1, "worktree comment", CommentStatus::Open)];
        let ct = vec![cmt("m.rs", 1, "commit comment", CommentStatus::Open)];

        let a = w.step("worktree", Path::new("p1"), &wt);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].kind, EventKind::New);

        // Same file and line, different scope: still a new comment, not an edit.
        let b = w.step("commit:abc", Path::new("p2"), &ct);
        assert_eq!(b.len(), 1);
        assert_eq!(
            b[0].kind,
            EventKind::New,
            "a different scope is a different comment"
        );

        // And the agent resolving the worktree one must stay silent even though
        // the commit scope holds the same file:line.
        let resolved = vec![cmt("m.rs", 1, "worktree comment", CommentStatus::Resolved)];
        assert!(w.step("worktree", Path::new("p1"), &resolved).is_empty());
    }

    #[test]
    fn the_same_line_in_two_files_is_two_comments() {
        let mut w = Watcher::new();
        let items = vec![
            cmt("a.rs", 1, "fix a", CommentStatus::Open),
            cmt("b.rs", 1, "fix b", CommentStatus::Open),
        ];
        assert_eq!(w.step("worktree", Path::new("p"), &items).len(), 2);
    }

    #[test]
    fn prime_suppresses_the_existing_backlog() {
        let mut w = Watcher::new();
        let items = vec![cmt("a.rs", 1, "old comment", CommentStatus::Open)];
        w.prime("worktree", &items);
        assert!(
            w.step("worktree", Path::new("p"), &items).is_empty(),
            "priming must adopt existing comments silently"
        );
        // But a genuinely new one still comes through.
        let mut more = items.clone();
        more.push(cmt("a.rs", 9, "new comment", CommentStatus::Open));
        let ev = w.step("worktree", Path::new("p"), &more);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].orig_line, 9);
    }

    #[test]
    fn event_serialises_as_one_json_line() {
        let mut w = Watcher::new();
        let items = vec![cmt("src/a.rs", 3, "explain this", CommentStatus::Open)];
        let ev = w.step("commit:abc123", Path::new("/r/.turboreview/x.json"), &items);
        let line = serde_json::to_string(&ev[0]).unwrap();
        assert!(
            !line.contains('\n'),
            "an event must occupy exactly one line"
        );
        let back: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(back["kind"], "new");
        assert_eq!(back["scope"], "commit:abc123");
        assert_eq!(back["file"], "src/a.rs");
        assert_eq!(back["text"], "explain this");
        assert_eq!(back["orig_line"], 3);
        // previous_response is absent unless this is a reopen.
        assert!(back.get("previous_response").is_none());
    }

    #[test]
    fn scopes_lists_the_worktree_then_each_commit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let tr = root.join(".turboreview");
        std::fs::create_dir_all(tr.join("commits").join("bbb")).unwrap();
        std::fs::create_dir_all(tr.join("commits").join("aaa")).unwrap();
        std::fs::write(tr.join("commits").join("aaa").join("comments.json"), "[]").unwrap();
        std::fs::write(tr.join("commits").join("bbb").join("comments.json"), "[]").unwrap();

        let found = scopes(root);
        assert_eq!(found[0].0, "worktree");
        // Sorted, so output ordering is stable run to run.
        assert_eq!(found[1].0, "commit:aaa");
        assert_eq!(found[2].0, "commit:bbb");
    }

    #[test]
    fn scopes_skips_a_commit_dir_with_no_comments_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".turboreview").join("commits").join("empty")).unwrap();
        let found = scopes(root);
        assert_eq!(found.len(), 1, "only the worktree scope should be listed");
    }
}
