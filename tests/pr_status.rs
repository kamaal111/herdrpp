use herdrpp::pr::github::{CiStatus, Lifecycle, parse_ci_history, parse_summary};
use serde_json::{Value, json};

fn summary(state: &str, draft: bool, checks: Value) -> Value {
    let rollup = checks.as_array().map(|checks| {
        json!({"contexts": {
            "nodes": checks, "pageInfo": {"hasNextPage": false, "endCursor": null}
        }})
    });
    json!({"number":123,"state":state,"isDraft":draft,"commits":{"nodes":[{
        "commit":{"oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","statusCheckRollup":rollup}
    }]}})
}

fn check(status: &str, conclusion: Option<&str>) -> Value {
    json!({"__typename":"CheckRun","status":status,"conclusion":conclusion})
}

fn parse(value: Value) -> herdrpp::pr::github::PrSummary {
    parse_summary(value.to_string().as_bytes(), 123).unwrap()
}

#[test]
fn lifecycle_uses_merged_and_closed_before_draft() {
    assert_eq!(
        parse(summary("OPEN", false, json!([]))).lifecycle,
        Lifecycle::Open
    );
    assert_eq!(
        parse(summary("OPEN", true, json!([]))).lifecycle,
        Lifecycle::Draft
    );
    assert_eq!(
        parse(summary("MERGED", true, json!([]))).lifecycle,
        Lifecycle::Merged
    );
    assert_eq!(
        parse(summary("CLOSED", true, json!([]))).lifecycle,
        Lifecycle::Closed
    );
}

#[test]
fn successful_check_runs_and_commit_statuses_pass() {
    for value in [
        check("COMPLETED", Some("SUCCESS")),
        json!({"__typename":"StatusContext","state":"SUCCESS"}),
    ] {
        assert_eq!(
            parse(summary("OPEN", false, json!([value]))).ci,
            Some(CiStatus::Passed)
        );
    }
}

#[test]
fn pending_check_runs_and_commit_statuses_are_pending() {
    for status in ["IN_PROGRESS", "PENDING", "QUEUED", "REQUESTED", "WAITING"] {
        assert_eq!(
            parse(summary("OPEN", false, json!([check(status, None)]))).ci,
            Some(CiStatus::Pending)
        );
    }
    for state in ["EXPECTED", "PENDING"] {
        assert_eq!(
            parse(summary(
                "OPEN",
                false,
                json!([{"__typename":"StatusContext","state":state}])
            ))
            .ci,
            Some(CiStatus::Pending)
        );
    }
}

#[test]
fn github_cli_empty_conclusion_is_pending_until_completed() {
    assert_eq!(
        parse(summary(
            "OPEN",
            false,
            json!([check("IN_PROGRESS", Some(""))])
        ))
        .ci,
        Some(CiStatus::Pending)
    );
    assert!(
        parse_summary(
            summary("OPEN", false, json!([check("COMPLETED", Some(""))]))
                .to_string()
                .as_bytes(),
            123
        )
        .is_err()
    );
}

#[test]
fn failures_win_over_pending_and_passing() {
    let checks = json!([
        check("COMPLETED", Some("SUCCESS")),
        check("QUEUED", None),
        check("COMPLETED", Some("FAILURE"))
    ]);
    assert_eq!(
        parse(summary("OPEN", false, checks)).ci,
        Some(CiStatus::Failed)
    );
}

#[test]
fn pending_wins_over_passing() {
    let checks = json!([check("COMPLETED", Some("SUCCESS")), check("WAITING", None)]);
    assert_eq!(
        parse(summary("OPEN", false, checks)).ci,
        Some(CiStatus::Pending)
    );
}

#[test]
fn unsuccessful_conclusions_fail() {
    for conclusion in [
        "FAILURE",
        "CANCELLED",
        "TIMED_OUT",
        "ACTION_REQUIRED",
        "STALE",
        "STARTUP_FAILURE",
    ] {
        assert_eq!(
            parse(summary(
                "OPEN",
                false,
                json!([check("COMPLETED", Some(conclusion))])
            ))
            .ci,
            Some(CiStatus::Failed)
        );
    }
    for state in ["ERROR", "FAILURE"] {
        assert_eq!(
            parse(summary(
                "OPEN",
                false,
                json!([{"__typename":"StatusContext","state":state}])
            ))
            .ci,
            Some(CiStatus::Failed)
        );
    }
}

#[test]
fn no_checks_and_only_neutral_or_skipped_checks_have_no_icon() {
    assert_eq!(parse(summary("OPEN", false, json!([]))).ci, None);
    assert_eq!(parse(summary("OPEN", false, json!(null))).ci, None);
    assert_eq!(
        parse(summary(
            "OPEN",
            false,
            json!([
                check("COMPLETED", Some("NEUTRAL")),
                check("COMPLETED", Some("SKIPPED"))
            ])
        ))
        .ci,
        None
    );
}

#[test]
fn malformed_unknown_and_mismatched_details_are_errors() {
    assert!(parse_summary(b"invalid", 123).is_err());
    assert!(parse_summary(b"{}", 123).is_err());
    assert!(
        parse_summary(
            summary("OPEN", false, json!([])).to_string().as_bytes(),
            124
        )
        .is_err()
    );
    assert!(
        parse_summary(
            summary("UNKNOWN", false, json!([])).to_string().as_bytes(),
            123
        )
        .is_err()
    );
    assert!(
        parse_summary(
            summary("OPEN", false, json!([check("COMPLETED", None)]))
                .to_string()
                .as_bytes(),
            123
        )
        .is_err()
    );
    assert!(
        parse_summary(
            summary("OPEN", false, json!([check("COMPLETED", Some("UNKNOWN"))]))
                .to_string()
                .as_bytes(),
            123
        )
        .is_err()
    );
    assert!(
        parse_summary(
            summary("OPEN", false, json!([{"__typename":"Unknown"}]))
                .to_string()
                .as_bytes(),
            123
        )
        .is_err()
    );
}

