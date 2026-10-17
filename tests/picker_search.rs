use herdrpp::picker::{
    api::parse_snapshot,
    config::SearchMode,
    model::Target,
    search::{Row, RowKind, Search},
};

fn rows(query: &str, mode: SearchMode) -> Vec<Row> {
    let snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    Search::default().rows(&snapshot, query, mode)
}
fn ids(rows: &[Row]) -> Vec<Target> {
    rows.iter().map(|r| r.target.clone()).collect()
}
fn workspace(id: &str) -> Target {
    Target::Workspace(id.into())
}
fn tab(id: &str) -> Target {
    Target::Tab(id.into())
}

#[test]
fn workspace_match_reveals_all_tabs_in_ui_order() {
    assert_eq!(
        ids(&rows("ColorPicker", SearchMode::Hierarchical)),
        vec![workspace("w1"), tab("w1:t4"), tab("w1:t8"), tab("w1:t9")]
    );
}
#[test]
fn noncontiguous_case_insensitive_workspace_match() {
    assert_eq!(rows("CLP", SearchMode::Hierarchical).len(), 4);
}
#[test]
fn partial_tab_matches_include_selectable_parent() {
    assert_eq!(
        ids(&rows("serv", SearchMode::Hierarchical)),
        vec![workspace("w2"), tab("w2:t2")]
    );
}
#[test]
fn tab_matches_keep_group_order_instead_of_fuzzy_rank() {
    assert_eq!(
        ids(&rows("codex", SearchMode::Hierarchical)),
        vec![
            workspace("w2"),
            tab("w2:t7"),
            workspace("w1"),
            tab("w1:t8"),
            workspace("w3"),
            tab("w3:t1")
        ]
    );
}
#[test]
fn query_terms_match_across_parent_and_child() {
    assert_eq!(
        ids(&rows("color cod", SearchMode::Hierarchical)),
        vec![workspace("w1"), tab("w1:t4"), tab("w1:t8")]
    );
}
#[test]
fn full_names_match_across_hierarchy() {
    assert_eq!(
        ids(&rows("colorpicker codex", SearchMode::Hierarchical)),
        vec![workspace("w1"), tab("w1:t8")]
    );
}
#[test]
fn multiple_terms_have_and_semantics() {
    assert!(
        rows(
            "colorpicker randomnonexistentterm",
            SearchMode::Hierarchical
        )
        .is_empty()
    );
}
#[test]
fn no_results() {
    assert!(rows("zzzzzzz", SearchMode::Flat).is_empty());
}
#[test]
fn empty_query_has_every_selectable_item() {
    assert_eq!(rows("", SearchMode::Hierarchical).len(), 9);
}
#[test]
fn whitespace_query_has_every_selectable_item() {
    assert_eq!(rows("  \t", SearchMode::Flat).len(), 9);
}
#[test]
fn branch_metadata_is_not_searchable() {
    assert!(rows("secret-branch", SearchMode::Flat).is_empty());
}
#[test]
fn paths_are_not_searchable() {
    assert!(rows("secret/path", SearchMode::Flat).is_empty());
}
#[test]
fn flat_workspace_result_is_independent() {
    assert_eq!(
        ids(&rows("TCG", SearchMode::Flat)),
        vec![workspace("w2"), tab("w2:t7"), tab("w2:t2")]
    );
}
#[test]
fn flat_tab_retains_parent_context() {
    let result = rows("server", SearchMode::Flat);
    assert_eq!(ids(&result), vec![workspace("w2"), tab("w2:t2")]);
    assert_eq!(result[1].ancestors[0].label, "TCG");
}
#[test]
fn flat_rank_puts_exact_name_above_longer_matches() {
    let result: Vec<_> = rows("Codex", SearchMode::Flat)
        .into_iter()
        .filter(|row| !row.label_matches.is_empty())
        .collect();
    assert_eq!(result[0].label, "Codex");
    assert!(result.windows(2).all(|r| r[0].score >= r[1].score));
}
#[test]
fn highlights_match_both_hierarchy_labels() {
    let result = rows("col codex", SearchMode::Flat);
    assert_eq!(
        ids(&result),
        vec![workspace("w1"), tab("w1:t8"), workspace("w3"), tab("w3:t1")]
    );
    let codex = result
        .iter()
        .find(|row| row.target == tab("w1:t8"))
        .unwrap();
    assert_eq!(codex.label_matches, vec![0, 1, 2, 3, 4]);
    assert_eq!(codex.ancestors[0].matches, vec![0, 1, 2]);
}
#[test]
fn query_term_order_is_independent() {
    assert_eq!(
        ids(&rows("cod color", SearchMode::Flat)),
        ids(&rows("color cod", SearchMode::Flat))
    );
}
#[test]
fn duplicate_display_names_keep_distinct_ids() {
    let result = rows("codex", SearchMode::Flat);
    assert!(ids(&result).contains(&tab("w1:t8")));
    assert!(ids(&result).contains(&tab("w2:t7")));
}

