use crate::search::matcher::{FieldMatcher, Query};

use crate::picker::{
    config::SearchMode,
    model::{AgentStatus, Snapshot, Tab, Target, Workspace},
    ordering,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    RootWorkspace,
    Worktree,
    Tab,
}

#[derive(Clone, Debug)]
pub struct Ancestor {
    pub kind: RowKind,
    pub label: String,
    pub matches: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub kind: RowKind,
    pub target: Target,
    pub workspace_id: String,
    pub label: String,
    pub ancestors: Vec<Ancestor>,
    pub depth: usize,
    pub label_matches: Vec<usize>,
    pub score: u32,
    pub matches_query: bool,
    pub active: bool,
    pub agent_status: AgentStatus,
}

impl Row {
    fn tab(snapshot: &Snapshot, tab: &Tab, ancestors: Vec<Ancestor>) -> Self {
        Self {
            kind: RowKind::Tab,
            agent_status: tab.agent_status,
            target: Target::Tab(tab.tab_id.clone()),
            workspace_id: tab.workspace_id.clone(),
            label: tab.label.clone(),
            depth: ancestors.len(),
            ancestors,
            label_matches: Vec::new(),
            score: 0,
            matches_query: false,
            active: snapshot.focused_tab_id.as_ref() == Some(&tab.tab_id),
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceResult {
    pub workspace: Row,
    pub tabs: Vec<Row>,
}

#[derive(Default)]
pub struct Search {
    matcher: FieldMatcher,
}

impl Search {
    pub fn results(
        &mut self,
        snapshot: &Snapshot,
        query: &str,
        mode: SearchMode,
    ) -> Vec<WorkspaceResult> {
        let rows = self.rows(snapshot, query, mode);
        let mut results = Vec::new();
        for workspace in ordering::workspaces(snapshot) {
            let Some(owner) = rows.iter().find(
                |row| matches!(&row.target, Target::Workspace(id) if id == &workspace.workspace_id),
            ) else {
                continue;
            };
            let mut tabs: Vec<_> = snapshot
                .tabs
                .iter()
                .filter(|tab| tab.workspace_id == workspace.workspace_id)
                .filter_map(|tab| {
                    rows.iter()
                        .find(|row| matches!(&row.target, Target::Tab(id) if id == &tab.tab_id))
                        .cloned()
                        .or_else(|| {
                            (tab.tab_id == workspace.active_tab_id).then(|| {
                                let mut ancestors = owner.ancestors.clone();
                                for ancestor in &mut ancestors {
                                    ancestor.matches.clear();
                                }
                                ancestors.push(Ancestor {
                                    kind: owner.kind,
                                    label: owner.label.clone(),
                                    matches: Vec::new(),
                                });
                                Row::tab(snapshot, tab, ancestors)
                            })
                        })
                        .map(|row| (tab.number, row))
                })
                .collect();
            tabs.sort_by_key(|(number, _)| *number);
            if !query.trim().is_empty() {
                tabs.sort_by_key(|(_, row)| std::cmp::Reverse((row.matches_query, row.score)));
            }
            results.push(WorkspaceResult {
                workspace: owner.clone(),
                tabs: tabs.into_iter().map(|(_, row)| row).collect(),
            });
        }
        if !query.trim().is_empty() {
            results.sort_by_key(|result| {
                std::cmp::Reverse(
                    result
                        .tabs
                        .iter()
                        .chain(std::iter::once(&result.workspace))
                        .filter(|row| row.matches_query)
                        .map(|row| row.score)
                        .max()
                        .unwrap_or(0),
                )
            });
        }
        results
    }

    fn match_row(&mut self, row: &mut Row, query: &Query) -> bool {
        let labels: Vec<_> = row
            .ancestors
            .iter()
            .map(|ancestor| ancestor.label.as_str())
            .chain(std::iter::once(row.label.as_str()))
            .collect();
        let Some(found) = self.matcher.matches(&labels, query) else {
            return false;
        };
        for (ancestor, indices) in row.ancestors.iter_mut().zip(&found.indices) {
            ancestor.matches.clone_from(indices);
        }
        row.label_matches.clone_from(&found.indices[row.depth]);
        row.score = found.score;
        row.matches_query = true;
        true
    }

    fn workspace_rows(
        &mut self,
        snapshot: &Snapshot,
        workspace: &Workspace,
        worktrees: &[&Workspace],
        ancestors: &[Ancestor],
        terms: &Query,
        mode: SearchMode,
    ) -> (Vec<Row>, Vec<Row>) {
        let tabs: Vec<_> = snapshot
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace.workspace_id)
            .collect();
        let kind = if ancestors.is_empty() {
            RowKind::RootWorkspace
        } else {
            RowKind::Worktree
        };
        let mut row = Row {
            kind,
            agent_status: AgentStatus::Unknown,
            target: Target::Workspace(workspace.workspace_id.clone()),
            workspace_id: workspace.workspace_id.clone(),
            label: workspace.label.clone(),
            ancestors: ancestors.to_vec(),
            depth: ancestors.len(),
            label_matches: Vec::new(),
            score: 0,
            matches_query: false,
            active: snapshot.focused_workspace_id.as_ref() == Some(&workspace.workspace_id),
        };
        let matched = self.match_row(&mut row, terms);
        let mut child_ancestors = ancestors.to_vec();
        child_ancestors.push(Ancestor {
            kind,
            label: workspace.label.clone(),
            matches: Vec::new(),
        });
        let mut children = Vec::new();
        for tab in tabs {
            let mut child = Row::tab(snapshot, tab, child_ancestors.clone());
            if self.match_row(&mut child, terms) {
                children.push(child);
            }
        }
        let mut context_owners = Vec::new();
        for worktree in worktrees {
            let (worktree_rows, owners) =
                self.workspace_rows(snapshot, worktree, &[], &child_ancestors, terms, mode);
            children.extend(worktree_rows);
            context_owners.extend(owners);
        }
        let mut rows = Vec::new();
        let owns_matching_tabs = children.iter().any(|child| {
            child.kind == RowKind::Tab && child.workspace_id == workspace.workspace_id
        });
        if matched || !children.is_empty() {
            if mode == SearchMode::Hierarchical || (!matched && owns_matching_tabs) {
                for child in &children {
                    for (ancestor, context) in row.ancestors.iter_mut().zip(&child.ancestors) {
                        ancestor.matches.extend(&context.matches);
                        ancestor.matches.sort_unstable();
                        ancestor.matches.dedup();
                    }
                    row.label_matches
                        .extend(&child.ancestors[row.depth].matches);
                }
                row.label_matches.sort_unstable();
                row.label_matches.dedup();
            }
            if matched || mode == SearchMode::Hierarchical {
                rows.push(row);
            } else if owns_matching_tabs {
                context_owners.push(row);
            }
        }
        rows.extend(children);
        (rows, context_owners)
    }

    pub fn rows(&mut self, snapshot: &Snapshot, query: &str, mode: SearchMode) -> Vec<Row> {
        let terms = Query::new(query);
        let mut rows = Vec::new();
        let mut context_owners = Vec::new();
        for group in ordering::groups(snapshot) {
            let (group_rows, owners) =
                self.workspace_rows(snapshot, group.root, &group.worktrees, &[], &terms, mode);
            rows.extend(group_rows);
            context_owners.extend(owners);
        }
        if mode == SearchMode::Flat && !terms.is_empty() {
            rows.sort_by_key(|r| std::cmp::Reverse(r.score));
        }
        for owner in context_owners {
            let index = rows
                .iter()
                .position(|row| row.kind == RowKind::Tab && row.workspace_id == owner.workspace_id)
                .expect("context owner has a matching tab");
            rows.insert(index, owner);
        }
        rows
    }
}
