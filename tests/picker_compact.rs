use herdrpp::picker::{api::parse_snapshot, app::App, config::SearchMode, ui};
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn popup_manifest_keeps_the_live_terminal_visible_around_the_picker() {
    let manifest: toml::Value = toml::from_str(include_str!("../herdr-plugin.toml")).unwrap();
    let picker = &manifest["panes"][0];
    assert_eq!(picker["placement"].as_str(), Some("popup"));
    assert_eq!(picker["width"].as_str(), Some("80%"));
    assert_eq!(picker["height"].as_integer(), Some(22));
}

#[test]
fn contents_use_the_host_popup_surface_without_a_nested_frame() {
    let mut app = App::new(
        parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
        SearchMode::Hierarchical,
    );
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    terminal.draw(|frame| ui::render(frame, &mut app)).unwrap();
    assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), ">");
    assert_eq!(app.ok_area.y, 19);
    assert_eq!(app.cancel_area.y, 19);
}
