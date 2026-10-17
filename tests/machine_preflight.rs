use herdrpp::machines::catalog::{Destination, Profile, preflight};
use std::{fs, os::unix::fs::PermissionsExt, sync::Mutex};

static CLI_TEST_LOCK: Mutex<()> = Mutex::new(());

fn check(stdout: &str, stderr: &str, exit: i32) -> anyhow::Result<()> {
    let _guard = CLI_TEST_LOCK.lock().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("herdr");
    fs::write(
        &executable,
        format!("#!/bin/sh\ncat <<'STATUS_JSON'\n{stdout}\nSTATUS_JSON\ncat >&2 <<'STATUS_ERROR'\n{stderr}\nSTATUS_ERROR\nexit {exit}\n"),
    ).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    preflight(
        executable.as_os_str(),
        &Destination::Remote(Profile {
            id: "alpha".into(),
            label: "Alpha".into(),
            target: "alice@build".into(),
            session: "engineering".into(),
            enabled: true,
        }),
    )
}

#[test]
fn nonzero_status_surfaces_structured_ssh_failure_from_stdout() {
    let error = check(
        r#"[{"id":"alpha","status":"error","error":"ssh: connect: Connection refused"}]"#,
        "",
        1,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Alpha: error: ssh: connect: Connection refused")
    );
}

#[test]
fn nonzero_status_never_accepts_a_reachable_record() {
    let error = check(
        r#"[{"id":"alpha","status":"reachable","error":null}]"#,
        "probe failed",
        17,
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("17") && message.contains("probe failed"));
}

#[test]
fn malformed_status_preserves_exit_and_stderr_context() {
    let error = check("not json", "SSH unavailable", 23).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("23") && message.contains("SSH unavailable"));
}
