use std::collections::HashSet;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::path::PathBuf;

use notify::Error;
use notify::Event;
use notify::EventKind;
use notify::RecursiveMode;
use notify::Watcher;
use notify::event::ModifyKind;
use notify::event::RenameMode;
use tui_pane::PERF_LOG_TARGET;
use walkdir::WalkDir;

use super::skip::WatchSkipSet;
use crate::constants::DOT_CARGO_DIR;
use crate::project::AbsolutePath;

/// Subscription strategy chosen for the platform watcher.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum WatchRegistration {
    RecursiveRoot,
    #[default]
    PerFolder,
}

impl WatchRegistration {
    const fn mode(self) -> RecursiveMode {
        match self {
            Self::RecursiveRoot => RecursiveMode::Recursive,
            Self::PerFolder => RecursiveMode::NonRecursive,
        }
    }
}

/// Roots, their subscription strategy, and successfully registered paths.
#[derive(Clone, Default)]
pub(super) struct RegisteredRoots {
    dirs:         Vec<AbsolutePath>,
    watched:      HashSet<AbsolutePath>,
    skips:        WatchSkipSet,
    registration: WatchRegistration,
}

impl RegisteredRoots {
    pub(super) fn dirs(&self) -> &[AbsolutePath] { &self.dirs }

    #[cfg(test)]
    pub(super) fn from_dirs(dirs: Vec<AbsolutePath>) -> Self {
        let watched = dirs
            .iter()
            .flat_map(|dir| watch_directories(dir, &WatchSkipSet::default()))
            .collect();
        Self {
            dirs,
            watched,
            skips: WatchSkipSet::default(),
            registration: WatchRegistration::PerFolder,
        }
    }

    pub(super) fn covers(&self, path: &Path) -> bool {
        !self.skips.skips(path) && self.dirs.iter().any(|root| path.starts_with(root))
    }

    pub(super) fn skips(&self, path: &Path) -> bool { self.skips.skips(path) }
}

pub(super) fn filter_event_paths(registered: &RegisteredRoots, event: &mut Event) {
    event.paths.retain(|path| !registered.skips(path));
}

pub(super) struct WatchRootRegistrationFailure {
    pub(super) dir:    AbsolutePath,
    pub(super) reason: WatchRootRegistrationFailureReason,
}

pub(super) enum WatchRootRegistrationFailureReason {
    NotADirectory,
    Notify(Error),
}

impl Display for WatchRootRegistrationFailureReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotADirectory => f.write_str("path is not a directory"),
            Self::Notify(err) => write!(f, "notify watch failed: {err}"),
        }
    }
}

fn directory_walk<'a>(
    root: &'a AbsolutePath,
    skips: &'a WatchSkipSet,
) -> impl Iterator<Item = AbsolutePath> + 'a {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |entry| !entry.file_type().is_symlink() && !skips.skips(entry.path()))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_dir())
        .map(|entry| AbsolutePath::from(entry.into_path()))
}

#[cfg(test)]
pub(super) fn watch_directories(root: &AbsolutePath, skips: &WatchSkipSet) -> Vec<AbsolutePath> {
    directory_walk(root, skips).collect()
}

/// Yield subscriptions lazily so each parent is watched before walking its children.
pub(super) fn registration_plan<'a>(
    root: &'a AbsolutePath,
    skips: &'a WatchSkipSet,
    registration: WatchRegistration,
) -> Box<dyn Iterator<Item = (AbsolutePath, RecursiveMode)> + 'a> {
    match registration {
        WatchRegistration::RecursiveRoot => Box::new(
            std::iter::once((root.clone(), RecursiveMode::Recursive))
                .filter(move |(path, _)| !skips.skips(path)),
        ),
        WatchRegistration::PerFolder => {
            Box::new(directory_walk(root, skips).map(|path| (path, RecursiveMode::NonRecursive)))
        },
    }
}

#[cfg(test)]
pub(super) fn register_watch_roots(
    watcher: &mut impl Watcher,
    watch_dirs: &[AbsolutePath],
) -> (RegisteredRoots, Vec<WatchRootRegistrationFailure>) {
    register_watch_roots_with_skips(
        watcher,
        watch_dirs,
        WatchSkipSet::default(),
        WatchRegistration::PerFolder,
    )
}

pub(super) fn register_watch_roots_with_skips(
    watcher: &mut impl Watcher,
    watch_dirs: &[AbsolutePath],
    skips: WatchSkipSet,
    registration: WatchRegistration,
) -> (RegisteredRoots, Vec<WatchRootRegistrationFailure>) {
    let mut registered = RegisteredRoots {
        skips,
        registration,
        ..RegisteredRoots::default()
    };
    let mut failures = Vec::new();
    for dir in watch_dirs {
        failures.extend(register_project_root(watcher, &mut registered, dir));
    }
    (registered, failures)
}

pub(super) fn register_project_root(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    dir: &AbsolutePath,
) -> Vec<WatchRootRegistrationFailure> {
    if registered.skips.skips(dir) {
        return Vec::new();
    }
    if !dir.is_dir() {
        return vec![WatchRootRegistrationFailure {
            dir:    dir.clone(),
            reason: WatchRootRegistrationFailureReason::NotADirectory,
        }];
    }
    let failures = register_directory_tree(watcher, registered, dir);
    if registered.watched.contains(dir) && !registered.dirs.contains(dir) {
        registered.dirs.push(dir.clone());
    }
    failures
}

