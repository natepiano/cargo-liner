use std::collections::HashMap;
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

/// Why a folder needs a non-recursive subscription.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WatchedFolderRole {
    Project,
    Discovery,
    Candidate,
    ConfigSource,
}

/// Whether a folder has a subscription, and the reason it is watched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Subscription {
    Unwatched,
    Watched(WatchedFolderRole),
}

#[derive(Clone, Copy)]
enum ConfigWatchScope {
    Folder,
    GitTree,
}

/// Configured discovery boundaries, known projects, and successful subscriptions.
#[derive(Clone, Default)]
pub(super) struct RegisteredRoots {
    dirs:           Vec<AbsolutePath>,
    project_roots:  HashSet<AbsolutePath>,
    config_sources: HashMap<AbsolutePath, ConfigWatchScope>,
    watched:        HashMap<AbsolutePath, WatchedFolderRole>,
    skips:          WatchSkipSet,
    registration:   WatchRegistration,
}

impl RegisteredRoots {
    pub(super) fn dirs(&self) -> &[AbsolutePath] { &self.dirs }

    pub(super) const fn registration(&self) -> WatchRegistration { self.registration }

    pub(super) fn role(&self, path: &Path) -> Subscription {
        self.watched
            .get(path)
            .copied()
            .map_or(Subscription::Unwatched, Subscription::Watched)
    }

    pub(super) fn event_role(&self, path: &Path) -> Subscription {
        match self.role(path) {
            Subscription::Unwatched => path
                .parent()
                .map_or(Subscription::Unwatched, |parent| self.role(parent)),
            Subscription::Watched(role) => Subscription::Watched(role),
        }
    }

    #[cfg(test)]
    pub(super) fn from_dirs(dirs: Vec<AbsolutePath>) -> Self {
        let watched = dirs
            .iter()
            .flat_map(|dir| watch_directories(dir, &WatchSkipSet::default()))
            .map(|path| (path, WatchedFolderRole::Project))
            .collect();
        Self {
            project_roots: dirs.iter().cloned().collect(),
            config_sources: HashMap::new(),
            dirs,
            watched,
            skips: WatchSkipSet::default(),
            registration: WatchRegistration::PerFolder,
        }
    }

    pub(super) fn covers(&self, path: &Path) -> bool {
        !self.skips(path)
            && match self.registration {
                WatchRegistration::RecursiveRoot => {
                    self.dirs.iter().any(|root| path.starts_with(root))
                },
                WatchRegistration::PerFolder => {
                    self.role(path) == Subscription::Watched(WatchedFolderRole::Project)
                },
            }
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
        if registered.watched.contains_key(dir) && !registered.dirs.contains(dir) {
            registered.dirs.push(dir.clone());
        }
    }
    (registered, failures)
}

pub(super) fn register_discovery_roots(
    watcher: &mut impl Watcher,
    watch_dirs: &[AbsolutePath],
    skips: WatchSkipSet,
) -> (RegisteredRoots, Vec<WatchRootRegistrationFailure>) {
    let mut registered = RegisteredRoots {
        dirs: watch_dirs.to_vec(),
        skips,
        ..RegisteredRoots::default()
    };
    let mut failures = Vec::new();
    for dir in watch_dirs {
        failures.extend(register_folder(
            watcher,
            &mut registered,
            dir,
            WatchedFolderRole::Discovery,
        ));
    }
    (registered, failures)
}

