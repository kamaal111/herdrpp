//! Native switching transactions on Herdr 0.9.5, with explicit reattachment fallback.
use super::{
    catalog::Destination,
    navigator_config::{ConfigMirror, ControlBinding},
    navigator_screen::{Navigator, NavigatorScreen},
    navigator_state::StateMirror,
    terminal::{self, Client},
};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

pub struct Bridge {
    temporary: tempfile::TempDir,
    state: StateMirror,
    config: ConfigMirror,
    aliases: BTreeMap<String, String>,
    session: String,
    inspection: bool,
    inspection_catalog: Option<serde_json::Value>,
    verified_previous: Option<String>,
}

impl Bridge {
    pub fn create(binary: &OsStr, original_home: &Path, session: &str) -> Result<Self> {
        let version = crate::command::run(
            Command::new(binary).arg("--version"),
            Duration::from_secs(5),
        )?;
        if !String::from_utf8_lossy(&version)
            .split_whitespace()
            .any(|word| word == "0.9.5")
        {
            bail!("native controller requires Herdr 0.9.5; using reattachment");
        }
        let temporary = tempfile::Builder::new()
            .prefix("herdrpp-native-")
            .tempdir()?;
        let private = temporary.path().join("herdr");
        let state = StateMirror::create(&original_home.join("herdr"), &private)?;
        fs::create_dir_all(private.join("client"))?;
        fs::write(
            private.join("client/endpoint-selection.json"),
            br#"{"version":1,"selected_profile":null}"#,
        )?;
        let user_config = crate::configure::viewing_config_path()?;
        let template = crate::command::run(
            Command::new(binary).arg("--default-config"),
            Duration::from_secs(5),
        )?;
        let template =
            std::str::from_utf8(&template).context("invalid Herdr default config encoding")?;
        let controls = vec![
            ControlBinding {
                action: "goto".into(),
                key: "ctrl+]".into(),
                defaults: super::navigator_config::default_binding(template, "goto")?,
            },
            ControlBinding {
                action: "reload_config".into(),
                key: "ctrl+backslash".into(),
                defaults: super::navigator_config::default_binding(template, "reload_config")?,
            },
        ];
        let config = ConfigMirror::create(
            &user_config,
            &temporary.path().join("config.toml"),
            controls,
            &original_home.join("herdrpp/native-config"),
        )?;
        Ok(Self {
            temporary,
            state,
            config,
            aliases: BTreeMap::new(),
            session: session.into(),
            inspection: false,
            inspection_catalog: None,
            verified_previous: None,
        })
    }

    pub fn command(&self, binary: &OsStr) -> Command {
        let mut command = Destination::Local {
            session: self.session.clone(),
        }
        .command(binary);
        command.env("XDG_STATE_HOME", self.temporary.path()).env(
            "HERDR_CONFIG_PATH",
            self.temporary.path().join("config.toml"),
        );
        command
    }

    pub fn synchronize(&mut self) -> Result<bool> {
        let changed = self.config.synchronize()?;
        self.state.synchronize()?;
        Ok(changed)
    }

    pub fn verified_previous(&self) -> Option<&str> {
        self.verified_previous.as_deref()
    }

    /// Used only to highlight the picker, never to acknowledge activation.
    pub fn active_hint(&self) -> Option<String> {
        let bytes = fs::read(
            self.temporary
                .path()
                .join("herdr/client/endpoint-selection.json"),
        )
        .ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        Some(
            value
                .get("selected_profile")?
                .as_str()
                .unwrap_or("local")
                .to_owned(),
        )
    }

    fn catalog_path(&self) -> PathBuf {
        self.temporary.path().join("herdr/client/endpoints.json")
    }

