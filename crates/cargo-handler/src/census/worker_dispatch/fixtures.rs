//! Staff documents and claim files for dispatch tests.

#![expect(clippy::expect_used, reason = "fixture documents must parse and write")]

use serde_json::Value;

use super::*;
use crate::census::HierarchySource;
use crate::census::hierarchy::StaffFileReader;

pub(in crate::census) struct FixtureWorkers {
    pub(super) workers: StaffWorkers,
}

impl FixtureWorkers {
    pub(super) fn read(directory: &Path) -> Self {
        let mut names = fixture_names(directory);
        Self {
            workers: names.staff_workers(directory).clone(),
        }
    }

    pub(super) fn parse(text: &str) -> Self {
        let directory = tempfile::tempdir().expect("staff fixture directory");
        let entries = serde_json::from_str(text).expect("worker fixture entries");
        write_worker_fixture(directory.path(), &entries);
        Self::read(directory.path())
    }
}

pub(in crate::census) fn fixture_names(directory: &Path) -> WorkerNames {
    let source = HierarchySource::File(directory.join("staff.json").to_string_lossy().into_owned());
    let mut names = WorkerNames::default();
    names.set_staff(StaffFileReader::default().roster(&source));
    names
}

/// Each test entry supplies one staff row and an optional launcher claim.
pub(in crate::census) fn write_worker_fixture(directory: &Path, entries: &Value) {
    fs::write(
        directory.join("progress_history_state.json"),
        serde_json::json!({"owner_staff_id":"unit"}).to_string(),
    )
    .expect("unit owner");
    let mut rows = vec![
        serde_json::json!({"id":"boss", "kind":"showrunner", "name":"boss",
            "parent":null, "current_session":"boss-session"}),
        serde_json::json!({"id":"unit", "kind":"unit", "name":"unit",
            "parent":"boss", "current_session":"director-id", "run":directory}),
    ];
    for (name, entry) in entries.as_object().expect("worker entries") {
        let mut row = serde_json::json!({"id":name, "kind":"worker", "name":name,
            "parent":"unit", "current_session":entry["current_session"]});
        if let Some(role) = entry.get("role") {
            row["role"] = role.clone();
        }
        rows.push(row);
        if entry.get("launcher_pid").is_some() {
            fs::write(
                directory.join(format!("impl_claim_{name}.json")),
                serde_json::json!({"worker":name,"pid":entry["launcher_pid"]}).to_string(),
            )
            .expect("launcher claim");
        }
    }
    fs::write(
        directory.join("staff.json"),
        serde_json::json!({"schema":2,"rows":rows}).to_string(),
    )
    .expect("staff document");
}