fn register_folder(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    dir: &AbsolutePath,
    role: WatchedFolderRole,
) -> Vec<WatchRootRegistrationFailure> {
    if registered.skips.skips_resolved(dir) {
        return Vec::new();
    }
    if !dir.is_dir() {
        return vec![WatchRootRegistrationFailure {
            dir:    dir.clone(),
            reason: WatchRootRegistrationFailureReason::NotADirectory,
        }];
    }
    if let Some(current) = registered.watched.get_mut(dir) {
        // A project supplies its own discovery and configuration coverage.
        // A discovery path must not be demoted by a transient candidate.
        *current = match (*current, role) {
            (WatchedFolderRole::Project, _) | (_, WatchedFolderRole::Project) => {
                WatchedFolderRole::Project
            },
            (WatchedFolderRole::Discovery, _) => WatchedFolderRole::Discovery,
            (_, WatchedFolderRole::Candidate) => *current,
            _ => role,
        };
        return Vec::new();
    }
    match watcher.watch(dir, RecursiveMode::NonRecursive) {
        Ok(()) => {
            registered.watched.insert(dir.clone(), role);
            Vec::new()
        },
        Err(err) => vec![WatchRootRegistrationFailure {
            dir:    dir.clone(),
            reason: WatchRootRegistrationFailureReason::Notify(err),
        }],
    }
}

pub(super) fn register_project_root(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    dir: &AbsolutePath,
) -> Vec<WatchRootRegistrationFailure> {
    if registered.skips.skips_resolved(dir) {
        return Vec::new();
    }
    if registered.registration == WatchRegistration::RecursiveRoot {
        let failures = register_recursive_root(watcher, registered, dir);
        if registered.watched.contains_key(dir) && !registered.dirs.contains(dir) {
            registered.dirs.push(dir.clone());
        }
        return failures;
    }
    registered.project_roots.insert(dir.clone());
    let mut failures = Vec::new();
    if let Some(boundary) = registered
        .dirs
        .iter()
        .filter(|root| dir.starts_with(root))
        .min_by_key(|root| root.components().count())
        .cloned()
    {
        let mut ancestors: Vec<_> = dir
            .ancestors()
            .skip(1)
            .take_while(|path| path.starts_with(&boundary))
            .map(AbsolutePath::from)
            .collect();
        ancestors.reverse();
        for parent in ancestors {
            failures.extend(register_folder(
                watcher,
                registered,
                &parent,
                WatchedFolderRole::Discovery,
            ));
        }
    }
    if !dir.is_dir() {
        failures.push(WatchRootRegistrationFailure {
            dir:    dir.clone(),
            reason: WatchRootRegistrationFailureReason::NotADirectory,
        });
        return failures;
    }
    failures.extend(register_directory_tree(
        watcher,
        registered,
        dir,
        WatchedFolderRole::Project,
    ));
    failures
}

fn register_recursive_root(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    dir: &AbsolutePath,
) -> Vec<WatchRootRegistrationFailure> {
    if !dir.is_dir() {
        return vec![WatchRootRegistrationFailure {
            dir:    dir.clone(),
            reason: WatchRootRegistrationFailureReason::NotADirectory,
        }];
    }
    let skips = registered.skips.clone();
    let mut failures = Vec::new();
    for (folder, mode) in registration_plan(dir, &skips, WatchRegistration::RecursiveRoot) {
        if registered.watched.contains_key(&folder) {
            continue;
        }
        match watcher.watch(&folder, mode) {
            Ok(()) => {
                registered
                    .watched
                    .insert(folder, WatchedFolderRole::Project);
            },
            Err(error) => failures.push(WatchRootRegistrationFailure {
                dir:    folder,
                reason: WatchRootRegistrationFailureReason::Notify(error),
            }),
        }
    }
    failures
}

fn register_directory_tree(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    dir: &AbsolutePath,
    role: WatchedFolderRole,
) -> Vec<WatchRootRegistrationFailure> {
    let mut failures = Vec::new();
    // Each parent receives its watch before the lazy walk reads its children.
    let skips = registered.skips.clone();
    for folder in directory_walk(dir, &skips) {
        failures.extend(register_folder(watcher, registered, &folder, role));
    }
    failures
}

