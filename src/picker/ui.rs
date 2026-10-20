use std::{
    io::Stdout,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::Duration,
};

use anyhow::Result;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Modifier, Stylize},
    text::{Line, Span},
    widgets::Paragraph,
};
use terminal_input::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use unicode_width::UnicodeWidthStr;

use crate::picker::{
    api::Herdr,
    app::{Action, App, ResultHit},
    config::SearchMode,
    model::{Snapshot, Target},
    search::{Row, RowKind},
    theme::{Palette, Theme},
};

use crate::search::terminal::Screen;

fn wait_for_snapshot(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    receiver: &Receiver<Result<Snapshot>>,
    theme: &mut Theme,
) -> Result<Option<(Snapshot, Vec<Event>)>> {
    let mut pending = Vec::new();
    loop {
        match receiver.try_recv() {
            Ok(result) => return Ok(Some((result?, pending))),
            Err(TryRecvError::Disconnected) => {
                anyhow::bail!("snapshot worker stopped unexpectedly")
            }
            Err(TryRecvError::Empty) => {}
        }
        let mut cancel_area = Rect::default();
        terminal.draw(|frame| {
            cancel_area = render_loading(frame, theme.palette());
        })?;
        if event::poll(Duration::from_millis(20))? {
            let event = event::read()?;
            if let Event::Key(key) = event
                && key.kind != KeyEventKind::Release
                && (key.code == KeyCode::Esc
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)))
            {
                return Ok(None);
            }
            if let Event::Mouse(mouse) = event
                && mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && cancel_area.contains((mouse.column, mouse.row).into())
            {
                return Ok(None);
            }
            if let Event::ThemeModeChanged(appearance) = event {
                theme.set_appearance(appearance);
            }
            if matches!(event, Event::Key(_) | Event::Paste(_)) {
                pending.push(event);
            }
        }
    }
}

pub fn run(api: &Herdr, mode: SearchMode, mut theme: Theme) -> Result<Option<(Snapshot, Target)>> {
    let (_screen, mut terminal) = Screen::enter(theme.auto_switch())?;
    let cancelled = AtomicBool::new(false);
    let loaded = thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        let cancellation = &cancelled;
        scope.spawn(move || {
            let _ = sender.send(api.snapshot_cancellable(cancellation));
        });
        let result = wait_for_snapshot(&mut terminal, &receiver, &mut theme);
        cancelled.store(true, Ordering::Relaxed);
        result
    })?;
    let Some((snapshot, pending)) = loaded else {
        return Ok(None);
    };
    let mut app = App::new(snapshot, mode);
    app.theme = theme;
    for event in pending {
        match app.handle(event) {
            Action::Close => return Ok(None),
            Action::Switch(target) => return Ok(Some((app.snapshot, target))),
            Action::Continue => {}
        }
    }
    loop {
        terminal.draw(|frame| render(frame, &mut app))?;
        match app.handle(event::read()?) {
            Action::Close => return Ok(None),
            Action::Switch(target) => return Ok(Some((app.snapshot, target))),
            Action::Continue => {}
        }
    }
}

pub use crate::search::view::label_spans;

fn kind_color(kind: RowKind, palette: Palette) -> Color {
    match kind {
        RowKind::RootWorkspace => palette.accent,
        RowKind::Worktree => palette.green,
        RowKind::Tab => palette.blue,
    }
}

fn colored_label_spans(
    label: &str,
    matches: &[usize],
    width: usize,
    kind: RowKind,
    palette: Palette,
) -> Vec<Span<'static>> {
    label_spans(label, matches, width)
        .into_iter()
        .map(|span| span.fg(kind_color(kind, palette)))
        .collect()
}

fn row_line(
    row: &Row,
    flat: bool,
    width: usize,
    selected: bool,
    palette: Palette,
) -> Line<'static> {
    let prefix = format!(
        "{}{} ",
        if selected { '›' } else { ' ' },
        if row.active { '●' } else { ' ' }
    );
    let mut spans = vec![Span::raw(prefix)];
    let mut remaining = width.saturating_sub(3);
    if flat {
        for (index, ancestor) in row.ancestors.iter().enumerate() {
            let parts_left = row.ancestors.len() - index + 1;
            let separators = 3 * (parts_left - 1);
            if remaining <= separators {
                break;
            }
            let ancestor_width =
                ((remaining - separators) / parts_left).min(ancestor.label.width());
            let ancestor_spans = colored_label_spans(
                &ancestor.label,
                &ancestor.matches,
                ancestor_width,
                ancestor.kind,
                palette,
            );
            let used: usize = ancestor_spans.iter().map(|span| span.content.width()).sum();
            spans.extend(ancestor_spans);
            spans.push(Span::raw(" / "));
            remaining = remaining.saturating_sub(used + 3);
        }
    } else {
        let indent = (row.depth * 2).min(remaining.saturating_sub(1));
        spans.push(Span::raw(" ".repeat(indent)));
        remaining = remaining.saturating_sub(indent);
    }
    spans.extend(colored_label_spans(
        &row.label,
        &row.label_matches,
        remaining,
        row.kind,
        palette,
    ));
    let style = if selected {
        palette
            .base_style()
            .bg(palette.selection_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        palette.base_style()
    };
    Line::from(spans).style(style)
}

