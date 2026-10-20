use super::{
    catalog::{self, Command, Context, Hit, Item},
    naming::Naming,
};
use crate::search::input::{InputChange, TextInput};
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
    Name {
        item: usize,
        input: TextInput,
        error: bool,
    },
    Confirm {
        item: usize,
    },
}

pub struct App {
    pub tasks: bool,
    pub diagnostics_offset: u16,
    pub diagnostics_max: u16,
    pub context: Context,
    pub naming: Naming,
    pub items: Vec<Item>,
    input: TextInput,
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
    recent_ids: Vec<String>,
    matcher: FieldMatcher,
}

impl App {
    pub fn new(
        context: Context,
        items: Vec<Item>,
        naming: Naming,
        theme: Theme,
        warning: Option<String>,
    ) -> Self {
        let mut app = Self {
            tasks: false,
            diagnostics_offset: 0,
            diagnostics_max: 0,
            context,
            naming,
            items,
            input: TextInput::default(),
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
            recent_ids: vec![],
            matcher: FieldMatcher::default(),
        };
        app.set_query(String::new());
        app
    }
    pub fn set_recent_ids(&mut self, recent_ids: Vec<String>) {
        self.recent_ids = recent_ids;
        self.refresh_results();
    }
    pub fn set_query(&mut self, text: String) {
        self.input = TextInput::new(text, false);
        self.refresh_results();
    }
    pub fn query(&self) -> &str {
        self.input.text()
    }
    pub fn input(&self) -> &TextInput {
        &self.input
    }
    fn refresh_results(&mut self) {
        self.results = catalog::search_with_recent(
            &self.items,
            self.input.text(),
            &mut self.matcher,
            &self.recent_ids,
        );
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
            Mode::Name {
                item, input, error, ..
            } => {
                if matches!(self.items[*item].command, Command::Rename(_))
                    && input.text().trim().is_empty()
                {
                    *error = true;
                    return Action::Continue;
                }
                Action::Execute(Selection {
                    id: self.items[*item].id.clone(),
                    input: Some(input.text().to_owned()),
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
                    Command::Rename(scope) | Command::Create(scope) => {
                        let creating = matches!(self.items[item].command, Command::Create(_));
                        self.mode = Mode::Name {
                            item,
                            input: TextInput::new(
                                if creating {
                                    self.naming.suggestion(scope)
                                } else {
                                    self.naming.rename_name(scope)
                                }
                                .to_owned(),
                                creating,
                            ),
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
    fn edit_input(&mut self, event: &Event) {
        match &mut self.mode {
            Mode::Search => {
                if self.input.handle(event) == InputChange::Edited {
                    self.refresh_results();
                }
            }
            Mode::Name { input, error, .. } => {
                if input.handle(event) == InputChange::Edited {
                    *error = false;
                }
            }
            Mode::Confirm { .. } => {}
        }
    }
    pub fn handle(&mut self, event: Event) -> Action {
        if self.tasks && self.warning.is_some() {
            match event {
                Event::ThemeModeChanged(appearance) => self.theme.set_appearance(appearance),
                Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                    KeyCode::Esc => return Action::Cancel,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Action::Cancel;
                    }
                    KeyCode::Down | KeyCode::PageDown => {
                        self.diagnostics_offset = self
                            .diagnostics_offset
                            .saturating_add(if key.code == KeyCode::PageDown { 10 } else { 1 })
                            .min(self.diagnostics_max)
                    }
                    KeyCode::Up | KeyCode::PageUp => {
                        self.diagnostics_offset = self
                            .diagnostics_offset
                            .saturating_sub(if key.code == KeyCode::PageUp { 10 } else { 1 })
                    }
                    KeyCode::Home => self.diagnostics_offset = 0,
                    KeyCode::End => self.diagnostics_offset = self.diagnostics_max,
                    _ => {}
                },
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        self.diagnostics_offset = self
                            .diagnostics_offset
                            .saturating_add(3)
                            .min(self.diagnostics_max)
                    }
                    MouseEventKind::ScrollUp => {
                        self.diagnostics_offset = self.diagnostics_offset.saturating_sub(3)
                    }
                    _ => {}
                },
                _ => {}
            }
            return Action::Continue;
        }
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
                    _ => self.edit_input(&Event::Key(key)),
                }
            }
            event @ Event::Paste(_) => self.edit_input(&event),
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
