#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use notify::Config;
use notify::Event;
use notify::EventKind;
use notify::RecursiveMode;
use notify::Watcher;
use notify::WatcherKind;
use notify::event::CreateKind;
use notify::event::ModifyKind;
use notify::event::RemoveKind;

use super::events;
use super::events::EventContext;
use super::events::WatcherDispatchContext;
use super::probe;
use super::roots;
use super::roots::CargoHome;
use super::roots::RegisteredRoots;
use super::roots::Subscription;
use super::roots::WatchedFolderRole;
use super::runtime;
use super::runtime::ProjectEntry;
use super::runtime::WatcherLoopState;
use super::runtime::WatcherRegistrationPhase;
use super::skip::WatchSkipSet;
use crate::channel;
use crate::config::NonRustInclusion;
use crate::project::AbsolutePath;
use crate::project::WorkspaceMetadataStore;
use crate::scan::BackgroundMsg;
use crate::scan::MetadataDispatchContext;
use crate::test_support;

/// Records the subscriptions requested by production code without a kernel watcher.
#[derive(Default)]
struct FolderSubscriptions {
    active: HashSet<PathBuf>,
}

impl Watcher for FolderSubscriptions {
    fn new<F: notify::EventHandler>(_: F, _: Config) -> notify::Result<Self> { Ok(Self::default()) }

    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        assert_eq!(mode, RecursiveMode::NonRecursive);
        self.active.insert(path.to_path_buf());
        Ok(())
    }

    fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        assert!(
            self.active.remove(path),
            "unwatch without a subscription: {}",
            path.display()
        );
        Ok(())
    }

    fn kind() -> WatcherKind { WatcherKind::NullWatcher }
}

fn discovery_root(root: &Path) -> (FolderSubscriptions, RegisteredRoots) {
    let mut watcher = FolderSubscriptions::default();
    let (registered, failures) = roots::register_discovery_roots(
        &mut watcher,
        &[AbsolutePath::from(root)],
        WatchSkipSet::default(),
    );
    assert!(failures.is_empty());
    (watcher, registered)
}

fn workspace(path: &Path) -> AbsolutePath {
    std::fs::create_dir_all(path.join("src")).expect("workspace source directory");
    std::fs::write(path.join("Cargo.toml"),
        "[package]\nname = \"watch-role-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[workspace]\n"
    ).expect("workspace manifest");
    std::fs::write(path.join("src/lib.rs"), "").expect("library target");
    AbsolutePath::from(path)
}

fn project_entry(path: AbsolutePath) -> ProjectEntry {
    ProjectEntry {
        label:          path.display().to_string(),
        abs_path:       path,
        repo_root:      None,
        git_dir:        None,
        common_git_dir: None,
    }
}

fn created_folder(path: &Path) -> Event {
    Event::new(EventKind::Create(CreateKind::Folder)).add_path(path.to_path_buf())
}

fn watcher_state() -> WatcherLoopState {
    WatcherLoopState {
        projects:        HashMap::new(),
        project_parents: HashSet::new(),
        pending_disk:    HashMap::new(),
        pending_git:     HashMap::new(),
        pending_new:     HashMap::new(),
        discovered:      HashSet::new(),
        registration:    WatcherRegistrationPhase::Initializing,
        buffered_events: Vec::new(),
    }
}

fn promote_probed_project(
    watcher: &mut FolderSubscriptions,
    registered: &mut RegisteredRoots,
    state: &mut WatcherLoopState,
    path: &AbsolutePath,
) {
    let (sender, receiver) = channel::unbounded();
    runtime::apply_folder_probe(
        watcher,
        registered,
        state,
        &sender,
        probe::probe_folder(path, NonRustInclusion::Exclude),
    );
    assert_eq!(
        registered.role(path),
        Subscription::Watched(WatchedFolderRole::Project)
    );
    assert!(watcher.active.contains(&path.join("src")));
    assert!(state.projects.contains_key(path));
    assert!(state.discovered.contains(path));
    assert!(
        receiver.is_empty(),
        "watch promotion does not launch enrichment"
    );
}

