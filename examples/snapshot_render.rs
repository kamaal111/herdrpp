//! Export otherwise unreachable UI states for the visual snapshot harness.
use anyhow::{Context, Result, bail};
use herdrpp::picker::{app::App, config::SearchMode, model::Snapshot, theme::Theme, ui};
use ratatui::{Terminal, backend::TestBackend, style::Modifier};
use serde_json::json;

fn main() -> Result<()> {
    let case = std::env::args().nth(1).context("expected snapshot case")?;
    let (width, height) = match case.as_str() {
        "empty-snapshot" | "loading" => (109, 20),
        "tiny-terminal" => (6, 1),
        _ => bail!("unknown controlled snapshot: {case}"),
    };
    let mut app = App::new(
        Snapshot {
            workspaces: vec![],
            tabs: vec![],
            focused_workspace_id: None,
            focused_tab_id: None,
        },
        SearchMode::Hierarchical,
    );
    app.theme = Theme::parse("[theme]\nname = 'dracula'\n")?;
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    terminal.draw(|frame| {
        if case == "loading" {
            ui::render_loading(frame, app.theme.palette());
        } else {
            ui::render(frame, &mut app);
        }
    })?;
    let cells: Vec<_> = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| {
            json!({
                "text": cell.symbol(),
                "fg": format!("{:?}", cell.fg),
                "bg": format!("{:?}", cell.bg),
                "bold": cell.modifier.contains(Modifier::BOLD),
                "underline": cell.modifier.contains(Modifier::UNDERLINED),
                "reverse": cell.modifier.contains(Modifier::REVERSED),
            })
        })
        .collect();
    println!(
        "{}",
        json!({"width": width, "height": height, "cells": cells})
    );
    Ok(())
}
