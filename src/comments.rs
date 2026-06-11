use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use anyhow::Result;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CommentStatus {
    #[default]
    Open,
    Resolved,
    Wontfix,
    NeedsInfo,
}

impl CommentStatus {
    pub fn label(&self) -> &'static str {
        match self {
            CommentStatus::Open => "open",
            CommentStatus::Resolved => "resolved",
            CommentStatus::Wontfix => "wontfix",
            CommentStatus::NeedsInfo => "needs-info",
        }
    }
}

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
    #[serde(default)]
    pub status: CommentStatus,       // agent-facing: open | resolved | wontfix | needs_info
    #[serde(default)]
    pub response: Option<String>,    // agent's reply when addressing the comment
}

#[derive(Clone, Debug, Default)]
pub struct Comments {
    pub items: Vec<Comment>,
}

impl Comments {
    /// Load comments from `<scope_dir>/comments.json`.
    /// `scope_dir` is either `<repo_root>/.turboreview` (worktree scope)
    /// or `<repo_root>/.turboreview/commits/<sha>` (commit scope).
    pub fn load(scope_dir: &Path) -> Result<Comments> {
        let file = path_for(scope_dir);
        if !file.exists() {
            return Ok(Comments::default());
        }
        let bytes = std::fs::read(&file)?;
        let items: Vec<Comment> = serde_json::from_slice(&bytes)?;
        Ok(Comments { items })
    }

