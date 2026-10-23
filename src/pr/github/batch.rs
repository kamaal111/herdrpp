use super::*;
use crate::command::run_cancellable;
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

pub(crate) const STATUS_BATCH_SIZE: usize = 10;

type LookupResult = std::result::Result<Option<WorkspaceStatus>, LookupFailure>;

#[derive(Clone)]
enum TargetKind {
    PullRequest {
        base_repository: String,
        number: u64,
    },
    DefaultBranch,
}

#[derive(Clone)]
pub(crate) struct StatusTarget {
    checkout: Checkout,
    kind: TargetKind,
}

impl StatusTarget {
    fn failure(&self, error: anyhow::Error) -> LookupFailure {
        LookupFailure {
            default_branch: matches!(self.kind, TargetKind::DefaultBranch),
            message: format!("{error:#}"),
        }
    }
}

pub(crate) fn prepare(
    checkout: &Checkout,
    should_continue: &mut impl FnMut() -> Result<bool>,
) -> std::result::Result<Option<StatusTarget>, LookupFailure> {
    let (checkout, default_branch, base_repository) =
        resolve_repository_while(checkout, should_continue)?;
    let kind = if default_branch {
        TargetKind::DefaultBranch
    } else {
        if !should_continue()? {
            return Err(anyhow::anyhow!("GitHub status lookup stopped").into());
        }
        let Some(number) = lookup_pr_number(&checkout, &base_repository)? else {
            return Ok(None);
        };
        TargetKind::PullRequest {
            base_repository,
            number,
        }
    };
    Ok(Some(StatusTarget { checkout, kind }))
}

pub(crate) fn fetch_statuses(
    targets: &[StatusTarget],
    should_continue: &mut impl FnMut() -> Result<bool>,
) -> Vec<LookupResult> {
    let mut hosts: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, target) in targets.iter().enumerate() {
        hosts.entry(&target.checkout.host).or_default().push(index);
    }
    let mut results: Vec<LookupResult> = targets
        .iter()
        .map(|target| Err(target.failure(anyhow::anyhow!("GitHub status result missing"))))
        .collect();
    for indices in hosts.values() {
        for chunk in indices.chunks(STATUS_BATCH_SIZE) {
            let batch: Vec<_> = chunk.iter().map(|&index| &targets[index]).collect();
            match request(&batch, should_continue) {
                Ok(response) => {
                    for (alias, &index) in chunk.iter().enumerate() {
                        results[index] =
                            parse_target(&response, alias, &targets[index], should_continue)
                                .map(Some)
                                .map_err(|error| targets[index].failure(error));
                    }
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    for &index in chunk {
                        results[index] =
                            Err(targets[index].failure(anyhow::anyhow!(message.clone())));
                    }
                }
            }
        }
    }
    results
}

fn request(
    targets: &[&StatusTarget],
    should_continue: &mut impl FnMut() -> Result<bool>,
) -> Result<Value> {
    ensure!(should_continue()?, "GitHub status lookup stopped");
    let mut declarations = Vec::new();
    let mut selections = Vec::new();
    let mut variables = Map::new();
    for (index, target) in targets.iter().enumerate() {
        let repository = match &target.kind {
            TargetKind::PullRequest {
                base_repository, ..
            } => base_repository,
            TargetKind::DefaultBranch => &target.checkout.head_repo,
        };
        let (owner, name) = repository
            .split_once('/')
            .context("invalid GitHub repository")?;
        declarations.extend([
            format!("$owner{index}:String!"),
            format!("$name{index}:String!"),
        ]);
        variables.insert(format!("owner{index}"), json!(owner));
        variables.insert(format!("name{index}"), json!(name));
        declarations.push(format!("$branch{index}:String!"));
        variables.insert(
            format!("branch{index}"),
            json!(format!("refs/heads/{}", target.checkout.head_branch)),
        );
        let selection = match &target.kind {
            TargetKind::PullRequest { number, .. } => {
                declarations.push(format!("$pr{index}:Int!"));
                variables.insert(format!("pr{index}"), json!(number));
                format!(
                    "pullRequest(number:$pr{index}) {{ number state isDraft commits(last:1) {{ nodes {{ commit {{ oid statusCheckRollup {{ contexts(first:100) {{ {CHECK_FIELDS} }} }} }} }} }} }}"
                )
            }
            TargetKind::DefaultBranch => history_selection(index),
        };
        selections.push(format!(
            "target{index}: repository(owner:$owner{index},name:$name{index}) {{ {selection} }}"
        ));
        if matches!(target.kind, TargetKind::PullRequest { .. }) {
            let (owner, name) = target
                .checkout
                .head_repo
                .split_once('/')
                .context("invalid GitHub head repository")?;
            declarations.extend([
                format!("$ciOwner{index}:String!"),
                format!("$ciName{index}:String!"),
            ]);
            variables.insert(format!("ciOwner{index}"), json!(owner));
            variables.insert(format!("ciName{index}"), json!(name));
            selections.push(format!(
                "ci{index}: repository(owner:$ciOwner{index},name:$ciName{index}) {{ {} }}",
                history_selection(index)
            ));
        }
    }
    let query = format!(
        "query StatusBatch({}) {{ {} }}",
        declarations.join(","),
        selections.join(" ")
    );
    let mut command = gh(&targets[0].checkout);
    command.args([
        "api",
        "graphql",
        "--hostname",
        &targets[0].checkout.host,
        "-f",
        &format!("query={query}"),
    ]);
    for (name, value) in variables {
        let (flag, value) = match value {
            Value::String(value) => ("-f", value),
            value => ("-F", value.to_string()),
        };
        command.args([flag, &format!("{name}={value}")]);
    }
    let output = run_cancellable(
        &mut command,
        Duration::from_secs(20),
        4 * 1024 * 1024,
        &AtomicBool::new(false),
        "GitHub status batch",
    )?;
    let response: Value = serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "parse GitHub status batch ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )
    })?;
    ensure!(
        output.status.success()
            || response
                .get("errors")
                .and_then(Value::as_array)
                .is_some_and(|errors| !errors.is_empty()),
        "GitHub status batch failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(response)
}