#[test]
fn one_term_matches_across_parent_and_child_without_spaces() {
    let result = rows("pickercod", SearchMode::Flat);
    assert_eq!(
        ids(&result),
        vec![workspace("w1"), tab("w1:t8"), tab("w1:t4")]
    );
    let codex = result
        .iter()
        .find(|row| row.target == tab("w1:t8"))
        .unwrap();
    assert_eq!(codex.ancestors[0].matches, (5..11).collect::<Vec<_>>());
    assert_eq!(codex.label_matches, vec![0, 1, 2]);
    assert_eq!(
        ids(&rows("pickercod", SearchMode::Hierarchical)),
        vec![workspace("w1"), tab("w1:t4"), tab("w1:t8")]
    );
}

#[test]
fn unicode_match_indices_address_visible_codepoints() {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    snapshot.workspaces[0].label = "日本ColorPic".into();
    let result = Search::default().rows(&snapshot, "colorpic", SearchMode::Hierarchical);
    assert_eq!(result[0].label_matches, vec![2, 3, 4, 5, 6, 7, 8, 9]);
}

fn assert_single_tab_workspace(mode: SearchMode) {
    let snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    for query in ["", "  ", "exact", "terminal", "exact terminal"] {
        let result = Search::default().results(&snapshot, query, mode);
        let owner = result
            .iter()
            .find(|result| result.workspace.workspace_id == "w3")
            .unwrap();
        assert_eq!(owner.workspace.target, workspace("w3"));
        assert_eq!(owner.workspace.label, "Exact Codex");
        assert_eq!(ids(&owner.tabs), vec![tab("w3:t1")]);
        assert_eq!(owner.tabs[0].label, "Terminal");
    }
    let result = Search::default().results(&snapshot, "exact terminal", mode);
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0].workspace.label_matches,
        (0..5).collect::<Vec<_>>()
    );
    assert_eq!(result[0].tabs[0].label_matches, (0..8).collect::<Vec<_>>());
    assert!(!result[0].workspace.active);
    assert_eq!(
        ids(&rows("codexterminal", mode)),
        vec![workspace("w3"), tab("w3:t1")]
    );
    assert!(rows("exact nonexistent", mode).is_empty());
}

#[test]
fn hierarchical_single_tab_workspace_has_one_grouped_result() {
    assert_single_tab_workspace(SearchMode::Hierarchical);
}

#[test]
fn flat_single_tab_workspace_has_one_grouped_result() {
    assert_single_tab_workspace(SearchMode::Flat);
}

#[test]
fn filtered_multi_tab_workspace_keeps_workspace_and_tab_targets() {
    let hierarchical = rows("server", SearchMode::Hierarchical);
    assert_eq!(ids(&hierarchical), vec![workspace("w2"), tab("w2:t2")]);
    let flat = rows("server", SearchMode::Flat);
    assert_eq!(ids(&flat), vec![workspace("w2"), tab("w2:t2")]);
}

fn worktree_rows(query: &str, mode: SearchMode) -> Vec<Row> {
    let snapshot = parse_snapshot(include_bytes!("fixtures/worktrees.json")).unwrap();
    Search::default().rows(&snapshot, query, mode)
}

