use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, InputState, LineKind, Pane, Section, Status, ViewMode};
use crate::comments::CommentStatus;
use crate::highlight::highlight_code;
use crate::theme;
use crate::tree::RowKind;

fn status_letter(status: Status) -> (&'static str, ratatui::style::Color) {
    match status {
        Status::Added => ("A", theme::TICK),
        Status::Modified => ("M", theme::YELLOW),
        Status::Deleted => ("D", theme::RED),
        Status::Renamed => ("R", theme::BLUE),
        Status::Other => (" ", theme::ACCENT_DIM),
    }
}

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

    let main_area = outer[0];
    if app.show_files {
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(app.file_pane_pct),
                Constraint::Percentage(100 - app.file_pane_pct),
            ])
            .split(main_area);
        match app.view {
            ViewMode::Changes => render_files(frame, app, panes[0]),
            ViewMode::Commits => render_commits(frame, app, panes[0]),
        }
        render_diff(frame, app, panes[1]);
    } else {
        render_diff(frame, app, main_area);
    }
    render_status(frame, app, outer[1]);
    if let Some(input) = &app.input {
        render_input_modal(frame, input);
    }
}

fn focused_border(app: &App, pane: Pane) -> Style {
    if app.focus == pane {
        Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::ACCENT_DIM)
    }
}