fn history_selection(index: usize) -> String {
    format!(
        "ref(qualifiedName:$branch{index}) {{ target {{ ... on Commit {{ history(first:10) {{ nodes {{ oid statusCheckRollup {{ contexts(first:100) {{ {CHECK_FIELDS} }} }} }} }} }} }} }}"
    )
}

fn repository_result<'a>(response: &'a Value, alias: &str) -> Result<&'a Value> {
    if let Some(errors) = response.get("errors") {
        for error in errors.as_array().context("invalid GitHub GraphQL errors")? {
            let path = error.get("path").and_then(Value::as_array);
            ensure!(
                path.and_then(|path| path.first())
                    .and_then(Value::as_str)
                    .is_some_and(|name| name != alias),
                "GitHub status query returned errors: {error}"
            );
        }
    }
    response
        .get("data")
        .and_then(|data| data.get(alias))
        .filter(|value| !value.is_null())
        .context("GitHub status target missing")
}

fn branch_ci(
    repository: &Value,
    checkout: &Checkout,
    should_continue: &mut impl FnMut() -> Result<bool>,
) -> Result<Option<CiStatus>> {
    let repository: HistoryRepository = serde_json::from_value(repository.clone())?;
    let commits = repository.commits()?;
    select_ci(
        commits
            .into_iter()
            .map(|commit| complete_commit(checkout, &checkout.head_repo, commit, should_continue)),
    )
}

fn parse_target(
    response: &Value,
    index: usize,
    target: &StatusTarget,
    should_continue: &mut impl FnMut() -> Result<bool>,
) -> Result<WorkspaceStatus> {
    let repository = repository_result(response, &format!("target{index}"))?;
    match target.kind {
        TargetKind::PullRequest {
            number,
            ref base_repository,
        } => {
            let mut details: PrDetails = serde_json::from_value(
                repository
                    .get("pullRequest")
                    .context("GitHub PR missing")?
                    .clone(),
            )?;
            let history = repository_result(response, &format!("ci{index}"))?;
            if history.get("ref") != Some(&Value::Null) {
                let ci = branch_ci(history, &target.checkout, should_continue)?;
                return details
                    .metadata
                    .summary(number, ci)
                    .map(WorkspaceStatus::PullRequest);
            }
            details.commits.nodes = details
                .commits
                .nodes
                .into_iter()
                .map(|node| {
                    complete_commit(
                        &target.checkout,
                        base_repository,
                        node.commit,
                        should_continue,
                    )
                    .map(|commit| PrCommit { commit })
                })
                .collect::<Result<_>>()?;
            summarize_pr(details, number).map(WorkspaceStatus::PullRequest)
        }
        TargetKind::DefaultBranch => {
            let ci = branch_ci(repository, &target.checkout, should_continue)?;
            Ok(WorkspaceStatus::DefaultBranch { ci })
        }
    }
}
