use ratatui::layout::Rect;
use terminal_input::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};

use crate::picker::{
    config::SearchMode,
    model::{Snapshot, Target},
    ordering,
    search::{Search, WorkspaceResult},
    theme::Theme,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Continue,
    Close,
    Switch(Target),
}

#[derive(Clone, Debug)]
pub struct ResultHit {
    pub area: Rect,
    pub workspace: usize,
    pub tab: Option<usize>,
}

pub struct App {
    pub snapshot: Snapshot,
    pub query: String,
    pub results: Vec<WorkspaceResult>,
    pub selected: Option<usize>,
    pub selected_tab: Option<usize>,
    pub result_hits: Vec<ResultHit>,
    pub offset: usize,
    pub results_area: Rect,
    pub mode: SearchMode,
    pub ok_area: Rect,
    pub cancel_area: Rect,
    pub picker_area: Option<Rect>,
    pub theme: Theme,
    search: Search,
}

impl App {
    pub fn new(snapshot: Snapshot, mode: SearchMode) -> Self {
        let mut search = Search::default();
        let results = search.results(&snapshot, "", mode);
        let initial = ordering::initial_target(&snapshot);
        let selected = results.iter().position(
            |r| matches!(&r.workspace.target, Target::Workspace(id) if Some(id.as_str()) == initial),
        );
        Self {
            snapshot,
            query: String::new(),
            results,
            selected,
            selected_tab: None,
            result_hits: Vec::new(),
            offset: 0,
            results_area: Rect::default(),
            ok_area: Rect::default(),
            cancel_area: Rect::default(),
            picker_area: None,
            theme: Theme::default(),
            mode,
            search,
        }
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.results = self.search.results(&self.snapshot, &self.query, self.mode);
        self.selected = (!self.results.is_empty()).then_some(0);
        self.selected_tab = None;
        self.result_hits.clear();
        self.offset = 0;
    }

    pub fn navigate(&mut self, delta: isize) {
        if self.results.is_empty() {
            self.selected = None;
            self.selected_tab = None;
            return;
        }
        let current = self.selected.unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(self.results.len() - 1);
        if next != current {
            self.selected = Some(next);
            self.selected_tab = None;
        }
    }

    fn navigate_tab(&mut self, delta: isize) {
        let Some(result) = self.selected.and_then(|index| self.results.get(index)) else {
            return;
        };
        let count = result.tabs.len();
        if count == 0 {
            return;
        }
        self.selected_tab = Some(match self.selected_tab {
            Some(index) => (index as isize + delta).rem_euclid(count as isize) as usize,
            None if delta > 0 => 0,
            None => count - 1,
        });
    }

    pub fn ensure_visible(&mut self, height: usize) {
        if let Some(selected) = self.selected {
            if selected < self.offset {
                self.offset = selected;
            }
            if height > 0 && selected >= self.offset + height {
                self.offset = selected + 1 - height;
            }
        }
        self.offset = self.offset.min(self.results.len().saturating_sub(height));
    }

    fn activate(&self) -> Action {
        self.selected
            .and_then(|index| self.results.get(index))
            .map(|result| {
                self.selected_tab
                    .and_then(|index| result.tabs.get(index))
                    .unwrap_or(&result.workspace)
            })
            .map(|row| Action::Switch(row.target.clone()))
            .unwrap_or(Action::Continue)
    }

    pub fn handle(&mut self, event: Event) -> Action {
        match event {
            Event::ThemeModeChanged(appearance) => self.theme.set_appearance(appearance),
            Event::Resize(_, _) => {
                self.result_hits.clear();
                self.results_area = Rect::default();
                self.ok_area = Rect::default();
                self.cancel_area = Rect::default();
                self.picker_area = None;
            }
            Event::Mouse(mouse) if is_backdrop_click(self.picker_area, mouse) => {
                return Action::Close;
            }
            Event::Mouse(mouse)
                if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                    && self.cancel_area.contains((mouse.column, mouse.row).into()) =>
            {
                return Action::Close;
            }
            Event::Mouse(mouse)
                if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                    && self.ok_area.contains((mouse.column, mouse.row).into()) =>
            {
                return self.activate();
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Esc => return Action::Close,
                    KeyCode::Char('c') if ctrl => return Action::Close,
                    KeyCode::Up | KeyCode::Char('p') if key.code == KeyCode::Up || ctrl => {
                        self.navigate(-1)
                    }
                    KeyCode::Down | KeyCode::Char('n') if key.code == KeyCode::Down || ctrl => {
                        self.navigate(1)
                    }
                    KeyCode::BackTab => self.navigate_tab(-1),
                    KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        self.navigate_tab(-1)
                    }
                    KeyCode::Tab => self.navigate_tab(1),
                    KeyCode::Left => self.navigate_tab(-1),
                    KeyCode::Right => self.navigate_tab(1),
                    KeyCode::Enter => return self.activate(),
                    KeyCode::Backspace => {
                        let mut query = self.query.clone();
                        query.pop();
                        self.set_query(query);
                    }
                    KeyCode::Char(ch)
                        if !key.modifiers.intersects(
                            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                        ) =>
                    {
                        self.set_query(format!("{}{ch}", self.query))
                    }
                    _ => {}
                }
            }
            Event::Paste(text) => self.set_query(format!(
                "{}{}",
                self.query,
                text.chars().filter(|c| !c.is_control()).collect::<String>()
            )),
            Event::Mouse(mouse) if self.results_area.contains((mouse.column, mouse.row).into()) => {
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(hit) = self
                            .result_hits
                            .iter()
                            .find(|hit| hit.area.contains((mouse.column, mouse.row).into()))
                        {
                            self.selected = Some(hit.workspace);
                            self.selected_tab = hit.tab;
                            return self.activate();
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        self.selected_tab = None;
                        self.navigate(3);
                    }
                    MouseEventKind::ScrollUp => {
                        self.selected_tab = None;
                        self.navigate(-3);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Action::Continue
    }
}

pub(crate) fn is_backdrop_click(
    area: Option<Rect>,
    mouse: terminal_input::event::MouseEvent,
) -> bool {
    mouse.kind == MouseEventKind::Down(MouseButton::Left)
        && area.is_some_and(|area| !area.contains((mouse.column, mouse.row).into()))
}
