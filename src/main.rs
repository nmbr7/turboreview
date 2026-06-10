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

use turboreview::app::{App, Mode, Pane};
use turboreview::git::Repo;
use turboreview::{review, ui};

fn main() -> Result<()> {
    let repo_arg = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let repo = Repo::discover(&PathBuf::from(&repo_arg))?;
    let root = repo.workdir()?;

    let files = repo.changed_files(Mode::Unstaged)?;
    let mut app = App::new(files, root.clone());
    app.reviewed = review::load(&root)?;
    refresh_diff(&repo, &mut app);

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, &repo, &mut app);
    let restore = restore_terminal(&mut terminal);
    result.and(restore)
}

fn refresh_diff(repo: &Repo, app: &mut App) {
    match app.selected_path() {
        Some(path) => {
            let path = path.clone();
            match repo.diff_for(&path, app.mode) {
                Ok(lines) => {
                    app.status_msg = None;
                    app.set_diff(lines);
                }
                Err(e) => {
                    app.status_msg = Some(format!("diff error: {e}"));
                    app.set_diff(Vec::new());
                }
            }
        }
        None => app.set_diff(Vec::new()),
    }
}

fn reload_files(repo: &Repo, app: &mut App) {
    match repo.changed_files(app.mode) {
        Ok(files) => {
            app.files = files;
            app.rebuild_rows(); // rebuilds rows and clamps selected
        }
        Err(e) => app.status_msg = Some(format!("list error: {e}")),
    }
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
                        app.toggle_mode();
                        reload_files(repo, app);
                    }
                    (KeyCode::Char(' '), _) => {
                        app.toggle_reviewed();
                        if let Err(e) = review::save(&app.repo_root, &app.reviewed) {
                            app.status_msg = Some(format!("save error: {e}"));
                        }
                    }
                    (KeyCode::Char('G'), _) => app.to_bottom(),
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => move_in_focus(repo, app, -1),
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => move_in_focus(repo, app, 1),
                    (KeyCode::Enter, _) => {
                        if app.focus == Pane::Files {
                            app.toggle_collapse();
                            // Only reload the diff if the selection now sits on a
                            // file; collapsing a dir shouldn't wipe the visible diff.
                            if app.selected_path().is_some() {
                                refresh_diff(repo, app);
                            }
                        }
                    }
                    (KeyCode::Char('l'), _) | (KeyCode::Right, _) => {
                        if app.focus == Pane::Diff { app.scroll_h(1); }
                    }
                    (KeyCode::Char('h'), _) | (KeyCode::Left, _) => {
                        if app.focus == Pane::Diff { app.scroll_h(-1); }
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
            app.move_selection(delta);
            refresh_diff(repo, app);
        }
        Pane::Diff => app.scroll_diff(delta),
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
