use super::AbsolutePath;
use super::CARGO_TOML;
use super::Component;
use super::HashSet;
use super::Path;
use super::RootItem;
use super::Table;
use super::Value;

pub(super) fn workspace_member_paths_new(
    ws_path: &Path,
    items: &[RootItem],
) -> HashSet<AbsolutePath> {
    let manifest = ws_path.join(CARGO_TOML);
    let Some((members, excludes)) = workspace_member_patterns(&manifest) else {
        return items
            .iter()
            .filter(|item| item.path().starts_with(ws_path) && item.path() != ws_path)
            .map(|item| item.path().clone())
            .collect();
    };

    items
        .iter()
        .filter(|item| item.path().starts_with(ws_path) && item.path() != ws_path)
        .filter_map(|item| {
            item.path().strip_prefix(ws_path).ok().and_then(|relative| {
                let relative_str = normalize_workspace_path(relative);
                let included = members
                    .iter()
                    .any(|pattern| workspace_pattern_matches(pattern, &relative_str));
                let is_excluded = excludes
                    .iter()
                    .any(|pattern| workspace_pattern_matches(pattern, &relative_str));
                if included && !is_excluded {
                    Some(item.path().clone())
                } else {
                    None
                }
            })
        })
        .collect()
}

fn workspace_member_patterns(manifest_path: &Path) -> Option<(Vec<String>, Vec<String>)> {
    let contents = std::fs::read_to_string(manifest_path).ok()?;
    let table: Table = contents.parse().ok()?;
    let workspace = table.get("workspace")?.as_table()?;

    let members = workspace
        .get("members")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(std::string::ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    let excludes = workspace
        .get("exclude")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(std::string::ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    Some((members, excludes))
}

pub(crate) fn normalize_workspace_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(segment) => Some(segment.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn workspace_pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let path_segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    workspace_pattern_matches_segments(&pattern_segments, &path_segments)
}

fn workspace_pattern_matches_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            workspace_pattern_matches_segments(rest, path)
                || (!path.is_empty() && workspace_pattern_matches_segments(pattern, &path[1..]))
        },
        Some((segment, rest)) => {
            !path.is_empty()
                && workspace_pattern_matches_segment(segment, path[0])
                && workspace_pattern_matches_segments(rest, &path[1..])
        },
    }
}

fn workspace_pattern_matches_segment(pattern: &str, value: &str) -> bool {
    fn matches(pattern: &[u8], value: &[u8]) -> bool {
        match pattern.split_first() {
            None => value.is_empty(),
            Some((b'*', rest)) => {
                matches(rest, value) || (!value.is_empty() && matches(pattern, &value[1..]))
            },
            Some((b'?', rest)) => !value.is_empty() && matches(rest, &value[1..]),
            Some((head, rest)) => {
                !value.is_empty() && *head == value[0] && matches(rest, &value[1..])
            },
        }
    }

    matches(pattern.as_bytes(), value.as_bytes())
}
