mod batch;

use crate::command::run;
use anyhow::{Context, Result, ensure};
pub(crate) use batch::{STATUS_BATCH_SIZE, StatusTarget, fetch_statuses, prepare};
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
    pub remotes: Vec<Remote>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Remote {
    pub host: String,
    pub repository: String,
}

pub fn parse_remote(value: &str) -> Option<Remote> {
    let remote = crate::repository::parse_remote(value)?;
    if remote.repository.split('/').count() != 2 {
        return None;
    }
    Some(Remote {
        host: remote.host,
        repository: remote.repository,
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
    let remote_names = git(&root, &["remote"])?;
    let mut remotes = Vec::new();
    for name in remote_names.lines() {
        if let Some(remote) = parse_remote(&git(&root, &["remote", "get-url", name])?)
            && !remotes.contains(&remote)
        {
            remotes.push(remote);
        }
    }
    if remotes.is_empty() {
        return Ok(None);
    }
    let tracking = git(
        &root,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )
    .ok();
    let selected = match tracking.as_deref() {
        Some(".") => return Ok(None),
        Some(name) if !name.is_empty() => {
            let Some(remote) = parse_remote(&git(&root, &["remote", "get-url", name])?) else {
                return Ok(None);
            };
            Some(remote)
        }
        _ if remotes.len() == 1 => remotes.first().cloned(),
        _ => None,
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
        head_repo: selected
            .as_ref()
            .map(|r| r.repository.clone())
            .unwrap_or_default(),
        host: selected.map(|r| r.host).unwrap_or_default(),
        remotes,
    }))
}

#[derive(Deserialize)]
struct Repository {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
    url: String,
    #[serde(rename = "defaultBranchRef")]
    default_branch_ref: Option<BranchRef>,
}

