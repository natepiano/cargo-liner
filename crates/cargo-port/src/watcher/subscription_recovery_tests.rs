use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;

use notify::Event;
use notify::EventKind;
use notify::event::ModifyKind;
use notify::event::RenameMode;

use super::ProjectEntry;
use super::WatcherLoopState;
use super::events;
use super::events::EventContext;
use super::events::WatcherDispatchContext;
use super::roots;
use super::roots::CandidateProbeResult;
use super::roots::Subscription;
use super::roots::WatchedFolderRole;
use super::runtime::WatcherRegistrationPhase;
use super::skip::WatchSkipSet;
use super::watch_test_support::RecordedFolderWatches;
use crate::channel;
use crate::project::AbsolutePath;

#[test]
fn missing_scanned_project_keeps_surviving_ancestors_for_reconciliation()
-> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let old = AbsolutePath::from(scratch.path().join("group/old"));
    std::fs::create_dir_all(&old)?;
    std::fs::write(old.join("Cargo.toml"), "[workspace]\n")?;
    let mut watcher = RecordedFolderWatches::default();
    let (mut registered, failures) = roots::register_discovery_roots(
        &mut watcher,
        &[AbsolutePath::from(scratch.path())],
        WatchSkipSet::default(),
    );
    assert_eq!(failures.len(), 0);
    std::fs::remove_dir_all(&old)?;
    let new = AbsolutePath::from(scratch.path().join("group/new"));
    std::fs::create_dir(&new)?;
    std::fs::write(new.join("Cargo.toml"), "[workspace]\n")?;

    let failures = roots::register_project_root(&mut watcher, &mut registered, &old);

    assert_eq!(failures.len(), 1);
    assert_eq!(
        registered.role(&scratch.path().join("group")),
        Subscription::Watched(WatchedFolderRole::Discovery)
    );
    let candidates = roots::startup_reconciliation(&registered, std::slice::from_ref(&old));
    assert!(candidates.contains(&new));
    assert!(candidates.contains(&old));
    assert_eq!(registered.role(&old), Subscription::Unwatched);
    Ok(())
}

#[derive(Clone, Copy)]
enum RestorationTiming {
    BeforeDrain,
    AfterDrain,
}

#[test]
fn discovery_ancestor_restored_before_drain_recovers_known_project_trees()
-> Result<(), Box<dyn Error>> {
    assert_restored_project_trees(RestorationTiming::BeforeDrain)
}

#[test]
fn discovery_ancestor_restored_after_drain_recovers_known_project_trees()
-> Result<(), Box<dyn Error>> {
    assert_restored_project_trees(RestorationTiming::AfterDrain)
}

fn assert_restored_project_trees(timing: RestorationTiming) -> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let group = AbsolutePath::from(scratch.path().join("group"));
    let projects = [
        AbsolutePath::from(group.join("a")),
        AbsolutePath::from(group.join("nested/b")),
    ];
    let mut watcher = RecordedFolderWatches::default();
    let (mut registered, failures) = roots::register_discovery_roots(
        &mut watcher,
        &[AbsolutePath::from(scratch.path())],
        WatchSkipSet::default(),
    );
    assert_eq!(failures.len(), 0);
    for project in &projects {
        std::fs::create_dir_all(project.join("src/sub"))?;
        std::fs::create_dir_all(project.join("target/debug"))?;
        assert_eq!(
            roots::register_project_root(&mut watcher, &mut registered, project).len(),
            0
        );
    }
    let away = scratch.path().join("away");
    std::fs::rename(&group, &away)?;
    if matches!(timing, RestorationTiming::BeforeDrain) {
        std::fs::rename(&away, &group)?;
    }
    roots::update_directory_watches(
        &mut watcher,
        &mut registered,
        &Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From)))
            .add_path(group.to_path_buf()),
    );
    if matches!(timing, RestorationTiming::AfterDrain) {
        assert_eq!(registered.role(&projects[0]), Subscription::Unwatched);
        std::fs::rename(&away, &group)?;
    }
    roots::update_directory_watches(
        &mut watcher,
        &mut registered,
        &Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::To)))
            .add_path(group.to_path_buf()),
    );
    roots::finish_candidate_probe(
        &mut watcher,
        &mut registered,
        &group,
        CandidateProbeResult::NotProject,
    );

    assert_eq!(
        registered.role(&group),
        Subscription::Watched(WatchedFolderRole::Discovery)
    );
    assert_eq!(
        registered.role(&group.join("nested")),
        Subscription::Watched(WatchedFolderRole::Discovery)
    );
    for project in &projects {
        assert_eq!(
            registered.role(project),
            Subscription::Watched(WatchedFolderRole::Project)
        );
        assert_eq!(
            registered.role(&project.join("src/sub")),
            Subscription::Watched(WatchedFolderRole::Project)
        );
        assert_eq!(
            registered.role(&project.join("target")),
            Subscription::Unwatched
        );
    }
    Ok(())
}

