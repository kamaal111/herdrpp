//! Explicit, versioned client configuration; never called by watcher actions.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

// Bump only for a new, targeted config migration, never for a package release.
const CONFIG_VERSION: u32 = 3;
const FNV1A_64_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV1A_64_PRIME: u64 = 0x100000001b3;
const TOKENS: [(&str, &str); 4] = [
    ("$pr_open", "#238636"),
    ("$pr_draft", "#8b949e"),
    ("$pr_merged", "#a371f7"),
    ("$pr_closed", "#f85149"),
];

#[derive(Serialize, Deserialize)]
struct Migration {
    path: PathBuf,
    version: u32,
}

fn resolve(path: &Path, depth: usize) -> Result<PathBuf> {
    ensure!(depth < 64, "too many config path symlinks");
    if let Ok(target) = fs::read_link(path) {
        return resolve(
            &path.parent().context("config parent")?.join(target),
            depth + 1,
        );
    }
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().context("config path has no parent")?;
            Ok(resolve(parent, depth + 1)?.join(path.file_name().context("config filename")?))
        }
        Err(error) => Err(error).context("resolve config path"),
    }
}

fn atomic_write(path: &Path, bytes: &[u8], permissions: Option<fs::Permissions>) -> Result<()> {
    let parent = path.parent().context("file parent")?;
    let mut pending = tempfile::NamedTempFile::new_in(parent)?;
    if let Some(permissions) = permissions {
        pending.as_file().set_permissions(permissions)?;
    }
    pending.write_all(bytes)?;
    pending.as_file().sync_all()?;
    pending.persist(path).map_err(|error| error.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn token(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.as_inline_table()?.get("token")?.as_str())
}

fn styled(name: &str, color: &str) -> Value {
    let mut style = InlineTable::new();
    style.insert("token", name.into());
    style.insert("fg", color.into());
    Value::InlineTable(style)
}

fn missing_tokens<'a>(values: impl Iterator<Item = &'a Value>) -> Vec<Value> {
    let present: Vec<&str> = values.filter_map(token).collect();
    TOKENS
        .iter()
        .filter(|(name, _)| !present.contains(name))
        .map(|(name, color)| styled(name, color))
        .collect()
}

fn validate(rows: &Array) -> Result<()> {
    ensure!(rows.len() <= 16, "sidebar exceeds 16 rows");
    for row in rows {
        let row = row.as_array().context("sidebar rows must contain arrays")?;
        ensure!(row.len() <= 16, "sidebar row exceeds 16 tokens");
        for value in row {
            ensure!(
                token(value).is_some(),
                "sidebar entry must be a token or styled token"
            );
        }
    }
    Ok(())
}

fn migrate_sidebar(original: &str) -> Result<String> {
    let mut document = original
        .parse::<DocumentMut>()
        .context("parse Herdr config TOML")?;
    let mut table = document.as_table_mut();
    for key in ["ui", "sidebar", "spaces"] {
        if !table.contains_key(key) {
            let mut child = Table::new();
            child.set_implicit(true);
            table.insert(key, Item::Table(child));
        }
        table = table
            .get_mut(key)
            .and_then(Item::as_table_mut)
            .with_context(|| {
                format!("{key} must be a TOML table; merge sidebar tokens manually")
            })?;
    }
    if !table.contains_key("rows") {
        let example = include_str!("../examples/herdr.toml").parse::<DocumentMut>()?;
        table.insert("rows", example["ui"]["sidebar"]["spaces"]["rows"].clone());
        return Ok(document.to_string());
    }
    let rows = table
        .get_mut("rows")
        .and_then(Item::as_array_mut)
        .context("sidebar rows must be an array")?;
    validate(rows)?;
    let missing = missing_tokens(
        rows.iter()
            .flat_map(|row| row.as_array().into_iter().flatten()),
    );
    let mut replaced = false;
    for row in rows.iter_mut() {
        let row = row.as_array_mut().context("sidebar row")?;
        let row_missing = missing_tokens(row.iter());
        let mut row_replaced = false;
        let mut index = 0;
        while index < row.len() {
            if row.get(index).and_then(token) == Some("$pr_number") {
                let old = row.remove(index);
                // Keep deliberate PR displays on separate rows, while reusing
                // styled tokens already present on this row.
                if !row_replaced {
                    for (offset, value) in row_missing.iter().enumerate() {
                        let mut value = value.clone();
                        if offset == 0 {
                            *value.decor_mut() = old.decor().clone();
                        }
                        row.insert(index + offset, value);
                    }
                    index += row_missing.len();
                }
                row_replaced = true;
                replaced = true;
            } else {
                index += 1;
            }
        }
    }
    if !replaced && !missing.is_empty() {
        let position = rows.iter().enumerate().find_map(|(r, row)| {
            row.as_array()?
                .iter()
                .position(|v| token(v) == Some("workspace"))
                .map(|c| (r, c))
        });
        if let Some((r, c)) = position {
            let row = rows
                .get_mut(r)
                .and_then(Value::as_array_mut)
                .context("workspace row")?;
            for (offset, value) in missing.into_iter().enumerate() {
                row.insert(c + 1 + offset, value);
            }
        } else {
            let mut row = Array::new();
            for value in missing {
                row.push(value);
            }
            rows.push(row);
        }
    }
    validate(rows)?;
    Ok(document.to_string())
}

