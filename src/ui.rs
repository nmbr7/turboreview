use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap,
};
use ratatui::Frame;

use crate::app::{App, CommentRow, InputState, LineKind, Pane, Section, Status, ViewMode};
use crate::comments::CommentStatus;
use crate::highlight::highlight_code;
use crate::theme::Palette;
use crate::tree::RowKind;

fn status_letter(status: Status, pal: &Palette) -> (&'static str, ratatui::style::Color) {
    match status {
        Status::Added => ("A", pal.tick),
        Status::Modified => ("M", pal.yellow),
        Status::Deleted => ("D", pal.red),
        Status::Renamed => ("R", pal.blue),
        Status::Other => (" ", pal.accent_dim),
    }
}

fn gutter(dl: &crate::app::DiffLine) -> String {
    let n = dl.new_lineno.or(dl.old_lineno);
    match n {
        Some(n) => format!("{:>4} ", n),
        None => "     ".to_string(),
    }
}

/// Word-wrap `text` to `width` columns. Each `\n`-delimited source line is wrapped
/// independently; words longer than `width` are hard-split. An empty source line
/// yields one empty visual line, so blank lines are preserved. `width` is clamped
/// to at least 1. Returns the visual lines (no trailing newline).
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for src_line in text.split('\n') {
        if src_line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        let mut cur_len = 0usize; // in chars
        for word in src_line.split(' ') {
            let wlen = word.chars().count();
            // Hard-split a word longer than the full width.
            if wlen > width {
                if cur_len > 0 {
                    out.push(std::mem::take(&mut cur));
                    cur_len = 0;
                }
                let mut chunk = String::new();
                for ch in word.chars() {
                    if chunk.chars().count() == width {
                        out.push(std::mem::take(&mut chunk));
                    }
                    chunk.push(ch);
                }
                if !chunk.is_empty() {
                    cur = chunk;
                    cur_len = cur.chars().count();
                }
                continue;
            }
            // +1 for the joining space when cur is non-empty.
            let needed = if cur_len == 0 {
                wlen
            } else {
                cur_len + 1 + wlen
            };
            if needed > width {
                out.push(std::mem::take(&mut cur));
                cur = word.to_string();
                cur_len = wlen;
            } else {
                if cur_len > 0 {
                    cur.push(' ');
                    cur_len += 1;
                }
                cur.push_str(word);
                cur_len += wlen;
            }
        }
        out.push(cur);
    }
    out
}

/// One visual row in side-by-side mode. A `header` (hunk) row spans the full
/// width and ignores left/right. Otherwise `left`/`right` hold the diff indices
/// shown on the old/new sides (`None` = a blank cell).
#[derive(Debug, Clone, PartialEq, Eq)]
struct RowPair {
    header: Option<usize>,
    left: Option<usize>,
    right: Option<usize>,
}

/// Pair a unified `diff` into side-by-side rows. Context lines map to one row
/// with the same index on both sides. A run of Del lines immediately followed by
/// a run of Add lines is zipped position-wise (the shorter side padded blank).
/// Hunk lines become full-width header rows.
fn pair_diff_rows(diff: &[crate::app::DiffLine]) -> Vec<RowPair> {
    let mut rows = Vec::new();
    let mut i = 0;
    while i < diff.len() {
        match diff[i].kind {
            LineKind::Hunk => {
                rows.push(RowPair {
                    header: Some(i),
                    left: None,
                    right: None,
                });
                i += 1;
            }
            LineKind::Context => {
                rows.push(RowPair {
                    header: None,
                    left: Some(i),
                    right: Some(i),
                });
                i += 1;
            }
            LineKind::Del | LineKind::Add => {
                // Gather the maximal run of Dels, then the maximal run of Adds.
                let dels_start = i;
                while i < diff.len() && diff[i].kind == LineKind::Del {
                    i += 1;
                }
                let dels: Vec<usize> = (dels_start..i).collect();
                let adds_start = i;
                while i < diff.len() && diff[i].kind == LineKind::Add {
                    i += 1;
                }
                let adds: Vec<usize> = (adds_start..i).collect();
                let n = dels.len().max(adds.len());
                for k in 0..n {
                    rows.push(RowPair {
                        header: None,
                        left: dels.get(k).copied(),
                        right: adds.get(k).copied(),
                    });
                }
            }
        }
    }
    rows
}

pub fn render(frame: &mut Frame, app: &App) {
    // The bottom status row only exists when it has something to show — a search
    // input line or a transient message. Otherwise the panes use the full height
    // (no empty padding line), since the "? help" hint lives in the diff border.
    let want_status =
        app.search_input.is_some() || app.status_msg.is_some() || app.show_coverage;
    let status_h = if want_status { 1 } else { 0 };
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(status_h)])
        .split(frame.area());

    let main_area = outer[0];
    // The right pane (tabbed Comments / Debug) shows when comments are enabled
    // or a debug session is active.
    let show_right = app.right_pane_visible();
    let right_pct: u16 = app.right_pane_pct;

    let render_files_or_commits = |frame: &mut Frame, app: &App, area: Rect| match app.view {
        ViewMode::Changes => render_files(frame, app, area),
        ViewMode::Commits if app.open_commit.is_none() => render_commits(frame, app, area),
        ViewMode::Commits => render_files(frame, app, area),
    };

    if app.show_files && show_right {
        let diff_pct = 100u16
            .saturating_sub(app.file_pane_pct)
            .saturating_sub(right_pct)
            .max(20);
        let files_pct = 100u16.saturating_sub(diff_pct).saturating_sub(right_pct);
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(files_pct),
                Constraint::Percentage(diff_pct),
                Constraint::Percentage(right_pct),
            ])
            .split(main_area);
        render_files_or_commits(frame, app, panes[0]);
        render_diff(frame, app, panes[1]);
        render_right_pane(frame, app, panes[2]);
    } else if show_right {
        let diff_pct = 100u16.saturating_sub(right_pct).max(20);
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(diff_pct),
                Constraint::Percentage(right_pct),
            ])
            .split(main_area);
        render_diff(frame, app, panes[0]);
        render_right_pane(frame, app, panes[1]);
    } else if app.show_files {
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(app.file_pane_pct),
                Constraint::Percentage(100 - app.file_pane_pct),
            ])
            .split(main_area);
        render_files_or_commits(frame, app, panes[0]);
        render_diff(frame, app, panes[1]);
    } else {
        render_diff(frame, app, main_area);
    }
    if status_h > 0 {
        render_status(frame, app, outer[1]);
    }
    if let Some(input) = &app.input {
        render_input_modal(frame, app, input);
    }
    if app.show_help {
        render_help_modal(frame, app);
    }
    if app.launch_picker_active() {
        render_launch_picker(frame, app);
    }
}

fn focused_border(app: &App, pane: Pane) -> Style {
    let pal = app.palette();
    if app.focus == pane {
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.accent_dim)
    }
}

fn status_color(status: CommentStatus, pal: &Palette) -> ratatui::style::Color {
    match status {
        CommentStatus::Open => pal.accent,
        CommentStatus::NeedsInfo => pal.yellow,
        CommentStatus::Wontfix => pal.red,
        CommentStatus::Resolved => pal.tick,
    }
}

fn render_comment_list(frame: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    let count = app.comments.items.len();
    let title = format!(" Comments ({}) ", count);

    let rows = app.comment_rows();
    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| match row {
            CommentRow::Header(status, cnt) => {
                let label = format!("▌ {} ({})", status.label(), cnt);
                let line = Line::from(Span::styled(
                    label,
                    Style::default()
                        .fg(status_color(*status, &pal))
                        .add_modifier(Modifier::BOLD),
                ));
                ListItem::new(line)
            }
            CommentRow::Item(i) => {
                let c = &app.comments.items[*i];
                let basename = c.file.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let first_line = c.text.lines().next().unwrap_or("");
                let max_text = area.width.saturating_sub(20) as usize;
                let text_display = if first_line.chars().count() > max_text && max_text > 3 {
                    format!(
                        "{}…",
                        first_line
                            .chars()
                            .take(max_text.saturating_sub(1))
                            .collect::<String>()
                    )
                } else {
                    first_line.to_string()
                };
                let line = Line::from(vec![
                    Span::styled(
                        format!("  {}:{} ", basename, c.line),
                        Style::default().fg(pal.accent_dim),
                    ),
                    Span::raw(text_display),
                ]);
                ListItem::new(line)
            }
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(focused_border(app, Pane::Comments))
                .title(title),
        )
        .highlight_style(
            Style::default()
                .bg(pal.selected_bg)
                .add_modifier(Modifier::BOLD),
        );

    let mut state = ListState::default();
    if !rows.is_empty() {
        state.select(Some(app.comment_selected));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

/// The tabbed right pane: a tab header (Comments | Debug) plus the active tab's
/// content. The Debug tab only appears while a session is active.
fn render_right_pane(frame: &mut Frame, app: &App, area: Rect) {
    use crate::app::RightTab;
    let pal = app.palette();
    // 1-row tab header, then the content area.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    let focused = app.focus == Pane::Comments;
    let tab = |label: &str, active: bool| {
        let st = if active {
            Style::default()
                .fg(if focused { pal.accent } else { pal.accent_dim })
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(pal.accent_dim)
        };
        Span::styled(format!(" {label} "), st)
    };
    let mut header = vec![
        tab("Comments", app.right_tab == RightTab::Comments),
        Span::raw(" "),
    ];
    if app.debug_active() {
        header.push(tab("Debug", app.right_tab == RightTab::Debug));
    }
    header.push(Span::styled("  [ ]", Style::default().fg(pal.accent_dim)));
    frame.render_widget(Paragraph::new(Line::from(header)), rows[0]);

    match app.right_tab {
        RightTab::Comments => render_comment_list(frame, app, rows[1]),
        RightTab::Debug => render_debug_panel(frame, app, rows[1]),
    }
}

/// Render the breakpoint list into `lines`: one row per breakpoint, with an
/// enabled marker (● red / ○ dim), file:line, and the selected row highlighted.
fn render_breakpoint_lines(
    lines: &mut Vec<Line<'static>>,
    app: &crate::app::App,
    d: &crate::app::DebugState,
    pal: &crate::theme::Palette,
) {
    let bps = d.breakpoint_list();
    if bps.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no breakpoints — press b on a line",
            Style::default().fg(pal.accent_dim),
        )));
        return;
    }
    lines.push(Line::from(Span::styled(
        " Enter: go · Space: on/off · d: delete",
        Style::default().fg(pal.accent_dim),
    )));
    for (i, (file, line, on)) in bps.iter().enumerate() {
        let base = file
            .strip_prefix(&app.repo_root)
            .unwrap_or(file.as_path())
            .display()
            .to_string();
        let (mark, mark_fg) = if *on { ("●", pal.red) } else { ("○", pal.accent_dim) };
        let mut row_style = Style::default();
        let mut text_fg = if *on { pal.accent } else { pal.accent_dim };
        if i == d.bp_sel {
            row_style = row_style.bg(pal.selected_bg);
            text_fg = if *on { pal.accent } else { pal.accent_dim };
        }
        lines.push(Line::from(vec![
            Span::styled(format!(" {mark} "), row_style.fg(mark_fg)),
            Span::styled(format!("{base}:{line}"), row_style.fg(text_fg)),
        ]));
    }
}

