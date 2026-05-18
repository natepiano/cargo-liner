//! User-themes scan, registry assembly, and on-disk watch primitive.
//!
//! Phase 2 owns the cargo-port-side of theming: where on disk to look
//! for user themes (the app-specific config path layout), how to read
//! and parse each `*.toml`, and how to detect changes between ticks
//! so the main loop can hot-reload. The framework-side
//! [`tui_pane::ThemeRegistry`] only knows about variants — not files.

#[cfg(test)]
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use tui_pane::Appearance;
use tui_pane::ThemeFamily;
use tui_pane::ThemeId;
use tui_pane::ThemeLoadError;
use tui_pane::ThemeRegistry;
use tui_pane::ThemeVariant;

use crate::constants::APP_NAME;

const THEMES_DIRNAME: &str = "themes";

/// Compute the per-user themes directory:
/// `dirs::config_dir() / "cargo-port" / "themes"`.
///
/// Returns `None` on platforms where the OS config dir can't be
/// resolved (extremely rare; same conservative behavior as
/// [`crate::config::config_path`]). Tests can override via
/// [`set_themes_dir_override_for_test`].
#[must_use]
pub(crate) fn themes_dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = THEMES_DIR_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return Some(path);
    }
    dirs::config_dir().map(|d| d.join(APP_NAME).join(THEMES_DIRNAME))
}

/// One file that failed to load — captured so the caller can record
/// it in [`ThemeRegistry::record_failed_file`] and toast the user.
struct FailedFile {
    path:  PathBuf,
    error: ThemeLoadError,
}

/// Read every `*.toml` under `dir` in sorted ASCII filename order.
///
/// Sorted iteration is what makes the "later file overrides earlier"
/// tie-break deterministic across runs. Returns the parsed families
/// alongside any files that failed to parse — neither is fatal; the
/// caller decides whether to toast or just log.
fn scan_themes_dir(dir: &Path) -> (Vec<ThemeFamily>, Vec<FailedFile>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return (Vec::new(), Vec::new());
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
        })
        .collect();
    paths.sort();

    let mut loaded = Vec::with_capacity(paths.len());
    let mut failed = Vec::new();
    for path in paths {
        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<ThemeFamily>(&contents) {
                Ok(family) => loaded.push(family),
                Err(err) => failed.push(FailedFile {
                    path,
                    error: ThemeLoadError::new(format!("parse error: {err}")),
                }),
            },
            Err(err) => failed.push(FailedFile {
                path,
                error: ThemeLoadError::new(format!("read error: {err}")),
            }),
        }
    }
    (loaded, failed)
}

/// Build a [`ThemeRegistry`] seeded with the compiled-in built-ins
/// and extended with every variant registered from `dir` (if
/// `Some`).
///
/// Returns the assembled registry; the caller installs or replaces
/// it via [`tui_pane::install_theme_state`] / [`tui_pane::replace_registry`].
#[must_use]
pub(crate) fn build_user_registry(dir: Option<&Path>) -> ThemeRegistry {
    let mut registry = ThemeRegistry::new_with_builtins();
    let Some(dir) = dir else {
        return registry;
    };
    let (loaded, failed) = scan_themes_dir(dir);
    for family in loaded {
        for variant_file in family.variants {
            let id = ThemeId::new(variant_file.name.clone());
            let appearance: Appearance = variant_file.appearance;
            let theme = variant_file.into_theme();
            registry.register(ThemeVariant {
                id,
                appearance,
                theme,
            });
        }
    }
    for FailedFile { path, error } in failed {
        registry.record_failed_file(path, error);
    }
    registry
}

/// Polled directory-change detector for `themes/*.toml`.
///
/// Mirrors [`tui_pane::WatchedFile`] in spirit, but watches a *set*
/// of files. The fingerprint hashes each `*.toml` entry's filename +
/// modified time + length; any addition, removal, or content change
/// flips the fingerprint so [`Self::take_change`] reports a delta on
/// the next tick.
pub(crate) struct ThemesWatch {
    dir:         Option<PathBuf>,
    fingerprint: u64,
}

impl ThemesWatch {
    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        let fingerprint = dir.as_deref().map_or(0, directory_fingerprint);
        Self { dir, fingerprint }
    }

    pub(crate) fn dir(&self) -> Option<&Path> { self.dir.as_deref() }

    /// Return `Some(dir)` if the themes directory's fingerprint has
    /// changed since the last check, updating the cached fingerprint
    /// before returning. Polled per-tick from the main loop.
    pub(crate) fn take_change(&mut self) -> Option<&Path> {
        let dir = self.dir.as_deref()?;
        let current = directory_fingerprint(dir);
        if current == self.fingerprint {
            return None;
        }
        self.fingerprint = current;
        Some(dir)
    }
}

