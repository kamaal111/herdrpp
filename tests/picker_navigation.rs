use herdrpp::picker::{
    api::parse_snapshot,
    app::{Action, App},
    config::SearchMode,
    model::Target,
};
use ratatui::layout::Rect;
use terminal_input::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

fn app() -> App {
    App::new(
        parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
        SearchMode::Hierarchical,
    )
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn mouse(kind: MouseEventKind, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column: 5,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn initial_selection_uses_first_other_workspace_when_no_mru_available() {
    let mut app = app();
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w1".into()))
    );
}
#[test]
fn initial_order_is_current_workspace_then_ui_order() {
    assert_eq!(
        app().results[0].workspace.target,
        Target::Workspace("w2".into())
    );
}
#[test]
fn equal_scores_prefer_workspace_even_after_equally_ranked_tabs() {
    let mut app = app();
    app.set_query("codex".into());
    assert_eq!(app.selected, Some(2));
    assert_eq!(app.offset, 0);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w3".into()))
    );
}
#[test]
fn backtab_from_first_tab_wraps_to_last_tab() {
    let mut app = app();
    app.set_query("colorpicker".into());
    app.handle(key(KeyCode::BackTab));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t4".into()))
    );
    app.handle(key(KeyCode::BackTab));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t9".into()))
    );
}

#[test]
fn control_n_and_p_navigate() {
    let mut app = app();
    app.set_query("".into());
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('n'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(app.selected, Some(1));
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('p'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(app.selected, Some(0));
}
#[test]
fn empty_results_ignore_enter_and_navigation() {
    let mut app = app();
    app.set_query("nonexistenttttt".into());
    app.navigate(1);
    app.navigate(-1);
    assert_eq!(app.selected, None);
    assert_eq!(app.handle(key(KeyCode::Enter)), Action::Continue);
}
#[test]
fn escape_closes_immediately_without_clearing_query() {
    let mut app = app();
    app.set_query("color".into());
    assert_eq!(app.handle(key(KeyCode::Esc)), Action::Close);
    assert_eq!(app.query(), "color");
}
#[test]
fn typing_filters_immediately_without_switching() {
    let mut app = app();
    assert_eq!(app.handle(key(KeyCode::Char('c'))), Action::Continue);
    assert_eq!(app.query(), "c");
    assert_eq!(app.selected, Some(1));
}
#[test]
fn backspace_updates_query_and_selection() {
    let mut app = app();
    app.set_query("color".into());
    app.navigate(1);
    app.handle(key(KeyCode::Backspace));
    assert_eq!(app.query(), "colo");
    assert_eq!(app.selected, Some(0));
}

#[test]
fn command_backspace_clears_query_and_restores_unfiltered_selection() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(
            parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
            mode,
        );
        app.set_query("colorpicker".into());
        app.handle(key(KeyCode::Tab));
        assert!(app.selected_tab.is_some());
        app.offset = 1;
        app.result_hits.push(herdrpp::picker::app::ResultHit {
            area: Rect::new(0, 0, 10, 1),
            workspace: 0,
            tab: Some(0),
        });

        assert_eq!(
            app.handle(Event::Key(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::SUPER,
            ))),
            Action::Continue
        );
        assert_eq!(app.query(), "");
        assert_eq!(
            app.results
                .iter()
                .map(|result| result.workspace.target.clone())
                .collect::<Vec<_>>(),
            vec![
                Target::Workspace("w2".into()),
                Target::Workspace("w1".into()),
                Target::Workspace("w3".into()),
            ]
        );
        assert_eq!(app.selected, Some(0));
        assert_eq!(app.selected_tab, None);
        assert_eq!(app.offset, 0);
        assert!(app.result_hits.is_empty());
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Workspace("w2".into()))
        );
    }
}

