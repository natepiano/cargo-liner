use std::path::Path;
use std::time::Duration;

use notify::event::EventKind;
#[cfg(test)]
use notify::Event;

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
    use super::*;

    #[test]
    fn relevant_changes_ignore_git_and_target_paths() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        let modify_kind = EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any,
        ));

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
    fn remove_events_use_longer_debounce() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        let source_path = project_dir.path().join("src/lib.rs");
        let remove_event = Event {
            kind:  EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![source_path.clone()],
            attrs: notify::event::EventAttributes::default(),
        };
        let modify_event = Event {
            kind:  EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
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