#[test]
fn known_project_watches_ancestors_and_source_without_sibling_tree() {
    let scratch = tempfile::tempdir().expect("scratch root");
    let project = workspace(&scratch.path().join("group/project"));
    let sibling = scratch.path().join("sessions/a/b/c");
    std::fs::create_dir_all(&sibling).expect("unrelated session tree");
    std::fs::create_dir_all(project.join("target/debug")).expect("build output");
    let (mut watcher, mut registered) = discovery_root(scratch.path());

    assert_eq!(
        watcher.active,
        HashSet::from([scratch.path().to_path_buf()])
    );
    assert!(roots::register_project_root(&mut watcher, &mut registered, &project).is_empty());

    assert_eq!(
        watcher.active,
        HashSet::from([
            scratch.path().to_path_buf(),
            scratch.path().join("group"),
            project.to_path_buf(),
            project.join("src"),
        ])
    );
    assert_eq!(
        registered.role(&scratch.path().join("group")),
        Subscription::Watched(WatchedFolderRole::Discovery)
    );
    assert_eq!(
        registered.role(&project.join("src")),
        Subscription::Watched(WatchedFolderRole::Project)
    );
    assert_eq!(registered.dirs(), &[AbsolutePath::from(scratch.path())]);
}

#[test]
fn startup_reconciliation_finds_project_created_after_scan_before_watches() {
    let scratch = tempfile::tempdir().expect("scratch root");
    let scanned = workspace(&scratch.path().join("group/scanned"));
    let scan_projects = [scanned.clone()];
    let (mut watcher, mut registered) = discovery_root(scratch.path());
    let late = workspace(&scratch.path().join("group/late"));
    assert!(roots::register_project_root(&mut watcher, &mut registered, &scanned).is_empty());

    let reconciliation = roots::startup_reconciliation(&registered, &scan_projects);

    assert!(
        reconciliation.contains(&late),
        "scan/watch gap lost the new project"
    );
    let mut state = watcher_state();
    state
        .projects
        .insert(scanned.clone(), project_entry(scanned.clone()));
    state.discovered.insert(scanned);
    let (sender, receiver) = channel::unbounded();
    for path in reconciliation {
        runtime::apply_folder_probe(
            &mut watcher,
            &mut registered,
            &mut state,
            &sender,
            probe::probe_folder(&path, NonRustInclusion::Exclude),
        );
    }
    assert_eq!(
        registered.role(&late),
        Subscription::Watched(WatchedFolderRole::Project)
    );
    assert!(watcher.active.contains(&late.join("src")));
    assert!(state.projects.contains_key(&late));
    assert!(
        receiver.is_empty(),
        "watch promotion does not launch enrichment"
    );
}

#[test]
fn discovery_child_becomes_candidate_then_project_with_source_watches() {
    let scratch = tempfile::tempdir().expect("scratch root");
    let known = workspace(&scratch.path().join("group/known"));
    let (mut watcher, mut registered) = discovery_root(scratch.path());
    assert!(roots::register_project_root(&mut watcher, &mut registered, &known).is_empty());
    let new = AbsolutePath::from(scratch.path().join("group/new"));
    std::fs::create_dir(&new).expect("new discovery child");
    let (sender, receiver) = channel::unbounded();
    let mut state = watcher_state();

    assert_eq!(
        events::route_folder_event(
            &mut watcher,
            &mut registered,
            &created_folder(&new),
            &mut state,
            &sender
        ),
        [] as [notify::Event; 0]
    );
    assert_eq!(
        registered.role(&new),
        Subscription::Watched(WatchedFolderRole::Candidate)
    );
    assert!(state.pending_new.contains_key(&new));
    workspace(&new);
    let manifest = Event::new(EventKind::Create(CreateKind::File)).add_path(new.join("Cargo.toml"));
    events::route_folder_event(
        &mut watcher,
        &mut registered,
        &manifest,
        &mut state,
        &sender,
    );
    assert!(
        !state
            .pending_new
            .contains_key(&AbsolutePath::from(new.join("Cargo.toml")))
    );

    promote_probed_project(&mut watcher, &mut registered, &mut state, &new);
    assert_eq!(registered.dirs(), &[AbsolutePath::from(scratch.path())]);
    assert!(receiver.is_empty());
}

#[test]
fn manifest_in_configured_root_is_probed_as_root() {
    let scratch = tempfile::tempdir().expect("scratch root");
    let (mut watcher, mut registered) = discovery_root(scratch.path());
    let root = workspace(scratch.path());
    let manifest = root.join("Cargo.toml");
    let event = Event::new(EventKind::Create(CreateKind::File)).add_path(manifest.clone());
    let (sender, receiver) = channel::unbounded();
    let mut state = watcher_state();

    events::route_folder_event(&mut watcher, &mut registered, &event, &mut state, &sender);

    assert_eq!(
        probe::project_level_dir(&manifest, registered.dirs(), &state.project_parents),
        Some(root.clone())
    );
    assert!(state.pending_new.contains_key(&root));
    assert!(
        !state
            .pending_new
            .contains_key(&AbsolutePath::from(manifest))
    );
    promote_probed_project(&mut watcher, &mut registered, &mut state, &root);
    assert!(receiver.is_empty());
}