#[test]
fn command_backspace_restores_results_from_no_matches_and_handles_empty_query() {
    let mut app = app();
    app.set_query("nonexistenttttt".into());
    assert!(app.results.is_empty());
    let clear = Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::SUPER));
    assert_eq!(app.handle(clear.clone()), Action::Continue);
    assert_eq!(app.query(), "");
    assert_eq!(app.results.len(), 3);
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.handle(clear), Action::Continue);
    assert_eq!(app.query(), "");
    assert_eq!(app.results.len(), 3);
    assert_eq!(app.selected, Some(0));
}

#[test]
fn command_backspace_release_does_not_edit_query() {
    let mut app = app();
    app.set_query("colorpicker".into());
    let mut event = KeyEvent::new(KeyCode::Backspace, KeyModifiers::SUPER);
    event.kind = terminal_input::event::KeyEventKind::Release;
    assert_eq!(app.handle(Event::Key(event)), Action::Continue);
    assert_eq!(app.query(), "colorpicker");
}

#[test]
fn ghostty_command_backspace_control_u_clears_query_and_restores_selection() {
    let mut app = app();
    app.set_query("colorpicker".into());
    app.handle(key(KeyCode::Tab));
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        ))),
        Action::Continue
    );
    assert_eq!(app.query(), "");
    assert_eq!(app.results.len(), 3);
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.selected_tab, None);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w2".into()))
    );
}

#[test]
fn plain_u_types_and_control_u_release_does_not_clear() {
    let mut app = app();
    app.set_query("color".into());
    assert_eq!(app.handle(key(KeyCode::Char('u'))), Action::Continue);
    assert_eq!(app.query(), "coloru");
    let mut release = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
    release.kind = terminal_input::event::KeyEventKind::Release;
    assert_eq!(app.handle(Event::Key(release)), Action::Continue);
    assert_eq!(app.query(), "coloru");
}

#[test]
fn backspace_removes_one_unicode_character() {
    let mut app = app();
    app.set_query("color日本".into());
    assert_eq!(app.handle(key(KeyCode::Backspace)), Action::Continue);
    assert_eq!(app.query(), "color日");
}

#[test]
fn search_reset_and_resize_keep_selection_visible() {
    let mut app = app();
    app.navigate(100);
    app.ensure_visible(2);
    assert_eq!(app.offset, 1);
    app.set_query("cod".into());
    app.ensure_visible(1);
    assert_eq!(app.offset, 2);
}
#[test]
fn horizontal_navigation_wraps_in_ui_order_without_selecting_the_workspace() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(
            parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
            mode,
        );
        app.set_query("colorpicker".into());
        for tab in ["w1:t9", "w1:t4", "w1:t8", "w1:t9"] {
            assert_eq!(app.handle(key(KeyCode::Tab)), Action::Continue);
            assert_eq!(
                app.handle(key(KeyCode::Enter)),
                Action::Switch(Target::Tab(tab.into()))
            );
        }
        for tab in ["w1:t8", "w1:t4", "w1:t9", "w1:t8"] {
            app.handle(key(KeyCode::BackTab));
            assert_eq!(
                app.handle(key(KeyCode::Enter)),
                Action::Switch(Target::Tab(tab.into()))
            );
        }
        assert_eq!(app.results.len(), 1);
    }
}

#[test]
fn vertical_navigation_skips_tabs_and_uses_the_selected_tabs_owner() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(
            parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
            mode,
        );
        app.handle(key(KeyCode::Tab));
        app.handle(key(KeyCode::Down));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Workspace("w3".into()))
        );
        app.handle(key(KeyCode::Down));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Workspace("w3".into()))
        );
        app.handle(key(KeyCode::Up));
        app.handle(key(KeyCode::Tab));
        app.handle(key(KeyCode::Up));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Workspace("w2".into()))
        );
        app.handle(key(KeyCode::Up));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Workspace("w2".into()))
        );
    }
}

#[test]
fn flat_filtered_navigation_follows_relevance_and_retains_remembered_tab() {
    let mut app = App::new(
        parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
        SearchMode::Flat,
    );
    app.set_query("pickercod".into());
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t8".into()))
    );
    app.handle(key(KeyCode::Tab));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t4".into()))
    );
    app.handle(key(KeyCode::Tab));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t8".into()))
    );
    app.handle(key(KeyCode::Tab));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t4".into()))
    );
    app.set_query("server".into());
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w2:t2".into()))
    );
    app.handle(key(KeyCode::Tab));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w2:t7".into()))
    );
    app.handle(key(KeyCode::BackTab));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w2:t2".into()))
    );
}

