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
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind, ThemeMode,
};

fn app(status: &str, setting: &str, mode: SearchMode) -> App {
    app_with_agents(status, setting, mode, json!([]))
}

fn app_with_agents(
    status: &str,
    setting: &str,
    mode: SearchMode,
    agents: serde_json::Value,
) -> App {
    let snapshot = parse_snapshot(&serde_json::to_vec(&json!({"result":{"snapshot":{
        "workspaces":[{"workspace_id":"w1","number":1,"label":"Project","active_tab_id":"w1:t1"}],
        "tabs":[{"workspace_id":"w1","tab_id":"w1:t1","number":1,"label":"Codex","agent_status":status}],
        "focused_workspace_id":null,"focused_tab_id":null,"agents":agents
    }}})).unwrap()).unwrap();
    let mut app = App::new(snapshot, mode);
    app.theme = Theme::parse(setting).unwrap();
    app.set_query("codex".into());
    app
}

fn agents(statuses: &[&str]) -> serde_json::Value {
    statuses
        .iter()
        .map(|status| {
            json!({
                "workspace_id":"w1", "tab_id":"w1:t1", "agent_status":status
            })
        })
        .collect()
}

fn tab_text(app: &App, buffer: &ratatui::buffer::Buffer) -> String {
    let hit = app
        .result_hits
        .iter()
        .find(|hit| hit.tab == Some(0))
        .unwrap()
        .area;
    (hit.x..hit.right())
        .map(|x| buffer[(x, hit.y)].symbol())
        .collect()
}

#[test]
fn renders_first_four_agent_statuses_in_snapshot_order_with_overflow() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        for (setting, expected) in [
            ("", "[● ● ● ● +2 Codex]"),
            ("[ui]\nstatus_indicators = 'symbols'", "[◐ × ✓ ◐ +2 Codex]"),
        ] {
            let mut app = app_with_agents(
                "blocked",
                setting,
                mode,
                agents(&["working", "blocked", "done", "working", "idle", "future"]),
            );
            let buffer = draw(&mut app, 80);
            assert_eq!(tab_text(&app, &buffer), expected);
            let hit = app
                .result_hits
                .iter()
                .find(|hit| hit.tab == Some(0))
                .unwrap()
                .area;
            let palette = app.theme.palette();
            for (offset, color) in [
                (1, palette.yellow),
                (3, palette.red),
                (5, palette.teal),
                (7, palette.yellow),
                (9, palette.overlay1),
                (12, palette.red),
            ] {
                assert_eq!(buffer[(hit.x + offset, hit.y)].fg, color);
                assert_eq!(buffer[(hit.x + offset, hit.y)].bg, palette.selection_bg);
                assert!(
                    buffer[(hit.x + offset, hit.y)]
                        .modifier
                        .contains(Modifier::BOLD)
                );
            }
            assert!(
                buffer[(hit.x + 12, hit.y)]
                    .modifier
                    .contains(Modifier::UNDERLINED)
            );
        }
    }
}

#[test]
fn agent_status_icon_limit_is_four_and_counts_every_remaining_agent() {
    for (count, expected) in [
        (1, "[◐ Codex]"),
        (4, "[◐ ◐ ◐ ◐ Codex]"),
        (5, "[◐ ◐ ◐ ◐ +1 Codex]"),
        (15, "[◐ ◐ ◐ ◐ +11 Codex]"),
    ] {
        let mut app = app_with_agents(
            "working",
            "[ui]\nstatus_indicators = 'symbols'",
            SearchMode::Flat,
            agents(&vec!["working"; count]),
        );
        let buffer = draw(&mut app, 80);
        assert_eq!(tab_text(&app, &buffer), expected);
    }
}

#[test]
fn agent_statuses_are_scoped_to_workspace_and_tab() {
    let mut app = app_with_agents(
        "done",
        "[ui]\nstatus_indicators = 'symbols'",
        SearchMode::Flat,
        json!([
            {"workspace_id":"w2","tab_id":"w1:t1","agent_status":"blocked"},
            {"workspace_id":"w1","tab_id":"w1:t2","agent_status":"working"},
            {"workspace_id":"w1","tab_id":"w1:t1","agent_status":"idle"}
        ]),
    );
    let buffer = draw(&mut app, 80);
    assert_eq!(tab_text(&app, &buffer), "[○ Codex]");
}

#[test]
fn missing_and_future_individual_statuses_are_unknown() {
    let mut app = app_with_agents(
        "done",
        "",
        SearchMode::Flat,
        json!([
            {"workspace_id":"w1","tab_id":"w1:t1"},
            {"workspace_id":"w1","tab_id":"w1:t1","agent_status":"future"}
        ]),
    );
    let buffer = draw(&mut app, 80);
    assert_eq!(tab_text(&app, &buffer), "[· · Codex]");
}

#[test]
fn multiple_agents_truncate_with_exact_hidden_count() {
    for (width, expected) in [
        (20, "[◐ × ✓ …]"),
        (18, "[◐ +2 …]"),
        (16, "[+3 C…]"),
        (12, "[+3]"),
        (10, "[+3]"),
        (8, "[…]"),
    ] {
        let mut app = app_with_agents(
            "blocked",
            "[ui]\nstatus_indicators = 'symbols'",
            SearchMode::Flat,
            agents(&["working", "blocked", "done"]),
        );
        app.snapshot.focused_workspace_id = Some("w1".into());
        app.snapshot.focused_tab_id = Some("w1:t1".into());
        app.set_query("codex".into());
        let buffer = draw(&mut app, width);
        assert_eq!(
            tab_text(&app, &buffer).trim_end(),
            expected,
            "width {width}"
        );
        assert!(app.result_hits.iter().all(|hit| hit.area.right() <= width));
    }
}

