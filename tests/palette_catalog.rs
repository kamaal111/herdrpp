use herdrpp::{
    palette::catalog::{self, Context},
    search::matcher::{FieldMatcher, Query},
};
use serde_json::json;

fn context() -> Context {
    serde_json::from_value(
        json!({"workspace_id":"w2", "tab_id":"w2:t7", "focused_pane_id":"w2:p8"}),
    )
    .unwrap()
}

#[test]
fn builtin_catalog_has_all_sixteen_actions_and_no_client_only_actions() {
    let items = catalog::builtins(&context());
    assert_eq!(items.len(), 16);
    assert!(!items.iter().any(|item| item.title.contains("Settings")));
}

#[test]
fn palette_uses_shared_match_scores_and_highlights() {
    let items = catalog::builtins(&context());
    let hits = catalog::search(&items, "SPL RGT", &mut FieldMatcher::default());
    let item = &items[hits[0].item];
    assert_eq!(item.title, "Split Pane Right");
    assert_eq!(
        hits[0].matched,
        FieldMatcher::default()
            .matches(&item.fields(), &Query::new("SPL RGT"))
            .unwrap()
    );
    assert!(!hits[0].matched.indices[3].is_empty());
}

#[test]
fn all_terms_are_required_and_whitespace_restores_alphabetical_order() {
    let items = catalog::builtins(&context());
    assert!(catalog::search(&items, "split nonexistent", &mut FieldMatcher::default()).is_empty());
    assert_eq!(
        catalog::search(&items, " \t", &mut FieldMatcher::default()).len(),
        items.len()
    );
    assert_eq!(items[0].title, "Close Pane");
}

#[test]
fn plugin_catalog_filters_disabled_platform_and_missing_selection_context() {
    let plugins = json!({"plugins":[{"plugin_id":"tools","name":"Tools","enabled":true},{"plugin_id":"off","name":"Off","enabled":false}]});
    let actions = json!({"actions":[
        {"plugin_id":"tools","action_id":"run","title":"Run","contexts":["pane"]},
        {"plugin_id":"tools","action_id":"windows","title":"Windows","platforms":["windows"]},
        {"plugin_id":"tools","action_id":"selection","title":"Selection","contexts":["selection"]},
        {"plugin_id":"off","action_id":"run","title":"Run"}
    ]});
    let items = catalog::plugin_items(plugins, actions, &context(), "macos").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "tools.run");
}

#[test]
fn plugin_title_and_identity_are_searchable_without_merging_duplicate_titles() {
    let items = catalog::plugin_items(
        json!({"plugins":[{"plugin_id":"herdrpp","name":"herdr++","enabled":true}]}),
        json!({"actions":[{"plugin_id":"herdrpp","action_id":"refresh","title":"Refresh"}, {"plugin_id":"herdrpp","action_id":"other","title":"Refresh"}]}),
        &context(), "linux").unwrap();
    assert_eq!(items.len(), 2);
    let hits = catalog::search(&items, "herdrpp refresh", &mut FieldMatcher::default());
    assert_eq!(hits.len(), 2);
    assert_eq!(items[hits[0].item].id, "herdrpp.other");
}

#[test]
fn duplicate_plugin_action_ids_are_rejected_even_with_different_titles() {
    let result = catalog::plugin_items(
        json!({"plugins":[{"plugin_id":"tools","name":"Tools","enabled":true}]}),
        json!({"actions":[{"plugin_id":"tools","action_id":"run","title":"A"},
                          {"plugin_id":"tools","action_id":"other","title":"B"},
                          {"plugin_id":"tools","action_id":"run","title":"C"}]}),
        &context(),
        "linux",
    );
    assert!(result.is_err());
}
