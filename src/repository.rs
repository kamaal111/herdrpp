use anyhow::{Context, Result, ensure};
use std::{
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub host: String,
    pub repository: String,
    scheme: String,
    authority: String,
}

impl Remote {
    pub fn url(&self) -> String {
        format!("{}://{}/{}", self.scheme, self.authority, self.repository)
    }
}

pub fn parse_remote(value: &str) -> Option<Remote> {
    let value = value.trim();
    if value.chars().any(|c| c.is_whitespace() || c.is_control())
        || value.contains(['?', '#', '\\'])
    {
        return None;
    }
    let (scheme, authority, path, web) = if let Some(rest) = value.strip_prefix("https://") {
        let (authority, path) = rest.split_once('/')?;
        ("https", authority, path, true)
    } else if let Some(rest) = value.strip_prefix("http://") {
        let (authority, path) = rest.split_once('/')?;
        ("http", authority, path, true)
    } else if let Some(rest) = value.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/')?;
        ("https", authority, path, false)
    } else {
        if value.contains("://") {
            return None;
        }
        let (authority, path) = value.split_once(':')?;
        ("https", authority, path, false)
    };
    let authority = authority.rsplit('@').next()?;
    let (host, port) = authority
        .split_once(':')
        .map_or((authority, None), |(host, port)| (host, Some(port)));
    if host.is_empty()
        || host.starts_with('-')
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
    {
        return None;
    }
    if let Some(port) = port {
        port.parse::<u16>().ok().filter(|port| *port > 0)?;
    }
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty()
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return None;
    }
    Some(Remote {
        host: host.to_owned(),
        repository: path.to_owned(),
        scheme: scheme.to_owned(),
        authority: if web { authority } else { host }.to_owned(),
    })
}

fn git(directory: &Path, args: &[&str], cancelled: &AtomicBool) -> Result<Option<String>> {
    ensure!(
        !cancelled.load(Ordering::Relaxed),
        "repository discovery cancelled"
    );
    let output = crate::command::run_cancellable(
        Command::new("git")
            .env("LC_ALL", "C")
            .arg("-C")
            .arg(directory)
            .args(args),
        Duration::from_secs(5),
        4 * 1024 * 1024,
        cancelled,
        "repository discovery",
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8(output.stdout)
            .context("Git output is not UTF-8")?
            .trim()
            .to_owned(),
    ))
}

pub fn repository_url(directory: &Path, cancelled: &AtomicBool) -> Result<Option<String>> {
    if !directory.is_absolute() || !directory.is_dir() {
        return Ok(None);
    }
    if git(
        directory,
        &["rev-parse", "--is-inside-work-tree"],
        cancelled,
    )?
    .as_deref()
        != Some("true")
    {
        return Ok(None);
    }
    let branch = git(directory, &["branch", "--show-current"], cancelled)?.unwrap_or_default();
    if !branch.is_empty()
        && let Some(tracking) = git(
            directory,
            &["config", "--get", &format!("branch.{branch}.remote")],
            cancelled,
        )?
        && !tracking.is_empty()
    {
        if tracking == "." {
            return Ok(None);
        }
        return Ok(git(
            directory,
            &["remote", "get-url", "--", &tracking],
            cancelled,
        )?
        .and_then(|value| parse_remote(&value))
        .map(|remote| remote.url()));
    }
    let names = git(directory, &["remote"], cancelled)?.unwrap_or_default();
    let mut urls = Vec::new();
    for name in names.lines() {
        if let Some(remote) = git(directory, &["remote", "get-url", "--", name], cancelled)?
            .and_then(|value| parse_remote(&value))
        {
            let url = remote.url();
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    Ok((urls.len() == 1).then(|| urls.remove(0)))
}
