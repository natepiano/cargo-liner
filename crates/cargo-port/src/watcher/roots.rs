use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::path::PathBuf;

use notify::Error;
use notify::RecursiveMode;
use notify::Watcher;

use crate::project::AbsolutePath;

/// Witness that a set of watch roots was actually registered with the
/// underlying `notify::Watcher`. Constructed only by
/// [`register_watch_roots`]; the watcher loop accepts a
/// `&RegisteredRoots` instead of a `&[AbsolutePath]` so the
/// previously-representable state where the watcher loop runs but
/// silently dropped a watch root is no longer constructible.
pub(super) struct RegisteredRoots {
    dirs: Vec<AbsolutePath>,
}

impl RegisteredRoots {
    pub(super) fn dirs(&self) -> &[AbsolutePath] { &self.dirs }

    #[cfg(test)]
    pub(super) const fn from_dirs(dirs: Vec<AbsolutePath>) -> Self { Self { dirs } }

    /// True when `path` is equal to or descends from any registered
    /// root. Used to suppress redundant per-project ancestor watches
    /// that would re-register an already-recursively-watched dir as
    /// `NonRecursive` — on macOS `FSEvents` this changes the mode for
    /// the path and silently drops subsequent recursive events.
    pub(super) fn covers(&self, path: &Path) -> bool {
        self.dirs.iter().any(|root| path.starts_with(root))
    }
}

impl Default for RegisteredRoots {
    /// An empty registered set — trivially consistent (we are
    /// watching nothing, and we claim to be watching nothing). Used
    /// by tests that exercise watcher logic without exercising
    /// registration.
    fn default() -> Self { Self { dirs: Vec::new() } }
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

/// Try to register every entry in `watch_dirs` with the underlying
/// [`Watcher`]. Returns the witness for the subset that succeeded plus
/// the per-root failures. The caller must visibly handle the failures
/// (the reason this function returns a tuple instead of `Result` is
/// that running with a partial root set is still better than not
/// running at all — but the caller cannot pretend the failures don't
/// exist, because they are returned by value).
pub(super) fn register_watch_roots(
    watcher: &mut impl Watcher,
    watch_dirs: &[AbsolutePath],
) -> (RegisteredRoots, Vec<WatchRootRegistrationFailure>) {
    let mut registered = Vec::with_capacity(watch_dirs.len());
    let mut failures = Vec::new();
    for dir in watch_dirs {
        if !dir.is_dir() {
            failures.push(WatchRootRegistrationFailure {
                dir:    dir.clone(),
                reason: WatchRootRegistrationFailureReason::NotADirectory,
            });
            continue;
        }
        match watcher.watch(dir, RecursiveMode::Recursive) {
            Ok(()) => registered.push(dir.clone()),
            Err(err) => failures.push(WatchRootRegistrationFailure {
                dir:    dir.clone(),
                reason: WatchRootRegistrationFailureReason::Notify(err),
            }),
        }
    }
    (RegisteredRoots { dirs: registered }, failures)
}

/// Resolve `$CARGO_HOME` (falling back to `~/.cargo`).
fn resolve_cargo_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CARGO_HOME")
        && !home.is_empty()
    {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir().map(|home| home.join(".cargo"))
}

/// Subscribe to the cargo home directory (`$CARGO_HOME` or
/// `~/.cargo`) so edits to `~/.cargo/config.toml` reach the watcher
/// even when the user's recursive `include_dirs` don't cover it.
/// Skipped when the cargo home is already inside one of the recursive
/// roots — registering it again as `NonRecursive` would clobber the
/// recursive subscription on macOS `FSEvents`.
pub(super) fn register_cargo_home_watch(
    watcher: &mut impl Watcher,
    registered_roots: &RegisteredRoots,
) {
    let Some(cargo_home) = resolve_cargo_home() else {
        return;
    };
    if !cargo_home.is_dir() {
        return;
    }
    if registered_roots.covers(cargo_home.as_path()) {
        return;
    }
    match watcher.watch(cargo_home.as_path(), RecursiveMode::NonRecursive) {
        Ok(()) => tracing::info!(
            cargo_home = %cargo_home.display(),
            "watcher_cargo_home_registered"
        ),
        Err(err) => tracing::error!(
            cargo_home = %cargo_home.display(),
            error = %err,
            "watcher_cargo_home_registration_failed"
        ),
    }
}
