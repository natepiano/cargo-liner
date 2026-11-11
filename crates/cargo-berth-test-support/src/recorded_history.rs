//! Copy recorded repositories and relocate paths in text and JSON records.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::CommandDeadlineExt as _;
use crate::DirectorySnapshot;
use crate::git_command;

/// Copy a repository and its linked checkouts, then return their path mapping.
///
/// # Panics
/// Panics when a repository, checkout, or saved record cannot be read or copied.
#[must_use]
pub fn copy_recorded_repository(
    executable: &str,
    root: &Path,
    source: &Path,
) -> Vec<(String, String)> {
    let source_spelling = source.to_string_lossy().into_owned();
    let source = fs::canonicalize(source).expect("history source should resolve");
    let listed = git_command(executable)
        .current_dir(&source)
        .args(["worktree", "list", "--porcelain", "-z"])
        .bounded_output()
        .expect("history worktrees should list");
    assert!(listed.status.success(), "{listed:?}");
    let listed = String::from_utf8(listed.stdout).expect("fixture paths should be UTF-8");
    let mut relocations = Vec::new();
    let mut linked = Vec::new();
    for path in listed
        .split('\0')
        .filter_map(|line| line.strip_prefix("worktree "))
    {
        let path = Path::new(path);
        if !path.exists() {
            continue;
        }
        let path = fs::canonicalize(path).expect("history worktree should resolve");
        if path.starts_with(&source) {
            continue;
        }
        let destination = root.join(".git/fixture-worktrees").join(
            path.file_name()
                .expect("linked checkout should have a name"),
        );
        assert!(
            linked.iter().all(|(path, _)| path != &destination),
            "linked checkout names should differ"
        );
        linked.push((destination.clone(), DirectorySnapshot::capture(&path)));
        relocations.push((
            path.to_string_lossy().into_owned(),
            destination.to_string_lossy().into_owned(),
        ));
    }
    relocations.push((
        source.to_string_lossy().into_owned(),
        root.to_string_lossy().into_owned(),
    ));
    if source_spelling != source.to_string_lossy() {
        relocations.push((source_spelling, root.to_string_lossy().into_owned()));
    }
    DirectorySnapshot::capture(&source).restore(root);
    for (path, snapshot) in linked {
        fs::create_dir_all(&path).expect("history worktree should have a parent");
        snapshot.restore(&path);
    }
    relocations
}

/// Relocate paths in text files, JSON values, and journal records.
///
/// # Panics
/// Panics when a file cannot be read, decoded, or written.
pub fn relocate_tree(root: &Path, relocations: &[(String, String)]) {
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).expect("history directory should read") {
            let entry = entry.expect("history entry should read");
            let path = entry.path();
            if entry
                .file_type()
                .expect("history entry type should read")
                .is_dir()
            {
                directories.push(path);
                continue;
            }
            let bytes = fs::read(&path).expect("history file should read");
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            let relocated = match path.extension().and_then(|value| value.to_str()) {
                Some("json") => {
                    let mut value: Value =
                        serde_json::from_str(&text).expect("history JSON should decode");
                    relocate_json(&mut value, relocations);
                    serde_json::to_string(&value).expect("history JSON should encode")
                },
                Some("ndjson") => {
                    let mut journal = String::new();
                    for line in text.lines() {
                        let mut value: Value =
                            serde_json::from_str(line).expect("history event should decode");
                        relocate_json(&mut value, relocations);
                        journal.push_str(
                            &serde_json::to_string(&value).expect("history event should encode"),
                        );
                        journal.push('\n');
                    }
                    journal
                },
                _ => relocations
                    .iter()
                    .fold(text.clone(), |text, (old, new)| text.replace(old, new)),
            };
            if relocated != text {
                fs::write(path, relocated).expect("relocated history should write");
            }
        }
    }
}

/// Relocate and save the witnesses for a recorded operation.
///
/// # Panics
/// Panics when the witnesses cannot be encoded or written.
pub fn write_recorded_state(
    root: &Path,
    state_path: &str,
    mut state: Value,
    relocations: &[(String, String)],
) {
    canonicalize_state_paths(&mut state);
    relocate_json(&mut state, relocations);
    fs::write(
        root.join(state_path),
        serde_json::to_vec(&state).expect("history state should encode"),
    )
    .expect("history state should write");
}

fn relocate_json(value: &mut Value, relocations: &[(String, String)]) {
    match value {
        Value::String(text) => {
            for (old, new) in relocations {
                *text = text.replace(old, new);
            }
        },
        Value::Array(values) => {
            for value in values {
                relocate_json(value, relocations);
            }
        },
        Value::Object(values) => {
            for value in values.values_mut() {
                relocate_json(value, relocations);
            }
        },
        Value::Null | Value::Bool(_) | Value::Number(_) => {},
    }
}

fn canonicalize_state_paths(value: &mut Value) {
    match value {
        Value::String(text) if Path::new(text).is_absolute() => {
            if let Ok(path) = fs::canonicalize(&*text) {
                *text = path.to_string_lossy().into_owned();
            }
        },
        Value::Array(values) => {
            for value in values {
                canonicalize_state_paths(value);
            }
        },
        Value::Object(fields) => {
            for value in fields.values_mut() {
                canonicalize_state_paths(value);
            }
        },
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {},
    }
}
