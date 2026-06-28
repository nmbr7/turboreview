//! LCOV coverage parsing for the diff coverage-highlight feature.
//!
//! We only need per-line hit counts. LCOV records we care about:
//! - `SF:<path>`        — start of a file's record (source file path)
//! - `DA:<line>,<hits>` — a line's execution count
//! - `end_of_record`    — end of the current file's record
//!
//! All other records (FN/FNDA/BRDA/LF/LH/…) are ignored.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parsed coverage: source file (as written in the LCOV `SF:` line) → line
/// number → execution count. A line present with count 0 is *uncovered*; a line
/// absent has no coverage data (e.g. a blank line or non-executable).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Coverage {
    files: HashMap<PathBuf, HashMap<u32, u64>>,
}

/// Coverage state of a single source line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCov {
    /// Executed at least once.
    Covered,
    /// Instrumented but never executed.
    Uncovered,
    /// No coverage data for this line.
    None,
}

impl Coverage {
    /// Parse LCOV text into a [`Coverage`]. Lenient: malformed `DA:` lines are
    /// skipped rather than failing the whole parse.
    pub fn parse_lcov(text: &str) -> Coverage {
        let mut files = HashMap::new();
        let mut cur_path: Option<PathBuf> = None;
        let mut cur: HashMap<u32, u64> = HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if let Some(p) = line.strip_prefix("SF:") {
                // Starting a new record; flush any previous (defensive).
                if let Some(path) = cur_path.take() {
                    merge(&mut files, path, std::mem::take(&mut cur));
                }
                cur_path = Some(PathBuf::from(p.trim()));
            } else if let Some(da) = line.strip_prefix("DA:") {
                if let Some((ln, hits)) = parse_da(da) {
                    // Keep the max hit count if a line appears more than once.
                    let e = cur.entry(ln).or_insert(0);
                    *e = (*e).max(hits);
                }
            } else if line == "end_of_record" {
                if let Some(path) = cur_path.take() {
                    merge(&mut files, path, std::mem::take(&mut cur));
                }
            }
        }
        // Trailing record without an explicit end_of_record.
        if let Some(path) = cur_path.take() {
            merge(&mut files, path, cur);
        }
        Coverage { files }
    }

    /// Number of files with coverage data.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Coverage of `(file, line)`. `file` is matched by suffix so a repo-relative
    /// path (e.g. `src/main.rs`) matches an absolute LCOV `SF:` path
    /// (`/abs/repo/src/main.rs`) and vice versa.
    pub fn line_cov(&self, file: &Path, line: u32) -> LineCov {
        let Some(lines) = self.lookup(file) else {
            return LineCov::None;
        };
        match lines.get(&line) {
            Some(0) => LineCov::Uncovered,
            Some(_) => LineCov::Covered,
            None => LineCov::None,
        }
    }

    /// Find the line map for `file` by matching the stored path as a suffix of
    /// `file` or vice versa (handles abs vs repo-relative mismatch).
    fn lookup(&self, file: &Path) -> Option<&HashMap<u32, u64>> {
        // Exact hit first.
        if let Some(m) = self.files.get(file) {
            return Some(m);
        }
        self.files
            .iter()
            .find(|(p, _)| p.ends_with(file) || file.ends_with(p.as_path()))
            .map(|(_, m)| m)
    }
}

fn merge(files: &mut HashMap<PathBuf, HashMap<u32, u64>>, path: PathBuf, lines: HashMap<u32, u64>) {
    if lines.is_empty() {
        return;
    }
    let entry = files.entry(path).or_default();
    for (ln, hits) in lines {
        let e = entry.entry(ln).or_insert(0);
        *e = (*e).max(hits);
    }
}

fn parse_da(da: &str) -> Option<(u32, u64)> {
    let mut it = da.split(',');
    let ln = it.next()?.trim().parse::<u32>().ok()?;
    let hits = it.next()?.trim().parse::<u64>().ok()?;
    Some((ln, hits))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
TN:
SF:src/main.rs
DA:1,3
DA:2,0
DA:4,1
LF:3
LH:2
end_of_record
SF:/abs/repo/src/lib.rs
DA:10,5
end_of_record
";

    #[test]
    fn parses_files_and_line_hits() {
        let c = Coverage::parse_lcov(SAMPLE);
        assert_eq!(c.file_count(), 2);
        assert_eq!(c.line_cov(Path::new("src/main.rs"), 1), LineCov::Covered);
        assert_eq!(c.line_cov(Path::new("src/main.rs"), 2), LineCov::Uncovered);
        assert_eq!(c.line_cov(Path::new("src/main.rs"), 4), LineCov::Covered);
        // Line with no DA record → None.
        assert_eq!(c.line_cov(Path::new("src/main.rs"), 3), LineCov::None);
    }

    #[test]
    fn matches_relative_against_absolute_sf_path() {
        let c = Coverage::parse_lcov(SAMPLE);
        // Repo-relative query matches the absolute SF: path by suffix.
        assert_eq!(c.line_cov(Path::new("src/lib.rs"), 10), LineCov::Covered);
    }

    #[test]
    fn unknown_file_or_line_is_none() {
        let c = Coverage::parse_lcov(SAMPLE);
        assert_eq!(c.line_cov(Path::new("nope.rs"), 1), LineCov::None);
        assert_eq!(c.line_cov(Path::new("src/main.rs"), 999), LineCov::None);
    }

    #[test]
    fn skips_malformed_da_lines() {
        let c = Coverage::parse_lcov("SF:a.rs\nDA:bad\nDA:5,2\nend_of_record\n");
        assert_eq!(c.line_cov(Path::new("a.rs"), 5), LineCov::Covered);
    }

    #[test]
    fn trailing_record_without_end_is_kept() {
        let c = Coverage::parse_lcov("SF:a.rs\nDA:1,1\n");
        assert_eq!(c.line_cov(Path::new("a.rs"), 1), LineCov::Covered);
    }
}
