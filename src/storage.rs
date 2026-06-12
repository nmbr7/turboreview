use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

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

/// Persisted configuration (theme preference).
#[derive(Serialize, Deserialize, Default)]
struct Config {
    theme: String, // "dark" | "light"
}

/// Load the persisted theme from `<repo_root>/.turboreview/config.json`.
/// Returns `Theme::Dark` if the file is missing or cannot be parsed.
pub fn load_theme(repo_root: &Path) -> crate::theme::Theme {
    let path = worktree_dir(repo_root).join("config.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return crate::theme::Theme::Dark;
    };
    let cfg: Config = serde_json::from_slice(&bytes).unwrap_or_default();
    match cfg.theme.as_str() {
        "light" => crate::theme::Theme::Light,
        _ => crate::theme::Theme::Dark,
    }
}

/// Persist the current theme to `<repo_root>/.turboreview/config.json`.
pub fn save_theme(repo_root: &Path, theme: crate::theme::Theme) -> Result<()> {
    let dir = worktree_dir(repo_root);
    std::fs::create_dir_all(&dir)?;
    let cfg = Config {
        theme: match theme {
            crate::theme::Theme::Light => "light".into(),
            _ => "dark".into(),
        },
    };
    std::fs::write(dir.join("config.json"), serde_json::to_vec_pretty(&cfg)?)?;
    Ok(())
}

const ARCHIVE_DAYS: i64 = 14;

/// Returns the archive file path: `<repo_root>/.turboreview/archive/comments-archive.jsonl`.
pub fn archive_path(repo_root: &Path) -> PathBuf {
    worktree_dir(repo_root).join("archive").join("comments-archive.jsonl")
}

