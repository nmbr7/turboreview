use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::app::CommentScope;

/// Returns the worktree-scope directory: `<repo_root>/.turboreview`.
pub fn worktree_dir(repo_root: &Path) -> PathBuf {
    repo_root.join(".turboreview")
}

/// Returns the commit-scope directory: `<repo_root>/.turboreview/commits/<sha>`.
pub fn commit_dir(repo_root: &Path, sha: &str) -> PathBuf {
    repo_root.join(".turboreview").join("commits").join(sha)
}

/// The directory holding comments.json / reviewed.json for the given scope.
pub fn scope_dir(repo_root: &Path, scope: &CommentScope) -> PathBuf {
    match scope {
        CommentScope::Worktree => worktree_dir(repo_root),
        CommentScope::Commit(sha) => commit_dir(repo_root, sha),
    }
}

#[derive(Serialize)]
struct LogEntry<'a> {
    path: &'a str,
    line: u32,
    scope: &'a str,
    date: String,
    action: &'a str,
}

/// The debug-adapter command and its arguments (e.g. `codelldb`, `lldb-dap`,
/// `debugpy`). Spawned with stdin/stdout piped to speak DAP.
#[derive(Clone, Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct AdapterConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Remote-attach configuration. `host`/`port` build an lldb-dap
/// `attachCommands` of `gdb-remote <host>:<port>`; `attach_commands` overrides
/// that with raw adapter commands (e.g. for codelldb / custom setups).
#[derive(Clone, Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct RemoteConfig {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    /// Raw adapter attach commands; when non-empty, used instead of host/port.
    #[serde(default)]
    pub attach_commands: Vec<String>,
}

impl RemoteConfig {
    /// Whether a remote target is configured (commands or host:port present).
    pub fn is_set(&self) -> bool {
        !self.attach_commands.is_empty() || (!self.host.is_empty() && self.port != 0)
    }

    /// The lldb commands to run on attach: explicit `attach_commands`, else a
    /// `gdb-remote host:port`.
    pub fn commands(&self) -> Vec<String> {
        if !self.attach_commands.is_empty() {
            self.attach_commands.clone()
        } else {
            vec![format!("gdb-remote {}:{}", self.host, self.port)]
        }
    }
}

/// Per-repo debug configuration: how to build the debuggee, which binary to run,
/// and which adapter to drive. Source map handles old-commit / remote paths.
#[derive(Clone, Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct DebugConfig {
    #[serde(default)]
    pub adapter: AdapterConfig,
    /// Shell command run before launch (e.g. `cargo build`). Empty = skip.
    #[serde(default)]
    pub build: String,
    /// Path to the built binary, relative to the source root.
    #[serde(default)]
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory for the debuggee (relative to source root). Empty = ".".
    #[serde(default)]
    pub cwd: String,
    /// `[[from, to]]` source path remaps for remote / old-commit debugging.
    #[serde(default)]
    pub source_map: Vec<(String, String)>,
    /// Remote-attach target (gdbserver / Docker). Empty = no remote configured.
    #[serde(default)]
    pub remote: RemoteConfig,
}

/// A named macro-expansion command (shown in the expand picker).
#[derive(Clone, Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct ExpandCommand {
    pub name: String,
    pub command: String,
}

fn default_diff_style() -> String {
    "dim".into()
}

/// Persisted configuration. New fields use `#[serde(default)]` so older
/// config.json files (which may lack them) keep loading.
#[derive(Serialize, Deserialize)]
struct Config {
    theme: String, // "dark" | "light"
    #[serde(default)]
    split_diff: bool, // side-by-side diff toggle
    #[serde(default = "default_diff_style")]
    diff_style: String, // "dim" (default) | "bright" | "plain"
    #[serde(default)]
    wrap_lines: bool, // wrap long diff lines (default false)
    #[serde(default)]
    debug: DebugConfig, // debugger build/adapter/program config
    #[serde(default)]
    coverage_file: String, // path to an LCOV file (relative to repo root)
    #[serde(default)]
    coverage_command: String, // shell command that generates the LCOV file
    #[serde(default)]
    expand_command: String, // single macro-expansion command (fallback)
    #[serde(default)]
    expand_commands: Vec<ExpandCommand>, // named commands shown in the expand picker
}