#[test]
fn agent_icons_and_overflow_count_focus_the_tab() {
    for (width, offsets) in [(80, vec![1, 3, 5]), (18, vec![1, 3, 4])] {
        for offset in offsets {
            let mut app = app_with_agents(
                "blocked",
                "",
                SearchMode::Flat,
                agents(&["working", "blocked", "done"]),
            );
            draw(&mut app, width);
            let hit = app
                .result_hits
                .iter()
                .find(|hit| hit.tab == Some(0))
                .unwrap()
                .area;
            assert_eq!(
                app.handle(Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: hit.x + offset,
                    row: hit.y,
                    modifiers: KeyModifiers::NONE,
                })),
                Action::Switch(Target::Tab("w1:t1".into()))
            );
        }
    }
}

#[test]
fn multi_digit_overflow_count_and_unicode_labels_stay_within_bounds() {
    let mut app = app_with_agents(
        "working",
        "[ui]\nstatus_indicators = 'symbols'",
        SearchMode::Flat,
        agents(&["working"; 15]),
    );
    app.snapshot.tabs[0].label = "開発🦀".into();
    app.set_query("".into());
    let buffer = draw(&mut app, 20);
    assert_eq!(tab_text(&app, &buffer), "[◐ +14 …]");
    let buffer = draw(&mut app, 80);
    let text = tab_text(&app, &buffer);
    assert!(text.contains('開'));
    assert!(text.contains('発'));
    assert!(text.contains('🦀'));
    for width in 8..100 {
        draw(&mut app, width);
        assert!(app.result_hits.iter().all(|hit| hit.area.right() <= width));
    }
}

#[test]
fn overflow_counts_inherit_the_selected_tab_background() {
    for (width, offset) in [(18, 3), (10, 1)] {
        let mut app = app_with_agents(
            "blocked",
            "",
            SearchMode::Flat,
            agents(&["working", "blocked", "done"]),
        );
        let buffer = draw(&mut app, width);
        let hit = app
            .result_hits
            .iter()
            .find(|hit| hit.tab == Some(0))
            .unwrap()
            .area;
        let count = &buffer[(hit.x + offset, hit.y)];
        assert_eq!(count.symbol(), "+");
        assert_eq!(count.fg, app.theme.palette().overlay1);
        assert_eq!(count.bg, app.theme.palette().selection_bg);
        assert!(count.modifier.contains(Modifier::BOLD));
    }
}

#[test]
fn active_marker_is_preserved_when_all_individual_icons_fit() {
    let mut app = app_with_agents(
        "blocked",
        "[ui]\nstatus_indicators = 'symbols'",
        SearchMode::Flat,
        agents(&["working", "blocked", "done"]),
    );
    app.snapshot.focused_workspace_id = Some("w1".into());
    app.snapshot.focused_tab_id = Some("w1:t1".into());
    app.set_query("codex".into());
    let buffer = draw(&mut app, 80);
    assert_eq!(tab_text(&app, &buffer), "[● ◐ × ✓ Codex]");
}

#[test]
fn multi_agent_tabs_remain_reachable_when_cycling_and_resizing() {
    let mut app = app_with_agents("working", "", SearchMode::Flat, agents(&["working"; 40]));
    let mut other = app.snapshot.tabs[0].clone();
    other.tab_id = "w1:t2".into();
    other.number = 2;
    other.label = "Server".into();
    app.snapshot.tabs.push(other);
    app.set_query("".into());
    for width in [80, 18, 40] {
        app.handle(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        draw(&mut app, width);
        assert_eq!(app.highlighted_tab(), Some(1));
        assert!(app.result_hits.iter().any(|hit| hit.tab == Some(1)));
        assert_eq!(
            app.handle(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            ))),
            Action::Switch(Target::Tab("w1:t2".into()))
        );
        app.handle(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        let buffer = draw(&mut app, width);
        assert_eq!(app.highlighted_tab(), Some(0));
        assert!(tab_text(&app, &buffer).contains('+'));
        assert!(app.result_hits.iter().all(|hit| hit.area.right() <= width));
    }
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
    assert_eq!(app.query(), "codex");
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
        let mut app = app_with_agents("done", "", mode, agents(&["done", "idle"]));
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
        assert_eq!(
            app.results[0].tabs[1].agent_statuses,
            vec![AgentStatus::Done, AgentStatus::Idle]
        );
        assert_eq!(
            app.results[0].tabs[0].agent_statuses,
            vec![AgentStatus::Working]
        );
        assert!(!app.results[0].tabs[1].matches_query);
        app.set_query("done".into());
        assert!(app.results.is_empty());
    }
}

#[test]
fn reopening_uses_new_status_without_refreshing_existing_picker() {
    let mut original = app_with_agents(
        "working",
        "[ui]\nstatus_indicators = 'symbols'",
        SearchMode::Flat,
        agents(&["working", "blocked"]),
    );
    let mut reopened = app_with_agents(
        "done",
        "[ui]\nstatus_indicators = 'symbols'",
        SearchMode::Flat,
        agents(&["done", "idle"]),
    );
    let old = draw(&mut original, 80);
    let new = draw(&mut reopened, 80);
    let hit = original
        .result_hits
        .iter()
        .find(|hit| hit.tab == Some(0))
        .unwrap()
        .area;
    assert_eq!(old[(hit.x + 5, hit.y)].fg, original.theme.palette().yellow);
    assert_eq!(new[(hit.x + 5, hit.y)].fg, reopened.theme.palette().teal);
    assert_eq!(tab_text(&original, &old), "[◐ × Codex]");
    assert_eq!(tab_text(&reopened, &new), "[✓ ○ Codex]");
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