fn register_directory_tree(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    dir: &AbsolutePath,
) -> Vec<WatchRootRegistrationFailure> {
    let mut failures = Vec::new();
    // Walk lazily: watch each parent before reading its children, so concurrent
    // folder creation is either seen by the walk or queued as a parent event.
    for (folder, mode) in registration_plan(dir, &registered.skips, registered.registration) {
        if registered.watched.contains(&folder) {
            continue;
        }
        match watcher.watch(&folder, mode) {
            Ok(()) => {
                registered.watched.insert(folder);
            },
            Err(err) => failures.push(WatchRootRegistrationFailure {
                dir:    folder,
                reason: WatchRootRegistrationFailureReason::Notify(err),
            }),
        }
    }
    failures
}

fn remove_directory_watches(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    path: &Path,
) {
    if !registered.watched.contains(path) {
        return;
    }
    let removed: Vec<_> = registered
        .watched
        .iter()
        .filter(|dir| dir.starts_with(path))
        .cloned()
        .collect();
    for dir in removed {
        // notify may have already removed a watch when its inode disappeared.
        let _ = watcher.unwatch(&dir);
        registered.watched.remove(&dir);
    }
}

pub(super) fn update_directory_watches(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    event: &Event,
) {
    if registered.registration == WatchRegistration::RecursiveRoot {
        return;
    }
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    ) {
        return;
    }
    for (index, path) in event.paths.iter().enumerate() {
        if !registered.covers(path) {
            continue;
        }
        let removed = matches!(
            event.kind,
            EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(RenameMode::From))
        ) || matches!(
            event.kind,
            EventKind::Modify(ModifyKind::Name(RenameMode::Both))
        ) && index == 0;
        if removed || !path.is_dir() {
            remove_directory_watches(watcher, registered, path);
        }
        // The path may already have been recreated before the remove event was drained.
        if path.is_dir() {
            for failure in
                register_directory_tree(watcher, registered, &AbsolutePath::from(path.as_path()))
            {
                tracing::error!(dir = %failure.dir.display(), reason = %failure.reason, "watcher_folder_registration_failed");
            }
        }
    }
}

pub(super) fn replace_skip_set(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    skips: WatchSkipSet,
) {
    if registered.registration == WatchRegistration::RecursiveRoot {
        registered.skips = skips;
        return;
    }
    if registered.skips == skips {
        return;
    }
    let removed: Vec<_> = registered
        .watched
        .iter()
        .filter(|dir| skips.skips_resolved(dir))
        .cloned()
        .collect();
    for dir in removed {
        let _ = watcher.unwatch(&dir);
        registered.watched.remove(&dir);
    }
    registered.skips = skips;
    // A former target directory may now contain source files after a config edit.
    for root in registered.dirs.clone() {
        for failure in register_directory_tree(watcher, registered, &root) {
            tracing::error!(dir = %failure.dir.display(), reason = %failure.reason, "watcher_folder_registration_failed");
        }
    }
    register_cargo_home_watch(watcher, registered);
}

/// Resolve `$CARGO_HOME` (falling back to `~/.cargo`).
fn resolve_cargo_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CARGO_HOME")
        && !home.is_empty()
    {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir().map(|home| home.join(DOT_CARGO_DIR))
}

/// Subscribe to the cargo home directory (`$CARGO_HOME` or
/// `~/.cargo`) so edits to `~/.cargo/config.toml` reach the watcher
/// even when the configured include directories do not cover it.
pub(super) fn register_cargo_home_watch(
    watcher: &mut impl Watcher,
    registered_roots: &mut RegisteredRoots,
) {
    let Some(cargo_home) = resolve_cargo_home() else {
        return;
    };
    register_cargo_home_directory(watcher, registered_roots, &cargo_home);
}

pub(super) fn register_cargo_home_directory(
    watcher: &mut impl Watcher,
    registered_roots: &mut RegisteredRoots,
    cargo_home: &Path,
) {
    if !cargo_home.is_dir() || registered_roots.skips.skips_resolved(cargo_home) {
        return;
    }
    if registered_roots.watched.contains(cargo_home) {
        return;
    }
    if registered_roots.registration == WatchRegistration::RecursiveRoot
        && registered_roots
            .watched
            .iter()
            .any(|root| cargo_home.starts_with(root))
    {
        return;
    }
    match watcher.watch(cargo_home, registered_roots.registration.mode()) {
        Ok(()) => {
            registered_roots
                .watched
                .insert(AbsolutePath::from(cargo_home));
            tracing::trace!(
            target: PERF_LOG_TARGET,
            cargo_home = %cargo_home.display(),
            "watcher_cargo_home_registered"
            );
        },
        Err(err) => tracing::error!(
            cargo_home = %cargo_home.display(),
            error = %err,
            "watcher_cargo_home_registration_failed"
        ),
    }
}

#[cfg(test)]
#[path = "roots_repair_tests.rs"]
mod repair_tests;