fn history(checks: Vec<Value>) -> Value {
    json!({"data": {"repository": {"ref": {"target": {"history": {"nodes":
        checks.into_iter().enumerate().map(|(index, checks)| json!({
            "oid": format!("{:040x}", index + 1), "statusCheckRollup": {"contexts": {
                "nodes": checks, "pageInfo": {"hasNextPage": false, "endCursor": null}
            }}
        })).collect::<Vec<_>>()
    }}}}}})
}

fn ci(checks: Vec<Value>) -> Option<CiStatus> {
    parse_ci_history(history(checks).to_string().as_bytes()).unwrap()
}

#[test]
fn ci_uses_newest_started_commit_instead_of_newer_queued_checks() {
    assert_eq!(
        ci(vec![
            json!([check("QUEUED", None)]),
            json!([check("COMPLETED", Some("FAILURE"))])
        ]),
        Some(CiStatus::Failed)
    );
    assert_eq!(
        ci(vec![
            json!([]),
            json!([check("IN_PROGRESS", None)]),
            json!([check("COMPLETED", Some("SUCCESS"))])
        ]),
        Some(CiStatus::Pending)
    );
}

#[test]
fn ci_aggregates_all_checks_on_selected_commit_without_combining_commits() {
    assert_eq!(
        ci(vec![
            json!([check("COMPLETED", Some("SUCCESS")), check("QUEUED", None)]),
            json!([check("COMPLETED", Some("FAILURE"))])
        ]),
        Some(CiStatus::Pending)
    );
    assert_eq!(
        ci(vec![
            json!([check("COMPLETED", Some("SUCCESS"))]),
            json!([check("COMPLETED", Some("FAILURE"))])
        ]),
        Some(CiStatus::Passed)
    );
}

#[test]
fn ci_uses_queued_fallback_and_ignores_neutral_completed_checks_for_selection() {
    assert_eq!(
        ci(vec![
            json!([check("COMPLETED", Some("SKIPPED")), check("QUEUED", None)]),
            json!([check("COMPLETED", Some("SUCCESS"))])
        ]),
        Some(CiStatus::Passed)
    );
    assert_eq!(
        ci(vec![json!([check("QUEUED", None)]), json!([])]),
        Some(CiStatus::Pending)
    );
    assert_eq!(ci(vec![json!([check("COMPLETED", Some("NEUTRAL"))])]), None);
}

#[test]
fn ci_handles_legacy_statuses_and_empty_history() {
    assert_eq!(
        ci(vec![
            json!([{"__typename":"StatusContext","state":"PENDING"}]),
            json!([{"__typename":"StatusContext","state":"ERROR"}])
        ]),
        Some(CiStatus::Failed)
    );
    assert_eq!(ci(vec![]), None);
    assert_eq!(ci(vec![json!([]); 10]), None);
}

#[test]
fn ci_rejects_excess_history_errors_and_incomplete_contexts() {
    assert!(parse_ci_history(history(vec![json!([]); 11]).to_string().as_bytes()).is_err());
    assert!(parse_ci_history(br#"{"data":null,"errors":[{"message":"unavailable"}]}"#).is_err());
    assert!(parse_ci_history(b"{}").is_err());
    let mut value = history(vec![json!([])]);
    value["data"]["repository"]["ref"]["target"]["history"]["nodes"][0]["statusCheckRollup"]["contexts"]
        ["pageInfo"]["hasNextPage"] = json!(true);
    assert!(parse_ci_history(value.to_string().as_bytes()).is_err());
}

#[test]
fn ci_rejects_missing_rollup_but_accepts_explicit_null() {
    let mut value = history(vec![json!([])]);
    value["data"]["repository"]["ref"]["target"]["history"]["nodes"][0]["statusCheckRollup"] =
        Value::Null;
    assert_eq!(
        parse_ci_history(value.to_string().as_bytes()).unwrap(),
        None
    );
    value["data"]["repository"]["ref"]["target"]["history"]["nodes"][0]
        .as_object_mut()
        .unwrap()
        .remove("statusCheckRollup");
    assert!(parse_ci_history(value.to_string().as_bytes()).is_err());
}

#[test]
fn pr_requires_one_head_commit_with_complete_contexts() {
    let mut missing_head = summary("OPEN", false, json!([]));
    missing_head["commits"]["nodes"] = json!([]);
    assert!(parse_summary(missing_head.to_string().as_bytes(), 123).is_err());

    let mut multiple_heads = summary("OPEN", false, json!([]));
    let head = multiple_heads["commits"]["nodes"][0].clone();
    multiple_heads["commits"]["nodes"] = json!([head, head]);
    assert!(parse_summary(multiple_heads.to_string().as_bytes(), 123).is_err());

    let mut incomplete = summary("OPEN", false, json!([]));
    incomplete["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["pageInfo"]["hasNextPage"] =
        json!(true);
    assert!(parse_summary(incomplete.to_string().as_bytes(), 123).is_err());

    let mut missing_rollup = summary("OPEN", false, json!([]));
    missing_rollup["commits"]["nodes"][0]["commit"]
        .as_object_mut()
        .unwrap()
        .remove("statusCheckRollup");
    assert!(parse_summary(missing_rollup.to_string().as_bytes(), 123).is_err());
}
