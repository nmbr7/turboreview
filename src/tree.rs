use std::collections::HashSet;
use std::path::PathBuf;

use crate::app::{FileChange, Section};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Header { section: Section, count: usize },
    Dir { section: Section, path: PathBuf, collapsed: bool },
    File { section: Section, file_index: usize },
}

#[derive(Debug, Clone)]
pub struct Row {
    pub depth: usize,
    pub name: String,
    pub kind: RowKind,
}

/// Build rows for both unstaged and staged sections.
/// `collapsed` keys are (Section, PathBuf) to allow same dir path in each section.
/// `hidden` is a set of file paths hidden across all sections (reviewed files).
pub fn build_rows(
    unstaged: &[FileChange],
    staged: &[FileChange],
    collapsed: &HashSet<(Section, PathBuf)>,
    hidden: &HashSet<PathBuf>,
) -> Vec<Row> {
    let mut rows = Vec::new();

    // Emit Header then tree for each section
    rows.push(Row {
        depth: 0,
        name: "Unstaged".to_string(),
        kind: RowKind::Header { section: Section::Unstaged, count: unstaged.len() },
    });
    build_section_rows(unstaged, Section::Unstaged, collapsed, hidden, &mut rows);

    rows.push(Row {
        depth: 0,
        name: "Staged".to_string(),
        kind: RowKind::Header { section: Section::Staged, count: staged.len() },
    });
    build_section_rows(staged, Section::Staged, collapsed, hidden, &mut rows);

    rows
}

/// Build rows for a single commit's changed files (Section::Commit).
/// Produces: Header{section: Commit, count} followed by the tree of commit files.
pub fn build_commit_rows(
    files: &[FileChange],
    collapsed: &HashSet<(Section, PathBuf)>,
    hidden: &HashSet<PathBuf>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    rows.push(Row {
        depth: 0,
        name: "Commit".to_string(),
        kind: RowKind::Header { section: Section::Commit, count: files.len() },
    });
    build_section_rows(files, Section::Commit, collapsed, hidden, &mut rows);
    rows
}