#[test]
fn worktrees_are_nested_once_after_root_tabs_in_ui_order() {
    assert_eq!(
        ids(&worktree_rows("", SearchMode::Hierarchical)),
        vec![
            workspace("w1"),
            tab("w1:t1"),
            tab("w1:t2"),
            workspace("w2"),
            tab("w2:t1"),
            tab("w2:t2"),
            workspace("w3"),
            tab("w3:t1"),
            workspace("w4"),
            tab("w4:t1"),
            workspace("w5"),
            tab("w5:t1"),
            tab("w5:t2"),
            workspace("w0"),
            tab("w0:t1")
        ]
    );
}

#[test]
fn root_search_reveals_worktrees_and_their_tabs() {
    assert_eq!(worktree_rows("herdrpp", SearchMode::Hierarchical).len(), 13);
}

#[test]
fn worktree_search_retains_selectable_root_and_sole_tab() {
    let result = worktree_rows("just", SearchMode::Hierarchical);
    assert_eq!(
        ids(&result),
        vec![workspace("w1"), workspace("w4"), tab("w4:t1")]
    );
    assert_eq!(result[2].label, "Terminal");
}

#[test]
fn three_level_search_keeps_only_matching_branch_and_ancestors() {
    assert_eq!(
        ids(&worktree_rows(
            "herdrpp multimachine codex",
            SearchMode::Hierarchical
        )),
        vec![workspace("w1"), workspace("w2"), tab("w2:t1")]
    );
    assert_eq!(
        ids(&worktree_rows(
            "herdrpp just terminal",
            SearchMode::Hierarchical
        )),
        vec![workspace("w1"), workspace("w4"), tab("w4:t1")]
    );
    assert!(worktree_rows("herdrpp just nonexistent", SearchMode::Hierarchical).is_empty());
    assert_eq!(
        ids(&worktree_rows("ppmultimachine", SearchMode::Flat)),
        vec![workspace("w2"), tab("w2:t1"), tab("w2:t2")]
    );
}