/// The debugger right-hand panel: a session strip, the active session's call
/// stack, and the selected frame's local variables.
fn render_debug_panel(frame: &mut Frame, app: &App, area: Rect) {
    use crate::app::{DebugTab, SessionState};
    let pal = app.palette();
    let Some(d) = app.debug.as_ref() else { return };
    let hscroll = d.hscroll as u16;

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Tab header: [ Vars | Breakpoints ], active tab highlighted.
    let tab_span = |label: &str, active: bool| {
        let st = if active {
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(pal.accent_dim)
        };
        Span::styled(format!(" {label} "), st)
    };
    lines.push(Line::from(vec![
        tab_span("Vars", d.tab == DebugTab::Vars),
        Span::raw(" "),
        tab_span("Breakpoints", d.tab == DebugTab::Breakpoints),
        Span::styled("   (t: switch)", Style::default().fg(pal.accent_dim)),
    ]));
    lines.push(Line::from(""));

    if d.tab == DebugTab::Breakpoints {
        render_breakpoint_lines(&mut lines, app, d, &pal);
        let para = Paragraph::new(lines)
            .scroll((0, hscroll))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(focused_border(app, Pane::Comments))
                    .title(" Debug "),
            );
        frame.render_widget(para, area);
        return;
    }

    // Session strip: one row per session, active one bracketed.
    for (i, s) in d.sessions.iter().enumerate() {
        let state = match s.state {
            SessionState::Starting => "…",
            SessionState::Running => "▶ running",
            SessionState::Stopped => "⏸ stopped",
            SessionState::Exited => "✗ exited",
        };
        let marker = if i == d.active { "▌ " } else { "  " };
        let style = if i == d.active {
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(pal.accent_dim)
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{}  {state}", s.label),
            style,
        )));
    }
    lines.push(Line::from(""));

    // Active session: flattened call stack + locals (recursing into expanded
    // structured values). panel_sel selects a row; Enter expands a var.
    if let Some(sess) = d.active_session() {
        lines.push(Line::from(Span::styled(
            " Call stack + locals  (Enter: expand · h/l: scroll)",
            Style::default().fg(pal.hunk).add_modifier(Modifier::BOLD),
        )));
        if sess.stack.is_empty() {
            lines.push(Line::from(Span::styled(
                "   (not stopped)",
                Style::default().fg(pal.accent_dim),
            )));
        }
        for (ri, row) in d.debug_rows().iter().enumerate() {
            let selected = ri == d.panel_sel;
            let sel_bg = |st: Style| if selected { st.bg(pal.selected_bg) } else { st };
            match row {
                crate::app::DebugRow::Frame(fi) => {
                    let f = &sess.stack[*fi];
                    let loc = f
                        .file
                        .as_deref()
                        .map(|p| {
                            let base = std::path::Path::new(p)
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or(p);
                            format!("  {base}:{}", f.line)
                        })
                        .unwrap_or_default();
                    // Frame header: name (yellow, bold) clearly separate from
                    // the variable subtree below; location in cyan.
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {}", f.name),
                            sel_bg(Style::default().fg(pal.yellow).add_modifier(Modifier::BOLD)),
                        ),
                        Span::styled(loc, sel_bg(Style::default().fg(pal.hunk))),
                    ]));
                }
                crate::app::DebugRow::Var(fi, path) => {
                    let v = crate::app::var_at_path(&sess.stack[*fi].locals, path);
                    if let Some(v) = v {
                        let depth = path.len();
                        let indent = "  ".repeat(depth + 1);
                        let marker = if v.var_ref > 0 {
                            if v.expanded { "▾ " } else { "▸ " }
                        } else {
                            "  "
                        };
                        // Distinct colors: name (accent), value (green/tick),
                        // type (blue), address (dim).
                        let mut spans = vec![
                            Span::styled(
                                format!("{indent}{marker}{}", v.name),
                                sel_bg(Style::default().fg(pal.accent)),
                            ),
                            Span::styled(" = ", sel_bg(Style::default().fg(pal.accent_dim))),
                            Span::styled(v.value.clone(), sel_bg(Style::default().fg(pal.tick))),
                        ];
                        if let Some(t) = v.ty.as_deref() {
                            spans.push(Span::styled(
                                format!("  : {t}"),
                                sel_bg(Style::default().fg(pal.blue)),
                            ));
                        }
                        if let Some(addr) = v.memory_ref.as_deref() {
                            spans.push(Span::styled(
                                format!("  @{addr}"),
                                sel_bg(Style::default().fg(pal.accent_dim)),
                            ));
                        }
                        lines.push(Line::from(spans));
                    }
                }
            }
        }
    }

    let para = Paragraph::new(lines)
        .scroll((0, hscroll))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(focused_border(app, Pane::Comments))
                .title(" Debug "),
        );
    frame.render_widget(para, area);
}

