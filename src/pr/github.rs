use crate::command::run;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Checkout {
    pub root: PathBuf,
    pub branch: String,
    pub head_repo: String,
    pub head_branch: String,
    pub host: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Remote {
    pub host: String,
    pub repository: String,
}

pub fn parse_remote(value: &str) -> Option<Remote> {
    let value = value.trim();
    let (host, path) = if let Some(rest) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .or_else(|| value.strip_prefix("ssh://"))
    {
        let (authority, path) = rest.split_once('/')?;
        (authority.rsplit('@').next()?.split(':').next()?, path)
    } else {
        let (authority, path) = value.split_once(':')?;
        (authority.rsplit('@').next()?, path)
    };
    let path = path
        .trim_end_matches('/')
        .strip_suffix(".git")
        .unwrap_or(path.trim_end_matches('/'));
    let parts: Vec<_> = path.split('/').collect();
    if host.is_empty() || parts.len() != 2 || parts.iter().any(|s| s.is_empty()) {
        return None;
    }
    Some(Remote {
        host: host.to_owned(),
        repository: path.to_owned(),
    })
}

fn git(directory: &Path, args: &[&str]) -> Result<String> {
    let output = run(
        Command::new("git")
            .env("LC_ALL", "C")
            .arg("-C")
            .arg(directory)
            .args(args),
        Duration::from_secs(5),
    )?;
    Ok(String::from_utf8(output)
        .context("Git output is not UTF-8")?
        .trim()
        .to_owned())
}

pub fn checkout(directory: &Path) -> Result<Option<Checkout>> {
    if !directory.is_absolute() || !directory.is_dir() {
        return Ok(None);
    }
    // --is-inside-work-tree distinguishes ordinary directories without treating
    // authentication or network errors as evidence that a PR does not exist.
    let inside = git(directory, &["rev-parse", "--is-inside-work-tree"]);
    match inside {
        Ok(value) if value == "true" => {}
        Ok(_) => return Ok(None),
        Err(error) if error.to_string().contains("not a git repository") => return Ok(None),
        Err(error) => return Err(error),
    }
    let root = PathBuf::from(git(directory, &["rev-parse", "--show-toplevel"])?);
    let branch = git(&root, &["branch", "--show-current"])?;
    if branch.is_empty() {
        return Ok(None);
    }
    let remote_name = git(
        &root,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )
    .ok()
    .filter(|s| !s.is_empty() && s != ".")
    .unwrap_or_else(|| "origin".into());
    let remote_url = git(&root, &["remote", "get-url", &remote_name]).unwrap_or_default();
    let Some(remote) = parse_remote(&remote_url) else {
        return Ok(None);
    };
    let head_branch = git(
        &root,
        &["config", "--get", &format!("branch.{branch}.merge")],
    )
    .ok()
    .and_then(|s| s.strip_prefix("refs/heads/").map(str::to_owned))
    .unwrap_or_else(|| branch.clone());
    Ok(Some(Checkout {
        root,
        branch,
        head_branch,
        head_repo: remote.repository,
        host: remote.host,
    }))
}

#[derive(Deserialize)]
struct Repository {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
    url: String,
}

#[derive(Deserialize)]
pub struct PullRequest {
    number: u64,
    created_at: String,
    head: Head,
}
#[derive(Deserialize)]
struct Head {
    #[serde(rename = "ref")]
    branch: String,
    repo: Option<HeadRepository>,
}
#[derive(Deserialize)]
struct HeadRepository {
    full_name: String,
}

pub fn newest_pr(bytes: &[u8], checkout: &Checkout) -> Result<Option<u64>> {
    let pages = serde_json::Deserializer::from_slice(bytes).into_iter::<Vec<PullRequest>>();
    let mut newest: Option<PullRequest> = None;
    for page in pages {
        for pr in page.context("parse GitHub PR response")? {
            if pr.head.branch == checkout.head_branch
                && pr
                    .head
                    .repo
                    .as_ref()
                    .is_some_and(|r| r.full_name.eq_ignore_ascii_case(&checkout.head_repo))
            {
                ensure!(
                    pr.number > 0 && !pr.created_at.is_empty(),
                    "invalid GitHub PR number/date"
                );
                if newest
                    .as_ref()
                    .is_none_or(|old| (&pr.created_at, pr.number) > (&old.created_at, old.number))
                {
                    newest = Some(pr);
                }
            }
        }
    }
    ensure!(
        !bytes.iter().all(u8::is_ascii_whitespace),
        "empty GitHub response"
    );
    Ok(newest.map(|pr| pr.number))
}

fn gh(checkout: &Checkout) -> Command {
    let mut command = Command::new("gh");
    command
        .current_dir(&checkout.root)
        .env_remove("GH_REPO")
        .env("GH_HOST", &checkout.host)
        .env("GH_PROMPT_DISABLED", "1");
    command
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CiStatus {
    Passed,
    Pending,
    Failed,
}

impl CiStatus {
    pub fn icon(self) -> &'static str {
        match self {
            Self::Passed => "✅",
            Self::Pending => "🟡",
            Self::Failed => "❌",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrSummary {
    pub number: u64,
    pub lifecycle: Lifecycle,
    pub ci: Option<CiStatus>,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum PrState {
    Open,
    Closed,
    Merged,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrDetails {
    number: u64,
    state: PrState,
    is_draft: bool,
    status_check_rollup: Option<Vec<Check>>,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum Check {
    CheckRun {
        status: CheckStatus,
        conclusion: Option<Conclusion>,
    },
    StatusContext {
        state: CommitStatus,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CheckStatus {
    Completed,
    InProgress,
    Pending,
    Queued,
    Requested,
    Waiting,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Conclusion {
    #[serde(rename = "")]
    Unset,
    Success,
    Neutral,
    Skipped,
    Failure,
    Cancelled,
    TimedOut,
    ActionRequired,
    Stale,
    StartupFailure,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CommitStatus {
    Success,
    Expected,
    Pending,
    Error,
    Failure,
}

impl Check {
    fn result(self) -> Result<Option<CiStatus>> {
        Ok(match self {
            Self::CheckRun {
                status: CheckStatus::Completed,
                conclusion,
            } => match conclusion.context("completed GitHub check has no conclusion")? {
                Conclusion::Unset => anyhow::bail!("completed GitHub check has empty conclusion"),
                Conclusion::Success => Some(CiStatus::Passed),
                Conclusion::Neutral | Conclusion::Skipped => None,
                _ => Some(CiStatus::Failed),
            },
            Self::CheckRun { .. } => Some(CiStatus::Pending),
            Self::StatusContext { state } => Some(match state {
                CommitStatus::Success => CiStatus::Passed,
                CommitStatus::Expected | CommitStatus::Pending => CiStatus::Pending,
                CommitStatus::Error | CommitStatus::Failure => CiStatus::Failed,
            }),
        })
    }
}

pub fn parse_summary(bytes: &[u8], expected_number: u64) -> Result<PrSummary> {
    let details: PrDetails = serde_json::from_slice(bytes).context("parse GitHub PR details")?;
    ensure!(
        details.number > 0 && details.number == expected_number,
        "GitHub PR details number mismatch"
    );
    let lifecycle = match details.state {
        PrState::Merged => Lifecycle::Merged,
        PrState::Closed => Lifecycle::Closed,
        PrState::Open if details.is_draft => Lifecycle::Draft,
        PrState::Open => Lifecycle::Open,
    };
    let mut ci = None;
    for check in details.status_check_rollup.unwrap_or_default() {
        ci = ci.max(check.result()?);
    }
    Ok(PrSummary {
        number: details.number,
        lifecycle,
        ci,
    })
}

pub fn lookup(checkout: &Checkout) -> Result<Option<PrSummary>> {
    let repository: Repository = serde_json::from_slice(&run(
        gh(checkout).args(["repo", "view", "--json", "nameWithOwner,url"]),
        Duration::from_secs(15),
    )?)
    .context("resolve GitHub base repository")?;
    let base = parse_remote(&repository.url).context("invalid GitHub repository URL")?;
    ensure!(
        base.host.eq_ignore_ascii_case(&checkout.host),
        "GitHub base/head hosts differ"
    );
    ensure!(
        base.repository
            .eq_ignore_ascii_case(&repository.name_with_owner),
        "GitHub repository identity mismatch"
    );
    let (owner, _) = checkout
        .head_repo
        .split_once('/')
        .context("invalid head repository")?;
    let endpoint = format!("repos/{}/pulls", repository.name_with_owner);
    let head = format!("head={owner}:{}", checkout.head_branch);
    let output = run(
        gh(checkout).args([
            "api",
            "--hostname",
            &checkout.host,
            "--method",
            "GET",
            &endpoint,
            "-f",
            "state=all",
            "-f",
            &head,
            "-f",
            "sort=created",
            "-f",
            "direction=desc",
            "-f",
            "per_page=100",
            "--paginate",
        ]),
        Duration::from_secs(20),
    )?;
    let Some(number) = newest_pr(&output, checkout)? else {
        return Ok(None);
    };
    let repo = format!("{}/{}", checkout.host, repository.name_with_owner);
    let output = run(
        gh(checkout).args([
            "pr",
            "view",
            &number.to_string(),
            "--repo",
            &repo,
            "--json",
            "number,state,isDraft,statusCheckRollup",
        ]),
        Duration::from_secs(20),
    )?;
    parse_summary(&output, number).map(Some)
}
