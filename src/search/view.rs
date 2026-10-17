use crate::picker::theme::Palette;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn layout(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area)
}

pub fn render_input(
    frame: &mut Frame<'_>,
    input_area: Rect,
    separator_area: Rect,
    palette: Palette,
    query: &str,
    placeholder: &str,
    count: usize,
) {
    let count = format!(" {} results ", count);
    let input_width = usize::from(input_area.width).saturating_sub(count.len());
    let prompt = if query.is_empty() { placeholder } else { query };
    let mut tail: String = prompt.to_owned();
    while tail.width() > input_width.saturating_sub(2) && !tail.is_empty() {
        tail.remove(0);
    }
    let mut header = vec![Span::raw("> "), Span::raw(tail.clone())];
    header.push(Span::raw(
        " ".repeat(input_width.saturating_sub(2 + tail.width())),
    ));
    header.push(Span::styled(
        count,
        palette.base_style().fg(palette.overlay0),
    ));
    frame.render_widget(
        Paragraph::new(Line::from(header)).style(palette.base_style()),
        input_area,
    );
    frame.render_widget(
        Paragraph::new("─".repeat(usize::from(separator_area.width)))
            .style(palette.base_style().fg(palette.overlay0)),
        separator_area,
    );
    let cursor_x = (input_area.x
        + 2
        + if query.is_empty() {
            0
        } else {
            tail.width() as u16
        })
    .min(input_area.right().saturating_sub(1));
    frame.set_cursor_position((cursor_x, input_area.y));
}

/// Truncate by terminal cells, keeping character indices aligned with nucleo.
pub fn label_spans(label: &str, matches: &[usize], width: usize) -> Vec<Span<'static>> {
    let clean: Vec<char> = label
        .chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect();
    let total: usize = clean.iter().map(|c| c.width().unwrap_or(0)).sum();
    let truncated = total > width;
    let budget = width.saturating_sub(usize::from(truncated));
    let mut used = 0;
    let mut spans = Vec::new();
    for (i, ch) in clean.into_iter().enumerate() {
        let cells = ch.width().unwrap_or(0);
        if used + cells > budget {
            break;
        }
        used += cells;
        let style = if matches.contains(&i) {
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default()
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    if truncated && width > 0 {
        spans.push(Span::raw("…"));
    }
    spans
}

pub fn render_actions(
    frame: &mut Frame<'_>,
    area: Rect,
    palette: Palette,
    enabled: bool,
) -> (Rect, Rect) {
    if area.height == 0 || area.width < 8 {
        return (Rect::default(), Rect::default());
    }
    let group_width = if area.width >= 20 { 20 } else { 8 };
    let left = area.x + (area.width - group_width) / 2;
    let (ok, cancel, label) = if area.width >= 20 {
        (
            Rect::new(left, area.y, 6, 1),
            Rect::new(left + 8, area.y, 12, 1),
            " Esc Cancel ",
        )
    } else {
        (Rect::default(), Rect::new(left, area.y, 8, 1), " Cancel ")
    };
    if ok.width > 0 {
        let style = if enabled {
            palette.primary_style()
        } else {
            palette.secondary_style().fg(palette.overlay0)
        };
        frame.render_widget(Paragraph::new(" ↵ OK ").style(style), ok);
    }
    frame.render_widget(
        Paragraph::new(label).style(palette.secondary_style()),
        cancel,
    );
    (ok, cancel)
}

pub fn render_empty(frame: &mut Frame<'_>, area: Rect, palette: Palette, message: &str) {
    frame.render_widget(
        Paragraph::new(message).style(palette.base_style().fg(palette.subtext0)),
        area,
    );
}

pub fn render_loading(frame: &mut Frame<'_>, area: Rect, palette: Palette, message: &str) -> Rect {
    frame.render_widget(Paragraph::new("").style(palette.base_style()), area);
    if area.width < 8 || area.height < 6 {
        frame.render_widget(
            Paragraph::new("Esc closes").style(palette.base_style()),
            area,
        );
        return Rect::default();
    }
    frame.render_widget(
        Paragraph::new(message).style(palette.base_style()),
        Rect::new(area.x, area.y, area.width, area.height - 2),
    );
    render_actions(
        frame,
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
        palette,
        false,
    )
    .1
}

/// Keep selection visible without changing item ordering or navigation policy.
pub fn visible_offset(
    selected: Option<usize>,
    offset: usize,
    count: usize,
    height: usize,
) -> usize {
    let mut offset = offset;
    if let Some(selected) = selected {
        offset = offset.min(selected);
        if height > 0 && selected >= offset + height {
            offset = selected + 1 - height;
        }
    }
    offset.min(count.saturating_sub(height))
}