#[test]
fn mouse_click_switches_the_visible_workspace_label() {
    let mut app = drawn_app(80, 20);
    assert_eq!(
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left), 2)),
        Action::Switch(Target::Workspace("w2".into()))
    );
}

#[test]
fn mouse_wheel_scrolls_and_keyboard_continues_from_selection() {
    let mut app = app();
    app.results_area = Rect::new(0, 2, 40, 5);
    app.set_query("".into());
    app.handle(mouse(MouseEventKind::ScrollDown, 3));
    app.ensure_visible(2);
    assert_eq!(app.selected, Some(2));
    assert_eq!(app.offset, 1);
    app.handle(key(KeyCode::Up));
    assert_eq!(app.selected, Some(1));
}
#[test]
fn reopening_has_no_previous_query() {
    assert_eq!(app().query(), "");
}

#[test]
fn single_workspace_fallback_selects_its_workspace_row() {
    let snapshot = herdrpp::picker::model::Snapshot {
        workspaces: vec![herdrpp::picker::model::Workspace {
            workspace_id: "w1".into(),
            label: "Only".into(),
            number: 1,
            active_tab_id: "w1:t1".into(),
            worktree: None,
        }],
        tabs: vec![herdrpp::picker::model::Tab {
            agent_status: Default::default(),
            tab_id: "w1:t1".into(),
            workspace_id: "w1".into(),
            label: "Terminal".into(),
            number: 1,
        }],
        focused_workspace_id: Some("w1".into()),
        focused_tab_id: Some("w1:t1".into()),
    };
    let mut app = App::new(snapshot, SearchMode::Hierarchical);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w1".into()))
    );
}

#[test]
fn empty_session_has_no_selection_and_can_close() {
    let snapshot = herdrpp::picker::model::Snapshot {
        workspaces: vec![],
        tabs: vec![],
        focused_workspace_id: None,
        focused_tab_id: None,
    };
    let mut app = App::new(snapshot, SearchMode::Flat);
    assert_eq!(app.selected, None);
    assert_eq!(app.handle(key(KeyCode::Enter)), Action::Continue);
    assert_eq!(app.handle(key(KeyCode::Esc)), Action::Close);
}

#[test]
fn single_tab_search_enter_focuses_tab_and_workspace_click_focuses_owner() {
    let mut app = app();
    app.set_query("exact terminal".into());
    assert_eq!(app.results.len(), 1);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w3:t1".into()))
    );
    draw_app(&mut app, 80, 20);
    assert_eq!(
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left), 2)),
        Action::Switch(Target::Workspace("w3".into()))
    );
}

fn worktree_app() -> App {
    App::new(
        parse_snapshot(include_bytes!("fixtures/worktrees.json")).unwrap(),
        SearchMode::Hierarchical,
    )
}

#[test]
fn focused_worktree_group_starts_with_root_without_marking_root_active() {
    let app = worktree_app();
    assert_eq!(
        app.results[0].workspace.target,
        Target::Workspace("w1".into())
    );
    assert!(!app.results[0].workspace.active);
    let active: Vec<_> = app
        .results
        .iter()
        .filter(|row| row.workspace.active)
        .map(|row| row.workspace.target.clone())
        .collect();
    assert_eq!(active, vec![Target::Workspace("w4".into())]);
}

#[test]
fn worktree_enter_focuses_actual_workspace_and_nested_tab_enter_focuses_tab() {
    let mut app = worktree_app();
    app.set_query("multimachine".into());
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w2".into()))
    );
    app.handle(key(KeyCode::BackTab));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w2:t2".into()))
    );
}