/// Append archived comments as JSON lines to the archive file. Best-effort; errors returned.
pub fn append_archive(repo_root: &Path, comments: &[crate::comments::Comment]) -> anyhow::Result<()> {
    if comments.is_empty() {
        return Ok(());
    }
    let path = archive_path(repo_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    for c in comments {
        let mut line = serde_json::to_string(c)?;
        line.push('\n');
        f.write_all(line.as_bytes())?;
    }
    Ok(())
}

/// Returns the cutoff unix epoch seconds for auto-archiving: `now - ARCHIVE_DAYS * 86400`.
pub fn archive_cutoff_secs(now: i64) -> i64 {
    now - ARCHIVE_DAYS * 86400
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
        date: crate::git::format_datetime(now_secs()),
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

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ─── TDD: archive_path, append_archive, archive_cutoff_secs ──────────────

    #[test]
    fn archive_path_is_under_dot_turboreview_archive() {
        let root = PathBuf::from("/my/repo");
        let p = archive_path(&root);
        assert_eq!(
            p,
            PathBuf::from("/my/repo/.turboreview/archive/comments-archive.jsonl")
        );
    }

    #[test]
    fn append_archive_writes_one_json_line_per_comment() {
        use crate::comments::{Comment, CommentStatus};
        let dir = tempdir().unwrap();
        let root = dir.path();

        let comments = vec![
            Comment {
                file: std::path::PathBuf::from("a.rs"),
                line: 1,
                hunk: "@@".to_string(),
                text: "note one".to_string(),
                line_text: "fn a()".to_string(),
                context_before: vec![],
                context_after: vec![],
                orig_line: 1,
                stale: false,
                status: CommentStatus::Resolved,
                response: None,
                updated: 1000,
            },
            Comment {
                file: std::path::PathBuf::from("b.rs"),
                line: 5,
                hunk: "@@".to_string(),
                text: "note two".to_string(),
                line_text: "fn b()".to_string(),
                context_before: vec![],
                context_after: vec![],
                orig_line: 5,
                stale: false,
                status: CommentStatus::Resolved,
                response: None,
                updated: 2000,
            },
        ];

        append_archive(root, &comments).unwrap();

        let archive = archive_path(root);
        assert!(archive.exists(), "archive file must exist");
        let contents = std::fs::read_to_string(&archive).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "must write one line per comment");

        // Each line must be valid JSON and contain the comment text
        let v1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v1["text"], "note one");
        let v2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(v2["text"], "note two");
    }

    #[test]
    fn append_archive_is_append_only() {
        use crate::comments::{Comment, CommentStatus};
        let dir = tempdir().unwrap();
        let root = dir.path();

        let c1 = Comment {
            file: std::path::PathBuf::from("a.rs"),
            line: 1,
            hunk: "@@".to_string(),
            text: "first".to_string(),
            line_text: "".to_string(),
            context_before: vec![],
            context_after: vec![],
            orig_line: 1,
            stale: false,
            status: CommentStatus::Resolved,
            response: None,
            updated: 100,
        };
        let c2 = Comment {
            file: std::path::PathBuf::from("b.rs"),
            line: 2,
            hunk: "@@".to_string(),
            text: "second".to_string(),
            line_text: "".to_string(),
            context_before: vec![],
            context_after: vec![],
            orig_line: 2,
            stale: false,
            status: CommentStatus::Resolved,
            response: None,
            updated: 200,
        };

        append_archive(root, &[c1]).unwrap();
        append_archive(root, &[c2]).unwrap();

        let contents = std::fs::read_to_string(archive_path(root)).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "second call must append, not overwrite");
        let v1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v1["text"], "first");
        let v2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(v2["text"], "second");
    }

    #[test]
    fn append_archive_errors_when_path_unwritable() {
        use crate::comments::{Comment, CommentStatus};
        let dir = tempdir().unwrap();
        // Create a FILE where the .turboreview dir should be, so create_dir_all fails.
        std::fs::write(dir.path().join(".turboreview"), b"x").unwrap();
        let c = Comment {
            file: std::path::PathBuf::from("a.rs"),
            line: 1,
            hunk: "@@".to_string(),
            text: "test".to_string(),
            line_text: "fn a()".to_string(),
            context_before: vec![],
            context_after: vec![],
            orig_line: 1,
            stale: false,
            status: CommentStatus::Resolved,
            response: None,
            updated: 1000,
        };
        let res = append_archive(dir.path(), &[c]);
        assert!(res.is_err(), "append_archive must error when the dir can't be created");
    }

    #[test]
    fn append_archive_empty_slice_does_nothing() {
        use crate::comments::Comment;
        let dir = tempdir().unwrap();
        let root = dir.path();
        append_archive(root, &[] as &[Comment]).unwrap();
        assert!(!archive_path(root).exists(), "empty slice must not create archive file");
    }

    #[test]
    fn archive_cutoff_secs_is_14_days_before_now() {
        let now = 1_000_000_i64;
        let cutoff = archive_cutoff_secs(now);
        assert_eq!(cutoff, now - 14 * 86400);
    }

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
    fn save_then_load_theme_round_trips_light() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        save_theme(root, crate::theme::Theme::Light).unwrap();
        let loaded = load_theme(root);
        assert_eq!(loaded, crate::theme::Theme::Light);
    }

    #[test]
    fn save_then_load_theme_round_trips_dark() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        save_theme(root, crate::theme::Theme::Dark).unwrap();
        let loaded = load_theme(root);
        assert_eq!(loaded, crate::theme::Theme::Dark);
    }

    #[test]
    fn load_theme_missing_file_returns_dark() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        // No config.json created — should default to Dark
        let loaded = load_theme(root);
        assert_eq!(loaded, crate::theme::Theme::Dark);
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
        let date_str = entry1["date"].as_str().expect("date should be a string");
        // date now holds YYYY-MM-DD HH:MM:SS (19 chars)
        assert_eq!(
            date_str.len(),
            19,
            "date field must be YYYY-MM-DD HH:MM:SS (19 chars): {}",
            date_str
        );
        assert!(
            date_str.contains(' '),
            "date field must contain a space separating date and time"
        );
        assert_eq!(
            date_str.chars().filter(|&c| c == ':').count(),
            2,
            "date field must have two colons for HH:MM:SS"
        );

        let entry2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(entry2["path"], "src/lib.rs");
        assert_eq!(entry2["line"], 10);
        assert_eq!(entry2["scope"], "commit:deadbeef");
        assert_eq!(entry2["action"], "remove");
    }
}