#[test]
fn nested_rows_have_full_ancestor_context_and_depth() {
    let result = worktree_rows("herdrpp multimachine codex", SearchMode::Hierarchical);
    assert_eq!(
        result.iter().map(|row| row.depth).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(
        result[2]
            .ancestors
            .iter()
            .map(|a| a.label.as_str())
            .collect::<Vec<_>>(),
        vec!["herdrpp", "multimachine"]
    );
    assert_eq!(result[0].label_matches, (0..7).collect::<Vec<_>>());
    assert_eq!(result[1].label_matches, (0..12).collect::<Vec<_>>());
    assert_eq!(result[2].label_matches, (0..5).collect::<Vec<_>>());
}

#[test]
fn flat_nested_tab_matches_and_highlights_all_three_names() {
    let result = worktree_rows("herdrpp multimachine codex", SearchMode::Flat);
    assert_eq!(ids(&result), vec![workspace("w2"), tab("w2:t1")]);
    assert_eq!(result[1].ancestors[0].label, "herdrpp");
    assert_eq!(result[1].ancestors[1].label, "multimachine");
    assert_eq!(result[1].ancestors[0].matches, (0..7).collect::<Vec<_>>());
    assert_eq!(result[1].ancestors[1].matches, (0..12).collect::<Vec<_>>());
    assert_eq!(result[1].label_matches, (0..5).collect::<Vec<_>>());
}

#[test]
fn flat_worktree_results_retain_their_sole_tab() {
    let result = worktree_rows("herdrpp just terminal", SearchMode::Flat);
    assert_eq!(ids(&result), vec![workspace("w4"), tab("w4:t1")]);
    assert_eq!(result[0].ancestors[0].label, "herdrpp");
    assert_eq!(result[1].label, "Terminal");
    assert_eq!(result[1].label_matches, (0..8).collect::<Vec<_>>());
}

#[test]
fn equal_name_scores_highlight_the_deepest_matching_label() {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/worktrees.json")).unwrap();
    snapshot.workspaces[1].label = "Codex".into();
    snapshot.workspaces[2].label = "Codex".into();
    let result = Search::default().rows(&snapshot, "codex", SearchMode::Flat);
    let tab = result
        .iter()
        .find(|row| row.target == Target::Tab("w2:t1".into()))
        .unwrap();
    assert!(
        tab.ancestors
            .iter()
            .all(|ancestor| ancestor.matches.is_empty())
    );
    assert_eq!(tab.label_matches, (0..5).collect::<Vec<_>>());
}

#[test]
fn single_root_tab_is_not_search_context_for_other_worktrees() {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/worktrees.json")).unwrap();
    snapshot.tabs.retain(|tab| tab.tab_id != "w1:t2");
    let result = Search::default().rows(&snapshot, "terminal", SearchMode::Hierarchical);
    assert_eq!(result[1].label, "Terminal");
    assert!(
        result
            .iter()
            .all(|row| row.target != Target::Tab("w2:t1".into()))
    );
    assert!(
        result
            .iter()
            .all(|row| row.target != Target::Tab("w5:t1".into()))
    );
}

#[test]
fn nested_rows_and_breadcrumbs_retain_destination_kinds() {
    let result = worktree_rows("herdrpp multimachine codex", SearchMode::Hierarchical);
    assert_eq!(
        result.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![RowKind::RootWorkspace, RowKind::Worktree, RowKind::Tab]
    );
    assert_eq!(
        result[2]
            .ancestors
            .iter()
            .map(|ancestor| ancestor.kind)
            .collect::<Vec<_>>(),
        vec![RowKind::RootWorkspace, RowKind::Worktree]
    );
    let flat = worktree_rows("herdrpp multimachine codex", SearchMode::Flat);
    assert_eq!(flat[1].kind, RowKind::Tab);
    assert_eq!(flat[1].ancestors[0].kind, RowKind::RootWorkspace);
    assert_eq!(flat[1].ancestors[1].kind, RowKind::Worktree);
}

#[test]
fn ungrouped_linked_worktree_uses_root_kind() {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/worktrees.json")).unwrap();
    snapshot
        .workspaces
        .retain(|workspace| workspace.workspace_id != "w1");
    snapshot.tabs.retain(|tab| tab.workspace_id != "w1");
    let result = Search::default().rows(&snapshot, "multimachine codex", SearchMode::Flat);
    assert_eq!(result[1].kind, RowKind::Tab);
    assert_eq!(result[1].ancestors.len(), 1);
    assert_eq!(result[1].ancestors[0].kind, RowKind::RootWorkspace);
}

#[test]
fn space_free_three_level_search_highlights_each_label() {
    let query = "herdrppmultimachinecodex";
    let result = worktree_rows(query, SearchMode::Flat);
    assert_eq!(ids(&result), vec![workspace("w2"), tab("w2:t1")]);
    assert_eq!(result[1].ancestors[0].matches, (0..7).collect::<Vec<_>>());
    assert_eq!(result[1].ancestors[1].matches, (0..12).collect::<Vec<_>>());
    assert_eq!(result[1].label_matches, (0..5).collect::<Vec<_>>());
    assert_eq!(
        ids(&worktree_rows(query, SearchMode::Hierarchical)),
        vec![workspace("w1"), workspace("w2"), tab("w2:t1")]
    );
}

#[test]
fn space_free_fuzzy_search_can_skip_an_intermediate_label() {
    assert_eq!(
        ids(&worktree_rows("hdrppmltmchn", SearchMode::Flat)),
        vec![workspace("w2"), tab("w2:t1"), tab("w2:t2")]
    );
    assert_eq!(
        ids(&worktree_rows("herdrppcodex", SearchMode::Flat)),
        vec![workspace("w2"), tab("w2:t1"), workspace("w5"), tab("w5:t1")]
    );
}

#[test]
fn space_free_search_cannot_reverse_labels_or_combine_siblings() {
    assert!(rows("codexpicker", SearchMode::Flat).is_empty());
    assert!(rows("servercodex", SearchMode::Hierarchical).is_empty());
    assert!(worktree_rows("multimachinejust", SearchMode::Flat).is_empty());
    assert!(rows("tcgcolorpicker", SearchMode::Flat).is_empty());
}

#[test]
fn space_free_search_maps_unicode_indices_to_each_label() {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    snapshot.workspaces[2].label = "日本Exact".into();
    snapshot
        .tabs
        .iter_mut()
        .find(|tab| tab.tab_id == "w3:t1")
        .unwrap()
        .label = "世界Terminal".into();
    let result = Search::default().rows(&snapshot, "exactterminal", SearchMode::Flat);
    assert_eq!(ids(&result), vec![workspace("w3"), tab("w3:t1")]);
    assert_eq!(result[0].label_matches, (2..7).collect::<Vec<_>>());
    assert_eq!(result[1].label_matches, (2..10).collect::<Vec<_>>());
}

#[test]
fn space_free_terms_keep_and_semantics_and_order_independence() {
    assert_eq!(
        ids(&rows("pickercod codex", SearchMode::Flat)),
        vec![workspace("w1"), tab("w1:t8")]
    );
    assert_eq!(
        ids(&rows("codex pickercod", SearchMode::Flat)),
        vec![workspace("w1"), tab("w1:t8")]
    );
    assert!(rows("pickercod nonexistent", SearchMode::Flat).is_empty());
}

#[test]
fn flat_context_owner_is_inserted_once_before_its_highest_ranked_tab() {
    let result = rows("pickercod", SearchMode::Flat);
    assert_eq!(
        ids(&result),
        vec![workspace("w1"), tab("w1:t8"), tab("w1:t4")]
    );
    assert_eq!(result[0].label_matches, (5..11).collect::<Vec<_>>());
    assert!(result[1].score > result[2].score);
}

#[test]
fn grouped_results_keep_tab_ui_order_even_when_flat_relevance_order_differs() {
    let mut snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    snapshot.tabs.reverse();
    let result = Search::default().results(&snapshot, "pickercod", SearchMode::Flat);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].workspace.target, workspace("w1"));
    assert_eq!(ids(&result[0].tabs), vec![tab("w1:t4"), tab("w1:t8")]);
    assert!(result[0].tabs[1].score > result[0].tabs[0].score);
}