#[test]
fn nested_worktree_and_tab_mouse_clicks_focus_actual_ids_on_the_same_row() {
    let mut app = worktree_app();
    app.set_query("multimachine codex".into());
    draw_app(&mut app, 80, 20);
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
    assert_eq!(owner.y, tab.y);
    assert!(tab.x > owner.right());
    assert_eq!(
        app.handle(click(
            owner.x,
            owner.y,
            MouseEventKind::Down(MouseButton::Left)
        )),
        Action::Switch(Target::Workspace("w2".into()))
    );
    assert_eq!(
        app.handle(click(tab.x, tab.y, MouseEventKind::Down(MouseButton::Left))),
        Action::Switch(Target::Tab("w2:t1".into()))
    );
}

#[test]
fn nested_rows_remain_reachable_after_scrolling_and_query_reset() {
    let mut app = worktree_app();
    assert_eq!(app.selected, Some(0));
    app.navigate(10);
    app.ensure_visible(2);
    assert_eq!(app.offset, 4);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w0".into()))
    );
    app.set_query("just terminal".into());
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.offset, 0);
    app.navigate(1);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w1".into()))
    );
}

fn draw_app(app: &mut App, width: u16, height: u16) {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| herdrpp::picker::ui::render(frame, app))
        .unwrap();
}

fn drawn_app(width: u16, height: u16) -> App {
    let mut app = app();
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| herdrpp::picker::ui::render(frame, &mut app))
        .unwrap();
    app
}

fn click(column: u16, row: u16, kind: MouseEventKind) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn ok_click_performs_enter_for_workspace_and_tab() {
    let mut app = drawn_app(80, 20);
    let ok = app.ok_area;
    assert_eq!(
        app.handle(click(ok.x, ok.y, MouseEventKind::Down(MouseButton::Left))),
        Action::Switch(Target::Workspace("w1".into()))
    );
    app.set_query("colorpicker".into());
    app.handle(key(KeyCode::Tab));
    assert_eq!(
        app.handle(click(
            ok.right() - 1,
            ok.y,
            MouseEventKind::Down(MouseButton::Left)
        )),
        app.handle(key(KeyCode::Enter))
    );
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t9".into()))
    );
}

#[test]
fn cancel_closes_without_changing_query_or_selection() {
    let mut app = drawn_app(80, 20);
    app.set_query("color".into());
    let selected = app.selected;
    let cancel = app.cancel_area;
    assert_eq!(
        app.handle(click(
            cancel.x,
            cancel.y,
            MouseEventKind::Down(MouseButton::Left)
        )),
        Action::Close
    );
    assert_eq!(app.query(), "color");
    assert_eq!(app.selected, selected);
}

#[test]
fn empty_results_disable_ok_but_cancel_still_closes() {
    let mut app = drawn_app(80, 20);
    app.set_query("nonexistenttttt".into());
    assert_eq!(
        app.handle(click(
            app.ok_area.x,
            app.ok_area.y,
            MouseEventKind::Down(MouseButton::Left)
        )),
        Action::Continue
    );
    assert_eq!(
        app.handle(click(
            app.cancel_area.x,
            app.cancel_area.y,
            MouseEventKind::Down(MouseButton::Left)
        )),
        Action::Close
    );
}

#[test]
fn button_gaps_releases_and_right_clicks_do_nothing() {
    let mut app = drawn_app(80, 20);
    let ok = app.ok_area;
    assert_eq!(
        app.handle(click(
            ok.right(),
            ok.y,
            MouseEventKind::Down(MouseButton::Left)
        )),
        Action::Continue
    );
    assert_eq!(
        app.handle(click(ok.x, ok.y, MouseEventKind::Up(MouseButton::Left))),
        Action::Continue
    );
    assert_eq!(
        app.handle(click(
            app.cancel_area.x,
            ok.y,
            MouseEventKind::Down(MouseButton::Right)
        )),
        Action::Continue
    );
}

