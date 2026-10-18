use anyhow::Result;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io::{self, Stdout, Write};
use terminal_input::terminal;

pub(crate) struct Screen {
    appearance_reporting: bool,
}

impl Screen {
    pub(crate) fn enter(
        appearance_reporting: bool,
    ) -> Result<(Self, Terminal<CrosstermBackend<Stdout>>)> {
        terminal::enable_raw_mode()?;
        let guard = Self {
            appearance_reporting,
        };
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        if appearance_reporting {
            let mut stdout = io::stdout();
            stdout.write_all(b"\x1b[?2031h\x1b[?996n")?;
            stdout.flush()?;
        }
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok((guard, terminal))
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        if self.appearance_reporting {
            let _ = io::stdout().write_all(b"\x1b[?2031l");
        }
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            PopKeyboardEnhancementFlags,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        let _ = terminal::disable_raw_mode();
    }
}

/// The PTY slave disappears when Herdr releases the popup's master after exit.
/// Observe the path without retaining a slave descriptor, which would keep it alive.
pub(crate) fn popup_terminal_path() -> Result<std::path::PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut buffer = [0 as libc::c_char; 1024];
    // SAFETY: buffer is writable for its length; ttyname_r terminates it on success.
    let status = unsafe { libc::ttyname_r(libc::STDIN_FILENO, buffer.as_mut_ptr(), buffer.len()) };
    anyhow::ensure!(
        status == 0,
        "command palette requires a popup terminal: {}",
        std::io::Error::from_raw_os_error(status)
    );
    // SAFETY: successful ttyname_r wrote a NUL-terminated path into buffer.
    let name = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) };
    Ok(std::path::PathBuf::from(std::ffi::OsStr::from_bytes(
        name.to_bytes(),
    )))
}
