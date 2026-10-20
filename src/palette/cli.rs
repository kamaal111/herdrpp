use super::{
    api,
    app::Selection,
    catalog::{self, Command, Context, Item},
    history::{self, History, UsageKey},
    naming::Naming,
    ui,
};
use crate::picker::{
    api::{Herdr, decode},
    theme::Theme,
};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    env, fs,
    io::{Read, Write},
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub tasks: bool,
    #[serde(default)]
    pub recent_ids: Vec<String>,
    pub context: Context,
    pub naming: Naming,
    pub items: Vec<Item>,
    pub warning: Option<String>,
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= LIMIT as usize,
        "palette data exceeds size limit"
    );
    serde_json::from_slice(&bytes).context("invalid palette data")
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= LIMIT as usize,
        "palette data exceeds size limit"
    );
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("palette data parent")?)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

pub fn validate_selection<'a>(items: &'a [Item], selection: &Selection) -> Result<&'a Command> {
    let command = &items
        .iter()
        .find(|item| item.id == selection.id)
        .context("unknown palette command")?
        .command;
    match command {
        Command::Close(_) => ensure!(
            selection.confirmed && selection.input.is_none(),
            "closing requires confirmation"
        ),
        Command::Create(_) => ensure!(
            selection
                .input
                .as_ref()
                .is_some_and(|text| !text.chars().any(char::is_control))
                && !selection.confirmed,
            "creation requires a name prompt"
        ),
        Command::Rename(_) => {
            ensure!(
                selection.input.as_ref().is_some_and(
                    |text| !text.trim().is_empty() && !text.chars().any(char::is_control)
                ) && !selection.confirmed,
                "rename requires a name"
            )
        }
        _ => ensure!(
            selection.input.is_none() && !selection.confirmed,
            "unexpected palette command input"
        ),
    }
    Ok(command)
}

fn load(
    api: &Herdr,
    mut catalog: Catalog,
    cancelled: &AtomicBool,
    repo_directory: Option<&Path>,
) -> Catalog {
    if let Some(directory) = repo_directory
        && let Ok(Some(item)) = super::repository::item(directory, cancelled)
    {
        catalog.items.push(item);
    }
    catalog.naming.resolve_workspace_suggestion(cancelled);
    let plugins = (|| {
        let plugins = decode(&api.call_cancellable(&["plugin", "list", "--json"], cancelled)?)?;
        let actions = decode(&api.call_cancellable(&["plugin", "action", "list"], cancelled)?)?;
        catalog::plugin_items(plugins, actions, &catalog.context, env::consts::OS)
    })();
    catalog.warning = match plugins {
        Ok(actions) => {
            catalog.items.extend(actions);
            None
        }
        Err(error) => Some(format!("Plugin commands unavailable: {error:#}")),
    };
    catalog::sort(&mut catalog.items);
    catalog
}

fn owner(
    api: &Herdr,
    directory: &Path,
    context: &Context,
    initial_catalog: Catalog,
    cancelled: &AtomicBool,
    tasks: bool,
    repo_directory: Option<&Path>,
) -> Result<Option<(Catalog, Selection)>> {
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        scope.spawn(move || {
            let _ = sender.send(if tasks {
                crate::tasks::load(api, context)
            } else {
                load(api, initial_catalog, cancelled, repo_directory)
            });
        });
        let result = (|| {
            let mut catalog = None;
            let started = Instant::now();
            let mut terminal: Option<PathBuf> = None;
            let mut selected: Option<Option<Selection>> = None;
            let mut selected_at = None;
            loop {
                if catalog.is_none()
                    && let Ok(mut loaded) = receiver.try_recv()
                {
                    match history::state_directory().and_then(|directory| History::load(&directory))
                    {
                        Ok(history) => loaded.recent_ids = history.recent_ids(&loaded.items),
                        Err(error) => {
                            eprintln!("herdr++: palette usage history unavailable: {error:#}")
                        }
                    }
                    write_json(&directory.join("catalog.json"), &loaded)?;
                    catalog = Some(loaded);
                }
                if terminal.is_none() && directory.join("terminal.json").exists() {
                    let path: PathBuf = read_json(&directory.join("terminal.json"))?;
                    ensure!(
                        path.is_absolute() && path.starts_with("/dev"),
                        "invalid popup terminal path"
                    );
                    if let Ok(metadata) = fs::metadata(&path) {
                        ensure!(
                            metadata.file_type().is_char_device(),
                            "popup terminal must be a character device"
                        );
                    }
                    terminal = Some(path);
                }
                if selected.is_none() && directory.join("selection.json").exists() {
                    let selection: Option<Selection> =
                        read_json(&directory.join("selection.json"))?;
                    if let Some(selection) = &selection {
                        let loaded = catalog
                            .as_ref()
                            .context("palette selected before discovery completed")?;
                        validate_selection(&loaded.items, selection)?;
                    }
                    selected = Some(selection);
                    selected_at = Some(Instant::now());
                    cancelled.store(true, Ordering::Relaxed);
                }
                if let Some(path) = &terminal {
                    match fs::metadata(path) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            // The UI may have committed its selection after this
                            // iteration checked the mailbox but before PTY teardown.
                            if selected.is_none() && directory.join("selection.json").exists() {
                                continue;
                            }
                            return match selected
                                .take()
                                .context("command palette closed unexpectedly")?
                            {
                                Some(selection) => Ok(Some((
                                    catalog.take().context("missing command catalog")?,
                                    selection,
                                ))),
                                None => Ok(None),
                            };
                        }
                        Err(error) => return Err(error).context("inspect popup terminal"),
                        Ok(_) => {}
                    }
                }
                ensure!(
                    terminal.is_some() || started.elapsed() < Duration::from_secs(5),
                    "command palette did not start"
                );
                ensure!(
                    selected_at.is_none_or(|time| time.elapsed() < Duration::from_secs(5)),
                    "command palette did not close"
                );
                thread::sleep(Duration::from_millis(20));
            }
        })();
        cancelled.store(true, Ordering::Relaxed);
        result
    })
}