#[derive(Deserialize)]
struct BranchRef {
    name: String,
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
        .env("GH_PROMPT_DISABLED", "1");
    if checkout.host.is_empty() {
        command.env_remove("GH_HOST");
    } else {
        command.env("GH_HOST", &checkout.host);
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceStatus {
    PullRequest(PrSummary),
    DefaultBranch { ci: Option<CiStatus> },
}

#[derive(Debug, Clone)]
pub struct LookupFailure {
    pub default_branch: bool,
    pub message: String,
}

impl std::fmt::Display for LookupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for LookupFailure {}

impl From<anyhow::Error> for LookupFailure {
    fn from(error: anyhow::Error) -> Self {
        Self {
            default_branch: false,
            message: format!("{error:#}"),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum PrState {
    Open,
    Closed,
    Merged,
}

impl PrState {
    fn lifecycle(self, is_draft: bool) -> Lifecycle {
        match self {
            Self::Merged => Lifecycle::Merged,
            Self::Closed => Lifecycle::Closed,
            Self::Open if is_draft => Lifecycle::Draft,
            Self::Open => Lifecycle::Open,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrDetails {
    number: u64,
    state: PrState,
    is_draft: bool,
    commits: PrCommits,
}

#[derive(Deserialize)]
struct PrCommits {
    nodes: Vec<PrCommit>,
}

#[derive(Deserialize)]
struct PrCommit {
    commit: CiCommit,
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
    fn has_started(&self) -> bool {
        matches!(
            self,
            Self::CheckRun {
                status: CheckStatus::InProgress | CheckStatus::Completed,
                ..
            } | Self::StatusContext {
                state: CommitStatus::Success | CommitStatus::Error | CommitStatus::Failure
            }
        )
    }

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
    summarize_pr(details, expected_number)
}

fn summarize_pr(details: PrDetails, expected_number: u64) -> Result<PrSummary> {
    ensure!(
        details.number > 0 && details.number == expected_number,
        "GitHub PR details number mismatch"
    );
    let lifecycle = details.state.lifecycle(details.is_draft);
    ensure!(
        details.commits.nodes.len() == 1,
        "GitHub PR has no unique head commit"
    );
    let (ci, _) = commit_ci(
        details
            .commits
            .nodes
            .into_iter()
            .next()
            .context("GitHub PR has no head commit")?
            .commit,
    )?;
    Ok(PrSummary {
        number: details.number,
        lifecycle,
        ci,
    })
}

fn aggregate_checks(checks: Vec<Check>) -> Result<(Option<CiStatus>, bool)> {
    let mut ci = None;
    let mut started = false;
    for check in checks {
        let has_started = check.has_started();
        let result = check.result()?;
        started |= has_started && result.is_some();
        ci = ci.max(result);
    }
    Ok((ci, started))
}

pub fn lookup(checkout: &Checkout) -> std::result::Result<Option<WorkspaceStatus>, LookupFailure> {
    let Some(target) = prepare(checkout, &mut || Ok(true))? else {
        return Ok(None);
    };
    fetch_statuses(&[target], &mut || Ok(true))
        .pop()
        .context("GitHub status result missing")?
}

fn resolve_repository(checkout: &Checkout) -> Result<(Checkout, bool, String)> {
    resolve_repository_while(checkout, &mut || Ok(true))
}

fn resolve_repository_while(
    checkout: &Checkout,
    should_continue: &mut impl FnMut() -> Result<bool>,
) -> Result<(Checkout, bool, String)> {
    ensure!(should_continue()?, "GitHub status lookup stopped");
    let repository: Repository = serde_json::from_slice(&run(
        gh(checkout).args([
            "repo",
            "view",
            "--json",
            "nameWithOwner,url,defaultBranchRef",
        ]),
        Duration::from_secs(15),
    )?)
    .context("resolve GitHub base repository")?;
    let base = repository.remote()?;
    let mut resolved = checkout.clone();
    if resolved.head_repo.is_empty() {
        ensure!(
            checkout
                .remotes
                .iter()
                .any(|remote| remote.host.eq_ignore_ascii_case(&base.host)
                    && remote.repository.eq_ignore_ascii_case(&base.repository)),
            "GitHub CLI repository does not match a configured remote"
        );
        resolved.host = base.host.clone();
        resolved.head_repo = base.repository.clone();
    }
    let checkout = &resolved;
    ensure!(
        base.host.eq_ignore_ascii_case(&checkout.host),
        "GitHub base/head hosts differ"
    );
    let head_repository = if base.repository.eq_ignore_ascii_case(&checkout.head_repo) {
        repository
    } else {
        ensure!(should_continue()?, "GitHub status lookup stopped");
        let name = format!("{}/{}", checkout.host, checkout.head_repo);
        let head: Repository = serde_json::from_slice(&run(
            gh(checkout).args([
                "repo",
                "view",
                &name,
                "--json",
                "nameWithOwner,url,defaultBranchRef",
            ]),
            Duration::from_secs(15),
        )?)
        .context("resolve GitHub head repository")?;
        let remote = head.remote()?;
        ensure!(
            remote.host.eq_ignore_ascii_case(&checkout.host)
                && remote.repository.eq_ignore_ascii_case(&checkout.head_repo),
            "GitHub head repository identity mismatch"
        );
        head
    };
    let default_branch_name = head_repository
        .default_branch_ref
        .context("GitHub repository has no default branch")?
        .name;
    if checkout.branch == "gitbutler/workspace" {
        resolved.head_branch = default_branch_name.clone();
    }
    let default_branch = default_branch_name == resolved.head_branch;
    Ok((resolved, default_branch, base.repository))
}

impl Repository {
    fn remote(&self) -> Result<Remote> {
        let remote = parse_remote(&self.url).context("invalid GitHub repository URL")?;
        ensure!(
            remote
                .repository
                .eq_ignore_ascii_case(&self.name_with_owner),
            "GitHub repository identity mismatch"
        );
        Ok(remote)
    }
}

fn lookup_pr_number(checkout: &Checkout, base_repository: &str) -> Result<Option<u64>> {
    let (owner, _) = checkout
        .head_repo
        .split_once('/')
        .context("invalid head repository")?;
    let endpoint = format!("repos/{}/pulls", base_repository);
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
    newest_pr(&output, checkout)
}

fn pr_view(
    checkout: &Checkout,
    base_repository: &str,
    number: u64,
    fields: &str,
) -> Result<Vec<u8>> {
    let repo = format!("{}/{}", checkout.host, base_repository);
    run(
        gh(checkout).args([
            "pr",
            "view",
            &number.to_string(),
            "--repo",
            &repo,
            "--json",
            fields,
        ]),
        Duration::from_secs(20),
    )
}

pub fn pull_request_url(directory: &Path, number: u64) -> Result<String> {
    ensure!(number > 0, "invalid GitHub PR number");
    let checkout = checkout(directory)?.context("workspace checkout is unavailable")?;
    let (checkout, _, base_repository) = resolve_repository(&checkout)?;
    let repo = format!("{}/{}", checkout.host, base_repository);
    let output = pr_view(&checkout, &base_repository, number, "number,url")?;
    #[derive(Deserialize)]
    struct PrUrl {
        number: u64,
        url: String,
    }
    let pr: PrUrl = serde_json::from_slice(&output).context("parse GitHub PR URL")?;
    ensure!(pr.number == number, "GitHub PR URL number mismatch");
    let target = pr
        .url
        .strip_prefix("https://")
        .or_else(|| pr.url.strip_prefix("http://"));
    ensure!(
        target.is_some_and(|target| target.eq_ignore_ascii_case(&format!("{repo}/pull/{number}"))),
        "GitHub PR URL does not match the resolved repository and number"
    );
    Ok(pr.url)
}

const CHECK_FIELDS: &str = "nodes { __typename ... on CheckRun { status conclusion } ... on StatusContext { state } } pageInfo { hasNextPage endCursor }";

#[derive(Deserialize)]
struct Graphql<T> {
    data: Option<T>,
    errors: Option<Vec<serde_json::Value>>,
}

fn graphql_data<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let response: Graphql<T> = serde_json::from_slice(bytes).context("parse GitHub CI response")?;
    ensure!(
        response.errors.is_none_or(|errors| errors.is_empty()),
        "GitHub CI query returned errors"
    );
    response.data.context("GitHub CI query returned no data")
}

#[derive(Deserialize)]
struct HistoryData {
    repository: HistoryRepository,
}

#[derive(Deserialize)]
struct HistoryRepository {
    #[serde(rename = "ref")]
    branch: HistoryRef,
}

#[derive(Deserialize)]
struct HistoryRef {
    target: HistoryTarget,
}

#[derive(Deserialize)]
struct HistoryTarget {
    history: History,
}

#[derive(Deserialize)]
struct History {
    nodes: Vec<CiCommit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CiCommit {
    oid: String,
    #[serde(deserialize_with = "Option::deserialize")]
    status_check_rollup: Option<Rollup>,
}

#[derive(Deserialize)]
struct Rollup {
    contexts: Contexts,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Contexts {
    nodes: Vec<Check>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
struct ContextData {
    repository: ContextRepository,
}

#[derive(Deserialize)]
struct ContextRepository {
    object: CiCommit,
}

fn ci_history(bytes: &[u8]) -> Result<Vec<CiCommit>> {
    let data: HistoryData = graphql_data(bytes)?;
    let commits = data.repository.branch.target.history.nodes;
    ensure!(commits.len() <= 10, "GitHub CI history exceeds 10 commits");
    Ok(commits)
}

fn commit_ci(commit: CiCommit) -> Result<(Option<CiStatus>, bool)> {
    ensure!(!commit.oid.is_empty(), "GitHub CI commit has no identity");
    let Some(rollup) = commit.status_check_rollup else {
        return Ok((None, false));
    };
    ensure!(
        !rollup.contexts.page_info.has_next_page,
        "incomplete GitHub CI contexts"
    );
    aggregate_checks(rollup.contexts.nodes)
}

fn select_ci(commits: impl IntoIterator<Item = Result<CiCommit>>) -> Result<Option<CiStatus>> {
    let mut queued = None;
    for commit in commits {
        let (ci, started) = commit_ci(commit?)?;
        if started {
            return Ok(ci);
        }
        queued = queued.or(ci);
    }
    Ok(queued)
}

pub fn parse_ci_history(bytes: &[u8]) -> Result<Option<CiStatus>> {
    select_ci(ci_history(bytes)?.into_iter().map(Ok))
}

fn ci_query(
    checkout: &Checkout,
    repository: &str,
    query: &str,
    fields: &[(&str, &str)],
) -> Result<Vec<u8>> {
    let (owner, name) = repository
        .split_once('/')
        .context("invalid head repository")?;
    let mut command = gh(checkout);
    command.args([
        "api",
        "graphql",
        "--hostname",
        &checkout.host,
        "-f",
        &format!("query={query}"),
        "-f",
        &format!("owner={owner}"),
        "-f",
        &format!("name={name}"),
    ]);
    for (key, value) in fields {
        command.args(["-f", &format!("{key}={value}")]);
    }
    run(&mut command, Duration::from_secs(20))
}

fn complete_commit(
    checkout: &Checkout,
    repository: &str,
    mut commit: CiCommit,
    should_continue: &mut impl FnMut() -> Result<bool>,
) -> Result<CiCommit> {
    let page_query = format!(
        "query($owner:String!,$name:String!,$oid:GitObjectID!,$cursor:String!) {{ repository(owner:$owner,name:$name) {{ object(oid:$oid) {{ ... on Commit {{ oid statusCheckRollup {{ contexts(first:100,after:$cursor) {{ {CHECK_FIELDS} }} }} }} }} }} }}"
    );
    if let Some(rollup) = &mut commit.status_check_rollup {
        while rollup.contexts.page_info.has_next_page {
            ensure!(should_continue()?, "GitHub status lookup stopped");
            let cursor = rollup
                .contexts
                .page_info
                .end_cursor
                .as_deref()
                .context("GitHub CI contexts have no next cursor")?;
            let bytes = ci_query(
                checkout,
                repository,
                &page_query,
                &[("oid", &commit.oid), ("cursor", cursor)],
            )?;
            let data: ContextData = graphql_data(&bytes)?;
            ensure!(
                data.repository.object.oid == commit.oid,
                "GitHub CI commit identity changed"
            );
            let next = data
                .repository
                .object
                .status_check_rollup
                .context("GitHub CI rollup disappeared")?
                .contexts;
            ensure!(
                !next.page_info.has_next_page
                    || next.page_info.end_cursor.as_deref() != Some(cursor),
                "GitHub CI cursor did not advance"
            );
            rollup.contexts.nodes.extend(next.nodes);
            rollup.contexts.page_info = next.page_info;
        }
    }
    Ok(commit)
}
