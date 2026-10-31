#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use notify::Config;
use notify::Event;
use notify::EventKind;
use notify::RecursiveMode;
use notify::Watcher;
use notify::WatcherKind;
use notify::event::CreateKind;
use notify::event::RemoveKind;
use tempfile::TempDir;

use super::roots;
use super::roots::CargoHome;
use super::roots::WatchRegistration;
use super::skip::WatchSkipSet;
use crate::project::AbsolutePath;

/// Records the registration plan without opening a platform watcher.
#[derive(Default)]
struct RegistrationCalls {
    added:   Vec<(PathBuf, RecursiveMode)>,
    removed: Vec<PathBuf>,
    fail_on: HashSet<PathBuf>,
}

impl Watcher for RegistrationCalls {
    fn new<F: notify::EventHandler>(_: F, _: Config) -> notify::Result<Self> { Ok(Self::default()) }

    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        self.added.push((path.to_path_buf(), mode));
        if self.fail_on.contains(path) {
            return Err(notify::Error::generic("injected registration failure"));
        }
        Ok(())
    }

    fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        self.removed.push(path.to_path_buf());
        Ok(())
    }

    fn kind() -> WatcherKind { WatcherKind::NullWatcher }
}

fn fixture() -> TempDir {
    let scratch = tempfile::tempdir().expect("scratch root");
    for directory in [
        "src/nested",
        "target/debug/deps",
        ".git/objects/ab",
        ".git/refs",
    ] {
        std::fs::create_dir_all(scratch.path().join(directory)).expect("fixture directory");
    }
    scratch
}

#[test]
fn recursive_root_overlaps_keep_boundaries_with_one_watch_in_both_orders() {
    let scratch = fixture();
    let parent = AbsolutePath::from(scratch.path());
    let child = AbsolutePath::from(scratch.path().join("src"));
    for configured in [
        [parent.clone(), child.clone()],
        [child.clone(), parent.clone()],
    ] {
        let mut calls = RegistrationCalls::default();
        let (mut registered, failures) = roots::register_watch_roots_with_skips(
            &mut calls,
            &configured,
            WatchSkipSet::default(),
            WatchRegistration::RecursiveRoot,
        );

        assert!(failures.is_empty());
        assert_eq!(registered.dirs(), &configured);
        assert_eq!(
            calls.added,
            vec![(parent.to_path_buf(), RecursiveMode::Recursive)]
        );
        assert!(registered.covers(&child.join("nested/lib.rs")));
        assert_eq!(calls.removed, [] as [std::path::PathBuf; 0]);

        // A discovered project already covered by the root must not add a watch either.
        let failures = roots::register_project_root(&mut calls, &mut registered, &child);
        assert!(failures.is_empty());
        assert_eq!(calls.added.len(), 1);
        assert_eq!(registered.dirs(), &configured);
    }
}

#[test]
fn recursive_root_failed_parent_still_watches_child_in_both_orders() {
    let scratch = fixture();
    let parent = AbsolutePath::from(scratch.path());
    let child = AbsolutePath::from(scratch.path().join("src"));
    for configured in [
        [parent.clone(), child.clone()],
        [child.clone(), parent.clone()],
    ] {
        let mut calls = RegistrationCalls {
            fail_on: HashSet::from([parent.to_path_buf()]),
            ..RegistrationCalls::default()
        };
        let (registered, failures) = roots::register_watch_roots_with_skips(
            &mut calls,
            &configured,
            WatchSkipSet::default(),
            WatchRegistration::RecursiveRoot,
        );

        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].dir, parent);
        assert!(matches!(
            failures[0].reason,
            roots::WatchRootRegistrationFailureReason::Notify(_)
        ));
        assert_eq!(registered.dirs(), std::slice::from_ref(&child));
        assert_eq!(
            calls.added,
            vec![
                (parent.to_path_buf(), RecursiveMode::Recursive),
                (child.to_path_buf(), RecursiveMode::Recursive),
            ]
        );
        assert!(registered.covers(&child.join("nested/lib.rs")));
        assert!(!registered.covers(&parent.join("elsewhere/lib.rs")));
        assert_eq!(calls.removed, [] as [std::path::PathBuf; 0]);
    }
}

#[test]
fn recursive_root_registers_only_the_root_and_filters_build_events() {
    let scratch = fixture();
    let root = AbsolutePath::from(scratch.path());
    let mut calls = RegistrationCalls::default();
    let (registered, failures) = roots::register_watch_roots_with_skips(
        &mut calls,
        std::slice::from_ref(&root),
        WatchSkipSet::default(),
        WatchRegistration::RecursiveRoot,
    );
    let plan: Vec<_> = roots::registration_plan(
        &root,
        &WatchSkipSet::default(),
        WatchRegistration::RecursiveRoot,
    )
    .collect();
    assert_eq!(plan, vec![(root, RecursiveMode::Recursive)]);

    assert!(failures.is_empty());
    assert_eq!(
        calls.added,
        vec![(scratch.path().to_path_buf(), RecursiveMode::Recursive)]
    );
    let source = scratch.path().join("src/nested/lib.rs");
    let mut event = Event::new(EventKind::Create(CreateKind::File))
        .add_path(scratch.path().join("target/debug/deps/output"))
        .add_path(scratch.path().join(".git/objects/ab/object"))
        .add_path(source.clone());
    roots::filter_event_paths(&registered, &mut event);
    assert_eq!(event.paths, vec![source]);
}