pub(super) fn remove_directory_watches(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    path: &Path,
) {
    if registered.registration == WatchRegistration::RecursiveRoot {
        return;
    }
    let removed: Vec<_> = registered
        .watched
        .keys()
        .filter(|dir| dir.starts_with(path))
        .cloned()
        .collect();
    for dir in removed {
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
        let parent_role = path
            .parent()
            .map_or(Subscription::Unwatched, |parent| registered.role(parent));
        let old_role = registered.role(path);
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
        if !path.is_dir() || registered.skips.skips_resolved(path) {
            continue;
        }
        let folder = AbsolutePath::from(path.as_path());
        let role = match (old_role, parent_role) {
            (Subscription::Watched(WatchedFolderRole::Project), _)
            | (_, Subscription::Watched(WatchedFolderRole::Project)) => WatchedFolderRole::Project,
            _ if registered.config_sources.contains_key(&folder) => WatchedFolderRole::ConfigSource,
            (Subscription::Watched(WatchedFolderRole::ConfigSource), _) => {
                WatchedFolderRole::ConfigSource
            },
            (_, Subscription::Watched(WatchedFolderRole::ConfigSource))
                if registered.config_sources.iter().any(|(root, scope)| {
                    matches!(scope, ConfigWatchScope::GitTree) && path.starts_with(root)
                }) =>
            {
                WatchedFolderRole::ConfigSource
            },
            (Subscription::Watched(WatchedFolderRole::Discovery), _) => {
                WatchedFolderRole::Discovery
            },
            (_, Subscription::Watched(WatchedFolderRole::Discovery)) => {
                WatchedFolderRole::Candidate
            },
            _ => continue,
        };
        let failures = match role {
            WatchedFolderRole::Project => {
                register_directory_tree(watcher, registered, &folder, role)
            },
            WatchedFolderRole::ConfigSource
                if registered.config_sources.iter().any(|(root, scope)| {
                    matches!(scope, ConfigWatchScope::GitTree) && folder.starts_with(root)
                }) =>
            {
                register_directory_tree(watcher, registered, &folder, role)
            },
            WatchedFolderRole::ConfigSource
            | WatchedFolderRole::Discovery
            | WatchedFolderRole::Candidate => register_folder(watcher, registered, &folder, role),
        };
        log_failures(failures);
        // Renaming an ancestor removes its subscriptions but retains the scan's
        // known projects. Restore those paths when the ancestor comes back,
        // even when the ancestor itself is not a project.
        let projects: Vec<_> = registered
            .project_roots
            .iter()
            .filter(|project| project.starts_with(&folder))
            .cloned()
            .collect();
        for project in projects {
            log_failures(register_project_root(watcher, registered, &project));
        }
    }
}

fn log_failures(failures: Vec<WatchRootRegistrationFailure>) {
    for failure in failures {
        tracing::error!(dir = %failure.dir.display(), reason = %failure.reason, "watcher_folder_registration_failed");
    }
}

#[derive(Clone, Copy)]
pub(super) enum CandidateProbeResult {
    Project,
    NotProject,
}

pub(super) fn finish_candidate_probe(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    path: &AbsolutePath,
    result: CandidateProbeResult,
) {
    match result {
        CandidateProbeResult::Project => {
            log_failures(register_project_root(watcher, registered, path));
        },
        CandidateProbeResult::NotProject => {
            if registered.role(path) == Subscription::Watched(WatchedFolderRole::Candidate) {
                remove_directory_watches(watcher, registered, path);
            }
        },
    }
}

