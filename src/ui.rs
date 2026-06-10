use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, LineKind, Mode, Pane};
use crate::highlight::highlight_code;

const SELECTED_BG: Color = Color::Rgb(60, 60, 90);
const ADD_BG: Color = Color::Rgb(20, 50, 20);
const DEL_BG: Color = Color::Rgb(55, 20, 20);

pub fn render(frame: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(outer[0]);

    render_files(frame, app, panes[0]);
    render_diff(frame, app, panes[1]);
    render_status(frame, app, outer[1]);
}

fn focused_border(app: &App, pane: Pane) -> Style {
    if app.focus == pane {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn render_files(frame: &mut Frame, app: &App, area: Rect) {
    let mode = match app.mode {
        Mode::Staged => "STAGED",
        Mode::Unstaged => "UNSTAGED",
    };
    let items: Vec<ListItem> = app
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let check = if app.is_reviewed(i) { "[x] " } else { "[ ] " };
            let mut style = Style::default();
            if i == app.selected {
                style = style.bg(SELECTED_BG).add_modifier(Modifier::BOLD);
            }
            ListItem::new(Line::from(format!("{}{}", check, f.path.display()))).style(style)
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(focused_border(app, Pane::Files))
            .title(format!(" Files [{}] ", mode)),
    );
    frame.render_widget(list, area);
}

fn render_diff(frame: &mut Frame, app: &App, area: Rect) {
    let ext = app
        .selected_path()
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();

    let title = app
        .selected_path()
        .map(|p| format!(" Diff: {} ", p.display()))
        .unwrap_or_else(|| " Diff ".to_string());

    let lines: Vec<Line> = if app.diff.is_empty() {
        vec![Line::from(Span::styled(
            "No changes",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.diff
            .iter()
            .skip(app.diff_scroll)
            .take(area.height.saturating_sub(2) as usize)
            .map(|dl| {
                let bg = match dl.kind {
                    LineKind::Add => Some(ADD_BG),
                    LineKind::Del => Some(DEL_BG),
                    _ => None,
                };
                if dl.kind == LineKind::Hunk {
                    return Line::from(Span::styled(
                        dl.text.clone(),
                        Style::default().fg(Color::Cyan),
                    ));
                }
                let mut spans: Vec<Span> = highlight_code(&dl.text, &ext);
                if let Some(bg) = bg {
                    for s in spans.iter_mut() {
                        s.style = s.style.bg(bg);
                    }
                }
                Line::from(spans)
            })
            .collect()
    };

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(focused_border(app, Pane::Diff))
            .title(title),
    );
    frame.render_widget(para, area);
}

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let base = "Tab:focus  s:staged  Space:review  up/down/jk:move  gg/G  q:quit";
    let text = match &app.status_msg {
        Some(msg) => format!("{}   |   {}", base, msg),
        None => base.to_string(),
    };
    let para = Paragraph::new(text).style(Style::default().fg(Color::Gray));
    frame.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, DiffLine, FileChange, Status};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn app_with_diff() -> App {
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, PathBuf::from("/repo"));
        app.set_diff(vec![
            DiffLine { kind: LineKind::Hunk, text: "@@ -1 +1 @@".into(), old_lineno: None, new_lineno: None },
            DiffLine { kind: LineKind::Add, text: "let x = 1;".into(), old_lineno: None, new_lineno: Some(1) },
        ]);
        app
    }

    #[test]
    fn render_does_not_panic_and_shows_file() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = app_with_diff();
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let dump: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("a.rs"));
        assert!(dump.contains("[ ]"));
    }

    #[test]
    fn scroll_offset_hides_earlier_lines() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = app_with_diff();
        // diff is [Hunk "@@ -1 +1 @@", Add "let x = 1;"]; scroll past the hunk header.
        app.diff_scroll = 1;
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(!dump.contains("@@ -1 +1 @@"));
        assert!(dump.contains("let x = 1;"));
    }

    #[test]
    fn empty_diff_shows_placeholder() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let app = App::new(files, PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("No changes"));
    }
}
