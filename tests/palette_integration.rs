use herdrpp::palette::{
    api,
    app::Selection,
    catalog::{self, Command, Context, Direction, Scope},
    cli,
};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
};
fn context() -> Context {
    serde_json::from_value(json!({"workspace_id":"w2","tab_id":"w2:t7","focused_pane_id":"w2:p8"}))
        .unwrap()
}

#[test]
fn rename_request_preserves_literal_name_and_split_uses_original_target() {
    assert_eq!(
        api::rename_request(Scope::Tab, &context(), Some("--世界 Review")).unwrap(),
        (
            "tab.rename".into(),
            json!({"tab_id":"w2:t7","label":"--世界 Review"})
        )
    );
    assert_eq!(
        api::arguments(&Command::Split(Direction::Right), &context())
            .unwrap()
            .unwrap(),
        [
            "pane",
            "split",
            "--pane",
            "w2:p8",
            "--direction",
            "right",
            "--focus"
        ]
    );
    assert!(api::rename_request(Scope::Tab, &context(), Some(" \t")).is_err());
    assert!(api::rename_request(Scope::Tab, &context(), Some("bad\nname")).is_err());
}

#[test]
fn unavailable_original_target_never_falls_back_to_current_focus() {
    let snapshot = json!({"workspaces":[{"workspace_id":"w2"}],"tabs":[{"tab_id":"w2:t7","workspace_id":"w2"}],"panes":[]});
    assert!(api::validate_context(&snapshot, &context()).is_err());
}

#[test]
fn mismatched_target_parents_are_rejected() {
    let snapshot = json!({"workspaces":[{"workspace_id":"w2"}],"tabs":[{"tab_id":"w2:t7","workspace_id":"w2"}],"panes":[{"pane_id":"w2:p8","workspace_id":"w2","tab_id":"w2:t9"}]});
    assert!(api::validate_context(&snapshot, &context()).is_err());
}

#[test]
fn selections_must_belong_to_catalog_and_close_requires_confirmation() {
    let items = catalog::builtins(&context());
    assert!(
        cli::validate_selection(
            &items,
            &Selection {
                id: "injected".into(),
                input: None,
                confirmed: true
            }
        )
        .is_err()
    );
    assert!(
        cli::validate_selection(
            &items,
            &Selection {
                id: "herdr:tab_close".into(),
                input: None,
                confirmed: false
            }
        )
        .is_err()
    );
    assert_eq!(
        cli::validate_selection(
            &items,
            &Selection {
                id: "herdr:tab_close".into(),
                input: None,
                confirmed: true
            }
        )
        .unwrap(),
        &Command::Close(Scope::Tab)
    );
}

#[test]
fn socket_transport_correlates_requests_and_preserves_original_context() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("api.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        writeln!(
            stream,
            "{}",
            json!({"id":request["id"],"result":{"type":"ok"}})
        )
        .unwrap();
        request
    });
    assert_eq!(
        api::request(
            &socket,
            "plugin.action.invoke",
            json!({"context":context()})
        )
        .unwrap(),
        json!({"type":"ok"})
    );
    let request = server.join().unwrap();
    assert_eq!(request["method"], "plugin.action.invoke");
    assert_eq!(request["params"]["context"]["focused_pane_id"], "w2:p8");
}

#[test]
fn socket_transport_rejects_uncorrelated_response_without_retry() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("api.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        writeln!(stream, "{}", json!({"id":"wrong","result":{"type":"ok"}})).unwrap();
    });
    assert!(
        api::request(&socket, "plugin.action.invoke", json!({}))
            .unwrap_err()
            .to_string()
            .contains("ID mismatch")
    );
    server.join().unwrap();
}

#[test]
fn creation_requests_use_original_workspace_and_preserve_literal_custom_names() {
    let naming = herdrpp::palette::naming::Naming {
        workspace_cwd: Some("/projects/herdrpp".into()),
        workspace_suggestion: "herdrpp".into(),
        tab_suggestion: "3".into(),
        ..Default::default()
    };
    assert_eq!(
        api::create_request(Scope::Tab, &context(), &naming, Some("--世界 Review")).unwrap(),
        (
            "tab.create".into(),
            json!({"workspace_id":"w2","focus":true,"label":"--世界 Review"})
        )
    );
    assert_eq!(
        api::create_request(Scope::Workspace, &context(), &naming, Some("Review")).unwrap(),
        (
            "workspace.create".into(),
            json!({"source_workspace_id":"w2","cwd":"/projects/herdrpp","focus":true,"label":"Review"})
        )
    );
    assert_eq!(
        api::create_request(Scope::Tab, &context(), &naming, Some("3"))
            .unwrap()
            .1["label"],
        serde_json::Value::Null
    );
    assert_eq!(
        api::create_request(Scope::Workspace, &context(), &naming, Some("herdrpp"))
            .unwrap()
            .1["label"],
        serde_json::Value::Null
    );
    assert_eq!(
        api::create_request(Scope::Tab, &context(), &naming, Some("  "))
            .unwrap()
            .1["label"],
        serde_json::Value::Null
    );
    assert!(api::create_request(Scope::Tab, &context(), &naming, Some("bad\nname")).is_err());
    assert!(api::create_request(Scope::Tab, &context(), &naming, None).is_err());
    assert!(api::create_request(Scope::Tab, &Context::default(), &naming, Some("Review")).is_err());
}

#[test]
fn creation_selection_requires_prompt_input_and_rejects_control_characters() {
    let items = catalog::builtins(&context());
    assert_eq!(
        cli::validate_selection(
            &items,
            &Selection {
                id: "herdr:tab_create".into(),
                input: Some(String::new()),
                confirmed: false
            }
        )
        .unwrap(),
        &Command::Create(Scope::Tab)
    );
    assert!(
        cli::validate_selection(
            &items,
            &Selection {
                id: "herdr:tab_create".into(),
                input: None,
                confirmed: false
            }
        )
        .is_err()
    );
    assert!(
        cli::validate_selection(
            &items,
            &Selection {
                id: "herdr:tab_create".into(),
                input: Some("name".into()),
                confirmed: true
            }
        )
        .is_err()
    );
    assert!(
        cli::validate_selection(
            &items,
            &Selection {
                id: "herdr:tab_create".into(),
                input: Some("bad\nname".into()),
                confirmed: false
            }
        )
        .is_err()
    );
}

#[test]
fn view_pull_request_selection_requires_no_input_or_confirmation() {
    let command = Command::ViewPullRequest {
        number: 123,
        directory: "/projects/herdrpp".into(),
    };
    let items = vec![catalog::Item {
        id: "herdr:view_pull_request".into(),
        title: "View Pull Request".into(),
        source: "Herdr".into(),
        description: String::new(),
        command: command.clone(),
    }];
    let selection = Selection {
        id: "herdr:view_pull_request".into(),
        input: None,
        confirmed: false,
    };
    assert_eq!(
        cli::validate_selection(&items, &selection).unwrap(),
        &command
    );
    assert!(
        cli::validate_selection(
            &items,
            &Selection {
                input: Some("123".into()),
                id: selection.id.clone(),
                confirmed: false,
            }
        )
        .is_err()
    );
    assert!(
        cli::validate_selection(
            &items,
            &Selection {
                confirmed: true,
                ..selection
            }
        )
        .is_err()
    );
    assert!(api::arguments(&command, &context()).unwrap().is_none());
}
