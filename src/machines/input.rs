use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Bytes(Vec<u8>),
    Picker,
    Quit,
}

#[derive(Default)]
pub struct Input {
    pending: Vec<u8>,
    bindings: super::shortcuts::Bindings,
    paste: bool,
    opaque: Option<ControlString>,
    since: Option<Instant>,
}

struct ControlString {
    osc: bool,
    escaped: bool,
}

const START_PASTE: &[u8] = b"\x1b[200~";
const END_PASTE: &[u8] = b"\x1b[201~";

impl Input {
    pub fn configured() -> anyhow::Result<Self> {
        Ok(Self::with_bindings(super::shortcuts::Bindings::load()?))
    }
    pub fn with_bindings(bindings: super::shortcuts::Bindings) -> Self {
        Self {
            bindings,
            ..Self::default()
        }
    }
    pub fn reset(&mut self) {
        let bindings = self.bindings.clone();
        *self = Self {
            bindings,
            ..Self::default()
        };
    }

    pub fn byte(&mut self, byte: u8) -> Vec<Action> {
        if let Some(string) = self.opaque.as_mut() {
            let complete = (string.osc && byte == 7)
                || (string.escaped && byte == b'\\')
                || matches!(byte, 0x18 | 0x1a);
            string.escaped = byte == 0x1b;
            if complete {
                self.opaque = None;
            }
            return vec![Action::Bytes(vec![byte])];
        }
        self.pending.push(byte);
        self.since = Some(Instant::now());
        if self.paste {
            let mut literal = Vec::new();
            while !self.pending.is_empty() {
                if self.pending == END_PASTE {
                    literal.append(&mut self.pending);
                    self.paste = false;
                    break;
                }
                if END_PASTE.starts_with(&self.pending) {
                    break;
                }
                literal.push(self.pending.remove(0));
            }
            return self.forward(literal);
        }
        if !self.paste
            && self.bindings.0.iter().any(|(sequence, _)| {
                sequence.len() > self.pending.len() && sequence.starts_with(&self.pending)
            })
        {
            return Vec::new();
        }
        if let Some(prefix) =
            self.bindings.1.iter().find(|prefix| {
                self.pending.starts_with(prefix) && self.pending.len() > prefix.len()
            })
        {
            let tail = &self.pending[prefix.len()..];
            if let Some((key, modifiers, 3)) = report(tail) {
                let prefix_key = report(prefix).map(|(key, modifiers, _)| (key, modifiers));
                if prefix_key == Some((key, modifiers))
                    || (modifiers & !0xc0 == 4 && key < 128 && prefix == &[(key as u8) & 31])
                {
                    self.pending.truncate(prefix.len());
                    return Vec::new();
                }
            }
            if tail == b"\x1b"
                || (tail.starts_with(b"\x1b[")
                    && (tail.len() == 2 || !(0x40..=0x7e).contains(&byte)))
            {
                return Vec::new();
            }
        }
        if self.pending == [0x1b] {
            return Vec::new();
        }
        if self.pending.starts_with(b"\x1b[") {
            if self.pending.len() == 2 {
                return Vec::new();
            }
            if !(0x40..=0x7e).contains(&byte) && !matches!(byte, 0..=0x1f | 0x7f) {
                return Vec::new();
            }
        } else if self.pending.len() == 2 && self.pending[0] == 0x1b {
            match byte {
                b']' | b'P' | b'_' | b'^' | b'X' => return Vec::new(),
                b'O' | 0x20..=0x2f => return Vec::new(),
                _ => {}
            }
        } else if self.pending.len() == 3
            && self.pending[0] == 0x1b
            && matches!(self.pending[1], b']' | b'P' | b'_' | b'^' | b'X')
        {
            let osc = self.pending[1] == b']';
            if !(osc && byte == 7) && !matches!(byte, 0x18 | 0x1a) {
                self.opaque = Some(ControlString {
                    osc,
                    escaped: byte == 0x1b,
                });
            }
        } else if self.pending.starts_with(b"\x1b")
            && self
                .pending
                .get(1)
                .is_some_and(|byte| (0x20..=0x2f).contains(byte))
            && (0x20..=0x2f).contains(&byte)
        {
            return Vec::new();
        }
        if self.pending == START_PASTE {
            self.paste = true;
        }
        if let Some(prefix) = self
            .bindings
            .1
            .iter()
            .find(|prefix| self.pending.starts_with(prefix))
            && matches!(
                shortcut(&self.pending[prefix.len()..]),
                Some(Shortcut::Quit)
            )
        {
            self.pending.clear();
            self.since = None;
            return vec![Action::Quit];
        }
        let canonical = report(&self.pending).map(|(key, modifiers, event)| {
            format!("\x1b[{key};{}:{event}u", modifiers + 1).into_bytes()
        });
        if !self.paste {
            if let Some((_, release)) = self.bindings.0.iter().find(|(sequence, _)| {
                sequence == &self.pending || canonical.as_ref() == Some(sequence)
            }) {
                let release = *release;
                self.pending.clear();
                self.since = None;
                return if release {
                    Vec::new()
                } else {
                    vec![Action::Picker]
                };
            }
            if self
                .bindings
                .0
                .iter()
                .any(|(sequence, _)| sequence.starts_with(&self.pending))
            {
                return Vec::new();
            }
        }
        if let Some(shortcut) = shortcut(&self.pending) {
            self.pending.clear();
            self.since = None;
            return match shortcut {
                Shortcut::Picker => vec![Action::Picker],
                Shortcut::Quit => vec![Action::Quit],
                Shortcut::Release => Vec::new(),
            };
        }
        let literal = std::mem::take(&mut self.pending);
        self.forward(literal)
    }