#[test]
fn recursive_root_created_folder_adds_no_watch() {
    let scratch = fixture();
    let mut calls = RegistrationCalls::default();
    let (mut registered, failures) = roots::register_watch_roots_with_skips(
        &mut calls,
        &[AbsolutePath::from(scratch.path())],
        WatchSkipSet::default(),
        WatchRegistration::RecursiveRoot,
    );
    assert!(failures.is_empty());
    calls.added.clear();
    let created = scratch.path().join("src/new/nested");
    std::fs::create_dir_all(&created).expect("created folders");
    let event =
        Event::new(EventKind::Create(CreateKind::Folder)).add_path(scratch.path().join("src/new"));

    roots::update_directory_watches(&mut calls, &mut registered, &event);

    assert!(
        calls.added.is_empty(),
        "recursive root already covers new folders"
    );
    assert_eq!(calls.removed, [] as [PathBuf; 0]);
}

#[test]
fn recursive_root_removed_folder_drops_no_watch() {
    let scratch = fixture();
    let mut calls = RegistrationCalls::default();
    let (mut registered, failures) = roots::register_watch_roots_with_skips(
        &mut calls,
        &[AbsolutePath::from(scratch.path())],
        WatchSkipSet::default(),
        WatchRegistration::RecursiveRoot,
    );
    assert!(failures.is_empty());
    calls.added.clear();
    let removed = scratch.path().join("src/nested");
    std::fs::remove_dir_all(&removed).expect("remove source folder");
    let event = Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(removed);

    roots::update_directory_watches(&mut calls, &mut registered, &event);

    assert!(
        calls.removed.is_empty(),
        "recursive backend owns only the root watch"
    );
    assert_eq!(calls.added, [] as [(PathBuf, RecursiveMode); 0]);
}

#[test]
fn recursive_root_target_change_updates_filter_without_changing_watches() {
    let scratch = fixture();
    let target = AbsolutePath::from(scratch.path().join("custom-build"));
    std::fs::create_dir_all(target.join("debug/deps")).expect("custom build output");
    let mut calls = RegistrationCalls::default();
    let (mut registered, failures) = roots::register_watch_roots_with_skips(
        &mut calls,
        &[AbsolutePath::from(scratch.path())],
        WatchSkipSet::default(),
        WatchRegistration::RecursiveRoot,
    );
    assert!(failures.is_empty());
    calls.added.clear();
    let mut skips = WatchSkipSet::default();
    skips.add_target_directory(target.clone());

    roots::replace_skip_set(&mut calls, &mut registered, skips, &CargoHome::Unavailable);

    assert!(registered.skips(&target.join("debug/deps/output")));
    assert!(
        calls.added.is_empty(),
        "skip changes must not rebuild recursive streams"
    );
    assert!(
        calls.removed.is_empty(),
        "skip changes must not remove recursive streams"
    );
}

#[test]
fn per_folder_registration_preserves_phase_two_paths_and_modes() {
    let scratch = fixture();
    let mut calls = RegistrationCalls::default();
    let (_, failures) = roots::register_watch_roots_with_skips(
        &mut calls,
        &[AbsolutePath::from(scratch.path())],
        WatchSkipSet::default(),
        WatchRegistration::PerFolder,
    );
    assert!(failures.is_empty());
    let mut expected: Vec<_> = ["", "src", "src/nested", ".git", ".git/refs"]
        .into_iter()
        .map(|relative| (scratch.path().join(relative), RecursiveMode::NonRecursive))
        .collect();
    calls.added.sort_by(|left, right| left.0.cmp(&right.0));
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(calls.added, expected);
    let mut plan: Vec<_> = roots::registration_plan(
        &AbsolutePath::from(scratch.path()),
        &WatchSkipSet::default(),
        WatchRegistration::PerFolder,
    )
    .map(|(path, mode)| (path.to_path_buf(), mode))
    .collect();
    plan.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(plan, expected);
    assert_eq!(calls.removed, [] as [PathBuf; 0]);
}

#[test]
fn recursive_cargo_home_registers_once_and_reuses_covering_root() {
    let scratch = fixture();
    let external = tempfile::tempdir().expect("external Cargo home");
    std::fs::create_dir_all(external.path().join("registry/src")).expect("Cargo registry fixture");
    let covered = scratch.path().join(".cargo");
    std::fs::create_dir_all(covered.join("registry/src")).expect("covered Cargo registry fixture");
    let mut calls = RegistrationCalls::default();
    let (mut registered, failures) = roots::register_watch_roots_with_skips(
        &mut calls,
        &[AbsolutePath::from(scratch.path())],
        WatchSkipSet::default(),
        WatchRegistration::RecursiveRoot,
    );
    assert!(failures.is_empty());
    calls.added.clear();

    roots::register_cargo_home_directory(&mut calls, &mut registered, external.path());
    roots::register_cargo_home_directory(&mut calls, &mut registered, external.path());
    roots::register_cargo_home_directory(&mut calls, &mut registered, &covered);

    assert_eq!(
        calls.added,
        vec![(external.path().to_path_buf(), RecursiveMode::Recursive)]
    );
    assert_eq!(calls.removed, [] as [PathBuf; 0]);
}