#[test]
fn resize_clears_old_button_hits_and_keeps_compact_cancel() {
    let mut app = drawn_app(80, 20);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(14, 10)).unwrap();
    terminal
        .draw(|frame| herdrpp::picker::ui::render(frame, &mut app))
        .unwrap();
    assert_eq!(app.ok_area, Rect::default());
    assert_eq!(app.cancel_area, Rect::new(3, 9, 8, 1));
    assert_eq!(
        app.handle(click(0, 19, MouseEventKind::Down(MouseButton::Left))),
        Action::Continue
    );
    assert_eq!(
        app.handle(click(3, 9, MouseEventKind::Down(MouseButton::Left))),
        Action::Close
    );
}

#[test]
fn appearance_changes_preserve_query_selection_and_scroll() {
    let mut app = drawn_app(80, 20);
    app.theme =
        herdrpp::picker::theme::Theme::parse("[theme]\nname = 'catppuccin'\nauto_switch = true")
            .unwrap();
    app.set_query("color".into());
    app.navigate(1);
    app.ensure_visible(1);
    let selected = app.selected;
    let offset = app.offset;
    assert_eq!(
        app.handle(Event::ThemeModeChanged(
            terminal_input::event::ThemeMode::Light
        )),
        Action::Continue
    );
    assert_eq!(app.query(), "color");
    assert_eq!(app.selected, selected);
    assert_eq!(app.offset, offset);
    assert_eq!(
        app.theme.palette(),
        herdrpp::picker::theme::Palette::catppuccin_latte()
    );
}

#[test]
fn flat_interleaved_results_navigate_tabs_by_owner_and_workspaces_by_result_order() {
    let mut app = App::new(
        parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
        SearchMode::Flat,
    );
    app.set_query("cod".into());
    app.handle(key(KeyCode::Up));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w1".into()))
    );
    app.handle(key(KeyCode::Tab));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t4".into()))
    );
    app.handle(key(KeyCode::Tab));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t8".into()))
    );
    app.handle(key(KeyCode::Down));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w3".into()))
    );
}

#[test]
fn worktree_horizontal_navigation_stays_with_its_actual_owner_in_both_modes() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(
            parse_snapshot(include_bytes!("fixtures/worktrees.json")).unwrap(),
            mode,
        );
        app.set_query("".into());
        app.handle(key(KeyCode::Down));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Workspace("w2".into()))
        );
        for tab in ["w2:t2", "w2:t1", "w2:t2"] {
            app.handle(key(KeyCode::Tab));
            assert_eq!(
                app.handle(key(KeyCode::Enter)),
                Action::Switch(Target::Tab(tab.into()))
            );
        }
        app.handle(key(KeyCode::BackTab));
        app.handle(key(KeyCode::BackTab));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Tab("w2:t2".into()))
        );
        app.handle(key(KeyCode::Tab));
        app.handle(key(KeyCode::Down));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Workspace("w3".into()))
        );
    }
}

#[test]
fn horizontal_navigation_cycles_a_sole_tab_and_ignores_empty_results() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(
            parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
            mode,
        );
        app.set_query("exact terminal".into());
        app.handle(key(KeyCode::Tab));
        app.handle(key(KeyCode::BackTab));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Tab("w3:t1".into()))
        );
        assert_eq!(app.results.len(), 1);
        app.set_query("nonexistenttttt".into());
        app.handle(key(KeyCode::BackTab));
        app.handle(key(KeyCode::Tab));
        assert_eq!(app.selected, None);
        assert_eq!(app.handle(key(KeyCode::Enter)), Action::Continue);
    }
}

#[test]
fn horizontal_navigation_sorts_by_tab_number_and_keeps_selection_visible() {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    snapshot.tabs.reverse();
    let mut app = App::new(snapshot, SearchMode::Hierarchical);
    app.set_query("colorpicker".into());
    app.handle(key(KeyCode::Tab));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t9".into()))
    );
    app.ensure_visible(1);
    assert_eq!(app.offset, app.selected.unwrap());
    app.handle(key(KeyCode::BackTab));
    app.ensure_visible(1);
    assert_eq!(app.offset, 0);
}

