use herdrpp::picker::{
    api::parse_snapshot,
    app::{Action, App},
    config::SearchMode,
    ui,
};
use ratatui::{Terminal, backend::TestBackend};
use terminal_input::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

fn rendered_app(width: u16, height: u16) -> App {
    let mut app = App::new(
        parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap(),
        SearchMode::Hierarchical,
    );
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::render(frame, &mut app)).unwrap();
    app
}

fn mouse(column: u16, row: u16, kind: MouseEventKind) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn backdrop_left_click_cancels_without_changing_selection() {
    let mut app = rendered_app(100, 40);
    let selected = app.selected;
    assert_eq!(
        app.handle(mouse(0, 0, MouseEventKind::Down(MouseButton::Left))),
        Action::Close
    );
    assert_eq!(app.selected, selected);
}

#[test]
fn backdrop_right_click_scroll_and_drag_do_not_cancel() {
    let mut app = rendered_app(100, 40);
    assert_eq!(
        app.handle(mouse(0, 0, MouseEventKind::Down(MouseButton::Right))),
        Action::Continue
    );
    assert_eq!(
        app.handle(mouse(0, 0, MouseEventKind::ScrollDown)),
        Action::Continue
    );
    assert_eq!(
        app.handle(mouse(0, 0, MouseEventKind::Drag(MouseButton::Left))),
        Action::Continue
    );
    assert_eq!(
        app.handle(mouse(0, 0, MouseEventKind::Up(MouseButton::Left))),
        Action::Continue
    );
}

#[test]
fn blank_interior_and_border_clicks_do_not_cancel() {
    let mut app = rendered_app(100, 40);
    assert_eq!(
        app.handle(mouse(10, 9, MouseEventKind::Down(MouseButton::Left))),
        Action::Continue
    );
    assert_eq!(
        app.handle(mouse(50, 25, MouseEventKind::Down(MouseButton::Left))),
        Action::Continue
    );
}

#[test]
fn resize_ignores_stale_bounds_then_uses_redrawn_bounds() {
    let mut app = rendered_app(100, 40);
    assert_eq!(app.handle(Event::Resize(60, 30)), Action::Continue);
    assert_eq!(
        app.handle(mouse(10, 10, MouseEventKind::Down(MouseButton::Left))),
        Action::Continue
    );
    let mut terminal = Terminal::new(TestBackend::new(60, 30)).unwrap();
    terminal.draw(|frame| ui::render(frame, &mut app)).unwrap();
    assert_eq!(
        app.handle(mouse(59, 29, MouseEventKind::Down(MouseButton::Left))),
        Action::Close
    );
}

#[test]
fn every_backdrop_edge_cancels_and_panel_edges_stay_open() {
    let mut app = rendered_app(100, 40);
    assert_eq!(
        app.handle(mouse(9, 20, MouseEventKind::Down(MouseButton::Left))),
        Action::Close
    );
    assert_eq!(
        app.handle(mouse(90, 20, MouseEventKind::Down(MouseButton::Left))),
        Action::Close
    );
    assert_eq!(
        app.handle(mouse(50, 8, MouseEventKind::Down(MouseButton::Left))),
        Action::Close
    );
    assert_eq!(
        app.handle(mouse(50, 31, MouseEventKind::Down(MouseButton::Left))),
        Action::Close
    );
    assert_eq!(
        app.handle(mouse(89, 30, MouseEventKind::Down(MouseButton::Left))),
        Action::Continue
    );
}

#[test]
fn centered_result_and_button_targets_still_work() {
    let mut app = rendered_app(100, 40);
    let hit = app.result_hits[0].clone();
    let target = app.results[hit.workspace].workspace.target.clone();
    assert_eq!(
        app.handle(mouse(
            hit.area.x,
            hit.area.y,
            MouseEventKind::Down(MouseButton::Left)
        )),
        Action::Switch(target.clone())
    );
    let ok = app.ok_area;
    assert_eq!(
        app.handle(mouse(ok.x, ok.y, MouseEventKind::Down(MouseButton::Left))),
        Action::Switch(target)
    );
    let cancel = app.cancel_area;
    assert_eq!(
        app.handle(mouse(
            cancel.x,
            cancel.y,
            MouseEventKind::Down(MouseButton::Left)
        )),
        Action::Close
    );
}

#[test]
fn loading_and_ready_panels_share_bounds_and_cancel_position() {
    let app = rendered_app(100, 40);
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    let mut loading = ui::LoadingAreas::default();
    terminal
        .draw(|frame| loading = ui::render_loading(frame, app.theme.palette()))
        .unwrap();
    assert_eq!(Some(loading.picker), app.picker_area);
    assert_eq!(loading.cancel, app.cancel_area);
    assert_eq!(loading.picker, ratatui::layout::Rect::new(10, 9, 80, 22));
    assert_eq!(
        terminal.backend().buffer()[(0, 0)].bg,
        app.theme.palette().sidebar_bg
    );
    assert_eq!(
        terminal.backend().buffer()[(11, 10)].bg,
        app.theme.palette().panel_bg
    );
}

#[test]
fn tiny_terminal_renders_escape_guidance_without_stale_hit_targets() {
    let mut app = rendered_app(100, 40);
    let mut terminal = Terminal::new(TestBackend::new(6, 1)).unwrap();
    terminal.draw(|frame| ui::render(frame, &mut app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert_eq!(text, "Esc cl");
    assert!(app.result_hits.is_empty());
    assert!(app.cancel_area.is_empty());
    assert_eq!(
        app.picker_area,
        Some(ratatui::layout::Rect::new(0, 0, 6, 1))
    );
}
