use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::{Diff, DiffOptions, Repository};

use crate::app::{DiffLine, FileChange, LineKind, Mode, Status};

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
                    None => String::from_utf8_lossy(line.content()).trim_end().to_string(),
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
                self.inner.reset_default(Some(&obj), std::iter::once(path))?;
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
                files.push(FileChange { path, status: map_status(delta.status()) });
            }
        }
        Ok(files)
    }
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

    /// Init a repo, return (tempdir, Repository). Caller writes files.
    fn init_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "t").unwrap();
        cfg.set_str("user.email", "t@t").unwrap();
        (dir, repo)
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
        repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &parents).unwrap();
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
        let lines = repo.diff_for(Path::new("new.txt"), Mode::Unstaged, 3).unwrap();
        let added: Vec<_> = lines.iter().filter(|l| l.kind == LineKind::Add).collect();
        assert_eq!(added.len(), 2);
        assert!(added.iter().any(|l| l.text.contains("line1")));
    }

    #[test]
    fn diff_for_missing_path_is_empty() {
        let (dir, _repo) = init_repo();
        let repo = Repo::discover(dir.path()).unwrap();
        let lines = repo.diff_for(Path::new("missing.txt"), Mode::Unstaged, 3).unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn diff_for_modified_tracked_file_has_add_and_del_lines() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "f.txt", "alpha\nbeta\n");
        fs::write(dir.path().join("f.txt"), "alpha\nGAMMA\n").unwrap();
        let r = Repo::discover(dir.path()).unwrap();
        let lines = r.diff_for(Path::new("f.txt"), Mode::Unstaged, 3).unwrap();
        assert!(lines.iter().any(|l| l.kind == LineKind::Add && l.text.contains("GAMMA")));
        assert!(lines.iter().any(|l| l.kind == LineKind::Del && l.text.contains("beta")));
        assert!(lines.iter().any(|l| l.kind == LineKind::Context && l.text.contains("alpha")));
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
        assert_eq!(std::fs::read_to_string(dir.path().join("f.txt")).unwrap(), "one\ntwo\n");
    }

    #[test]
    fn diff_for_large_context_includes_more_lines() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "f.txt", "a\nb\nc\nd\ne\nf\ng\nh\n");
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\nd\nE\nf\ng\nh\n").unwrap();
        let r = Repo::discover(dir.path()).unwrap();
        let few = r.diff_for(Path::new("f.txt"), Mode::Unstaged, 0).unwrap();
        let many = r.diff_for(Path::new("f.txt"), Mode::Unstaged, 1_000_000).unwrap();
        // full-file view shows every line; zero-context shows only the changed lines (+ hunk header)
        assert!(many.len() > few.len());
        // full view should contain all 8 file lines as context/changed
        let ctx_and_changed = many.iter().filter(|l| l.kind != crate::app::LineKind::Hunk).count();
        assert!(ctx_and_changed >= 8);
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