fn render_files(frame: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
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
                        Section::Commit => {
                            let short = app.open_commit_short().unwrap_or("commit");
                            format!("{}▌ Commit {} ({})", indent, short, count)
                        }
                        Section::All => format!("{}▌ All files ({})", indent, count),
                    };
                    let line = Line::from(Span::styled(
                        label,
                        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                    ));
                    ListItem::new(line)
                }
                RowKind::Dir { collapsed, .. } => {
                    let glyph = if *collapsed { "▸" } else { "▾" };
                    let text = format!("{}{} {}", indent, glyph, row.name);
                    ListItem::new(Line::from(text))
                }
                RowKind::File {
                    section,
                    file_index,
                } => {
                    let files = app.section_files(*section);
                    let fc = &files[*file_index];
                    let file_path = &fc.path;
                    let (mark, mark_style) = if app.is_reviewed_path(file_path) {
                        ("✓ ", Style::default().fg(pal.tick))
                    } else {
                        ("○ ", Style::default().fg(pal.accent_dim))
                    };
                    let (letter, letter_color) = status_letter(fc.status, &pal);
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
    let title = if app.in_commit_detail() {
        let short = app.open_commit_short().unwrap_or("commit");
        format!(" Changes  Commits ▸ {} ", short)
    } else if app.hide_reviewed {
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
        .highlight_style(
            Style::default()
                .bg(pal.selected_bg)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_commits(frame: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    let mut items: Vec<ListItem> = app
        .commits
        .iter()
        .map(|ci| {
            // Reserve room for hash + author/date + diff stats so the summary
            // truncates instead of overflowing.
            let max_summary = area.width.saturating_sub(48) as usize;
            let summary = if ci.summary.chars().count() > max_summary && max_summary > 3 {
                format!(
                    "{}…",
                    ci.summary.chars().take(max_summary - 1).collect::<String>()
                )
            } else {
                ci.summary.clone()
            };
            let mut spans = vec![
                Span::styled(format!("{} ", ci.short), Style::default().fg(pal.yellow)),
                Span::raw(summary),
                Span::styled(
                    format!("  — {} {}", ci.author, ci.time),
                    Style::default().fg(pal.accent_dim),
                ),
            ];
            // Diff stats: "· N files +ins -del" (green/red), or a placeholder
            // while the stat is still being computed for this row.
            match app.commit_stats.get(&ci.id) {
                Some(s) => {
                    spans.push(Span::styled(
                        format!(" · {} files ", s.files),
                        Style::default().fg(pal.accent_dim),
                    ));
                    spans.push(Span::styled(
                        format!("+{}", s.insertions),
                        Style::default().fg(pal.tick),
                    ));
                    spans.push(Span::styled(
                        format!(" -{}", s.deletions),
                        Style::default().fg(pal.red),
                    ));
                }
                None => spans.push(Span::styled(
                    " · …",
                    Style::default().fg(pal.accent_dim),
                )),
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    // Footer hint when the page may have been truncated (more history available).
    if !app.commits.is_empty() && app.commits.len() == app.commit_limit {
        items.push(ListItem::new(Line::from(Span::styled(
            "  … press L to load more",
            Style::default()
                .fg(pal.accent_dim)
                .add_modifier(Modifier::ITALIC),
        ))));
    }

    let title = " Changes [Commits] ";
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(focused_border(app, Pane::Files))
                .title(title),
        )
        .highlight_style(
            Style::default()
                .bg(pal.selected_bg)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default();
    state.select(if app.commits.is_empty() {
        None
    } else {
        Some(app.selected_commit)
    });
    frame.render_stateful_widget(list, area, &mut state);
}

/// Keep the cursor visible at the bottom of the viewport (default diff scrolling).
fn diff_scroll_start_follow(cursor: usize, page: usize, rh: &impl Fn(usize) -> usize) -> usize {
    if page == 0 {
        return cursor;
    }
    let mut start = cursor;
    let mut used = rh(cursor);
    while start > 0 {
        let h = rh(start - 1);
        if used + h > page {
            break;
        }
        used += h;
        start -= 1;
    }
    start
}

/// Place the cursor near the vertical center, or near the top when the cursor is early.
fn diff_scroll_start_center(cursor: usize, page: usize, rh: &impl Fn(usize) -> usize) -> usize {
    if page == 0 {
        return cursor;
    }
    let cursor_h = rh(cursor);
    let above_target = page.saturating_sub(cursor_h) / 2;
    let mut start = cursor;
    let mut above_used = 0;
    while start > 0 {
        let h = rh(start - 1);
        if above_used + h > above_target {
            break;
        }
        above_used += h;
        start -= 1;
    }
    start
}

/// Number of rendered lines an inline comment box occupies for `wrap_w` columns:
/// 1 (top) + wrapped text + (response ? 1 blank + wrapped response : 0) + 1 (bottom).
/// Recursively render captured snapshot variables into the comment box, one
/// line per var (name = value : type), descending into children. `indent` is
/// the nesting depth in spaces (added after the box prefix).
#[allow(clippy::too_many_arguments)]
fn push_snapshot_vars(
    result: &mut Vec<Line<'static>>,
    rendered_rows: &mut usize,
    page: usize,
    vars: &[crate::dap::VarRow],
    indent: usize,
    border_style: Style,
    body_style: Style,
    pal: &Palette,
) {
    for v in vars {
        if *rendered_rows >= page {
            return;
        }
        let pad = " ".repeat(indent);
        let ty = v
            .ty
            .as_deref()
            .map(|t| format!("  : {t}"))
            .unwrap_or_default();
        result.push(Line::from(vec![
            Span::styled("    │ ", border_style),
            Span::styled(format!("{pad}{} = ", v.name), body_style),
            Span::styled(v.value.clone(), Style::default().fg(pal.tick).add_modifier(Modifier::ITALIC)),
            Span::styled(ty, Style::default().fg(pal.blue).add_modifier(Modifier::ITALIC)),
        ]));
        *rendered_rows += 1;
        if !v.children.is_empty() {
            push_snapshot_vars(result, rendered_rows, page, &v.children, indent + 2, border_style, body_style, pal);
        }
    }
}

/// Count the lines `push_snapshot_vars` would emit for `vars` (recursive).
fn count_snapshot_vars(vars: &[crate::dap::VarRow]) -> usize {
    vars.iter()
        .map(|v| 1 + count_snapshot_vars(&v.children))
        .sum()
}

fn comment_box_height(c: &crate::comments::Comment, wrap_w: usize) -> usize {
    let text_lines = wrap_text(&c.text, wrap_w).len().max(1);
    let response_lines = match c.response.as_deref() {
        Some(r) if !r.trim().is_empty() => 1 + wrap_text(r, response_wrap_w(wrap_w)).len(),
        _ => 0,
    };
    // Debug snapshot: 1 header line, then for each frame: 1 frame line + its
    // (recursive) variable lines.
    let snapshot_lines = match &c.debug_snapshot {
        Some(s) => {
            let frame_lines: usize = s
                .stack
                .iter()
                .map(|f| 1 + count_snapshot_vars(&f.locals))
                .sum();
            1 + frame_lines
        }
        None => 0,
    };
    1 + text_lines + response_lines + snapshot_lines + 1
}

/// Wrap width for the response block. The response prefix `"    │ ↳ response: "`
/// is 18 columns vs the body prefix `"    │ "` (6 columns), so response text has
/// 12 fewer columns than the body. Continuation lines align to the same start.
/// `wrap_w` is the body wrap width (= inner_w - 6).
fn response_wrap_w(wrap_w: usize) -> usize {
    wrap_w.saturating_sub(RESPONSE_PREFIX_W - BODY_PREFIX_W).max(1)
}

/// Visible width of the body line prefix `"    │ "`.
const BODY_PREFIX_W: usize = 6;
/// Right-side breathing room: wrap comment text this many columns short of the
/// pane edge so text doesn't butt against the border.
const RIGHT_PAD: usize = 2;
/// Visible width of the response first-line prefix `"    │ ↳ response: "`.
const RESPONSE_PREFIX_W: usize = 18;

/// Push an inline comment box (top border, wrapped text, optional response,
/// bottom border) into `result`, honoring the remaining `page` budget. Shared by
/// the unified and side-by-side diff renderers.
fn push_comment_box(
    result: &mut Vec<Line<'static>>,
    rendered_rows: &mut usize,
    page: usize,
    c: &crate::comments::Comment,
    wrap_w: usize,
    pal: &Palette,
) {
    let border_color = if c.stale {
        pal.yellow
    } else {
        match c.status {
            // Open: a subtle dim frame so the header/body colors stand out.
            CommentStatus::Open => pal.accent_dim,
            CommentStatus::Resolved => pal.tick,
            CommentStatus::Wontfix => pal.red,
            CommentStatus::NeedsInfo => pal.yellow,
        }
    };
    let border_style = Style::default()
        .fg(border_color)
        .add_modifier(Modifier::ITALIC | Modifier::DIM);
    let body_style = Style::default()
        .fg(pal.accent)
        .add_modifier(Modifier::ITALIC);

    // Top border line with status badge
    if *rendered_rows < page {
        let mut top_label = if c.stale {
            format!("    ╭─ ⚠ outdated · {}", c.status.label())
        } else {
            match c.status {
                CommentStatus::Open => "    ╭─ comment".to_string(),
                CommentStatus::Resolved => "    ╭─ ✓ resolved".to_string(),
                CommentStatus::Wontfix => "    ╭─ ✗ wontfix".to_string(),
                CommentStatus::NeedsInfo => "    ╭─ ? needs-info".to_string(),
            }
        };
        // Last-edit timestamp (UTC) + relative age. 0 = legacy comment, no stamp.
        if c.updated > 0 {
            top_label.push_str(&format!(
                " · {} ({})",
                crate::git::format_datetime(c.updated),
                crate::git::relative_time(c.updated, crate::storage::now_secs())
            ));
        }
        result.push(Line::from(Span::styled(top_label, border_style)));
        *rendered_rows += 1;
    }
    // Body lines (reviewer's comment text), wrapped to the box width.
    for comment_line in wrap_text(&c.text, wrap_w) {
        if *rendered_rows >= page {
            break;
        }
        let prefix = Span::styled("    │ ", border_style);
        let body = Span::styled(comment_line, body_style);
        result.push(Line::from(vec![prefix, body]));
        *rendered_rows += 1;
    }
    // Response block (only when response is present AND non-empty after trim)
    if c.response
        .as_deref()
        .map_or(false, |r| !r.trim().is_empty())
    {
        let resp = c.response.as_deref().unwrap();
        if *rendered_rows < page {
            result.push(Line::from(Span::styled("    │ ", border_style)));
            *rendered_rows += 1;
        }
        let mut first = true;
        for resp_line in wrap_text(resp, response_wrap_w(wrap_w)) {
            if *rendered_rows >= page {
                break;
            }
            // "    │ " is the border; the rest (label + text) uses body color.
            let border = Span::styled("    │ ", border_style);
            let text = if first {
                first = false;
                // "↳ response: " label + text, aligned to 18 cols total prefix.
                Span::styled(format!("↳ response: {}", resp_line), body_style)
            } else {
                // Align continuation under the first response char.
                Span::styled(format!("            {}", resp_line), body_style)
            };
            result.push(Line::from(vec![border, text]));
            *rendered_rows += 1;
        }
    }
    // Debug snapshot: a captured call stack attached at a breakpoint.
    if let Some(snap) = &c.debug_snapshot {
        let label_style = Style::default()
            .fg(pal.tick)
            .add_modifier(Modifier::ITALIC);
        if *rendered_rows < page {
            result.push(Line::from(vec![
                Span::styled("    │ ", border_style),
                Span::styled(
                    format!("↳ stack @ {} ({}:{})", snap.session_label, {
                        std::path::Path::new(&snap.stopped_file)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&snap.stopped_file)
                    }, snap.stopped_line),
                    label_style,
                ),
            ]));
            *rendered_rows += 1;
        }
        for f in &snap.stack {
            if *rendered_rows >= page {
                break;
            }
            let loc = f
                .file
                .as_deref()
                .map(|p| {
                    let base = std::path::Path::new(p)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(p);
                    format!("  {base}:{}", f.line)
                })
                .unwrap_or_default();
            // Frame line: name (yellow, italic) + location (cyan), distinct
            // from the variable subtree below.
            result.push(Line::from(vec![
                Span::styled("    │ ", border_style),
                Span::styled(
                    format!("  {}", f.name),
                    Style::default().fg(pal.yellow).add_modifier(Modifier::ITALIC | Modifier::BOLD),
                ),
                Span::styled(loc, Style::default().fg(pal.hunk).add_modifier(Modifier::ITALIC)),
            ]));
            *rendered_rows += 1;
            // This frame's locals, recursing into captured children.
            push_snapshot_vars(result, rendered_rows, page, &f.locals, 2, border_style, body_style, pal);
        }
    }
    // Bottom border line
    if *rendered_rows < page {
        result.push(Line::from(Span::styled("    ╰─", border_style)));
        *rendered_rows += 1;
    }
}

/// Build the rendered lines for side-by-side (split) diff mode. Old lines sit on
/// the left half, new lines on the right, separated by a vertical bar. Cursor row
/// (whichever side holds `diff_cursor`) is highlighted full-row; search matches
/// tint their cell; hunk headers span the full width; inline comment boxes render
/// full width under their row.
fn build_split_lines(app: &App, area: Rect, ext: &str) -> Vec<Line<'static>> {
    let pal = app.palette();
    let page = area.height.saturating_sub(2) as usize;
    let inner_w = area.width.saturating_sub(2) as usize; // minus borders
    // Two columns + a 1-char separator between them.
    let sep_w = 1usize;
    let cell_w = inner_w.saturating_sub(sep_w) / 2;
    let gutter_w = 5usize; // matches `gutter()` width
    let text_w = cell_w.saturating_sub(gutter_w).max(1);
    let wrap_w = inner_w.saturating_sub(BODY_PREFIX_W + RIGHT_PAD).max(1); // indent "    │ " + right pad

    // Breakpoint / stopped-line markers for the file shown in the diff.
    let bp_file = app.selected_path().map(|p| app.repo_root.join(p));
    let stopped_line = app
        .debug
        .as_ref()
        .and_then(|d| d.active_session())
        .and_then(|s| s.stopped_at.as_ref())
        .filter(|(f, _)| bp_file.as_deref() == Some(f.as_path()))
        .map(|(_, l)| *l);

    let pairs = pair_diff_rows(&app.diff);
    // Which paired row holds the cursor — its header (hunk), left, or right cell.
    let cur = Some(app.diff_cursor);
    let cursor_row = pairs
        .iter()
        .position(|p| p.header == cur || p.left == cur || p.right == cur)
        .unwrap_or(0);

    // Rendered height of a paired row = 1 + comment box for each distinct side
    // with a comment. Context rows have left==right; count that box only once.
    let row_height = |pi: usize| -> usize {
        let p = &pairs[pi];
        let mut h = 1;
        let right = if p.right == p.left { None } else { p.right };
        for side in [p.left, right] {
            if let Some(di) = side {
                if let Some(c) = app.comment_for(&app.diff[di]) {
                    h += comment_box_height(c, wrap_w);
                }
            }
        }
        h
    };

    let start = if app.history_active() {
        diff_scroll_start_center(cursor_row, page, &row_height)
    } else {
        diff_scroll_start_follow(cursor_row, page, &row_height)
    };

    // Render one cell (gutter + text), padded/truncated to `cell_w`, with the
    // appropriate background. `di` None => a blank cell.
    let render_cell = |di: Option<usize>, is_cursor_row: bool| -> Vec<Span<'static>> {
        let Some(di) = di else {
            // Blank cell: pad to cell width.
            let bg = if is_cursor_row {
                Style::default().bg(pal.selected_bg)
            } else {
                Style::default()
            };
            return vec![Span::styled(" ".repeat(cell_w), bg)];
        };
        let dl = &app.diff[di];
        let comment = app.comment_for(dl);
        let cov = match (app.selected_path(), dl.new_lineno.or(dl.old_lineno)) {
            (Some(f), Some(n)) => app.line_coverage(f, n),
            _ => crate::coverage::LineCov::None,
        };
        let gutter_fg = match comment {
            Some(c) if c.stale => pal.yellow,
            Some(_) => pal.accent,
            None => pal.accent_dim,
        };
        let bg = match dl.kind {
            LineKind::Add => Some(pal.add_bg),
            LineKind::Del => Some(pal.del_bg),
            _ => None,
        };
        let cell_bg = if is_cursor_row { Some(pal.selected_bg) } else { bg };
        let gutter_style = {
            let mut s = Style::default().fg(gutter_fg);
            if let Some(b) = cell_bg {
                s = s.bg(b);
            }
            s
        };
        // Text, horizontally scrolled then truncated to text width, syntax-highlighted.
        let shifted: String = dl
            .text
            .chars()
            .skip(app.diff_hscroll)
            .take(text_w)
            .collect();
        let visible = shifted.chars().count();
        let pad = text_w.saturating_sub(visible);
        // Search tint applies to the whole cell when matched (not on cursor row).
        let search_hit = app.search.as_ref().map_or(false, |s| {
            !is_cursor_row && dl.text.to_lowercase().contains(&s.query)
        });
        let mut text_spans: Vec<Span<'static>> = highlight_code(&shifted, ext, app.theme);
        for sp in text_spans.iter_mut() {
            if is_cursor_row {
                sp.style = sp.style.bg(pal.selected_bg);
            } else if search_hit {
                sp.style = sp.style.bg(pal.accent_dim);
            } else if let Some(b) = cell_bg {
                sp.style = sp.style.bg(b);
            }
            if !is_cursor_row && dl.kind == LineKind::Context {
                sp.style = sp.style.add_modifier(Modifier::DIM);
            }
        }
        // Pad the cell to full width so backgrounds fill the column.
        let pad_style = {
            let mut s = Style::default();
            if is_cursor_row {
                s = s.bg(pal.selected_bg);
            } else if let Some(b) = cell_bg {
                s = s.bg(b);
            }
            s
        };
        // Breakpoint (●) / current-stop (▶) marker in the leading gutter column,
        // keeping the 5-col gutter width (marker + 4-digit line number).
        let line_no = dl.new_lineno.or(dl.old_lineno);
        let is_bp = matches!((&bp_file, line_no), (Some(f), Some(n)) if app.has_breakpoint(f, n));
        let bp_on = matches!((&bp_file, line_no), (Some(f), Some(n)) if app.breakpoint_enabled(f, n));
        let is_stopped = stopped_line.is_some() && stopped_line == line_no;
        let mut spans: Vec<Span<'static>> = if is_stopped || is_bp {
            let (marker, fg) = if is_stopped {
                ("▶", pal.tick)
            } else if bp_on {
                ("●", pal.red)
            } else {
                ("○", pal.accent_dim) // disabled breakpoint
            };
            let num = line_no.map(|n| format!("{:>4}", n)).unwrap_or_else(|| "    ".into());
            let mut mstyle = Style::default().fg(fg);
            if let Some(b) = cell_bg {
                mstyle = mstyle.bg(b);
            }
            vec![
                Span::styled(marker.to_string(), mstyle),
                Span::styled(num, gutter_style),
            ]
        } else {
            // 4-digit number + a coverage `│` bar (green/red) in the trailing
            // gutter column, keeping the 5-col width.
            let num = line_no.map(|n| format!("{:>4}", n)).unwrap_or_else(|| "    ".into());
            let (cov_ch, cov_fg) = match cov {
                crate::coverage::LineCov::Covered => ("▌", pal.tick),
                crate::coverage::LineCov::Uncovered => ("▌", pal.red),
                crate::coverage::LineCov::None => (" ", gutter_fg),
            };
            let mut cov_style = Style::default().fg(cov_fg);
            if let Some(b) = cell_bg {
                cov_style = cov_style.bg(b);
            }
            vec![
                Span::styled(num, gutter_style),
                Span::styled(cov_ch.to_string(), cov_style),
            ]
        };
        spans.extend(text_spans);
        spans.push(Span::styled(" ".repeat(pad), pad_style));
        spans
    };

    // A row is highlighted when any of its diff indices fall in the visual-select
    // range (collapses to the single cursor row when nothing is selected).
    let (sel_lo, sel_hi) = app.select_range();
    let in_sel = |di: Option<usize>| matches!(di, Some(i) if i >= sel_lo && i <= sel_hi);

    let mut result: Vec<Line<'static>> = Vec::new();
    let mut rendered_rows = 0usize;
    for p in pairs.iter().skip(start) {
        if rendered_rows >= page {
            break;
        }
        let is_cursor_row = in_sel(p.header) || in_sel(p.left) || in_sel(p.right);

        // Hunk header: full width.
        if let Some(hi) = p.header {
            let shifted: String = app.diff[hi].text.chars().skip(app.diff_hscroll).collect();
            let mut style = Style::default().fg(pal.hunk);
            if is_cursor_row {
                style = style.bg(pal.selected_bg);
            }
            result.push(Line::from(Span::styled(shifted, style)));
            rendered_rows += 1;
            continue;
        }

        // Two cells + separator.
        let mut spans = render_cell(p.left, is_cursor_row);
        let sep_style = if is_cursor_row {
            Style::default().fg(pal.accent_dim).bg(pal.selected_bg)
        } else {
            Style::default().fg(pal.accent_dim)
        };
        spans.push(Span::styled("│", sep_style));
        spans.extend(render_cell(p.right, is_cursor_row));
        result.push(Line::from(spans));
        rendered_rows += 1;

        // Inline comment box(es) full-width under the row. For a context row,
        // left and right are the SAME diff index — render its box once.
        let right = if p.right == p.left { None } else { p.right };
        for side in [p.left, right] {
            if let Some(di) = side {
                if let Some(c) = app.comment_for(&app.diff[di]) {
                    push_comment_box(&mut result, &mut rendered_rows, page, c, wrap_w, &pal);
                }
            }
        }
    }
    result
}

fn render_diff(frame: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    let ext = app
        .selected_path()
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();
    // Absolute path of the file shown in the diff, for breakpoint lookups.
    let bp_file = app.selected_path().map(|p| app.repo_root.join(p));
    // Repo-relative path for coverage lookups (matched by suffix).
    let cov_file = app.selected_path().cloned();
    // Line where the active session is currently stopped (in this file).
    let stopped_line = app
        .debug
        .as_ref()
        .and_then(|d| d.active_session())
        .and_then(|s| s.stopped_at.as_ref())
        .filter(|(f, _)| bp_file.as_deref() == Some(f.as_path()))
        .map(|(_, l)| *l);

    let ctx_label = if app.full_file {
        "full file".to_string()
    } else {
        format!("ctx {}", app.context_lines)
    };
    let title = if let Some(commit) = app.history_current_commit() {
        let h = app.history.as_ref().unwrap();
        format!(
            " {} @ {} ({}/{}) — {} ",
            h.file.display(),
            commit.short,
            h.idx,
            h.commits.len(),
            commit.summary,
        )
    } else {
        app.selected_path()
            .map(|p| format!(" Diff: {} ({}) ", p.display(), ctx_label))
            .unwrap_or_else(|| " Diff ".to_string())
    };

    let lines: Vec<Line> = if app.view == ViewMode::Commits && app.open_commit.is_none() {
        vec![Line::from(Span::styled(
            "Press Enter to open commit  ·  [/] switch view",
            Style::default().fg(pal.placeholder),
        ))]
    } else if app.diff.is_empty() {
        vec![Line::from(Span::styled(
            "No changes",
            Style::default().fg(pal.placeholder),
        ))]
    } else if app.split_diff {
        build_split_lines(app, area, &ext)
    } else {
        let page = area.height.saturating_sub(2) as usize;

        // Inline comment box body is indented by "    │ " (6 columns). Wrap comment and
        // response text to the remaining inner width, less RIGHT_PAD, so long comments
        // don't overflow or butt against the pane border.
        let inner_w = area.width.saturating_sub(2) as usize; // minus the diff pane borders
        let wrap_w = inner_w.saturating_sub(BODY_PREFIX_W + RIGHT_PAD).max(1);

        // Compute the rendered height (diff line + its inline comment box lines) for a
        // given diff index, so we can scroll in rendered-line space and guarantee the
        // cursor line AND its comment are always visible. Wrapped text height must match
        // the body/response render below or scrolling drifts.
        // comment box: 1 (top) + wrapped text + (if response: 1 blank + wrapped response) + 1 (bottom)
        let rendered_height = |i: usize| -> usize {
            let dl = &app.diff[i];
            let comment_lines = app
                .comment_for(dl)
                .map(|c| comment_box_height(c, wrap_w))
                .unwrap_or(0);
            1 + comment_lines
        };

        // History overlay: center the cursor line. Otherwise: cursor at viewport bottom.
        let start = if app.history_active() {
            diff_scroll_start_center(app.diff_cursor, page, &rendered_height)
        } else {
            diff_scroll_start_follow(app.diff_cursor, page, &rendered_height)
        };

        let (sel_lo, sel_hi) = app.select_range();
        let mut result: Vec<Line> = Vec::new();
        let mut rendered_rows: usize = 0;
        for (idx, dl) in app.diff.iter().enumerate().skip(start) {
            if rendered_rows >= page {
                break;
            }
            // Highlight the whole visual-select range (collapses to the single
            // cursor line when nothing is selected).
            let is_cursor = idx >= sel_lo && idx <= sel_hi;
            let bg = match dl.kind {
                LineKind::Add => Some(pal.add_bg),
                LineKind::Del => Some(pal.del_bg),
                _ => None,
            };
            if dl.kind == LineKind::Hunk {
                let shifted: String = dl.text.chars().skip(app.diff_hscroll).collect();
                let mut span = Span::styled(shifted, Style::default().fg(pal.hunk));
                if is_cursor {
                    span.style = span.style.bg(pal.selected_bg);
                }
                result.push(Line::from(span));
                rendered_rows += 1;
                continue;
            }
            // Gutter: YELLOW for stale-commented lines, ACCENT for normal commented, ACCENT_DIM otherwise.
            let comment = app.comment_for(dl);
            // Coverage (when on) is shown as a colored `│` bar (see below), not
            // by recoloring the line number.
            let cov = match (&cov_file, dl.new_lineno.or(dl.old_lineno)) {
                (Some(f), Some(n)) => app.line_coverage(f, n),
                _ => crate::coverage::LineCov::None,
            };
            let gutter_fg = match comment {
                Some(c) if c.stale => pal.yellow,
                Some(_) => pal.accent,
                None => pal.accent_dim,
            };
            let gutter_style = if is_cursor {
                Style::default().fg(gutter_fg).bg(pal.selected_bg)
            } else {
                Style::default().fg(gutter_fg)
            };
            // Breakpoint marker (●) and current-stop marker (▶) in a 1-col
            // sidebar before the line-number gutter.
            let line_no = dl.new_lineno.or(dl.old_lineno);
            let is_bp = match (&bp_file, line_no) {
                (Some(f), Some(n)) => app.has_breakpoint(f, n),
                _ => false,
            };
            let bp_on = matches!((&bp_file, line_no), (Some(f), Some(n)) if app.breakpoint_enabled(f, n));
            let is_stopped = stopped_line.is_some() && stopped_line == line_no;
            let (marker, marker_fg) = if is_stopped {
                ("▶", pal.tick)
            } else if is_bp && bp_on {
                ("●", pal.red)
            } else if is_bp {
                ("○", pal.accent_dim) // disabled breakpoint
            } else {
                (" ", pal.accent_dim)
            };
            let mut marker_style = Style::default().fg(marker_fg);
            if is_cursor {
                marker_style = marker_style.bg(pal.selected_bg);
            } else if let Some(b) = bg {
                marker_style = marker_style.bg(b);
            }
            let marker_span = Span::styled(marker.to_string(), marker_style);
            let gutter_span = Span::styled(gutter(dl), gutter_style);
            let shifted: String = dl.text.chars().skip(app.diff_hscroll).collect();
            let mut spans: Vec<Span> = highlight_code(&shifted, &ext, app.theme);
            if is_cursor {
                for s in spans.iter_mut() {
                    s.style = s.style.bg(pal.selected_bg);
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
            // Search: tint the background of lines that match the active query.
            if let Some(s) = app.search.as_ref() {
                if !is_cursor && dl.text.to_lowercase().contains(&s.query) {
                    for sp in spans.iter_mut() {
                        sp.style = sp.style.bg(pal.accent_dim);
                    }
                }
            }
            // Coverage bar: a colored `│` between the marker and the line
            // number (green covered / red uncovered / blank when no data).
            let (cov_ch, cov_fg) = match cov {
                crate::coverage::LineCov::Covered => ("▌", pal.tick),
                crate::coverage::LineCov::Uncovered => ("▌", pal.red),
                crate::coverage::LineCov::None => (" ", pal.accent_dim),
            };
            let mut cov_style = Style::default().fg(cov_fg);
            if is_cursor {
                cov_style = cov_style.bg(pal.selected_bg);
            } else if let Some(b) = bg {
                cov_style = cov_style.bg(b);
            }
            let cov_span = Span::styled(cov_ch.to_string(), cov_style);

            let mut all_spans = Vec::with_capacity(3 + spans.len());
            all_spans.push(marker_span);
            all_spans.push(cov_span);
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
                push_comment_box(&mut result, &mut rendered_rows, page, c, wrap_w, &pal);
            }
        }
        result
    };

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(focused_border(app, Pane::Diff))
            .title(title)
            // Help hint sits in the bottom-right of the diff pane's border.
            .title_bottom(
                Line::from(Span::styled(
                    " ? help ",
                    Style::default().fg(pal.accent_dim),
                ))
                .right_aligned(),
            ),
    );
    frame.render_widget(para, area);
}

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    // While the search input line is open, it owns the status row.
    if let Some(buf) = app.search_input.as_ref() {
        let text = format!("/{}\u{2588}", buf); // trailing block as a cursor
        let para = Paragraph::new(text).style(Style::default().fg(pal.accent));
        frame.render_widget(para, area);
        return;
    }
    // Coverage metrics on the right when the highlight is on; status message on
    // the left.
    let msg = app.status_msg.clone().unwrap_or_default();
    if app.show_coverage {
        if let Some(cov) = app.coverage.as_ref() {
            let (c, t) = cov.totals();
            let mut metrics = format!("cov {:.0}% ({c}/{t})", cov.percent());
            if let Some((fc, ft)) = app.selected_path().and_then(|p| cov.file_totals(p)) {
                let fpct = if ft == 0 { 0.0 } else { 100.0 * fc as f64 / ft as f64 };
                metrics.push_str(&format!("  ·  file {fpct:.0}% ({fc}/{ft})"));
            }
            // Lay out: status msg (left) | metrics (right).
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Min(1),
                    Constraint::Length(metrics.chars().count() as u16 + 1),
                ])
                .split(area);
            frame.render_widget(
                Paragraph::new(msg).style(Style::default().fg(pal.accent_dim)),
                cols[0],
            );
            frame.render_widget(
                Paragraph::new(metrics).style(Style::default().fg(pal.tick)),
                cols[1],
            );
            return;
        }
    }
    // Transient status message only; the "? for help" hint lives in the diff pane's
    // bottom-right border (see render_diff).
    let para = Paragraph::new(msg).style(Style::default().fg(pal.accent_dim));
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

