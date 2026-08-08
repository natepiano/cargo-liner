#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::path::PathBuf;

use super::constants::THEMES_DIRNAME;
use crate::config::CargoPortConfigurationPathResolution;

/// Which source supplies the themes directory for this lookup.
enum ThemesDirectoryResolutionSource {
    /// A thread-local test fixture supplies the complete themes directory.
    #[cfg(test)]
    TestSpecificDirectory(PathBuf),
    /// Cargo Port resolves `themes/` under its shared configuration root.
    SharedConfigurationRoot,
}

/// Compute the per-user themes directory:
/// `CARGO_PORT_CONFIG_DIR / "themes"`, or, when unset,
/// `dirs::config_dir() / "cargo-port" / "themes"`.
///
/// Returns `None` on platforms where the OS config dir can't be
/// resolved (extremely rare; same conservative behavior as
/// [`crate::config::config_path`]). Tests can override via
/// `set_themes_dir_override_for_test`.
#[must_use]
pub(crate) fn themes_dir() -> CargoPortConfigurationPathResolution {
    #[cfg(test)]
    let themes_directory_resolution_source = THEMES_DIR_OVERRIDE.with(|slot| {
        slot.borrow().clone().map_or(
            ThemesDirectoryResolutionSource::SharedConfigurationRoot,
            ThemesDirectoryResolutionSource::TestSpecificDirectory,
        )
    });
    #[cfg(not(test))]
    let themes_directory_resolution_source =
        ThemesDirectoryResolutionSource::SharedConfigurationRoot;

    resolve_themes_dir(
        themes_directory_resolution_source,
        crate::config::cargo_port_configuration_root(),
    )
}

fn resolve_themes_dir(
    themes_directory_resolution_source: ThemesDirectoryResolutionSource,
    cargo_port_configuration_root: CargoPortConfigurationPathResolution,
) -> CargoPortConfigurationPathResolution {
    match themes_directory_resolution_source {
        #[cfg(test)]
        ThemesDirectoryResolutionSource::TestSpecificDirectory(path) => {
            CargoPortConfigurationPathResolution::Resolved(path)
        },
        ThemesDirectoryResolutionSource::SharedConfigurationRoot => {
            cargo_port_configuration_root.child(THEMES_DIRNAME)
        },
    }
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
    fn test_specific_themes_dir_wins_over_the_shared_configuration_root() {
        let test_themes_dir = PathBuf::from("/test-fixture/themes");
        let shared_configuration_root =
            CargoPortConfigurationPathResolution::Resolved(PathBuf::from("/ambient/cargo-port"));

        assert_eq!(
            resolve_themes_dir(
                ThemesDirectoryResolutionSource::TestSpecificDirectory(test_themes_dir.clone()),
                shared_configuration_root,
            ),
            CargoPortConfigurationPathResolution::Resolved(test_themes_dir)
        );
    }

    #[test]
    fn themes_dir_override_routes_through_themes_dir() {
        let dir = std::env::temp_dir().join(format!(
            "cargo_port_themes_override_route_{}",
            std::process::id()
        ));
        let _guard = set_themes_dir_override_for_test(dir.clone());
        assert_eq!(
            themes_dir(),
            CargoPortConfigurationPathResolution::Resolved(dir)
        );
    }
}
