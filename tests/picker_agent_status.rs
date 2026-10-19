use herdrpp::picker::{
    api::parse_snapshot,
    app::{Action, App},
    config::SearchMode,
    model::{AgentStatus, Target},
    theme::Theme,
    ui::render,
};
use ratatui::{Terminal, backend::TestBackend, style::Modifier};
use serde_json::json;
use terminal_input::event::{
    Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind, ThemeMode,
};

fn app(status: &str, setting: &str, mode: SearchMode) -> App {
    let snapshot = parse_snapshot(&serde_json::to_vec(&json!({"result":{"snapshot":{
        "workspaces":[{"workspace_id":"w1","number":1,"label":"Project","active_tab_id":"w1:t1"}],
        "tabs":[{"workspace_id":"w1","tab_id":"w1:t1","number":1,"label":"Codex","agent_status":status}],
        "focused_workspace_id":null,"focused_tab_id":null
    }}})).unwrap()).unwrap();
    let mut app = App::new(snapshot, mode);
    app.theme = Theme::parse(setting).unwrap();
    app.set_query("codex".into());
    app
}

fn draw(app: &mut App, width: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn renders_herdr_status_icons_and_colors_in_both_modes() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        for (setting, glyphs) in [
            ("", ["●", "●", "●", "○", "·"]),
            (
                "[ui]\nstatus_indicators = 'symbols'",
                ["◐", "×", "✓", "○", "·"],
            ),
        ] {
            for ((status, glyph), color) in ["working", "blocked", "done", "idle", "unknown"]
                .into_iter()
                .zip(glyphs)
                .zip([
                    Theme::default().palette().yellow,
                    Theme::default().palette().red,
                    Theme::default().palette().teal,
                    Theme::default().palette().green,
                    Theme::default().palette().overlay0,
                ])
            {
                let mut app = app(status, setting, mode);
                let buffer = draw(&mut app, 80);
                let hit = app
                    .result_hits
                    .iter()
                    .find(|hit| hit.tab == Some(0))
                    .unwrap()
                    .area;
                let text: String = (hit.x..hit.right())
                    .map(|x| buffer[(x, hit.y)].symbol())
                    .collect();
                assert_eq!(text, format!("[{glyph} Codex]"));
                assert_eq!(buffer[(hit.x + 1, hit.y)].fg, color);
                assert_eq!(buffer[(hit.x + 3, hit.y)].fg, color);
                assert_eq!(
                    buffer[(hit.x + 1, hit.y)].bg,
                    app.theme.palette().selection_bg
                );
                assert!(
                    buffer[(hit.x + 3, hit.y)]
                        .modifier
                        .contains(Modifier::UNDERLINED | Modifier::BOLD)
                );
            }
        }
    }
}

#[test]
fn missing_and_future_statuses_are_unknown() {
    let future = app("waiting_for_network", "", SearchMode::Flat);
    assert_eq!(future.snapshot.tabs[0].agent_status, AgentStatus::Unknown);
    let snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    assert_eq!(snapshot.tabs[2].agent_status, AgentStatus::Unknown);
}

#[test]
fn active_marker_is_separate_from_agent_status() {
    let mut app = app(
        "working",
        "[ui]\nstatus_indicators = 'symbols'",
        SearchMode::Flat,
    );
    app.snapshot.focused_workspace_id = Some("w1".into());
    app.snapshot.focused_tab_id = Some("w1:t1".into());
    app.set_query("codex".into());
    let buffer = draw(&mut app, 80);
    let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(text.contains("[● ◐ Codex]"));
}

#[test]
fn status_icon_click_focuses_the_tab() {
    let mut app = app("blocked", "", SearchMode::Flat);
    draw(&mut app, 80);
    let hit = app
        .result_hits
        .iter()
        .find(|hit| hit.tab == Some(0))
        .unwrap()
        .area;
    assert_eq!(
        app.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x + 1,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        })),
        Action::Switch(Target::Tab("w1:t1".into()))
    );
}

#[test]
fn status_uses_custom_colors_and_terminal_appearance() {
    let mut app = app(
        "done",
        "[theme]\nauto_switch = true\n[theme.custom]\nteal = '#123456'\n[theme.custom.light]\nteal = '#abcdef'",
        SearchMode::Flat,
    );
    let dark = draw(&mut app, 80);
    let hit = app
        .result_hits
        .iter()
        .find(|hit| hit.tab == Some(0))
        .unwrap()
        .area;
    assert_eq!(
        dark[(hit.x + 1, hit.y)].fg,
        ratatui::style::Color::Rgb(18, 52, 86)
    );
    app.handle(Event::ThemeModeChanged(ThemeMode::Light));
    let light = draw(&mut app, 80);
    assert_eq!(
        light[(hit.x + 3, hit.y)].fg,
        ratatui::style::Color::Rgb(171, 205, 239)
    );
    assert_eq!(app.query, "codex");
}

#[test]
fn invalid_indicator_setting_preserves_theme_and_reports_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[theme]\nname = 'dracula'\n[ui]\nstatus_indicators = 'animated'",
    )
    .unwrap();
    let (theme, diagnostics) = Theme::load_path(&path);
    assert_eq!(theme.palette(), herdrpp::picker::theme::Palette::dracula());
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("ui.status_indicators"))
    );
    assert_eq!(theme.status_indicator(AgentStatus::Working).0, "●");
}

#[test]
fn remembered_tab_retains_status_without_affecting_search() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = app("done", "", mode);
        let mut other = app.snapshot.tabs[0].clone();
        other.tab_id = "w1:t2".into();
        other.number = 2;
        other.label = "Server".into();
        other.agent_status = AgentStatus::Working;
        app.snapshot.tabs.push(other);
        app.set_query("server".into());
        assert_eq!(app.results[0].tabs.len(), 2);
        assert_eq!(app.results[0].tabs[0].label, "Server");
        assert_eq!(app.results[0].tabs[1].agent_status, AgentStatus::Done);
        assert!(!app.results[0].tabs[1].matches_query);
        app.set_query("done".into());
        assert!(app.results.is_empty());
    }
}

#[test]
fn reopening_uses_new_status_without_refreshing_existing_picker() {
    let mut original = app("working", "", SearchMode::Flat);
    let mut reopened = app("done", "", SearchMode::Flat);
    let old = draw(&mut original, 80);
    let new = draw(&mut reopened, 80);
    let hit = original
        .result_hits
        .iter()
        .find(|hit| hit.tab == Some(0))
        .unwrap()
        .area;
    assert_eq!(old[(hit.x + 3, hit.y)].fg, original.theme.palette().yellow);
    assert_eq!(new[(hit.x + 3, hit.y)].fg, reopened.theme.palette().teal);
}

#[test]
fn narrow_tabs_drop_active_marker_before_status_and_keep_click_bounds() {
    for (width, expected) in [(10, "[C…]"), (12, "[· …]"), (14, "[· C…]"), (16, "[● · …]")]
    {
        let mut app = app("unknown", "", SearchMode::Flat);
        app.snapshot.focused_workspace_id = Some("w1".into());
        app.snapshot.focused_tab_id = Some("w1:t1".into());
        app.set_query("codex".into());
        let buffer = draw(&mut app, width);
        let hit = app
            .result_hits
            .iter()
            .find(|hit| hit.tab == Some(0))
            .unwrap()
            .area;
        let text: String = (hit.x..hit.right())
            .map(|x| buffer[(x, hit.y)].symbol())
            .collect();
        assert_eq!(text, expected);
        assert!(hit.right() <= width);
    }
}