fn normalized_chord(key: &str) -> Vec<String> {
    let mut parts: Vec<_> = key
        .split('+')
        .map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "super" => "cmd".to_string(),
            value => value.to_string(),
        })
        .collect();
    parts.sort();
    parts
}

fn migrate_shortcut(
    original: &str,
    chord: &str,
    action: &str,
    description: &str,
) -> Result<(String, bool)> {
    let mut document = original
        .parse::<DocumentMut>()
        .context("parse Herdr config TOML")?;
    if !document.contains_key("keys") {
        document.insert("keys", Item::Table(Table::new()));
    }
    let keys = document["keys"]
        .as_table_mut()
        .context("keys must be a TOML table")?;
    let conflict = keys
        .iter()
        .filter(|(name, _)| *name != "command")
        .any(|(_, value)| {
            value
                .as_str()
                .is_some_and(|key| normalized_chord(key) == normalized_chord(chord))
                || value.as_array().is_some_and(|keys| {
                    keys.iter()
                        .filter_map(Value::as_str)
                        .any(|key| normalized_chord(key) == normalized_chord(chord))
                })
        });
    if conflict {
        return Ok((original.to_owned(), true));
    }
    if !keys.contains_key("command") {
        keys.insert(
            "command",
            Item::ArrayOfTables(toml_edit::ArrayOfTables::new()),
        );
    }
    let commands = keys.get_mut("command").context("keys.command")?;
    let mut binding = Table::new();
    binding.insert("key", toml_edit::value(chord));
    binding.insert("type", toml_edit::value("plugin_action"));
    binding.insert("command", toml_edit::value(action));
    binding.insert("description", toml_edit::value(description));
    let existing = if let Some(tables) = commands.as_array_of_tables_mut() {
        let mut existing = None;
        for table in tables.iter() {
            let key = table
                .get("key")
                .and_then(Item::as_str)
                .context("shortcut key must be a string")?;
            if normalized_chord(key) == normalized_chord(chord) {
                existing = Some(
                    table.get("type").and_then(Item::as_str) == Some("plugin_action")
                        && table.get("command").and_then(Item::as_str) == Some(action),
                );
                break;
            }
        }
        if existing.is_none() {
            tables.push(binding);
        }
        existing
    } else if let Some(array) = commands.as_array_mut() {
        let mut existing = None;
        for value in array.iter() {
            let table = value
                .as_inline_table()
                .context("shortcut entries must be tables")?;
            let key = table
                .get("key")
                .and_then(Value::as_str)
                .context("shortcut key must be a string")?;
            if normalized_chord(key) == normalized_chord(chord) {
                existing = Some(
                    table.get("type").and_then(Value::as_str) == Some("plugin_action")
                        && table.get("command").and_then(Value::as_str) == Some(action),
                );
                break;
            }
        }
        if existing.is_none() {
            let mut inline = InlineTable::new();
            for (key, item) in binding.iter() {
                inline.insert(key, item.as_value().context("shortcut value")?.clone());
            }
            array.push(Value::InlineTable(inline));
        }
        existing
    } else {
        bail!("keys.command must be an array of shortcut tables");
    };
    Ok((document.to_string(), existing == Some(false)))
}