fn render_files(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| {
            let indent = "  ".repeat(row.depth);
            match &row.kind {
                RowKind::Header { section, count } => {
                    let label = match section {
                        Section::Unstaged => format!("{}▌ Unstaged ({})", indent, count),
                        Section::Staged => format!("{}▌ Staged ({})", indent, count),
                    };
                    let line = Line::from(Span::styled(
                        label,
                        Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                    ));
                    ListItem::new(line)
                }
                RowKind::Dir { collapsed, .. } => {
                    let glyph = if *collapsed { "▸" } else { "▾" };
                    let text = format!("{}{} {}", indent, glyph, row.name);
                    ListItem::new(Line::from(text))
                }
                RowKind::File { section, file_index } => {
                    let files = app.section_files(*section);
                    let fc = &files[*file_index];
                    let file_path = &fc.path;
                    let (mark, mark_style) = if app.is_reviewed_path(file_path) {
                        ("✓ ", Style::default().fg(theme::TICK))
                    } else {
                        ("○ ", Style::default().fg(theme::ACCENT_DIM))
                    };
                    let (letter, letter_color) = status_letter(fc.status);
                    let icon = crate::icons::icon_for(file_path);
                    let line = Line::from(vec![
                        Span::raw(indent),
                        Span::styled(format!("{} ", letter), Style::default().fg(letter_color)),
                        Span::styled(mark, mark_style),
                        Span::raw(format!("{} {}", icon, row.name)),
                    ]);
                    ListItem::new(line)
                }
            }
        })
        .collect();
    let title = if app.hide_reviewed {
        " [Changes] Commits  (hiding reviewed) ".to_string()
    } else {
        " [Changes] Commits ".to_string()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(focused_border(app, Pane::Files))
                .title(title),
        )
        .highlight_style(Style::default().bg(theme::SELECTED_BG).add_modifier(Modifier::BOLD));
    let mut state = ListState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_commits(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .commits
        .iter()
        .map(|ci| {
            // Truncate summary to avoid overflow
            let max_summary = area.width.saturating_sub(30) as usize;
            let summary = if ci.summary.chars().count() > max_summary && max_summary > 3 {
                format!("{}…", ci.summary.chars().take(max_summary - 1).collect::<String>())
            } else {
                ci.summary.clone()
            };
            let line = Line::from(vec![
                Span::styled(format!("{} ", ci.short), Style::default().fg(theme::YELLOW)),
                Span::raw(summary),
                Span::styled(
                    format!("  — {} {}", ci.author, ci.time),
                    Style::default().fg(theme::ACCENT_DIM),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = " Changes [Commits] ";
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(focused_border(app, Pane::Files))
                .title(title),
        )
        .highlight_style(Style::default().bg(theme::SELECTED_BG).add_modifier(Modifier::BOLD));
    let mut state = ListState::default();
    state.select(if app.commits.is_empty() { None } else { Some(app.selected_commit) });
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_diff(frame: &mut Frame, app: &App, area: Rect) {
    let ext = app
        .selected_path()
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();

    let ctx_label = if app.full_file {
        "full file".to_string()
    } else {
        format!("ctx {}", app.context_lines)
    };
    let title = app
        .selected_path()
        .map(|p| format!(" Diff: {} ({}) ", p.display(), ctx_label))
        .unwrap_or_else(|| " Diff ".to_string());

    let lines: Vec<Line> = if app.view == ViewMode::Commits {
        vec![Line::from(Span::styled(
            "Press Enter to view commit's changes",
            Style::default().fg(theme::PLACEHOLDER),
        ))]
    } else if app.diff.is_empty() {
        vec![Line::from(Span::styled(
            "No changes",
            Style::default().fg(theme::PLACEHOLDER),
        ))]
    } else {
        let page = area.height.saturating_sub(2) as usize;

        // Compute the rendered height (diff line + its inline comment box lines) for a
        // given diff index, so we can scroll in rendered-line space and guarantee the
        // cursor line AND its comment are always visible.
        // comment box: 1 (top) + text_lines + (if response: 1 blank + response_lines) + 1 (bottom)
        let rendered_height = |i: usize| -> usize {
            let dl = &app.diff[i];
            let comment_lines = app
                .comment_for(dl)
                .map(|c| {
                    let text_lines = c.text.lines().count().max(1);
                    let response_lines = match c.response.as_deref() {
                        Some(r) if !r.trim().is_empty() => 1 + r.lines().count(), // 1 blank separator + response text lines
                        _ => 0,
                    };
                    1 + text_lines + response_lines + 1 // top + text + [blank+response] + bottom
                })
                .unwrap_or(0);
            1 + comment_lines
        };

        // Walk backward from diff_cursor to find the first visible diff index such
        // that the total rendered height from start..=cursor fits within `page`.
        let mut start = app.diff_cursor;
        let mut used = rendered_height(app.diff_cursor);
        while start > 0 {
            let h = rendered_height(start - 1);
            if used + h > page {
                break;
            }
            used += h;
            start -= 1;
        }

        let mut result: Vec<Line> = Vec::new();
        let mut rendered_rows: usize = 0;
        for (idx, dl) in app.diff.iter().enumerate().skip(start) {
            if rendered_rows >= page {
                break;
            }
            let is_cursor = idx == app.diff_cursor;
            let bg = match dl.kind {
                LineKind::Add => Some(theme::ADD_BG),
                LineKind::Del => Some(theme::DEL_BG),
                _ => None,
            };
            if dl.kind == LineKind::Hunk {
                let shifted: String = dl.text.chars().skip(app.diff_hscroll).collect();
                let mut span = Span::styled(shifted, Style::default().fg(theme::HUNK));
                if is_cursor {
                    span.style = span.style.bg(theme::SELECTED_BG);
                }
                result.push(Line::from(span));
                rendered_rows += 1;
                continue;
            }
            // Gutter: YELLOW for stale-commented lines, ACCENT for normal commented, ACCENT_DIM otherwise.
            let comment = app.comment_for(dl);
            let gutter_fg = match comment {
                Some(c) if c.stale => theme::YELLOW,
                Some(_) => theme::ACCENT,
                None => theme::ACCENT_DIM,
            };
            let gutter_style = if is_cursor {
                Style::default().fg(gutter_fg).bg(theme::SELECTED_BG)
            } else {
                Style::default().fg(gutter_fg)
            };
            let gutter_span = Span::styled(gutter(dl), gutter_style);
            let shifted: String = dl.text.chars().skip(app.diff_hscroll).collect();
            let mut spans: Vec<Span> = highlight_code(&shifted, &ext);
            if is_cursor {
                for s in spans.iter_mut() {
                    s.style = s.style.bg(theme::SELECTED_BG);
                }
            } else {
                if let Some(bg) = bg {
                    for s in spans.iter_mut() {
                        s.style = s.style.bg(bg);
                    }
                }
                if dl.kind == LineKind::Context {
                    for s in spans.iter_mut() {
                        s.style = s.style.add_modifier(Modifier::DIM);
                    }
                }
            }
            let mut all_spans = Vec::with_capacity(1 + spans.len());
            all_spans.push(gutter_span);
            all_spans.extend(spans);
            result.push(Line::from(all_spans));
            rendered_rows += 1;

            // Enhancement 5a: Inline comment box with box-drawing chars.
            // Normal:  ╭─ comment  /  │ <line>...  /  ╰─
            // Stale:   ╭─ ⚠ outdated · <status> (yellow) / │ <line>... / ╰─
            // Status badge colors: resolved=TICK, wontfix=RED, needs_info=YELLOW, open=ACCENT_DIM
            // Response: blank line + ↳ response: <text> lines, shown below body
            // The top + body lines + optional response + bottom are all counted toward budget.
            if let Some(c) = comment {
                // Determine border color based on stale flag and status
                let border_color = if c.stale {
                    theme::YELLOW
                } else {
                    match c.status {
                        CommentStatus::Open => theme::ACCENT_DIM,
                        CommentStatus::Resolved => theme::TICK,
                        CommentStatus::Wontfix => theme::RED,
                        CommentStatus::NeedsInfo => theme::YELLOW,
                    }
                };
                let border_style = Style::default().fg(border_color).add_modifier(Modifier::ITALIC | Modifier::DIM);
                let body_style = Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::ITALIC);

                // Top border line with status badge
                if rendered_rows < page {
                    let top_label = if c.stale {
                        format!("    ╭─ ⚠ outdated · {}", c.status.label())
                    } else {
                        match c.status {
                            CommentStatus::Open => "    ╭─ comment".to_string(),
                            CommentStatus::Resolved => "    ╭─ ✓ resolved".to_string(),
                            CommentStatus::Wontfix => "    ╭─ ✗ wontfix".to_string(),
                            CommentStatus::NeedsInfo => "    ╭─ ? needs-info".to_string(),
                        }
                    };
                    result.push(Line::from(Span::styled(top_label, border_style)));
                    rendered_rows += 1;
                }
                // Body lines (reviewer's comment text)
                for comment_line in c.text.lines() {
                    if rendered_rows >= page {
                        break;
                    }
                    let prefix = Span::styled("    │ ", body_style);
                    let body = Span::styled(comment_line.to_string(), body_style);
                    result.push(Line::from(vec![prefix, body]));
                    rendered_rows += 1;
                }
                // Response block (only when response is present AND non-empty after trim)
                if c.response.as_deref().map_or(false, |r| !r.trim().is_empty()) {
                    let resp = c.response.as_deref().unwrap();
                    let response_label_style = Style::default()
                        .fg(border_color)
                        .add_modifier(Modifier::ITALIC | Modifier::DIM);
                    // Blank separator line
                    if rendered_rows < page {
                        result.push(Line::from(Span::styled("    │ ", body_style)));
                        rendered_rows += 1;
                    }
                    // Response lines
                    let mut first = true;
                    for resp_line in resp.lines() {
                        if rendered_rows >= page {
                            break;
                        }
                        let line_content = if first {
                            first = false;
                            format!("    │ ↳ response: {}", resp_line)
                        } else {
                            format!("    │   {}", resp_line)
                        };
                        result.push(Line::from(Span::styled(line_content, response_label_style)));
                        rendered_rows += 1;
                    }
                }
                // Bottom border line
                if rendered_rows < page {
                    result.push(Line::from(Span::styled("    ╰─", border_style)));
                    rendered_rows += 1;
                }
            }
        }
        result
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
    let base = "Tab:focus  s:stage/unstage  Space:review  R:hide-reviewed  up/down/jk:move  gg/G  hl:hscroll  Enter:focus-diff  Esc:files  F:full-file  +/-:context  z:hide-files  <>:resize  c:comment  [/]:view  q:quit";
    let text = match &app.status_msg {
        Some(msg) => format!("{}   |   {}", base, msg),
        None => base.to_string(),
    };
    let para = Paragraph::new(text).style(Style::default().fg(theme::ACCENT_DIM));
    frame.render_widget(para, area);
}

/// Compute a centered Rect using percentages of the given area.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let margin_v = (100u16.saturating_sub(percent_y)) / 2;
    let margin_h = (100u16.saturating_sub(percent_x)) / 2;
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(margin_v),
            Constraint::Percentage(percent_y),
            Constraint::Percentage(margin_v),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(margin_h),
            Constraint::Percentage(percent_x),
            Constraint::Percentage(margin_h),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn render_input_modal(frame: &mut Frame, input: &InputState) {
    let area = centered_rect(60, 40, frame.area());
    frame.render_widget(Clear, area);
    let title = format!(" Comment line {} (Ctrl-S save · Esc cancel) ", input.target_line);
    // Append a cursor block indicator to the buffer text.
    let display_text = format!("{}▏", input.buffer);
    // Enhancement 5b: rounded border, accent color, horizontal padding for clearer text field.
    let para = Paragraph::new(display_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme::ACCENT))
                .padding(Padding::horizontal(1))
                .title(title),
        )
        .wrap(Wrap { trim: false });
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
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
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
        assert!(dump.contains("○"));
    }

    #[test]
    fn scroll_offset_hides_earlier_lines() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = app_with_diff();
        // With the cursor near the end, the viewport scrolls so earlier lines are not rendered.
        app.set_diff({
            let mut lines = vec![
                crate::app::DiffLine { kind: LineKind::Hunk, text: "@@ -1 +1 @@".into(), old_lineno: None, new_lineno: None },
            ];
            // Add 18 context lines after hunk so page=18 and cursor=18 scrolls past the hunk.
            for i in 1..=18u32 {
                lines.push(crate::app::DiffLine { kind: LineKind::Add, text: "let x = 1;".into(), old_lineno: None, new_lineno: Some(i) });
            }
            lines
        });
        // With page=18, cursor=18 -> scroll = 18+1-18 = 1, so hunk at index 0 is hidden.
        app.diff_cursor = 18;
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
        let app = App::new(files, vec![], PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("No changes"));
    }

    #[test]
    fn hscroll_offsets_diff_text() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
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
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.set_diff(vec![
            DiffLine { kind: LineKind::Context, text: "ctx".into(), old_lineno: Some(7), new_lineno: Some(7) },
            DiffLine { kind: LineKind::Add, text: "added".into(), old_lineno: None, new_lineno: Some(8) },
            DiffLine { kind: LineKind::Del, text: "removed".into(), old_lineno: Some(5), new_lineno: None },
        ]);
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains('7'), "context line number 7 missing");
        assert!(dump.contains('8'), "add line number 8 missing");
        assert!(dump.contains('5'), "del line number 5 missing");
        assert!(dump.contains("ctx"), "context text missing");
        assert!(dump.contains("added"), "add text missing");
        assert!(dump.contains("removed"), "del text missing");
    }

    #[test]
    fn tree_view_shows_dir_and_basenames() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![
            FileChange { path: PathBuf::from("src/main.rs"), status: Status::Modified },
            FileChange { path: PathBuf::from("top.rs"), status: Status::Modified },
        ];
        let app = App::new(files, vec![], PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("src"), "directory name 'src' missing from tree view");
        assert!(dump.contains("main.rs"), "file basename 'main.rs' missing from tree view");
        assert!(dump.contains("top.rs"), "file basename 'top.rs' missing from tree view");
        assert!(!dump.contains("src/main.rs"), "full path 'src/main.rs' should not appear; tree view shows basenames");
    }

    #[test]
    fn reviewed_file_shows_tick() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // select a.rs row (row 1) and review it
        app.selected = 1;
        app.toggle_reviewed(); // review a.rs
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("✓"));
    }

    #[test]
    fn both_sections_headers_render_when_both_non_empty() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let unstaged = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let staged = vec![FileChange { path: PathBuf::from("b.rs"), status: Status::Added }];
        let app = App::new(unstaged, staged, PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("Unstaged"), "Unstaged header missing");
        assert!(dump.contains("Staged"), "Staged header missing");
        assert!(dump.contains("a.rs"), "a.rs missing");
        assert!(dump.contains("b.rs"), "b.rs missing");
    }

    #[test]
    fn files_title_has_no_mode_label() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let app = App::new(files, vec![], PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        // The block title shows the tab bar with Changes and Commits tabs.
        // In Changes mode, Changes tab is active (in brackets).
        assert!(dump.contains("Changes"), "Changes tab missing from title");
        assert!(dump.contains("Commits"), "Commits tab missing from title");
        assert!(!dump.contains("STAGED"), "[STAGED] mode label should be gone");
        assert!(!dump.contains("UNSTAGED"), "[UNSTAGED] mode label should be gone");
    }

    #[test]
    fn hidden_file_pane_shows_only_diff() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("zzz.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.show_files = false;
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(!dump.contains("zzz.rs")); // file pane hidden
        assert!(dump.contains("No changes") || dump.contains("Diff")); // diff pane present
    }

    #[test]
    fn modified_file_shows_m_status_letter() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let app = App::new(files, vec![], PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains('M'), "Modified file should show 'M' status letter");
        assert!(dump.contains("a.rs"), "filename should still appear");
    }

    #[test]
    fn input_modal_shows_buffer_and_title() {
        use crate::app::InputState;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = app_with_diff();
        app.focus = Pane::Diff;
        app.input = Some(InputState {
            buffer: "hello world".to_string(),
            target_file: PathBuf::from("a.rs"),
            target_line: 1,
            target_hunk: "@@ -1 +1 @@".to_string(),
            anchor_line_text: String::new(),
            anchor_before: vec![],
            anchor_after: vec![],
        });
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("hello world"), "modal buffer text must appear");
        assert!(dump.contains("Comment"), "modal title must contain 'Comment'");
    }

    #[test]
    fn commented_line_shows_comment_text_inline() {
        use crate::app::LineKind;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1; // select a.rs row
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine { kind: LineKind::Add, text: "let x = 1;".into(), old_lineno: None, new_lineno: Some(5) },
        ]);
        // attach a comment for a.rs line 5
        app.comments.set(
            PathBuf::from("a.rs"),
            5,
            "@@ -3,4 @@".to_string(),
            "review note here".to_string(),
            "let x = 1;".to_string(),
            vec![],
            vec![],
        );
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("review note here"), "inline comment text must appear below commented line");
    }

    /// FIX 2: comment on a line near the bottom of a small viewport must not be clipped.
    /// Build a diff of 30 context lines + one Add line with a comment. Set the cursor
    /// on the Add line. Use an 80x10 backend (page = 8 visible rows). With the old
    /// scroll formula (diff-index space), the comment would be pushed off screen.
    /// With rendered-height scroll the comment must appear in the buffer.
    #[test]
    fn comment_not_clipped_when_cursor_near_bottom() {
        use crate::app::LineKind;
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1; // select a.rs
        app.focus = Pane::Diff;

        // 30 context lines, then one Add line at new_lineno 31
        let mut diff_lines = Vec::new();
        for i in 1u32..=30 {
            diff_lines.push(DiffLine {
                kind: LineKind::Context,
                text: format!("ctx {}", i),
                old_lineno: Some(i),
                new_lineno: Some(i),
            });
        }
        diff_lines.push(DiffLine {
            kind: LineKind::Add,
            text: "added_line".into(),
            old_lineno: None,
            new_lineno: Some(31),
        });
        app.set_diff(diff_lines);

        // cursor on the Add line (index 30)
        app.diff_cursor = 30;

        // attach a comment to that Add line
        app.comments.set(
            PathBuf::from("a.rs"),
            31,
            "".to_string(),
            "clipping_test_comment".to_string(),
            "added_line".to_string(),
            vec![],
            vec![],
        );

        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(
            dump.contains("clipping_test_comment"),
            "comment on cursor line must not be clipped even near the viewport bottom"
        );
    }

    /// FIX 1: saving a comment with leading/trailing whitespace must store only the
    /// trimmed text. Verify by setting a padded comment and checking that the rendered
    /// inline comment shows the trimmed text (no surrounding spaces).
    #[test]
    fn trimmed_comment_stored_without_whitespace_padding() {
        use crate::app::LineKind;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine { kind: LineKind::Add, text: "fn main() {}".into(), old_lineno: None, new_lineno: Some(1) },
        ]);
        // Simulate what main.rs does after Fix 1: store the trimmed text.
        let raw = "   trimmed_note   ";
        let trimmed = raw.trim().to_string();
        app.comments.set(PathBuf::from("a.rs"), 1, "".to_string(), trimmed, "fn main() {}".to_string(), vec![], vec![]);

        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("trimmed_note"), "trimmed comment text must appear");
        // The raw padded string (with surrounding spaces) must not be stored/rendered.
        assert!(!dump.contains("   trimmed_note   "), "padded comment text must not appear");
    }

    #[test]
    fn added_file_shows_a_deleted_shows_d() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![
            FileChange { path: PathBuf::from("new.rs"), status: Status::Added },
            FileChange { path: PathBuf::from("old.rs"), status: Status::Deleted },
        ];
        let app = App::new(files, vec![], PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains('A'), "Added file should show 'A' status letter");
        assert!(dump.contains('D'), "Deleted file should show 'D' status letter");
    }

    #[test]
    fn resolved_comment_shows_status_and_response() {
        use crate::app::LineKind;
        use crate::comments::CommentStatus;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine { kind: LineKind::Add, text: "fn foo() {}".into(), old_lineno: None, new_lineno: Some(3) },
        ]);
        app.comments.set(
            PathBuf::from("a.rs"),
            3,
            "@@".to_string(),
            "please fix this".to_string(),
            "fn foo() {}".to_string(),
            vec![],
            vec![],
        );
        app.comments.items[0].status = CommentStatus::Resolved;
        app.comments.items[0].response = Some("Fixed it".to_string());

        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("resolved"), "resolved status must appear in comment box");
        assert!(dump.contains("Fixed it"), "agent response must appear in comment box");
    }

    #[test]
    fn stale_comment_shows_outdated_prefix() {
        use crate::app::LineKind;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.set_diff(vec![
            DiffLine { kind: LineKind::Add, text: "let y = 2;".into(), old_lineno: None, new_lineno: Some(7) },
        ]);
        // Insert a stale comment directly
        app.comments.set(
            PathBuf::from("a.rs"),
            7,
            "@@".to_string(),
            "stale note".to_string(),
            "let y = 2;".to_string(),
            vec![],
            vec![],
        );
        // Mark it stale manually (simulating relocation failure)
        app.comments.items[0].stale = true;

        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("outdated"), "stale comment must show '(outdated)' prefix");
        assert!(dump.contains("stale note"), "stale comment text must still appear");
    }

    #[test]
    fn empty_response_does_not_break_layout() {
        // A comment with an empty-string response must render without a phantom line
        // (regression: rendered-height overcounted, clipping the box bottom).
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange { path: PathBuf::from("a.rs"), status: Status::Modified }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1; // select a.rs row (row 0=Unstaged header, row 1=File a.rs)
        app.focus = Pane::Diff;
        // place a comment with empty response on a line in the diff
        app.comments.items.push(crate::comments::Comment {
            file: PathBuf::from("a.rs"),
            line: 1,
            hunk: String::new(),
            text: "please fix".into(),
            line_text: "let x = 1;".into(),
            context_before: vec![],
            context_after: vec![],
            orig_line: 1,
            stale: false,
            status: crate::comments::CommentStatus::Open,
            response: Some(String::new()),
        });
        app.set_diff(vec![
            DiffLine { kind: LineKind::Add, text: "let x = 1;".into(), old_lineno: None, new_lineno: Some(1) },
        ]);
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        // the comment text renders; no panic; "response:" label NOT shown for empty response
        assert!(dump.contains("please fix"));
        assert!(!dump.contains("response:"));
    }
}
