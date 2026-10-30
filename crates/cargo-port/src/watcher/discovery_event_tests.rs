use std::error::Error;

use notify::Event;
use notify::EventKind;
use notify::event::ModifyKind;
use notify::event::RemoveKind;

use super::ProjectEntry;
use super::WatcherLoopState;
use crate::channel;
use crate::project::AbsolutePath;
use crate::watcher::events;
use crate::watcher::roots;
use crate::watcher::roots::Subscription;
use crate::watcher::roots::WatchedFolderRole;
use crate::watcher::skip::WatchSkipSet;
use crate::watcher::watch_test_support::RecordedFolderWatches;

#[test]
fn cargo_config_on_discovery_path_still_routes_to_metadata() -> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let cargo = scratch.path().join(".cargo");
    let project = AbsolutePath::from(cargo.join("project"));
    std::fs::create_dir_all(project.join("src"))?;
    let config = cargo.join("config.toml");
    std::fs::write(&config, "[build]\n")?;
    let mut watcher = RecordedFolderWatches::default();
    let (mut registered, failures) = roots::register_discovery_roots(
        &mut watcher,
        &[AbsolutePath::from(scratch.path())],
        WatchSkipSet::default(),
    );
    assert_eq!(failures.len(), 0);
    assert_eq!(
        roots::register_project_root(&mut watcher, &mut registered, &project).len(),
        0
    );
    let entry = ProjectEntry {
        label:          project.to_string(),
        abs_path:       project,
        repo_root:      None,
        git_dir:        None,
        common_git_dir: None,
    };
    roots::register_project_sources(&mut watcher, &mut registered, &entry);
    assert_eq!(
        registered.role(&cargo),
        Subscription::Watched(WatchedFolderRole::Discovery)
    );
    let mut state = WatcherLoopState::new();
    state.projects.insert(entry.abs_path.clone(), entry);
    let (sender, receiver) = channel::unbounded();
    let event = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(config);
    let routed =
        events::route_folder_event(&mut watcher, &mut registered, &event, &mut state, &sender);
    assert_eq!(routed, vec![event]);
    assert_eq!(state.pending_new.len(), 0);
    assert_eq!(receiver.len(), 0);
    Ok(())
}

#[test]
fn nested_configured_roots_keep_the_whole_discovery_path() -> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let nested = AbsolutePath::from(scratch.path().join("group/deep"));
    let project = AbsolutePath::from(nested.join("project"));
    std::fs::create_dir_all(project.join("src"))?;
    let configured = [AbsolutePath::from(scratch.path()), nested];
    let mut watcher = RecordedFolderWatches::default();
    let (mut registered, failures) =
        roots::register_discovery_roots(&mut watcher, &configured, WatchSkipSet::default());
    assert_eq!(failures.len(), 0);
    assert_eq!(
        roots::register_project_root(&mut watcher, &mut registered, &project).len(),
        0
    );
    assert_eq!(registered.dirs(), &configured);
    assert_eq!(
        registered.role(&scratch.path().join("group")),
        Subscription::Watched(WatchedFolderRole::Discovery)
    );
    assert_eq!(
        registered.role(&project.join("src")),
        Subscription::Watched(WatchedFolderRole::Project)
    );
    Ok(())
}

#[test]
fn removing_unwatched_ancestor_drops_nested_discovery_root() -> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let ancestor = scratch.path().join("group");
    let nested = AbsolutePath::from(ancestor.join("deep"));
    std::fs::create_dir_all(&nested)?;
    let configured = [AbsolutePath::from(scratch.path()), nested.clone()];
    let mut watcher = RecordedFolderWatches::default();
    let (mut registered, failures) =
        roots::register_discovery_roots(&mut watcher, &configured, WatchSkipSet::default());
    assert_eq!(failures.len(), 0);
    std::fs::remove_dir_all(&ancestor)?;
    let event = Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(ancestor);
    roots::update_directory_watches(&mut watcher, &mut registered, &event);
    assert_eq!(watcher.unwatched, vec![nested.to_path_buf()]);
    assert_eq!(registered.role(&nested), Subscription::Unwatched);
    assert_eq!(registered.dirs(), &configured);
    Ok(())
}