fn directory_fingerprint(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut items: Vec<(String, u64, Option<SystemTime>)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
            {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().into_owned();
            let metadata = fs::metadata(&path).ok()?;
            let modified = metadata.modified().ok();
            Some((name, metadata.len(), modified))
        })
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = DefaultHasher::new();
    items.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
thread_local! {
    static THEMES_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const {
        RefCell::new(None)
    };
}

/// Test-only override for the themes directory.
#[cfg(test)]
pub(crate) struct ThemesDirOverrideGuard {
    previous: Option<PathBuf>,
}

#[cfg(test)]
impl Drop for ThemesDirOverrideGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        THEMES_DIR_OVERRIDE.with(|slot| {
            *slot.borrow_mut() = previous;
        });
    }
}

/// Point [`themes_dir`] at `path` for the duration of the returned
/// guard. Tests use this to point the scan at a temp directory.
#[cfg(test)]
pub(crate) fn set_themes_dir_override_for_test(path: PathBuf) -> ThemesDirOverrideGuard {
    let previous = THEMES_DIR_OVERRIDE.with(|slot| slot.replace(Some(path)));
    ThemesDirOverrideGuard { previous }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::io::Write;
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering;

    use tui_pane::BUILTIN_DARK_NAME;
    use tui_pane::BUILTIN_LIGHT_NAME;

    use super::*;

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "cargo_port_themes_{label}_{n}_{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp themes dir");
        dir
    }

    fn write_file(path: &Path, contents: &str) {
        let mut f = fs::File::create(path).expect("create temp file");
        f.write_all(contents.as_bytes()).expect("write temp file");
        f.sync_all().expect("sync temp file");
    }

    const MINIMAL_DARK_FAMILY: &str = include_str!("../../tui_pane/themes/default_dark.toml");

    #[test]
    fn missing_directory_returns_only_builtins() {
        let registry = build_user_registry(Some(Path::new("/definitely/not/a/dir/xyzzy")));
        assert_eq!(registry.len(), 2);
        assert!(registry.find(&ThemeId::new(BUILTIN_DARK_NAME)).is_some());
        assert!(registry.find(&ThemeId::new(BUILTIN_LIGHT_NAME)).is_some());
        assert!(registry.status().failed_files.is_empty());
    }

    #[test]
    fn no_directory_argument_returns_only_builtins() {
        let registry = build_user_registry(None);
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn user_variant_overrides_builtin_with_same_name() {
        let dir = temp_dir("override");
        write_file(&dir.join("override.toml"), MINIMAL_DARK_FAMILY);
        let registry = build_user_registry(Some(&dir));
        assert_eq!(registry.len(), 2, "override must replace in place");
        assert_eq!(
            registry.status().overridden,
            vec![ThemeId::new(BUILTIN_DARK_NAME)]
        );
    }

    #[test]
    fn parse_failure_is_recorded_not_fatal() {
        let dir = temp_dir("badparse");
        write_file(&dir.join("bad.toml"), "this is not = valid toml [\n");
        let registry = build_user_registry(Some(&dir));
        assert_eq!(registry.len(), 2, "built-ins survive a parse error");
        assert_eq!(registry.status().failed_files.len(), 1);
        let (path, err) = &registry.status().failed_files[0];
        assert!(path.ends_with("bad.toml"));
        assert!(err.message().contains("parse error"));
    }

    #[test]
    fn themes_watch_reports_initial_no_change() {
        let dir = temp_dir("watch_initial");
        write_file(&dir.join("a.toml"), MINIMAL_DARK_FAMILY);
        let mut watch = ThemesWatch::new(Some(dir));
        assert!(
            watch.take_change().is_none(),
            "first call should not see a change"
        );
    }

    #[test]
    fn themes_watch_detects_new_file() {
        let dir = temp_dir("watch_new");
        let mut watch = ThemesWatch::new(Some(dir.clone()));
        assert!(watch.take_change().is_none());
        write_file(&dir.join("new.toml"), MINIMAL_DARK_FAMILY);
        assert!(watch.take_change().is_some(), "addition should fire");
        assert!(
            watch.take_change().is_none(),
            "second call should see no change"
        );
    }

    #[test]
    fn themes_watch_ignores_non_toml() {
        let dir = temp_dir("watch_nontoml");
        let mut watch = ThemesWatch::new(Some(dir.clone()));
        write_file(&dir.join("notes.md"), "ignore me");
        assert!(
            watch.take_change().is_none(),
            "non-toml additions should not fire"
        );
    }

    #[test]
    fn themes_dir_override_routes_through_themes_dir() {
        let dir = temp_dir("override_route");
        let _guard = set_themes_dir_override_for_test(dir.clone());
        let resolved = themes_dir().expect("override returns Some");
        assert_eq!(resolved, dir);
    }
}
