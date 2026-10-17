use herdrpp::search::matcher::{FieldMatcher, Query};

#[test]
fn terms_match_across_fields_and_highlight_their_original_characters() {
    let found = FieldMatcher::default()
        .matches(
            &["herdrpp", "Refresh workspace PR status"],
            &Query::new("HERD refresh"),
        )
        .unwrap();
    assert_eq!(found.indices[0], vec![0, 1, 2, 3]);
    assert_eq!(found.indices[1], vec![0, 1, 2, 3, 4, 5, 6]);
    assert!(found.score > 0);
}

#[test]
fn every_term_is_required() {
    assert!(
        FieldMatcher::default()
            .matches(&["Refresh"], &Query::new("refresh nonexistent"))
            .is_none()
    );
}

#[test]
fn whitespace_matches_every_field_without_highlights() {
    let found = FieldMatcher::default()
        .matches(&["世界", "Codex"], &Query::new(" \t"))
        .unwrap();
    assert_eq!(found.score, 0);
    assert_eq!(found.indices, vec![Vec::<usize>::new(), Vec::new()]);
}

#[test]
fn joined_field_fallback_preserves_unicode_character_indices() {
    let found = FieldMatcher::default()
        .matches(&["世界", "Codex"], &Query::new("界Co"))
        .unwrap();
    assert_eq!(found.indices, vec![vec![1], vec![0, 1]]);
}

#[test]
fn equal_field_scores_prefer_the_last_field_and_deduplicate_indices() {
    let found = FieldMatcher::default()
        .matches(&["Codex", "Codex"], &Query::new("cod cod"))
        .unwrap();
    assert_eq!(found.indices, vec![vec![], vec![0, 1, 2]]);
}

#[test]
fn smart_normalization_matches_accented_labels() {
    let found = FieldMatcher::default()
        .matches(&["Café"], &Query::new("cafe"))
        .unwrap();
    assert_eq!(found.indices, vec![vec![0, 1, 2, 3]]);
}
