use crate::pr::{
    github::{self, Checkout, LookupFailure, WorkspaceStatus},
    herdr,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ffi::CString,
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Write},
    os::fd::AsRawFd,
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

struct Wakeup(File);

impl Wakeup {
    fn open(path: &Path) -> Result<Self> {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("remove old watcher wakeup pipe"),
        }
        let name = CString::new(path.as_os_str().as_encoded_bytes())?;
        // SAFETY: name is a valid NUL-terminated path; mkfifo retains no pointer.
        if unsafe { libc::mkfifo(name.as_ptr(), 0o600) } != 0 {
            return Err(std::io::Error::last_os_error()).context("create watcher wakeup pipe");
        }
        Ok(Self(
            OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(path)
                .context("open watcher wakeup pipe")?,
        ))
    }

    fn wait(&mut self, deadline: Instant) -> Result<()> {
        loop {
            let mut descriptor = libc::pollfd {
                fd: self.0.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
            // SAFETY: descriptor is valid for one pollfd and the fd stays open.
            let result = unsafe { libc::poll(&mut descriptor, 1, timeout) };
            if result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error).context("wait for watcher wakeup");
            }
            if result > 0 {
                ensure!(
                    descriptor.revents & libc::POLLIN != 0,
                    "watcher wakeup pipe failed"
                );
                let mut bytes = [0; 128];
                while self.0.read(&mut bytes).is_ok_and(|count| count > 0) {}
            }
            return Ok(());
        }
    }
}

const EXECUTABLE_ENV: &str = "HERDRPP_EXECUTABLE";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Executable {
    path: PathBuf,
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
}

