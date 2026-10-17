//! Host replies belong to Herdr even while a local selector temporarily receives user keys.
use super::output::Output;
use std::{
    io,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Responses {
    stream: Output,
    paste: bool,
    paste_pending: Vec<u8>,
    last: Option<Instant>,
}

impl Responses {
    pub fn byte(&mut self, byte: u8, pending: &mut Output) -> io::Result<Option<(bool, Vec<u8>)>> {
        self.last = Some(Instant::now());
        if self.paste {
            const END: &[u8] = b"\x1b[201~";
            self.paste_pending.push(byte);
            let mut ready = Vec::new();
            while !self.paste_pending.is_empty() && !END.starts_with(&self.paste_pending) {
                ready.push(self.paste_pending.remove(0));
            }
            if self.paste_pending == END {
                ready.append(&mut self.paste_pending);
                self.paste = false;
            }
            return Ok((!ready.is_empty()).then_some((false, ready)));
        }
        let mut complete = Vec::new();
        self.stream.forward(&[byte], &mut complete)?;
        if complete.is_empty() {
            return Ok(None);
        }
        if complete == b"\x1b[200~" {
            self.paste = true;
        }
        if complete == b"\x1b[201~" {
            self.paste = false;
            return Ok(Some((false, complete)));
        }
        let response = !self.paste && is_response(&complete, pending);
        Ok(Some((response, complete)))
    }
    pub fn is_pasting(&self) -> bool {
        self.paste
    }

    pub fn flush_timeout(&mut self, pending: &Output) -> io::Result<Option<Vec<u8>>> {
        self.flush_timeout_at(pending, Instant::now())
    }
    fn flush_timeout_at(&mut self, pending: &Output, now: Instant) -> io::Result<Option<Vec<u8>>> {
        if self.paste {
            return Ok(None);
        }
        let prefix = self.stream.input_prefix();
        let identified = [
            b"\x1b]52;".as_slice(),
            b"\x1b]10;",
            b"\x1b]11;",
            b"\x1b]12;",
            b"\x1b_G",
            b"\x1bP1+",
            b"\x1bP0+",
        ]
        .iter()
        .any(|start| prefix.starts_with(start));
        let cursor = pending.pending_cursor_replies > 0
            && prefix.strip_prefix(b"\x1b[").is_some_and(|body| {
                !body.is_empty()
                    && body
                        .iter()
                        .all(|byte| byte.is_ascii_digit() || *byte == b';' || *byte == b'?')
            });
        let mode_reply = pending.pending_mode_replies > 0
            && prefix.strip_prefix(b"\x1b[?").is_some_and(|body| {
                !body.is_empty()
                    && body
                        .iter()
                        .all(|byte| byte.is_ascii_digit() || *byte == b';' || *byte == b'$')
            });
        if self.stream.pending_len() > 0
            && !identified
            && !cursor
            && !mode_reply
            && self.last.is_some_and(|last| {
                now.saturating_duration_since(last) >= Duration::from_millis(100)
            })
        {
            self.last = None;
            Ok(Some(self.stream.flush_input_pending()?))
        } else {
            Ok(None)
        }
    }
}

pub fn is_response(bytes: &[u8], pending: &mut Output) -> bool {
    if let Some(body) = bytes
        .strip_prefix(b"\x1b[")
        .and_then(|body| body.strip_suffix(b"$y"))
    {
        let body = body.strip_prefix(b"?").unwrap_or(body);
        if let Some(separator) = body.iter().position(|byte| *byte == b';') {
            let (mode, setting) = body.split_at(separator);
            if !mode.is_empty()
                && mode.iter().all(u8::is_ascii_digit)
                && setting.len() == 2
                && matches!(setting[1], b'0'..=b'4')
            {
                pending.pending_mode_replies = pending.pending_mode_replies.saturating_sub(1);
                return true;
            }
        }
    }
    if bytes.starts_with(b"\x1b_G") {
        return true;
    }
    if bytes.starts_with(b"\x1b]") {
        return [
            b"\x1b]52;".as_slice(),
            b"\x1b]10;",
            b"\x1b]11;",
            b"\x1b]12;",
            b"\x1b]4;",
        ]
        .iter()
        .any(|prefix| bytes.starts_with(prefix));
    }
    if bytes.starts_with(b"\x1bP") {
        return true;
    }
    if let Some(body) = bytes.strip_prefix(b"\x1b[") {
        if body.ends_with(b"R") && pending.pending_cursor_replies > 0 {
            pending.pending_cursor_replies -= 1;
            return true;
        }
        return body.ends_with(b"c")
            || body.ends_with(b"t")
            || (body.starts_with(b"?") && body.ends_with(b"u"))
            || body.ends_with(b"n");
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_decrpm_reply_is_held_past_the_keyboard_ambiguity_timeout() {
        let mut pending = Output::default();
        pending.forward(b"\x1b[?1016$p", &mut Vec::new()).unwrap();
        let mut responses = Responses::default();
        let prefix: Vec<_> = b"\x1b[?1016;"
            .iter()
            .map(|byte| responses.byte(*byte, &mut pending).unwrap())
            .collect();
        assert!(prefix.iter().all(Option::is_none));
        assert!(
            responses
                .flush_timeout_at(&pending, Instant::now() + Duration::from_secs(1))
                .unwrap()
                .is_none()
        );
        assert!(responses.byte(b'2', &mut pending).unwrap().is_none());
        assert!(responses.byte(b'$', &mut pending).unwrap().is_none());
        assert_eq!(
            responses.byte(b'y', &mut pending).unwrap(),
            Some((true, b"\x1b[?1016;2$y".to_vec()))
        );
        assert_eq!(pending.pending_mode_replies, 0);
    }

    #[test]
    fn bracketed_paste_remains_opaque_even_with_unterminated_control_strings() {
        let mut pending = Output::default();
        let mut responses = Responses::default();
        let paste = b"\x1b[200~literal\x1b]unfinished\x1c\x1d\x1b[6n\x1b[201~";
        let events: Vec<_> = paste
            .iter()
            .filter_map(|byte| responses.byte(*byte, &mut pending).unwrap())
            .collect();
        let bytes: Vec<_> = events
            .iter()
            .flat_map(|(_, bytes)| bytes.iter().copied())
            .collect();
        assert_eq!(bytes, paste);
        assert!(events.iter().all(|(response, _)| !response));
        assert!(!responses.is_pasting());
    }

    #[test]
    fn malformed_mode_replies_are_not_classified_as_terminal_responses() {
        let mut pending = Output::default();
        assert!(!is_response(b"\x1b[?1016;9$y", &mut pending));
        assert!(!is_response(b"\x1b[?mode;2$y", &mut pending));
        assert!(!is_response(b"\x1b[?1016;22$y", &mut pending));
        assert!(is_response(b"\x1b[?1016;0$y", &mut pending));
        assert!(is_response(b"\x1b[?1016;4$y", &mut pending));
    }
}
