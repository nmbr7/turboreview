use std::io::{self, Stdout};
use std::path::PathBuf;

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

use turboreview::app::{App, Mode, Pane, Section};
use turboreview::comments;
use turboreview::git::Repo;
use turboreview::{review, ui};

fn main() -> Result<()> {
    let repo_arg = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let repo = Repo::discover(&PathBuf::from(&repo_arg))?;
    let root = repo.workdir()?;

    let unstaged = repo.changed_files(Mode::Unstaged)?;
    let staged = repo.changed_files(Mode::Staged)?;
    let mut app = App::new(unstaged, staged, root.clone());
    app.reviewed = review::load(&root)?;
    app.comments = comments::Comments::load(&root).unwrap_or_default();
    refresh_diff(&repo, &mut app);

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, &repo, &mut app);
    let restore = restore_terminal(&mut terminal);
    result.and(restore)
}

fn refresh_diff(repo: &Repo, app: &mut App) {
    match (app.selected_path(), app.selected_section()) {
        (Some(path), Some(section)) => {
            let path = path.clone();
            let mode = match section {
                Section::Unstaged => Mode::Unstaged,
                Section::Staged => Mode::Staged,
            };
            match repo.diff_for(&path, mode, app.effective_context()) {
                Ok(lines) => {
                    app.status_msg = None;
                    app.set_diff(lines);
                    // Relocate comments for this file against the fresh diff.
                    let candidates: Vec<(u32, String)> = app.diff.iter()
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
    app.rebuild_rows();
    refresh_diff(repo, app);
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
                            if let Some((file, line, hunk, text)) = app.input_commit() {
                                let trimmed = text.trim().to_string();
                                if trimmed.is_empty() {
                                    app.comments.remove(&file, line);
                                } else {
                                    let (line_text, ctx_before, ctx_after) = app.comment_anchor();
                                    app.comments.set(file, line, hunk, trimmed, line_text, ctx_before, ctx_after);
                                }
                                if let Err(e) = app.comments.save(&app.repo_root) {
                                    app.status_msg = Some(format!("comment save error: {e}"));
                                }
                            }
                        }
                        KeyCode::Enter => app.input_newline(),
                        KeyCode::Backspace => app.input_backspace(),
                        KeyCode::Char(c) => app.input_push(c),
                        _ => {}
                    }
                    continue;
                }

                if matches!(key.code, KeyCode::Char('g')) && key.modifiers.is_empty() {
                    if pending_g {
                        app.to_top();
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
                        app.toggle_reviewed();
                        if let Err(e) = review::save(&app.repo_root, &app.reviewed) {
                            app.status_msg = Some(format!("save error: {e}"));
                        }
                        refresh_diff(repo, app);
                    }
                    (KeyCode::Char('G'), _) => app.to_bottom(),
                    (KeyCode::Char('R'), _) => {
                        app.toggle_hide_reviewed();
                        refresh_diff(repo, app);
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => move_in_focus(repo, app, -1),
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => move_in_focus(repo, app, 1),
                    (KeyCode::Enter, _) => {
                        if app.focus == Pane::Files {
                            if app.selected_path().is_some() {
                                app.focus = Pane::Diff; // jump into the diff to read this file
                            } else {
                                app.toggle_collapse(); // dir row: fold/unfold (no-op on header)
                                if app.selected_path().is_some() {
                                    refresh_diff(repo, app);
                                }
                            }
                        }
                    }
                    (KeyCode::Esc, _) => app.focus = Pane::Files,
                    (KeyCode::Char('F'), _) => {
                        app.toggle_full_file();
                        refresh_diff(repo, app);
                    }
                    (KeyCode::Char('l'), _) | (KeyCode::Right, _) => {
                        if app.focus == Pane::Diff { app.scroll_h(1); }
                    }
                    (KeyCode::Char('h'), _) | (KeyCode::Left, _) => {
                        if app.focus == Pane::Diff { app.scroll_h(-1); }
                    }
                    (KeyCode::Char('+'), _) | (KeyCode::Char('='), _) => {
                        app.inc_context();
                        refresh_diff(repo, app);
                    }
                    (KeyCode::Char('-'), _) => {
                        app.dec_context();
                        refresh_diff(repo, app);
                    }
                    (KeyCode::Char('z'), _) => app.toggle_files(),
                    (KeyCode::Char('>'), _) | (KeyCode::Char('.'), _) => app.widen_files(),
                    (KeyCode::Char('<'), _) | (KeyCode::Char(','), _) => app.narrow_files(),
                    // c (no modifier) opens comment modal; Ctrl-C is already handled above
                    (KeyCode::Char('c'), _) => app.start_comment(),
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
            app.move_selection(delta);
            refresh_diff(repo, app);
        }
        Pane::Diff => app.move_diff_cursor(delta),
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
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}