/// The debug launch-type picker overlay (worktree / commit / remote attach).
fn render_launch_picker(frame: &mut Frame, app: &App) {
    use crate::app::LaunchMode;
    let pal = app.palette();
    let Some(sel) = app.debug_launch_pick else { return };
    let modes = app.launch_modes();
    let area = centered_rect(48, 30, frame.area());
    frame.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    for (i, m) in modes.iter().enumerate() {
        let label = match m {
            LaunchMode::Worktree => "Debug the working tree",
            LaunchMode::Commit => "Debug the selected commit (worktree)",
            LaunchMode::Remote => "Attach to the remote target",
        };
        let mut st = Style::default().fg(pal.accent);
        if i == sel {
            st = st.bg(pal.selected_bg).add_modifier(Modifier::BOLD);
        }
        lines.push(Line::from(Span::styled(format!("  {label}"), st)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " ↑/↓ select · Enter start · Esc cancel",
        Style::default().fg(pal.accent_dim),
    )));
    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(pal.accent))
            .title(" Debug "),
    );
    frame.render_widget(para, area);
}

fn render_input_modal(frame: &mut Frame, app: &App, input: &InputState) {
    let pal = app.palette();
    let has_snap = input.debug_snapshot.is_some();
    // Taller modal when a debug snapshot is shown alongside the text field.
    let area = centered_rect(64, if has_snap { 70 } else { 40 }, frame.area());
    frame.render_widget(Clear, area);
    let title = format!(
        " Comment line {} (Ctrl-S save · Esc cancel) ",
        input.target_line
    );
    let display_text = format!("{}▏", input.buffer);
    let para = Paragraph::new(display_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(pal.accent))
                .padding(Padding::horizontal(1))
                .title(title),
        )
        .wrap(Wrap { trim: false });

    let Some(snap) = input.debug_snapshot.as_ref() else {
        frame.render_widget(para, area);
        return;
    };

    // Split: text field on top, debug stack/locals below.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);
    frame.render_widget(para, rows[0]);

    let on = input.attach_debug;
    let dbg_title = format!(
        " Debug stack — Ctrl-D: attach [{}] ",
        if on { "x" } else { " " }
    );
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("@ {} ({})", snap.session_label, snap.stopped_line),
        Style::default().fg(pal.tick).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        " call stack:",
        Style::default().fg(pal.hunk),
    )));
    for f in &snap.stack {
        let loc = f
            .file
            .as_deref()
            .map(|p| {
                let base = std::path::Path::new(p)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(p);
                format!("{base}:{}", f.line)
            })
            .unwrap_or_default();
        lines.push(Line::from(Span::styled(
            format!("   {}  {loc}", f.name),
            Style::default().fg(pal.accent_dim),
        )));
    }
    if !snap.locals.is_empty() {
        lines.push(Line::from(Span::styled(
            " locals:",
            Style::default().fg(pal.hunk),
        )));
        for v in &snap.locals {
            lines.push(Line::from(vec![
                Span::styled(format!("   {} = ", v.name), Style::default().fg(pal.accent)),
                Span::styled(v.value.clone(), Style::default().fg(pal.accent_dim)),
            ]));
        }
    }
    let dim = if on { pal.accent } else { pal.accent_dim };
    let dbg = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(dim))
                .padding(Padding::horizontal(1))
                .title(dbg_title),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(dbg, rows[1]);
}

