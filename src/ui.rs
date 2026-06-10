use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, LineKind, Mode, Pane};
use crate::highlight::highlight_code;
use crate::theme;
use crate::tree::RowKind;

fn gutter(dl: &crate::app::DiffLine) -> String {
    let n = dl.new_lineno.or(dl.old_lineno);
    match n {
        Some(n) => format!("{:>4} ", n),
        None => "     ".to_string(),
    }
}

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
        Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::ACCENT_DIM)
    }
}

fn render_files(frame: &mut Frame, app: &App, area: Rect) {
    let mode = match app.mode {
        Mode::Staged => "STAGED",
        Mode::Unstaged => "UNSTAGED",
    };
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let indent = "  ".repeat(row.depth);
            let mut style = Style::default();
            if i == app.selected {
                style = style.bg(theme::SELECTED_BG).add_modifier(Modifier::BOLD);
            }
            let text = match &row.kind {
                RowKind::Dir { collapsed, .. } => {
                    let glyph = if *collapsed { "▸" } else { "▾" };
                    format!("{}{} {}", indent, glyph, row.name)
                }
                RowKind::File { file_index } => {
                    let check = if app.is_reviewed(*file_index) { "[x] " } else { "[ ] " };
                    let icon = crate::icons::icon_for(&app.files[*file_index].path);
                    format!("{}{}{} {}", indent, check, icon, row.name)
                }
            };
            ListItem::new(Line::from(text)).style(style)
        })
        .collect();
    let title = if app.hide_reviewed {
        format!(" Files [{}] (hiding reviewed) ", mode)
    } else {
        format!(" Files [{}] ", mode)
    };
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(focused_border(app, Pane::Files))
            .title(title),
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
            Style::default().fg(theme::PLACEHOLDER),
        ))]
    } else {
        app.diff
            .iter()
            .skip(app.diff_scroll)
            .take(area.height.saturating_sub(2) as usize)
            .map(|dl| {
                let bg = match dl.kind {
                    LineKind::Add => Some(theme::ADD_BG),
                    LineKind::Del => Some(theme::DEL_BG),
                    _ => None,
                };
                if dl.kind == LineKind::Hunk {
                    let shifted: String = dl.text.chars().skip(app.diff_hscroll).collect();
                    return Line::from(Span::styled(
                        shifted,
                        Style::default().fg(theme::HUNK),
                    ));
                }
                let gutter_span = Span::styled(
                    gutter(dl),
                    Style::default().fg(theme::ACCENT_DIM),
                );
                let shifted: String = dl.text.chars().skip(app.diff_hscroll).collect();
                let mut spans: Vec<Span> = highlight_code(&shifted, &ext);
                if let Some(bg) = bg {
                    for s in spans.iter_mut() {
                        s.style = s.style.bg(bg);
                    }
                }
                let mut all_spans = Vec::with_capacity(1 + spans.len());
                all_spans.push(gutter_span);
                all_spans.extend(spans);
                Line::from(all_spans)
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
    let base = "Tab:focus  s:staged  Space:review  R:hide-reviewed  up/down/jk:move  gg/G  hl:hscroll  Enter:fold  q:quit";
    let text = match &app.status_msg {
        Some(msg) => format!("{}   |   {}", base, msg),
        None => base.to_string(),
    };
    let para = Paragraph::new(text).style(Style::default().fg(theme::ACCENT_DIM));
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

    #[test]
    fn hscroll_offsets_diff_text() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, PathBuf::from("/repo"));
        app.set_diff(vec![
            DiffLine { kind: LineKind::Context, text: "ABCDEFGHIJ".into(), old_lineno: Some(1), new_lineno: Some(1) },
        ]);
        // no scroll: "ABCDEF..." visible
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump0: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump0.contains("ABCDEFGHIJ"));
        // scroll right 4: leading "ABCD" gone, "EFGHIJ" remains
        app.diff_hscroll = 4;
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump1: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump1.contains("EFGHIJ"));
        assert!(!dump1.contains("ABCDEFGHIJ"));
    }

    #[test]
    fn diff_shows_line_numbers() {
        use crate::app::LineKind;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, PathBuf::from("/repo"));
        app.set_diff(vec![
            DiffLine { kind: LineKind::Context, text: "ctx".into(), old_lineno: Some(7), new_lineno: Some(7) },
            DiffLine { kind: LineKind::Add, text: "added".into(), old_lineno: None, new_lineno: Some(8) },
            DiffLine { kind: LineKind::Del, text: "removed".into(), old_lineno: Some(5), new_lineno: None },
        ]);
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        // gutter line numbers must appear in the rendered buffer
        assert!(dump.contains('7'), "context line number 7 missing");
        assert!(dump.contains('8'), "add line number 8 missing");
        assert!(dump.contains('5'), "del line number 5 missing");
        // code text must still appear
        assert!(dump.contains("ctx"), "context text missing");
        assert!(dump.contains("added"), "add text missing");
        assert!(dump.contains("removed"), "del text missing");
    }

    #[test]
    fn tree_view_shows_dir_and_basenames() {
        // App with src/main.rs and top.rs → should show "src" (dir row), "main.rs" (basename), "top.rs" (basename)
        // After the tree-view change, the full path "src/main.rs" must NOT appear as a single token —
        // instead "src" is a dir row and "main.rs" is a file row with just the basename.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![
            FileChange { path: PathBuf::from("src/main.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("top.rs"), status: Status::Modified },
        ];
        let app = App::new(files, PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("src"), "directory name 'src' missing from tree view");
        assert!(dump.contains("main.rs"), "file basename 'main.rs' missing from tree view");
        assert!(dump.contains("top.rs"), "file basename 'top.rs' missing from tree view");
        // Full path "src/main.rs" should NOT appear as one token (tree renders basename only)
        assert!(!dump.contains("src/main.rs"), "full path 'src/main.rs' should not appear; tree view shows basenames");
    }
}
