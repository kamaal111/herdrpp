use herdrpp::machines::catalog::{Destination, Profile};
use std::ffi::OsStr;

fn alpha() -> Destination {
    Destination::Remote(Profile {
        id: "ssh-alpha".into(),
        label: "Build machine".into(),
        target: "alice@build.example".into(),
        session: "engineering".into(),
        enabled: true,
    })
}

#[test]
fn remote_launch_uses_saved_target_and_session_without_shell_interpolation() {
    let command = alpha().command(OsStr::new("herdr"));
    let args: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_str().unwrap())
        .collect();
    assert_eq!(
        args,
        [
            "--remote",
            "alice@build.example",
            "--session",
            "engineering"
        ]
    );
    assert!(
        command
            .get_envs()
            .any(|(name, value)| name == "HERDR_SOCKET_PATH" && value.is_none())
    );
}

#[test]
fn local_launch_restores_exact_original_named_session() {
    let destination = Destination::Local {
        session: "local-work".into(),
    };
    let command = destination.command(OsStr::new("custom-herdr"));
    let args: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_str().unwrap())
        .collect();
    assert_eq!(args, ["--session", "local-work"]);
    assert_eq!(command.get_program(), "custom-herdr");
    assert_eq!(destination.id(), "local");
}

#[test]
fn both_launches_remove_stale_remote_and_pane_routing_environment() {
    let command = alpha().command(OsStr::new("herdr"));
    let removed: Vec<_> = command
        .get_envs()
        .filter_map(|(key, value)| value.is_none().then_some(key))
        .collect();
    assert!(removed.contains(&OsStr::new("HERDR_CLIENT_SOCKET_PATH")));
    assert!(removed.contains(&OsStr::new("HERDR_REMOTE_PREFERENCES_IDENTITY")));
    assert!(removed.contains(&OsStr::new("HERDR_PANE_ID")));
}
