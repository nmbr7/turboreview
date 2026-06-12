use std::io::{self, Stdout};
use std::path::{Path, PathBuf};

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;

use turboreview::app::{App, CommentScope, Mode, Pane, Section, ViewMode};
use turboreview::comments;
use turboreview::git::Repo;
use turboreview::{review, storage, ui};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--skill") {
        print!("{}", turboreview::skill::SKILL_DOC);
        return Ok(());
    }
    let repo_arg = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| ".".to_string());
    let repo = Repo::discover(&PathBuf::from(&repo_arg))?;
    let root = repo.workdir()?;

    let unstaged = repo.changed_files(Mode::Unstaged)?;
    let staged = repo.changed_files(Mode::Staged)?;
    let mut app = App::new(unstaged, staged, root.clone());
    // Startup: load from worktree scope
    let wt_dir = storage::worktree_dir(&root);
    app.reviewed = review::load(&wt_dir).unwrap_or_default();
    app.comments = comments::Comments::load(&wt_dir).unwrap_or_default();
    // Auto-archive resolved comments older than 14 days on startup
    let cutoff = storage::archive_cutoff_secs(storage::now_secs());
    let old = app.comments.drain_resolved_older_than(cutoff);
    if !old.is_empty() {
        match storage::append_archive(&root, &old) {
            Ok(()) => {
                let _ = app.comments.save(&wt_dir);
            }
            Err(_) => {
                // Archive write failed — put the drained comments back so they are not lost.
                app.comments.items.extend(old);
            }
        }
    }
    app.commits = repo.log(200).unwrap_or_default();
    // Load persisted theme preference
    app.theme = storage::load_theme(&root);
    refresh_diff(&repo, &mut app);

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, &repo, &mut app);
    let restore = restore_terminal(&mut terminal);
    result.and(restore)
}

/// Load the comments and reviewed set for the current scope into `app`.
fn load_scope(repo_root: &Path, app: &mut App) {
    let dir = storage::scope_dir(repo_root, &app.comment_scope);
    app.comments = comments::Comments::load(&dir).unwrap_or_default();
    app.reviewed = review::load(&dir).unwrap_or_default();
}

/// Re-sync the comment scope to the current history revision (or the baseline),
/// load that scope's comments/reviewed, and refresh the diff. Call after any
/// change to `history.idx`.
fn sync_history_scope(repo: &Repo, app: &mut App) {
    let anchor = app.history_active().then(|| app.cursor_lineno()).flatten();
    match app.history_current_commit() {
        Some(commit) => app.comment_scope = CommentScope::Commit(commit.id.clone()),
        None => {
            // Back at baseline (idx 0): restore whatever scope History saved.
            if let Some(h) = app.history.as_ref() {
                app.comment_scope = h.baseline_scope.clone();
            }
        }
    }
    let root = app.repo_root.clone();
    load_scope(&root, app);
    refresh_diff_preserving_line(repo, app, anchor);
}

/// Reload the diff, keeping the cursor on `anchor` when possible. Increases context
/// in steps of 5 (up to [`App::MAX_CONTEXT_LINES`]), then full-file, until visible.
fn refresh_diff_preserving_line(repo: &Repo, app: &mut App, anchor: Option<u32>) {
    let Some(lineno) = anchor else {
        refresh_diff(repo, app);
        return;
    };
    loop {
        refresh_diff(repo, app);
        if app.diff_has_lineno(lineno) {
            app.move_cursor_to_lineno(lineno);
            return;
        }
        if app.full_file {
            app.move_cursor_to_lineno(lineno);
            return;
        }
        if app.context_lines >= App::MAX_CONTEXT_LINES {
            app.full_file = true;
            continue;
        }
        app.context_lines = (app.context_lines + 5).min(App::MAX_CONTEXT_LINES);
    }
}

