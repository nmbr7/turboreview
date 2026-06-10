use std::collections::HashSet;
use std::path::PathBuf;

use crate::app::FileChange;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Dir { path: PathBuf, collapsed: bool },
    File { file_index: usize },
}

#[derive(Debug, Clone)]
pub struct Row {
    pub depth: usize,
    pub name: String,
    pub kind: RowKind,
}

pub fn build_rows(files: &[FileChange], collapsed: &HashSet<PathBuf>, hidden: &HashSet<PathBuf>) -> Vec<Row> {
    // Build a tree structure then emit rows in DFS pre-order.
    // Each node is either a directory or a file.
    // Dirs sort before files at each level; within each kind, alphabetical.

    // Represents a tree node
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

    // Insert a file into the tree rooted at `children`.
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
            // leaf file
            children.push(Node::File {
                name: components[0].to_string(),
                file_index,
            });
            return;
        }
        // directory component
        let dir_name = components[0];
        let full_path = if path_prefix.is_empty() {
            dir_name.to_string()
        } else {
            format!("{}/{}", path_prefix, dir_name)
        };
        // Find existing dir node or create one
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
        collapsed: &HashSet<PathBuf>,
        rows: &mut Vec<Row>,
    ) {
        for node in nodes {
            match node {
                Node::Dir { name, full_path, children } => {
                    let is_collapsed = collapsed.contains(full_path);
                    rows.push(Row {
                        depth,
                        name: name.clone(),
                        kind: RowKind::Dir {
                            path: full_path.clone(),
                            collapsed: is_collapsed,
                        },
                    });
                    if !is_collapsed {
                        emit(children, depth + 1, collapsed, rows);
                    }
                }
                Node::File { name, file_index } => {
                    rows.push(Row {
                        depth,
                        name: name.clone(),
                        kind: RowKind::File {
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

    let mut rows = Vec::new();
    emit(&root, 0, collapsed, &mut rows);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{FileChange, Status};

    fn make_files(paths: &[&str]) -> Vec<FileChange> {
        paths
            .iter()
            .map(|p| FileChange {
                path: PathBuf::from(p),
                status: Status::Modified,
            })
            .collect()
    }

    #[test]
    fn flat_files_no_dirs() {
        // a.rs, b.rs at top level (no dirs) -> two File rows at depth 0
        let files = make_files(&["a.rs", "b.rs"]);
        let collapsed = HashSet::new();
        let rows = build_rows(&files, &collapsed, &HashSet::new());

        assert_eq!(rows.len(), 2);

        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[0].name, "a.rs");
        assert_eq!(rows[0].kind, RowKind::File { file_index: 0 });

        assert_eq!(rows[1].depth, 0);
        assert_eq!(rows[1].name, "b.rs");
        assert_eq!(rows[1].kind, RowKind::File { file_index: 1 });
    }

    #[test]
    fn nested_files_dirs_before_files_at_root() {
        // src/main.rs, src/ui.rs, README.md
        // Expected order: Dir "src" (depth 0), File "main.rs" (depth 1), File "ui.rs" (depth 1), File "README.md" (depth 0)
        // dirs sort before files at root level, so "src" before "README.md"
        let files = make_files(&["src/main.rs", "src/ui.rs", "README.md"]);
        let collapsed = HashSet::new();
        let rows = build_rows(&files, &collapsed, &HashSet::new());

        assert_eq!(rows.len(), 4);

        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[0].name, "src");
        assert!(matches!(rows[0].kind, RowKind::Dir { collapsed: false, .. }));

        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[1].name, "main.rs");
        assert_eq!(rows[1].kind, RowKind::File { file_index: 0 });

        assert_eq!(rows[2].depth, 1);
        assert_eq!(rows[2].name, "ui.rs");
        assert_eq!(rows[2].kind, RowKind::File { file_index: 1 });

        assert_eq!(rows[3].depth, 0);
        assert_eq!(rows[3].name, "README.md");
        assert_eq!(rows[3].kind, RowKind::File { file_index: 2 });
    }

    #[test]
    fn collapsed_dir_hides_children() {
        // src/main.rs, src/ui.rs, README.md, with src collapsed
        // Expected: Dir "src" (collapsed=true, depth 0) + File "README.md" (depth 0)
        // Children of src NOT emitted.
        let files = make_files(&["src/main.rs", "src/ui.rs", "README.md"]);
        let mut collapsed = HashSet::new();
        collapsed.insert(PathBuf::from("src"));
        let rows = build_rows(&files, &collapsed, &HashSet::new());

        assert_eq!(rows.len(), 2);

        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[0].name, "src");
        assert!(matches!(
            rows[0].kind,
            RowKind::Dir { collapsed: true, .. }
        ));

        assert_eq!(rows[1].depth, 0);
        assert_eq!(rows[1].name, "README.md");
        assert_eq!(rows[1].kind, RowKind::File { file_index: 2 });
    }

    #[test]
    fn hidden_files_are_omitted_and_empty_dirs_pruned() {
        // src/a.rs, src/b.rs, top.rs ; hide src/a.rs and src/b.rs -> src dir pruned
        let files = make_files(&["src/a.rs", "src/b.rs", "top.rs"]);
        let collapsed = HashSet::new();
        let mut hidden = HashSet::new();
        hidden.insert(PathBuf::from("src/a.rs"));
        hidden.insert(PathBuf::from("src/b.rs"));
        let rows = build_rows(&files, &collapsed, &hidden);
        // only top.rs remains; src dir gone
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "top.rs");
        assert!(matches!(rows[0].kind, RowKind::File { .. }));
    }

    #[test]
    fn same_basename_at_root_and_nested_stay_distinct() {
        // A file at root and one nested under a dir share the basename "foo.rs".
        // They must remain two separate File rows pointing at distinct indices.
        let files = make_files(&["foo.rs", "src/foo.rs"]);
        let collapsed = HashSet::new();
        let rows = build_rows(&files, &collapsed, &HashSet::new());

        // Expected order: Dir "src" (depth 0), File "foo.rs" (depth 1, index 1),
        // File "foo.rs" (depth 0, index 0) — dirs sort before files at root.
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "src");
        assert!(matches!(rows[0].kind, RowKind::Dir { .. }));
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[1].kind, RowKind::File { file_index: 1 });
        assert_eq!(rows[2].depth, 0);
        assert_eq!(rows[2].kind, RowKind::File { file_index: 0 });
    }
}