impl Default for Config {
    fn default() -> Self {
        // diff_style defaults to "dim" to match the serde default, so a missing
        // config.json keeps the historical dimmed look.
        Config {
            theme: String::new(),
            split_diff: false,
            diff_style: default_diff_style(),
            wrap_lines: false,
            debug: DebugConfig::default(),
            coverage_file: String::new(),
            coverage_command: String::new(),
            expand_command: String::new(),
            expand_commands: Vec::new(),
        }
    }
}

/// Read the whole config (defaults if missing/unparseable).
fn load_config(repo_root: &Path) -> Config {
    let path = worktree_dir(repo_root).join("config.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return Config::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Write the whole config to `<repo_root>/.turboreview/config.json`.
fn save_config(repo_root: &Path, cfg: &Config) -> Result<()> {
    let dir = worktree_dir(repo_root);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("config.json"), serde_json::to_vec_pretty(cfg)?)?;
    Ok(())
}

/// Load the persisted theme. Returns `Theme::Dark` if missing/unparseable.
pub fn load_theme(repo_root: &Path) -> crate::theme::Theme {
    match load_config(repo_root).theme.as_str() {
        "light" => crate::theme::Theme::Light,
        _ => crate::theme::Theme::Dark,
    }
}

/// Persist the theme, preserving any other config fields (read-modify-write).
pub fn save_theme(repo_root: &Path, theme: crate::theme::Theme) -> Result<()> {
    let mut cfg = load_config(repo_root);
    cfg.theme = match theme {
        crate::theme::Theme::Light => "light".into(),
        _ => "dark".into(),
    };
    save_config(repo_root, &cfg)
}

/// Load the persisted side-by-side diff preference (false if unset).
pub fn load_split(repo_root: &Path) -> bool {
    load_config(repo_root).split_diff
}

/// Persist the side-by-side diff preference, preserving other config fields.
pub fn save_split(repo_root: &Path, split: bool) -> Result<()> {
    let mut cfg = load_config(repo_root);
    cfg.split_diff = split;
    save_config(repo_root, &cfg)
}

/// Load the persisted diff style ("dim" if unset).
pub fn load_diff_style(repo_root: &Path) -> crate::app::DiffStyle {
    crate::app::DiffStyle::from_str(&load_config(repo_root).diff_style)
}

/// Persist the diff style, preserving other config fields.
pub fn save_diff_style(repo_root: &Path, style: crate::app::DiffStyle) -> Result<()> {
    let mut cfg = load_config(repo_root);
    cfg.diff_style = style.as_str().into();
    save_config(repo_root, &cfg)
}

/// Load the persisted line-wrap preference (false if unset).
pub fn load_wrap_lines(repo_root: &Path) -> bool {
    load_config(repo_root).wrap_lines
}

/// Persist the line-wrap preference, preserving other config fields.
pub fn save_wrap_lines(repo_root: &Path, wrap: bool) -> Result<()> {
    let mut cfg = load_config(repo_root);
    cfg.wrap_lines = wrap;
    save_config(repo_root, &cfg)
}

/// Load the persisted debug configuration (defaults if unset/unparseable).
pub fn load_debug_config(repo_root: &Path) -> DebugConfig {
    load_config(repo_root).debug
}

/// Load and parse the configured LCOV coverage file. Returns an error message
/// when no path is set or the file can't be read.
pub fn load_coverage(repo_root: &Path) -> Result<crate::coverage::Coverage> {
    let cfg = load_config(repo_root);
    if cfg.coverage_file.trim().is_empty() {
        anyhow::bail!("no coverage file set (config: \"coverage_file\")");
    }
    let path = repo_root.join(&cfg.coverage_file);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    Ok(crate::coverage::Coverage::parse_lcov(&text))
}

/// Derive a Rust module path from a repo-relative source path for `cargo
/// expand`, e.g. `src/foo/bar.rs` → `foo::bar`, `src/lib.rs`/`src/main.rs` → "".
/// Returns the path string for non-`src` files unchanged (best effort).
pub fn module_path_for(file: &Path) -> String {
    let rel = file.strip_prefix("src").unwrap_or(file);
    let mut parts: Vec<String> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
        .collect();
    if let Some(last) = parts.last_mut() {
        // Drop the .rs extension; mod.rs / main.rs / lib.rs contribute nothing.
        let stem = last.trim_end_matches(".rs");
        if stem == "mod" || stem == "main" || stem == "lib" {
            parts.pop();
        } else {
            *last = stem.to_string();
        }
    }
    parts.join("::")
}

/// Run the configured macro-expansion command for `file` and return its stdout
/// (the expanded source). `{file}` and `{module}` in the command are replaced
/// with the repo-relative path and the derived module path. Defaults to
/// `cargo expand {module}` when no command is configured.
pub fn run_expand(repo_root: &Path, file: &Path) -> Result<String> {
    let cfg = load_config(repo_root);
    let template = if cfg.expand_command.trim().is_empty() {
        "cargo expand {module}".to_string()
    } else {
        cfg.expand_command
    };
    run_expand_template(repo_root, file, &template)
}

/// Named expand commands offered by the picker. Falls back to the single
/// `expand_command` (or the built-in default) when `expand_commands` is empty.
pub fn load_expand_commands(repo_root: &Path) -> Vec<ExpandCommand> {
    let cfg = load_config(repo_root);
    if !cfg.expand_commands.is_empty() {
        return cfg.expand_commands;
    }
    let command = if cfg.expand_command.trim().is_empty() {
        "cargo expand {module}".to_string()
    } else {
        cfg.expand_command
    };
    vec![ExpandCommand {
        name: "expand".into(),
        command,
    }]
}

/// Run a specific expand command `template` (with `{file}`/`{module}`
/// substitution) for `file` and return its stdout.
pub fn run_expand_template(repo_root: &Path, file: &Path, template: &str) -> Result<String> {
    let module = module_path_for(file);
    let cmd = template
        .replace("{file}", &file.to_string_lossy())
        .replace("{module}", &module);
    // Note: an empty {module} leaves a trailing space, which `sh` ignores. We do
    // NOT collapse whitespace, so shell constructs in the command (case, $(), …)
    // stay intact for path-aware expand commands.
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(repo_root)
        .output()
        .map_err(|e| anyhow::anyhow!("running expand command: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // `cargo expand` on a lib+bin crate needs an explicit target.
        let hint = if stderr.contains("can only be passed to one target") {
            "  (hint: set expand_command to `cargo expand --lib {module}` or `--bin <name>`)"
        } else {
            ""
        };
        anyhow::bail!(
            "expand command failed ({}): {}{hint}",
            out.status,
            stderr.trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run the configured coverage command (blocking) to (re)generate the LCOV
/// file, then load it. Errors when no command is set or the command fails.
pub fn run_coverage(repo_root: &Path) -> Result<crate::coverage::Coverage> {
    let cmd = load_config(repo_root).coverage_command;
    if cmd.trim().is_empty() {
        anyhow::bail!("no coverage command set (config: \"coverage_command\")");
    }
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(repo_root)
        .output()
        .map_err(|e| anyhow::anyhow!("running coverage command: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "coverage command failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    load_coverage(repo_root)
}

const ARCHIVE_DAYS: i64 = 14;

/// Returns the archive file path: `<repo_root>/.turboreview/archive/comments-archive.jsonl`.
pub fn archive_path(repo_root: &Path) -> PathBuf {
    worktree_dir(repo_root)
        .join("archive")
        .join("comments-archive.jsonl")
}

/// Append archived comments as JSON lines to the archive file. Best-effort; errors returned.
pub fn append_archive(
    repo_root: &Path,
    comments: &[crate::comments::Comment],
) -> anyhow::Result<()> {
    if comments.is_empty() {
        return Ok(());
    }
    let path = archive_path(repo_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    for c in comments {
        let mut line = serde_json::to_string(c)?;
        line.push('\n');
        f.write_all(line.as_bytes())?;
    }
    Ok(())
}

/// Returns the cutoff unix epoch seconds for auto-archiving: `now - ARCHIVE_DAYS * 86400`.
pub fn archive_cutoff_secs(now: i64) -> i64 {
    now - ARCHIVE_DAYS * 86400
}

/// Append one line to `<repo_root>/.turboreview/comment-log.jsonl`.
/// Best-effort; errors are returned but the caller is expected to ignore them.
pub fn append_comment_log(
    repo_root: &Path,
    path: &Path,
    line: u32,
    scope: &str,
    action: &str,
) -> Result<()> {
    let dir = repo_root.join(".turboreview");
    std::fs::create_dir_all(&dir)?;
    let path_str = path.to_string_lossy();
    let entry = LogEntry {
        path: &path_str,
        line,
        scope,
        date: crate::git::format_datetime(now_secs()),
        action,
    };
    let mut line_json = serde_json::to_string(&entry)?;
    line_json.push('\n');
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("comment-log.jsonl"))?;
    f.write_all(line_json.as_bytes())?;
    Ok(())
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn module_path_derivation() {
        use std::path::Path;
        assert_eq!(module_path_for(Path::new("src/foo/bar.rs")), "foo::bar");
        assert_eq!(module_path_for(Path::new("src/main.rs")), "");
        assert_eq!(module_path_for(Path::new("src/lib.rs")), "");
        assert_eq!(module_path_for(Path::new("src/app.rs")), "app");
        assert_eq!(module_path_for(Path::new("src/a/mod.rs")), "a");
    }

    #[test]
    fn remote_config_commands_and_is_set() {
        let mut r = RemoteConfig::default();
        assert!(!r.is_set());
        r.host = "localhost".into();
        r.port = 1234;
        assert!(r.is_set());
        assert_eq!(r.commands(), vec!["gdb-remote localhost:1234".to_string()]);
        // Explicit commands override host/port.
        r.attach_commands = vec!["process connect connect://x:9".into()];
        assert_eq!(r.commands(), vec!["process connect connect://x:9".to_string()]);
        assert!(r.is_set());
    }

    #[test]
    fn load_coverage_reads_configured_lcov() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        // Config points at a coverage file; write that file.
        std::fs::create_dir_all(worktree_dir(root)).unwrap();
        std::fs::write(
            worktree_dir(root).join("config.json"),
            br#"{"theme":"dark","coverage_file":"cov/lcov.info"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("cov")).unwrap();
        std::fs::write(
            root.join("cov/lcov.info"),
            "SF:src/x.rs\nDA:1,4\nDA:2,0\nend_of_record\n",
        )
        .unwrap();

        let cov = load_coverage(root).unwrap();
        use crate::coverage::LineCov;
        assert_eq!(
            cov.line_cov(std::path::Path::new("src/x.rs"), 1),
            LineCov::Covered
        );
        assert_eq!(
            cov.line_cov(std::path::Path::new("src/x.rs"), 2),
            LineCov::Uncovered
        );
    }

    #[test]
    fn load_coverage_errors_without_config() {
        let dir = tempdir().unwrap();
        assert!(load_coverage(dir.path()).is_err());
    }

    #[test]
    fn debug_config_round_trips_and_preserves_other_fields() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        // Seed other config fields first.
        save_theme(root, crate::theme::Theme::Light).unwrap();
        save_split(root, true).unwrap();
        // Write a debug block (no public setter yet — go through save_config).
        let mut cfg = load_config(root);
        cfg.debug = DebugConfig {
            adapter: AdapterConfig {
                command: "lldb-dap".into(),
                args: vec!["--port".into(), "0".into()],
            },
            build: "cargo build".into(),
            program: "target/debug/app".into(),
            args: vec!["--flag".into()],
            cwd: ".".into(),
            source_map: vec![("/old".into(), "/new".into())],
            remote: RemoteConfig::default(),
        };
        save_config(root, &cfg).unwrap();

        let loaded = load_debug_config(root);
        assert_eq!(loaded.adapter.command, "lldb-dap");
        assert_eq!(loaded.build, "cargo build");
        assert_eq!(loaded.source_map, vec![("/old".into(), "/new".into())]);
        // Other fields untouched.
        assert_eq!(load_theme(root), crate::theme::Theme::Light);
        assert!(load_split(root));
    }

    #[test]
    fn missing_debug_block_loads_default() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        // Config with only a theme, no debug block.
        std::fs::create_dir_all(worktree_dir(root)).unwrap();
        std::fs::write(
            worktree_dir(root).join("config.json"),
            br#"{"theme":"dark"}"#,
        )
        .unwrap();
        assert_eq!(load_debug_config(root), DebugConfig::default());
    }

    // ─── Config: theme + split_diff round-trip, no clobber ───────────────────

    #[test]
    fn split_round_trips() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        assert!(!load_split(root)); // default
        save_split(root, true).unwrap();
        assert!(load_split(root));
        save_split(root, false).unwrap();
        assert!(!load_split(root));
    }

    #[test]
    fn saving_theme_preserves_split() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        save_split(root, true).unwrap();
        save_theme(root, crate::theme::Theme::Light).unwrap();
        // Theme write must not wipe the split flag.
        assert!(load_split(root));
        assert_eq!(load_theme(root), crate::theme::Theme::Light);
    }

    #[test]
    fn saving_split_preserves_theme() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        save_theme(root, crate::theme::Theme::Light).unwrap();
        save_split(root, true).unwrap();
        assert_eq!(load_theme(root), crate::theme::Theme::Light);
        assert!(load_split(root));
    }

    #[test]
    fn old_theme_only_config_still_loads() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let cfgdir = worktree_dir(root);
        std::fs::create_dir_all(&cfgdir).unwrap();
        // A pre-split config.json with only the theme field.
        std::fs::write(cfgdir.join("config.json"), br#"{"theme":"light"}"#).unwrap();
        assert_eq!(load_theme(root), crate::theme::Theme::Light);
        assert!(!load_split(root)); // missing field defaults to false
        assert_eq!(load_diff_style(root), crate::app::DiffStyle::Dim); // missing -> dim
    }

    #[test]
    fn diff_style_defaults_dim_when_config_missing() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        // No config.json at all -> historical dimmed look is the default.
        assert_eq!(load_diff_style(root), crate::app::DiffStyle::Dim);
    }

    #[test]
    fn diff_style_round_trips_all_states() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        assert_eq!(load_diff_style(root), crate::app::DiffStyle::Dim); // default
        for st in [
            crate::app::DiffStyle::Bright,
            crate::app::DiffStyle::Plain,
            crate::app::DiffStyle::Dim,
        ] {
            save_diff_style(root, st).unwrap();
            assert_eq!(load_diff_style(root), st);
        }
    }

    #[test]
    fn saving_diff_style_preserves_theme_and_split() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        save_theme(root, crate::theme::Theme::Light).unwrap();
        save_split(root, true).unwrap();
        save_diff_style(root, crate::app::DiffStyle::Plain).unwrap();
        assert_eq!(load_theme(root), crate::theme::Theme::Light);
        assert!(load_split(root));
        assert_eq!(load_diff_style(root), crate::app::DiffStyle::Plain);
    }

    #[test]
    fn wrap_lines_defaults_false_and_round_trips() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        assert!(!load_wrap_lines(root)); // default
        save_wrap_lines(root, true).unwrap();
        assert!(load_wrap_lines(root));
        save_wrap_lines(root, false).unwrap();
        assert!(!load_wrap_lines(root));
    }

    #[test]
    fn saving_wrap_lines_preserves_other_fields() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        save_theme(root, crate::theme::Theme::Light).unwrap();
        save_diff_style(root, crate::app::DiffStyle::Plain).unwrap();
        save_wrap_lines(root, true).unwrap();
        assert_eq!(load_theme(root), crate::theme::Theme::Light);
        assert_eq!(load_diff_style(root), crate::app::DiffStyle::Plain);
        assert!(load_wrap_lines(root));
    }

    // ─── TDD: archive_path, append_archive, archive_cutoff_secs ──────────────

    #[test]
    fn archive_path_is_under_dot_turboreview_archive() {
        let root = PathBuf::from("/my/repo");
        let p = archive_path(&root);
        assert_eq!(
            p,
            PathBuf::from("/my/repo/.turboreview/archive/comments-archive.jsonl")
        );
    }

    #[test]
    fn append_archive_writes_one_json_line_per_comment() {
        use crate::comments::{Comment, CommentStatus};
        let dir = tempdir().unwrap();
        let root = dir.path();

        let comments = vec![
            Comment {
                file: std::path::PathBuf::from("a.rs"),
                line: 1,
                hunk: "@@".to_string(),
                text: "note one".to_string(),
                line_text: "fn a()".to_string(),
                context_before: vec![],
                context_after: vec![],
                orig_line: 1,
                stale: false,
                status: CommentStatus::Resolved,
                response: None,
                updated: 1000,
                debug_snapshot: None,
            },
            Comment {
                file: std::path::PathBuf::from("b.rs"),
                line: 5,
                hunk: "@@".to_string(),
                text: "note two".to_string(),
                line_text: "fn b()".to_string(),
                context_before: vec![],
                context_after: vec![],
                orig_line: 5,
                stale: false,
                status: CommentStatus::Resolved,
                response: None,
                updated: 2000,
                debug_snapshot: None,
            },
        ];

        append_archive(root, &comments).unwrap();

        let archive = archive_path(root);
        assert!(archive.exists(), "archive file must exist");
        let contents = std::fs::read_to_string(&archive).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "must write one line per comment");

        // Each line must be valid JSON and contain the comment text
        let v1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v1["text"], "note one");
        let v2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(v2["text"], "note two");
    }

    #[test]
    fn append_archive_is_append_only() {
        use crate::comments::{Comment, CommentStatus};
        let dir = tempdir().unwrap();
        let root = dir.path();

        let c1 = Comment {
            file: std::path::PathBuf::from("a.rs"),
            line: 1,
            hunk: "@@".to_string(),
            text: "first".to_string(),
            line_text: "".to_string(),
            context_before: vec![],
            context_after: vec![],
            orig_line: 1,
            stale: false,
            status: CommentStatus::Resolved,
            response: None,
            updated: 100,
            debug_snapshot: None,
        };
        let c2 = Comment {
            file: std::path::PathBuf::from("b.rs"),
            line: 2,
            hunk: "@@".to_string(),
            text: "second".to_string(),
            line_text: "".to_string(),
            context_before: vec![],
            context_after: vec![],
            orig_line: 2,
            stale: false,
            status: CommentStatus::Resolved,
            response: None,
            updated: 200,
            debug_snapshot: None,
        };

        append_archive(root, &[c1]).unwrap();
        append_archive(root, &[c2]).unwrap();

        let contents = std::fs::read_to_string(archive_path(root)).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "second call must append, not overwrite");
        let v1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v1["text"], "first");
        let v2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(v2["text"], "second");
    }

    #[test]
    fn append_archive_errors_when_path_unwritable() {
        use crate::comments::{Comment, CommentStatus};
        let dir = tempdir().unwrap();
        // Create a FILE where the .turboreview dir should be, so create_dir_all fails.
        std::fs::write(dir.path().join(".turboreview"), b"x").unwrap();
        let c = Comment {
            file: std::path::PathBuf::from("a.rs"),
            line: 1,
            hunk: "@@".to_string(),
            text: "test".to_string(),
            line_text: "fn a()".to_string(),
            context_before: vec![],
            context_after: vec![],
            orig_line: 1,
            stale: false,
            status: CommentStatus::Resolved,
            response: None,
            updated: 1000,
            debug_snapshot: None,
        };
        let res = append_archive(dir.path(), &[c]);
        assert!(
            res.is_err(),
            "append_archive must error when the dir can't be created"
        );
    }

    #[test]
    fn append_archive_empty_slice_does_nothing() {
        use crate::comments::Comment;
        let dir = tempdir().unwrap();
        let root = dir.path();
        append_archive(root, &[] as &[Comment]).unwrap();
        assert!(
            !archive_path(root).exists(),
            "empty slice must not create archive file"
        );
    }

    #[test]
    fn archive_cutoff_secs_is_14_days_before_now() {
        let now = 1_000_000_i64;
        let cutoff = archive_cutoff_secs(now);
        assert_eq!(cutoff, now - 14 * 86400);
    }

    #[test]
    fn worktree_dir_is_dot_turboreview() {
        let root = PathBuf::from("/my/repo");
        assert_eq!(worktree_dir(&root), PathBuf::from("/my/repo/.turboreview"));
    }

    #[test]
    fn commit_dir_is_commits_slash_sha() {
        let root = PathBuf::from("/my/repo");
        assert_eq!(
            commit_dir(&root, "abc123"),
            PathBuf::from("/my/repo/.turboreview/commits/abc123")
        );
    }

    #[test]
    fn save_then_load_theme_round_trips_light() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        save_theme(root, crate::theme::Theme::Light).unwrap();
        let loaded = load_theme(root);
        assert_eq!(loaded, crate::theme::Theme::Light);
    }

    #[test]
    fn save_then_load_theme_round_trips_dark() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        save_theme(root, crate::theme::Theme::Dark).unwrap();
        let loaded = load_theme(root);
        assert_eq!(loaded, crate::theme::Theme::Dark);
    }

    #[test]
    fn load_theme_missing_file_returns_dark() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        // No config.json created — should default to Dark
        let loaded = load_theme(root);
        assert_eq!(loaded, crate::theme::Theme::Dark);
    }

    #[test]
    fn append_comment_log_writes_two_valid_json_lines() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        append_comment_log(root, Path::new("src/main.rs"), 42, "worktree", "set").unwrap();
        append_comment_log(
            root,
            Path::new("src/lib.rs"),
            10,
            "commit:deadbeef",
            "remove",
        )
        .unwrap();

        let log_path = root.join(".turboreview/comment-log.jsonl");
        assert!(log_path.exists(), "log file should exist");

        let contents = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "should have 2 lines");

        // Parse each line as JSON and verify fields
        let entry1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(entry1["path"], "src/main.rs");
        assert_eq!(entry1["line"], 42);
        assert_eq!(entry1["scope"], "worktree");
        assert_eq!(entry1["action"], "set");
        let date_str = entry1["date"].as_str().expect("date should be a string");
        // date now holds YYYY-MM-DD HH:MM:SS (19 chars)
        assert_eq!(
            date_str.len(),
            19,
            "date field must be YYYY-MM-DD HH:MM:SS (19 chars): {}",
            date_str
        );
        assert!(
            date_str.contains(' '),
            "date field must contain a space separating date and time"
        );
        assert_eq!(
            date_str.chars().filter(|&c| c == ':').count(),
            2,
            "date field must have two colons for HH:MM:SS"
        );

        let entry2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(entry2["path"], "src/lib.rs");
        assert_eq!(entry2["line"], 10);
        assert_eq!(entry2["scope"], "commit:deadbeef");
        assert_eq!(entry2["action"], "remove");
    }
}