/// Inspect only immediate children of discovery folders; never traverse unknown trees.
pub(super) fn startup_reconciliation(
    registered: &RegisteredRoots,
    scan_projects: &[AbsolutePath],
) -> Vec<AbsolutePath> {
    let mut candidates: HashSet<_> = scan_projects
        .iter()
        .filter(|path| !path.is_dir())
        .cloned()
        .collect();
    for (folder, role) in &registered.watched {
        if *role != WatchedFolderRole::Discovery {
            continue;
        }
        candidates.insert(folder.clone());
        if let Ok(children) = std::fs::read_dir(folder) {
            for child in children.filter_map(Result::ok) {
                if child.file_type().is_ok_and(|kind| kind.is_dir())
                    && !registered.skips.skips(child.path().as_path())
                    && registered.role(&child.path())
                        != Subscription::Watched(WatchedFolderRole::Project)
                {
                    candidates.insert(AbsolutePath::from(child.path()));
                }
            }
        }
    }
    candidates.into_iter().collect()
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
        .keys()
        .filter(|dir| skips.skips_resolved(dir))
        .cloned()
        .collect();
    for dir in removed {
        let _ = watcher.unwatch(&dir);
        registered.watched.remove(&dir);
    }
    registered.skips = skips;
    for root in registered.project_roots.clone() {
        log_failures(register_directory_tree(
            watcher,
            registered,
            &root,
            WatchedFolderRole::Project,
        ));
    }
    for (source, scope) in registered.config_sources.clone() {
        if source.is_dir() {
            let failures = match scope {
                ConfigWatchScope::Folder => register_folder(
                    watcher,
                    registered,
                    &source,
                    WatchedFolderRole::ConfigSource,
                ),
                ConfigWatchScope::GitTree => register_directory_tree(
                    watcher,
                    registered,
                    &source,
                    WatchedFolderRole::ConfigSource,
                ),
            };
            log_failures(failures);
        }
    }
    register_cargo_home_watch(watcher, registered);
}

pub(super) fn register_project_sources(
    watcher: &mut impl Watcher,
    registered: &mut RegisteredRoots,
    project: &super::ProjectEntry,
) {
    if registered.registration == WatchRegistration::RecursiveRoot {
        return;
    }
    for ancestor in project.abs_path.ancestors() {
        let cargo_dir = ancestor.join(DOT_CARGO_DIR);
        let cargo_dir = AbsolutePath::from(cargo_dir);
        registered
            .config_sources
            .insert(cargo_dir.clone(), ConfigWatchScope::Folder);
        if cargo_dir.is_dir() {
            log_failures(register_folder(
                watcher,
                registered,
                &cargo_dir,
                WatchedFolderRole::ConfigSource,
            ));
        }
    }
    for git_dir in [&project.git_dir, &project.common_git_dir]
        .into_iter()
        .flatten()
    {
        if registered.role(git_dir) != Subscription::Watched(WatchedFolderRole::Project) {
            registered
                .config_sources
                .insert(git_dir.clone(), ConfigWatchScope::GitTree);
            log_failures(register_directory_tree(
                watcher,
                registered,
                git_dir,
                WatchedFolderRole::ConfigSource,
            ));
        }
    }
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
    if registered_roots.registration == WatchRegistration::RecursiveRoot {
        if !cargo_home.is_dir()
            || registered_roots.skips.skips_resolved(cargo_home)
            || registered_roots
                .watched
                .keys()
                .any(|root| cargo_home.starts_with(root))
        {
            return;
        }
        match watcher.watch(cargo_home, registered_roots.registration.mode()) {
            Ok(()) => {
                registered_roots.watched.insert(
                    AbsolutePath::from(cargo_home),
                    WatchedFolderRole::ConfigSource,
                );
            },
            Err(error) => {
                tracing::error!(cargo_home = %cargo_home.display(), %error, "watcher_cargo_home_registration_failed");
            },
        }
        return;
    }
    registered_roots
        .config_sources
        .insert(AbsolutePath::from(cargo_home), ConfigWatchScope::Folder);
    if cargo_home.is_dir() {
        log_failures(register_folder(
            watcher,
            registered_roots,
            &AbsolutePath::from(cargo_home),
            WatchedFolderRole::ConfigSource,
        ));
    }
}

#[cfg(test)]
#[path = "cargo_home_watch_tests.rs"]
mod folder_tests;