    /// Save comments to `<scope_dir>/comments.json`, creating the directory if needed.
    pub fn save(&self, scope_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(scope_dir)?;
        let json = serde_json::to_vec_pretty(&self.items)?;
        std::fs::write(scope_dir.join("comments.json"), json)?;
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
                status: CommentStatus::Open,
                response: None,
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

fn path_for(scope_dir: &Path) -> PathBuf {
    scope_dir.join("comments.json")
}

/// Outcome of trying to relocate a comment.
pub struct Relocation {
    pub line: u32,
    pub stale: bool,
}

/// Relocate `comment` against the current file lines `candidates` = slice of (new_lineno, trimmed_text),
/// sorted ascending by line number (caller guarantees sorted).
pub fn relocate(comment: &Comment, candidates: &[(u32, String)]) -> Relocation {
    // Legacy: truly empty line_text (old comments without anchor) — treat as non-stale, unchanged.
    if comment.line_text.is_empty() {
        return Relocation { line: comment.line, stale: false };
    }

    // FIX 2: NUL (\u{0}) is the blank-line marker sentinel. A blank-line anchor matches
    // candidates whose trimmed text is also empty. All other anchors match by exact equality.
    let is_blank_anchor = comment.line_text == "\u{0}";
    let text_matches = |cand_text: &str| -> bool {
        if is_blank_anchor {
            cand_text.is_empty()
        } else {
            cand_text == comment.line_text
        }
    };

    // Find all candidates whose text matches.
    let matches: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter(|(_, (_, text))| text_matches(text))
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
            //
            // FIX 1: Use a TUPLE score (ctx_count, Reverse(distance)) so that ANY context match
            // unconditionally beats a pure-proximity candidate, regardless of distance magnitude.
            //
            // FIX 3: Check context adjacency by new_lineno arithmetic (cand_line ± 1), not
            // slice index (idx ± 1). Since candidates only contain lines with a new_lineno
            // (Del lines excluded), slice neighbors may not be truly source-adjacent.
            // We binary-search the sorted candidates for new_lineno == cand_line ± 1.
            //
            // Helper: find the text at a given new_lineno via binary search (candidates sorted).
            let find_at_lineno = |target_lineno: u32| -> Option<&str> {
                candidates
                    .binary_search_by_key(&target_lineno, |(n, _)| *n)
                    .ok()
                    .map(|i| candidates[i].1.as_str())
            };

            let context_match_count = |idx: usize| -> usize {
                let cand_line = candidates[idx].0;
                let mut ctx = 0usize;

                // Check immediately preceding source line (new_lineno - 1)
                if cand_line > 0 {
                    if let Some(last_before) = comment.context_before.last() {
                        if find_at_lineno(cand_line - 1) == Some(last_before.as_str()) {
                            ctx += 1;
                        }
                    }
                }
                // Check immediately following source line (new_lineno + 1)
                {
                    if let Some(first_after) = comment.context_after.first() {
                        if find_at_lineno(cand_line + 1) == Some(first_after.as_str()) {
                            ctx += 1;
                        }
                    }
                }
                // Check second context_before line (new_lineno - 2)
                if cand_line > 1 && comment.context_before.len() >= 2 {
                    let second_before = &comment.context_before[comment.context_before.len() - 2];
                    if find_at_lineno(cand_line - 2) == Some(second_before.as_str()) {
                        ctx += 1;
                    }
                }
                // Check second context_after line (new_lineno + 2)
                if comment.context_after.len() >= 2 {
                    let second_after = &comment.context_after[1];
                    if find_at_lineno(cand_line + 2) == Some(second_after.as_str()) {
                        ctx += 1;
                    }
                }

                ctx
            };

            let best_idx = matches.iter().copied().max_by_key(|&idx| {
                let cand_line = candidates[idx].0 as i64;
                let distance = (cand_line - comment.orig_line as i64).unsigned_abs();
                let ctx = context_match_count(idx);
                // FIX 1: tuple so context dominates distance unconditionally
                (ctx, std::cmp::Reverse(distance))
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
            status: CommentStatus::Open,
            response: None,
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

    // FIX 1 TDD: context must beat pure proximity even when the context match is far away
    // Specifically: 1 context match at distance 120 must beat 0 context matches at distance 5.
    // Old formula: (1*100) - 120 = -20  vs  (0*100) - 5 = -5  => old picks the WRONG close one.
    // Tuple fix: (1, Reverse(120)) vs (0, Reverse(5)) => primary=ctx wins, so correct far one picked.
    #[test]
    fn relocate_context_beats_far_proximity() {
        // Wrong candidate: very close to orig_line (distance=5), NO context match
        // Correct candidate: far from orig_line (distance=120), WITH one matching context_before line
        // The comment was at line 100
        let comment = make_comment(
            100,
            "fn process()",
            vec!["// header comment"],
            vec![],
            100,
        );
        // Build candidates: same line_text "fn process()" appears at line 105 and line 220
        let candidates = cands(&[
            // Wrong: very close, line 105 (distance=5), unrelated neighbor
            (104, "completely_unrelated"),
            (105, "fn process()"),          // distance=5 from orig 100, but NO context match
            (106, "other_stuff"),
            // Correct: far, line 220 (distance=120), with exact context_before neighbor
            (219, "// header comment"),     // matches context_before last entry
            (220, "fn process()"),          // distance=120 from orig 100, but HAS context match
            (221, "something_after"),
        ]);
        let r = relocate(&comment, &candidates);
        // Should pick line 220 (context match), NOT line 105 (mere proximity)
        // Old formula: (1*100)-120 = -20 vs (0*100)-5 = -5  =>  old picks 105 (WRONG)
        // Tuple fix: (1, Reverse(120)) > (0, Reverse(5))    =>  new picks 220 (CORRECT)
        assert_eq!(r.line, 220, "context match must win over pure proximity even when far");
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

    // FIX 2 TDD: blank-line anchor (NUL sentinel) must relocate or stale, not legacy-skip
    #[test]
    fn relocate_blank_line_anchor_relocates_or_stales() {
        // Subcase A: blank-line comment with a blank candidate near orig with matching context -> relocates
        let comment = make_comment(
            10,
            "\u{0}",                        // NUL = blank-line marker
            vec!["fn foo() {"],
            vec!["let x = 1;"],
            10,
        );
        // Candidates: blank line at 11 with correct context neighbors
        let candidates = cands(&[
            (9,  "fn foo() {"),
            (11, ""),                        // blank line candidate (trimmed text is empty)
            (12, "let x = 1;"),
        ]);
        let r = relocate(&comment, &candidates);
        assert_eq!(r.line, 11, "blank-line anchor should relocate to the blank candidate");
        assert!(!r.stale);

        // Subcase B: no blank candidate anywhere -> stale
        let comment2 = make_comment(10, "\u{0}", vec![], vec![], 10);
        let candidates2 = cands(&[(9, "fn foo() {"), (11, "let x = 1;")]);
        let r2 = relocate(&comment2, &candidates2);
        assert!(r2.stale, "blank-line anchor with no blank candidates must be stale");
    }

    // FIX 3 TDD: context adjacency by new_lineno arithmetic, not slice index
    // The tricky case: the WRONG candidate happens to have the context text as its slice neighbor
    // (because the context line is far from it but happens to be adjacent in the slice), while
    // the CORRECT candidate has the context text truly adjacent by new_lineno.
    #[test]
    fn relocate_context_uses_new_lineno_not_slice_index() {
        // Comment: orig_line=100, line_text="}", context_before=["let x = 1;"]
        // Candidates include two "}" lines.
        //
        // Wrong candidate at line 50:
        //   - slice[idx-1] = (48, "let x = 1;")   <- slice-adjacent and matches context!
        //   - BUT new_lineno 50-1=49 is ABSENT (Del line), so the true source predecessor of line 50
        //     is line 48 with gap (lines 49 skipped). By new_lineno arithmetic, the immediately
        //     adjacent source line at new_lineno==49 is missing, so no true adjacency.
        //
        // Correct candidate at line 100:
        //   - new_lineno 99 = "let x = 1;" (truly adjacent by source line number)
        //   - slice[idx-1] = also (99, "let x = 1;") - coincidentally also correct
        //
        // Actually we want slice to be WRONG on the wrong candidate. Let me set up:
        // Candidates list:
        //   (48, "let x = 1;")  <- present in slice
        //   (50, "}")           <- slice[idx-1]=(48,"let x = 1;") MATCHES context; new_lineno 49 absent
        //   (99, "let x = 1;") <- new_lineno 99, truly adjacent to line 100
        //   (100, "}")          <- new_lineno-1=99 present and matches; also slice[idx-1] matches
        //   (101, "return;")
        //
        // In this layout, BOTH candidates score equally with either approach.
        // The distinguishing case needs the wrong candidate's slice[idx-1] to match
        // but its new_lineno-1 NOT to match (because the match is non-adjacent by lineno).
        //
        // Setup: wrong candidate at line 200, correct at line 100.
        // In the slice: wrong candidate's slice[idx-1] has the context text BUT is at new_lineno 150
        //   (which is NOT adjacent to 200 by source lines).
        // Correct candidate's new_lineno-1 = 99, which exists in candidates as "let x = 1;".
        //
        // Slice order: (99,"let x=1;"), (100,"}"), ..., (150,"let x=1;"), ..., (200,"}")
        //   For wrong candidate (200): slice[idx-1] = (150,"let x=1;") -> MATCHES context by slice!
        //     But new_lineno 200-1=199 is absent -> no match by new_lineno.
        //   For correct candidate (100): slice[idx-1] = (99,"let x=1;") -> matches
        //     new_lineno 100-1=99 present -> also matches.
        //
        // Old (slice-based) behavior: both get ctx_count=1, tie-break by proximity to orig_line=100
        //   -> picks correct line 100 by proximity (distance 0). Actually that would work!
        //
        // For slice-based to pick WRONG, we need orig_line closer to wrong candidate.
        // Set orig_line=200, correct at 100 (far), wrong at 205 (close, distance=5).
        // Slice-based: wrong gets ctx=1 (slice match), correct gets ctx=1 (slice match).
        //   Tie-break by proximity: wrong (distance=5) beats correct (distance=100).
        // New_lineno-based: wrong gets ctx=0 (lineno 204 absent), correct gets ctx=1.
        //   Correct wins regardless of distance.
        let comment = make_comment(200, "}", vec!["let x = 1;"], vec![], 200);
        let candidates = cands(&[
            (99,  "let x = 1;"),   // new_lineno 99, truly adjacent to line 100
            (100, "}"),             // correct: new_lineno-1=99 has "let x=1;"; slice[idx-1]=(99,"let x=1;")
            (101, "other_stuff"),
            // Gap here: linenos 102..149 absent
            (150, "let x = 1;"),   // present in slice; new_lineno 150 (adjacent to nothing meaningful)
            // Gap here: linenos 151..204 absent (line 204 is a Del line, not in candidates)
            (205, "}"),             // wrong: distance=5 from orig 200;
                                    // slice[idx-1]=(150,"let x=1;") -> matches by slice!
                                    // but new_lineno 205-1=204 is ABSENT -> no match by new_lineno
            (206, "other"),
        ]);
        let r = relocate(&comment, &candidates);
        // new_lineno-based: correct candidate (100) gets ctx=1, wrong (205) gets ctx=0
        // => must pick line 100 even though distance=100 (far) vs distance=5 (close)
        assert_eq!(r.line, 100, "new_lineno adjacency must pick the truly-adjacent context match");
        assert!(!r.stale);
    }

    // ─── Original tests (updated to new 7-arg set signature) ─────────────────

    #[test]
    fn load_missing_returns_empty() {
        let dir = tempdir().unwrap();
        let scope = dir.path().join(".turboreview");
        let comments = Comments::load(&scope).unwrap();
        assert!(comments.items.is_empty());
    }

    #[test]
    fn set_save_load_round_trip() {
        let dir = tempdir().unwrap();
        let scope = dir.path().join(".turboreview");
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
        comments.save(&scope).unwrap();
        assert!(scope.join("comments.json").exists());

        let loaded = Comments::load(&scope).unwrap();
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

    // ─── TDD: status + response round-trip ───────────────────────────────────

    #[test]
    fn comment_status_response_round_trips_through_save_load() {
        let dir = tempdir().unwrap();
        let scope = dir.path().join(".turboreview");
        let mut comments = Comments::default();
        comments.set(
            PathBuf::from("src/main.rs"),
            10,
            "@@ -8,4 @@".to_string(),
            "refactor this".to_string(),
            "fn foo()".to_string(),
            vec![],
            vec![],
        );
        // Manually set status and response on the created comment
        comments.items[0].status = CommentStatus::Resolved;
        comments.items[0].response = Some("Fixed in latest commit".to_string());
        comments.save(&scope).unwrap();

        let loaded = Comments::load(&scope).unwrap();
        assert_eq!(loaded.items.len(), 1);
        let c = &loaded.items[0];
        assert_eq!(c.status, CommentStatus::Resolved);
        assert_eq!(c.response, Some("Fixed in latest commit".to_string()));
    }

    #[test]
    fn old_json_without_status_response_deserializes_with_defaults() {
        // Simulate a JSON file written by an older version of turboreview
        // that has no status or response fields.
        let old_json = r#"[
            {
                "file": "src/lib.rs",
                "line": 5,
                "hunk": "@@ -3,4 +3,6 @@",
                "text": "needs refactor",
                "line_text": "fn bar()",
                "context_before": [],
                "context_after": [],
                "orig_line": 5,
                "stale": false
            }
        ]"#;
        let items: Vec<Comment> = serde_json::from_str(old_json).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, CommentStatus::Open);
        assert_eq!(items[0].response, None);
    }
}