#[test]
fn arrow_key_releases_do_not_move_selection() {
    let mut app = app();
    let selected = app.selected;
    for code in [
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Tab,
        KeyCode::BackTab,
    ] {
        let mut event = KeyEvent::new(code, KeyModifiers::NONE);
        event.kind = terminal_input::event::KeyEventKind::Release;
        assert_eq!(app.handle(Event::Key(event)), Action::Continue);
        assert_eq!(app.selected, selected);
    }
}

#[test]
fn tab_and_shift_tab_cycle_including_entry_and_wrap() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        for forward in [key(KeyCode::Tab)] {
            let mut app = App::new(
                parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
                mode,
            );
            app.set_query("colorpicker".into());
            for tab in ["w1:t9", "w1:t4", "w1:t8", "w1:t9"] {
                app.handle(forward.clone());
                assert_eq!(
                    app.handle(key(KeyCode::Enter)),
                    Action::Switch(Target::Tab(tab.into()))
                );
            }
        }
        for backward in [
            key(KeyCode::BackTab),
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT)),
        ] {
            let mut app = App::new(
                parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
                mode,
            );
            app.set_query("colorpicker".into());
            for tab in ["w1:t4", "w1:t9", "w1:t8", "w1:t4"] {
                app.handle(backward.clone());
                assert_eq!(
                    app.handle(key(KeyCode::Enter)),
                    Action::Switch(Target::Tab(tab.into()))
                );
            }
        }
    }
}

#[test]
fn vertical_boundary_keeps_tab_selection_and_workspace_moves_clear_it() {
    let mut app = app();
    app.set_query("".into());
    app.handle(key(KeyCode::Tab));
    app.handle(key(KeyCode::Up));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w2:t2".into()))
    );
    app.handle(key(KeyCode::Down));
    assert_eq!(app.selected_tab, None);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w1".into()))
    );
    app.handle(key(KeyCode::Tab));
    app.set_query("colorpicker".into());
    assert_eq!(app.selected_tab, None);
    assert_eq!(app.offset, 0);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w1".into()))
    );
}

#[test]
fn result_gaps_and_overflow_indicators_do_not_activate_destinations() {
    let mut app = app();
    app.set_query("colorpicker".into());
    app.handle(key(KeyCode::Tab));
    draw_app(&mut app, 24, 20);
    let tab = app
        .result_hits
        .iter()
        .find(|hit| hit.tab == Some(2))
        .unwrap()
        .area;
    assert_eq!(
        app.handle(click(
            tab.x - 1,
            tab.y,
            MouseEventKind::Down(MouseButton::Left)
        )),
        Action::Continue
    );
    assert_eq!(
        app.handle(click(
            tab.x - 2,
            tab.y,
            MouseEventKind::Down(MouseButton::Left)
        )),
        Action::Continue
    );
    assert_eq!(app.selected_tab, Some(2));
    app.handle(key(KeyCode::Tab));
    draw_app(&mut app, 24, 20);
    assert_eq!(
        app.handle(click(23, tab.y, MouseEventKind::Down(MouseButton::Left))),
        Action::Continue
    );
    assert_eq!(app.selected_tab, Some(0));
}

#[test]
fn filtering_and_tiny_resize_discard_stale_result_hits() {
    let mut app = drawn_app(80, 20);
    assert!(!app.result_hits.is_empty());
    app.set_query("exact terminal".into());
    assert!(app.result_hits.is_empty());
    assert_eq!(
        app.handle(click(34, 2, MouseEventKind::Down(MouseButton::Left))),
        Action::Continue
    );
    draw_app(&mut app, 80, 20);
    assert_eq!(app.result_hits.len(), 2);
    draw_app(&mut app, 6, 1);
    assert!(app.result_hits.is_empty());
    assert_eq!(
        app.handle(click(3, 2, MouseEventKind::Down(MouseButton::Left))),
        Action::Continue
    );
}