    fn forward(&mut self, literal: Vec<u8>) -> Vec<Action> {
        if self.pending.is_empty() {
            self.since = None;
        }
        if literal.is_empty() {
            Vec::new()
        } else {
            vec![Action::Bytes(literal)]
        }
    }

    pub fn flush(&mut self) -> Option<Action> {
        self.flush_at(Instant::now())
    }
    fn flush_at(&mut self, now: Instant) -> Option<Action> {
        if !self.paste
            && !self.bindings.1.contains(&self.pending)
            && self.since.is_some_and(|since| {
                now.saturating_duration_since(since) >= Duration::from_millis(100)
            })
        {
            self.since = None;
            Some(Action::Bytes(std::mem::take(&mut self.pending)))
        } else {
            None
        }
    }
}

enum Shortcut {
    Picker,
    Quit,
    Release,
}

fn number(value: &str) -> Option<u32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn codepoint(value: &str) -> Option<u32> {
    let value = number(value)?;
    char::from_u32(value).map(|_| value)
}

fn shortcut(bytes: &[u8]) -> Option<Shortcut> {
    match bytes {
        b"\x1d" => return Some(Shortcut::Picker),
        b"\x1c" => return Some(Shortcut::Quit),
        _ => {}
    }
    let (key, modifiers, event) = report(bytes)?;
    if modifiers & !0xc0 != 4 || !matches!(event, 1..=3) {
        return None;
    }
    match (key, event) {
        (92 | 93, 3) => Some(Shortcut::Release),
        (93, _) => Some(Shortcut::Picker),
        (92, _) => Some(Shortcut::Quit),
        _ => None,
    }
}

fn report(bytes: &[u8]) -> Option<(u32, u16, u8)> {
    let sequence = std::str::from_utf8(bytes.strip_prefix(b"\x1b[")?).ok()?;
    let (key, modifiers, event) = if let Some(body) = sequence.strip_suffix('u') {
        let fields = body.split(';').collect::<Vec<_>>();
        if fields.len() > 3 {
            return None;
        }
        let codes = fields.first()?.split(':').collect::<Vec<_>>();
        if codes.len() > 3 {
            return None;
        }
        let primary = codepoint(codes[0])?;
        for alternate in codes.iter().skip(1).filter(|value| !value.is_empty()) {
            codepoint(alternate)?;
        }
        let key = match codes.get(2).filter(|value| !value.is_empty()) {
            Some(base) => codepoint(base)?,
            None => primary,
        };
        let modifier = fields
            .get(1)
            .copied()
            .unwrap_or("1")
            .split(':')
            .collect::<Vec<_>>();
        if modifier.len() > 2 {
            return None;
        }
        let modifiers = u16::try_from(number(modifier[0])?).ok()?.checked_sub(1)?;
        let event = u8::try_from(number(modifier.get(1).copied().unwrap_or("1"))?).ok()?;
        if let Some(text) = fields.get(2) {
            if text.is_empty() {
                return None;
            }
            for character in text.split(':') {
                codepoint(character)?;
            }
        }
        (key, modifiers, event)
    } else {
        let body = sequence.strip_suffix('~')?;
        let fields = body.split(';').collect::<Vec<_>>();
        if fields.len() != 3 || fields[0] != "27" {
            return None;
        }
        (
            codepoint(fields[2])?,
            u16::try_from(number(fields[1])?).ok()?.checked_sub(1)?,
            1,
        )
    };
    Some((key, modifiers, event))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_configured_prefix_remains_owned_until_the_following_key() {
        let bindings = super::super::shortcuts::Bindings::parse(
            "[[keys.command]]\nkey='prefix+f'\ntype='plugin_action'\ncommand='herdrpp.open'\n",
        )
        .unwrap();
        let mut input = Input::with_bindings(bindings);
        assert!(input.byte(2).is_empty());
        assert!(
            input
                .flush_at(Instant::now() + Duration::from_secs(10))
                .is_none()
        );
        assert_eq!(input.byte(b'f'), vec![Action::Picker]);
    }
}
