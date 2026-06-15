#[cfg(test)]
use std::cell::RefCell;
use std::path::PathBuf;

use super::constants::THEMES_DIRNAME;
use crate::constants::APP_NAME;

/// Compute the per-user themes directory:
/// `dirs::config_dir() / "cargo-port" / "themes"`.
///
/// Returns `None` on platforms where the OS config dir can't be
/// resolved (extremely rare; same conservative behavior as
/// [`crate::config::config_path`]). Tests can override via
/// `set_themes_dir_override_for_test`.
#[must_use]
pub(crate) fn themes_dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = THEMES_DIR_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return Some(path);
    }
    dirs::config_dir().map(|d| d.join(APP_NAME).join(THEMES_DIRNAME))
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
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn themes_dir_override_routes_through_themes_dir() {
        let dir = std::env::temp_dir().join(format!(
            "cargo_port_themes_override_route_{}",
            std::process::id()
        ));
        let _guard = set_themes_dir_override_for_test(dir.clone());
        let resolved = themes_dir().expect("override returns Some");
        assert_eq!(resolved, dir);
    }
}
