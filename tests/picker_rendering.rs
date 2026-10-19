use herdrpp::picker::{
    api::parse_snapshot,
    app::App,
    config::SearchMode,
    ui::{label_spans, render},
};
use ratatui::{Terminal, backend::TestBackend};
use terminal_input::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

fn draw(width: u16, height: u16, query: &str) -> String {
    let mut app = App::new(
        parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
        SearchMode::Hierarchical,
    );
    app.set_query(query.into());
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, &mut app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
#[test]
fn empty_result_text_is_visible() {
    assert!(draw(60, 18, "zzzzzzz").contains("No matching results"));
}
#[test]
fn normal_palette_contains_input_count_results_and_help() {
    let rendered = draw(80, 20, "");
    assert!(rendered.contains("Search workspaces, worktrees and tabs"));
    assert!(rendered.contains("3 results"));
    assert!(rendered.contains("ColorPicker"));
    assert!(rendered.contains("↑↓/Ctrl+P,N Workspaces  ←→/Tab/Shift+Tab Tabs"));
    assert!(rendered.contains("↵ OK"));
    assert!(rendered.contains("Esc Cancel"));
    assert!(!rendered.contains("Enter Switch"));
    assert!(!rendered.contains("Esc Close"));
}
#[test]
fn tiny_terminal_has_escape_guidance() {
    assert_eq!(draw(6, 1, ""), "Esc cl");
}
#[test]
fn long_wide_labels_are_truncated_by_cell_width() {
    let spans = label_spans("世界Hello", &[0], 5);
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "世界…");
    assert_eq!(text.width(), 5);
}
#[test]
fn matched_characters_are_underlined() {
    let spans = label_spans("Codex", &[0, 2], 20);
    assert!(
        spans[0]
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    assert!(
        spans[2]
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    assert_eq!(spans[1].style, ratatui::style::Style::default());
}
#[test]
fn control_characters_cannot_inject_terminal_sequences() {
    let spans = label_spans("a\n\u{1b}b", &[], 10);
    assert_eq!(
        spans.iter().map(|s| s.content.as_ref()).collect::<String>(),
        "a��b"
    );
}
#[test]
fn zero_width_labels_are_empty() {
    assert!(label_spans("Codex", &[], 0).is_empty());
}

#[test]
fn single_tab_workspace_renders_one_tab_to_its_right() {
    let rendered = draw(80, 20, "exact terminal");
    assert!(rendered.contains("[· Terminal]"));
    assert_eq!(rendered.matches("Terminal").count(), 1);
    assert!(rendered.contains("1 results"));
}

fn single_tab_buffer(width: u16, mode: SearchMode) -> ratatui::buffer::Buffer {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    snapshot.workspaces[2].label = "世界Exact".into();
    snapshot.focused_workspace_id = Some("w3".into());
    snapshot.focused_tab_id = Some("w3:t1".into());
    let mut app = App::new(snapshot, mode);
    app.set_query("世界 terminal".into());
    app.handle(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
    let mut terminal = Terminal::new(TestBackend::new(width, 10)).unwrap();
    terminal.draw(|frame| render(frame, &mut app)).unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn sole_tab_and_workspace_keep_highlights_and_active_markers() {
    let buffer = single_tab_buffer(40, SearchMode::Hierarchical);
    assert_eq!(buffer[(0, 2)].symbol(), " ");
    assert_eq!(buffer[(1, 2)].symbol(), "●");
    assert_eq!(buffer[(3, 2)].symbol(), "世");
    assert_eq!(buffer[(22, 2)].symbol(), "T");
    assert!(
        buffer[(3, 2)]
            .modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    assert!(
        buffer[(22, 2)]
            .modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    assert_eq!(
        buffer[(22, 2)].bg,
        herdrpp::picker::theme::Palette::catppuccin().selection_bg
    );
    assert_eq!(
        buffer[(22, 2)].fg,
        herdrpp::picker::theme::Palette::catppuccin().overlay0
    );
}

#[test]
fn flat_single_tab_uses_the_right_hand_strip() {
    let buffer = single_tab_buffer(40, SearchMode::Flat);
    let text: String = (0..40).map(|x| buffer[(x, 2)].symbol()).collect();
    assert!(text.contains("世 界 Exact"));
    assert!(text.contains("[● · Terminal]"));
}

#[test]
fn narrow_single_tab_row_truncates_each_column_by_cell_width() {
    let buffer = single_tab_buffer(14, SearchMode::Hierarchical);
    let text: String = (0..14).map(|x| buffer[(x, 2)].symbol()).collect();
    assert!(text.contains("世 …"));
    assert!(text.contains("[· T…]"), "{text:?}");
}

fn worktree_buffer(width: u16, mode: SearchMode, query: &str) -> ratatui::buffer::Buffer {
    let snapshot = parse_snapshot(include_bytes!("fixtures/worktrees.json")).unwrap();
    let mut app = App::new(snapshot, mode);
    app.set_query(query.into());
    let mut terminal = Terminal::new(TestBackend::new(width, 18)).unwrap();
    terminal.draw(|frame| render(frame, &mut app)).unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn root_and_worktree_stay_indented_with_tabs_on_the_worktree_row() {
    let buffer = worktree_buffer(80, SearchMode::Hierarchical, "herdrpp multimachine codex");
    assert_eq!(buffer[(3, 3)].symbol(), "h");
    assert_eq!(buffer[(5, 2)].symbol(), "m");
    assert_eq!(buffer[(36, 2)].symbol(), "C");
    assert!(
        buffer[(3, 3)]
            .modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    assert!(
        buffer[(5, 2)]
            .modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    assert!(
        buffer[(36, 2)]
            .modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    let blank: String = (0..80).map(|x| buffer[(x, 4)].symbol()).collect();
    assert_eq!(blank, " ".repeat(80));
}

#[test]
fn flat_workspace_breadcrumb_and_tab_share_one_row() {
    let buffer = worktree_buffer(80, SearchMode::Flat, "herdrpp multimachine codex");
    let text: String = (0..80).map(|x| buffer[(x, 2)].symbol()).collect();
    assert!(text.contains("herdrpp / multimachine"));
    assert!(text.contains("[· Codex]"));
    assert!(
        buffer[(3, 2)]
            .modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    assert!(
        buffer[(18, 2)]
            .modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    assert!(
        buffer[(36, 2)]
            .modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
}

#[test]
fn nested_single_tab_worktree_keeps_its_active_workspace_marker() {
    let buffer = worktree_buffer(80, SearchMode::Hierarchical, "just terminal");
    let text: String = (0..80).map(|x| buffer[(x, 2)].symbol()).collect();
    assert!(text.contains("[● · Terminal]"));
    assert_eq!(buffer[(1, 3)].symbol(), " ");
    assert_eq!(buffer[(1, 2)].symbol(), "●");
    assert_eq!(buffer[(5, 2)].symbol(), "j");
}

#[test]
fn narrow_rows_keep_tabs_to_the_right_instead_of_stacking() {
    let buffer = worktree_buffer(10, SearchMode::Hierarchical, "multimachine codex");
    assert_eq!(buffer[(7, 2)].symbol(), "C");
    assert_eq!(buffer[(8, 2)].symbol(), "…");
    let flat = worktree_buffer(14, SearchMode::Flat, "herdrpp multimachine codex");
    let text: String = (0..14).map(|x| flat[(x, 2)].symbol()).collect();
    assert!(text.contains("[· C…]"), "{text:?}");
}

#[test]
fn nested_unicode_breadcrumb_truncates_without_moving_tabs_below_workspace() {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/worktrees.json")).unwrap();
    snapshot.workspaces[1].label = "世界herdrpp".into();
    let mut app = App::new(snapshot, SearchMode::Flat);
    app.set_query("世界 multimachine codex".into());
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    let buffer = app_buffer(&mut app, 24);
    let text: String = (0..24).map(|x| buffer[(x, 2)].symbol()).collect();
    assert!(text.contains("[· Codex]"));
    assert!(text.contains("…"));
    assert_eq!(buffer[(11, 2)].bg, app.theme.palette().selection_bg);
}

fn themed_draw(width: u16, height: u16, query: &str) -> (App, ratatui::buffer::Buffer) {
    let mut app = App::new(
        parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
        SearchMode::Hierarchical,
    );
    app.theme = herdrpp::picker::theme::Theme::parse("[theme]\nname = 'dracula'").unwrap();
    app.set_query(query.into());
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, &mut app)).unwrap();
    (app, terminal.backend().buffer().clone())
}

#[test]
fn dracula_colors_fill_panel_selection_hints_and_buttons() {
    let (app, buffer) = themed_draw(80, 20, "color");
    let palette = app.theme.palette();
    assert_eq!(buffer[(79, 10)].bg, palette.panel_bg);
    assert_eq!(buffer[(0, 0)].fg, palette.text);
    assert_eq!(buffer[(0, 2)].bg, palette.selection_bg);
    assert_eq!(buffer[(0, 2)].fg, palette.text);
    assert_eq!(buffer[(1, 17)].fg, palette.overlay1);
    assert_eq!(buffer[(30, 19)].bg, palette.accent);
    assert_eq!(buffer[(33, 19)].symbol(), "O");
    assert_eq!(buffer[(38, 19)].bg, palette.surface0);
    assert_eq!(buffer[(38, 19)].fg, palette.text);
    let footer: String = (30..50).map(|x| buffer[(x, 19)].symbol()).collect();
    assert_eq!(footer, " ↵ OK    Esc Cancel ");
    let padding: String = (0..80).map(|x| buffer[(x, 18)].symbol()).collect();
    assert_eq!(padding, " ".repeat(80));
    assert_eq!(app.ok_area.x, 30);
    assert_eq!(app.cancel_area.right(), 50);
    assert_eq!(buffer[(29, 19)].bg, palette.panel_bg);
    assert_eq!(buffer[(50, 19)].bg, palette.panel_bg);
}

#[test]
fn empty_results_use_disabled_ok_style() {
    let (app, buffer) = themed_draw(80, 20, "nonexistenttttt");
    let palette = app.theme.palette();
    assert_eq!(buffer[(30, 19)].bg, palette.surface0);
    assert_eq!(buffer[(30, 19)].fg, palette.overlay0);
    assert_eq!(buffer[(38, 19)].fg, palette.text);
}

#[test]
fn compact_footer_shows_cancel_and_tiny_resize_clears_hits() {
    let (mut app, buffer) = themed_draw(14, 10, "");
    assert_eq!(app.ok_area, ratatui::layout::Rect::default());
    assert_eq!(app.cancel_area, ratatui::layout::Rect::new(3, 9, 8, 1));
    let footer: String = (0..14).map(|x| buffer[(x, 9)].symbol()).collect();
    assert_eq!(footer, "    Cancel    ");
    let mut terminal = Terminal::new(TestBackend::new(6, 1)).unwrap();
    terminal.draw(|frame| render(frame, &mut app)).unwrap();
    assert_eq!(app.cancel_area, ratatui::layout::Rect::default());
    assert_eq!(app.results_area, ratatui::layout::Rect::default());
}

#[test]
fn loading_panel_uses_shared_footer_with_disabled_ok() {
    let palette = herdrpp::picker::theme::Palette::dracula();
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    let mut cancel = ratatui::layout::Rect::default();
    terminal
        .draw(|frame| cancel = herdrpp::picker::ui::render_loading(frame, palette))
        .unwrap();
    assert_eq!(cancel, ratatui::layout::Rect::new(38, 19, 12, 1));
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(79, 10)].bg, palette.panel_bg);
    assert_eq!(buffer[(30, 19)].fg, palette.overlay0);
    assert_eq!(buffer[(38, 19)].bg, palette.surface0);
    let padding: String = (0..80).map(|x| buffer[(x, 18)].symbol()).collect();
    assert_eq!(padding, " ".repeat(80));
}

fn colored_worktree_app(mode: SearchMode, query: &str) -> App {
    let snapshot = parse_snapshot(include_bytes!("fixtures/worktrees.json")).unwrap();
    let mut app = App::new(snapshot, mode);
    app.theme = herdrpp::picker::theme::Theme::parse("[theme]\nname = 'dracula'").unwrap();
    app.set_query(query.into());
    app
}

fn app_buffer(app: &mut App, width: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, 18)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn dracula_hierarchy_colors_selected_root_worktree_and_tab_names() {
    let mut app = colored_worktree_app(SearchMode::Hierarchical, "herdrpp multimachine codex");
    app.selected_tab = None;
    let buffer = app_buffer(&mut app, 80);
    assert_eq!(buffer[(3, 3)].fg, ratatui::style::Color::Rgb(189, 147, 249));
    assert_eq!(buffer[(5, 2)].fg, ratatui::style::Color::Rgb(80, 250, 123));
    assert_eq!(buffer[(36, 2)].fg, ratatui::style::Color::Rgb(98, 114, 164));
    assert_eq!(buffer[(5, 2)].bg, app.theme.palette().selection_bg);
    assert_eq!(buffer[(3, 3)].bg, app.theme.palette().panel_bg);
    assert_eq!(buffer[(0, 2)].symbol(), "›");
    assert!(
        buffer[(5, 2)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD | ratatui::style::Modifier::UNDERLINED)
    );
}

#[test]
fn dracula_flat_breadcrumb_keeps_each_type_color_on_selected_row() {
    let mut app = colored_worktree_app(SearchMode::Flat, "herdrpp multimachine codex");
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    let buffer = app_buffer(&mut app, 80);
    assert_eq!(buffer[(3, 2)].fg, ratatui::style::Color::Rgb(189, 147, 249));
    assert_eq!(buffer[(18, 2)].fg, ratatui::style::Color::Rgb(80, 250, 123));
    assert_eq!(buffer[(36, 2)].fg, ratatui::style::Color::Rgb(98, 114, 164));
    assert_eq!(buffer[(11, 2)].symbol(), "/");
    assert_eq!(buffer[(11, 2)].fg, app.theme.palette().text);
    assert_eq!(buffer[(36, 2)].bg, app.theme.palette().selection_bg);
}

#[test]
fn dracula_single_tab_worktree_and_tab_have_separate_colors() {
    let mut app = colored_worktree_app(SearchMode::Hierarchical, "just terminal");
    let buffer = app_buffer(&mut app, 80);
    assert_eq!(buffer[(5, 2)].symbol(), "j");
    assert_eq!(buffer[(38, 2)].symbol(), "T");
    assert_eq!(buffer[(5, 2)].fg, app.theme.palette().green);
    assert_eq!(buffer[(38, 2)].fg, app.theme.palette().overlay0);
    assert_eq!(buffer[(33, 2)].fg, app.theme.palette().text);
}

#[test]
fn truncated_workspace_and_tab_keep_type_colors() {
    let mut app = colored_worktree_app(SearchMode::Flat, "herdrpp multimachine codex");
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    let buffer = app_buffer(&mut app, 24);
    assert_eq!(buffer[(3, 2)].fg, app.theme.palette().accent);
    assert_eq!(buffer[(7, 2)].fg, app.theme.palette().green);
    assert_eq!(buffer[(11, 2)].fg, app.theme.palette().overlay0);
    assert_eq!(buffer[(11, 2)].bg, app.theme.palette().selection_bg);
}

#[test]
fn appearance_updates_apply_custom_type_colors_without_changing_navigation() {
    let mut app = colored_worktree_app(SearchMode::Flat, "herdrpp multimachine codex");
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    app.theme = herdrpp::picker::theme::Theme::parse("[theme]\nname = 'dracula'\nauto_switch = true\nlight_name = 'one-light'\n[theme.custom]\naccent = '#123456'\ngreen = '#654321'\noverlay0 = '#abcdef'\n[theme.custom.light]\ngreen = '#102030'").unwrap();
    let dark = app_buffer(&mut app, 80);
    assert_eq!(dark[(3, 2)].fg, ratatui::style::Color::Rgb(18, 52, 86));
    assert_eq!(dark[(18, 2)].fg, ratatui::style::Color::Rgb(101, 67, 33));
    assert_eq!(dark[(36, 2)].fg, ratatui::style::Color::Rgb(171, 205, 239));
    let selected = app.selected;
    let offset = app.offset;
    app.handle(terminal_input::event::Event::ThemeModeChanged(
        terminal_input::event::ThemeMode::Light,
    ));
    let light = app_buffer(&mut app, 80);
    assert_eq!(light[(3, 2)].fg, ratatui::style::Color::Rgb(18, 52, 86));
    assert_eq!(light[(18, 2)].fg, ratatui::style::Color::Rgb(16, 32, 48));
    assert_eq!(light[(36, 2)].fg, ratatui::style::Color::Rgb(171, 205, 239));
    assert_eq!(
        light[(36, 2)].bg,
        herdrpp::picker::theme::Palette::one_light().selection_bg
    );
    assert_eq!(app.query, "herdrpp multimachine codex");
    assert_eq!(app.selected, selected);
    assert_eq!(app.offset, offset);
}

#[test]
fn dracula_single_tab_root_colors_workspace_and_tab_separately() {
    let (app, buffer) = themed_draw(80, 20, "exact terminal");
    assert_eq!(buffer[(3, 2)].symbol(), "E");
    assert_eq!(buffer[(36, 2)].symbol(), "T");
    assert_eq!(buffer[(3, 2)].fg, app.theme.palette().accent);
    assert_eq!(buffer[(36, 2)].fg, app.theme.palette().overlay0);
    assert_eq!(buffer[(19, 2)].fg, app.theme.palette().text);
}

#[test]
fn narrow_footer_keeps_both_arrow_navigation_hints_visible() {
    let rendered = draw(23, 20, "codex");
    assert!(rendered.contains("↑↓ Workspaces ←→/Tab"));
}

#[test]
fn selected_tab_scrolls_into_view_and_wrap_resets_the_horizontal_window() {
    let mut app = colored_worktree_app(SearchMode::Hierarchical, "multimachine");
    app.handle(Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)));
    let buffer = app_buffer(&mut app, 24);
    let text: String = (0..24).map(|x| buffer[(x, 2)].symbol()).collect();
    assert!(text.contains("◀[· Terminal]"), "{text:?}");
    let hit = app
        .result_hits
        .iter()
        .find(|hit| hit.workspace == 0 && hit.tab == Some(1))
        .unwrap()
        .area;
    assert_eq!(
        buffer[(hit.x + 1, hit.y)].bg,
        app.theme.palette().selection_bg
    );
    assert_eq!(buffer[(5, 2)].bg, app.theme.palette().panel_bg);
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    let buffer = app_buffer(&mut app, 24);
    let text: String = (0..24).map(|x| buffer[(x, 2)].symbol()).collect();
    assert!(text.contains("[· Codex]"));
    assert!(!text.contains("◀"));
    assert_eq!(app.selected_tab, Some(0));
}

#[test]
fn horizontal_strip_truncates_wide_unicode_labels_by_cell_width() {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    snapshot.tabs[0].label = "世界世界世界世界".into();
    let mut app = App::new(snapshot, SearchMode::Hierarchical);
    app.set_query("colorpicker".into());
    app.handle(Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)));
    let buffer = app_buffer(&mut app, 24);
    let hit = app
        .result_hits
        .iter()
        .find(|hit| hit.tab == Some(0))
        .unwrap()
        .area;
    let text: String = (hit.x..hit.right())
        .map(|x| buffer[(x, hit.y)].symbol())
        .collect();
    assert!(text.contains("…"));
    assert!(text.contains("世"));
    assert_eq!(buffer[(hit.right() - 1, hit.y)].symbol(), "]");
    assert!(hit.right() <= 24);
}

#[test]
fn active_tab_marker_is_retained_beside_its_workspace() {
    let rendered = draw(80, 20, "tcg");
    assert!(rendered.contains("[● · Codex]"));
    assert!(rendered.contains("[· Server]"));
}

#[test]
fn workspace_and_remembered_tab_are_highlighted_in_both_modes() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = colored_worktree_app(mode, "multimachine");
        app.selected = app
            .results
            .iter()
            .position(|result| result.workspace.workspace_id == "w2");
        let buffer = app_buffer(&mut app, 80);
        let workspace = app
            .result_hits
            .iter()
            .find(|hit| Some(hit.workspace) == app.selected && hit.tab.is_none())
            .unwrap()
            .area;
        let tab = app
            .result_hits
            .iter()
            .find(|hit| Some(hit.workspace) == app.selected && hit.tab == Some(0))
            .unwrap()
            .area;
        assert_eq!(
            buffer[(workspace.x, workspace.y)].bg,
            app.theme.palette().selection_bg
        );
        assert_eq!(buffer[(tab.x, tab.y)].bg, app.theme.palette().selection_bg);
        assert_eq!(workspace.y, tab.y);
    }
}

#[test]
fn strongest_tab_is_highlighted_without_highlighting_workspace_in_both_modes() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = colored_worktree_app(mode, "herdrpp multimachine codex");
        let buffer = app_buffer(&mut app, 24);
        let owner = app
            .result_hits
            .iter()
            .find(|hit| hit.workspace == 0 && hit.tab.is_none())
            .unwrap()
            .area;
        let tab = app
            .result_hits
            .iter()
            .find(|hit| hit.workspace == 0 && hit.tab == Some(0))
            .unwrap()
            .area;
        assert_eq!(buffer[(owner.x, owner.y)].bg, app.theme.palette().panel_bg);
        assert_eq!(buffer[(tab.x, tab.y)].bg, app.theme.palette().selection_bg);
        assert!(
            buffer[(tab.x + 3, tab.y)]
                .modifier
                .contains(ratatui::style::Modifier::UNDERLINED)
        );
    }
}

#[test]
fn remembered_tab_preview_scrolls_into_view_at_narrow_widths() {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    snapshot.workspaces[0].active_tab_id = "w1:t9".into();
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(snapshot.clone(), mode);
        app.set_query("colorpicker".into());
        let buffer = app_buffer(&mut app, 24);
        let hit = app
            .result_hits
            .iter()
            .find(|hit| hit.tab == Some(2))
            .unwrap()
            .area;
        assert_eq!(app.selected_tab, None);
        assert_eq!(buffer[(hit.x, hit.y)].bg, app.theme.palette().selection_bg);
        assert_eq!(buffer[(hit.x + 3, hit.y)].symbol(), "T");
        assert!(
            !buffer[(hit.x + 3, hit.y)]
                .modifier
                .contains(ratatui::style::Modifier::UNDERLINED)
        );
    }
}

#[test]
fn context_workspace_preview_shows_retained_tab_without_match_underlines() {
    let snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(snapshot.clone(), mode);
        app.set_query("server".into());
        app.results_area = ratatui::layout::Rect::new(0, 2, 80, 5);
        app.handle(Event::Mouse(terminal_input::event::MouseEvent {
            kind: terminal_input::event::MouseEventKind::ScrollDown,
            column: 5,
            row: 2,
            modifiers: KeyModifiers::NONE,
        }));
        let buffer = app_buffer(&mut app, 80);
        let hit = app
            .result_hits
            .iter()
            .find(|hit| hit.tab == Some(1))
            .unwrap()
            .area;
        assert_eq!(app.selected_tab, None);
        assert_eq!(buffer[(hit.x + 5, hit.y)].symbol(), "C");
        assert_eq!(buffer[(hit.x, hit.y)].bg, app.theme.palette().selection_bg);
        assert!(
            !buffer[(hit.x + 5, hit.y)]
                .modifier
                .contains(ratatui::style::Modifier::UNDERLINED)
        );
    }
}
