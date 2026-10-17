//! Public snapshots have no MRU history. Keep a deterministic, explicit fallback.
use crate::picker::model::{Snapshot, Workspace, WorkspaceGroup};

pub fn groups(snapshot: &Snapshot) -> Vec<WorkspaceGroup<'_>> {
    let mut groups = snapshot.workspace_groups();
    groups.sort_by_key(|group| {
        let focused = std::iter::once(group.root)
            .chain(group.worktrees.iter().copied())
            .any(|workspace| {
                Some(&workspace.workspace_id) == snapshot.focused_workspace_id.as_ref()
            });
        (!focused, group.root.number)
    });
    groups
}

pub fn workspaces(snapshot: &Snapshot) -> Vec<&Workspace> {
    groups(snapshot)
        .into_iter()
        .flat_map(|group| std::iter::once(group.root).chain(group.worktrees))
        .collect()
}

pub fn initial_target(snapshot: &Snapshot) -> Option<&str> {
    let ordered = workspaces(snapshot);
    ordered
        .iter()
        .find(|w| Some(&w.workspace_id) != snapshot.focused_workspace_id.as_ref())
        .or_else(|| ordered.first())
        .map(|w| w.workspace_id.as_str())
}