#[test]
fn wheel_clears_tab_selection_even_when_the_workspace_cannot_move() {
    let mut app = app();
    app.set_query("colorpicker".into());
    app.handle(key(KeyCode::Tab));
    draw_app(&mut app, 80, 20);
    app.handle(mouse(MouseEventKind::ScrollDown, 2));
    assert_eq!(app.selected_tab, None);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w1".into()))
    );
}

#[test]
fn resize_event_clears_old_click_targets_before_redraw() {
    let mut app = drawn_app(80, 20);
    let tab = app
        .result_hits
        .iter()
        .find(|hit| hit.tab == Some(0))
        .unwrap()
        .area;
    let selected = app.selected;
    app.handle(Event::Resize(24, 10));
    assert!(app.result_hits.is_empty());
    assert_eq!(app.ok_area, Rect::default());
    assert_eq!(app.cancel_area, Rect::default());
    assert_eq!(
        app.handle(click(tab.x, tab.y, MouseEventKind::Down(MouseButton::Left))),
        Action::Continue
    );
    assert_eq!(app.selected, selected);
    draw_app(&mut app, 24, 10);
    assert!(
        app.result_hits
            .iter()
            .all(|hit| hit.area.right() <= 24 && hit.area.bottom() <= 10)
    );
}

#[test]
fn sole_tab_supports_all_cycle_controls_and_a_separate_mouse_target() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(
            parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
            mode,
        );
        app.set_query("exact terminal".into());
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Tab("w3:t1".into()))
        );
        for event in [
            key(KeyCode::Tab),
            key(KeyCode::BackTab),
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT)),
        ] {
            app.handle(event);
            assert_eq!(
                app.handle(key(KeyCode::Enter)),
                Action::Switch(Target::Tab("w3:t1".into()))
            );
        }
        draw_app(&mut app, 80, 20);
        let tab = app
            .result_hits
            .iter()
            .find(|hit| hit.tab == Some(0))
            .unwrap()
            .area;
        assert_eq!(
            app.handle(click(tab.x, tab.y, MouseEventKind::Down(MouseButton::Left))),
            Action::Switch(Target::Tab("w3:t1".into()))
        );
        let workspace = app
            .result_hits
            .iter()
            .find(|hit| hit.tab.is_none())
            .unwrap()
            .area;
        assert_eq!(
            app.handle(click(
                workspace.x,
                workspace.y,
                MouseEventKind::Down(MouseButton::Left)
            )),
            Action::Switch(Target::Workspace("w3".into()))
        );
    }
}

#[test]
fn search_enter_activates_strongest_destination_in_both_modes() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        for (query, expected) in [
            ("server", Target::Tab("w2:t2".into())),
            ("pickercod", Target::Tab("w1:t8".into())),
            ("colorpicker", Target::Workspace("w1".into())),
            ("exact terminal", Target::Tab("w3:t1".into())),
        ] {
            let mut app = App::new(
                parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
                mode,
            );
            app.set_query(query.into());
            assert_eq!(app.handle(key(KeyCode::Enter)), Action::Switch(expected));
        }
    }
}

#[test]
fn workspace_preview_and_tab_navigation_start_from_remembered_tab() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(
            parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
            mode,
        );
        app.set_query("colorpicker".into());
        assert_eq!(app.selected_tab, None);
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Workspace("w1".into()))
        );
        app.handle(key(KeyCode::Tab));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Tab("w1:t9".into()))
        );
        app.set_query("colorpicker".into());
        app.handle(key(KeyCode::BackTab));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Tab("w1:t4".into()))
        );
    }
}

#[test]
fn nested_search_enter_activates_matching_tab_without_navigation() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(
            parse_snapshot(include_bytes!("fixtures/worktrees.json")).unwrap(),
            mode,
        );
        app.set_query("herdrpp multimachine codex".into());
        assert_eq!(app.selected, Some(0));
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Tab("w2:t1".into()))
        );
    }
}

