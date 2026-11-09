use std::fs;
use std::time::Instant;

use notify::Event;
use notify::EventKind;
use notify::event::RemoveKind;

use super::WatchRequest;
use super::WatcherLoopState;
use super::apply_watch_request;
use crate::channel;
use crate::project::AbsolutePath;
use crate::scan::BackgroundMsg;
use crate::watcher::events;
use crate::watcher::refresh;
use crate::watcher::roots;
use crate::watcher::skip::WatchSkipSet;
use crate::watcher::watch_test_support::RecordedFolderWatches;

#[test]
fn deleted_projects_release_tracking_and_disk_poll_entries() -> std::io::Result<()> {
    let directory = tempfile::tempdir()?;
    let mut watcher = RecordedFolderWatches::default();
    let (mut registered, failures) = roots::register_discovery_roots(
        &mut watcher,
        &[AbsolutePath::from(directory.path())],
        WatchSkipSet::default(),
    );
    assert!(failures.is_empty());
    let mut state = WatcherLoopState::new();
    let before = (
        state.projects.len(),
        state.project_parents.len(),
        state.pending_disk.len(),
    );
    let (sender, receiver) = channel::unbounded();
    for revision in 0..64 {
        let parent = AbsolutePath::from(directory.path().join(format!("group-{revision}")));
        let projects = [
            AbsolutePath::from(parent.join("first")),
            AbsolutePath::from(parent.join("second")),
        ];
        for path in &projects {
            fs::create_dir_all(path)?;
            fs::write(path.join("Cargo.toml"), "[workspace]\n")?;
            let label = path.to_string();
            apply_watch_request(
                WatchRequest {
                    project_label: label.clone(),
                    abs_path:      path.clone(),
                    repo_root:     None,
                },
                &mut state,
                &mut watcher,
                &mut registered,
            );
            refresh::schedule_disk_refresh(&mut state.pending_disk, &label, Instant::now());
        }
        for (index, path) in projects.iter().enumerate() {
            fs::remove_dir_all(path)?;
            let event =
                Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(path.to_path_buf());
            events::route_folder_event(&mut watcher, &mut registered, &event, &mut state, &sender);
            assert!(receiver.try_iter().any(|message| matches!(message, BackgroundMsg::DiskUsage { path: removed, bytes: 0 } if removed == *path)));
            if index == 0 {
                assert!(state.projects.contains_key(&projects[1]));
                assert!(state.project_parents.contains(&parent));
            }
        }
        assert_eq!(
            (
                state.projects.len(),
                state.project_parents.len(),
                state.pending_disk.len()
            ),
            before
        );
    }
    Ok(())
}
