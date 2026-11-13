use std::collections::HashMap;
use std::fs;
use std::time::Duration;
use std::time::Instant;

use tempfile::TempDir;
use tokio::runtime::Runtime;

use super::DiskUpdateInputs;
use super::WatchState;
use super::handle_disk_completion;
use super::schedule_disk_poll_with_config;
use super::schedule_disk_refresh;
use crate::channel;
use crate::config::WatcherConfig;
use crate::project::AbsolutePath;
use crate::scan::BackgroundMsg;
use crate::scan::DiskUsageTree;
use crate::watcher::ProjectEntry;
use crate::watcher::events;
use crate::watcher::events::EventContext;
use crate::watcher::runtime::GitRefreshDemand;
use crate::watcher::watch_test_support;

#[test]
fn idle_disk_ticks_do_not_schedule_existing_project_walks() -> std::io::Result<()> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("Cargo.toml"), "[workspace]\n")?;
    let root = AbsolutePath::from(directory.path());
    let projects = HashMap::from([(
        root.clone(),
        ProjectEntry {
            git_event_visibility: std::sync::Mutex::new(
                crate::project::GitEventVisibilityCache::default(),
            ),
            git_refresh_demand:   GitRefreshDemand::default(),
            label:                "idle-project".to_string(),
            abs_path:             root,
            repo_root:            None,
            git_dir:              None,
            common_git_dir:       None,
        },
    )]);
    let started = Instant::now();
    let mut last_poll = started;
    let mut pending = HashMap::new();
    for minute in 1..=10 {
        schedule_disk_poll_with_config(
            started + Duration::from_secs(minute * 60),
            &mut last_poll,
            &projects,
            &mut pending,
            &WatcherConfig::default(),
        );
        assert!(
            pending.is_empty(),
            "idle minute {minute} scheduled a tree walk"
        );
    }
    fs::remove_dir_all(directory.path())?;
    schedule_disk_poll_with_config(
        started + Duration::from_mins(11),
        &mut last_poll,
        &projects,
        &mut pending,
        &WatcherConfig::default(),
    );
    assert!(matches!(
        pending.get("idle-project"),
        Some(WatchState::Pending { .. })
    ));
    Ok(())
}

#[test]
fn disk_event_bursts_keep_one_pending_entry_and_publish_the_final_size() -> std::io::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = AbsolutePath::from(directory.path());
    let file = directory.path().join("source.rs");
    fs::write(&file, "initial")?;
    for name in ["unchanged-first", "unchanged-second"] {
        fs::write(root.join(name), "")?;
    }
    let (sender, receiver) = channel::unbounded();
    drop(receiver);
    let dispatch = watch_test_support::target_metadata(
        sender,
        root.clone(),
        AbsolutePath::from(root.join("target")),
    );
    let mut pending = HashMap::new();
    let now = Instant::now();
    for _ in 0..1_000 {
        schedule_disk_refresh(&mut pending, "project", now, &WatcherConfig::default());
    }
    assert_eq!(pending.len(), 1);
    assert!(matches!(pending["project"], WatchState::Pending { .. }));
    pending.insert("project".to_string(), WatchState::Running);
    let before = DiskUpdateInputs::new(&dispatch, &root).measure().entries[0]
        .1
        .total;
    fs::write(&file, "final source after the event burst")?;
    let projects = HashMap::from([(
        root.clone(),
        ProjectEntry {
            git_event_visibility: std::sync::Mutex::new(
                crate::project::GitEventVisibilityCache::default(),
            ),
            git_refresh_demand:   GitRefreshDemand::default(),
            label:                "project".to_string(),
            abs_path:             root.clone(),
            repo_root:            None,
            git_dir:              None,
            common_git_dir:       None,
        },
    )]);
    let context = EventContext {
        watch_roots:     &[],
        projects:        &projects,
        project_parents: &std::collections::HashSet::new(),
        discovered:      &std::collections::HashSet::new(),
    };
    events::handle_notify_event(
        &file,
        None,
        &context,
        &dispatch.sender,
        None,
        Some(&dispatch),
        &mut pending,
        &mut HashMap::new(),
        &mut HashMap::new(),
        &WatcherConfig::default(),
    );
    for _ in 0..1_000 {
        schedule_disk_refresh(&mut pending, "project", now, &WatcherConfig::default());
    }
    assert_eq!(pending.len(), 1);
    assert_eq!(pending["project"], WatchState::RunningDirty);
    handle_disk_completion(&mut pending, "project", &WatcherConfig::default());
    assert!(matches!(pending["project"], WatchState::Pending { .. }));
    pending.insert("project".to_string(), WatchState::Running);
    let final_update = DiskUpdateInputs::new(&dispatch, &root).measure();
    assert!(
        final_update.aggregation_entries_examined <= 2,
        "a file change rebuilt totals from unchanged file records"
    );
    let final_size = final_update.entries[0].1.total;
    assert_ne!(before, final_size);
    assert_eq!(final_size, fs::metadata(file)?.len());
    handle_disk_completion(&mut pending, "project", &WatcherConfig::default());
    assert_eq!(pending["project"], WatchState::Idle);
    Ok(())
}

/// `DiskWorkerFixture` drains blocking workers before it removes their directory.
struct DiskWorkerFixture {
    runtime:   Runtime,
    directory: TempDir,
}

impl DiskWorkerFixture {
    fn new() -> std::io::Result<Self> {
        Ok(Self {
            runtime:   tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()?,
            directory: tempfile::tempdir()?,
        })
    }
}