#[test]
fn query_edits_replace_manual_tab_selection_and_clearing_restores_first_workspace() {
    let mut app = app();
    app.set_query("colorpicker".into());
    app.handle(key(KeyCode::Tab));
    app.handle(Event::Paste(" codex".into()));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t8".into()))
    );
    app.handle(key(KeyCode::Backspace));
    assert_eq!(app.query(), "colorpicker code");
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Tab("w1:t4".into()))
    );
    app.set_query("  ".into());
    assert_eq!(app.selected_tab, None);
    assert_eq!(app.highlighted_tab(), Some(0));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w2".into()))
    );
    app.set_query("nonexistenttttt".into());
    assert_eq!(app.highlighted_tab(), None);
    assert_eq!(app.selected_tab, None);
    assert_eq!(app.handle(key(KeyCode::Enter)), Action::Continue);
}

#[test]
fn initial_and_vertical_workspace_selection_preview_remembered_tabs() {
    let mut app = app();
    assert_eq!(app.highlighted_tab(), Some(1));
    assert_eq!(app.selected_tab, None);
    app.handle(key(KeyCode::Up));
    assert_eq!(app.highlighted_tab(), Some(0));
    app.handle(key(KeyCode::Tab));
    assert_eq!(app.highlighted_tab(), Some(1));
    app.handle(key(KeyCode::Down));
    assert_eq!(app.highlighted_tab(), Some(1));
    assert_eq!(app.selected_tab, None);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Action::Switch(Target::Workspace("w1".into()))
    );
}

#[test]
fn equal_tab_scores_keep_first_workspace_as_winner_without_name_bias() {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    snapshot.workspaces[2].label = "Other".into();
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(snapshot.clone(), mode);
        app.set_query("codex".into());
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Tab("w2:t7".into()))
        );
    }
}

#[test]
fn cursor_arrows_preserve_results_tab_selection_scroll_and_hit_targets() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(
            parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
            mode,
        );
        app.set_query("colorpicker".into());
        app.handle(key(KeyCode::Tab));
        draw_app(&mut app, 80, 20);
        let selected = app.selected;
        let tab = app.selected_tab;
        let offset = app.offset;
        let hits = app.result_hits.len();
        app.handle(key(KeyCode::Left));
        app.handle(key(KeyCode::Right));
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::SUPER,
        )));
        assert_eq!(app.input().cursor(), 0);
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::SUPER,
        )));
        assert_eq!(app.query(), "colorpicker");
        assert_eq!(app.input().cursor(), 11);
        assert_eq!(app.selected, selected);
        assert_eq!(app.selected_tab, tab);
        assert_eq!(app.offset, offset);
        assert_eq!(app.result_hits.len(), hits);
    }
}

#[test]
fn picker_backspace_inserts_and_pastes_at_cursor_in_both_modes() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        let mut app = App::new(
            parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
            mode,
        );
        app.set_query("colorpickxr".into());
        app.handle(key(KeyCode::Left));
        app.handle(key(KeyCode::Backspace));
        assert_eq!(app.query(), "colorpickr");
        app.handle(key(KeyCode::Char('e')));
        assert_eq!(app.query(), "colorpicker");
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Action::Switch(Target::Workspace("w1".into()))
        );
        app.handle(Event::Paste("\nX".into()));
        assert_eq!(app.query(), "colorpickeXr");
    }
}

#[test]
fn picker_clear_shortcuts_clear_every_cursor_position_and_restore_results() {
    for mode in [SearchMode::Hierarchical, SearchMode::Flat] {
        for (code, modifiers) in [
            (KeyCode::Backspace, KeyModifiers::SUPER),
            (KeyCode::Char('u'), KeyModifiers::CONTROL),
        ] {
            for lefts in [0, 2, 4] {
                let mut app = App::new(
                    parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
                    mode,
                );
                app.set_query("zzzz".into());
                for _ in 0..lefts {
                    app.handle(key(KeyCode::Left));
                }
                app.handle(Event::Key(KeyEvent::new(code, modifiers)));
                assert_eq!(app.query(), "");
                assert_eq!(app.input().cursor(), 0);
                assert_eq!(app.results.len(), 3);
                assert_eq!(app.selected, Some(0));
            }
        }
    }
}