fn build_section_rows(
    files: &[FileChange],
    section: Section,
    collapsed: &HashSet<(Section, PathBuf)>,
    hidden: &HashSet<PathBuf>,
    out: &mut Vec<Row>,
) {
    // Build a tree structure then emit rows in DFS pre-order.
    // Each node is either a directory or a file.
    // Dirs sort before files at each level; within each kind, alphabetical.

    enum Node {
        Dir {
            name: String,
            full_path: PathBuf,
            children: Vec<Node>,
        },
        File {
            name: String,
            file_index: usize,
        },
    }

    impl Node {
        fn sort_key(&self) -> (u8, &str) {
            match self {
                Node::Dir { name, .. } => (0, name.as_str()),
                Node::File { name, .. } => (1, name.as_str()),
            }
        }
    }

    fn insert(
        children: &mut Vec<Node>,
        components: &[&str],
        file_index: usize,
        path_prefix: &str,
    ) {
        if components.is_empty() {
            return;
        }
        if components.len() == 1 {
            children.push(Node::File {
                name: components[0].to_string(),
                file_index,
            });
            return;
        }
        let dir_name = components[0];
        let full_path = if path_prefix.is_empty() {
            dir_name.to_string()
        } else {
            format!("{}/{}", path_prefix, dir_name)
        };
        let idx = children.iter().position(|n| match n {
            Node::Dir { name, .. } => name == dir_name,
            _ => false,
        });
        if let Some(i) = idx {
            if let Node::Dir { children: sub, .. } = &mut children[i] {
                insert(sub, &components[1..], file_index, &full_path);
            }
        } else {
            let mut sub = Vec::new();
            insert(&mut sub, &components[1..], file_index, &full_path);
            children.push(Node::Dir {
                name: dir_name.to_string(),
                full_path: PathBuf::from(&full_path),
                children: sub,
            });
        }
    }

    fn sort_nodes(children: &mut Vec<Node>) {
        children.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        for child in children.iter_mut() {
            if let Node::Dir { children: sub, .. } = child {
                sort_nodes(sub);
            }
        }
    }

    fn emit(
        nodes: &[Node],
        depth: usize,
        section: Section,
        collapsed: &HashSet<(Section, PathBuf)>,
        rows: &mut Vec<Row>,
    ) {
        for node in nodes {
            match node {
                Node::Dir { name, full_path, children } => {
                    let is_collapsed = collapsed.contains(&(section, full_path.clone()));
                    rows.push(Row {
                        depth,
                        name: name.clone(),
                        kind: RowKind::Dir {
                            section,
                            path: full_path.clone(),
                            collapsed: is_collapsed,
                        },
                    });
                    if !is_collapsed {
                        emit(children, depth + 1, section, collapsed, rows);
                    }
                }
                Node::File { name, file_index } => {
                    rows.push(Row {
                        depth,
                        name: name.clone(),
                        kind: RowKind::File {
                            section,
                            file_index: *file_index,
                        },
                    });
                }
            }
        }
    }

    let mut root: Vec<Node> = Vec::new();
    for (idx, file) in files.iter().enumerate() {
        if hidden.contains(&file.path) {
            continue;
        }
        let path_str = file.path.to_string_lossy();
        let components: Vec<&str> = path_str.split('/').collect();
        insert(&mut root, &components, idx, "");
    }
    sort_nodes(&mut root);

    // Tree rows at depth 1 (indented under header at depth 0)
    emit(&root, 1, section, collapsed, out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{FileChange, Section, Status};

    fn make_files(paths: &[&str]) -> Vec<FileChange> {
        paths
            .iter()
            .map(|p| FileChange {
                path: PathBuf::from(p),
                status: Status::Modified,
            })
            .collect()
    }

    fn empty_collapsed() -> HashSet<(Section, PathBuf)> {
        HashSet::new()
    }

    #[test]
    fn flat_files_no_dirs() {
        // a.rs, b.rs at top level (no dirs) -> Header + two File rows at depth 1
        let files = make_files(&["a.rs", "b.rs"]);
        let collapsed = empty_collapsed();
        let rows = build_rows(&files, &[], &collapsed, &HashSet::new());

        // Header(Unstaged) + a.rs + b.rs + Header(Staged)
        assert_eq!(rows.len(), 4);

        assert!(matches!(rows[0].kind, RowKind::Header { section: Section::Unstaged, count: 2 }));

        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[1].name, "a.rs");
        assert_eq!(rows[1].kind, RowKind::File { section: Section::Unstaged, file_index: 0 });

        assert_eq!(rows[2].depth, 1);
        assert_eq!(rows[2].name, "b.rs");
        assert_eq!(rows[2].kind, RowKind::File { section: Section::Unstaged, file_index: 1 });

        assert!(matches!(rows[3].kind, RowKind::Header { section: Section::Staged, count: 0 }));
    }

    #[test]
    fn nested_files_dirs_before_files_at_root() {
        // src/main.rs, src/ui.rs, README.md
        // Expected order: Header(Unstaged), Dir "src" (depth 1), File "main.rs" (depth 2),
        // File "ui.rs" (depth 2), File "README.md" (depth 1), Header(Staged)
        let files = make_files(&["src/main.rs", "src/ui.rs", "README.md"]);
        let collapsed = empty_collapsed();
        let rows = build_rows(&files, &[], &collapsed, &HashSet::new());

        // Header + Dir(src) + main.rs + ui.rs + README.md + Header(Staged)
        assert_eq!(rows.len(), 6);

        assert!(matches!(rows[0].kind, RowKind::Header { section: Section::Unstaged, .. }));

        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[1].name, "src");
        assert!(matches!(rows[1].kind, RowKind::Dir { section: Section::Unstaged, collapsed: false, .. }));

        assert_eq!(rows[2].depth, 2);
        assert_eq!(rows[2].name, "main.rs");
        assert_eq!(rows[2].kind, RowKind::File { section: Section::Unstaged, file_index: 0 });

        assert_eq!(rows[3].depth, 2);
        assert_eq!(rows[3].name, "ui.rs");
        assert_eq!(rows[3].kind, RowKind::File { section: Section::Unstaged, file_index: 1 });

        assert_eq!(rows[4].depth, 1);
        assert_eq!(rows[4].name, "README.md");
        assert_eq!(rows[4].kind, RowKind::File { section: Section::Unstaged, file_index: 2 });

        assert!(matches!(rows[5].kind, RowKind::Header { section: Section::Staged, count: 0 }));
    }

    #[test]
    fn collapsed_dir_hides_children() {
        // src/main.rs, src/ui.rs, README.md, with src collapsed
        // Expected: Header(Unstaged), Dir "src" (collapsed=true, depth 1), File "README.md" (depth 1), Header(Staged)
        let files = make_files(&["src/main.rs", "src/ui.rs", "README.md"]);
        let mut collapsed = empty_collapsed();
        collapsed.insert((Section::Unstaged, PathBuf::from("src")));
        let rows = build_rows(&files, &[], &collapsed, &HashSet::new());

        // Header(Unstaged) + Dir(src,collapsed) + README.md + Header(Staged)
        assert_eq!(rows.len(), 4);

        assert!(matches!(rows[0].kind, RowKind::Header { section: Section::Unstaged, .. }));

        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[1].name, "src");
        assert!(matches!(
            rows[1].kind,
            RowKind::Dir { section: Section::Unstaged, collapsed: true, .. }
        ));

        assert_eq!(rows[2].depth, 1);
        assert_eq!(rows[2].name, "README.md");
        assert_eq!(rows[2].kind, RowKind::File { section: Section::Unstaged, file_index: 2 });

        assert!(matches!(rows[3].kind, RowKind::Header { section: Section::Staged, count: 0 }));
    }

    #[test]
    fn hidden_files_are_omitted_and_empty_dirs_pruned() {
        // src/a.rs, src/b.rs, top.rs ; hide src/a.rs and src/b.rs -> src dir pruned
        let files = make_files(&["src/a.rs", "src/b.rs", "top.rs"]);
        let collapsed = empty_collapsed();
        let mut hidden = HashSet::new();
        hidden.insert(PathBuf::from("src/a.rs"));
        hidden.insert(PathBuf::from("src/b.rs"));
        let rows = build_rows(&files, &[], &collapsed, &hidden);
        // Header(Unstaged) + top.rs + Header(Staged)
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].name, "top.rs");
        assert!(matches!(rows[1].kind, RowKind::File { .. }));
    }

    #[test]
    fn same_basename_at_root_and_nested_stay_distinct() {
        let files = make_files(&["foo.rs", "src/foo.rs"]);
        let collapsed = empty_collapsed();
        let rows = build_rows(&files, &[], &collapsed, &HashSet::new());

        // Header(Unstaged) + Dir "src" (depth 1) + File "foo.rs" (depth 2, index 1) +
        // File "foo.rs" (depth 1, index 0) + Header(Staged)
        assert_eq!(rows.len(), 5);
        assert!(matches!(rows[0].kind, RowKind::Header { .. }));
        assert_eq!(rows[1].name, "src");
        assert!(matches!(rows[1].kind, RowKind::Dir { .. }));
        assert_eq!(rows[2].depth, 2);
        assert_eq!(rows[2].kind, RowKind::File { section: Section::Unstaged, file_index: 1 });
        assert_eq!(rows[3].depth, 1);
        assert_eq!(rows[3].kind, RowKind::File { section: Section::Unstaged, file_index: 0 });
        assert!(matches!(rows[4].kind, RowKind::Header { section: Section::Staged, .. }));
    }

    #[test]
    fn both_sections_non_empty_shows_both_headers() {
        let unstaged = make_files(&["a.rs"]);
        let staged = make_files(&["b.rs"]);
        let collapsed = empty_collapsed();
        let rows = build_rows(&unstaged, &staged, &collapsed, &HashSet::new());

        // Header(Unstaged) + a.rs + Header(Staged) + b.rs
        assert_eq!(rows.len(), 4);
        assert!(matches!(rows[0].kind, RowKind::Header { section: Section::Unstaged, count: 1 }));
        assert_eq!(rows[1].kind, RowKind::File { section: Section::Unstaged, file_index: 0 });
        assert!(matches!(rows[2].kind, RowKind::Header { section: Section::Staged, count: 1 }));
        assert_eq!(rows[3].kind, RowKind::File { section: Section::Staged, file_index: 0 });
    }

    #[test]
    fn build_commit_rows_emits_commit_section() {
        let files = make_files(&["src/main.rs", "lib.rs"]);
        let collapsed = empty_collapsed();
        let rows = super::build_commit_rows(&files, &collapsed, &HashSet::new());

        // Header(Commit) + Dir(src) + main.rs + lib.rs = 4 rows
        assert_eq!(rows.len(), 4);
        assert!(matches!(rows[0].kind, RowKind::Header { section: Section::Commit, count: 2 }));
        // All non-header rows should have Section::Commit
        for row in &rows[1..] {
            match &row.kind {
                RowKind::Dir { section, .. } => assert_eq!(*section, Section::Commit),
                RowKind::File { section, .. } => assert_eq!(*section, Section::Commit),
                RowKind::Header { .. } => panic!("unexpected extra header"),
            }
        }
    }

    #[test]
    fn collapsed_dir_in_unstaged_does_not_affect_staged() {
        // Same dir name "src" in both sections; collapse in unstaged only
        let unstaged = make_files(&["src/a.rs"]);
        let staged = make_files(&["src/b.rs"]);
        let mut collapsed = empty_collapsed();
        collapsed.insert((Section::Unstaged, PathBuf::from("src")));
        let rows = build_rows(&unstaged, &staged, &collapsed, &HashSet::new());

        // Header(Unstaged) + Dir(src,collapsed,Unstaged) + Header(Staged) + Dir(src,NOT-collapsed,Staged) + b.rs
        assert_eq!(rows.len(), 5);
        assert!(matches!(rows[1].kind, RowKind::Dir { section: Section::Unstaged, collapsed: true, .. }));
        assert!(matches!(rows[3].kind, RowKind::Dir { section: Section::Staged, collapsed: false, .. }));
        assert_eq!(rows[4].kind, RowKind::File { section: Section::Staged, file_index: 0 });
    }
}