#[test]
fn shared_disk_changes_publish_every_project() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = DiskWorkerFixture::new()?;
    let root = AbsolutePath::from(fixture.directory.path());
    let child = AbsolutePath::from(root.join("child"));
    fs::create_dir(&child)?;
    let file = child.join("log");
    fs::write(&file, "old")?;
    let (sender, receiver) = channel::unbounded();
    let dispatch = watch_test_support::target_metadata(
        sender.clone(),
        root.clone(),
        AbsolutePath::from(root.join("target")),
    );
    let tracker = dispatch
        .metadata_store
        .lock()
        .map_err(|error| error.to_string())?
        .disk_usage
        .for_tree(DiskUsageTree {
            root_abs_path: root.clone(),
            entries:       vec![root.clone(), child.clone()],
        });
    assert!(
        tracker
            .refresh()
            .entries
            .iter()
            .all(|(_, sizes)| sizes.total == 3)
    );
    fs::write(&file, "changed child log")?;
    let projects = [root.clone(), child.clone()]
        .into_iter()
        .map(|path| {
            (
                path.clone(),
                ProjectEntry {
                    git_event_visibility: std::sync::Mutex::new(
                        crate::project::GitEventVisibilityCache::default(),
                    ),
                    git_refresh_demand:   GitRefreshDemand::default(),
                    label:                path.display().to_string(),
                    abs_path:             path,
                    repo_root:            None,
                    git_dir:              None,
                    common_git_dir:       None,
                },
            )
        })
        .collect();
    let context = EventContext {
        watch_roots:     &[],
        projects:        &projects,
        project_parents: &std::collections::HashSet::new(),
        discovered:      &std::collections::HashSet::new(),
    };
    let mut pending = HashMap::new();
    events::handle_notify_event(
        &file,
        None,
        &context,
        &sender,
        None,
        Some(&dispatch),
        &mut pending,
        &mut HashMap::new(),
        &mut HashMap::new(),
        &WatcherConfig::default(),
    );
    assert_eq!(pending.len(), 1);
    let earlier = Instant::now()
        .checked_sub(Duration::from_secs(30))
        .ok_or("clock range")?;
    for state in pending.values_mut() {
        *state = WatchState::pending(earlier, true, &WatcherConfig::default());
    }
    let (done_sender, done_receiver) = std::sync::mpsc::channel();
    super::fire_disk_updates(
        fixture.runtime.handle(),
        &std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
        &done_sender,
        &sender,
        &projects,
        &mut pending,
        &dispatch,
    );
    done_receiver.recv_timeout(Duration::from_millis(250))?;
    let mut sizes = HashMap::new();
    while let Ok(message) = receiver.try_recv() {
        if let BackgroundMsg::DiskUsageBatch { entries, .. } = message {
            sizes.extend(entries);
        }
    }
    for path in [root, child] {
        assert_eq!(sizes.get(&path).map(|sizes| sizes.total), Some(17));
    }
    assert_eq!(tracker.refresh().visited_entries, 0);
    Ok(())
}

#[test]
fn grouped_deletion_checks_keep_root_and_child_changes() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = DiskWorkerFixture::new()?;
    let root = AbsolutePath::from(fixture.directory.path());
    let child = AbsolutePath::from(root.join("child"));
    fs::create_dir(&child)?;
    fs::write(root.join("direct"), "root file")?;
    fs::write(child.join("nested"), "child file")?;
    let (sender, receiver) = channel::unbounded();
    let dispatch = watch_test_support::target_metadata(
        sender.clone(),
        root.clone(),
        AbsolutePath::from(root.join("target")),
    );
    let tracker = dispatch
        .metadata_store
        .lock()
        .map_err(|error| error.to_string())?
        .disk_usage
        .for_tree(DiskUsageTree {
            root_abs_path: root.clone(),
            entries:       vec![root.clone(), child.clone()],
        });
    let measured = tracker.refresh();
    assert_eq!(measured.entries[0].1.total, 19);
    assert_eq!(measured.entries[1].1.total, 10);
    let mut pending = HashMap::from([
        ("first".to_string(), WatchState::Idle),
        ("second".to_string(), WatchState::Idle),
    ]);
    let labels: Vec<_> = pending.keys().cloned().collect();
    let projects = [child.clone(), root.clone()]
        .into_iter()
        .zip(labels)
        .map(|(path, label)| {
            (
                path.clone(),
                ProjectEntry {
                    git_event_visibility: std::sync::Mutex::new(
                        crate::project::GitEventVisibilityCache::default(),
                    ),
                    git_refresh_demand: GitRefreshDemand::default(),
                    label,
                    abs_path: path,
                    repo_root: None,
                    git_dir: None,
                    common_git_dir: None,
                },
            )
        })
        .collect();
    fs::remove_dir_all(fixture.directory.path())?;
    let now = Instant::now();
    let earlier = now
        .checked_sub(Duration::from_secs(60))
        .ok_or("clock range")?;
    let mut last_poll = earlier;
    schedule_disk_poll_with_config(
        earlier + Duration::from_secs(30),
        &mut last_poll,
        &projects,
        &mut pending,
        &WatcherConfig::default(),
    );
    assert!(
        pending
            .values()
            .all(|state| matches!(state, WatchState::Pending { .. }))
    );
    let (done_sender, done_receiver) = std::sync::mpsc::channel();
    super::fire_disk_updates(
        fixture.runtime.handle(),
        &std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
        &done_sender,
        &sender,
        &projects,
        &mut pending,
        &dispatch,
    );
    for _ in 0..2 {
        done_receiver.recv_timeout(Duration::from_millis(250))?;
    }
    let mut sizes = HashMap::new();
    while let Ok(message) = receiver.try_recv() {
        if let BackgroundMsg::DiskUsageBatch { entries, .. } = message {
            sizes.extend(entries);
        }
    }
    for path in [root, child] {
        assert_eq!(sizes.get(&path).map(|sizes| sizes.total), Some(0));
    }
    Ok(())
}
