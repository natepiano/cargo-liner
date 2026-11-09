use std::collections::HashMap;
use std::fs;
use std::time::Instant;

use super::DiskProbeScope;
use super::WatchState;
use super::handle_disk_completion;
use super::project_disk_sizes;
use super::schedule_disk_refresh;
use crate::project::AbsolutePath;

#[test]
fn disk_event_bursts_keep_one_pending_entry_and_publish_the_final_size() -> std::io::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = AbsolutePath::from(directory.path());
    let file = directory.path().join("source.rs");
    fs::write(&file, "initial")?;
    let mut pending = HashMap::new();
    let now = Instant::now();
    for _ in 0..1_000 {
        schedule_disk_refresh(&mut pending, "project", now);
    }
    assert_eq!(pending.len(), 1);
    assert!(matches!(pending["project"], WatchState::Pending { .. }));
    pending.insert("project".to_string(), WatchState::Running);
    let before = project_disk_sizes(&root, &DiskProbeScope::ProjectTree).total;
    fs::write(&file, "final source after the event burst")?;
    for _ in 0..1_000 {
        schedule_disk_refresh(&mut pending, "project", now);
    }
    assert_eq!(pending.len(), 1);
    assert_eq!(pending["project"], WatchState::RunningDirty);
    handle_disk_completion(&mut pending, "project");
    assert!(matches!(pending["project"], WatchState::Pending { .. }));
    pending.insert("project".to_string(), WatchState::Running);
    let final_size = project_disk_sizes(&root, &DiskProbeScope::ProjectTree).total;
    assert_ne!(before, final_size);
    assert_eq!(final_size, fs::metadata(file)?.len());
    handle_disk_completion(&mut pending, "project");
    assert_eq!(pending["project"], WatchState::Idle);
    Ok(())
}
