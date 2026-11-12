#![cfg(unix)]

use std::collections::HashMap;
use std::error::Error;
use std::os::unix::fs::symlink;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use super::disk_probe_scope;
use super::fire_disk_updates;
use super::project_disk_sizes;
use super::schedule_disk_refresh;
use crate::channel;
use crate::config::WatcherConfig;
use crate::project::AbsolutePath;
use crate::scan::BackgroundMsg;
use crate::test_support;
use crate::watcher::ProjectEntry;
use crate::watcher::watch_test_support;

#[test]
fn target_symlink_artifacts_reach_explicit_project_size() -> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let project = AbsolutePath::from(scratch.path().join("project"));
    let artifacts = scratch.path().join("artifacts");
    std::fs::create_dir_all(&project)?;
    std::fs::create_dir_all(&artifacts)?;
    std::fs::write(project.join("Cargo.toml"), "[workspace]\n")?;
    std::fs::write(artifacts.join("output"), [1_u8; 4096])?;
    let target = AbsolutePath::from(project.join("target"));
    symlink(&artifacts, &target)?;
    let (sender, receiver) = channel::unbounded();
    let dispatch = watch_test_support::target_metadata(sender.clone(), project.clone(), target);
    let projects = HashMap::from([(
        project.clone(),
        ProjectEntry {
            label:          "linked-target".to_string(),
            abs_path:       project.clone(),
            repo_root:      None,
            git_dir:        None,
            common_git_dir: None,
        },
    )]);
    let now = Instant::now();
    let requested_at = now
        .checked_sub(Duration::from_secs(30))
        .ok_or("monotonic clock cannot represent the previous poll")?;
    let mut pending = HashMap::new();
    schedule_disk_refresh(
        &mut pending,
        "linked-target",
        requested_at,
        &WatcherConfig::default(),
    );
    let (done_sender, done_receiver) = mpsc::channel();
    fire_disk_updates(
        test_support::test_runtime().handle(),
        &Arc::new(tokio::sync::Semaphore::new(1)),
        &done_sender,
        &sender,
        &projects,
        &mut pending,
        &dispatch,
    );
    done_receiver.recv_timeout(Duration::from_millis(250))?;
    assert!(
        matches!(receiver.try_recv()?, BackgroundMsg::DiskUsageBatch { root_path, entries }
            if root_path == project && entries.len() == 1 && entries[0].0 == project
                && entries[0].1.total == 4108 && entries[0].1.in_project_target == 4096
                && entries[0].1.in_project_non_target == 12)
    );
    Ok(())
}

#[test]
fn target_symlink_to_in_tree_artifacts_is_counted_once() -> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let project = AbsolutePath::from(scratch.path());
    let artifacts = project.join("build-real");
    std::fs::create_dir(&artifacts)?;
    std::fs::write(artifacts.join("output"), [1_u8; 4096])?;
    let target = AbsolutePath::from(project.join("target"));
    symlink(&artifacts, &target)?;
    let (sender, receiver) = channel::unbounded();
    drop(receiver);
    let dispatch = watch_test_support::target_metadata(sender, project.clone(), target);
    assert_eq!(
        project_disk_sizes(&project, &disk_probe_scope(&dispatch, &project)).total,
        4096
    );
    Ok(())
}