fn refresh_diff(repo: &Repo, app: &mut App) {
    // File-history overlay: when showing a past revision (idx >= 1), render that
    // commit's diff for the history file. idx 0 falls through to the baseline branches.
    if let Some(commit) = app.history_current_commit() {
        let id = commit.id.clone();
        let file = app.history.as_ref().unwrap().file.clone();
        match repo.commit_diff_for(&id, &file, app.effective_context()) {
            Ok(lines) => {
                app.status_msg = None;
                app.set_diff(lines);
                let candidates: Vec<(u32, String)> = app
                    .diff
                    .iter()
                    .filter_map(|l| l.new_lineno.map(|n| (n, l.text.trim().to_string())))
                    .collect();
                app.comments.relocate_file(&file, &candidates);
            }
            Err(e) => {
                app.status_msg = Some(format!("diff error: {e}"));
                app.set_diff(Vec::new());
            }
        }
        return;
    }

    // Commit-detail: use commit_diff_for instead of working-tree diff.
    if app.in_commit_detail() {
        if let (Some(path), Some(id)) = (app.selected_path().cloned(), app.open_commit.clone()) {
            match repo.commit_diff_for(&id, &path, app.effective_context()) {
                Ok(lines) => {
                    app.status_msg = None;
                    app.set_diff(lines);
                    let candidates: Vec<(u32, String)> = app
                        .diff
                        .iter()
                        .filter_map(|l| l.new_lineno.map(|n| (n, l.text.trim().to_string())))
                        .collect();
                    app.comments.relocate_file(&path, &candidates);
                }
                Err(e) => {
                    app.status_msg = Some(format!("diff error: {e}"));
                    app.set_diff(Vec::new());
                }
            }
        } else {
            app.set_diff(Vec::new());
        }
        return;
    }

    // Working-tree diff.
    match (app.selected_path(), app.selected_section()) {
        (Some(path), Some(section)) => {
            let path = path.clone();
            let mode = match section {
                Section::Unstaged => Mode::Unstaged,
                Section::Staged => Mode::Staged,
                Section::Commit => return, // unreachable in working-tree branch
            };
            match repo.diff_for(&path, mode, app.effective_context()) {
                Ok(lines) => {
                    app.status_msg = None;
                    app.set_diff(lines);
                    // Relocate comments for this file against the fresh diff.
                    let candidates: Vec<(u32, String)> = app
                        .diff
                        .iter()
                        .filter_map(|l| l.new_lineno.map(|n| (n, l.text.trim().to_string())))
                        .collect();
                    app.comments.relocate_file(&path, &candidates);
                }
                Err(e) => {
                    app.status_msg = Some(format!("diff error: {e}"));
                    app.set_diff(Vec::new());
                }
            }
        }
        _ => app.set_diff(Vec::new()),
    }
}

fn reload_all(repo: &Repo, app: &mut App) {
    let unstaged = match repo.changed_files(Mode::Unstaged) {
        Ok(f) => f,
        Err(e) => {
            app.status_msg = Some(format!("list error: {e}"));
            return;
        }
    };
    let staged = match repo.changed_files(Mode::Staged) {
        Ok(f) => f,
        Err(e) => {
            app.status_msg = Some(format!("list error: {e}"));
            return;
        }
    };
    app.unstaged = unstaged;
    app.staged = staged;
    load_scope(&app.repo_root.clone(), app);
    app.rebuild_rows();
    refresh_diff(repo, app);
}

