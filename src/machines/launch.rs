use std::{
    env,
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, IsTerminal, Read, Seek, SeekFrom, Write},
    os::fd::{AsRawFd, FromRawFd},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};

use super::{
    catalog::{self, Destination},
    handoff::InputChunk,
    input::{Action, Input},
    terminal::{self, Client, Host},
};
use anyhow::{Context, Result, bail};

static STOP: AtomicBool = AtomicBool::new(false);
static RESIZE: AtomicBool = AtomicBool::new(false);
extern "C" fn stop_signal(_: i32) {
    STOP.store(true, Ordering::Relaxed);
}
extern "C" fn resize_signal(_: i32) {
    RESIZE.store(true, Ordering::Relaxed);
}

struct Signals(Vec<(i32, libc::sighandler_t)>);
impl Signals {
    fn install() -> Self {
        STOP.store(false, Ordering::Relaxed);
        RESIZE.store(false, Ordering::Relaxed);
        let mut handlers = Vec::new();
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            // SAFETY: these handlers only write lock-free atomics.
            handlers.push((signal, unsafe {
                libc::signal(signal, stop_signal as *const () as libc::sighandler_t)
            }));
        }
        handlers.push((libc::SIGWINCH, unsafe {
            libc::signal(
                libc::SIGWINCH,
                resize_signal as *const () as libc::sighandler_t,
            )
        }));
        Self(handlers)
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        for (signal, handler) in &self.0 {
            unsafe {
                libc::signal(*signal, *handler);
            }
        }
    }
}

enum PendingInput {
    Byte(u8),
    Literal(Vec<u8>),
}

fn queue_input(queue: &mut std::collections::VecDeque<PendingInput>, chunks: Vec<InputChunk>) {
    for chunk in chunks {
        match chunk {
            InputChunk::Keyboard(bytes) => queue.extend(bytes.into_iter().map(PendingInput::Byte)),
            InputChunk::Literal(bytes) => queue.push_back(PendingInput::Literal(bytes)),
        }
    }
}

fn release_focused_input(
    queue: &mut std::collections::VecDeque<PendingInput>,
    before: Vec<InputChunk>,
    after: Vec<InputChunk>,
) {
    let buffered = std::mem::take(queue);
    queue_input(queue, before);
    queue.extend(buffered);
    queue_input(queue, after);
}

fn borrowed_controls(responses: &super::responses::Responses) -> Input {
    let mut controls = Input::default();
    if responses.is_pasting() {
        // The paste opener may already be queued by the closing selector.
        let _ = borrowed_keyboard(&mut controls, b"\x1b[200~");
    }
    controls
}

fn borrowed_keyboard(keys: &mut Input, bytes: &[u8]) -> Option<Vec<u8>> {
    let mut forwarded = Vec::new();
    for byte in bytes {
        for action in keys.byte(*byte) {
            match action {
                Action::Bytes(bytes) => forwarded.extend(bytes),
                Action::Picker => forwarded.push(0x1d),
                Action::Quit => return None,
            }
        }
    }
    Some(forwarded)
}

fn discard_failed_input(
    host: &Host,
    queue: &mut std::collections::VecDeque<PendingInput>,
    keys: &mut Input,
    responses: &mut super::responses::Responses,
) -> Result<()> {
    queue.clear();
    keys.reset();
    *responses = super::responses::Responses::default();
    host.discard_input()
}

pub fn binary() -> OsString {
    env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into())
}

