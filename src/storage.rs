use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::app::CommentScope;

/// Returns the worktree-scope directory: `<repo_root>/.turboreview`.
pub fn worktree_dir(repo_root: &Path) -> PathBuf {
    repo_root.join(".turboreview")
}

/// Returns the commit-scope directory: `<repo_root>/.turboreview/commits/<sha>`.
pub fn commit_dir(repo_root: &Path, sha: &str) -> PathBuf {
    repo_root.join(".turboreview").join("commits").join(sha)
}

/// The directory holding comments.json / reviewed.json for the given scope.
pub fn scope_dir(repo_root: &Path, scope: &CommentScope) -> PathBuf {
    match scope {
        CommentScope::Worktree => worktree_dir(repo_root),
        CommentScope::Commit(sha) => commit_dir(repo_root, sha),
    }
}

#[derive(Serialize)]
struct LogEntry<'a> {
    path: &'a str,
    line: u32,
    scope: &'a str,
    date: String,
    action: &'a str,
}

/// Append one line to `<repo_root>/.turboreview/comment-log.jsonl`.
/// Best-effort; errors are returned but the caller is expected to ignore them.
pub fn append_comment_log(
    repo_root: &Path,
    path: &Path,
    line: u32,
    scope: &str,
    action: &str,
) -> Result<()> {
    let dir = repo_root.join(".turboreview");
    std::fs::create_dir_all(&dir)?;
    let path_str = path.to_string_lossy();
    let entry = LogEntry {
        path: &path_str,
        line,
        scope,
        date: crate::git::format_date(now_secs()),
        action,
    };
    let mut line_json = serde_json::to_string(&entry)?;
    line_json.push('\n');
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("comment-log.jsonl"))?;
    f.write_all(line_json.as_bytes())?;
    Ok(())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn worktree_dir_is_dot_turboreview() {
        let root = PathBuf::from("/my/repo");
        assert_eq!(worktree_dir(&root), PathBuf::from("/my/repo/.turboreview"));
    }

    #[test]
    fn commit_dir_is_commits_slash_sha() {
        let root = PathBuf::from("/my/repo");
        assert_eq!(
            commit_dir(&root, "abc123"),
            PathBuf::from("/my/repo/.turboreview/commits/abc123")
        );
    }

    #[test]
    fn append_comment_log_writes_two_valid_json_lines() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        append_comment_log(root, Path::new("src/main.rs"), 42, "worktree", "set").unwrap();
        append_comment_log(
            root,
            Path::new("src/lib.rs"),
            10,
            "commit:deadbeef",
            "remove",
        )
        .unwrap();

        let log_path = root.join(".turboreview/comment-log.jsonl");
        assert!(log_path.exists(), "log file should exist");

        let contents = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "should have 2 lines");

        // Parse each line as JSON and verify fields
        let entry1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(entry1["path"], "src/main.rs");
        assert_eq!(entry1["line"], 42);
        assert_eq!(entry1["scope"], "worktree");
        assert_eq!(entry1["action"], "set");
        assert!(
            entry1["date"].as_str().is_some(),
            "date should be a string"
        );

        let entry2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(entry2["path"], "src/lib.rs");
        assert_eq!(entry2["line"], 10);
        assert_eq!(entry2["scope"], "commit:deadbeef");
        assert_eq!(entry2["action"], "remove");
    }
}
