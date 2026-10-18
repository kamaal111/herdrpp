use super::catalog::{self, Command, Context, Hit, Item};
use crate::{picker::theme::Theme, search::matcher::FieldMatcher};
use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};
use terminal_input::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Selection {
    pub id: String,
    pub input: Option<String>,
    pub confirmed: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Continue,
    Cancel,
    Execute(Selection),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    Search,
    Rename {
        item: usize,
        input: String,
        error: bool,
    },
    Confirm {
        item: usize,
    },
}

pub struct App {
    pub context: Context,
    pub items: Vec<Item>,
    pub query: String,
    pub results: Vec<Hit>,
    pub selected: Option<usize>,
    pub offset: usize,
    pub mode: Mode,
    pub theme: Theme,
    pub warning: Option<String>,
    pub results_area: Rect,
    pub hits: Vec<(Rect, usize)>,
    pub ok_area: Rect,
    pub cancel_area: Rect,
    matcher: FieldMatcher,
}

impl App {
    pub fn new(context: Context, items: Vec<Item>, theme: Theme, warning: Option<String>) -> Self {
        let mut app = Self {
            context,
            items,
            query: String::new(),
            results: vec![],
            selected: None,
            offset: 0,
            mode: Mode::Search,
            theme,
            warning,
            results_area: Rect::default(),
            hits: vec![],
            ok_area: Rect::default(),
            cancel_area: Rect::default(),
            matcher: FieldMatcher::default(),
        };
        app.set_query(String::new());
        app
    }
    pub fn set_query(&mut self, text: String) {
        self.query = text;
        self.results = catalog::search(&self.items, &self.query, &mut self.matcher);
        self.selected = (!self.results.is_empty()).then_some(0);
        self.offset = 0;
        self.hits.clear();
    }
    pub fn navigate(&mut self, delta: isize) {
        self.selected = self.selected.map(|index| {
            index
                .saturating_add_signed(delta)
                .min(self.results.len().saturating_sub(1))
        });
    }
    pub fn ensure_visible(&mut self, height: usize) {
        self.offset = crate::search::view::visible_offset(
            self.selected,
            self.offset,
            self.results.len(),
            height,
        );
    }
    pub fn clear_hits(&mut self) {
        self.hits.clear();
        self.results_area = Rect::default();
        self.ok_area = Rect::default();
        self.cancel_area = Rect::default();
    }
    fn cancel(&mut self) -> Action {
        if self.mode == Mode::Search {
            Action::Cancel
        } else {
            self.mode = Mode::Search;
            self.clear_hits();
            Action::Continue
        }
    }
    fn activate(&mut self) -> Action {
        match &mut self.mode {
            Mode::Rename { item, input, error } => {
                if input.trim().is_empty() {
                    *error = true;
                    return Action::Continue;
                }
                Action::Execute(Selection {
                    id: self.items[*item].id.clone(),
                    input: Some(input.clone()),
                    confirmed: false,
                })
            }
            Mode::Confirm { item } => Action::Execute(Selection {
                id: self.items[*item].id.clone(),
                input: None,
                confirmed: true,
            }),
            Mode::Search => {
                let Some(item) = self
                    .selected
                    .and_then(|index| self.results.get(index))
                    .map(|hit| hit.item)
                else {
                    return Action::Continue;
                };
                match self.items[item].command {
                    Command::Rename(_) => {
                        self.mode = Mode::Rename {
                            item,
                            input: String::new(),
                            error: false,
                        };
                        self.clear_hits();
                        Action::Continue
                    }
                    Command::Close(_) => {
                        self.mode = Mode::Confirm { item };
                        self.clear_hits();
                        Action::Continue
                    }
                    _ => Action::Execute(Selection {
                        id: self.items[item].id.clone(),
                        input: None,
                        confirmed: false,
                    }),
                }
            }
        }
    }
    fn clear_input(&mut self) {
        match &mut self.mode {
            Mode::Search => self.set_query(String::new()),
            Mode::Rename { input, error, .. } => {
                input.clear();
                *error = false;
            }
            Mode::Confirm { .. } => {}
        }
    }
    fn edit(&mut self, text: Option<&str>) {
        match &mut self.mode {
            Mode::Search => {
                let mut query = self.query.clone();
                if let Some(text) = text {
                    query.push_str(text);
                } else {
                    query.pop();
                }
                self.set_query(query);
            }
            Mode::Rename { input, error, .. } => {
                if let Some(text) = text {
                    input.push_str(text);
                } else {
                    input.pop();
                }
                *error = false;
            }
            Mode::Confirm { .. } => {}
        }
    }
    pub fn handle(&mut self, event: Event) -> Action {
        match event {
            Event::Resize(_, _) => self.clear_hits(),
            Event::ThemeModeChanged(appearance) => self.theme.set_appearance(appearance),
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Char('c') if ctrl => return Action::Cancel,
                    KeyCode::Esc => return self.cancel(),
                    KeyCode::Enter => return self.activate(),
                    KeyCode::Up | KeyCode::Char('p')
                        if (key.code == KeyCode::Up || ctrl) && self.mode == Mode::Search =>
                    {
                        self.navigate(-1)
                    }
                    KeyCode::Down | KeyCode::Char('n')
                        if (key.code == KeyCode::Down || ctrl) && self.mode == Mode::Search =>
                    {
                        self.navigate(1)
                    }
                    _ if crate::search::input::is_clear_shortcut(&key) => self.clear_input(),
                    KeyCode::Backspace => self.edit(None),
                    KeyCode::Char(ch)
                        if !key.modifiers.intersects(
                            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                        ) =>
                    {
                        self.edit(Some(&ch.to_string()))
                    }
                    _ => {}
                }
            }
            Event::Paste(text) => self.edit(Some(
                &text
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .collect::<String>(),
            )),
            Event::Mouse(mouse) => {
                let point = (mouse.column, mouse.row).into();
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) if self.cancel_area.contains(point) => {
                        return self.cancel();
                    }
                    MouseEventKind::Down(MouseButton::Left) if self.ok_area.contains(point) => {
                        return self.activate();
                    }
                    MouseEventKind::Down(MouseButton::Left) if self.mode == Mode::Search => {
                        if let Some((_, selected)) =
                            self.hits.iter().find(|(area, _)| area.contains(point))
                        {
                            self.selected = Some(*selected);
                            return self.activate();
                        }
                    }
                    MouseEventKind::ScrollDown
                        if self.mode == Mode::Search && self.results_area.contains(point) =>
                    {
                        self.navigate(3)
                    }
                    MouseEventKind::ScrollUp
                        if self.mode == Mode::Search && self.results_area.contains(point) =>
                    {
                        self.navigate(-3)
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Action::Continue
    }
}
