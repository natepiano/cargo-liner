use std::path::Path;

use crate::project::AbsolutePath;
use crate::scan;
use crate::tui::app::App;
use crate::tui::project_list::ExpandTarget;

fn last_selected_path_file() -> AbsolutePath { scan::cache_dir().join("last_selected.txt").into() }

/// Read the pre-`tree_state.toml` selection file. Retained only to migrate the
/// last selection forward on the first launch after the upgrade.
fn load_last_selected() -> Option<AbsolutePath> {
    let path = last_selected_path_file();
    let raw = std::fs::read_to_string(&*path).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty() && Path::new(trimmed).is_absolute()).then(|| AbsolutePath::from(trimmed))
}

fn tree_state_file() -> AbsolutePath { scan::cache_dir().join("tree_state.toml").into() }

/// On-disk form of the project-tree UI state: the selected project and the set
/// of expanded containers. Each expanded entry is a tab-delimited token
/// (`kind\tpath[\tgroup]`) — see [`encode_expand_target`].
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct TreeStateFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected: Option<String>,
    #[serde(default)]
    expanded: Vec<String>,
}

/// Load the persisted selection and expansion targets. Falls back to the legacy
/// `last_selected.txt` for the selection when `tree_state.toml` is absent (the
/// first launch after the upgrade), so the cursor still lands where it did.
pub(super) fn load_tree_state() -> (Option<AbsolutePath>, Vec<ExpandTarget>) {
    let Ok(text) = std::fs::read_to_string(&*tree_state_file()) else {
        return (load_last_selected(), Vec::new());
    };
    let file: TreeStateFile = toml::from_str(&text).unwrap_or_default();
    let selected = file
        .selected
        .as_deref()
        .filter(|raw| Path::new(raw).is_absolute())
        .map(AbsolutePath::from);
    let expanded = file
        .expanded
        .iter()
        .filter_map(|token| decode_expand_target(token))
        .collect();
    (selected, expanded)
}

/// Write the current selection and expanded containers to `tree_state.toml`.
pub(super) fn save_tree_state(app: &App) {
    let file = TreeStateFile {
        selected: app
            .project_list
            .last_selected_path()
            .map(ToString::to_string),
        expanded: app
            .project_list
            .export_expanded()
            .iter()
            .map(encode_expand_target)
            .collect(),
    };
    if let Ok(text) = toml::to_string(&file) {
        let _ = std::fs::write(tree_state_file(), text);
    }
}

/// Tab-delimited token for one expanded container. The leading kind tag keeps a
/// worktree group's `Worktree` entry distinct from the `Root` at the same path.
pub(super) fn encode_expand_target(target: &ExpandTarget) -> String {
    match target {
        ExpandTarget::Root(path) => format!("root\t{path}"),
        ExpandTarget::Group(path, group) => format!("group\t{path}\t{group}"),
        ExpandTarget::Worktree(path) => format!("worktree\t{path}"),
        ExpandTarget::WorktreeGroup(path, group) => format!("worktreegroup\t{path}\t{group}"),
    }
}

pub(super) fn decode_expand_target(token: &str) -> Option<ExpandTarget> {
    let mut parts = token.split('\t');
    let kind = parts.next()?;
    let raw = parts.next()?;
    if !Path::new(raw).is_absolute() {
        return None;
    }
    let path = AbsolutePath::from(raw);
    match kind {
        "root" => Some(ExpandTarget::Root(path)),
        "worktree" => Some(ExpandTarget::Worktree(path)),
        "group" => Some(ExpandTarget::Group(path, parts.next()?.to_string())),
        "worktreegroup" => Some(ExpandTarget::WorktreeGroup(path, parts.next()?.to_string())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_target_token_round_trips_every_variant() {
        let targets = [
            ExpandTarget::Root(AbsolutePath::from("/proj")),
            ExpandTarget::Group(AbsolutePath::from("/proj"), "examples".to_string()),
            ExpandTarget::Worktree(AbsolutePath::from("/proj-wt")),
            ExpandTarget::WorktreeGroup(AbsolutePath::from("/proj-wt"), "benches".to_string()),
        ];
        for target in targets {
            let token = encode_expand_target(&target);
            assert_eq!(decode_expand_target(&token), Some(target));
        }
    }
}