/// Keybindings grouped by category for the help overlay. Each group is a
/// (title, &[(key, description)]) pair rendered as a labelled section.
const HELP_SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigation",
        &[
            ("Tab", "switch focus (Files/Diff/Comments)"),
            ("j/k, ↑/↓", "move selection / cursor"),
            ("J/K, ⇧↑/⇧↓", "jump (fast scroll)"),
            ("gg / G", "top / bottom"),
            (
                "Enter",
                "open file diff / open commit / fold dir / jump to comment",
            ),
            ("Esc", "back / focus files"),
            ("[ / ]", "switch Changes/Commits view"),
        ],
    ),
    (
        "Diff view",
        &[
            ("h/l, ←/→", "scroll diff horizontally"),
            ("+/-", "context lines (±5, + at max → full file)"),
            ("F", "full-file diff toggle"),
            ("v", "side-by-side / unified diff"),
            ("y", "copy line / selection to clipboard"),
            ("V", "visual select (move, then y to copy, Esc cancels)"),
        ],
    ),
    (
        "History & search",
        &[
            ("H", "file-history overlay for the current file diff"),
            ("{ / }", "older / newer revision (in history overlay)"),
            ("/", "search within the diff"),
            ("n / N", "next / previous search match"),
        ],
    ),
    (
        "Layout",
        &[
            ("a", "fold/unfold all directories"),
            ("O", "view all files / changes only"),
            ("%", "toggle coverage highlight"),
            ("M", "run coverage command + show"),
            ("z", "hide/show file pane"),
            ("< / >", "resize focused pane (files / right)"),
            ("C", "toggle comment-list pane"),
        ],
    ),
    (
        "Review",
        &[
            ("c", "comment on line"),
            ("s", "stage / unstage file"),
            ("Space", "toggle reviewed"),
            ("R", "hide reviewed files"),
        ],
    ),
    (
        "Debug",
        &[
            ("b", "toggle breakpoint on cursor line"),
            ("D", "debug: pick worktree / commit / remote attach"),
            ("c / n / i / o", "continue / over / in / out"),
            ("[ / ]", "right pane: Comments / Debug tab"),
            ("t", "Debug: Vars / Breakpoints"),
            ("Enter", "Vars: expand a variable"),
            ("h / l", "Debug: scroll horizontally"),
            ("Space / d", "bp list: enable-disable / delete"),
            ("Enter", "bp list: jump to breakpoint"),
            ("Ctrl-D", "comment box: attach stack"),
            ("X", "end debug sessions"),
        ],
    ),
    (
        "App",
        &[
            ("r", "refresh"),
            ("T", "toggle theme"),
            ("?", "this help"),
            ("qq / Ctrl-C", "quit"),
        ],
    ),
];