impl Executable {
    fn at(path: PathBuf) -> Result<Self> {
        let metadata = fs::metadata(&path).context("inspect PR watcher executable")?;
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
        })
    }

    fn current() -> Result<Self> {
        Self::at(std::env::current_exe().context("locate PR watcher executable")?)
    }

    fn reload_if_changed(&self, plugin_root: Option<&Path>) -> Result<()> {
        let path = match plugin_root {
            Some(root) => {
                ensure!(root.is_absolute(), "installed plugin root must be absolute");
                root.join(concat!("target/release/", env!("CARGO_PKG_NAME")))
            }
            None => self.path.clone(),
        };
        let replacement = Self::at(path)?;
        if replacement != *self {
            eprintln!("PR watcher binary changed; reloading");
            // The session lock closes on successful exec, before the new watcher
            // acquires it. On failure this process keeps its lock and retries.
            let mut command = Command::new(&replacement.path);
            command
                .arg("watch")
                .env(EXECUTABLE_ENV, serde_json::to_string(&replacement)?);
            if let Some(root) = plugin_root {
                command.current_dir(root).env("HERDR_PLUGIN_ROOT", root);
            }
            return Err(command.exec()).context("reload updated PR watcher");
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct WatcherIdentity {
    pid: u32,
    executable: Executable,
}

pub struct Session {
    directory: PathBuf,
    socket: PathBuf,
    identity: (u64, u64),
}

impl Session {
    pub fn load() -> Result<Self> {
        let socket = PathBuf::from(
            std::env::var_os("HERDR_SOCKET_PATH")
                .context("HERDR_SOCKET_PATH is required; run through a Herdr plugin action")?,
        );
        ensure!(socket.is_absolute(), "HERDR_SOCKET_PATH must be absolute");
        let metadata = fs::metadata(&socket).context("inspect Herdr session socket")?;
        let state = PathBuf::from(
            std::env::var_os("HERDR_PLUGIN_STATE_DIR")
                .context("HERDR_PLUGIN_STATE_DIR is required")?,
        );
        ensure!(
            state.is_absolute(),
            "HERDR_PLUGIN_STATE_DIR must be absolute"
        );
        let hash = socket
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .fold(0xcbf29ce484222325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
            });
        Ok(Self {
            directory: state.join(format!(
                "session-{hash:016x}-{:x}-{:x}",
                metadata.dev(),
                metadata.ino()
            )),
            socket,
            identity: (metadata.dev(), metadata.ino()),
        })
    }

    pub(crate) fn state_directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn same_session(&self) -> bool {
        fs::metadata(&self.socket).is_ok_and(|m| (m.dev(), m.ino()) == self.identity)
    }

    fn stopped(&self) -> bool {
        self.directory.join("stop").exists() || !self.same_session()
    }

    fn file(&self, name: &str) -> Result<File> {
        fs::create_dir_all(&self.directory)?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(self.directory.join(name))
            .context("open session lock/log")
    }

    fn lock(&self, name: &str) -> Result<Option<File>> {
        let file = self.file(name)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn running_executable_matches(&self, expected: &Executable, pid: Option<u32>) -> Result<bool> {
        let bytes = match fs::read(self.directory.join("watcher.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error).context("read PR watcher identity"),
        };
        let Ok(owner) = serde_json::from_slice::<WatcherIdentity>(&bytes) else {
            return Ok(false);
        };
        let Ok(owner_pid) = libc::pid_t::try_from(owner.pid) else {
            return Ok(false);
        };
        // SAFETY: signal zero only checks whether the recorded process exists.
        let running = owner_pid > 0 && unsafe { libc::kill(owner_pid, 0) } == 0;
        Ok(running && owner.executable == *expected && pid.is_none_or(|pid| pid == owner.pid))
    }

    fn request_stop(&self) -> Result<()> {
        fs::write(self.directory.join("stop"), []).context("request PR watcher shutdown")?;
        // Older watchers have no wakeup pipe and observe the stop marker by polling.
        if let Ok(mut pipe) = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(self.directory.join("wake.pipe"))
        {
            let _ = pipe.write_all(&[1]);
        }
        Ok(())
    }

    pub fn start(&self) -> Result<()> {
        let _control = self
            .lock("control.lock")?
            .context("another start/stop is in progress")?;
        ensure!(herdr::enabled()?, "plugin is not installed and enabled");
        let executable = Executable::current()?;
        let mut restart = self.directory.join("stop").exists();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(probe) = self.lock("watch.lock")? {
                drop(probe);
                break;
            }
            if !restart && self.running_executable_matches(&executable, None)? {
                println!("PR watcher is already running");
                return Ok(());
            }
            if !restart {
                println!("Replacing outdated PR watcher");
                self.request_stop()?;
                restart = true;
            }
            ensure!(
                Instant::now() < deadline,
                "previous watcher is still stopping; retry start shortly"
            );
            thread::sleep(Duration::from_millis(20));
        }
        match fs::remove_file(self.directory.join("stop")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        ensure!(herdr::enabled()?, "plugin is not installed and enabled");
        let log = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(self.directory.join("watch.log"))?;
        let mut command = Command::new(&executable.path);
        command
            .arg("watch")
            .env(EXECUTABLE_ENV, serde_json::to_string(&executable)?)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        // SAFETY: setsid has no pointer arguments and is async-signal-safe.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().context("launch PR watcher")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait()? {
                if status.success() && self.running_executable_matches(&executable, None)? {
                    println!("PR watcher is already running");
                    return Ok(());
                }
                bail!("PR watcher exited ({status}); inspect watch.log");
            }
            if self.running_executable_matches(&executable, Some(child.id()))? {
                println!(
                    "PR watcher started; diagnostics: {}",
                    self.directory.join("watch.log").display()
                );
                return Ok(());
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("PR watcher did not start within five seconds");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn stop(&self) -> Result<()> {
        let _control = self
            .lock("control.lock")?
            .context("another start/stop is in progress")?;
        self.request_stop()?;
        println!("Stop requested; PR tokens expire within 180 seconds");
        Ok(())
    }

    pub fn watch(&self) -> Result<()> {
        let Some(_watch) = self.lock("watch.lock")? else {
            return Ok(());
        };
        let executable = match std::env::var(EXECUTABLE_ENV) {
            Ok(value) => {
                serde_json::from_str(&value).context("parse launched executable identity")?
            }
            Err(std::env::VarError::NotPresent) => Executable::current()?,
            Err(error) => return Err(error).context("read launched executable identity"),
        };
        let mut wakeup = Wakeup::open(&self.directory.join("wake.pipe"))?;
        let owner = WatcherIdentity {
            pid: std::process::id(),
            executable,
        };
        fs::write(
            self.directory.join("watcher.json"),
            serde_json::to_vec(&owner)?,
        )
        .context("record PR watcher identity")?;
        eprintln!("PR watcher started");
        let mut reload_error = None;
        let mut refresh_at = Instant::now();
        while !self.stopped() {
            let installation = match herdr::installation() {
                Ok(Some(plugin)) if plugin.enabled => plugin,
                Ok(_) => break,
                Err(error) => {
                    eprintln!("PR watcher lost session: {error:#}");
                    break;
                }
            };
            match owner
                .executable
                .reload_if_changed(installation.plugin_root.as_deref())
            {
                Ok(()) => reload_error = None,
                Err(error) => {
                    let message = format!("{error:#}");
                    if reload_error.as_ref() != Some(&message) {
                        eprintln!(
                            "PR watcher update deferred; current watcher retained: {message}"
                        );
                    }
                    reload_error = Some(message);
                }
            }
            if Instant::now() >= refresh_at {
                if let Err(error) = self.refresh(false) {
                    eprintln!("PR refresh failed: {error:#}");
                }
                refresh_at = Instant::now() + REFRESH_INTERVAL;
            }
            if self.stopped() || !herdr::enabled()? {
                break;
            }
            wakeup.wait(refresh_at)?;
        }
        fs::remove_file(self.directory.join("wake.pipe"))
            .context("remove stopped watcher wakeup pipe")?;
        match fs::remove_file(self.directory.join("watcher.json")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("remove stopped PR watcher identity"),
        }
        eprintln!("PR watcher stopped");
        Ok(())
    }

    pub fn refresh(&self, manual: bool) -> Result<()> {
        let Some(_refresh) = self.lock("refresh.lock")? else {
            if manual {
                println!("A PR refresh is already in progress");
            }
            return Ok(());
        };
        let snapshot = herdr::snapshot()?;
        let mut cache: HashMap<
            Checkout,
            std::result::Result<Option<WorkspaceStatus>, LookupFailure>,
        > = HashMap::new();
        let mut failures = 0;
        let mut reported = 0;
        for workspace in &snapshot.workspaces {
            if !self.same_session() || (!manual && (self.stopped() || !herdr::enabled()?)) {
                break;
            }
            let result = (|| -> Result<()> {
                let original = snapshot
                    .directory(workspace)
                    .map(github::checkout)
                    .transpose()?
                    .flatten();
                let pr = match &original {
                    Some(checkout) => cache
                        .entry(checkout.clone())
                        .or_insert_with(|| github::lookup(checkout))
                        .clone(),
                    None => Ok(None),
                };
                let fresh = herdr::snapshot()?;
                let Some(current) = fresh
                    .workspaces
                    .iter()
                    .find(|w| w.workspace_id == workspace.workspace_id)
                else {
                    return Ok(());
                };
                let current_checkout = fresh
                    .directory(current)
                    .map(github::checkout)
                    .transpose()?
                    .flatten();
                if !self.same_session() || (!manual && (self.stopped() || !herdr::enabled()?)) {
                    return Ok(());
                }
                if original != current_checkout {
                    herdr::report(&workspace.workspace_id, None)?;
                } else {
                    match pr {
                        Ok(status) => herdr::report(&workspace.workspace_id, status)?,
                        Err(error) => {
                            if error.default_branch {
                                herdr::clear_pr(&workspace.workspace_id)?;
                            }
                            return Err(error.into());
                        }
                    }
                }
                reported += 1;
                Ok(())
            })();
            if let Err(error) = result {
                failures += 1;
                eprintln!("workspace {}: {error:#}", workspace.workspace_id);
            }
        }
        eprintln!("PR refresh: {reported} workspace reports, {failures} failures");
        ensure!(
            failures == 0,
            "{failures} workspace PR lookups/reports failed"
        );
        Ok(())
    }
}

pub fn check() -> Result<()> {
    let snapshot = herdr::snapshot()?;
    let mut cache = HashMap::new();
    for workspace in &snapshot.workspaces {
        let checkout = snapshot
            .directory(workspace)
            .map(github::checkout)
            .transpose()?
            .flatten();
        let pr = match checkout {
            Some(checkout) => {
                if let Some(pr) = cache.get(&checkout) {
                    *pr
                } else {
                    let pr = github::lookup(&checkout)?;
                    cache.insert(checkout, pr);
                    pr
                }
            }
            None => None,
        };
        println!(
            "{}: {}",
            workspace.workspace_id,
            match pr {
                Some(WorkspaceStatus::PullRequest(pr)) => format!("#{}", pr.number),
                Some(WorkspaceStatus::DefaultBranch { ci: Some(ci) }) =>
                    format!("CI {}", ci.icon()),
                Some(WorkspaceStatus::DefaultBranch { ci: None }) => "no CI status".into(),
                None => "no associated PR".into(),
            }
        );
    }
    Ok(())
}
