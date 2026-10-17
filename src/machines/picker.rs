//! The workspace picker embedded in the launcher, with read-only machine browsing.
use super::{
    catalog,
    handoff::{PickerEvent, Selection},
};
use crate::picker::{
    api::MachineApi,
    app::{Action, App},
    config::{Config, SearchMode},
    model::Snapshot,
    theme::Theme,
    ui,
};
use anyhow::Result;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::Rect,
    widgets::{Paragraph, Wrap},
};
use std::{
    fs::File,
    io::{self, Write},
    os::fd::{AsRawFd, FromRawFd},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use terminal_input::{
    event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind},
    terminal,
};

fn empty() -> Snapshot {
    Snapshot {
        workspaces: vec![],
        tabs: vec![],
        focused_workspace_id: None,
        focused_tab_id: None,
    }
}

pub struct MachinePicker {
    pub app: App,
    rows: Vec<(String, String)>,
    selected: usize,
    generation: u64,
    pub loading: bool,
    pub error: Option<String>,
}
impl MachinePicker {
    pub fn new(rows: Vec<(String, String)>, active: &str, mode: SearchMode) -> Self {
        let selected = rows.iter().position(|(id, _)| id == active).unwrap_or(0);
        Self {
            app: App::new(empty(), mode),
            rows,
            selected,
            generation: 0,
            loading: true,
            error: None,
        }
    }
    pub fn machine(&self) -> &str {
        &self.rows[self.selected].0
    }
    pub fn cycle(&mut self, delta: isize) -> u64 {
        self.selected =
            (self.selected as isize + delta).rem_euclid(self.rows.len() as isize) as usize;
        self.generation += 1;
        self.loading = true;
        self.error = None;
        self.replace(empty());
        self.generation
    }
    fn replace(&mut self, snapshot: Snapshot) {
        let mut app = App::new(snapshot, self.app.mode);
        app.theme = self.app.theme.clone();
        if !self.app.query.is_empty() {
            app.set_query(self.app.query.clone());
        }
        self.app = app;
    }
    pub fn complete(&mut self, generation: u64, result: Result<Snapshot>) {
        if generation != self.generation {
            return;
        }
        self.loading = false;
        match result {
            Ok(snapshot) => self.replace(snapshot),
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }
}

struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            io::stdout(),
            crossterm::event::DisableMouseCapture,
            crossterm::event::DisableBracketedPaste,
            crossterm::cursor::Show
        );
        let _ = terminal::disable_raw_mode();
    }
}
struct Loads(Vec<(Arc<AtomicBool>, thread::JoinHandle<()>)>);
impl Loads {
    fn start(
        &mut self,
        binary: &std::ffi::OsStr,
        machine: &str,
        session: &str,
        generation: u64,
        profile: Option<&super::catalog::Profile>,
        sender: &mpsc::Sender<(u64, Result<Snapshot>)>,
    ) {
        for (cancel, _) in &self.0 {
            cancel.store(true, Ordering::Relaxed);
        }
        let profile = profile.cloned();
        let binary = binary.to_owned();
        let machine = machine.to_owned();
        let session = session.to_owned();
        let sender = sender.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = cancelled.clone();
        self.0.retain(|(_, thread)| !thread.is_finished());
        self.0.push((
            cancelled,
            thread::spawn(move || {
                let result = MachineApi {
                    binary: &binary,
                    machine: &machine,
                    local_session: &session,
                    profile: profile.as_ref(),
                }
                .snapshot(&token);
                let _ = sender.send((generation, result));
            }),
        ));
    }
}
impl Drop for Loads {
    fn drop(&mut self) {
        for (token, _) in &self.0 {
            token.store(true, Ordering::Relaxed);
        }
        for (_, worker) in self.0.drain(..) {
            let _ = worker.join();
        }
    }
}

