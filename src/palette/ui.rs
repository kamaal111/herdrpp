use super::{
    app::{Action, App, Mode, Selection},
    cli::{Catalog, read_json},
    registry::{self, Detail, Screen as ScreenSpec, Step},
};
use crate::{
    picker::theme::{Palette, Theme},
    search::{terminal::Screen, view},
};
use anyhow::Result;
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
};
use std::{path::Path, time::Duration};
use terminal_input::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};

pub fn diagnostic_lines(message: &str, width: u16) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    let mut lines = Vec::new();
    for line in message.split('\n') {
        let mut current = String::new();
        let mut used = 0;
        for ch in line.chars() {
            let size = ch.width().unwrap_or(0);
            if used + size > usize::from(width.max(1)) && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                used = 0;
            }
            current.push(ch);
            used += size;
        }
        lines.push(current);
    }
    lines
}

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let palette = app.theme.palette();
    frame.render_widget(Paragraph::new("").style(palette.base_style()), frame.area());
    let area = frame.area();
    app.clear_hits();
    if area.width < 8 || area.height < 6 {
        frame.render_widget(
            Paragraph::new("Esc closes").style(palette.base_style()),
            frame.area(),
        );
        return;
    }
    if app.screen.diagnostics
        && let Some(message) = &app.warning
    {
        let body = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
        let wrapped = diagnostic_lines(message, body.width);
        let lines = wrapped.len().saturating_sub(usize::from(body.height));
        let paragraph = Paragraph::new(wrapped.join("\n")).style(palette.base_style());
        app.diagnostics_max = u16::try_from(lines).unwrap_or(u16::MAX);
        app.diagnostics_offset = app.diagnostics_offset.min(app.diagnostics_max);
        frame.render_widget(paragraph.scroll((app.diagnostics_offset, 0)), body);
        frame.render_widget(
            Paragraph::new("↑↓/PgUp/PgDn Scroll · Esc closes").style(palette.base_style()),
            Rect::new(area.x, area.y + area.height - 1, area.width, 1),
        );
        return;
    }
    let chunks = view::layout(area);
    match &app.mode {
        Mode::Search => {
            view::render_input(
                frame,
                chunks[0],
                chunks[1],
                palette,
                app.input(),
                &app.screen.search_label,
                app.results.len(),
            );
            app.results_area = chunks[2];
            app.ensure_visible(usize::from(chunks[2].height));
            if app.results.is_empty() {
                view::render_empty(frame, chunks[2], palette, &app.screen.empty);
            }
            for (y, index) in (app.offset..app.results.len())
                .take(usize::from(chunks[2].height))
                .enumerate()
            {
                let hit = &app.results[index];
                let item = &app.items[hit.item];
                let width = usize::from(chunks[2].width).saturating_sub(2);
                let mut spans = vec![Span::raw(if app.selected == Some(index) {
                    "› "
                } else {
                    "  "
                })];
                let (detail, detail_matches) = match item.behavior.detail {
                    Detail::Id => (&item.id, &hit.matched.indices[1]),
                    Detail::Source => (&item.source, &hit.matched.indices[0]),
                };
                let detail_width =
                    unicode_width::UnicodeWidthStr::width(detail.as_str()).min(width / 3);
                let title_width = width.saturating_sub(detail_width + 1);
                let title = view::label_spans(&item.title, &hit.matched.indices[3], title_width);
                let used: usize = title.iter().map(|span| span.width()).sum();
                spans.extend(title);
                spans.push(Span::raw(" ".repeat(title_width.saturating_sub(used) + 1)));
                spans.extend(view::label_spans(
                    detail,
                    detail_matches,
                    width.saturating_sub(title_width + 1),
                ));
                let style = if app.selected == Some(index) {
                    palette
                        .base_style()
                        .bg(palette.selection_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    palette.base_style()
                };
                let row = Rect::new(chunks[2].x, chunks[2].y + y as u16, chunks[2].width, 1);
                frame.render_widget(Paragraph::new(Line::from(spans)).style(style), row);
                app.hits.push((row, index));
            }
            frame.render_widget(
                Paragraph::new(app.screen.help.as_str())
                    .style(palette.base_style().fg(palette.overlay1)),
                chunks[3],
            );
            let details = app
                .selected
                .and_then(|index| app.results.get(index))
                .map(|hit| {
                    let item = &app.items[hit.item];
                    format!(
                        "{} · {} {}",
                        app.screen.title, item.source, item.description
                    )
                })
                .unwrap_or_else(|| app.screen.title.clone());
            let message = app.warning.as_deref().unwrap_or(&details);
            frame.render_widget(
                Paragraph::new(Line::from(view::label_spans(
                    message,
                    &[],
                    usize::from(chunks[4].width),
                )))
                .style(palette.base_style().fg(palette.overlay0)),
                chunks[4],
            );
        }
        Mode::Name { input, error, .. } => {
            view::render_name_input(frame, chunks[0], chunks[1], palette, input);
            if let Some((
                item,
                Step::Text {
                    message,
                    help,
                    error: fallback,
                    ..
                },
            )) = app.active_step()
            {
                let message = if *error {
                    app.prompt_error.clone().unwrap_or_else(|| fallback.clone())
                } else {
                    registry::format_text(message, item, &app.context)
                };
                frame.render_widget(
                    Paragraph::new(message).style(palette.base_style()),
                    chunks[2],
                );
                frame.render_widget(
                    Paragraph::new(help.as_str()).style(palette.base_style()),
                    chunks[3],
                );
            }
        }
        Mode::Confirm { .. } => {
            if let Some((item, Step::Confirm { message, help })) = app.active_step() {
                let text = registry::format_text(message, item, &app.context);
                frame.render_widget(
                    Paragraph::new(Line::from(view::label_spans(
                        &text,
                        &[],
                        usize::from(chunks[0].width),
                    )))
                    .style(palette.base_style()),
                    chunks[0],
                );
                frame.render_widget(
                    Paragraph::new(help.as_str()).style(palette.base_style()),
                    chunks[2],
                );
            }
        }
    }
    (app.ok_area, app.cancel_area) = view::render_actions(
        frame,
        chunks[5],
        palette,
        app.selected.is_some() || app.mode != Mode::Search,
    );
}