#[test]
fn empty_candidate_drops_watch_and_never_watches_its_children() {
    let scratch = tempfile::tempdir().expect("scratch root");
    let (mut watcher, mut registered) = discovery_root(scratch.path());
    let candidate = AbsolutePath::from(scratch.path().join("session"));
    std::fs::create_dir(&candidate).expect("candidate directory");
    roots::update_directory_watches(&mut watcher, &mut registered, &created_folder(&candidate));
    assert!(watcher.active.contains(candidate.as_path()));
    let child = candidate.join("nested");
    std::fs::create_dir(&child).expect("candidate child");

    roots::update_directory_watches(&mut watcher, &mut registered, &created_folder(&child));

    assert!(!watcher.active.contains(&child));
    let mut state = watcher_state();
    let (sender, receiver) = channel::unbounded();
    runtime::apply_folder_probe(
        &mut watcher,
        &mut registered,
        &mut state,
        &sender,
        probe::probe_folder(&candidate, NonRustInclusion::Exclude),
    );
    assert_eq!(
        watcher.active,
        HashSet::from([scratch.path().to_path_buf()])
    );
    assert_eq!(registered.role(&candidate), Subscription::Unwatched);
    assert!(receiver.is_empty());
}

#[test]
fn late_target_change_prunes_project_output_without_walking_siblings() {
    let scratch = tempfile::tempdir().expect("scratch root");
    let project = workspace(&scratch.path().join("group/project"));
    let target = AbsolutePath::from(project.join("custom-build"));
    std::fs::create_dir_all(target.join("deps")).expect("custom build tree");
    std::fs::create_dir_all(scratch.path().join("sessions/a/b")).expect("unrelated sibling");
    let (mut watcher, mut registered) = discovery_root(scratch.path());
    assert!(roots::register_project_root(&mut watcher, &mut registered, &project).is_empty());
    assert!(watcher.active.contains(target.as_path()));
    let mut skips = WatchSkipSet::default();
    skips.add_target_directory(target.clone());

    roots::replace_skip_set(
        &mut watcher,
        &mut registered,
        skips,
        &CargoHome::Unavailable,
    );

    assert!(!watcher.active.iter().any(|path| path.starts_with(&target)));
    assert!(
        !watcher
            .active
            .iter()
            .any(|path| path.starts_with(scratch.path().join("sessions")))
    );
    assert!(watcher.active.contains(&project.join("src")));
}

#[test]
fn ancestor_cargo_config_keeps_subscription_and_metadata_refresh() {
    let scratch = tempfile::tempdir().expect("scratch root");
    let project = workspace(&scratch.path().join("group/project"));
    let cargo_dir = scratch.path().join(".cargo");
    std::fs::create_dir(&cargo_dir).expect("ancestor Cargo config directory");
    let config = cargo_dir.join("config.toml");
    std::fs::write(&config, "[build]\n").expect("Cargo config");
    let (mut watcher, mut registered) = discovery_root(scratch.path());
    assert!(roots::register_project_root(&mut watcher, &mut registered, &project).is_empty());
    let entry = project_entry(project.clone());
    roots::register_project_sources(&mut watcher, &mut registered, &entry);
    assert!(watcher.active.contains(&cargo_dir));
    assert_eq!(
        registered.role(&cargo_dir),
        Subscription::Watched(WatchedFolderRole::ConfigSource)
    );
    let mut state = watcher_state();
    state.projects.insert(project.clone(), entry);
    let (sender, receiver) = channel::unbounded();
    let dispatch = MetadataDispatchContext {
        handle:         test_support::test_runtime().handle().clone(),
        sender:         sender.clone(),
        metadata_store: Arc::new(Mutex::new(WorkspaceMetadataStore::new())),
        metadata_limit: Arc::new(tokio::sync::Semaphore::new(1)),
    };
    // Force the metadata worker's missing-manifest reply, avoiding a Cargo subprocess.
    // The event still has to traverse the config subscription and dispatch path.
    std::fs::remove_file(project.join("Cargo.toml"))
        .expect("remove manifest before metadata dispatch");
    let event = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(config);
    let routed =
        events::route_folder_event(&mut watcher, &mut registered, &event, &mut state, &sender);
    let context = EventContext {
        watch_roots:     registered.dirs(),
        projects:        &state.projects,
        project_parents: &state.project_parents,
        discovered:      &state.discovered,
    };
    events::replay_buffered_events(
        &routed,
        &WatcherDispatchContext {
            event:             context,
            background_tx:     &sender,
            lint_runtime:      None,
            metadata_dispatch: Some(&dispatch),
        },
        &mut state.pending_disk,
        &mut state.pending_git,
        &mut state.pending_new,
    );

    let reply = receiver
        .recv_timeout(Duration::from_millis(500))
        .expect("config change reaches metadata worker");
    assert!(
        matches!(reply, BackgroundMsg::CargoMetadata { workspace_root, .. } if workspace_root == project)
    );
}