    fn begin(&mut self) -> Result<()> {
        self.state.begin_inspection()?;
        self.inspection = true;
        let path = self.catalog_path();
        let mut catalog: serde_json::Value = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                serde_json::json!({"version":1,"ssh":[]})
            }
            Err(error) => return Err(error.into()),
        };
        self.inspection_catalog = Some(catalog.clone());
        let profiles = catalog["ssh"]
            .as_array_mut()
            .context("invalid private machine catalog")?;
        self.aliases.clear();
        for (index, profile) in profiles.iter_mut().enumerate() {
            if profile.get("enabled").and_then(serde_json::Value::as_bool) == Some(false) {
                continue;
            }
            let id = profile["id"]
                .as_str()
                .context("missing machine id")?
                .to_owned();
            // Random per-viewer prefix; short ASCII labels survive native truncation.
            let nonce = self
                .temporary
                .path()
                .file_name()
                .context("private directory name")?
                .to_string_lossy();
            let alias = format!("pp{}x{index}", nonce.trim_start_matches("herdrpp-native-"));
            profile["label"] = alias.clone().into();
            self.aliases.insert(id, alias);
        }
        crate::configure::atomic_write(&path, &serde_json::to_vec(&catalog)?, None)?;
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        if self.inspection {
            self.state.end_inspection()?;
            self.inspection = false;
            let restored: serde_json::Value = match fs::read(self.catalog_path()) {
                Ok(bytes) => serde_json::from_slice(&bytes)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    serde_json::json!({"version":1,"ssh":[]})
                }
                Err(error) => return Err(error.into()),
            };
            if self
                .inspection_catalog
                .take()
                .is_some_and(|before| before != restored)
            {
                bail!(
                    "machine catalog changed during native inspection; native success was not accepted"
                );
            }
        }
        Ok(())
    }

    /// `pump` owns FD0: terminal replies go to this client, user input stays queued.
    pub fn switch(
        &mut self,
        client: &mut Client,
        selected: &Destination,
        mut pump: impl FnMut(&mut Client) -> Result<()>,
    ) -> Result<()> {
        self.verified_previous = None;
        let size = terminal::geometry()?;
        if size.ws_col < 70 || size.ws_row < 18 {
            bail!("native navigator viewport is too small; using reattachment");
        }
        if self.synchronize()? {
            client.send(b"\x1c")?;
        }
        self.begin()?;
        let mut screen = NavigatorScreen::new(size.ws_row, size.ws_col);
        let operation = (|| {
            if self.aliases.is_empty() {
                bail!(
                    "native switching needs an enabled remote profile for its catalog completion fence"
                );
            }
            let alias = if selected.id() == "local" {
                "Local".to_owned()
            } else {
                self.aliases
                    .get(selected.id())
                    .context("native target disappeared from catalog")?
                    .clone()
            };
            client.signal(libc::SIGWINCH)?;
            Self::open(client, &mut screen, &mut pump).context("open native navigator")?;
            // Prove this whole catalog generation was applied before later
            // acknowledging its removal. An empty search alone could otherwise
            // precede a queued alias-catalog update on Local -> Local.
            if let Some(probe) = self.aliases.values().next() {
                Self::search(client, &mut screen, probe, &mut pump, |n| {
                    n.machine_rows.len() == 1 && n.machine_rows[0].1 == *probe
                })
                .context("acknowledge native alias catalog generation")?;
                Self::close(client, &mut screen, &mut pump)?;
                Self::open(client, &mut screen, &mut pump)?;
            }
            let prior_id = self
                .active_hint()
                .context("native prior machine preference is unavailable")?;
            let prior_alias = if prior_id == "local" {
                "Local".to_owned()
            } else {
                self.aliases
                    .get(&prior_id)
                    .context("native prior machine is missing from catalog")?
                    .clone()
            };
            Self::filter(client, &mut screen, &prior_alias, &mut pump)
                .context("inspect prior native machine")?;
            Self::select_header(client, &mut screen, &prior_alias, &mut pump)?;
            if screen
                .navigator()?
                .and_then(|n| n.current_machine)
                .as_deref()
                != Some(&prior_alias)
            {
                bail!("native prior machine has no verified current pane");
            }
            self.verified_previous = Some(prior_id);
            Self::close(client, &mut screen, &mut pump)?;
            Self::open(client, &mut screen, &mut pump)?;
            Self::filter(client, &mut screen, &alias, &mut pump).context("filter native alias")?;
            Self::select_header(client, &mut screen, &alias, &mut pump)?;
            client.send(b"\r")?;
            Self::wait(client, &mut screen, &mut pump, |n| n.is_none())
                .context("commit native machine header")?;
            Self::open(client, &mut screen, &mut pump).context("open native navigator")?;
            Self::filter(client, &mut screen, &alias, &mut pump)
                .context("filter committed native alias")?;
            Self::select_header(client, &mut screen, &alias, &mut pump)?;
            if screen
                .navigator()?
                .and_then(|n| n.current_machine)
                .as_deref()
                != Some(&alias)
            {
                Self::wait(client, &mut screen, &mut pump, |n| {
                    n.is_some_and(|n| n.current_machine.as_deref() == Some(&alias))
                })
                .context("verify native committed current pane")?;
            }
            Ok(())
        })();
        // Restore the whole atomic catalog before allowing keyboard replay.
        // Herdr 0.9.5 applies catalog reloads only when pending_activation is
        // None, after shell_runtime unfreezes pane input. Observed alias ->
        // original-label generations therefore fence presentation completion;
        // the earlier current-pane projection alone is not that final fence.
        let restore = self.restore();
        if let Err(error) = operation {
            return match restore {
                Ok(()) => Err(error),
                Err(restore) => {
                    Err(error.context(format!("native state restoration also failed: {restore:#}")))
                }
            };
        }
        restore?;
        (|| {
            Self::close(client, &mut screen, &mut pump).context("close native navigator")?;
            if let Some(alias) = self.aliases.values().next() {
                Self::open(client, &mut screen, &mut pump).context("open native navigator")?;
                Self::search(client, &mut screen, alias, &mut pump, |n| {
                    n.machine_rows.is_empty()
                })
                .context("acknowledge restored native labels")?;
                Self::close(client, &mut screen, &mut pump).context("close native navigator")?;
            }
            client.signal(libc::SIGWINCH)?;
            Ok(())
        })()
    }

    fn select_header(
        client: &mut Client,
        screen: &mut NavigatorScreen,
        alias: &str,
        pump: &mut impl FnMut(&mut Client) -> Result<()>,
    ) -> Result<()> {
        if screen.navigator()?.is_some_and(|n| n.search_focused) {
            client.send(b"\x1b[27u")?;
            Self::wait(client, screen, pump, |n| {
                n.is_some_and(|n| !n.search_focused)
            })
            .context("leave native search")?;
        }
        // Herdr Home selects the first pane, not the first machine. Page upward
        // with its native Ctrl+U action; selection clamps at the first header.
        for _ in 0..128 {
            if screen
                .navigator()?
                .is_some_and(|n| n.selected_machine.as_deref() == Some(alias))
            {
                return Ok(());
            }
            client.send(b"\x15")?;
            Self::wait(client, screen, pump, |n| n.is_some())
                .context("move to native machine header")?;
        }
        bail!("native machine header could not be selected");
    }

    fn filter(
        client: &mut Client,
        screen: &mut NavigatorScreen,
        query: &str,
        pump: &mut impl FnMut(&mut Client) -> Result<()>,
    ) -> Result<()> {
        Self::search(client, screen, query, pump, |n| {
            n.machine_rows.len() == 1
                && n.machine_rows[0].1 == query
                && n.ready_machines.iter().any(|machine| machine == query)
                && n.pane_count > 0
        })
    }

    fn search(
        client: &mut Client,
        screen: &mut NavigatorScreen,
        query: &str,
        pump: &mut impl FnMut(&mut Client) -> Result<()>,
        matches: impl Fn(&Navigator) -> bool,
    ) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut requests = 0;
        while !screen.navigator()?.is_some_and(|n| n.search_focused) {
            if Instant::now() >= deadline {
                bail!("native search focus acknowledgement timed out");
            }
            if screen.navigator()?.is_none() {
                // A pending activation may close an acknowledged overlay. Reopen
                // before requesting focus so search text never enters the pane.
                Self::open_until(client, screen, pump, deadline)?;
            }
            // Focus requests can arrive while activation still changes the
            // shell. Confirm focus before sending text; a delayed extra slash
            // is harmless because the editor is cleared after all requests.
            client.send(b"/")?;
            requests += 1;
            Self::wait_for(client, screen, pump, Duration::from_millis(300), |n| {
                n.is_some_and(|n| n.search_focused)
            })?;
        }
        if requests > 1 || screen.navigator()?.is_some_and(|n| !n.query().is_empty()) {
            // These are modal editor actions only: Ctrl+E moves to the end,
            // Ctrl+U clears the prefix. Herdr repaints even an empty editor.
            client.send(b"\x05\x15")?;
            Self::wait(client, screen, pump, |n| {
                n.is_some_and(|n| n.search_focused && n.query().is_empty())
            })
            .context("clear acknowledged native search")?;
        }
        client.send(query.as_bytes())?;
        Self::wait(client, screen, pump, |n| {
            n.is_some_and(|n| n.search_focused && n.query() == query && matches(n))
        })
    }
    fn open(
        client: &mut Client,
        screen: &mut NavigatorScreen,
        pump: &mut impl FnMut(&mut Client) -> Result<()>,
    ) -> Result<()> {
        // Herdr ignores OpenNavigator while its overlay is already open.
        // Activation can discard an earlier request, so retry only timeouts;
        // resize, cancellation and transport errors must reach the caller.
        Self::open_until(
            client,
            screen,
            pump,
            Instant::now() + Duration::from_secs(15),
        )
    }

    fn open_until(
        client: &mut Client,
        screen: &mut NavigatorScreen,
        pump: &mut impl FnMut(&mut Client) -> Result<()>,
        deadline: Instant,
    ) -> Result<()> {
        loop {
            if Instant::now() >= deadline {
                bail!("native navigator acknowledgement timed out");
            }
            client.send_user(b"\x1d")?;
            if Self::wait_for(client, screen, pump, Duration::from_millis(300), |n| {
                n.is_some()
            })? {
                return Ok(());
            }
        }
    }

    fn close(
        client: &mut Client,
        screen: &mut NavigatorScreen,
        pump: &mut impl FnMut(&mut Client) -> Result<()>,
    ) -> Result<()> {
        if screen.navigator()?.is_some_and(|n| n.search_focused) {
            client.send(b"\x1b[27u")?;
            Self::wait(client, screen, pump, |n| {
                n.is_some_and(|n| !n.search_focused)
            })
            .context("leave native search")?;
        }
        if screen.navigator()?.is_some() {
            client.send(b"\x1b[27u")?;
            Self::wait(client, screen, pump, |n| n.is_none())?;
        }
        Ok(())
    }
    fn wait(
        client: &mut Client,
        screen: &mut NavigatorScreen,
        pump: &mut impl FnMut(&mut Client) -> Result<()>,
        predicate: impl Fn(Option<&Navigator>) -> bool,
    ) -> Result<()> {
        if Self::wait_for(client, screen, pump, Duration::from_secs(15), predicate)? {
            Ok(())
        } else {
            bail!("native navigator acknowledgement timed out")
        }
    }
    fn wait_for(
        client: &mut Client,
        screen: &mut NavigatorScreen,
        pump: &mut impl FnMut(&mut Client) -> Result<()>,
        duration: Duration,
        predicate: impl Fn(Option<&Navigator>) -> bool,
    ) -> Result<bool> {
        let epoch = screen.epoch;
        let deadline = Instant::now() + duration;
        loop {
            pump(client)?;
            client.drain_observed(&mut |token| screen.token(token))?;
            if screen.epoch > epoch && predicate(screen.navigator()?.as_ref()) {
                return Ok(true);
            }
            if client.child.try_wait()?.is_some() {
                bail!("native viewing client exited during switch");
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if let Err(error) = self.restore().and_then(|()| self.synchronize().map(|_| ())) {
            eprintln!(
                "herdrpp: {error:#}; native state recovery at {}",
                self.temporary.path().display()
            );
            // Preserve both sides when concurrent edits cannot be reconciled.
            self.temporary.disable_cleanup(true);
        }
    }
}
