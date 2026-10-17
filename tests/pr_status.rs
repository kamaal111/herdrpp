use herdrpp::pr::github::{CiStatus, Lifecycle, parse_summary};
use serde_json::{Value, json};

fn summary(state: &str, draft: bool, checks: Value) -> Value {
    json!({"number":123,"state":state,"isDraft":draft,"statusCheckRollup":checks})
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