#[test]
fn external_worktree_fetch_head_reaches_git_refresh() -> Result<(), Box<dyn Error>> {
    assert_external_git_refresh("FETCH_HEAD", GitLayout::LinkedWorktree)
}

#[test]
fn external_worktree_orig_head_reaches_git_refresh() -> Result<(), Box<dyn Error>> {
    assert_external_git_refresh("ORIG_HEAD", GitLayout::LinkedWorktree)
}

#[test]
fn containing_repository_config_reaches_nested_project_git_refresh() -> Result<(), Box<dyn Error>> {
    assert_external_git_refresh("config", GitLayout::NestedProject)
}

#[derive(Clone, Copy)]
enum GitLayout {
    LinkedWorktree,
    NestedProject,
}

fn assert_external_git_refresh(file: &str, layout: GitLayout) -> Result<(), Box<dyn Error>> {
    let scratch = tempfile::tempdir()?;
    let repo = AbsolutePath::from(scratch.path().join("repo"));
    let project = AbsolutePath::from(repo.join("project"));
    let common = AbsolutePath::from(repo.join(".git"));
    let git_dir = match layout {
        GitLayout::LinkedWorktree => AbsolutePath::from(common.join("worktrees/linked")),
        GitLayout::NestedProject => common.clone(),
    };
    std::fs::create_dir_all(&project)?;
    std::fs::create_dir_all(&git_dir)?;
    std::fs::write(project.join("Cargo.toml"), "[workspace]\n")?;
    let path = git_dir.join(file);
    std::fs::write(&path, "changed\n")?;
    let entry = ProjectEntry {
        label:          project.to_string(),
        abs_path:       project.clone(),
        repo_root:      Some(repo),
        git_dir:        Some(git_dir),
        common_git_dir: Some(common.clone()),
    };
    let mut watcher = RecordedFolderWatches::default();
    let (mut registered, failures) = roots::register_discovery_roots(
        &mut watcher,
        std::slice::from_ref(&project),
        WatchSkipSet::default(),
    );
    assert_eq!(failures.len(), 0);
    assert_eq!(
        roots::register_project_root(&mut watcher, &mut registered, &project).len(),
        0
    );
    roots::register_project_sources(&mut watcher, &mut registered, &entry);
    assert_eq!(
        registered.event_role(&path),
        Subscription::Watched(WatchedFolderRole::ConfigSource)
    );
    let mut state = WatcherLoopState {
        projects:        HashMap::new(),
        project_parents: HashSet::new(),
        pending_disk:    HashMap::new(),
        pending_git:     HashMap::new(),
        pending_new:     HashMap::new(),
        discovered:      HashSet::new(),
        registration:    WatcherRegistrationPhase::Initializing,
        buffered_events: Vec::new(),
    };
    state.projects.insert(project, entry);
    let (sender, receiver) = channel::unbounded();
    let event = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path);
    let routed =
        events::route_folder_event(&mut watcher, &mut registered, &event, &mut state, &sender);
    assert_eq!(routed, vec![event]);
    events::replay_buffered_events(
        &routed,
        &WatcherDispatchContext {
            event:             EventContext {
                watch_roots:     registered.dirs(),
                projects:        &state.projects,
                project_parents: &state.project_parents,
                discovered:      &state.discovered,
            },
            background_tx:     &sender,
            lint_runtime:      None,
            metadata_dispatch: None,
        },
        &mut state.pending_disk,
        &mut state.pending_git,
        &mut state.pending_new,
    );
    assert!(state.pending_git.contains_key(&common));
    assert_eq!(state.pending_new.len(), 0);
    assert_eq!(state.pending_disk.len(), 0);
    assert_eq!(receiver.len(), 0);
    Ok(())
}
