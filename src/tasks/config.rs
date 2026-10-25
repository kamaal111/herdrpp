//! Strict task-file parsing retains duplicate object keys for useful diagnostics.
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use std::{collections::HashMap, fmt, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    pub new_tab: bool,
}

#[derive(Debug)]
enum Raw {
    Scalar(Value),
    Number,
    Array(Vec<Raw>),
    Object(Vec<(String, Raw)>),
}

impl<'de> Deserialize<'de> for Raw {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RawVisitor;
        impl<'de> Visitor<'de> for RawVisitor {
            type Value = Raw;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a task configuration value")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Raw, E> {
                Ok(Raw::Scalar(v.into()))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Raw, E> {
                Ok(Raw::Number)
            }
            fn visit_u64<E>(self, _: u64) -> Result<Raw, E> {
                Ok(Raw::Number)
            }
            fn visit_f64<E>(self, _: f64) -> Result<Raw, E> {
                Ok(Raw::Number)
            }
            fn visit_i128<E>(self, _: i128) -> Result<Raw, E> {
                Ok(Raw::Number)
            }
            fn visit_u128<E>(self, _: u128) -> Result<Raw, E> {
                Ok(Raw::Number)
            }
            fn visit_str<E>(self, v: &str) -> Result<Raw, E> {
                Ok(Raw::Scalar(v.into()))
            }
            fn visit_string<E>(self, v: String) -> Result<Raw, E> {
                Ok(Raw::Scalar(v.into()))
            }
            fn visit_unit<E>(self) -> Result<Raw, E> {
                Ok(Raw::Scalar(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Raw, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element()? {
                    values.push(value);
                }
                Ok(Raw::Array(values))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Raw, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Raw::Object(entries))
            }
        }
        deserializer.deserialize_any(RawVisitor)
    }
}

impl Raw {
    fn string(&self) -> Option<&str> {
        match self {
            Self::Scalar(Value::String(s)) => Some(s),
            _ => None,
        }
    }
    fn kind(&self) -> &'static str {
        match self {
            Self::Array(_) => "array",
            Self::Object(_) => "object",
            Self::Scalar(Value::Null) => "null",
            Self::Scalar(Value::String(_)) => "string",
            Self::Scalar(Value::Bool(_)) => "boolean",
            _ => "number",
        }
    }
}

fn required_string(
    value: Option<&&Raw>,
    at: &str,
    label: bool,
    errors: &mut Vec<String>,
) -> Option<String> {
    let Some(text) = value.and_then(|v| v.string()) else {
        errors.push(format!(
            "{at}: expected a nonblank string; add a quoted string value."
        ));
        return None;
    };
    if text.trim().is_empty() {
        errors.push(format!(
            "{at}: expected a nonblank string; enter visible text."
        ));
    } else if text.contains('\0') {
        errors.push(format!("{at}: NUL is not allowed; remove \\u0000."));
    } else if label && text.chars().any(char::is_control) {
        errors.push(format!(
            "{at}: control characters are not allowed; use a single-line visible label."
        ));
    }
    Some(text.to_owned())
}

fn validate_fields<'a>(
    fields: &'a [(String, Raw)],
    at: &str,
    supported: &[&str],
    errors: &mut Vec<String>,
) -> HashMap<&'a str, &'a Raw> {
    let mut values = HashMap::new();
    for (key, value) in fields {
        if values.insert(key.as_str(), value).is_some() {
            errors.push(format!(
                "{at}, {key}: duplicate field; keep only one {key} field."
            ));
        }
        if !supported.contains(&key.as_str()) {
            errors.push(format!(
                "{at}, {key:?}: unknown field; remove it. Supported fields: {}.",
                supported.join(", ")
            ));
        }
    }
    values
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Json5,
    Yaml,
    Toml,
}

pub const FILES: [(&str, Format); 5] = [
    ("tasks.toml", Format::Toml),
    ("tasks.yaml", Format::Yaml),
    ("tasks.yml", Format::Yaml),
    ("tasks.json", Format::Json5),
    ("tasks.json5", Format::Json5),
];

pub fn parse(text: &str) -> Result<Vec<Task>, Vec<String>> {
    parse_with_format(text, Format::Json5)
}

