use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// Load the reviewed-file set from `<repo_root>/.turboreview/reviewed.json`.
/// Returns an empty set if the file does not exist.
pub fn load(repo_root: &Path) -> Result<HashSet<PathBuf>> {
    let file = reviewed_path(repo_root);
    if !file.exists() {
        return Ok(HashSet::new());
    }
    let bytes = std::fs::read(&file)?;
    let list: Vec<PathBuf> = serde_json::from_slice(&bytes)?;
    Ok(list.into_iter().collect())
}

/// Persist the reviewed-file set, creating the `.turboreview` dir if needed.
pub fn save(repo_root: &Path, reviewed: &HashSet<PathBuf>) -> Result<()> {
    let dir = repo_root.join(".turboreview");
    std::fs::create_dir_all(&dir)?;
    let mut list: Vec<&PathBuf> = reviewed.iter().collect();
    list.sort();
    let json = serde_json::to_vec_pretty(&list)?;
    std::fs::write(dir.join("reviewed.json"), json)?;
    Ok(())
}

fn reviewed_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".turboreview").join("reviewed.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_missing_returns_empty() {
        let dir = tempdir().unwrap();
        let set = load(dir.path()).unwrap();
        assert!(set.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempdir().unwrap();
        let mut set = HashSet::new();
        set.insert(PathBuf::from("src/a.rs"));
        set.insert(PathBuf::from("src/b.rs"));
        save(dir.path(), &set).unwrap();
        assert!(dir.path().join(".turboreview/reviewed.json").exists());
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded, set);
    }
}
