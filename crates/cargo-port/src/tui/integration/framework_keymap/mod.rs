//! Framework-side keymap scaffolding.
//!
//! The `tui_pane`-driven keymap path coexists with the legacy
//! `src/keymap.rs` path: the framework keymap owns targeted structural
//! lookups while broad key dispatch remains on the legacy path.
//!
//! Surface:
//!
//! - [`AppPaneId`]: every app-side pane id the framework keys on.
//! - [`NavAction`]: the framework-owned directional nav enum the [`Navigation`] singleton routes
//!   through.
//! - [`AppGlobalAction`]: app-extension globals scope. Currently ships a single placeholder variant
//!   ([`AppGlobalAction::Find`]); grows to cover the rest of the binary's non-framework globals.
//! - [`AppNavigation`] / [`PackagePane`]: the `Navigation` and `Pane` + `Shortcuts` impls the
//!   builder typestate requires.
//! - [`build_framework_keymap`]: assembles a [`tui_pane::Keymap<App>`] using the canonical builder
//!   chain. Called once at startup.

use std::rc::Rc;

use tui_pane::Action;
use tui_pane::AppContext;
use tui_pane::BarRegion;
use tui_pane::BarSlot;
use tui_pane::Bindings;
use tui_pane::Configuring;
use tui_pane::CopyLabel;
use tui_pane::CopyPayload;
use tui_pane::CopySelection;
use tui_pane::CopySelectionResult;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::Globals;
use tui_pane::KeyBind;
use tui_pane::KeyOutcome;
use tui_pane::Keymap;
use tui_pane::KeymapBuilder;
use tui_pane::KeymapError;
use tui_pane::KeymapUiContext;
use tui_pane::Mode;
use tui_pane::NavAction;
use tui_pane::Navigation;
use tui_pane::Pane;
use tui_pane::PaneFocusState;
use tui_pane::ShortcutState;
use tui_pane::Shortcuts;
use tui_pane::TabStop;
use tui_pane::TrackedItemKey;
use tui_pane::VimMode;
use tui_pane::Visibility;

use super::constants::CI_RUNS_TAB_ORDER;
use super::constants::CPU_TAB_ORDER;
use super::constants::GIT_TAB_ORDER;
use super::constants::KEYMAP_OVERLAY_PANE_ORDER;
use super::constants::LANG_TAB_ORDER;
use super::constants::LINTS_TAB_ORDER;
use super::constants::OUTPUT_TAB_ORDER;
use super::constants::PACKAGE_TAB_ORDER;
use super::constants::PROJECT_LIST_TAB_ORDER;
use super::constants::TARGETS_TAB_ORDER;
use crate::ci::OwnerRepo;
use crate::config::NavigationKeys;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::app::CargoPortToastAction;
use crate::tui::app::CiRunDisplayMode;
use crate::tui::finder;
use crate::tui::input;
use crate::tui::keymap::CiRunsAction;
use crate::tui::keymap::FinderAction;
use crate::tui::keymap::GitAction;
use crate::tui::keymap::LintsAction;
use crate::tui::keymap::OutputAction;
use crate::tui::keymap::PackageAction;
use crate::tui::keymap::ProjectListAction;
use crate::tui::keymap::TargetsAction;
use crate::tui::panes;
use crate::tui::panes::GitRow;
use crate::tui::panes::PackageRow;
use crate::tui::panes::PaneId;
use crate::tui::sccache;

mod app_context;
mod builder;
mod ci_runs_pane;
mod cpu_pane;
mod finder_pane;
mod git_pane;
mod lang_pane;
mod lints_pane;
mod navigation;
mod output_pane;
mod package_pane;
mod project_list_pane;
mod targets_pane;

pub use app_context::AppGlobalAction;
pub use app_context::AppPaneId;
use app_context::ci_runs_is_tabbable;
use app_context::cpu_is_tabbable;
use app_context::git_is_tabbable;
use app_context::lang_is_tabbable;
use app_context::lints_is_tabbable;
use app_context::output_is_tabbable;
use app_context::package_is_tabbable;
use app_context::project_list_is_tabbable;
use app_context::targets_is_tabbable;
pub use app_context::vim_mode_from_config;
pub use builder::build_framework_keymap;
pub use builder::owner_repo_key;
pub use builder::path_key;
pub use ci_runs_pane::CiRunsPane;
use cpu_pane::CpuPane;
pub use finder_pane::FinderPane;
pub use git_pane::GitPane;
use lang_pane::LangPane;
pub use lints_pane::LintsPane;
pub use navigation::AppNavigation;
pub use output_pane::OutputPane;
pub use package_pane::PackagePane;
pub use project_list_pane::ProjectListPane;
pub use targets_pane::TargetsPane;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

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

    #[test]
    fn app_pane_id_round_trips_to_legacy() {
        for (app_id, legacy) in [
            (AppPaneId::Package, PaneId::Package),
            (AppPaneId::Git, PaneId::Git),
            (AppPaneId::Output, PaneId::Output),
            (AppPaneId::Finder, PaneId::Finder),
        ] {
            assert_eq!(app_id.to_legacy(), legacy);
        }
    }

    #[test]
    fn ci_runs_branch_and_all_defaults() {
        let defaults = CiRunsPane::defaults().into_scope_map();

        assert_eq!(
            defaults.action_for(&tui_pane::KeyBind::from('b')),
            Some(CiRunsAction::ShowBranch),
        );
        assert_eq!(
            defaults.action_for(&tui_pane::KeyBind::from('a')),
            Some(CiRunsAction::ShowAll),
        );
        assert_eq!(defaults.action_for(&tui_pane::KeyBind::from('v')), None);
    }

    #[test]
    fn app_global_copy_defaults_to_y_without_terminal_copy_keys() {
        let defaults = AppGlobalAction::defaults().into_scope_map();

        assert_eq!(
            defaults.action_for(&tui_pane::KeyBind::from('y')),
            Some(AppGlobalAction::Copy),
        );
        assert_eq!(defaults.action_for(&tui_pane::KeyBind::ctrl('c')), None,);
        assert_eq!(
            defaults.action_for(&tui_pane::KeyBind::ctrl(tui_pane::KeyBind::shift('c'))),
            None,
        );
    }

    #[test]
    fn app_global_sccache_stats_defaults_to_shift_s() {
        let defaults = AppGlobalAction::defaults().into_scope_map();

        assert_eq!(
            defaults.action_for(&tui_pane::KeyBind::from('S')),
            Some(AppGlobalAction::SccacheStats),
        );
    }
}
