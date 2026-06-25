use super::AbsolutePath;
use super::App;
use super::AppGlobalAction;
use super::AppNavigation;
use super::CiRunsPane;
use super::Configuring;
use super::CpuPane;
use super::FinderPane;
use super::Framework;
use super::GitPane;
use super::Keymap;
use super::KeymapBuilder;
use super::KeymapError;
use super::LangPane;
use super::LintsPane;
use super::OutputPane;
use super::OwnerRepo;
use super::PackagePane;
use super::ProjectListPane;
use super::TargetsPane;
use super::TrackedItemKey;

/// Assemble the framework keymap from a configured builder. Called
/// once during App construction after the builder has loaded the
/// production keymap TOML, if any. Errors propagate so the caller can
/// surface them through the existing keymap-diagnostics toast
/// plumbing.
///
/// Built in [`ignore_unknown_entries`](tui_pane::KeymapBuilder::ignore_unknown_entries)
/// mode: a binding for an action or scope that no longer exists (a
/// stale keymap from an older version) is skipped rather than failing
/// the build. The dropped entries are recorded on the returned keymap
/// — see [`Keymap::unknown_warnings`] — for the caller to surface.
pub fn build_framework_keymap(
    builder: KeymapBuilder<App, Configuring>,
    framework: &mut Framework<App>,
) -> Result<Keymap<App>, KeymapError> {
    builder
        .ignore_unknown_entries()
        .dismiss_fallback(dismiss_fallback)
        .register_navigation::<AppNavigation>()?
        .register_globals::<AppGlobalAction>()?
        .register_overlay()?
        .register::<ProjectListPane>(ProjectListPane)
        .register_copy_selection::<ProjectListPane>()
        .register::<PackagePane>(PackagePane)
        .register_copy_selection::<PackagePane>()
        .register_pane::<LangPane>()
        .register_pane::<CpuPane>()
        .register::<GitPane>(GitPane)
        .register_copy_selection::<GitPane>()
        .register::<TargetsPane>(TargetsPane)
        .register_copy_selection::<TargetsPane>()
        .register::<LintsPane>(LintsPane)
        .register_copy_selection::<LintsPane>()
        .register::<CiRunsPane>(CiRunsPane)
        .register_copy_selection::<CiRunsPane>()
        .register::<OutputPane>(OutputPane)
        .register_copy_selection::<OutputPane>()
        .register::<FinderPane>(FinderPane)
        .build_into(framework)
}

fn dismiss_fallback(app: &mut App) -> bool {
    let Some(target) = app.focused_dismiss_target() else {
        return false;
    };
    app.dismiss(target);
    true
}

pub fn path_key(path: &AbsolutePath) -> TrackedItemKey { TrackedItemKey::new(path.to_string()) }

pub fn owner_repo_key(repo: &OwnerRepo) -> TrackedItemKey { TrackedItemKey::new(repo.to_string()) }

#[cfg(test)]
mod tests {
    use super::owner_repo_key;
    use super::path_key;
    use crate::ci::OwnerRepo;
    use crate::project::AbsolutePath;

    #[test]
    fn path_key_uses_cargo_port_absolute_path_string() {
        let path = AbsolutePath::from("/tmp/cargo-port");
        let expected = crate::project::normalize_test_path(std::path::Path::new("/tmp/cargo-port"));

        assert_eq!(path_key(&path).as_str(), expected.display().to_string());
    }

    #[test]
    fn owner_repo_key_uses_cargo_port_owner_repo_string() {
        let repo = OwnerRepo::new("natepiano", "cargo-port");

        assert_eq!(owner_repo_key(&repo).as_str(), "natepiano/cargo-port");
    }
}
