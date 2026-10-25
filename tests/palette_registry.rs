use herdrpp::palette::{catalog::Context, registry::Registry};

#[test]
fn bundled_registry_preserves_builtin_ids_and_availability() {
    let registry = Registry::bundled().unwrap();
    let context = Context {
        workspace_id: Some("w2".into()),
        tab_id: Some("w2:t7".into()),
        focused_pane_id: Some("w2:p8".into()),
        ..Context::default()
    };
    let items = registry.builtins(&context).unwrap();
    assert_eq!(items.len(), 16);
    assert!(items.iter().any(|item| item.id == "herdr:tab_rename"));
    assert_eq!(registry.builtins(&Context::default()).unwrap().len(), 1);
}

#[test]
fn toml_only_alias_reuses_existing_rust_handler_and_prompt() {
    let registry = Registry::parse(
        r#"
[[commands]]
id = "review:rename_tab"
title = "Rename Review Tab"
requires = ["workspace", "tab"]
handler = { kind = "rename", scope = "tab" }
[[commands.steps]]
kind = "text"
key = "name"
initial = "current_label"
validator = "visible_name"
"#,
    )
    .unwrap();
    let context = Context {
        workspace_id: Some("w2".into()),
        tab_id: Some("w2:t7".into()),
        ..Context::default()
    };
    let items = registry.builtins(&context).unwrap();
    assert_eq!(items[0].title, "Rename Review Tab");
    assert_eq!(
        items[0].command,
        herdrpp::palette::catalog::Command::Rename(herdrpp::palette::catalog::Scope::Tab)
    );
    assert_eq!(items[0].behavior.steps.len(), 1);
}

#[test]
fn invalid_handler_properties_and_missing_required_prompts_are_rejected() {
    assert!(Registry::parse("[[commands]]\nid='x'\ntitle='X'\nhandler={kind='unknown'}").is_err());
    assert!(
        Registry::parse("[[commands]]\nid='x'\ntitle='X'\nhandler={kind='split',direciton='down'}")
            .is_err()
    );
    assert!(
        Registry::parse("[[commands]]\nid='x'\ntitle='X'\nhandler={kind='split',direction='left'}")
            .is_err()
    );
    assert!(
        Registry::parse("[[commands]]\nid='x'\ntitle='X'\nhandler={kind='rename',scope='tab'}")
            .is_err()
    );
    assert!(
        Registry::parse("[[commands]]\nid='x'\ntitle='X'\nhandler={kind='close',scope='tab'}")
            .is_err()
    );
}

#[test]
fn duplicate_ids_and_unknown_step_bindings_are_rejected() {
    assert!(Registry::parse("[[commands]]\nid='x'\ntitle='X'\nhandler={kind='zoom'}\n[[commands]]\nid='x'\ntitle='Y'\nhandler={kind='zoom'}").is_err());
    assert!(Registry::parse("[[commands]]\nid='x'\ntitle='X'\nhandler={kind='rename',scope='tab'}\nsteps=[{kind='text',key='name',validator='guess'}]").is_err());
    assert!(Registry::parse("[[commands]]\nid='x'\ntitle='X'\nhandler={kind='zoom'}\nsteps=[{kind='choice',provider='guess',screen='commands'}]").is_err());
}

