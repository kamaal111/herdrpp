//! Read the existing picker bindings without editing the viewing configuration.
use anyhow::{Context, Result};

#[derive(Clone, Default)]
pub struct Bindings(pub Vec<(Vec<u8>, bool)>, pub Vec<Vec<u8>>);

fn combo(chord: &str) -> Option<(u32, u16)> {
    let mut modifiers = 0;
    let mut code = None;
    for part in chord.split('+') {
        match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= 4,
            "alt" | "option" | "meta" => modifiers |= 2,
            "shift" => modifiers |= 1,
            "cmd" | "command" | "super" => modifiers |= 8,
            "hyper" => modifiers |= 16,
            "space" => code = Some(32),
            key => {
                let mut chars = key.chars();
                let ch = chars.next()?;
                if chars.next().is_some() || code.is_some() {
                    return None;
                }
                code = Some(ch as u32);
            }
        }
    }
    Some((code?, modifiers))
}
fn sequences(code: u32, modifiers: u16) -> Vec<(Vec<u8>, bool)> {
    let mut result = Vec::new();
    if modifiers == 0
        && let Some(ch) = char::from_u32(code)
    {
        result.push((ch.to_string().into_bytes(), false));
    }
    if modifiers == 4 && (64..=127).contains(&code) {
        result.push((vec![(code as u8) & 31], false));
    }
    if modifiers == 2 && code < 128 {
        result.push((vec![27, code as u8], false));
    }
    for locks in [0, 64, 128, 192] {
        let mask = modifiers + locks + 1;
        result.push((format!("\x1b[{code};{mask}u").into_bytes(), false));
        for event in 1..=3 {
            result.push((
                format!("\x1b[{code};{mask}:{event}u").into_bytes(),
                event == 3,
            ));
        }
    }
    result.push((
        format!("\x1b[27;{};{code}~", modifiers + 1).into_bytes(),
        false,
    ));
    result
}
impl Bindings {
    pub fn load() -> Result<Self> {
        let path = crate::picker::theme::viewing_config_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context("read picker shortcuts"),
        };
        Self::parse(&text)
    }
    pub fn parse(text: &str) -> Result<Self> {
        let document: toml::Value = toml::from_str(text).context("read picker shortcuts")?;
        let mut result = Vec::new();
        let mut prefixes = Vec::new();
        if let Some(commands) = document
            .get("keys")
            .and_then(|keys| keys.get("command"))
            .and_then(toml::Value::as_array)
        {
            for command in commands {
                if command.get("type").and_then(toml::Value::as_str) != Some("plugin_action")
                    || command.get("command").and_then(toml::Value::as_str) != Some("herdrpp.open")
                {
                    continue;
                }
                let Some(key) = command.get("key").and_then(toml::Value::as_str) else {
                    continue;
                };
                if let Some(suffix) = key.strip_prefix("prefix+") {
                    let prefix = document
                        .get("keys")
                        .and_then(|keys| keys.get("prefix"))
                        .and_then(toml::Value::as_str)
                        .unwrap_or("ctrl+b");
                    if let (Some((p, pm)), Some((k, km))) = (combo(prefix), combo(suffix)) {
                        for (start, release) in
                            sequences(p, pm).into_iter().filter(|(_, release)| !release)
                        {
                            prefixes.push(start.clone());
                            for (end, end_release) in sequences(k, km) {
                                let mut sequence = start.clone();
                                sequence.extend(end);
                                result.push((sequence, release || end_release));
                            }
                        }
                    }
                } else if let Some((code, modifiers)) = combo(key) {
                    result.extend(sequences(code, modifiers));
                }
            }
        }
        Ok(Self(result, prefixes))
    }
}