pub fn open(api: &Herdr) -> Result<()> {
    open_mode(api, false)
}

pub fn open_mode(api: &Herdr, tasks: bool) -> Result<()> {
    let context: Context = serde_json::from_str(
        &env::var("HERDR_PLUGIN_CONTEXT_JSON")
            .context("launch commands through a Herdr plugin action")?,
    )
    .context("invalid Herdr plugin context")?;
    let socket =
        PathBuf::from(env::var_os("HERDR_SOCKET_PATH").context("missing Herdr session socket")?);
    let snapshot = api.raw_snapshot()?;
    api::validate_context(&snapshot, &context)?;
    let naming = Naming::from_snapshot(&snapshot, &context);
    let mut items = catalog::builtins(&context);
    if !tasks && let Some(item) = super::pull_request::item_from_snapshot(&snapshot, &context) {
        items.push(item);
    }
    let repo_directory = super::repository::directory_from_snapshot(&snapshot, &context);
    let initial_catalog = Catalog {
        tasks,
        recent_ids: vec![],
        context: context.clone(),
        naming,
        items,
        warning: None,
    };
    let directory = tempfile::tempdir()?;
    let request_env = format!("HERDRPP_PALETTE_REQUEST={}", directory.path().display());
    api.open_popup(
        if tasks {
            "task-palette"
        } else {
            "command-palette"
        },
        Some(&request_env),
    )?;
    let cancelled = AtomicBool::new(false);
    let selected = owner(
        api,
        directory.path(),
        &context,
        initial_catalog,
        &cancelled,
        tasks,
        repo_directory.as_deref(),
    )?;
    if let Some((catalog, selection)) = selected {
        let command = validate_selection(&catalog.items, &selection)?;
        let key = catalog
            .items
            .iter()
            .find(|item| item.id == selection.id)
            .map(UsageKey::for_item)
            .context("unknown palette command")?;
        api::execute(
            api,
            &socket,
            command,
            &context,
            &catalog.naming,
            selection.input.as_deref(),
        )?;
        if let Err(error) =
            history::state_directory().and_then(|directory| History::record(&directory, key))
        {
            eprintln!("herdr++: cannot save palette usage history: {error:#}");
        }
    }
    Ok(())
}

pub fn pick() -> Result<()> {
    pick_mode(false)
}

pub fn pick_mode(tasks: bool) -> Result<()> {
    let directory = PathBuf::from(
        env::var_os("HERDRPP_PALETTE_REQUEST")
            .context("launch pick-command through the command palette")?,
    );
    let (theme, diagnostics) = Theme::load();
    for message in diagnostics {
        eprintln!("herdr++: {message}");
    }
    let terminal = crate::search::terminal::popup_terminal_path()?;
    write_json(&directory.join("terminal.json"), &terminal)?;
    let selection = ui::run_mode(&directory, theme, tasks)?;
    write_json(&directory.join("selection.json"), &selection)?;
    // Exit exactly as the workspace picker does. The owner waits until Herdr
    // releases this popup's PTY before invoking any selected command.
    Ok(())
}
