use terminal_input::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextInput {
    text: String,
    cursor: usize,
    selected: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum InputChange {
    Unhandled,
    Cursor,
    Edited,
}

impl TextInput {
    pub fn new(text: String, selected: bool) -> Self {
        Self {
            cursor: text.len(),
            text,
            selected,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn selected(&self) -> bool {
        self.selected
    }

    fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.selected = false;
    }

    fn insert(&mut self, text: &str) -> InputChange {
        let text: String = text.chars().filter(|ch| !ch.is_control()).collect();
        if text.is_empty() {
            return InputChange::Unhandled;
        }
        if self.selected {
            self.clear();
        }
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
        InputChange::Edited
    }

    pub fn handle(&mut self, event: &Event) -> InputChange {
        match event {
            Event::Paste(text) => self.insert(text),
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                _ if is_clear_shortcut(key) => {
                    self.clear();
                    InputChange::Edited
                }
                KeyCode::Left => {
                    self.cursor = if self.selected {
                        0
                    } else {
                        self.text[..self.cursor]
                            .char_indices()
                            .next_back()
                            .map_or(0, |(i, _)| i)
                    };
                    self.selected = false;
                    InputChange::Cursor
                }
                KeyCode::Right => {
                    self.cursor = if self.selected {
                        self.text.len()
                    } else {
                        self.cursor
                            + self.text[self.cursor..]
                                .chars()
                                .next()
                                .map_or(0, char::len_utf8)
                    };
                    self.selected = false;
                    InputChange::Cursor
                }
                KeyCode::Backspace => {
                    if self.selected {
                        self.clear();
                    } else if let Some((start, _)) =
                        self.text[..self.cursor].char_indices().next_back()
                    {
                        self.text.drain(start..self.cursor);
                        self.cursor = start;
                    }
                    InputChange::Edited
                }
                KeyCode::Char(ch)
                    if !key.modifiers.intersects(
                        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                    ) =>
                {
                    self.insert(&ch.to_string())
                }
                _ => InputChange::Unhandled,
            },
            _ => InputChange::Unhandled,
        }
    }
}

fn is_clear_shortcut(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Backspace => key.modifiers.contains(KeyModifiers::SUPER),
        KeyCode::Char('u') => key.modifiers == KeyModifiers::CONTROL,
        _ => false,
    }
}
