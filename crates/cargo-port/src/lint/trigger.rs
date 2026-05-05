use std::path::Path;
use std::time::Duration;

#[cfg(test)]
use notify::Event;
use notify::event::EventKind;

use crate::project::AbsolutePath;

const LINT_DEBOUNCE: Duration = Duration::from_millis(750);
const DELETE_LINT_DEBOUNCE: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LintTriggerKind {
    Manifest,
    Lockfile,
    RustSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LintEventKind {
    CreateOrModify,
    Remove,
    OtherRelevant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintTriggerEvent {
    pub project_root: AbsolutePath,
    pub trigger:      LintTriggerKind,
    pub event_kind:   LintEventKind,
    pub removal:      bool,
}

impl LintTriggerEvent {
    pub const fn debounce(&self) -> Duration {
        if self.removal {
            DELETE_LINT_DEBOUNCE
        } else {
            LINT_DEBOUNCE
        }
    }
}

/// Kind of trigger for a `cargo metadata` refresh. Driven by the same
/// watcher events that drive lint runs; callers dispatch a refresh on any
/// match. See `docs/cargo_metadata.md` → **In-flight race handling** for
/// why the fingerprint — rather than the kind — decides whether a pending
/// spawn is still relevant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CargoMetadataTriggerKind {
    Manifest,
    Lockfile,
    Toolchain,
    CargoConfig,
}

/// Does `path` (under `project_root`) warrant a `cargo metadata` refresh?
///
/// Hits:
/// - `<any>/Cargo.toml`
/// - `<any>/Cargo.lock`
/// - `<any>/rust-toolchain` or `<any>/rust-toolchain.toml`
/// - `<any>/.cargo/config` or `<any>/.cargo/config.toml`
///
/// Skips events under `target/` and `.git/` directories. The
/// `path.starts_with(project_root)` gate ensures out-of-tree hits do not
/// leak in through the shared recursive watch — the ancestor
/// `.cargo/config` case that lives *above* the project is handled
/// separately by the ancestor watch-set subsystem.
pub fn classify_cargo_metadata_event_path(
    project_root: &Path,
    path: &Path,
) -> Option<CargoMetadataTriggerKind> {
    if !path.starts_with(project_root) {
        return None;
    }
    if path.components().any(|component| {
        let part = component.as_os_str();
        part == "target" || part == ".git"
    }) {
        return None;
    }
    classify_cargo_metadata_basename(path)
}

/// Basename-only variant used by the ancestor `.cargo/` watch-set path,
/// where the `starts_with(project_root)` gate does not apply.
pub fn classify_cargo_metadata_basename(path: &Path) -> Option<CargoMetadataTriggerKind> {
    let file_name = path.file_name().and_then(|name| name.to_str())?;
    match file_name {
        "Cargo.toml" => Some(CargoMetadataTriggerKind::Manifest),
        "Cargo.lock" => Some(CargoMetadataTriggerKind::Lockfile),
        "rust-toolchain" | "rust-toolchain.toml" => Some(CargoMetadataTriggerKind::Toolchain),
        "config" | "config.toml" => {
            let parent_is_dot_cargo = path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == ".cargo");
            parent_is_dot_cargo.then_some(CargoMetadataTriggerKind::CargoConfig)
        },
        _ => None,
    }
}

#[cfg(test)]
pub fn classify_event(project_root: &Path, event: &Event) -> Option<LintTriggerEvent> {
    event
        .paths
        .iter()
        .find_map(|path| classify_event_path(project_root, event.kind, path))
}

