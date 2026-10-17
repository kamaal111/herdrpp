use super::{
    app::{Action, App, Mode, Selection},
    cli::{Catalog, read_json},
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
    let chunks = view::layout(area);
    match &app.mode {
        Mode::Search => {
            view::render_input(
                frame,
                chunks[0],
                chunks[1],
                palette,
                &app.query,
                "Search commands",
                app.results.len(),
            );
            app.results_area = chunks[2];
            app.ensure_visible(usize::from(chunks[2].height));
            if app.results.is_empty() {
                view::render_empty(frame, chunks[2], palette, "No matching commands");
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
                let (detail, detail_matches) = match &item.command {
                    super::catalog::Command::Plugin { .. } => (&item.id, &hit.matched.indices[1]),
                    _ => (&item.source, &hit.matched.indices[0]),
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
                Paragraph::new("↑↓/Ctrl+P,N Commands  Enter Run")
                    .style(palette.base_style().fg(palette.overlay1)),
                chunks[3],
            );
            let details = app
                .selected
                .and_then(|index| app.results.get(index))
                .map(|hit| {
                    let item = &app.items[hit.item];
                    format!("Command Palette · {} {}", item.source, item.description)
                })
                .unwrap_or_else(|| "Command Palette".into());
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
        Mode::Rename { item, input, error } => {
            view::render_input(
                frame,
                chunks[0],
                chunks[1],
                palette,
                input,
                "Enter a name",
                1,
            );
            let message = if *error {
                "Enter a nonblank name".to_string()
            } else {
                app.items[*item].title.clone()
            };
            frame.render_widget(
                Paragraph::new(message).style(palette.base_style()),
                chunks[2],
            );
            frame.render_widget(
                Paragraph::new("Enter Save  Esc Back").style(palette.base_style()),
                chunks[3],
            );
        }
        Mode::Confirm { item } => {
            let command = &app.items[*item];
            let target = match command.command {
                super::catalog::Command::Close(scope) => scope.label(&app.context),
                _ => "",
            };
            let text = format!("{}: {}?", command.title, target);
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
                Paragraph::new("Enter confirms closing; Esc goes back").style(palette.base_style()),
                chunks[2],
            );
        }
    }
    (app.ok_area, app.cancel_area) = view::render_actions(
        frame,
        chunks[5],
        palette,
        app.selected.is_some() || app.mode != Mode::Search,
    );
}

pub fn render_loading(frame: &mut Frame<'_>, palette: Palette) -> Rect {
    frame.render_widget(Paragraph::new("").style(palette.base_style()), frame.area());
    view::render_loading(frame, frame.area(), palette, "Loading commands…")
}

pub fn run(directory: &Path, mut theme: Theme) -> Result<Option<Selection>> {
    let (_screen, mut terminal) = Screen::enter(theme.auto_switch())?;
    let mut pending = Vec::new();
    let catalog: Catalog = loop {
        anyhow::ensure!(directory.exists(), "command palette owner exited");
        if directory.join("catalog.json").exists() {
            break read_json(&directory.join("catalog.json"))?;
        }
        let mut cancel_area = Rect::default();
        terminal.draw(|frame| {
            cancel_area = render_loading(frame, theme.palette());
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
    let mut app = App::new(catalog.context, catalog.items, theme, catalog.warning);
    for event in pending {
        match app.handle(event) {
            Action::Cancel => return Ok(None),
            Action::Execute(selection) => return Ok(Some(selection)),
            Action::Continue => {}
        }
    }
    loop {
        terminal.draw(|frame| render(frame, &mut app))?;
        match app.handle(event::read()?) {
            Action::Cancel => return Ok(None),
            Action::Execute(selection) => return Ok(Some(selection)),
            Action::Continue => {}
        }
    }
}