pub fn run(
    binary: &std::ffi::OsStr,
    result_fd: i32,
    active: &str,
    local_session: &str,
) -> Result<()> {
    // SAFETY: launch validates and transfers ownership of this inherited pipe.
    let mut result = unsafe { File::from_raw_fd(result_fd) };
    if unsafe { libc::fcntl(result.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let profiles = catalog::load(binary)?;
    let mut rows = vec![("local".into(), "Local".into())];
    rows.extend(profiles.iter().map(|profile| {
        (
            profile.id.clone(),
            format!(
                "{} [{} · {}]",
                profile.label,
                profile.session,
                profile.id.chars().take(6).collect::<String>()
            ),
        )
    }));
    let config_path = crate::command::run(
        std::process::Command::new(binary).args(["plugin", "config-dir", "herdrpp"]),
        Duration::from_secs(5),
    )?;
    let config_dir = std::path::PathBuf::from(std::str::from_utf8(&config_path)?.trim());
    let config = Config::load(&config_dir)?;
    let mut picker = MachinePicker::new(rows, active, config.picker.search_mode);
    picker.app.theme = Theme::load().0;
    picker
        .app
        .set_query(std::env::var("HERDRPP_PICKER_QUERY").unwrap_or_default());
    let notice = std::env::var("HERDRPP_PICKER_ERROR").ok();
    terminal::enable_raw_mode()?;
    let _screen = Screen;
    crossterm::execute!(
        io::stdout(),
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste
    )?;
    let mut terminal = Terminal::new(super::embedded::Embedded(CrosstermBackend::new(
        io::stdout(),
    )))?;
    let (sender, receiver) = mpsc::channel();
    let mut loads = Loads(vec![]);
    loads.start(
        binary,
        picker.machine(),
        local_session,
        0,
        profiles
            .iter()
            .find(|profile| profile.id == picker.machine()),
        &sender,
    );
    let mut previous = Rect::default();
    let mut next = Rect::default();
    loop {
        for (generation, result) in receiver.try_iter() {
            picker.complete(generation, result);
        }
        terminal.draw(|frame| {
            let area = frame.area();
            let width = area.width.saturating_mul(4) / 5;
            let height = area.height.min(22);
            let panel = Rect::new(
                (area.width - width) / 2,
                (area.height - height) / 2,
                width,
                height,
            );
            let header = Rect::new(panel.x, panel.y, panel.width, panel.height.min(1));
            previous = Rect::new(header.x, header.y, header.width.min(2), header.height);
            next = Rect::new(
                header.right().saturating_sub(2),
                header.y,
                header.width.min(2),
                header.height,
            );
            let status = if picker.loading { " · Loading…" } else { "" };
            let label = format!(
                "  Machine: {}{} · PgUp/PgDn",
                picker.rows[picker.selected].1, status
            );
            frame.render_widget(
                Paragraph::new(label).style(picker.app.theme.palette().base_style()),
                header,
            );
            frame.render_widget(
                Paragraph::new("‹").style(picker.app.theme.palette().base_style()),
                previous,
            );
            frame.render_widget(
                Paragraph::new(" ›").style(picker.app.theme.palette().base_style()),
                next,
            );
            let content = Rect::new(
                panel.x,
                panel.y + header.height,
                panel.width,
                panel.height.saturating_sub(header.height),
            );
            ui::render_in(frame, &mut picker.app, content);
            if let Some(notice) = &notice {
                let status = Rect::new(
                    content.x,
                    content.bottom().saturating_sub(2),
                    content.width,
                    content.height.min(1),
                );
                frame.render_widget(
                    Paragraph::new(notice.as_str()).style(picker.app.theme.palette().base_style()),
                    status,
                );
            }
            if let Some(error) = &picker.error {
                let error_area = Rect::new(
                    content.x,
                    content.y.saturating_add(2),
                    content.width,
                    content.height.saturating_sub(5),
                );
                frame.render_widget(
                    Paragraph::new(format!("{error}\nPgUp/PgDn changes machine · Esc cancels"))
                        .style(picker.app.theme.palette().base_style())
                        .wrap(Wrap { trim: false }),
                    error_area,
                );
            }
        })?;
        if !event::poll(Duration::from_millis(20))? {
            continue;
        }
        let event = event::read()?;
        let click = matches!(&event, Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left));
        let delta = match &event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::PageUp => -1,
                KeyCode::PageDown => 1,
                _ => 0,
            },
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                if previous.contains((mouse.column, mouse.row).into()) {
                    -1
                } else if next.contains((mouse.column, mouse.row).into()) {
                    1
                } else {
                    0
                }
            }
            _ => 0,
        };
        if delta != 0 {
            let generation = picker.cycle(delta);
            loads.start(
                binary,
                picker.machine(),
                local_session,
                generation,
                profiles
                    .iter()
                    .find(|profile| profile.id == picker.machine()),
                &sender,
            );
            if click {
                acknowledge(&mut result, &PickerEvent::Continue)?;
            }
            continue;
        }
        let submit = matches!(&event, Event::Key(key) if key.kind != KeyEventKind::Release && key.code == KeyCode::Enter);
        match picker.app.handle(event) {
            Action::Close => {
                acknowledge(&mut result, &PickerEvent::Cancelled)?;
                return Ok(());
            }
            Action::Switch(target) if !picker.loading && picker.error.is_none() => {
                acknowledge(
                    &mut result,
                    &PickerEvent::Selected(Selection {
                        machine: picker.machine().into(),
                        profile: profiles
                            .iter()
                            .find(|profile| profile.id == picker.machine())
                            .cloned(),
                        target,
                        query: picker.app.query.clone(),
                    }),
                )?;
                return Ok(());
            }
            _ if submit || click => acknowledge(&mut result, &PickerEvent::Continue)?,
            _ => {}
        }
    }
}
fn acknowledge(result: &mut File, event: &PickerEvent) -> Result<()> {
    serde_json::to_writer(&mut *result, event)?;
    result.write_all(b"\n")?;
    result.flush()?;
    Ok(())
}
