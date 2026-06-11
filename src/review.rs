use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// Load the reviewed-file set from `<scope_dir>/reviewed.json`.
/// `scope_dir` is either `<repo_root>/.turboreview` (worktree scope)
/// or `<repo_root>/.turboreview/commits/<sha>` (commit scope).
/// Returns an empty set if the file does not exist.
pub fn load(scope_dir: &Path) -> Result<HashSet<PathBuf>> {
    let file = scope_dir.join("reviewed.json");
    if !file.exists() {
        return Ok(HashSet::new());
    }
    let bytes = std::fs::read(&file)?;
    let list: Vec<PathBuf> = serde_json::from_slice(&bytes)?;
    Ok(list.into_iter().collect())
}

/// Persist the reviewed-file set to `<scope_dir>/reviewed.json`,
/// creating the directory if needed.
pub fn save(scope_dir: &Path, reviewed: &HashSet<PathBuf>) -> Result<()> {
    std::fs::create_dir_all(scope_dir)?;
    let mut list: Vec<&PathBuf> = reviewed.iter().collect();
    list.sort();
    let json = serde_json::to_vec_pretty(&list)?;
    std::fs::write(scope_dir.join("reviewed.json"), json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_missing_returns_empty() {
        let dir = tempdir().unwrap();
        // Pass the scope dir directly (.turboreview subdirectory)
        let scope = dir.path().join(".turboreview");
        let set = load(&scope).unwrap();
        assert!(set.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempdir().unwrap();
        let scope = dir.path().join(".turboreview");
        let mut set = HashSet::new();
        set.insert(PathBuf::from("src/a.rs"));
        set.insert(PathBuf::from("src/b.rs"));
        save(&scope, &set).unwrap();
        assert!(scope.join("reviewed.json").exists());
        let loaded = load(&scope).unwrap();
        assert_eq!(loaded, set);
    }

    #[test]
    fn commit_scope_save_load_round_trips() {
        let dir = tempdir().unwrap();
        let scope = dir
            .path()
            .join(".turboreview")
            .join("commits")
            .join("abc123");
        let mut set = HashSet::new();
        set.insert(PathBuf::from("main.rs"));
        save(&scope, &set).unwrap();
        assert!(scope.join("reviewed.json").exists());
        let loaded = load(&scope).unwrap();
        assert_eq!(loaded, set);
    }
}
