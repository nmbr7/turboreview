use std::path::Path;

pub fn icon_for(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "rs" => "\u{e7a8}",
        "py" => "\u{e606}",
        "js" | "mjs" | "cjs" => "\u{e74e}",
        "ts" | "tsx" => "\u{e628}",
        "go" => "\u{e627}",
        "c" | "h" => "\u{e61e}",
        "cpp" | "cc" | "hpp" | "cxx" => "\u{e61d}",
        "md" | "markdown" => "\u{e73e}",
        "json" => "\u{e60b}",
        "toml" | "yaml" | "yml" | "ini" | "cfg" | "conf" => "\u{e615}",
        "lock" => "\u{f023}",
        "sh" | "bash" | "zsh" => "\u{f489}",
        "html" | "htm" => "\u{e736}",
        "css" | "scss" => "\u{e749}",
        _ => "\u{f15b}",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn known_extensions_map_to_distinct_icons() {
        assert_eq!(icon_for(Path::new("a.rs")), "\u{e7a8}");
        assert_eq!(icon_for(Path::new("b.py")), "\u{e606}");
        assert_ne!(icon_for(Path::new("a.rs")), icon_for(Path::new("b.py")));
    }

    #[test]
    fn unknown_extension_falls_back_to_generic() {
        assert_eq!(icon_for(Path::new("file.zzz")), "\u{f15b}");
        assert_eq!(icon_for(Path::new("noext")), "\u{f15b}");
    }
}