pub fn render_loading(frame: &mut Frame<'_>, palette: Palette, screen: &ScreenSpec) -> Rect {
    frame.render_widget(Paragraph::new("").style(palette.base_style()), frame.area());
    view::render_loading(frame, frame.area(), palette, &screen.loading)
}

pub fn run(directory: &Path, theme: Theme) -> Result<Option<Selection>> {
    run_mode(directory, theme, "commands")
}

pub fn run_mode(directory: &Path, mut theme: Theme, screen_key: &str) -> Result<Option<Selection>> {
    let (_screen, mut terminal) = Screen::enter(theme.auto_switch())?;
    let screen: ScreenSpec = if directory.join("screen.json").exists() {
        read_json(&directory.join("screen.json"))?
    } else {
        registry::Registry::bundled()?.screen(screen_key)?
    };
    let mut pending = Vec::new();
    let catalog: Catalog = loop {
        anyhow::ensure!(directory.exists(), "command palette owner exited");
        if directory.join("catalog.json").exists() {
            break read_json(&directory.join("catalog.json"))?;
        }
        let mut cancel_area = Rect::default();
        terminal.draw(|frame| {
            cancel_area = render_loading(frame, theme.palette(), &screen);
        })?;
        if event::poll(Duration::from_millis(20))? {
            let input = event::read()?;
            match input {
                Event::Key(key)
                    if key.kind != KeyEventKind::Release
                        && (key.code == KeyCode::Esc
                            || (key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL))) =>
                {
                    if key.code == KeyCode::Char('c') {
                        super::cli::write_json(&directory.join("abort.json"), &true)?;
                    }
                    return Ok(None);
                }
                Event::Mouse(mouse)
                    if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                        && cancel_area.contains((mouse.column, mouse.row).into()) =>
                {
                    return Ok(None);
                }
                Event::ThemeModeChanged(appearance) => theme.set_appearance(appearance),
                Event::Key(_) | Event::Paste(_) if pending.len() < 1024 => pending.push(input),
                _ => {}
            }
        }
    };
    let mut app = App::new(
        catalog.context,
        catalog.items,
        catalog.naming,
        theme,
        catalog.warning,
        catalog.screen,
    );
    app.set_recent_ids(catalog.recent_ids);
    for event in pending {
        match app.handle(event) {
            Action::Cancel => return Ok(None),
            Action::Abort => {
                super::cli::write_json(&directory.join("abort.json"), &true)?;
                return Ok(None);
            }
            Action::Execute(selection) => return Ok(Some(selection)),
            Action::Continue => {}
        }
    }
    loop {
        terminal.draw(|frame| render(frame, &mut app))?;
        match app.handle(event::read()?) {
            Action::Cancel => return Ok(None),
            Action::Abort => {
                super::cli::write_json(&directory.join("abort.json"), &true)?;
                return Ok(None);
            }
            Action::Execute(selection) => return Ok(Some(selection)),
            Action::Continue => {}
        }
    }
}
