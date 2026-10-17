//! Keep typeahead in the parent until the selector acknowledges a closing key.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::output::Output;

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "event", content = "id", rename_all = "snake_case")]
pub enum PickerEvent {
    Continue,
    Selected(Selection),
    Cancelled,
}

#[derive(Debug, PartialEq, Eq)]
pub enum InputChunk {
    Keyboard(Vec<u8>),
    Literal(Vec<u8>),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Selection {
    pub machine: String,
    pub profile: Option<super::catalog::Profile>,
    pub target: crate::picker::model::Target,
    pub query: String,
}

pub struct PickerChoice {
    pub selection: Option<Selection>,
    pub queued: Vec<InputChunk>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClosingKey {
    Submit,
    Cancel,
}

#[derive(Default)]
pub struct Handoff {
    stream: Output,
    paste: bool,
    closing: Option<ClosingKey>,
    queued: Vec<u8>,
    queued_timeouts: Vec<usize>,
    queued_literals: Vec<std::ops::Range<usize>>,
}

impl Handoff {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<u8>> {
        let mut forwarded = Vec::new();
        for byte in bytes {
            if self.closing.is_some() {
                self.queued.push(*byte);
                continue;
            }
            let mut token = Vec::new();
            self.stream.forward(&[*byte], &mut token)?;
            forwarded.push(*byte);
            self.observe(&token);
        }
        Ok(forwarded)
    }

    pub fn timeout(&mut self, bytes: &[u8]) -> Result<Vec<u8>> {
        let was_closing = self.closing.is_some();
        let start = self.queued.len();
        let forwarded = self.feed(bytes)?;
        if was_closing {
            self.queued_literals.push(start..self.queued.len());
            self.queued_timeouts.push(self.queued.len());
            return Ok(forwarded);
        }
        if self.stream.pending_len() > 0 {
            let token = self.stream.flush_input_pending()?;
            self.observe(&token);
        }
        Ok(forwarded)
    }

    fn observe(&mut self, token: &[u8]) {
        if token == b"\x1b[200~" {
            self.paste = true;
        } else if token == b"\x1b[201~" {
            self.paste = false;
        } else if !self.paste {
            self.closing = closing_key(token);
        }
    }

    pub fn acknowledge(&mut self, event: &PickerEvent) -> Result<Vec<u8>> {
        match (self.closing, event) {
            (Some(ClosingKey::Submit), PickerEvent::Continue) => {
                self.closing = None;
                let pending = std::mem::take(&mut self.queued);
                let timeouts = std::mem::take(&mut self.queued_timeouts);
                self.queued_literals.clear();
                let mut forwarded = Vec::new();
                let mut start = 0;
                for end in timeouts {
                    forwarded.extend(self.timeout(&pending[start..end])?);
                    start = end;
                }
                forwarded.extend(self.feed(&pending[start..])?);
                Ok(forwarded)
            }
            (Some(ClosingKey::Submit), PickerEvent::Selected(_))
            | (Some(ClosingKey::Cancel | ClosingKey::Submit), PickerEvent::Cancelled) => {
                Ok(Vec::new())
            }
            _ => bail!("machine picker returned an unexpected input acknowledgement"),
        }
    }

    pub fn finish(&mut self, selection: Option<Selection>) -> PickerChoice {
        let mut chunks = Vec::new();
        let mut start = 0;
        for literal in std::mem::take(&mut self.queued_literals) {
            if start < literal.start {
                chunks.push(InputChunk::Keyboard(
                    self.queued[start..literal.start].to_vec(),
                ));
            }
            if !literal.is_empty() {
                chunks.push(InputChunk::Literal(self.queued[literal.clone()].to_vec()));
            }
            start = literal.end;
        }
        if start < self.queued.len() {
            chunks.push(InputChunk::Keyboard(self.queued[start..].to_vec()));
        }
        self.queued.clear();
        self.queued_timeouts.clear();
        PickerChoice {
            selection,
            queued: chunks,
        }
    }

    pub fn take_queued(&mut self) -> Vec<u8> {
        self.queued_timeouts.clear();
        self.queued_literals.clear();
        std::mem::take(&mut self.queued)
    }
}

fn closing_key(token: &[u8]) -> Option<ClosingKey> {
    match token {
        b"\r" => return Some(ClosingKey::Submit),
        b"\x1b" | b"\x03" => return Some(ClosingKey::Cancel),
        _ => {}
    }
    if token.starts_with(b"\x1b") && !token.starts_with(b"\x1b[") {
        return closing_key(&token[1..]);
    }
    if let Some(mouse) = token
        .strip_prefix(b"\x1b[<")
        .and_then(|bytes| bytes.strip_suffix(b"M"))
    {
        let button = std::str::from_utf8(mouse)
            .ok()?
            .split(';')
            .next()?
            .parse::<u8>()
            .ok()?;
        return (button & 0b1110_0011 == 0).then_some(ClosingKey::Submit);
    }
    let sequence = token.strip_prefix(b"\x1b[")?;
    let body = sequence.strip_suffix(b"u")?;
    let text = std::str::from_utf8(body).ok()?;
    let mut fields = text.split(';');
    let mut codes = fields.next()?.split(':');
    let code = codes.next()?.parse::<u32>().ok()?;
    let mut modifiers = fields.next().unwrap_or("1").split(':');
    let decoded = modifiers
        .next()
        .and_then(|mask| mask.parse::<u8>().ok())
        .and_then(|mask| {
            let kind = modifiers.next().unwrap_or("1").parse::<u8>().ok()?;
            Some((mask.saturating_sub(1), kind))
        });
    let (flags, kind) = decoded.unwrap_or((0, 1));
    if kind == 3 {
        return None;
    }
    if flags & 1 != 0
        && let Some(shifted) = codes
            .next()
            .and_then(|code| code.parse::<u32>().ok())
            .and_then(char::from_u32)
    {
        return (shifted == 'c' && flags & 4 != 0).then_some(ClosingKey::Cancel);
    }
    match code {
        13 | 57414 => Some(ClosingKey::Submit),
        27 => Some(ClosingKey::Cancel),
        99 if flags & 4 != 0 => Some(ClosingKey::Cancel),
        _ => None,
    }
}
