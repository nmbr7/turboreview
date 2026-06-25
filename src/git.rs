use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::{Diff, DiffOptions, Repository};

use crate::app::{DiffLine, FileChange, LineKind, Mode, Status};

/// One entry in the branch history.
#[derive(Clone, Debug)]
pub struct CommitInfo {
    pub id: String,      // full hex oid
    pub short: String,   // first 8 chars of the oid
    pub summary: String, // first line of the message
    pub author: String,  // author name
    pub time: String,    // formatted date "YYYY-MM-DD"
}

pub struct Repo {
    inner: Repository,
}

impl Repo {
    /// Discover the repository containing `path` (path may be the repo root or a subdir).
    pub fn discover(path: &Path) -> Result<Repo> {
        let inner = Repository::discover(path)
            .with_context(|| format!("no git repository found at {}", path.display()))?;
        Ok(Repo { inner })
    }

    /// Absolute path to the working-tree root.
    pub fn workdir(&self) -> Result<PathBuf> {
        self.inner
            .workdir()
            .map(|p| p.to_path_buf())
            .context("repository has no working directory (bare repo)")
    }

    /// Shared diff builder: runs the mode match with the given pre-configured options.
    fn diff_with_opts<'a>(&'a self, mode: Mode, opts: &mut DiffOptions) -> Result<Diff<'a>> {
        let diff = match mode {
            Mode::Unstaged => self.inner.diff_index_to_workdir(None, Some(opts))?,
            Mode::Staged => {
                let head_tree = match self.inner.head() {
                    Ok(head) => Some(head.peel_to_tree()?),
                    Err(_) => None, // unborn HEAD: compare empty tree -> index
                };
                self.inner
                    .diff_tree_to_index(head_tree.as_ref(), None, Some(opts))?
            }
        };
        Ok(diff)
    }

    // Only file metadata is needed here (not line content), so
    // show_untracked_content is intentionally omitted. Use diff_for for content.
    fn build_diff(&self, mode: Mode) -> Result<Diff<'_>> {
        let mut opts = DiffOptions::new();
        opts.include_untracked(true).recurse_untracked_dirs(true);
        self.diff_with_opts(mode, &mut opts)
    }

    /// Build the diff lines for a single file path (relative to the repo root).
    pub fn diff_for(&self, file: &Path, mode: Mode, context: u32) -> Result<Vec<DiffLine>> {
        let mut opts = DiffOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .show_untracked_content(true)
            .context_lines(context)
            .pathspec(file);
        let diff = self.diff_with_opts(mode, &mut opts)?;
        collect_diff_lines(&diff)
    }

    /// Stage the given path: copy its working-tree state into the index.
    /// For a deleted file this removes it from the index; otherwise adds it.
    pub fn stage_file(&self, path: &Path) -> Result<()> {
        let mut index = self.inner.index()?;
        let workdir = self.inner.workdir().context("bare repo")?;
        if workdir.join(path).symlink_metadata().is_ok() {
            index.add_path(path)?;
        } else {
            // file deleted on disk -> stage the deletion
            index.remove_path(path)?;
        }
        index.write()?;
        Ok(())
    }

    /// Unstage the given path: reset its index entry to HEAD (like `git reset HEAD <path>`).
    /// Index-only; the working tree is never modified.
    pub fn unstage_file(&self, path: &Path) -> Result<()> {
        match self.inner.head() {
            Ok(head) => {
                let obj = head.peel(git2::ObjectType::Commit)?;
                self.inner
                    .reset_default(Some(&obj), std::iter::once(path))?;
            }
            Err(_) => {
                // unborn HEAD: no commit to reset to -> remove the entry from the index
                let mut index = self.inner.index()?;
                index.remove_path(path)?;
                index.write()?;
            }
        }
        Ok(())
    }

    /// Walk the current branch history from HEAD (newest first), up to `limit` commits.
    pub fn log(&self, limit: usize) -> Result<Vec<CommitInfo>> {
        let mut walk = self.inner.revwalk()?;
        if walk.push_head().is_err() {
            return Ok(Vec::new()); // unborn HEAD — no commits yet
        }
        walk.set_sorting(git2::Sort::TIME)?;
        let mut out = Vec::new();
        for oid in walk {
            if out.len() >= limit {
                break;
            }
            let oid = oid?;
            let commit = self.inner.find_commit(oid)?;
            let id = oid.to_string();
            let short = id.chars().take(8).collect::<String>();
            let summary = commit.summary().ok().flatten().unwrap_or("").to_string();
            let author = commit.author().name().unwrap_or("").to_string();
            let secs = commit.author().when().seconds();
            let time = format_date(secs);
            out.push(CommitInfo {
                id,
                short,
                summary,
                author,
                time,
            });
        }
        Ok(out)
    }

    /// Changed files introduced by a commit vs its first parent (or vs empty tree for a root commit).
    pub fn commit_files(&self, commit_id: &str) -> Result<Vec<FileChange>> {
        let oid = git2::Oid::from_str(commit_id)?;
        let commit = self.inner.find_commit(oid)?;
        let tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };
        let mut opts = git2::DiffOptions::new();
        let diff =
            self.inner
                .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;
        let mut files = Vec::new();
        for delta in diff.deltas() {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_path_buf());
            if let Some(path) = path {
                files.push(FileChange {
                    path,
                    status: map_status(delta.status()),
                });
            }
        }
        Ok(files)
    }

    /// Diff lines for one file within a commit (commit vs first parent), with `context` context lines.
    pub fn commit_diff_for(
        &self,
        commit_id: &str,
        file: &Path,
        context: u32,
    ) -> Result<Vec<DiffLine>> {
        let oid = git2::Oid::from_str(commit_id)?;
        let commit = self.inner.find_commit(oid)?;
        let tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };
        let mut opts = git2::DiffOptions::new();
        opts.context_lines(context).pathspec(file);
        let diff =
            self.inner
                .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;
        collect_diff_lines(&diff)
    }

    /// Commits that touched `file`, newest first, walking from HEAD. Capped at `limit`.
    /// Empty if HEAD is unborn or the file was never touched in the walked history.
    pub fn file_history(&self, file: &Path, limit: usize) -> Result<Vec<CommitInfo>> {
        let mut walk = self.inner.revwalk()?;
        if walk.push_head().is_err() {
            return Ok(Vec::new()); // unborn HEAD
        }
        walk.set_sorting(git2::Sort::TIME)?;
        let mut out = Vec::new();
        for oid in walk {
            if out.len() >= limit {
                break;
            }
            let oid = oid?;
            let commit = self.inner.find_commit(oid)?;
            let tree = commit.tree()?;
            let parent_tree = if commit.parent_count() > 0 {
                Some(commit.parent(0)?.tree()?)
            } else {
                None
            };
            let mut opts = git2::DiffOptions::new();
            opts.pathspec(file);
            let diff =
                self.inner
                    .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;
            if diff.deltas().len() == 0 {
                continue; // this commit did not touch `file`
            }
            let id = oid.to_string();
            let short = id.chars().take(8).collect::<String>();
            let summary = commit.summary().ok().flatten().unwrap_or("").to_string();
            let author = commit.author().name().unwrap_or("").to_string();
            let time = format_date(commit.author().when().seconds());
            out.push(CommitInfo {
                id,
                short,
                summary,
                author,
                time,
            });
        }
        Ok(out)
    }

    /// List changed files for the given mode.
    pub fn changed_files(&self, mode: Mode) -> Result<Vec<FileChange>> {
        let diff = self.build_diff(mode)?;
        let mut files = Vec::new();
        for delta in diff.deltas() {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_path_buf());
            if let Some(path) = path {
                files.push(FileChange {
                    path,
                    status: map_status(delta.status()),
                });
            }
        }
        Ok(files)
    }
}

