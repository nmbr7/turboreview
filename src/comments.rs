use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use anyhow::Result;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Comment {
    pub file: PathBuf,
    pub line: u32,        // current best line (post-relocation)
    pub hunk: String,     // hunk header for context (e.g. "@@ -1,4 +1,8 @@")
    pub text: String,
    #[serde(default)]
    pub line_text: String,           // the commented line's content (trimmed of trailing ws)
    #[serde(default)]
    pub context_before: Vec<String>, // up to 2 lines before (content)
    #[serde(default)]
    pub context_after: Vec<String>,  // up to 2 lines after (content)
    #[serde(default)]
    pub orig_line: u32,              // line number when the comment was created
    #[serde(default)]
    pub stale: bool,                 // true if relocation couldn't confidently place it
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
    pub fn set(&mut self, file: PathBuf, line: u32, hunk: String, text: String,
               line_text: String, context_before: Vec<String>, context_after: Vec<String>) {
        if let Some(c) = self.items.iter_mut().find(|c| c.file == file && c.line == line) {
            c.text = text;
            c.hunk = hunk;
            c.line_text = line_text;
            c.context_before = context_before;
            c.context_after = context_after;
            c.orig_line = line;
            c.stale = false;
        } else {
            self.items.push(Comment {
                file,
                line,
                hunk,
                text,
                line_text,
                context_before,
                context_after,
                orig_line: line,
                stale: false,
            });
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

/// Outcome of trying to relocate a comment.
pub struct Relocation {
    pub line: u32,
    pub stale: bool,
}

/// Relocate `comment` against the current file lines `candidates` = slice of (new_lineno, trimmed_text),
/// sorted ascending by line number (caller guarantees sorted).
pub fn relocate(comment: &Comment, candidates: &[(u32, String)]) -> Relocation {
    // Legacy: empty line_text means no anchor — treat as non-stale, unchanged.
    if comment.line_text.is_empty() {
        return Relocation { line: comment.line, stale: false };
    }

    // Find all candidates whose text matches exactly.
    let matches: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter(|(_, (_, text))| text == &comment.line_text)
        .map(|(i, _)| i)
        .collect();

    match matches.len() {
        0 => {
            // No match — stale, keep stored line.
            Relocation { line: comment.line, stale: true }
        }
        1 => {
            // Exactly one — unambiguous.
            Relocation { line: candidates[matches[0]].0, stale: false }
        }
        _ => {
            // Multiple matches: score by context + proximity.
            // score = (context_match_count * 100) - distance_from_orig_line
            // context_match_count = # of context_before/context_after lines that appear
            // adjacent to the candidate in candidates.
            let best_idx = matches.iter().copied().max_by_key(|&idx| {
                let cand_line = candidates[idx].0 as i64;
                let distance = (cand_line - comment.orig_line as i64).unsigned_abs() as i64;

                let mut ctx_count = 0i64;
                // Check immediately preceding candidate vs last of context_before
                if idx > 0 {
                    if let Some(last_before) = comment.context_before.last() {
                        if &candidates[idx - 1].1 == last_before {
                            ctx_count += 1;
                        }
                    }
                }
                // Check immediately following candidate vs first of context_after
                if idx + 1 < candidates.len() {
                    if let Some(first_after) = comment.context_after.first() {
                        if &candidates[idx + 1].1 == first_after {
                            ctx_count += 1;
                        }
                    }
                }
                // Also check the second context_before line (two steps back)
                if idx > 1 {
                    if comment.context_before.len() >= 2 {
                        let second_before = &comment.context_before[comment.context_before.len() - 2];
                        if &candidates[idx - 2].1 == second_before {
                            ctx_count += 1;
                        }
                    }
                }
                // Also check the second context_after line (two steps forward)
                if idx + 2 < candidates.len() {
                    if comment.context_after.len() >= 2 {
                        let second_after = &comment.context_after[1];
                        if &candidates[idx + 2].1 == second_after {
                            ctx_count += 1;
                        }
                    }
                }

                (ctx_count * 100) - distance
            }).expect("matches non-empty");

            Relocation { line: candidates[best_idx].0, stale: false }
        }
    }
}

impl Comments {
    pub fn relocate_file(&mut self, file: &Path, candidates: &[(u32, String)]) {
        for c in self.items.iter_mut().filter(|c| c.file == file) {
            let r = relocate(c, candidates);
            c.line = r.line;
            c.stale = r.stale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ─── Relocation tests (TDD - written first) ───────────────────────────────

    fn make_comment(line: u32, line_text: &str, ctx_before: Vec<&str>, ctx_after: Vec<&str>, orig_line: u32) -> Comment {
        Comment {
            file: PathBuf::from("a.rs"),
            line,
            hunk: "@@".to_string(),
            text: "note".to_string(),
            line_text: line_text.to_string(),
            context_before: ctx_before.iter().map(|s| s.to_string()).collect(),
            context_after: ctx_after.iter().map(|s| s.to_string()).collect(),
            orig_line,
            stale: false,
        }
    }

    fn cands(pairs: &[(u32, &str)]) -> Vec<(u32, String)> {
        pairs.iter().map(|(n, s)| (*n, s.to_string())).collect()
    }

    #[test]
    fn relocate_exact_single_match() {
        let comment = make_comment(5, "foo", vec![], vec![], 5);
        let candidates = cands(&[(3, "bar"), (7, "foo"), (10, "baz")]);
        let r = relocate(&comment, &candidates);
        assert_eq!(r.line, 7);
        assert!(!r.stale);
    }

    #[test]
    fn relocate_no_match_is_stale() {
        let comment = make_comment(5, "foo", vec![], vec![], 5);
        let candidates = cands(&[(3, "bar"), (7, "baz")]);
        let r = relocate(&comment, &candidates);
        assert!(r.stale);
        assert_eq!(r.line, 5); // unchanged
    }

    #[test]
    fn relocate_shifted_down() {
        // comment was at line 5 with text "foo"; now "foo" appears at line 8 (shifted +3)
        let comment = make_comment(5, "foo", vec![], vec![], 5);
        let candidates = cands(&[(1, "hello"), (4, "world"), (8, "foo"), (12, "end")]);
        let r = relocate(&comment, &candidates);
        assert_eq!(r.line, 8);
        assert!(!r.stale);
    }

    #[test]
    fn relocate_duplicate_lines_uses_context_and_proximity() {
        // Two candidates both have "}" - one near orig_line 10 with matching context_before,
        // one far away at line 50 without matching context.
        let comment = make_comment(10, "}", vec!["let x = 1;"], vec!["return x;"], 10);
        // near match: line 11 with correct context neighbors
        // far match: line 50 with unrelated neighbors
        let candidates = cands(&[
            (9, "let x = 1;"),
            (11, "}"),
            (12, "return x;"),
            (48, "something_else"),
            (50, "}"),
            (51, "other_stuff"),
        ]);
        let r = relocate(&comment, &candidates);
        assert_eq!(r.line, 11);
        assert!(!r.stale);
    }

    #[test]
    fn relocate_legacy_no_anchor_keeps_line() {
        // comment with empty line_text -> non-stale, line unchanged
        let comment = make_comment(15, "", vec![], vec![], 15);
        let candidates = cands(&[(10, "foo"), (20, "bar")]);
        let r = relocate(&comment, &candidates);
        assert!(!r.stale);
        assert_eq!(r.line, 15);
    }

    #[test]
    fn relocate_empty_candidates_is_stale() {
        let comment = make_comment(5, "foo", vec![], vec![], 5);
        let r = relocate(&comment, &[]);
        assert!(r.stale);
        assert_eq!(r.line, 5);
    }

    // ─── Original tests (updated to new 7-arg set signature) ─────────────────

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
            "fn main() {".to_string(),
            vec![],
            vec![],
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
            "let x = 1;".to_string(),
            vec![],
            vec![],
        );
        assert_eq!(comments.items.len(), 1);

        // Replace same (file, line)
        comments.set(
            PathBuf::from("a.rs"),
            10,
            "@@ -8,4 +8,6 @@".to_string(),
            "updated text".to_string(),
            "let x = 1;".to_string(),
            vec![],
            vec![],
        );
        assert_eq!(comments.items.len(), 1);
        assert_eq!(comments.items[0].text, "updated text");
    }

    #[test]
    fn remove_deletes_comment() {
        let mut comments = Comments::default();
        comments.set(PathBuf::from("a.rs"), 5, "@@".to_string(), "note".to_string(), String::new(), vec![], vec![]);
        comments.set(PathBuf::from("b.rs"), 7, "@@".to_string(), "other".to_string(), String::new(), vec![], vec![]);
        assert_eq!(comments.items.len(), 2);

        comments.remove(Path::new("a.rs"), 5);
        assert_eq!(comments.items.len(), 1);
        assert_eq!(comments.items[0].file, PathBuf::from("b.rs"));
    }

    #[test]
    fn get_finds_comment_by_file_and_line() {
        let mut comments = Comments::default();
        comments.set(PathBuf::from("src/lib.rs"), 99, "@@ -95,4 @@".to_string(), "look here".to_string(), String::new(), vec![], vec![]);
        comments.set(PathBuf::from("src/main.rs"), 5, "@@ -3,4 @@".to_string(), "other".to_string(), String::new(), vec![], vec![]);

        let found = comments.get(Path::new("src/lib.rs"), 99);
        assert!(found.is_some());
        assert_eq!(found.unwrap().text, "look here");

        // Wrong line number
        assert!(comments.get(Path::new("src/lib.rs"), 100).is_none());
        // Wrong file
        assert!(comments.get(Path::new("src/other.rs"), 99).is_none());
    }
}
