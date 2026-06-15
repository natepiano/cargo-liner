//! Filesystem loader for user theme directories.
//!
//! Scans a directory for `*.toml` files, parses each as a
//! [`ThemeFamily`], and registers every contained variant on a
//! [`ThemeRegistry`]. Files that fail to read or parse are recorded
//! via [`ThemeRegistry::record_failed_file`] so the UI can surface the
//! error.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use super::ThemeFamily;
use super::ThemeId;
use super::ThemeLoadError;
use super::ThemeRegistry;
use super::ThemeVariant;

struct FailedFile {
    path:  PathBuf,
    error: ThemeLoadError,
}

impl ThemeRegistry {
    /// Build a registry seeded with the compiled-in built-ins and
    /// extended with every variant registered from `dir` (if `Some`).
    ///
    /// Returns the assembled registry; the caller installs or replaces
    /// it via [`crate::install_theme_state`] / [`crate::replace_registry`].
    #[must_use]
    pub fn from_dir_with_builtins(dir: Option<&Path>) -> Self {
        let mut registry = Self::new_with_builtins();
        let Some(dir) = dir else {
            return registry;
        };
        let (loaded, failed) = scan_themes_dir(dir);
        for family in loaded {
            for variant_file in family.variants {
                let id = ThemeId::new(variant_file.name.clone());
                let appearance = variant_file.appearance;
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
}

/// Read every `*.toml` under `dir` in sorted ASCII filename order.
///
/// Sorted iteration is what makes the "later file overrides earlier"
/// tie-break deterministic across runs. Returns the parsed families
/// alongside any files that failed to parse; neither is fatal, and the
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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::io::Write;
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::theme::BUILTIN_DARK_NAME;
    use crate::theme::BUILTIN_LIGHT_NAME;

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "tui_pane_theme_loader_{label}_{n}_{}",
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

    const MINIMAL_DARK_FAMILY: &str = include_str!("../../themes/default_dark.toml");

    #[test]
    fn missing_directory_returns_only_builtins() {
        let registry =
            ThemeRegistry::from_dir_with_builtins(Some(Path::new("/definitely/not/a/dir/xyzzy")));
        assert_eq!(registry.len(), 4);
        assert!(registry.find(&ThemeId::new(BUILTIN_DARK_NAME)).is_some());
        assert!(registry.find(&ThemeId::new(BUILTIN_LIGHT_NAME)).is_some());
        assert!(registry.status().failed_files.is_empty());
    }

    #[test]
    fn no_directory_argument_returns_only_builtins() {
        let registry = ThemeRegistry::from_dir_with_builtins(None);
        assert_eq!(registry.len(), 4);
    }

    #[test]
    fn user_variant_overrides_builtin_with_same_name() {
        let dir = temp_dir("override");
        write_file(&dir.join("override.toml"), MINIMAL_DARK_FAMILY);
        let registry = ThemeRegistry::from_dir_with_builtins(Some(&dir));
        assert_eq!(registry.len(), 4, "override must replace in place");
        assert_eq!(
            registry.status().overridden,
            vec![ThemeId::new(BUILTIN_DARK_NAME)]
        );
    }

    #[test]
    fn parse_failure_is_recorded_not_fatal() {
        let dir = temp_dir("badparse");
        write_file(&dir.join("bad.toml"), "this is not = valid toml [\n");
        let registry = ThemeRegistry::from_dir_with_builtins(Some(&dir));
        assert_eq!(registry.len(), 4, "built-ins survive a parse error");
        assert_eq!(registry.status().failed_files.len(), 1);
        let (path, err) = &registry.status().failed_files[0];
        assert!(path.ends_with("bad.toml"));
        assert!(err.message().contains("parse error"));
    }
}