pub fn classify_event_path(
    project_root: &Path,
    event_kind: EventKind,
    path: &Path,
) -> Option<LintTriggerEvent> {
    if !path.starts_with(project_root) {
        return None;
    }
    if path.components().any(|component| {
        let part = component.as_os_str();
        part == "target" || part == ".git"
    }) {
        return None;
    }

    let file_name = path.file_name().and_then(|name| name.to_str())?;
    let trigger = if file_name == "Cargo.toml" {
        LintTriggerKind::Manifest
    } else if file_name == "Cargo.lock" {
        LintTriggerKind::Lockfile
    } else if path.extension().is_some_and(|ext| ext == "rs") {
        LintTriggerKind::RustSource
    } else {
        return None;
    };

    let removal = matches!(event_kind, EventKind::Remove(_));
    let event_kind = if removal {
        LintEventKind::Remove
    } else if matches!(event_kind, EventKind::Create(_) | EventKind::Modify(_)) {
        LintEventKind::CreateOrModify
    } else {
        LintEventKind::OtherRelevant
    };

    Some(LintTriggerEvent {
        project_root: AbsolutePath::from(project_root),
        trigger,
        event_kind,
        removal,
    })
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use notify::event::DataChange;
    use notify::event::ModifyKind;
    use notify::event::RemoveKind;

    use super::*;

    #[test]
    fn relevant_changes_ignore_git_and_target_paths() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        let modify_kind = EventKind::Modify(ModifyKind::Data(DataChange::Any));

        assert_eq!(
            classify_event_path(
                project_dir.path(),
                modify_kind,
                &project_dir.path().join("src/main.rs")
            )
            .expect("src main trigger")
            .trigger,
            LintTriggerKind::RustSource
        );
        assert_eq!(
            classify_event_path(
                project_dir.path(),
                modify_kind,
                &project_dir.path().join("Cargo.toml")
            )
            .expect("manifest trigger")
            .trigger,
            LintTriggerKind::Manifest
        );
        assert!(
            classify_event_path(
                project_dir.path(),
                modify_kind,
                &project_dir.path().join("target/debug/app")
            )
            .is_none()
        );
        assert!(
            classify_event_path(
                project_dir.path(),
                modify_kind,
                &project_dir.path().join(".git/index")
            )
            .is_none()
        );
    }

    #[test]
    fn cargo_metadata_basename_classifier_matches_irrespective_of_project_root() {
        // The basename-only variant is how the (future) ancestor `.cargo/`
        // watch-set subsystem will classify events that live *above* any
        // registered project, where the `starts_with(project_root)` gate
        // on `classify_cargo_metadata_event_path` would filter them out.
        use std::path::Path;

        let hits = [
            (
                Path::new("/home/user/.cargo/config.toml"),
                CargoMetadataTriggerKind::CargoConfig,
            ),
            (
                Path::new("/home/user/.cargo/config"),
                CargoMetadataTriggerKind::CargoConfig,
            ),
            (
                Path::new("/opt/proj/Cargo.toml"),
                CargoMetadataTriggerKind::Manifest,
            ),
            (
                Path::new("/opt/proj/Cargo.lock"),
                CargoMetadataTriggerKind::Lockfile,
            ),
            (
                Path::new("/opt/proj/rust-toolchain"),
                CargoMetadataTriggerKind::Toolchain,
            ),
            (
                Path::new("/opt/proj/rust-toolchain.toml"),
                CargoMetadataTriggerKind::Toolchain,
            ),
        ];
        for (path, expected) in hits {
            assert_eq!(
                classify_cargo_metadata_basename(path),
                Some(expected),
                "expected basename hit for {}",
                path.display()
            );
        }

        // `config.toml` without a `.cargo` parent is a miss — otherwise
        // any ambient TOML file would trigger a refresh.
        let misses = [
            Path::new("/home/user/some/config.toml"),
            Path::new("/etc/config"),
            Path::new("/home/user/Cargo.toml.bak"),
        ];
        for path in misses {
            assert_eq!(
                classify_cargo_metadata_basename(path),
                None,
                "unexpected basename hit for {}",
                path.display()
            );
        }
    }

    #[test]
    fn cargo_metadata_classifier_hits_manifest_lock_toolchain_and_cargo_config() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        let root = project_dir.path();

        let hits = [
            (root.join("Cargo.toml"), CargoMetadataTriggerKind::Manifest),
            (root.join("Cargo.lock"), CargoMetadataTriggerKind::Lockfile),
            (
                root.join("rust-toolchain.toml"),
                CargoMetadataTriggerKind::Toolchain,
            ),
            (
                root.join("rust-toolchain"),
                CargoMetadataTriggerKind::Toolchain,
            ),
            (
                root.join(".cargo/config.toml"),
                CargoMetadataTriggerKind::CargoConfig,
            ),
            (
                root.join(".cargo/config"),
                CargoMetadataTriggerKind::CargoConfig,
            ),
            (
                root.join("nested/member/Cargo.toml"),
                CargoMetadataTriggerKind::Manifest,
            ),
        ];
        for (path, expected) in hits {
            assert_eq!(
                classify_cargo_metadata_event_path(root, &path),
                Some(expected),
                "expected metadata trigger for {}",
                path.display()
            );
        }

        let misses = [
            root.join("src/main.rs"),
            root.join("README.md"),
            root.join("Cargo.toml.bak"),
            root.join("target/debug/build.lock"),
            root.join(".git/config"),
            // `config.toml` *not* under a `.cargo/` parent must miss.
            root.join("docs/config.toml"),
        ];
        for path in &misses {
            assert_eq!(
                classify_cargo_metadata_event_path(root, path),
                None,
                "unexpected metadata trigger for {}",
                path.display()
            );
        }
    }

    #[test]
    fn remove_events_use_longer_debounce() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        let source_path = project_dir.path().join("src/lib.rs");
        let remove_event = Event {
            kind:  EventKind::Remove(RemoveKind::File),
            paths: vec![source_path.clone()],
            attrs: notify::event::EventAttributes::default(),
        };
        let modify_event = Event {
            kind:  EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            paths: vec![source_path],
            attrs: notify::event::EventAttributes::default(),
        };

        assert_eq!(
            classify_event(project_dir.path(), &remove_event)
                .expect("remove trigger")
                .debounce(),
            DELETE_LINT_DEBOUNCE
        );
        assert_eq!(
            classify_event(project_dir.path(), &modify_event)
                .expect("modify trigger")
                .debounce(),
            LINT_DEBOUNCE
        );
    }
}