#[test]
fn linked_worktree_git_event_keeps_subscription_and_git_refresh() {
    let scratch = tempfile::tempdir().expect("scratch root");
    let root = scratch.path().join("projects");
    std::fs::create_dir(&root).expect("discovery root");
    let project = workspace(&root.join("linked"));
    let common = AbsolutePath::from(scratch.path().join("main/.git"));
    let git_dir = AbsolutePath::from(common.join("worktrees/linked"));
    std::fs::create_dir_all(&git_dir).expect("external worktree Git folder");
    std::fs::create_dir_all(common.join("objects/ab")).expect("Git object tree");
    std::fs::write(
        project.join(".git"),
        format!("gitdir: {}\n", git_dir.display()),
    )
    .expect("worktree pointer");
    std::fs::write(git_dir.join("commondir"), "../..\n").expect("common Git pointer");
    std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").expect("worktree HEAD");
    let (mut watcher, mut registered) = discovery_root(&root);
    assert!(roots::register_project_root(&mut watcher, &mut registered, &project).is_empty());
    let mut entry = project_entry(project.clone());
    entry.repo_root = Some(project.clone());
    entry.git_dir = Some(git_dir.clone());
    entry.common_git_dir = Some(common.clone());
    roots::register_project_sources(&mut watcher, &mut registered, &entry);
    assert!(watcher.active.contains(git_dir.as_path()));
    assert!(!watcher.active.contains(&common.join("objects")));
    let mut state = watcher_state();
    state.projects.insert(project, entry);
    let (sender, receiver) = channel::unbounded();
    let event = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(git_dir.join("HEAD"));
    let routed =
        events::route_folder_event(&mut watcher, &mut registered, &event, &mut state, &sender);
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

    assert!(
        state.pending_git.contains_key(&common),
        "external Git event lost its refresh"
    );
    drop(receiver);
}

#[test]
fn removed_project_drops_watches_and_publishes_zero_disk_usage() {
    assert_removal_replies(RemovedTree::Project);
}

#[test]
fn removed_ancestor_publishes_zero_for_both_projects() {
    assert_removal_replies(RemovedTree::Ancestor);
}

#[derive(Clone, Copy)]
enum RemovedTree {
    Project,
    Ancestor,
}

fn assert_removal_replies(removed_tree: RemovedTree) {
    let scratch = tempfile::tempdir().expect("scratch root");
    let first = workspace(&scratch.path().join("group/first"));
    let second = workspace(&scratch.path().join("group/second"));
    let (mut watcher, mut registered) = discovery_root(scratch.path());
    let mut state = watcher_state();
    for path in [&first, &second] {
        assert!(roots::register_project_root(&mut watcher, &mut registered, path).is_empty());
        state
            .projects
            .insert(path.clone(), project_entry(path.clone()));
        state.discovered.insert(path.clone());
    }
    let removed = match removed_tree {
        RemovedTree::Project => first.to_path_buf(),
        RemovedTree::Ancestor => scratch.path().join("group"),
    };
    std::fs::remove_dir_all(&removed).expect("remove watched tree");
    let event = Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(removed.clone());
    let (sender, receiver) = channel::unbounded();

    events::route_folder_event(&mut watcher, &mut registered, &event, &mut state, &sender);

    let deleted: HashSet<_> = receiver
        .try_iter()
        .map(|message| {
            let path = match message {
                BackgroundMsg::DiskUsage { path, bytes: 0 } => Some(path),
                _ => None,
            };
            path.expect("removal must publish zero-byte project usage")
        })
        .collect();
    let expected = match removed_tree {
        RemovedTree::Project => HashSet::from([first.clone()]),
        RemovedTree::Ancestor => HashSet::from([first.clone(), second.clone()]),
    };
    assert_eq!(deleted, expected);
    assert!(!watcher.active.iter().any(|path| path.starts_with(&removed)));
    assert!(
        state.projects.contains_key(&first),
        "retain project for Deleted row"
    );
    assert!(state.projects.contains_key(&second));
    if matches!(removed_tree, RemovedTree::Project) {
        assert!(watcher.active.contains(second.as_path()));
    }
}