const MAX_AGENT_STATUS_ICONS: usize = 4;

fn agent_indicator_width(visible: usize, hidden: usize) -> usize {
    2 * visible
        + if hidden > 0 {
            format!("+{hidden} ").width()
        } else {
            0
        }
}

fn tab_width(row: &Row) -> usize {
    let count = row.agent_statuses.len();
    let visible = count.min(MAX_AGENT_STATUS_ICONS);
    row.label.width()
        + 2
        + agent_indicator_width(visible, count - visible)
        + 2 * usize::from(row.active)
}

fn tab_window(tabs: &[Row], selected: Option<usize>, width: usize) -> (usize, Vec<(usize, usize)>) {
    let layout = |start: usize| {
        let mut remaining = width.saturating_sub(usize::from(start > 0));
        let mut visible = Vec::new();
        for (index, tab) in tabs.iter().enumerate().skip(start) {
            let reserve = usize::from(index + 1 < tabs.len());
            if remaining <= reserve {
                break;
            }
            let used = tab_width(tab).min(remaining - reserve);
            visible.push((index, used));
            remaining = remaining.saturating_sub(used + 1);
        }
        visible
    };
    let mut start = 0;
    let mut visible = layout(start);
    if let Some(selected) = selected {
        while start < selected
            && !visible.iter().any(|(index, used)| {
                let available = width.saturating_sub(
                    usize::from(start > 0) + usize::from(selected + 1 < tabs.len()),
                );
                *index == selected && *used >= tab_width(&tabs[selected]).min(available)
            })
        {
            start += 1;
            visible = layout(start);
        }
    }
    (start, visible)
}

fn tab_prefix(row: &Row, width: usize, theme: &Theme) -> (Vec<Span<'static>>, usize) {
    let mut spans = Vec::new();
    let count = row.agent_statuses.len();
    let limit = count.min(MAX_AGENT_STATUS_ICONS);
    let preferred_width = agent_indicator_width(limit, count - limit);
    let (visible, hidden, active) = if width > preferred_width {
        (
            limit,
            count - limit,
            row.active && width > preferred_width + 2,
        )
    } else if count > 1 {
        let visible = (0..limit)
            .rev()
            .find(|visible| agent_indicator_width(*visible, count - visible) < width);
        match visible {
            Some(visible) => (visible, count - visible, false),
            None => {
                let summary = format!("+{count}");
                if summary.width() <= width {
                    spans.push(Span::styled(
                        summary,
                        ratatui::style::Style::default().fg(theme.palette().overlay1),
                    ));
                    return (spans, 0);
                }
                return (spans, width);
            }
        }
    } else {
        (0, 0, false)
    };
    if active {
        spans.push(Span::raw("● "));
    }
    for status in row.agent_statuses.iter().take(visible) {
        let (icon, color) = theme.status_indicator(*status);
        spans.push(Span::styled(
            format!("{icon} "),
            ratatui::style::Style::default().fg(color),
        ));
    }
    if hidden > 0 {
        spans.push(Span::styled(
            format!("+{hidden} "),
            ratatui::style::Style::default().fg(theme.palette().overlay1),
        ));
    }
    let used: usize = spans.iter().map(|span| span.content.width()).sum();
    (spans, width - used)
}

fn tab_line(row: &Row, width: usize, selected: bool, theme: &Theme) -> Line<'static> {
    let palette = theme.palette();
    let (_, color) = theme.status_indicator(row.agent_status);
    let mut spans = Vec::new();
    let label_width = if width < 3 {
        width
    } else {
        spans.push(Span::raw("["));
        let (prefix, label_width) = tab_prefix(row, width - 2, theme);
        spans.extend(prefix);
        label_width
    };
    spans.extend(
        label_spans(&row.label, &row.label_matches, label_width)
            .into_iter()
            .map(|span| span.fg(color)),
    );
    if width >= 3 {
        spans.push(Span::raw("]"));
    }
    let style = if selected {
        palette
            .base_style()
            .bg(palette.selection_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        palette.base_style()
    };
    Line::from(spans).style(style)
}