fn render_help_modal(frame: &mut Frame, app: &App) {
    let pal = app.palette();
    let area = centered_rect(92, 80, frame.area());
    frame.render_widget(Clear, area);

    // Build the rendered lines for a slice of sections.
    let build = |sections: &[(&str, &[(&str, &str)])]| -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (si, (title, entries)) in sections.iter().enumerate() {
            if si > 0 {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                format!(" {}", title),
                Style::default()
                    .fg(pal.hunk)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            for (key, desc) in entries.iter() {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:14}", key),
                        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(desc.to_string(), Style::default().fg(pal.accent_dim)),
                ]));
            }
        }
        lines
    };

    // Split sections across two columns so the overlay never clips vertically.
    let mid = HELP_SECTIONS.len().div_ceil(2);
    let (left, right) = HELP_SECTIONS.split_at(mid);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.accent))
        .title(" Keybindings (? or Esc to close) ");
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);
    frame.render_widget(Paragraph::new(build(left)), cols[0]);
    frame.render_widget(Paragraph::new(build(right)), cols[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, DiffLine, FileChange, Status};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn dl(kind: LineKind, text: &str) -> DiffLine {
        DiffLine {
            kind,
            text: text.into(),
            old_lineno: None,
            new_lineno: None,
        }
    }

    #[test]
    fn split_diff_renders_both_sides_without_panic() {
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.split_diff = true;
        app.set_diff(vec![
            dl(LineKind::Hunk, "@@ -1,2 +1,2 @@"),
            dl(LineKind::Del, "old_left_token"),
            dl(LineKind::Add, "new_right_token"),
            dl(LineKind::Context, "shared_ctx"),
        ]);
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("old_left"), "old side token must render");
        assert!(dump.contains("new_right"), "new side token must render");
        assert!(dump.contains("│"), "column separator must render");
    }

    #[test]
    fn split_context_comment_renders_one_box() {
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.split_diff = true;
        app.set_diff(vec![DiffLine {
            kind: LineKind::Context,
            text: "ctx".into(),
            old_lineno: Some(1),
            new_lineno: Some(1),
        }]);
        // Comment on the context line (new_lineno 1).
        app.comments.set(
            PathBuf::from("a.rs"),
            1,
            "@@".to_string(),
            "uniq_box_text".to_string(),
            "ctx".to_string(),
            vec![],
            vec![],
            0,
        );
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        // The comment box top border "╭─ comment" must appear exactly once.
        let box_tops = dump.matches("╭─").count();
        assert_eq!(box_tops, 1, "context-line comment must render a single box");
    }

    #[test]
    fn pair_rows_context_maps_to_both_sides() {
        let diff = vec![dl(LineKind::Context, "same")];
        let rows = pair_diff_rows(&diff);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].left, Some(0));
        assert_eq!(rows[0].right, Some(0));
        assert_eq!(rows[0].header, None);
    }

    #[test]
    fn pair_rows_hunk_is_header() {
        let diff = vec![dl(LineKind::Hunk, "@@ -1 +1 @@")];
        let rows = pair_diff_rows(&diff);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].header, Some(0));
        assert_eq!(rows[0].left, None);
        assert_eq!(rows[0].right, None);
    }

    #[test]
    fn pair_rows_equal_del_add_run_zips() {
        // del0 del1 add2 add3 -> two rows: (0,2) (1,3)
        let diff = vec![
            dl(LineKind::Del, "old0"),
            dl(LineKind::Del, "old1"),
            dl(LineKind::Add, "new0"),
            dl(LineKind::Add, "new1"),
        ];
        let rows = pair_diff_rows(&diff);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].left, rows[0].right), (Some(0), Some(2)));
        assert_eq!((rows[1].left, rows[1].right), (Some(1), Some(3)));
    }

    #[test]
    fn pair_rows_unequal_run_pads_blank() {
        // 2 dels, 1 add -> rows: (0,2) (1,None)
        let diff = vec![
            dl(LineKind::Del, "old0"),
            dl(LineKind::Del, "old1"),
            dl(LineKind::Add, "new0"),
        ];
        let rows = pair_diff_rows(&diff);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].left, rows[0].right), (Some(0), Some(2)));
        assert_eq!((rows[1].left, rows[1].right), (Some(1), None));
    }

    #[test]
    fn pair_rows_add_only_run_has_blank_left() {
        let diff = vec![dl(LineKind::Add, "new0"), dl(LineKind::Add, "new1")];
        let rows = pair_diff_rows(&diff);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].left, rows[0].right), (None, Some(0)));
        assert_eq!((rows[1].left, rows[1].right), (None, Some(1)));
    }

    #[test]
    fn pair_rows_del_only_run_has_blank_right() {
        let diff = vec![dl(LineKind::Del, "old0")];
        let rows = pair_diff_rows(&diff);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].left, rows[0].right), (Some(0), None));
    }

    #[test]
    fn wrap_text_wraps_long_lines_on_word_boundaries() {
        let lines = wrap_text("the quick brown fox", 9);
        // "the quick" (9), "brown fox" (9)
        assert_eq!(
            lines,
            vec!["the quick".to_string(), "brown fox".to_string()]
        );
        // No visual line exceeds the width.
        assert!(lines.iter().all(|l| l.chars().count() <= 9));
    }

    #[test]
    fn wrap_text_hard_splits_overlong_word() {
        let lines = wrap_text("abcdefghij", 4);
        assert_eq!(lines, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_text_preserves_blank_lines_and_short_lines() {
        let lines = wrap_text("hi\n\nbye", 10);
        assert_eq!(
            lines,
            vec!["hi".to_string(), String::new(), "bye".to_string()]
        );
    }

    #[test]
    fn wrap_text_clamps_zero_width_to_one() {
        let lines = wrap_text("ab", 0);
        assert_eq!(lines, vec!["a", "b"]);
    }

    fn comment_with_response(text: &str, response: &str) -> crate::comments::Comment {
        crate::comments::Comment {
            file: PathBuf::from("a.rs"),
            line: 1,
            hunk: String::new(),
            text: text.into(),
            line_text: String::new(),
            context_before: vec![],
            context_after: vec![],
            orig_line: 0,
            stale: false,
            status: CommentStatus::Open,
            response: Some(response.into()),
            updated: 0,
            debug_snapshot: None,
        }
    }

    /// Render a comment box into plain strings (joining each line's spans).
    fn render_box(c: &crate::comments::Comment, wrap_w: usize) -> Vec<String> {
        let pal = Palette::for_theme(crate::theme::Theme::Dark);
        let mut out = Vec::new();
        let mut rows = 0usize;
        push_comment_box(&mut out, &mut rows, usize::MAX, c, wrap_w, &pal);
        out.iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn response_lines_never_overflow_inner_width() {
        // inner_w = wrap_w + BODY_PREFIX_W (body prefix is 6 cols).
        let wrap_w = 30usize;
        let inner_w = wrap_w + BODY_PREFIX_W;
        let resp = "this is a fairly long agent response that must wrap across \
                    several visual lines without spilling past the pane border";
        let c = comment_with_response("short comment", resp);
        for line in render_box(&c, wrap_w) {
            assert!(
                line.chars().count() <= inner_w,
                "rendered line {:?} ({} cols) exceeds inner_w {}",
                line,
                line.chars().count(),
                inner_w
            );
        }
    }

    #[test]
    fn comment_box_height_matches_rendered_line_count() {
        let wrap_w = 24usize;
        let resp = "multi line response text long enough to wrap onto several \
                    lines so the height calc and the renderer must agree exactly";
        let body = "comment body that itself wraps onto two or more lines here";
        let c = comment_with_response(body, resp);
        let rendered = render_box(&c, wrap_w);
        assert_eq!(
            rendered.len(),
            comment_box_height(&c, wrap_w),
            "height calc must equal rendered line count or scrolling drifts"
        );
    }

    fn app_with_diff() -> App {
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Hunk,
                text: "@@ -1 +1 @@".into(),
                old_lineno: None,
                new_lineno: None,
            },
            DiffLine {
                kind: LineKind::Add,
                text: "let x = 1;".into(),
                old_lineno: None,
                new_lineno: Some(1),
            },
        ]);
        app
    }

    #[test]
    fn status_row_shows_search_input() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = app_with_diff();
        app.focus = Pane::Diff;
        app.search_input = Some("foo".into());
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("/foo"),
            "status row should show the /-prefixed query while typing"
        );
    }

    #[test]
    fn render_with_active_search_is_stable() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = app_with_diff();
        app.focus = Pane::Diff;
        app.search = Some(crate::app::SearchState {
            query: "let".into(),
            matches: vec![1],
            cur: 0,
        });
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(!dump.is_empty());
        assert!(dump.contains("let x = 1;"));
    }

    #[test]
    fn diff_title_shows_history_revision() {
        use crate::app::{CommentScope, FileHistory};
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = app_with_diff();
        app.selected = 1;
        app.focus = Pane::Diff;
        app.history = Some(FileHistory {
            file: std::path::PathBuf::from("a.rs"),
            commits: vec![crate::git::CommitInfo {
                id: "deadbeefcafebabe".into(),
                short: "deadbeef".into(),
                summary: "old change".into(),
                author: "t".into(),
                time: "2024-01-01".into(),
            }],
            idx: 1,
            baseline_scope: CommentScope::Worktree,
        });
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("deadbeef"), "title should show short sha");
        assert!(dump.contains("1/1"), "title should show revision position");
        assert!(dump.contains("old change"), "title should show summary");
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
    fn diff_scroll_start_center_places_cursor_mid_viewport() {
        let rh = |_| 1usize;
        // page 10, cursor 15 -> ~4 lines above -> start 11
        assert_eq!(diff_scroll_start_center(15, 10, &rh), 11);
        // early cursor: clamped to top
        assert_eq!(diff_scroll_start_center(2, 10, &rh), 0);
    }

    #[test]
    fn diff_scroll_start_follow_places_cursor_at_bottom() {
        let rh = |_| 1usize;
        // page 10, cursor 15 -> start 6 (lines 6..=15 fill the page)
        assert_eq!(diff_scroll_start_follow(15, 10, &rh), 6);
    }

    #[test]
    fn history_mode_scrolls_cursor_toward_center() {
        use crate::app::{CommentScope, FileHistory};
        use ratatui::layout::Rect;
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let diff_area = Rect::new(0, 0, 80, 8); // page = 6 diff lines
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        let mut diff_lines = vec![DiffLine {
            kind: LineKind::Hunk,
            text: "@@".into(),
            old_lineno: None,
            new_lineno: None,
        }];
        for i in 0..20u32 {
            diff_lines.push(DiffLine {
                kind: LineKind::Context,
                text: format!("LINE_{i}"),
                old_lineno: Some(i + 1),
                new_lineno: Some(i + 1),
            });
        }
        app.set_diff(diff_lines);
        // Index 19 = LINE_18 (index 0 is the hunk header).
        app.diff_cursor = 19;

        // Default (follow): LINE_13 visible; centered history mode hides it.
        terminal.draw(|f| render_diff(f, &app, diff_area)).unwrap();
        let follow_dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            follow_dump.contains("LINE_13"),
            "follow scroll should include LINE_13 above cursor"
        );

        app.history = Some(FileHistory {
            file: PathBuf::from("a.rs"),
            commits: vec![crate::git::CommitInfo {
                id: "abc".into(),
                short: "abc".into(),
                summary: "s".into(),
                author: "t".into(),
                time: "2024".into(),
            }],
            idx: 1,
            baseline_scope: CommentScope::Worktree,
        });

        terminal.draw(|f| render_diff(f, &app, diff_area)).unwrap();
        let center_dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            !center_dump.contains("LINE_13"),
            "centered history scroll should hide LINE_13"
        );
        assert!(
            center_dump.contains("LINE_16"),
            "centered history scroll should show lines nearer the cursor"
        );
        assert!(
            center_dump.contains("LINE_18"),
            "centered history scroll must show the cursor line"
        );
    }

    #[test]
    fn scroll_offset_hides_earlier_lines() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = app_with_diff();
        // With the cursor near the end, the viewport scrolls so earlier lines are not rendered.
        app.set_diff({
            let mut lines = vec![crate::app::DiffLine {
                kind: LineKind::Hunk,
                text: "@@ -1 +1 @@".into(),
                old_lineno: None,
                new_lineno: None,
            }];
            // Add 18 context lines after hunk so page=18 and cursor=18 scrolls past the hunk.
            for i in 1..=18u32 {
                lines.push(crate::app::DiffLine {
                    kind: LineKind::Add,
                    text: "let x = 1;".into(),
                    old_lineno: None,
                    new_lineno: Some(i),
                });
            }
            lines
        });
        // With page=18, cursor=18 -> scroll = 18+1-18 = 1, so hunk at index 0 is hidden.
        app.diff_cursor = 18;
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(!dump.contains("@@ -1 +1 @@"));
        assert!(dump.contains("let x = 1;"));
    }

    #[test]
    fn empty_diff_shows_placeholder() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let app = App::new(files, vec![], PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("No changes"));
    }

    #[test]
    fn hscroll_offsets_diff_text() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.set_diff(vec![DiffLine {
            kind: LineKind::Context,
            text: "ABCDEFGHIJ".into(),
            old_lineno: Some(1),
            new_lineno: Some(1),
        }]);
        // no scroll: "ABCDEF..." visible
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump0: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump0.contains("ABCDEFGHIJ"));
        // scroll right 4: leading "ABCD" gone, "EFGHIJ" remains
        app.diff_hscroll = 4;
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump1: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump1.contains("EFGHIJ"));
        assert!(!dump1.contains("ABCDEFGHIJ"));
    }

    #[test]
    fn diff_shows_line_numbers() {
        use crate::app::LineKind;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.set_diff(vec![
            DiffLine {
                kind: LineKind::Context,
                text: "ctx".into(),
                old_lineno: Some(7),
                new_lineno: Some(7),
            },
            DiffLine {
                kind: LineKind::Add,
                text: "added".into(),
                old_lineno: None,
                new_lineno: Some(8),
            },
            DiffLine {
                kind: LineKind::Del,
                text: "removed".into(),
                old_lineno: Some(5),
                new_lineno: None,
            },
        ]);
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
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
            FileChange {
                path: PathBuf::from("src/main.rs"),
                status: Status::Modified,
            },
            FileChange {
                path: PathBuf::from("top.rs"),
                status: Status::Modified,
            },
        ];
        let app = App::new(files, vec![], PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("src"),
            "directory name 'src' missing from tree view"
        );
        assert!(
            dump.contains("main.rs"),
            "file basename 'main.rs' missing from tree view"
        );
        assert!(
            dump.contains("top.rs"),
            "file basename 'top.rs' missing from tree view"
        );
        assert!(
            !dump.contains("src/main.rs"),
            "full path 'src/main.rs' should not appear; tree view shows basenames"
        );
    }

    #[test]
    fn reviewed_file_shows_tick() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // select a.rs row (row 1) and review it
        app.selected = 1;
        app.toggle_reviewed(); // review a.rs
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("✓"));
    }

    #[test]
    fn both_sections_headers_render_when_both_non_empty() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let unstaged = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let staged = vec![FileChange {
            path: PathBuf::from("b.rs"),
            status: Status::Added,
        }];
        let app = App::new(unstaged, staged, PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("Unstaged"), "Unstaged header missing");
        assert!(dump.contains("Staged"), "Staged header missing");
        assert!(dump.contains("a.rs"), "a.rs missing");
        assert!(dump.contains("b.rs"), "b.rs missing");
    }

    #[test]
    fn files_title_has_no_mode_label() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let app = App::new(files, vec![], PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        // The block title shows the tab bar with Changes and Commits tabs.
        // In Changes mode, Changes tab is active (in brackets).
        assert!(dump.contains("Changes"), "Changes tab missing from title");
        assert!(dump.contains("Commits"), "Commits tab missing from title");
        assert!(
            !dump.contains("STAGED"),
            "[STAGED] mode label should be gone"
        );
        assert!(
            !dump.contains("UNSTAGED"),
            "[UNSTAGED] mode label should be gone"
        );
    }

    #[test]
    fn hidden_file_pane_shows_only_diff() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("zzz.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.show_files = false;
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(!dump.contains("zzz.rs")); // file pane hidden
        assert!(dump.contains("No changes") || dump.contains("Diff")); // diff pane present
    }

    #[test]
    fn modified_file_shows_m_status_letter() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let app = App::new(files, vec![], PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains('M'),
            "Modified file should show 'M' status letter"
        );
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
            debug_snapshot: None,
            attach_debug: false,
        });
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("hello world"),
            "modal buffer text must appear"
        );
        assert!(
            dump.contains("Comment"),
            "modal title must contain 'Comment'"
        );
    }

    #[test]
    fn commented_line_shows_comment_text_inline() {
        use crate::app::LineKind;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1; // select a.rs row
        app.focus = Pane::Diff;
        app.set_diff(vec![DiffLine {
            kind: LineKind::Add,
            text: "let x = 1;".into(),
            old_lineno: None,
            new_lineno: Some(5),
        }]);
        // attach a comment for a.rs line 5
        app.comments.set(
            PathBuf::from("a.rs"),
            5,
            "@@ -3,4 @@".to_string(),
            "review note here".to_string(),
            "let x = 1;".to_string(),
            vec![],
            vec![],
            0,
        );
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("review note here"),
            "inline comment text must appear below commented line"
        );
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
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
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
            0,
        );

        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
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
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.set_diff(vec![DiffLine {
            kind: LineKind::Add,
            text: "fn main() {}".into(),
            old_lineno: None,
            new_lineno: Some(1),
        }]);
        // Simulate what main.rs does after Fix 1: store the trimmed text.
        let raw = "   trimmed_note   ";
        let trimmed = raw.trim().to_string();
        app.comments.set(
            PathBuf::from("a.rs"),
            1,
            "".to_string(),
            trimmed,
            "fn main() {}".to_string(),
            vec![],
            vec![],
            0,
        );

        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("trimmed_note"),
            "trimmed comment text must appear"
        );
        // The raw padded string (with surrounding spaces) must not be stored/rendered.
        assert!(
            !dump.contains("   trimmed_note   "),
            "padded comment text must not appear"
        );
    }

    #[test]
    fn added_file_shows_a_deleted_shows_d() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![
            FileChange {
                path: PathBuf::from("new.rs"),
                status: Status::Added,
            },
            FileChange {
                path: PathBuf::from("old.rs"),
                status: Status::Deleted,
            },
        ];
        let app = App::new(files, vec![], PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains('A'),
            "Added file should show 'A' status letter"
        );
        assert!(
            dump.contains('D'),
            "Deleted file should show 'D' status letter"
        );
    }

    #[test]
    fn resolved_comment_shows_status_and_response() {
        use crate::app::LineKind;
        use crate::comments::CommentStatus;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.set_diff(vec![DiffLine {
            kind: LineKind::Add,
            text: "fn foo() {}".into(),
            old_lineno: None,
            new_lineno: Some(3),
        }]);
        app.comments.set(
            PathBuf::from("a.rs"),
            3,
            "@@".to_string(),
            "please fix this".to_string(),
            "fn foo() {}".to_string(),
            vec![],
            vec![],
            0,
        );
        app.comments.items[0].status = CommentStatus::Resolved;
        app.comments.items[0].response = Some("Fixed it".to_string());

        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("resolved"),
            "resolved status must appear in comment box"
        );
        assert!(
            dump.contains("Fixed it"),
            "agent response must appear in comment box"
        );
    }

    #[test]
    fn stale_comment_shows_outdated_prefix() {
        use crate::app::LineKind;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.selected = 1;
        app.focus = Pane::Diff;
        app.set_diff(vec![DiffLine {
            kind: LineKind::Add,
            text: "let y = 2;".into(),
            old_lineno: None,
            new_lineno: Some(7),
        }]);
        // Insert a stale comment directly
        app.comments.set(
            PathBuf::from("a.rs"),
            7,
            "@@".to_string(),
            "stale note".to_string(),
            "let y = 2;".to_string(),
            vec![],
            vec![],
            0,
        );
        // Mark it stale manually (simulating relocation failure)
        app.comments.items[0].stale = true;

        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("outdated"),
            "stale comment must show '(outdated)' prefix"
        );
        assert!(
            dump.contains("stale note"),
            "stale comment text must still appear"
        );
    }

    #[test]
    fn comment_pane_shows_status_header_and_item() {
        let backend = TestBackend::new(160, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // Enable comment pane
        app.show_comments = true;
        // Add an Open comment
        app.comments.set(
            PathBuf::from("a.rs"),
            5,
            "@@ -3,4 @@".to_string(),
            "look at this".to_string(),
            "fn foo()".to_string(),
            vec![],
            vec![],
            0,
        );
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        // The comment pane title must appear
        assert!(dump.contains("Comments"), "comment pane title must appear");
        // The Open status header must appear
        assert!(
            dump.contains("Open") || dump.contains("open"),
            "Open status header must appear"
        );
        // The file basename must appear
        assert!(
            dump.contains("a.rs"),
            "file basename must appear in comment list"
        );
    }

    #[test]
    fn help_hint_renders_in_diff_pane_border() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let app = App::new(files, vec![], PathBuf::from("/repo"));
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("? help"),
            "diff pane border must show the '? help' hint"
        );
    }

    #[test]
    fn comment_pane_hidden_by_default() {
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        // show_comments is false by default
        app.comments.set(
            PathBuf::from("a.rs"),
            1,
            "@@".to_string(),
            "hidden comment".to_string(),
            "fn x()".to_string(),
            vec![],
            vec![],
            0,
        );
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        // Comment pane title must NOT appear when show_comments is false
        assert!(
            !dump.contains("hidden comment"),
            "comment text must not appear when pane hidden"
        );
    }

    #[test]
    fn help_overlay_shows_keybindings_title() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.show_help = true;
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("Keybindings"),
            "help overlay must show 'Keybindings' title"
        );
    }

    #[test]
    fn help_overlay_shows_theme_toggle_key() {
        let backend = TestBackend::new(80, 50);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.show_help = true;
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("theme"),
            "help overlay must mention theme toggle"
        );
        // Grouped help: category headers must render.
        assert!(
            dump.contains("Navigation"),
            "help overlay must show the Navigation section header"
        );
        assert!(
            dump.contains("History"),
            "help overlay must show the History & search section header"
        );
    }

    #[test]
    fn empty_response_does_not_break_layout() {
        // A comment with an empty-string response must render without a phantom line
        // (regression: rendered-height overcounted, clipping the box bottom).
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
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
            updated: 0,
            debug_snapshot: None,
        });
        app.set_diff(vec![DiffLine {
            kind: LineKind::Add,
            text: "let x = 1;".into(),
            old_lineno: None,
            new_lineno: Some(1),
        }]);
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        // the comment text renders; no panic; "response:" label NOT shown for empty response
        assert!(dump.contains("please fix"));
        assert!(!dump.contains("response:"));
    }

    #[test]
    fn light_theme_renders_without_panic() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let files = vec![FileChange {
            path: PathBuf::from("a.rs"),
            status: Status::Modified,
        }];
        let mut app = App::new(files, vec![], PathBuf::from("/repo"));
        app.set_diff(vec![DiffLine {
            kind: LineKind::Add,
            text: "let x = 1;".into(),
            old_lineno: None,
            new_lineno: Some(1),
        }]);
        app.theme = crate::theme::Theme::Light;
        terminal.draw(|f| render(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("a.rs"), "file must appear in light theme");
    }
}