pub fn parse_with_format(text: &str, format: Format) -> Result<Vec<Task>, Vec<String>> {
    let raw: Raw = match format {
        Format::Json5 => json5::from_str(text).map_err(|error| {
            let position = error
                .position()
                .unwrap_or_else(|| json5::Position::from_offset(text.len(), text));
            format!("Invalid JSON5 at {position}: {error}. Fix the JSON5 syntax.")
        }),
        Format::Yaml => serde_saphyr::from_str_with_options(
            text,
            serde_saphyr::options! {
                strict_booleans: true,
                reject_unsupported_tags: true,
            },
        )
        .map_err(|error| format!("Invalid YAML: {error}. Fix the YAML syntax.")),
        Format::Toml => toml::from_str(text)
            .map_err(|error| format!("Invalid TOML: {error}. Fix the TOML syntax.")),
    }
    .map_err(|error| vec![error])?;
    let mut errors = Vec::new();
    let entries = match &raw {
        Raw::Array(entries) => entries,
        Raw::Object(fields) => {
            let values =
                validate_fields(fields, "Configuration", &["$schema", "tasks"], &mut errors);
            if values.contains_key("$schema") {
                required_string(
                    values.get("$schema"),
                    "Configuration, $schema",
                    false,
                    &mut errors,
                );
            }
            match values.get("tasks") {
                Some(Raw::Array(entries)) => entries,
                _ => {
                    errors.push("Configuration, tasks: expected an array; add tasks: [ ... ] containing task objects.".into());
                    return Err(errors);
                }
            }
        }
        _ => {
            return Err(vec![
                "expected a task array or an object containing tasks: [ ... ].".into(),
            ]);
        }
    };
    let mut tasks = Vec::new();
    let mut labels = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let number = index + 1;
        let Raw::Object(fields) = entry else {
            errors.push(format!(
                "Task {number}: expected an object, received {}; replace it with a task object.",
                entry.kind()
            ));
            continue;
        };
        let label = fields
            .iter()
            .find(|(key, _)| key == "label")
            .and_then(|(_, v)| v.string());
        let at = match label {
            Some(label) => format!("Task {number} ({label:?})"),
            None => format!("Task {number}"),
        };
        let values = validate_fields(
            fields,
            &at,
            &["label", "command", "args", "new_tab"],
            &mut errors,
        );
        let label = required_string(
            values.get("label"),
            &format!("{at}, label"),
            true,
            &mut errors,
        );
        let command = required_string(
            values.get("command"),
            &format!("{at}, command"),
            false,
            &mut errors,
        );
        if let Some(label) = &label {
            if let Some(previous) = labels.get(label) {
                errors.push(format!(
                    "{at}, label: duplicates Task {previous}; choose a unique label."
                ));
            } else {
                labels.insert(label.clone(), number);
            }
        }
        let mut args = Vec::new();
        match values.get("args") {
            None => {}
            Some(Raw::Array(entries)) => {
                for (index, arg) in entries.iter().enumerate() {
                    match arg.string() {
                        Some(s) if !s.contains('\0') => args.push(s.to_owned()),
                        Some(_) => errors.push(format!("{at}, args[{index}]: NUL is not allowed; remove \\u0000.")),
                        None => errors.push(format!("{at}, args[{index}]: expected a string, received {}; quote the argument.", arg.kind())),
                    }
                }
            }
            Some(value) => errors.push(format!(
                "{at}, args: expected an array of strings, received {}; use [] or [\"argument\"].",
                value.kind()
            )),
        }
        let new_tab = match values.get("new_tab") {
            None => true,
            Some(Raw::Scalar(Value::Bool(value))) => *value,
            Some(value) => {
                errors.push(format!("{at}, new_tab: expected true or false, received {}. Remove the quotes for string booleans; otherwise replace with true or false.", value.kind()));
                true
            }
        };
        if let (Some(label), Some(command)) = (label, command) {
            tasks.push(Task {
                label,
                command,
                args,
                new_tab,
            });
        }
    }
    if errors.is_empty() {
        Ok(tasks)
    } else {
        Err(errors)
    }
}

pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn shell_command(task: &Task, root: &Path) -> String {
    let args = task
        .args
        .iter()
        .map(|arg| quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        "cd {} && {} {}",
        quote(&root.to_string_lossy()),
        task.command,
        args
    );
    format!("/bin/sh -c {}", quote(&script))
}