fn render_result(frame: &mut Frame<'_>, app: &App, index: usize, area: Rect) -> Vec<ResultHit> {
    let result = &app.results[index];
    let mut hits = Vec::new();
    let palette = app.theme.palette();
    let selected = app.selected == Some(index);
    let workspace_width = if result.tabs.is_empty() {
        area.width
    } else {
        let content = area.width.saturating_sub(2);
        if area.width < 24 {
            content / 2
        } else {
            content * 2 / 5
        }
    };
    let workspace_area = Rect::new(area.x, area.y, workspace_width, 1);
    let line = row_line(
        &result.workspace,
        app.mode == SearchMode::Flat,
        usize::from(workspace_width),
        selected && app.selected_tab.is_none(),
        palette,
    );
    let hit_width = line.width().min(usize::from(workspace_width)) as u16;
    frame.render_widget(Paragraph::new(line), workspace_area);
    if hit_width > 0 {
        hits.push(ResultHit {
            area: Rect::new(area.x, area.y, hit_width, 1),
            workspace: index,
            tab: None,
        });
    }
    if result.tabs.is_empty() {
        return hits;
    }
    let tab_x = area.x + workspace_width + 2;
    let tab_area = Rect::new(tab_x, area.y, area.right().saturating_sub(tab_x), 1);
    let selected_tab = selected.then(|| app.highlighted_tab()).flatten();
    let (start, visible) = tab_window(&result.tabs, selected_tab, usize::from(tab_area.width));
    let mut x = tab_area.x;
    if start > 0 && tab_area.width > 0 {
        frame.render_widget(
            Paragraph::new("◀").style(palette.base_style().fg(palette.overlay1)),
            Rect::new(x, area.y, 1, 1),
        );
        x += 1;
    }
    for &(tab, width) in &visible {
        let hit = Rect::new(x, area.y, width as u16, 1);
        frame.render_widget(
            Paragraph::new(tab_line(
                &result.tabs[tab],
                width,
                selected_tab == Some(tab),
                &app.theme,
            )),
            hit,
        );
        hits.push(ResultHit {
            area: hit,
            workspace: index,
            tab: Some(tab),
        });
        x += width as u16 + 1;
    }
    if visible
        .last()
        .is_some_and(|(index, _)| index + 1 < result.tabs.len())
        && tab_area.width > 0
    {
        frame.render_widget(
            Paragraph::new("▶").style(palette.base_style().fg(palette.overlay1)),
            Rect::new(tab_area.right() - 1, area.y, 1, 1),
        );
    }
    hits
}

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    let palette = app.theme.palette();
    frame.render_widget(Paragraph::new("").style(palette.base_style()), area);
    app.ok_area = Rect::default();
    app.cancel_area = Rect::default();
    app.result_hits.clear();
    if area.width < 8 || area.height < 6 {
        app.results_area = Rect::default();
        frame.render_widget(
            Paragraph::new("Esc closes").style(palette.base_style()),
            area,
        );
        return;
    }
    let chunks = crate::search::view::layout(area);
    crate::search::view::render_input(
        frame,
        chunks[0],
        chunks[1],
        palette,
        app.input(),
        "Search workspaces, worktrees and tabs",
        app.results.len(),
    );
    app.results_area = chunks[2];
    app.ensure_visible(usize::from(chunks[2].height));
    if app.results.is_empty() {
        crate::search::view::render_empty(frame, chunks[2], palette, "No matching results");
    } else {
        for (y, index) in (app.offset..app.results.len())
            .take(usize::from(chunks[2].height))
            .enumerate()
        {
            let row_area = Rect::new(chunks[2].x, chunks[2].y + y as u16, chunks[2].width, 1);
            let hits = render_result(frame, app, index, row_area);
            app.result_hits.extend(hits);
        }
    }
    let position = app.selected.map(|i| i + 1).unwrap_or(0);
    let help = if area.width >= 65 {
        format!(
            " ↑↓/Ctrl+P,N Workspaces  ←→ Cursor  Tab/Shift+Tab Tabs  {position}/{}",
            app.results.len()
        )
    } else if area.width >= 40 {
        format!(
            " ↑↓ Workspaces  Tab/Shift+Tab Tabs  {position}/{}",
            app.results.len()
        )
    } else {
        " ↑↓ Workspaces Tab Tabs".to_owned()
    };
    frame.render_widget(
        Paragraph::new(help).style(palette.base_style().fg(palette.overlay1)),
        chunks[3],
    );
    (app.ok_area, app.cancel_area) =
        render_actions(frame, chunks[5], palette, app.selected.is_some());
}

use crate::search::view::render_actions;

pub fn render_loading(frame: &mut Frame<'_>, palette: Palette) -> Rect {
    crate::search::view::render_loading(
        frame,
        frame.area(),
        palette,
        "Loading workspaces, worktrees and tabs…",
    )
}
