use std::{
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use super::output::Output;
use anyhow::{Context, Result, bail};

pub struct Host {
    original: libc::termios,
}
impl Host {
    pub fn new() -> Result<Self> {
        let mut original = std::mem::MaybeUninit::uninit();
        // SAFETY: tcgetattr writes a full termios into the provided valid storage.
        if unsafe { libc::tcgetattr(0, original.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self {
            original: unsafe { original.assume_init() },
        })
    }
    pub fn raw(&self) -> Result<()> {
        let mut raw = self.original;
        // SAFETY: raw is initialized termios; fd 0 is the validated terminal.
        unsafe {
            libc::cfmakeraw(&mut raw);
        }
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    pub fn discard_input(&self) -> Result<()> {
        loop {
            // SAFETY: fd 0 is the host terminal; only its unread input is discarded.
            if unsafe { libc::tcflush(0, libc::TCIFLUSH) } == 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error.into());
            }
        }
    }
    pub fn restore(&self) -> Result<()> {
        // SAFETY: original is the complete termios captured from fd 0.
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &self.original) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.restore();
        let _ = io::stdout().write_all(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1016l\x1b[?2004l\x1b[?1004l\x1b[?1049l\x1b[?25h");
    }
}

pub fn geometry() -> Result<libc::winsize> {
    let mut size = std::mem::MaybeUninit::uninit();
    // SAFETY: ioctl writes the terminal geometry into valid winsize storage.
    if unsafe { libc::ioctl(0, libc::TIOCGWINSZ, size.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let size: libc::winsize = unsafe { size.assume_init() };
    if size.ws_row == 0 || size.ws_col == 0 {
        bail!("host terminal has zero size");
    }
    Ok(size)
}

pub struct Client {
    pub child: Child,
    pub master: File,
    pub output: Output,
    input: std::collections::VecDeque<u8>,
    startup_input: std::collections::VecDeque<u8>,
    stopped: bool,
    started: Instant,
}
impl Client {
    pub fn spawn(mut command: Command) -> Result<Self> {
        let mut size = geometry()?;
        let mut master = -1;
        let mut slave = -1;
        // SAFETY: openpty initializes both descriptor outputs; optional name and termios are null.
        if unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::addr_of_mut!(size),
            )
        } != 0
        {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: successful openpty returned fresh descriptors; each File owns one.
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        let mut raw = std::mem::MaybeUninit::uninit();
        // SAFETY: the owned slave is a PTY and raw points to valid termios storage.
        if unsafe { libc::tcgetattr(slave.as_raw_fd(), raw.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut raw = unsafe { raw.assume_init() };
        unsafe {
            libc::cfmakeraw(&mut raw);
        }
        if unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &raw) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        command
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        // SAFETY: only async-signal-safe libc calls are made between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        // SAFETY: fcntl acts on the owned PTY descriptor.
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                < 0
        {
            return Err(io::Error::last_os_error().into());
        }
        if unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let child = command.spawn().context("launch Herdr viewing client")?;
        Ok(Self {
            child,
            master,
            output: Output::default(),
            input: std::collections::VecDeque::new(),
            startup_input: std::collections::VecDeque::new(),
            stopped: false,
            started: Instant::now(),
        })
    }
    pub fn startup_expired(&self) -> bool {
        !self.output.input_ready && self.started.elapsed() >= Duration::from_secs(30)
    }
    pub fn signal(&self, signal: i32) -> Result<()> {
        // SAFETY: the negative child PID identifies only the process group this launcher created.
        if unsafe { libc::kill(-(self.child.id() as i32), signal) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    pub fn drain(&mut self) -> Result<()> {
        self.drain_observed(&mut |_| {})
    }
    pub fn drain_observed(&mut self, observer: &mut impl FnMut(&[u8])) -> Result<()> {
        self.drain_budget_observed(1024 * 1024, observer)
    }
    fn drain_budget(&mut self, budget: usize) -> Result<()> {
        self.drain_budget_observed(budget, &mut |_| {})
    }
    fn drain_budget_observed(
        &mut self,
        budget: usize,
        observer: &mut impl FnMut(&[u8]),
    ) -> Result<()> {
        let mut consumed = 0;
        let mut bytes = [0; 65536];
        let mut stdout = io::stdout().lock();
        while consumed < budget {
            match self.master.read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => {
                    consumed += count;
                    self.output
                        .forward_observed(&bytes[..count], &mut stdout, observer)?;
                }
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.raw_os_error() == Some(libc::EIO) =>
                {
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        stdout.flush()?;
        if !self.output.input_ready
            && !self.startup_input.is_empty()
            && self.started.elapsed() >= Duration::from_secs(30)
        {
            bail!(
                "Herdr terminal setup did not complete within 30 seconds; queued keyboard input was not forwarded"
            );
        }
        self.flush_input()
    }
    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.input.extend(bytes);
        self.flush_input()
    }
    pub fn send_user(&mut self, bytes: &[u8]) -> Result<()> {
        if self.output.input_ready {
            self.input.extend(bytes);
        } else {
            self.startup_input.extend(bytes);
        }
        self.flush_input()
    }
    pub fn queued_input(&self) -> usize {
        self.input.len() + self.startup_input.len()
    }
    fn flush_input(&mut self) -> Result<()> {
        if self.output.input_ready {
            self.input.append(&mut self.startup_input);
        }
        let mut written = 0;
        while !self.input.is_empty() && written < 65536 {
            let bytes = self.input.make_contiguous();
            match self.master.write(bytes) {
                Ok(0) => bail!("Herdr PTY input closed"),
                Ok(count) => {
                    self.input.drain(..count);
                    written += count;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
    pub fn resize(&self) -> Result<()> {
        let size = geometry()?;
        // SAFETY: ioctl reads initialized winsize for the owned PTY.
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        self.signal(libc::SIGWINCH)
    }
    pub fn pause(&mut self) -> Result<()> {
        self.signal(libc::SIGSTOP)?;
        self.stopped = true;
        let mut status = 0;
        // SAFETY: wait only for this owned direct child to acknowledge its stop.
        let result = loop {
            let result =
                unsafe { libc::waitpid(self.child.id() as i32, &mut status, libc::WUNTRACED) };
            if result >= 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                break result;
            }
        };
        if result < 0 || !libc::WIFSTOPPED(status) {
            bail!("Herdr exited while opening machine picker");
        }
        self.drain_budget(usize::MAX)?;
        let mut stdout = io::stdout().lock();
        if self.output.synchronized {
            stdout.write_all(b"\x1b[?2026l")?;
        }
        if self.output.saw_graphics {
            stdout.write_all(b"\x1b_Ga=d,d=a,q=2;\x1b\\")?;
        }
        stdout.flush()?;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<()> {
        self.signal(libc::SIGCONT)?;
        self.stopped = false;
        self.resize()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        // Once reaped, its PID may be reused: never signal that process group again.
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.signal(libc::SIGCONT);
            let _ = self.signal(libc::SIGTERM);
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                // Herdr restores terminal modes while exiting; keep its PTY output
                // moving so terminal drain cannot block the owned child's shutdown.
                let _ = self.drain();
                match self.child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.signal(libc::SIGKILL);
                // Closing the PTY releases macOS terminal-drain waits during exit.
                if let Ok(null) = File::open("/dev/null") {
                    drop(std::mem::replace(&mut self.master, null));
                }
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline {
                    if !matches!(self.child.try_wait(), Ok(None)) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        let _ = self.drain();
        let _ = self.output.restore_host_modes(&mut io::stdout().lock());
    }
}

pub fn poll(fds: &[i32], events: i16, timeout: i32) -> Result<Vec<bool>> {
    let mut fds: Vec<_> = fds
        .iter()
        .map(|fd| libc::pollfd {
            fd: *fd,
            events,
            revents: 0,
        })
        .collect();
    // SAFETY: poll receives a valid mutable descriptor array with its exact length.
    let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
    if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
        return Err(io::Error::last_os_error().into());
    }
    Ok(fds.iter().map(|fd| fd.revents != 0).collect())
}
