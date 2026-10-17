//! Keep terminal control strings intact when an external picker borrows the host terminal.

use std::io::{self, Seek, SeekFrom, Write};

#[derive(Default)]
pub struct Output {
    pending: Vec<u8>,
    head: Vec<u8>,
    mode: Mode,
    spool: Option<tempfile::NamedTempFile>,
    spooled: usize,
    pub saw_graphics: bool,
    pub synchronized: bool,
    pub pending_cursor_replies: usize,
    pub pending_mode_replies: usize,
    pub input_ready: bool,
    terminal_setup_seen: bool,
    keyboard_depth: usize,
    modify_other_keys: bool,
}

#[derive(Default)]
enum Mode {
    #[default]
    Ground,
    Escape,
    Intermediate,
    Csi,
    String {
        osc: bool,
        escaped: bool,
    },
    Utf8(u8),
}

impl Output {
    pub fn forward(&mut self, input: &[u8], writer: &mut impl Write) -> io::Result<()> {
        let mut ready = Vec::with_capacity(input.len());
        for &byte in input {
            let invalid_utf8 = matches!(self.mode, Mode::Utf8(_)) && byte & 0xc0 != 0x80;
            let restart_escape =
                byte == 0x1b && matches!(self.mode, Mode::Escape | Mode::Intermediate | Mode::Csi);
            if invalid_utf8 || restart_escape {
                self.head.clear();
                self.mode = Mode::Ground;
            }
            self.pending.push(byte);
            if self.head.len() < 64 {
                self.head.push(byte);
            }
            self.saw_graphics |= self.head.starts_with(b"\x1b_G");
            let complete = match &mut self.mode {
                Mode::Ground => match byte {
                    0x1b => {
                        self.mode = Mode::Escape;
                        false
                    }
                    0xc2..=0xdf => {
                        self.mode = Mode::Utf8(1);
                        false
                    }
                    0xe0..=0xef => {
                        self.mode = Mode::Utf8(2);
                        false
                    }
                    0xf0..=0xf4 => {
                        self.mode = Mode::Utf8(3);
                        false
                    }
                    _ => true,
                },
                Mode::Escape => match byte {
                    b'[' => {
                        self.mode = Mode::Csi;
                        false
                    }
                    b']' => {
                        self.mode = Mode::String {
                            osc: true,
                            escaped: false,
                        };
                        false
                    }
                    b'P' | b'_' | b'^' | b'X' => {
                        self.mode = Mode::String {
                            osc: false,
                            escaped: false,
                        };
                        false
                    }
                    0x20..=0x2f => {
                        self.mode = Mode::Intermediate;
                        false
                    }
                    _ => true,
                },
                Mode::Intermediate => (0x30..=0x7e).contains(&byte),
                Mode::Csi => (0x40..=0x7e).contains(&byte) || matches!(byte, 0x18 | 0x1a),
                Mode::String { osc, escaped } => {
                    let complete = (*osc && byte == 7)
                        || (*escaped && byte == b'\\')
                        || matches!(byte, 0x18 | 0x1a);
                    *escaped = byte == 0x1b;
                    complete
                }
                Mode::Utf8(remaining) => {
                    *remaining -= 1;
                    *remaining == 0 || byte & 0xc0 != 0x80
                }
            };
            if !complete && self.pending.len() >= 65536 {
                if self.spool.is_none() {
                    self.spool = Some(tempfile::NamedTempFile::new()?);
                }
                let spool = self
                    .spool
                    .as_mut()
                    .ok_or_else(|| io::Error::other("missing terminal token spool"))?;
                spool.write_all(&self.pending)?;
                self.spooled += self.pending.len();
                self.pending.clear();
            }
            if complete {
                self.flush_pending(&mut ready, writer)?;
                self.mode = Mode::Ground;
                self.head.clear();
            }
        }
        writer.write_all(&ready)
    }

    fn flush_pending(&mut self, ready: &mut Vec<u8>, writer: &mut impl Write) -> io::Result<()> {
        // Capability setup precedes the handshake and initial pane projection.
        if self.head == b"\x1b[?7l" {
            self.terminal_setup_seen = true;
        }
        if self.head.starts_with(b"\x1b[") && self.head.ends_with(b"$p") {
            self.pending_mode_replies += 1;
        }
        if self.head == b"\x1b[6n" || self.head == b"\x1b[?6n" {
            self.pending_cursor_replies += 1;
        }
        if let Some(body) = self
            .head
            .strip_prefix(b"\x1b[>")
            .and_then(|body| body.strip_suffix(b"u"))
            && body.iter().all(u8::is_ascii_digit)
        {
            self.keyboard_depth += 1;
        }

        if let Some(body) = self
            .head
            .strip_prefix(b"\x1b[<")
            .and_then(|body| body.strip_suffix(b"u"))
        {
            let count = std::str::from_utf8(body)
                .ok()
                .and_then(|body| body.parse().ok())
                .unwrap_or(1);
            self.keyboard_depth = self.keyboard_depth.saturating_sub(count);
        }
        if self.head.starts_with(b"\x1b[>4;") && self.head.ends_with(b"m") {
            self.modify_other_keys = self.head != b"\x1b[>4;0m";
        }
        if self.head == b"\x1b[?2026h" {
            self.synchronized = true;
        }
        if self.head == b"\x1b[?2026l" {
            if self.synchronized && self.terminal_setup_seen {
                self.input_ready = true;
            }
            self.synchronized = false;
        }
        if let Some(mut spool) = self.spool.take() {
            writer.write_all(ready)?;
            ready.clear();
            spool.seek(SeekFrom::Start(0))?;
            io::copy(&mut spool, writer)?;
            self.spooled = 0;
        }
        ready.append(&mut self.pending);
        Ok(())
    }

    pub fn restore_host_modes(&mut self, writer: &mut impl Write) -> io::Result<()> {
        if self.synchronized {
            writer.write_all(b"\x1b[?2026l")?;
            self.synchronized = false;
        }
        if self.modify_other_keys {
            writer.write_all(b"\x1b[>4;0m")?;
            self.modify_other_keys = false;
        }
        while self.keyboard_depth > 0 {
            writer.write_all(b"\x1b[<u")?;
            self.keyboard_depth -= 1;
        }
        writer.flush()
    }

    pub fn input_prefix(&self) -> &[u8] {
        &self.head
    }

    pub fn flush_input_pending(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut ready = Vec::new();
        self.flush_pending(&mut ready, &mut bytes)?;
        bytes.extend_from_slice(&ready);
        self.mode = Mode::Ground;
        self.head.clear();
        Ok(bytes)
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len() + self.spooled
    }
}