#[test]
fn toml_alias_runs_through_shared_input_serialization_validation_and_request_builder() {
    use herdrpp::palette::{
        api,
        app::{Action, App, Mode, Selection},
        naming::Naming,
    };
    use terminal_input::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let registry = Registry::parse(
        r#"
[[commands]]
id = "review:rename_tab"
title = "Rename Review Tab"
handler = { kind = "rename", scope = "tab" }
steps = [
  { kind = "text", key = "name", initial = "current_label", validator = "visible_name" },
  { kind = "confirm", message = "Rename to the entered name?" },
]
"#,
    )
    .unwrap();
    let context = Context {
        workspace_id: Some("w2".into()),
        tab_id: Some("w2:t7".into()),
        ..Context::default()
    };
    let items = registry.builtins(&context).unwrap();
    let items = serde_json::from_slice(&serde_json::to_vec(&items).unwrap()).unwrap();
    let mut app = App::new(
        context.clone(),
        items,
        Naming {
            tab_name: "Codex".into(),
            ..Naming::default()
        },
        Default::default(),
        None,
        Registry::bundled().unwrap().screen("commands").unwrap(),
    );
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        ))),
        Action::Continue
    );
    assert!(matches!(app.mode, Mode::Name { .. }));
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        ))),
        Action::Continue
    );
    assert!(app.prompt_error.is_some());
    app.handle(Event::Paste("Review".into()));
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        ))),
        Action::Continue
    );
    assert!(matches!(app.mode, Mode::Confirm { .. }));
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))),
        Action::Continue
    );
    assert!(matches!(app.mode, Mode::Name { .. }));
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    let selection = Selection {
        id: "review:rename_tab".into(),
        input: Some("Review".into()),
        confirmed: true,
    };
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        ))),
        Action::Execute(selection)
    );
    let selection = Selection {
        id: "review:rename_tab".into(),
        input: Some("Review".into()),
        confirmed: true,
    };
    assert!(herdrpp::palette::registry::validate_submission(&app.items[0], &selection).is_ok());
    assert!(
        herdrpp::palette::registry::validate_submission(
            &app.items[0],
            &Selection {
                confirmed: false,
                ..selection
            }
        )
        .is_err()
    );
    assert_eq!(
        api::rename_request(
            herdrpp::palette::catalog::Scope::Tab,
            &context,
            Some("Review")
        )
        .unwrap(),
        (
            "tab.rename".into(),
            serde_json::json!({"tab_id":"w2:t7","label":"Review"})
        )
    );
}

#[test]
fn handler_targets_remain_required_when_toml_omits_availability_requirements() {
    let registry =
        Registry::parse("[[commands]]\nid='x'\ntitle='Zoom'\nhandler={kind='zoom'}").unwrap();
    assert!(registry.builtins(&Context::default()).unwrap().is_empty());
    assert_eq!(
        registry
            .builtins(&Context {
                focused_pane_id: Some("w2:p8".into()),
                ..Context::default()
            })
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn malformed_definitions_identify_the_command_and_property() {
    let error = Registry::parse(
        "[[commands]]\nid='review:split'\ntitle='Split'\nhandler={kind='split',direciton='down'}",
    )
    .unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("review:split"));
    assert!(error.contains("direciton"));
}

#[test]
fn duplicate_dynamic_templates_and_incompatible_initial_values_are_rejected() {
    assert!(Registry::parse("[[commands]]\nid='a'\ntemplate=true\nhandler={kind='plugin'}\n[[commands]]\nid='b'\ntemplate=true\nhandler={kind='plugin'}").is_err());
    assert!(Registry::parse("[[commands]]\nid='x'\ntitle='Rename'\nhandler={kind='rename',scope='tab'}\nsteps=[{kind='text',key='name',initial='suggested_name',validator='visible_name'}]").is_err());
}

#[test]
fn message_substitution_preserves_literal_braces_in_command_and_target_labels() {
    let registry = Registry::parse(
        r#"
[[commands]]
id = "review:close_tab"
title = "Close {target}"
handler = { kind = "close", scope = "tab" }
steps = [{ kind = "confirm", message = "{title}: {target}?" }]
"#,
    )
    .unwrap();
    let context = Context {
        tab_id: Some("w2:t7".into()),
        tab_label: Some("Review {title}".into()),
        ..Context::default()
    };
    let items = registry.builtins(&context).unwrap();
    assert_eq!(
        herdrpp::palette::registry::format_text("{title}: {target}?", &items[0], &context),
        "Close {target}: Review {title}?"
    );
}

#[test]
fn repository_requirement_can_be_added_to_a_simple_handler_using_only_toml() {
    let registry = Registry::parse("[[commands]]\nid='review:zoom'\ntitle='Zoom in Git workspace'\nrequires=['repository']\nhandler={kind='zoom'}").unwrap();
    let context = Context {
        focused_pane_id: Some("w2:p8".into()),
        ..Context::default()
    };
    assert!(registry.builtins_in(&context, false).unwrap().is_empty());
    assert_eq!(
        registry.builtins_in(&context, true).unwrap()[0].title,
        "Zoom in Git workspace"
    );
}