fn choose(
    client: &mut Client,
    responses: &mut super::responses::Responses,
    active_id: &str,
    state_home: &std::path::Path,
    local_session: &str,
    retry: Option<&(super::handoff::Selection, String)>,
) -> Result<super::handoff::PickerChoice> {
    use super::handoff::{Handoff, PickerEvent};
    let mut handoff = Handoff::default();
    let mut controls = borrowed_controls(responses);
    let mut delayed_replies = tempfile::tempfile()?;
    client.pause()?;
    let choice = (|| {
        let mut descriptors = [-1; 2];
        // SAFETY: pipe initializes both descriptor slots.
        if unsafe { libc::pipe(descriptors.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut reader = unsafe { File::from_raw_fd(descriptors[0]) };
        let writer = unsafe { File::from_raw_fd(descriptors[1]) };
        let flags = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                < 0
        {
            return Err(io::Error::last_os_error().into());
        }
        let mut command = Command::new(env::current_exe()?);
        let picker_active = retry
            .map(|(selection, _)| selection.machine.as_str())
            .unwrap_or(active_id);
        if let Some((selection, error)) = retry {
            command
                .env("HERDRPP_PICKER_QUERY", &selection.query)
                .env("HERDRPP_PICKER_ERROR", error);
        }
        command.args([
            "machine-picker",
            &writer.as_raw_fd().to_string(),
            picker_active,
            local_session,
        ]);
        command.env("XDG_STATE_HOME", state_home);
        let mut picker = Client::spawn(command).context("open machine picker")?;
        drop(writer);
        let mut acknowledgements = Vec::new();
        let mut selected = None;
        let mut closing = false;
        let status = loop {
            let exited = picker.child.try_wait()?;
            picker.drain()?;
            let mut bytes = [0; 4096];
            loop {
                match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => acknowledgements.extend_from_slice(&bytes[..count]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            while let Some(end) = acknowledgements.iter().position(|byte| *byte == b'\n') {
                let event: PickerEvent = serde_json::from_slice(&acknowledgements[..end])
                    .context("read machine picker acknowledgement")?;
                acknowledgements.drain(..=end);
                let forwarded = handoff.acknowledge(&event)?;
                picker.send(&forwarded)?;
                match event {
                    PickerEvent::Continue => {}
                    PickerEvent::Selected(id) => {
                        selected = Some(id);
                        closing = true;
                    }
                    PickerEvent::Cancelled => closing = true,
                }
            }
            if STOP.load(Ordering::Relaxed) {
                return Ok(handoff.finish(None));
            }
            if RESIZE.swap(false, Ordering::Relaxed) {
                picker.resize()?;
                client.resize()?;
            }
            if let Some(status) = exited {
                break status;
            }
            if let Some(Action::Bytes(bytes)) = controls.flush() {
                picker.send(&handoff.timeout(&bytes)?)?;
            }
            if let Some(bytes) = responses.flush_timeout(&client.output)? {
                picker.send(&handoff.timeout(&bytes)?)?;
            }
            let ready = terminal::poll(
                &[0, picker.master.as_raw_fd(), reader.as_raw_fd()],
                libc::POLLIN,
                20,
            )?;
            if !ready[0] {
                continue;
            }
            let byte = match host_byte()? {
                HostInput::Byte(byte) => byte,
                HostInput::Eof => {
                    return Ok(handoff.finish(None));
                }
                HostInput::Interrupted => continue,
            };
            if let Some((response, bytes)) = responses.byte(byte, &mut client.output)? {
                if response {
                    delayed_replies.write_all(&bytes)?;
                } else {
                    let Some(bytes) = borrowed_keyboard(&mut controls, &bytes) else {
                        STOP.store(true, Ordering::Relaxed);
                        return Ok(handoff.finish(None));
                    };
                    picker.send(&handoff.feed(&bytes)?)?;
                }
            }
        };
        picker.drain()?;
        drop(picker);
        if !status.success() {
            bail!("machine picker failed ({status})");
        }
        if !closing {
            bail!("machine picker exited without acknowledging its closing key");
        }
        Ok(handoff.finish(selected))
    })();
    let resumed = client.resume().and_then(|()| {
        delayed_replies.seek(SeekFrom::Start(0))?;
        let mut bytes = [0; 65536];
        loop {
            let count = delayed_replies.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            client.send(&bytes[..count])?;
        }
        Ok(())
    });
    match (choice, resumed) {
        (Ok(choice), Ok(())) => Ok(choice),
        (Err(error), _) => Err(error),
        (_, Err(error)) => Err(error),
    }
}

enum HostInput {
    Byte(u8),
    Eof,
    Interrupted,
}
fn host_byte() -> Result<HostInput> {
    let mut byte = [0; 1];
    // SAFETY: fd 0 is the owned host input and byte has one writable byte.
    let count = unsafe { libc::read(0, byte.as_mut_ptr().cast(), byte.len()) };
    if count < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(HostInput::Interrupted);
        }
        return Err(error.into());
    }
    if count == 0 {
        Ok(HostInput::Eof)
    } else {
        Ok(HostInput::Byte(byte[0]))
    }
}

struct CancelOnDrop<'a>(&'a AtomicBool);
impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn check_destination(
    client: &mut Client,
    responses: &mut super::responses::Responses,
    binary: &OsStr,
    destination: &Destination,
    state_home: &std::path::Path,
    selection: &super::handoff::Selection,
    local_session: &str,
) -> Result<Vec<InputChunk>> {
    probe(client, responses, |cancellation| {
        catalog::preflight_in(binary, destination, cancellation, Some(state_home))?;
        crate::picker::api::MachineApi {
            binary,
            machine: &selection.machine,
            local_session,
            profile: selection.profile.as_ref(),
        }
        .validate_target(&selection.target, cancellation)
    })
}

fn probe(
    client: &mut Client,
    responses: &mut super::responses::Responses,
    operation: impl FnOnce(&AtomicBool) -> Result<()> + Send,
) -> Result<Vec<InputChunk>> {
    let cancelled = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancellation = &cancelled;
        scope.spawn(move || {
            let _ = sender.send(operation(cancellation));
        });
        let _cancel_on_return = CancelOnDrop(&cancelled);
        let mut queued = Vec::new();
        let mut controls = borrowed_controls(responses);
        loop {
            match receiver.try_recv() {
                Ok(result) => {
                    result?;
                    return Ok(queued);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    bail!("machine status worker stopped")
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            client.drain()?;
            if STOP.load(Ordering::Relaxed) {
                cancelled.store(true, Ordering::Relaxed);
            }
            if RESIZE.swap(false, Ordering::Relaxed) {
                client.resize()?;
            }
            if let Some(Action::Bytes(bytes)) = controls.flush() {
                queued.push(InputChunk::Literal(bytes));
            }
            if let Some(bytes) = responses.flush_timeout(&client.output)? {
                if bytes == [0x1b] && !responses.is_pasting() {
                    cancelled.store(true, Ordering::Relaxed);
                } else {
                    queued.push(InputChunk::Literal(bytes));
                }
            }
            let ready = terminal::poll(&[0, client.master.as_raw_fd()], libc::POLLIN, 20)?;
            if !ready[0] {
                continue;
            }
            let byte = match host_byte()? {
                HostInput::Byte(byte) => byte,
                HostInput::Eof => {
                    cancelled.store(true, Ordering::Relaxed);
                    STOP.store(true, Ordering::Relaxed);
                    continue;
                }
                HostInput::Interrupted => continue,
            };
            if let Some((response, bytes)) = responses.byte(byte, &mut client.output)? {
                if response {
                    client.send(&bytes)?;
                } else {
                    let Some(bytes) = borrowed_keyboard(&mut controls, &bytes) else {
                        STOP.store(true, Ordering::Relaxed);
                        cancelled.store(true, Ordering::Relaxed);
                        continue;
                    };
                    queued.push(InputChunk::Keyboard(bytes));
                }
            }
        }
    })
}

fn attachment_command(
    destination: &Destination,
    binary: &OsStr,
    state_home: &std::path::Path,
) -> Command {
    let mut command = destination.command(binary);
    command.env("XDG_STATE_HOME", state_home);
    command
}

fn deliver_user(
    client: &mut Client,
    pending: &mut Option<(super::handoff::Selection, Vec<InputChunk>)>,
    bytes: Vec<u8>,
) -> Result<()> {
    if let Some((_, queued)) = pending.as_mut() {
        queued.push(InputChunk::Keyboard(bytes));
        Ok(())
    } else {
        client.send_user(&bytes)
    }
}

pub fn run(args: &[String]) -> Result<()> {
    let mut session = env::var("HERDR_SESSION").unwrap_or_else(|_| "default".into());
    let mut initial = None;
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--session" => {
                session = args
                    .next()
                    .context("missing --session value; use --help")?
                    .clone()
            }
            "--machine" => {
                initial = Some(
                    args.next()
                        .context("missing --machine value; use --help")?
                        .clone(),
                )
            }
            _ => bail!("unknown launch argument {argument}; use --help"),
        }
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("launch requires a local interactive terminal");
    }
    if session.is_empty() || session.starts_with('-') || session.chars().any(char::is_control) {
        bail!("invalid local session name");
    }
    let binary = binary();
    let original_state_home = env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/state"))
        })
        .context("XDG_STATE_HOME or HOME is required")?;
    let mut mirror = super::state::StateMirror::create(&original_state_home)?;
    let local = Destination::Local {
        session: session.clone(),
    };
    let mut active = match initial {
        None => local.clone(),
        Some(id) if id == "local" => local.clone(),
        Some(id) => Destination::Remote(
            catalog::load_in(&binary, Some(&original_state_home))?
                .into_iter()
                .find(|profile| profile.id == id)
                .context("initial machine is missing or disabled")?,
        ),
    };
    catalog::preflight_in(&binary, &active, &STOP, Some(&original_state_home))?;
    let mut native = if matches!(active, Destination::Local { .. }) {
        match super::navigator::Bridge::create(&binary, &original_state_home, &session) {
            Ok(bridge) => Some(bridge),
            Err(error) => {
                eprintln!("herdrpp: {error:#}; using reattachment");
                None
            }
        }
    } else {
        None
    };
    let mut native_client = false;
    let _signals = Signals::install();
    let host = Host::new()?;
    let mut client = Client::spawn(attachment_command(&active, &binary, mirror.private_home()))?;
    host.raw()?;
    let mut retry: Option<(super::handoff::Selection, String)> = None;
    let mut pending_focus: Option<(super::handoff::Selection, Vec<InputChunk>)> = None;
    let mut previous: Option<Destination> = None;
    let mut keys = Input::configured()?;
    let mut responses = super::responses::Responses::default();
    let mut pending_user = std::collections::VecDeque::new();
    let mut reconcile_at = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while !STOP.load(Ordering::Relaxed) {
        if pending_focus.is_some() && client.startup_expired() {
            client.output.input_ready = false;
            eprintln!(
                "\r\nherdrpp: terminal setup did not complete within 30 seconds; restoring previous machine\r"
            );
            let failed_selection = pending_focus.take().map(|(selection, _)| selection);
            discard_failed_input(&host, &mut pending_user, &mut keys, &mut responses)?;
            if let Some(original) = previous.take() {
                native_client = false;
                drop(client);
                native = None;
                client = Client::spawn(attachment_command(
                    &original,
                    &binary,
                    mirror.private_home(),
                ))?;
                active = original;
            }
            if let Some(selection) = failed_selection {
                retry = Some((
                    selection,
                    "Terminal setup timed out; original machine restored".into(),
                ));
                pending_user.push_front(PendingInput::Byte(0x1d));
            }
        }
        let failure = match client.drain() {
            Err(error) => Some(error),
            Ok(()) => match client.child.try_wait()? {
                Some(status) if status.success() => return Ok(()),
                Some(status) => Some(anyhow::anyhow!("Herdr attachment failed ({status})")),
                None => None,
            },
        };
        if let Some(error) = failure {
            let failed_selection = pending_focus.take().map(|(selection, _)| selection);
            discard_failed_input(&host, &mut pending_user, &mut keys, &mut responses)?;
            if let Some(restore) = previous.take() {
                eprintln!("\r\nherdrpp: {error:#}; restoring previous machine\r");
                drop(client);
                active = restore;
                client = Client::spawn(attachment_command(&active, &binary, mirror.private_home()))
                    .with_context(|| format!("{error:#}; restoring previous machine failed"))?;
                if let Some(selection) = failed_selection {
                    retry = Some((selection, format!("{error:#}")));
                    pending_user.push_front(PendingInput::Byte(0x1d));
                }
                continue;
            }
            return Err(error.context("no previous attachment remains"));
        }
        if client.output.input_ready
            && let Some((selection, queued)) = pending_focus.take()
        {
            let api = crate::picker::api::MachineApi {
                binary: &binary,
                machine: &selection.machine,
                local_session: &session,
                profile: selection.profile.as_ref(),
            };
            match probe(&mut client, &mut responses, |cancelled| {
                if selection.machine != "local" {
                    let current = catalog::load_in(&binary, Some(&original_state_home))?;
                    anyhow::ensure!(
                        current
                            .iter()
                            .any(|profile| selection.profile.as_ref().is_some_and(
                                |browsed| profile.id == browsed.id
                                    && profile.target == browsed.target
                                    && profile.session == browsed.session
                            )),
                        "selected machine was removed, disabled or changed"
                    );
                }
                api.focus_cancellable(&selection.target, cancelled)
            }) {
                Ok(input) => {
                    release_focused_input(&mut pending_user, queued, input);
                    previous = None;
                    retry = None;
                }
                Err(error) => {
                    discard_failed_input(&host, &mut pending_user, &mut keys, &mut responses)?;
                    eprintln!("\r\nherdrpp: {error:#}; restoring previous machine\r");
                    if let Some(original) = previous.take() {
                        native_client = false;
                        drop(client);
                        native = None;
                        client = Client::spawn(attachment_command(
                            &original,
                            &binary,
                            mirror.private_home(),
                        ))?;
                        active = original;
                    }
                    retry = Some((selection, format!("{error:#}")));
                    pending_user.push_front(PendingInput::Byte(0x1d));
                }
            }
        }
        if RESIZE.swap(false, Ordering::Relaxed) {
            client.resize()?;
        }
        // This loop is suspended while the picker owns the terminal or native
        // aliases are installed, so reconciliation never persists inspection data.
        if native_client && std::time::Instant::now() >= reconcile_at {
            reconcile_at = std::time::Instant::now() + std::time::Duration::from_secs(1);
            if let Some(bridge) = native.as_mut()
                && bridge.synchronize()?
            {
                client.send(b"\x1c")?;
            }
        }
        if let Some(bytes) = responses.flush_timeout(&client.output)? {
            // The response decoder already resolved this incomplete escape as literal
            // keyboard input. Do not start a second control-string interpretation.
            deliver_user(&mut client, &mut pending_focus, bytes)?;
        }
        if let Some(Action::Bytes(bytes)) = keys.flush() {
            deliver_user(&mut client, &mut pending_focus, bytes)?;
        }
        let byte = if let Some(input) = pending_user.pop_front() {
            match input {
                PendingInput::Byte(byte) => byte,
                PendingInput::Literal(bytes) => {
                    deliver_user(&mut client, &mut pending_focus, bytes)?;
                    continue;
                }
            }
        } else {
            let ready = terminal::poll(&[0, client.master.as_raw_fd()], libc::POLLIN, 20)?;
            if !ready[0] || client.queued_input() > 1024 * 1024 {
                continue;
            }
            let byte = match host_byte()? {
                HostInput::Byte(byte) => byte,
                HostInput::Eof => return Ok(()),
                HostInput::Interrupted => continue,
            };
            let Some((response, bytes)) = responses.byte(byte, &mut client.output)? else {
                continue;
            };
            if response {
                client.send(&bytes)?;
            } else {
                pending_user.extend(bytes.into_iter().map(PendingInput::Byte));
            }
            continue;
        };
        for action in keys.byte(byte) {
            match action {
                Action::Bytes(bytes) => deliver_user(&mut client, &mut pending_focus, bytes)?,
                Action::Quit => return Ok(()),
                Action::Picker => {
                    if let Some((_, queued)) = pending_focus.as_mut() {
                        queued.push(InputChunk::Keyboard(vec![0x1d]));
                        continue;
                    }
                    if native_client {
                        if let Some(bridge) = native.as_mut()
                            && bridge.synchronize()?
                        {
                            client.send(b"\x1c")?;
                        }
                    } else {
                        mirror.synchronize()?;
                    }
                    let actual_id = if native_client {
                        native
                            .as_ref()
                            .and_then(|bridge| bridge.active_hint())
                            .unwrap_or_else(|| active.id().to_owned())
                    } else {
                        active.id().to_owned()
                    };
                    let choice = match choose(
                        &mut client,
                        &mut responses,
                        &actual_id,
                        &original_state_home,
                        &session,
                        retry.as_ref(),
                    ) {
                        Ok(choice) => choice,
                        Err(error) => {
                            discard_failed_input(
                                &host,
                                &mut pending_user,
                                &mut keys,
                                &mut responses,
                            )?;
                            eprintln!("\r\nherdrpp: {error:#}\r");
                            continue;
                        }
                    };
                    let super::handoff::PickerChoice { selection, queued } = choice;
                    if let Some(selection) = selection {
                        let id = selection.machine.clone();
                        if id == actual_id && !native_client {
                            pending_focus = Some((selection, queued));
                            continue;
                        }
                        let selected = if id == "local" {
                            Ok(local.clone())
                        } else {
                            catalog::load_in(&binary, Some(&original_state_home)).and_then(
                                |profiles| {
                                    profiles
                                        .into_iter()
                                        .find(|profile| {
                                            profile.id == id
                                                && selection.profile.as_ref().is_some_and(
                                                    |browsed| {
                                                        browsed.target == profile.target
                                                            && browsed.session == profile.session
                                                    },
                                                )
                                        })
                                        .map(Destination::Remote)
                                        .context(
                                            "selected machine was removed, disabled or changed",
                                        )
                                },
                            )
                        };
                        let mut typeahead = queued;
                        let selected = match selected.and_then(|selected| {
                            typeahead.extend(check_destination(
                                &mut client,
                                &mut responses,
                                &binary,
                                &selected,
                                &original_state_home,
                                &selection,
                                &session,
                            )?);
                            Ok(selected)
                        }) {
                            Ok(selected) => selected,
                            Err(error) => {
                                discard_failed_input(
                                    &host,
                                    &mut pending_user,
                                    &mut keys,
                                    &mut responses,
                                )?;
                                eprintln!("\r\nherdrpp: {error:#}\r");
                                retry = Some((selection, format!("{error:#}")));
                                pending_user.push_front(PendingInput::Byte(0x1d));
                                continue;
                            }
                        };
                        let mut known_previous = !native_client;
                        if let Some(bridge) = native.as_mut() {
                            let upgrade = if !native_client {
                                // Spawn before dropping bootstrap: a native launch error
                                // still has the original owned viewer for fallback.
                                match Client::spawn(bridge.command(&binary)) {
                                    Ok(candidate) => {
                                        drop(client);
                                        client = candidate;
                                        native_client = true;
                                        host.raw()?;
                                        Ok(())
                                    }
                                    Err(error) => Err(error.context("start native viewing client")),
                                }
                            } else {
                                Ok(())
                            };
                            let mut controls = borrowed_controls(&responses);
                            let result = upgrade.and_then(|()| {
                                bridge.switch(&mut client, &selected, |client| {
                                    if STOP.load(Ordering::Relaxed) {
                                        bail!("native switch interrupted");
                                    }
                                    if RESIZE.swap(false, Ordering::Relaxed) {
                                        client.resize()?;
                                        bail!("terminal resized during native inspection");
                                    }
                                    if let Some(bytes) = responses.flush_timeout(&client.output)? {
                                        typeahead.push(InputChunk::Literal(bytes));
                                    }
                                    let ready = terminal::poll(
                                        &[0, client.master.as_raw_fd()],
                                        libc::POLLIN,
                                        20,
                                    )?;
                                    if ready[0] {
                                        match host_byte()? {
                                            HostInput::Eof => {
                                                STOP.store(true, Ordering::Relaxed);
                                                bail!("host input closed");
                                            }
                                            HostInput::Interrupted => {}
                                            HostInput::Byte(byte) => {
                                                if let Some((response, bytes)) =
                                                    responses.byte(byte, &mut client.output)?
                                                {
                                                    if response {
                                                        client.send(&bytes)?;
                                                    } else {
                                                        let Some(bytes) = borrowed_keyboard(
                                                            &mut controls,
                                                            &bytes,
                                                        ) else {
                                                            STOP.store(true, Ordering::Relaxed);
                                                            bail!("native switch interrupted");
                                                        };
                                                        typeahead.push(InputChunk::Keyboard(bytes));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Ok(())
                                })
                            });
                            if STOP.load(Ordering::Relaxed) {
                                return Ok(());
                            }
                            if let Some(prior_id) = bridge.verified_previous() {
                                known_previous = true;
                                active = if prior_id == "local" {
                                    local.clone()
                                } else {
                                    Destination::Remote(
                                        catalog::load_in(&binary, Some(&original_state_home))?
                                            .into_iter()
                                            .find(|profile| profile.id == prior_id)
                                            .context("verified previous machine disappeared")?,
                                    )
                                };
                            }
                            match result {
                                Ok(()) => {
                                    previous = Some(active.clone());
                                    active = selected;
                                    pending_focus = Some((selection, typeahead));
                                    continue;
                                }
                                Err(error) => {
                                    eprintln!(
                                        "\r\nherdrpp: native switch unavailable: {error:#}; using reattachment\r"
                                    );
                                }
                            }
                            native_client = false;
                            // Keep the mirror alive until its viewing client is stopped.
                        }
                        // A concurrent catalog edit must not redirect buffered input to
                        // the old endpoint behind a now-redefined or removed profile.
                        if !native_client
                            && native.is_some()
                            && let Destination::Remote(profile) = &selected
                        {
                            let unchanged = catalog::load_in(&binary, Some(&original_state_home))?
                                .into_iter()
                                .any(|current| {
                                    current.id == profile.id
                                        && current.target == profile.target
                                        && current.session == profile.session
                                });
                            if !unchanged {
                                discard_failed_input(
                                    &host,
                                    &mut pending_user,
                                    &mut keys,
                                    &mut responses,
                                )?;
                                bail!(
                                    "selected machine changed during native inspection; buffered input was discarded; retry launch"
                                );
                            }
                        }
                        let original = active.clone();
                        drop(client);
                        if !native_client {
                            native = None;
                            mirror.synchronize()?;
                        }
                        match Client::spawn(attachment_command(
                            &selected,
                            &binary,
                            mirror.private_home(),
                        )) {
                            Ok(candidate) => {
                                client = candidate;
                                previous = known_previous.then_some(original);
                                active = selected;
                                pending_focus = Some((selection, typeahead));
                            }
                            Err(error) => {
                                discard_failed_input(
                                    &host,
                                    &mut pending_user,
                                    &mut keys,
                                    &mut responses,
                                )?;
                                if !known_previous {
                                    return Err(error.context("no verified prior machine remains; buffered input was discarded"));
                                }
                                eprintln!("\r\nherdrpp: {error:#}; restoring previous machine\r");
                                client = Client::spawn(attachment_command(
                                    &original,
                                    &binary,
                                    mirror.private_home(),
                                ))
                                .with_context(|| {
                                    format!("{error:#}; restoring previous machine failed")
                                })?;
                                active = original;
                                previous = None;
                            }
                        }
                        host.raw()?;
                    } else {
                        retry = None;
                        queue_input(&mut pending_user, queued);
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn picker(args: &[String]) -> Result<()> {
    if args.len() != 3 {
        bail!("invalid internal machine-picker arguments; use launch");
    }
    let descriptor: i32 = args[0].parse().context("invalid result descriptor")?;
    let mut metadata = std::mem::MaybeUninit::uninit();
    // SAFETY: fcntl only inspects the supplied descriptor, and fstat writes valid stat storage.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if descriptor < 3 || flags < 0 || unsafe { libc::fstat(descriptor, metadata.as_mut_ptr()) } < 0
    {
        bail!("machine-picker requires an inherited writable result pipe; use --help");
    }
    let metadata = unsafe { metadata.assume_init() };
    if metadata.st_mode & libc::S_IFMT != libc::S_IFIFO || flags & libc::O_ACCMODE == libc::O_RDONLY
    {
        bail!("machine-picker requires an inherited writable result pipe; use --help");
    }
    super::picker::run(OsStr::new(&binary()), descriptor, &args[1], &args[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_bytes(input: PendingInput) -> Vec<u8> {
        match input {
            PendingInput::Byte(byte) => vec![byte],
            PendingInput::Literal(bytes) => bytes,
        }
    }

    #[test]
    fn focus_completion_preserves_input_order_across_all_three_buffers() {
        let mut pending = std::collections::VecDeque::from([
            PendingInput::Byte(b'C'),
            PendingInput::Literal(b"AL".to_vec()),
        ]);
        release_focused_input(
            &mut pending,
            vec![InputChunk::Keyboard(b"printf 'HIDDEN_LO".to_vec())],
            vec![InputChunk::Keyboard(b"_ROUTE\\n'\r".to_vec())],
        );
        assert_eq!(
            pending
                .into_iter()
                .flat_map(pending_bytes)
                .collect::<Vec<_>>(),
            b"printf 'HIDDEN_LOCAL_ROUTE\\n'\r"
        );
    }
}
