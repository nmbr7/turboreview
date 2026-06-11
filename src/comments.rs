use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use anyhow::Result;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Comment {
    pub file: PathBuf,
    pub line: u32,        // the diff line's new_lineno (post-image line number)
    pub hunk: String,     // hunk header for context (e.g. "@@ -1,4 +1,8 @@")
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Comments {
    pub items: Vec<Comment>,
}

impl Comments {
    pub fn load(repo_root: &Path) -> Result<Comments> {
        let file = path_for(repo_root);
        if !file.exists() {
            return Ok(Comments::default());
        }
        let bytes = std::fs::read(&file)?;
        let items: Vec<Comment> = serde_json::from_slice(&bytes)?;
        Ok(Comments { items })
    }

    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let dir = repo_root.join(".turboreview");
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_vec_pretty(&self.items)?;
        std::fs::write(dir.join("comments.json"), json)?;
        Ok(())
    }

    /// Add or replace the comment for (file, line).
    pub fn set(&mut self, file: PathBuf, line: u32, hunk: String, text: String) {
        if let Some(c) = self.items.iter_mut().find(|c| c.file == file && c.line == line) {
            c.text = text;
            c.hunk = hunk;
        } else {
            self.items.push(Comment { file, line, hunk, text });
        }
    }

    /// Remove a comment for (file, line) if present.
    pub fn remove(&mut self, file: &Path, line: u32) {
        self.items.retain(|c| !(c.file == file && c.line == line));
    }

    pub fn get(&self, file: &Path, line: u32) -> Option<&Comment> {
        self.items.iter().find(|c| c.file == file && c.line == line)
    }
}

fn path_for(repo_root: &Path) -> PathBuf {
    repo_root.join(".turboreview").join("comments.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_missing_returns_empty() {
        let dir = tempdir().unwrap();
        let comments = Comments::load(dir.path()).unwrap();
        assert!(comments.items.is_empty());
    }

    #[test]
    fn set_save_load_round_trip() {
        let dir = tempdir().unwrap();
        let mut comments = Comments::default();
        comments.set(
            PathBuf::from("src/main.rs"),
            42,
            "@@ -40,4 +40,6 @@".to_string(),
            "This needs refactoring".to_string(),
        );
        comments.save(dir.path()).unwrap();
        assert!(dir.path().join(".turboreview/comments.json").exists());

        let loaded = Comments::load(dir.path()).unwrap();
        assert_eq!(loaded.items.len(), 1);
        let c = &loaded.items[0];
        assert_eq!(c.file, PathBuf::from("src/main.rs"));
        assert_eq!(c.line, 42);
        assert_eq!(c.hunk, "@@ -40,4 +40,6 @@");
        assert_eq!(c.text, "This needs refactoring");
    }

    #[test]
    fn set_replaces_existing_comment_for_same_file_and_line() {
        let mut comments = Comments::default();
        comments.set(
            PathBuf::from("a.rs"),
            10,
            "@@ -8,4 +8,6 @@".to_string(),
            "original text".to_string(),
        );
        assert_eq!(comments.items.len(), 1);

        // Replace same (file, line)
        comments.set(
            PathBuf::from("a.rs"),
            10,
            "@@ -8,4 +8,6 @@".to_string(),
            "updated text".to_string(),
        );
        assert_eq!(comments.items.len(), 1);
        assert_eq!(comments.items[0].text, "updated text");
    }

    #[test]
    fn remove_deletes_comment() {
        let mut comments = Comments::default();
        comments.set(PathBuf::from("a.rs"), 5, "@@".to_string(), "note".to_string());
        comments.set(PathBuf::from("b.rs"), 7, "@@".to_string(), "other".to_string());
        assert_eq!(comments.items.len(), 2);

        comments.remove(Path::new("a.rs"), 5);
        assert_eq!(comments.items.len(), 1);
        assert_eq!(comments.items[0].file, PathBuf::from("b.rs"));
    }

    #[test]
    fn get_finds_comment_by_file_and_line() {
        let mut comments = Comments::default();
        comments.set(PathBuf::from("src/lib.rs"), 99, "@@ -95,4 @@".to_string(), "look here".to_string());
        comments.set(PathBuf::from("src/main.rs"), 5, "@@ -3,4 @@".to_string(), "other".to_string());

        let found = comments.get(Path::new("src/lib.rs"), 99);
        assert!(found.is_some());
        assert_eq!(found.unwrap().text, "look here");

        // Wrong line number
        assert!(comments.get(Path::new("src/lib.rs"), 100).is_none());
        // Wrong file
        assert!(comments.get(Path::new("src/other.rs"), 99).is_none());
    }
}
