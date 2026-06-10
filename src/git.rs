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

    fn build_diff(&self, mode: Mode) -> Result<Diff<'_>> {
        let mut opts = DiffOptions::new();
        opts.include_untracked(true).recurse_untracked_dirs(true);
        let diff = match mode {
            Mode::Unstaged => self.inner.diff_index_to_workdir(None, Some(&mut opts))?,
            Mode::Staged => {
                let head_tree = match self.inner.head() {
                    Ok(head) => Some(head.peel_to_tree()?),
                    Err(_) => None, // unborn HEAD: compare empty tree -> index
                };
                self.inner
                    .diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts))?
            }
        };
        Ok(diff)
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
        Added | Untracked => Status::Added,
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
}