#[test]
fn grouped_hierarchy_keeps_worktree_owners_and_single_tabs() {
    let snapshot = parse_snapshot(include_bytes!("fixtures/worktrees.json")).unwrap();
    let result = Search::default().results(&snapshot, "", SearchMode::Hierarchical);
    let owners: Vec<_> = result
        .iter()
        .map(|result| result.workspace.target.clone())
        .collect();
    assert_eq!(
        owners,
        vec![
            workspace("w1"),
            workspace("w2"),
            workspace("w3"),
            workspace("w4"),
            workspace("w5"),
            workspace("w0")
        ]
    );
    assert_eq!(ids(&result[1].tabs), vec![tab("w2:t1"), tab("w2:t2")]);
    assert_eq!(ids(&result[2].tabs), vec![tab("w3:t1")]);
}

#[test]
fn grouped_flat_results_rank_by_best_destination_with_stable_workspace_ties() {
    let snapshot = parse_snapshot(include_bytes!("fixtures/snapshot.json")).unwrap();
    let result = Search::default().results(&snapshot, "codex", SearchMode::Flat);
    let owners: Vec<_> = result
        .iter()
        .map(|result| result.workspace.target.clone())
        .collect();
    assert_eq!(
        owners,
        vec![workspace("w2"), workspace("w1"), workspace("w3")]
    );
    assert_eq!(ids(&result[0].tabs), vec![tab("w2:t7")]);
    assert_eq!(ids(&result[1].tabs), vec![tab("w1:t8")]);
    let empty = Search::default().results(&snapshot, "nonexistenttttt", SearchMode::Flat);
    assert!(empty.is_empty());
}