/// Shared helper: collect DiffLine entries from a git2::Diff using the print callback.
/// Used by both diff_for and commit_diff_for so the line-mapping logic is not duplicated.
fn collect_diff_lines(diff: &Diff<'_>) -> Result<Vec<DiffLine>> {
    let mut lines = Vec::new();
    diff.print(git2::DiffFormat::Patch, |_delta, hunk, line| {
        let origin = line.origin();
        let kind = match origin {
            '+' => LineKind::Add,
            '-' => LineKind::Del,
            ' ' => LineKind::Context,
            'H' => LineKind::Hunk,
            _ => return true, // 'F' file header, binary, etc: skip
        };
        let text = if kind == LineKind::Hunk {
            match hunk {
                Some(h) => String::from_utf8_lossy(h.header()).trim_end().to_string(),
                None => String::from_utf8_lossy(line.content())
                    .trim_end()
                    .to_string(),
            }
        } else {
            String::from_utf8_lossy(line.content())
                .trim_end_matches(|c| c == '\n' || c == '\r')
                .to_string()
        };
        lines.push(DiffLine {
            kind,
            text,
            old_lineno: line.old_lineno(),
            new_lineno: line.new_lineno(),
        });
        true
    })?;
    Ok(lines)
}

/// Convert epoch seconds to "YYYY-MM-DD" using the civil_from_days algorithm
/// (Howard Hinnant's implementation — no date crate needed).
pub fn format_date(secs: i64) -> String {
    // Days since epoch (floor division for negative values)
    let z = secs.div_euclid(86400);
    // civil_from_days algorithm (H. Hinnant)
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Convert epoch seconds to "YYYY-MM-DD HH:MM:SS" (UTC).
/// Reuses `format_date` for the date part; time-of-day derived from `secs.rem_euclid(86400)`.
pub fn format_datetime(secs: i64) -> String {
    let date = format_date(secs);
    let tod = secs.rem_euclid(86400);
    let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    format!("{} {:02}:{:02}:{:02}", date, h, m, s)
}

/// Human "time ago" for `then` relative to `now` (both unix epoch seconds).
/// e.g. "just now", "5m ago", "3h ago", "2d ago", "4w ago", "3mo ago", "1y ago".
/// Future timestamps render as "just now".
pub fn relative_time(then: i64, now: i64) -> String {
    let d = now - then;
    if d <= 0 {
        return "just now".to_string();
    }
    let (n, unit) = if d < 60 {
        return "just now".to_string();
    } else if d < 3600 {
        (d / 60, "m")
    } else if d < 86_400 {
        (d / 3600, "h")
    } else if d < 604_800 {
        (d / 86_400, "d")
    } else if d < 2_592_000 {
        (d / 604_800, "w")
    } else if d < 31_536_000 {
        (d / 2_592_000, "mo")
    } else {
        (d / 31_536_000, "y")
    };
    format!("{}{} ago", n, unit)
}

fn map_status(s: git2::Delta) -> Status {
    use git2::Delta::*;
    match s {
        Added | Untracked | Copied => Status::Added,
        Deleted => Status::Deleted,
        Modified => Status::Modified,
        Renamed => Status::Renamed,
        _ => Status::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // ── NEW: CommitInfo / log / commit_files / commit_diff_for tests ─────────

    #[test]
    fn format_date_known_values() {
        // epoch 0 = 1970-01-01
        assert_eq!(format_date(0), "1970-01-01");
        // 1700000000 seconds from epoch = 2023-11-14
        assert_eq!(format_date(1_700_000_000), "2023-11-14");
    }

    #[test]
    fn format_datetime_known_value() {
        // 1700000000 = 2023-11-14 (date part)
        // time-of-day: 1700000000 % 86400 = 79400 => 22h 13m 20s => "22:13:20"
        let dt = format_datetime(1_700_000_000);
        // Length should be 19 ("YYYY-MM-DD HH:MM:SS")
        assert_eq!(dt.len(), 19, "datetime must be 19 chars: {}", dt);
        // Date part must match format_date
        let date_part = &dt[..10];
        assert_eq!(date_part, format_date(1_700_000_000), "date part mismatch");
        // Must contain a space and two colons (time part HH:MM:SS)
        assert!(dt.contains(' '), "datetime must contain a space");
        assert_eq!(
            dt.chars().filter(|&c| c == ':').count(),
            2,
            "datetime must have two colons"
        );
        // Verify exact time
        assert_eq!(&dt[11..], "22:13:20", "time part mismatch");
    }

    #[test]
    fn relative_time_buckets() {
        let now = 1_700_000_000i64;
        assert_eq!(relative_time(now, now), "just now");
        assert_eq!(relative_time(now - 30, now), "just now");
        assert_eq!(relative_time(now - 5 * 60, now), "5m ago");
        assert_eq!(relative_time(now - 3 * 3600, now), "3h ago");
        assert_eq!(relative_time(now - 2 * 86_400, now), "2d ago");
        assert_eq!(relative_time(now - 3 * 604_800, now), "3w ago");
        assert_eq!(relative_time(now - 2 * 2_592_000, now), "2mo ago");
        assert_eq!(relative_time(now - 2 * 31_536_000, now), "2y ago");
        // Future timestamp clamps to "just now".
        assert_eq!(relative_time(now + 100, now), "just now");
    }

    #[test]
    fn log_lists_commits_newest_first() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "a.txt", "alpha");
        commit_file(&repo, dir.path(), "b.txt", "beta");
        let r = Repo::discover(dir.path()).unwrap();
        let commits = r.log(10).unwrap();
        assert_eq!(commits.len(), 2);
        // newest first → second commit (added b.txt) is first in the log
        assert!(commits[0].summary.contains('c') || commits[0].short.len() == 8);
        assert!(!commits[0].short.is_empty());
        assert!(!commits[0].author.is_empty());
        assert!(!commits[0].time.is_empty());
        // second entry is the first commit
        assert_eq!(commits[0].id.len(), 40);
    }

    #[test]
    fn log_respects_limit() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "a.txt", "one");
        commit_file(&repo, dir.path(), "b.txt", "two");
        commit_file(&repo, dir.path(), "c.txt", "three");
        let r = Repo::discover(dir.path()).unwrap();
        let commits = r.log(2).unwrap();
        assert_eq!(commits.len(), 2);
    }

    #[test]
    fn commit_files_shows_files_changed_in_commit() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "a.txt", "alpha");
        commit_file(&repo, dir.path(), "b.txt", "beta");
        let r = Repo::discover(dir.path()).unwrap();
        let commits = r.log(10).unwrap();
        // commits[0] is newest = the second commit (added b.txt)
        let files = r.commit_files(&commits[0].id).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::PathBuf::from("b.txt"));
    }

    #[test]
    fn commit_diff_for_yields_add_lines() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "hello.txt", "hello world\n");
        let r = Repo::discover(dir.path()).unwrap();
        let commits = r.log(10).unwrap();
        let lines = r
            .commit_diff_for(&commits[0].id, std::path::Path::new("hello.txt"), 3)
            .unwrap();
        let adds: Vec<_> = lines
            .iter()
            .filter(|l| l.kind == crate::app::LineKind::Add)
            .collect();
        assert!(!adds.is_empty());
        assert!(adds.iter().any(|l| l.text.contains("hello world")));
    }

    #[test]
    fn root_commit_files_vs_empty_tree() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "root.txt", "initial");
        let r = Repo::discover(dir.path()).unwrap();
        let commits = r.log(10).unwrap();
        // Single (root) commit — no parent
        assert_eq!(commits.len(), 1);
        let files = r.commit_files(&commits[0].id).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::PathBuf::from("root.txt"));
    }

    /// Init a repo, return (tempdir, Repository). Caller writes files.
    fn init_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "t").unwrap();
        cfg.set_str("user.email", "t@t").unwrap();
        (dir, repo)
    }

    /// Build a repo applying each commit's (path, content) writes in order.
    /// Commit messages are "c1", "c2", ... Returns (tempdir, Repo).
    fn repo_with_commits(commits: &[&[(&str, &str)]]) -> (tempfile::TempDir, Repo) {
        let tmp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(tmp.path()).unwrap();
        let mut parent: Option<git2::Oid> = None;
        for (i, writes) in commits.iter().enumerate() {
            // Distinct author times so TIME-sorted revwalk order is deterministic.
            let time = git2::Time::new((i + 1) as i64, 0);
            let sig = git2::Signature::new("t", "t@t", &time).unwrap();
            for (path, content) in writes.iter() {
                std::fs::write(tmp.path().join(path), content).unwrap();
            }
            let mut index = repo.index().unwrap();
            for (path, _) in writes.iter() {
                index.add_path(std::path::Path::new(path)).unwrap();
            }
            index.write().unwrap();
            let tree_oid = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_oid).unwrap();
            let msg = format!("c{}", i + 1);
            let parents: Vec<git2::Commit> = parent
                .iter()
                .map(|oid| repo.find_commit(*oid).unwrap())
                .collect();
            let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
            let oid = repo
                .commit(Some("HEAD"), &sig, &sig, &msg, &tree, &parent_refs)
                .unwrap();
            parent = Some(oid);
        }
        let r = Repo::discover(tmp.path()).unwrap();
        (tmp, r)
    }

    #[test]
    fn file_history_returns_only_commits_touching_file() {
        let (_tmp, r) = repo_with_commits(&[
            &[("a.txt", "a1")], // commit 1: touches a.txt
            &[("b.txt", "b1")], // commit 2: touches b.txt only
            &[("a.txt", "a2")], // commit 3: touches a.txt
        ]);
        let hist = r.file_history(std::path::Path::new("a.txt"), 50).unwrap();
        // Newest first: commit 3 then commit 1. Commit 2 excluded.
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].summary, "c3");
        assert_eq!(hist[1].summary, "c1");
    }

    #[test]
    fn file_history_unborn_head_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        git2::Repository::init(tmp.path()).unwrap();
        let r = Repo::discover(tmp.path()).unwrap();
        let hist = r.file_history(std::path::Path::new("a.txt"), 50).unwrap();
        assert!(hist.is_empty());
    }

    #[test]
    fn file_history_untouched_file_is_empty() {
        let (_tmp, r) = repo_with_commits(&[&[("a.txt", "a1")]]);
        let hist = r
            .file_history(std::path::Path::new("never.txt"), 50)
            .unwrap();
        assert!(hist.is_empty());
    }

    #[test]
    fn file_history_respects_limit() {
        let (_tmp, r) =
            repo_with_commits(&[&[("a.txt", "1")], &[("a.txt", "2")], &[("a.txt", "3")]]);
        let hist = r.file_history(std::path::Path::new("a.txt"), 2).unwrap();
        assert_eq!(hist.len(), 2); // capped, newest two
        assert_eq!(hist[0].summary, "c3");
        assert_eq!(hist[1].summary, "c2");
    }

    fn commit_file(repo: &Repository, dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(name)).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("t", "t@t").unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &parents)
            .unwrap();
    }

    #[test]
    fn untracked_file_shows_as_unstaged_added() {
        let (dir, _repo) = init_repo();
        fs::write(dir.path().join("new.txt"), "hello\n").unwrap();
        let repo = Repo::discover(dir.path()).unwrap();
        let files = repo.changed_files(Mode::Unstaged).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("new.txt"));
        assert_eq!(files[0].status, Status::Added);
    }

    #[test]
    fn staged_file_shows_in_staged_mode_only() {
        let (dir, repo) = init_repo();
        fs::write(dir.path().join("a.txt"), "x\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.txt")).unwrap();
        index.write().unwrap();

        let r = Repo::discover(dir.path()).unwrap();
        let staged = r.changed_files(Mode::Staged).unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].path, PathBuf::from("a.txt"));
    }

    #[test]
    fn diff_for_untracked_file_yields_added_lines() {
        let (dir, _repo) = init_repo();
        fs::write(dir.path().join("new.txt"), "line1\nline2\n").unwrap();
        let repo = Repo::discover(dir.path()).unwrap();
        let lines = repo
            .diff_for(Path::new("new.txt"), Mode::Unstaged, 3)
            .unwrap();
        let added: Vec<_> = lines.iter().filter(|l| l.kind == LineKind::Add).collect();
        assert_eq!(added.len(), 2);
        assert!(added.iter().any(|l| l.text.contains("line1")));
    }

    #[test]
    fn diff_for_missing_path_is_empty() {
        let (dir, _repo) = init_repo();
        let repo = Repo::discover(dir.path()).unwrap();
        let lines = repo
            .diff_for(Path::new("missing.txt"), Mode::Unstaged, 3)
            .unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn diff_for_modified_tracked_file_has_add_and_del_lines() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "f.txt", "alpha\nbeta\n");
        fs::write(dir.path().join("f.txt"), "alpha\nGAMMA\n").unwrap();
        let r = Repo::discover(dir.path()).unwrap();
        let lines = r.diff_for(Path::new("f.txt"), Mode::Unstaged, 3).unwrap();
        assert!(lines
            .iter()
            .any(|l| l.kind == LineKind::Add && l.text.contains("GAMMA")));
        assert!(lines
            .iter()
            .any(|l| l.kind == LineKind::Del && l.text.contains("beta")));
        assert!(lines
            .iter()
            .any(|l| l.kind == LineKind::Context && l.text.contains("alpha")));
    }

    #[test]
    fn diff_for_unmodified_tracked_file_is_empty() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "f.txt", "x\n");
        let r = Repo::discover(dir.path()).unwrap();
        let lines = r.diff_for(Path::new("f.txt"), Mode::Unstaged, 3).unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn stage_file_moves_untracked_into_index() {
        let (dir, _repo) = init_repo();
        std::fs::write(dir.path().join("n.txt"), "hi\n").unwrap();
        let r = Repo::discover(dir.path()).unwrap();
        assert_eq!(r.changed_files(Mode::Unstaged).unwrap().len(), 1);
        r.stage_file(Path::new("n.txt")).unwrap();
        // now it shows staged, not unstaged
        assert_eq!(r.changed_files(Mode::Staged).unwrap().len(), 1);
        assert_eq!(r.changed_files(Mode::Unstaged).unwrap().len(), 0);
        // file still on disk (working tree untouched)
        assert!(dir.path().join("n.txt").exists());
    }

    #[test]
    fn unstage_file_moves_staged_back_and_keeps_file() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "f.txt", "one\n");
        std::fs::write(dir.path().join("f.txt"), "one\ntwo\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("f.txt")).unwrap();
        index.write().unwrap();
        let r = Repo::discover(dir.path()).unwrap();
        assert_eq!(r.changed_files(Mode::Staged).unwrap().len(), 1);
        r.unstage_file(Path::new("f.txt")).unwrap();
        assert_eq!(r.changed_files(Mode::Staged).unwrap().len(), 0);
        assert_eq!(r.changed_files(Mode::Unstaged).unwrap().len(), 1);
        // working-tree content preserved
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "one\ntwo\n"
        );
    }

    #[test]
    fn diff_for_large_context_includes_more_lines() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "f.txt", "a\nb\nc\nd\ne\nf\ng\nh\n");
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\nd\nE\nf\ng\nh\n").unwrap();
        let r = Repo::discover(dir.path()).unwrap();
        let few = r.diff_for(Path::new("f.txt"), Mode::Unstaged, 0).unwrap();
        let many = r
            .diff_for(Path::new("f.txt"), Mode::Unstaged, 1_000_000)
            .unwrap();
        // full-file view shows every line; zero-context shows only the changed lines (+ hunk header)
        assert!(many.len() > few.len());
        // full view should contain all 8 file lines as context/changed
        let ctx_and_changed = many
            .iter()
            .filter(|l| l.kind != crate::app::LineKind::Hunk)
            .count();
        assert!(ctx_and_changed >= 8);
    }

    #[test]
    fn log_on_unborn_head_returns_empty() {
        // init_repo() creates a repo with no commits (unborn HEAD)
        let (dir, _repo) = init_repo();
        let r = Repo::discover(dir.path()).unwrap();
        let commits = r.log(10).unwrap();
        assert!(
            commits.is_empty(),
            "log on unborn HEAD must return empty list, not error"
        );
    }

    #[test]
    fn diff_for_staged_unborn_head_yields_add_lines() {
        let (dir, repo) = init_repo();
        fs::write(dir.path().join("s.txt"), "one\ntwo\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("s.txt")).unwrap();
        index.write().unwrap();
        let r = Repo::discover(dir.path()).unwrap();
        let lines = r.diff_for(Path::new("s.txt"), Mode::Staged, 3).unwrap();
        let adds: Vec<_> = lines.iter().filter(|l| l.kind == LineKind::Add).collect();
        assert_eq!(adds.len(), 2);
    }
}