/// Reload everything from disk/git so new external changes appear.
fn reload_everything(repo: &Repo, app: &mut App) {
    // Reload file lists (on error set status_msg but keep going)
    match repo.changed_files(Mode::Unstaged) {
        Ok(f) => app.unstaged = f,
        Err(e) => app.status_msg = Some(format!("list error: {e}")),
    }
    match repo.changed_files(Mode::Staged) {
        Ok(f) => app.staged = f,
        Err(e) => app.status_msg = Some(format!("list error: {e}")),
    }
    // Reload commits
    app.commits = repo.log(200).unwrap_or_default();
    // If in a commit detail view, refresh that commit's files
    if app.in_commit_detail() {
        if let Some(id) = app.open_commit.clone() {
            app.commit_files = repo.commit_files(&id).unwrap_or_default();
        }
    }
    // Reload reviewed set and comments from the CURRENT scope
    load_scope(&app.repo_root.clone(), app);
    // Clamp comment_selected so it can't dangle past a now-shorter comment list.
    let clen = app.comment_rows().len();
    app.comment_selected = app.comment_selected.min(clen.saturating_sub(1));
    // Rebuild rows and refresh diff
    app.rebuild_rows();
    refresh_diff(repo, app);
    app.status_msg = Some("refreshed".into());
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    repo: &Repo,
    app: &mut App,
) -> Result<()> {
    let mut pending_g = false; // for the `gg` chord
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // When the comment input modal is active, route all keys to the
                // editor and skip normal key handling entirely.
                if app.input_active() {
                    match key.code {
                        KeyCode::Esc => app.input_cancel(),
                        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            // FIX 4: anchor comes from CommittedComment (captured at start_comment time).
                            if let Some(committed) = app.input_commit() {
                                let trimmed = committed.text.trim().to_string();
                                let action = if trimmed.is_empty() { "remove" } else { "set" };
                                if trimmed.is_empty() {
                                    app.comments.remove(&committed.file, committed.line);
                                } else {
                                    app.comments.set(
                                        committed.file.clone(),
                                        committed.line,
                                        committed.hunk.clone(),
                                        trimmed,
                                        committed.line_text.clone(),
                                        committed.context_before.clone(),
                                        committed.context_after.clone(),
                                        storage::now_secs(),
                                    );
                                }
                                // Save to the current scope directory
                                let scope_dir =
                                    storage::scope_dir(&app.repo_root, &app.comment_scope);
                                if let Err(e) = app.comments.save(&scope_dir) {
                                    app.status_msg = Some(format!("comment save error: {e}"));
                                }
                                // Append to the comment log (best-effort)
                                let scope_label = app.scope_label();
                                let _ = storage::append_comment_log(
                                    &app.repo_root,
                                    &committed.file,
                                    committed.line,
                                    &scope_label,
                                    action,
                                );
                            }
                        }
                        KeyCode::Enter => app.input_newline(),
                        KeyCode::Backspace => app.input_backspace(),
                        KeyCode::Char(c) => app.input_push(c),
                        _ => {}
                    }
                    continue;
                }

                // When the search input line is open, route keys to it.
                if app.search_input_active() {
                    match key.code {
                        KeyCode::Esc => app.search_input_cancel(),
                        KeyCode::Enter => {
                            if !app.search_commit() {
                                // search_commit already cleared `search`; tell the user.
                                app.status_msg = Some("no match".into());
                            }
                        }
                        KeyCode::Backspace => app.search_input_backspace(),
                        KeyCode::Char(c) => app.search_input_push(c),
                        _ => {}
                    }
                    continue;
                }

                // Help overlay: while open, any key closes it (swallow).
                if app.show_help {
                    app.show_help = false;
                    continue;
                }

                if matches!(key.code, KeyCode::Char('g')) && key.modifiers.is_empty() {
                    if pending_g {
                        if app.view == ViewMode::Commits
                            && app.open_commit.is_none()
                            && app.focus == Pane::Files
                        {
                            app.selected_commit = 0;
                        } else if app.focus == Pane::Comments {
                            app.comment_selected = 0;
                        } else {
                            app.to_top();
                        }
                        pending_g = false;
                    } else {
                        pending_g = true;
                    }
                    continue;
                }
                pending_g = false;

                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _) => return Ok(()),
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(()),
                    (KeyCode::Tab, _) => app.toggle_focus(),
                    (KeyCode::Char('s'), _) => {
                        if app.focus == Pane::Files {
                            if let (Some(path), Some(section)) =
                                (app.selected_path().cloned(), app.selected_section())
                            {
                                let result = match section {
                                    Section::Unstaged => repo.stage_file(&path),
                                    Section::Staged => repo.unstage_file(&path),
                                    // Commit-detail files cannot be staged/unstaged.
                                    Section::Commit => continue,
                                };
                                match result {
                                    Ok(()) => {
                                        app.status_msg = None;
                                        reload_all(repo, app);
                                    }
                                    Err(e) => {
                                        app.status_msg = Some(format!("stage error: {e}"));
                                    }
                                }
                            }
                            // If a Header or Dir is selected, s does nothing
                        }
                        // s in Diff pane is deferred (does nothing this phase)
                    }
                    (KeyCode::Char(' '), _) => {
                        // In Commits-list mode (not in detail), rows are stale working-tree rows;
                        // toggle_reviewed would act on the wrong file, so ignore.
                        if !(app.view == ViewMode::Commits && !app.in_commit_detail()) {
                            app.toggle_reviewed();
                            // Save reviewed to the current scope directory
                            let scope_dir = storage::scope_dir(&app.repo_root, &app.comment_scope);
                            if let Err(e) = review::save(&scope_dir, &app.reviewed) {
                                app.status_msg = Some(format!("save error: {e}"));
                            }
                            refresh_diff(repo, app);
                        }
                    }
                    (KeyCode::Char('G'), _) => {
                        if app.view == ViewMode::Commits
                            && app.open_commit.is_none()
                            && app.focus == Pane::Files
                        {
                            app.selected_commit = app.commits.len().saturating_sub(1);
                        } else if app.focus == Pane::Comments {
                            let len = app.comment_rows().len();
                            app.comment_selected = len.saturating_sub(1);
                        } else {
                            app.to_bottom();
                        }
                    }
                    (KeyCode::Char('C'), _) => app.toggle_comment_pane(),
                    (KeyCode::Char('R'), _) => {
                        app.toggle_hide_reviewed();
                        refresh_diff(repo, app);
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => move_in_focus(repo, app, -1),
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => move_in_focus(repo, app, 1),
                    (KeyCode::Enter, _) => {
                        if app.focus == Pane::Comments {
                            // Jump to the selected comment's file and line in the diff.
                            if let Some(c) = app.selected_comment().cloned() {
                                if app.select_row_for_path(&c.file) {
                                    refresh_diff(repo, app);
                                    app.move_cursor_to_line(c.line);
                                    app.focus = Pane::Diff;
                                } else {
                                    app.status_msg =
                                        Some("comment's file not in current view".into());
                                }
                            }
                        } else if app.focus == Pane::Files {
                            if app.view == ViewMode::Commits && !app.in_commit_detail() {
                                // Commit list: Enter drills into the selected commit.
                                if let Some(ci) = app.selected_commit_info() {
                                    let id = ci.id.clone();
                                    match repo.commit_files(&id) {
                                        Ok(files) => {
                                            app.status_msg = None;
                                            app.open_commit(id.clone(), files);
                                            // Load this commit's scope data
                                            load_scope(&app.repo_root.clone(), app);
                                            refresh_diff(repo, app);
                                        }
                                        Err(e) => {
                                            app.status_msg =
                                                Some(format!("commit files error: {e}"));
                                        }
                                    }
                                }
                            } else if app.selected_path().is_some() {
                                // File row (either Changes view or commit-detail): jump to diff.
                                app.focus = Pane::Diff;
                            } else {
                                // Dir/header row: fold/unfold.
                                app.toggle_collapse();
                                if app.selected_path().is_some() {
                                    refresh_diff(repo, app);
                                }
                            }
                        }
                    }
                    (KeyCode::Esc, _) => {
                        if app.history_active() && app.focus == Pane::Diff {
                            app.exit_history();
                            let root = app.repo_root.clone();
                            load_scope(&root, app);
                            refresh_diff(repo, app);
                        } else if app.in_commit_detail() && app.focus == Pane::Diff {
                            // Inside a commit's file diff: step back to that commit's
                            // file list, not all the way out to the commit list.
                            app.focus = Pane::Files;
                        } else if app.in_commit_detail() {
                            // On the commit's file list: back out to the commit list.
                            app.close_commit();
                            // Reload worktree scope after leaving commit detail
                            load_scope(&app.repo_root.clone(), app);
                        } else {
                            app.focus = Pane::Files;
                        }
                    }
                    (KeyCode::Char('F'), _) => {
                        app.toggle_full_file();
                        refresh_diff(repo, app);
                    }
                    (KeyCode::Char('H'), _) => {
                        if app.focus == Pane::Diff {
                            if app.history_active() {
                                // Toggle off: exit and reload baseline scope/diff.
                                app.exit_history();
                                let root = app.repo_root.clone();
                                load_scope(&root, app);
                                refresh_diff(repo, app);
                            } else if let Some(file) = app.selected_path().cloned() {
                                match repo.file_history(&file, App::MAX_FILE_HISTORY) {
                                    Ok(commits) => {
                                        if app.start_history(commits) {
                                            sync_history_scope(repo, app);
                                        } else {
                                            app.status_msg =
                                                Some(format!("no history for {}", file.display()));
                                        }
                                    }
                                    Err(e) => {
                                        app.status_msg = Some(format!("history error: {e}"));
                                    }
                                }
                            }
                        }
                    }
                    (KeyCode::Char('{'), _) => {
                        if app.history_active() {
                            app.history_step(1); // older
                            sync_history_scope(repo, app);
                        }
                    }
                    (KeyCode::Char('}'), _) => {
                        if app.history_active() {
                            app.history_step(-1); // newer
                            sync_history_scope(repo, app);
                        }
                    }
                    (KeyCode::Char('l'), _) | (KeyCode::Right, _) => {
                        if app.focus == Pane::Diff {
                            app.scroll_h(1);
                        }
                    }
                    (KeyCode::Char('h'), _) | (KeyCode::Left, _) => {
                        if app.focus == Pane::Diff {
                            app.scroll_h(-1);
                        }
                    }
                    (KeyCode::Char('+'), _) | (KeyCode::Char('='), _) => {
                        app.inc_context();
                        refresh_diff(repo, app);
                    }
                    (KeyCode::Char('-'), _) => {
                        app.dec_context();
                        refresh_diff(repo, app);
                    }
                    (KeyCode::Char(']'), _) => {
                        if app.history_active() {
                            app.exit_history();
                        }
                        let was_in_commit = app.in_commit_detail();
                        app.next_view();
                        // If we were in a commit detail and left, reload worktree scope
                        if was_in_commit {
                            load_scope(&app.repo_root.clone(), app);
                        }
                    }
                    (KeyCode::Char('['), _) => {
                        if app.history_active() {
                            app.exit_history();
                        }
                        let was_in_commit = app.in_commit_detail();
                        app.prev_view();
                        // If we were in a commit detail and left, reload worktree scope
                        if was_in_commit {
                            load_scope(&app.repo_root.clone(), app);
                        }
                    }
                    // a — smart fold-all: collapse every dir, or expand all if all collapsed.
                    (KeyCode::Char('a'), _) => {
                        app.toggle_fold_all();
                        refresh_diff(repo, app);
                    }
                    (KeyCode::Char('z'), _) => app.toggle_files(),
                    (KeyCode::Char('>'), _) | (KeyCode::Char('.'), _) => app.widen_files(),
                    (KeyCode::Char('<'), _) | (KeyCode::Char(','), _) => app.narrow_files(),
                    // r (lowercase) refreshes everything from disk/git; R (uppercase) hides reviewed.
                    (KeyCode::Char('r'), KeyModifiers::NONE) => reload_everything(repo, app),
                    // T toggles light / dark theme and persists the choice
                    (KeyCode::Char('T'), _) => {
                        app.toggle_theme();
                        let _ = storage::save_theme(&app.repo_root, app.theme);
                    }
                    // A (capital) — archive all resolved comments in the current scope
                    (KeyCode::Char('A'), _) => {
                        // Peek the resolved comments WITHOUT draining yet.
                        let resolved: Vec<_> = app.comments.items.iter()
                            .filter(|c| c.status == comments::CommentStatus::Resolved)
                            .cloned()
                            .collect();
                        if resolved.is_empty() {
                            app.status_msg = Some("no resolved comments to archive".into());
                        } else {
                            // Archive FIRST. Only mutate the active set if the archive write succeeded.
                            match storage::append_archive(&app.repo_root, &resolved) {
                                Ok(()) => {
                                    let n = app.comments.drain_resolved().len();
                                    let dir = storage::scope_dir(&app.repo_root, &app.comment_scope);
                                    match app.comments.save(&dir) {
                                        Ok(()) => {
                                            let clen = app.comment_rows().len();
                                            app.comment_selected = app.comment_selected.min(clen.saturating_sub(1));
                                            app.status_msg = Some(format!("archived {n} resolved comment(s)"));
                                        }
                                        Err(e) => app.status_msg = Some(format!("archive save error: {e}")),
                                    }
                                }
                                Err(e) => app.status_msg = Some(format!("archive error: {e}")),
                            }
                        }
                    }
                    // ? toggles the help overlay
                    (KeyCode::Char('?'), _) => app.toggle_help(),
                    (KeyCode::Char('/'), _) => {
                        if app.focus == Pane::Diff {
                            app.search_start();
                        }
                    }
                    (KeyCode::Char('n'), _) => {
                        if app.search_active() {
                            app.search_next(1);
                        }
                    }
                    (KeyCode::Char('N'), _) => {
                        if app.search_active() {
                            app.search_next(-1);
                        }
                    }
                    // c (no modifier) opens comment modal; Ctrl-C is already handled above.
                    // In Commits-list mode (not in detail), rows are stale; ignore.
                    (KeyCode::Char('c'), _) => {
                        if !(app.view == ViewMode::Commits && !app.in_commit_detail()) {
                            app.start_comment();
                        }
                    }
                    _ => {}
                }
            }
            Event::Mouse(m) => match m.kind {
                MouseEventKind::ScrollUp => move_in_focus(repo, app, -1),
                MouseEventKind::ScrollDown => move_in_focus(repo, app, 1),
                _ => {}
            },
            _ => {}
        }
    }
}

fn move_in_focus(repo: &Repo, app: &mut App, delta: isize) {
    match app.focus {
        Pane::Files => {
            if app.view == ViewMode::Commits && app.open_commit.is_none() {
                // Commit list: move commit selection.
                app.move_commit_selection(delta);
            } else {
                // Changes view or commit-detail: move file row selection.
                if app.history_active() {
                    app.exit_history();
                    let root = app.repo_root.clone();
                    load_scope(&root, app);
                }
                app.move_selection(delta);
                refresh_diff(repo, app);
            }
        }
        Pane::Diff => app.move_diff_cursor(delta),
        Pane::Comments => app.move_comment_selection(delta),
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        hook(info);
    }));
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}