pub fn run() -> Result<()> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is required")?);
    let selected = std::env::var_os("HERDR_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config/herdr/config.toml"));
    let path = resolve(&std::env::current_dir()?.join(selected), 0)?;
    // Stable hash for filesystem naming; compare the full path in the marker
    // before trusting it, so a collision cannot skip another config's setup.
    let hash = path
        .as_os_str()
        .as_bytes()
        .iter()
        .fold(FNV1A_64_OFFSET_BASIS, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV1A_64_PRIME)
        });
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"))
        .join("herdrpp/configs")
        .join(format!("{hash:016x}"));
    fs::create_dir_all(&state).context("create config migration directory")?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state.join("lock"))?;
    // SAFETY: lock owns a valid descriptor and remains open for the migration.
    ensure!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } == 0,
        "lock config migration: {}",
        std::io::Error::last_os_error()
    );
    let marker = state.join("migration.json");
    let completed = match fs::read(&marker) {
        Ok(bytes) => {
            let migration: Migration =
                serde_json::from_slice(&bytes).context("read config migration state")?;
            ensure!(migration.path == path, "config migration path collision");
            if migration.version >= CONFIG_VERSION {
                println!(
                    "Viewing config already initialized: {} (left untouched)",
                    path.display()
                );
                return Ok(());
            }
            migration.version
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error).context("read config migration state"),
    };
    let original = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("read Herdr config"),
    };
    let mut updated = original.clone().unwrap_or_default();
    let mut backups = Vec::new();
    let mut conflicts = Vec::new();
    if completed < 1 {
        backups.push((1, updated.clone()));
        updated = migrate_sidebar(&updated)?;
        let (text, conflict) = migrate_shortcut(
            &updated,
            "cmd+p",
            "herdrpp.open",
            "Search workspaces, worktrees and tabs",
        )?;
        updated = text;
        if conflict {
            conflicts.push("Cmd+P is already bound; existing shortcut preserved. Add herdrpp.open manually using another key, such as prefix+f.");
        }
    }
    if completed < 2 {
        backups.push((2, updated.clone()));
        let (text, conflict) = migrate_shortcut(
            &updated,
            "cmd+shift+k",
            "herdrpp.commands",
            "Open command palette",
        )?;
        updated = text;
        if conflict {
            conflicts.push("Cmd+Shift+K is already bound; existing shortcut preserved. Add herdrpp.commands manually using another key, such as prefix+shift+f.");
        }
    }
    if completed < 3 {
        backups.push((3, updated.clone()));
        let (text, conflict) = migrate_shortcut(
            &updated,
            "cmd+shift+r",
            "herdrpp.tasks",
            "Run workspace tasks",
        )?;
        updated = text;
        if conflict {
            conflicts.push("Cmd+Shift+R is already bound; existing shortcut preserved. Add herdrpp.tasks manually using another key, such as prefix+shift+r.");
        }
    }
    if original.as_deref() != Some(&updated) {
        if original.is_some() {
            for (version, text) in backups {
                let backup = state.join(format!("before-{version}.toml"));
                if !backup.exists() {
                    atomic_write(&backup, text.as_bytes(), None)?;
                }
            }
        }
        fs::create_dir_all(path.parent().context("config parent")?)?;
        // Avoid overwriting an edit made while the migration was being prepared.
        let current = fs::read_to_string(&path);
        match (&original, current) {
            (Some(expected), Ok(actual)) if *expected == actual => {}
            (None, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => bail!("config changed during setup; retry installation"),
        }
        atomic_write(
            &path,
            updated.as_bytes(),
            fs::metadata(&path).ok().map(|m| m.permissions()),
        )?;
    }
    atomic_write(
        &marker,
        &serde_json::to_vec(&Migration {
            path: path.clone(),
            version: CONFIG_VERSION,
        })?,
        None,
    )?;
    for message in conflicts {
        println!("{message}");
    }
    println!(
        "Viewing config initialized: {}. Select Reload config in the viewing client.",
        path.display()
    );
    Ok(())
}
