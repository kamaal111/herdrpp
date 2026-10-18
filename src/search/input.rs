use terminal_input::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) fn is_clear_shortcut(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Backspace => key.modifiers.contains(KeyModifiers::SUPER),
        KeyCode::Char('u') => key.modifiers == KeyModifiers::CONTROL,
        _ => false,
    }
}
