mod actions;
mod load;
mod resolved;
mod scope_map;

pub(crate) use actions::CiRunsAction;
pub(crate) use actions::FinderAction;
pub(crate) use actions::GitAction;
pub(crate) use actions::LintsAction;
pub(crate) use actions::OutputAction;
pub(crate) use actions::PackageAction;
pub(crate) use actions::ProjectListAction;
pub(crate) use actions::TargetsAction;
use crossterm::event::KeyCode;
pub(crate) use load::KeymapError;
pub(crate) use load::KeymapErrorReason;
pub(crate) use load::keymap_path;
pub(crate) use load::load_keymap;
pub(crate) use load::load_keymap_from_str;
pub(crate) use load::migrate_removed_action_keys_on_disk;
#[cfg(test)]
pub(crate) use load::override_keymap_path_for_test;
#[cfg(test)]
pub(crate) use load::override_keymap_path_for_test_if_absent;
pub(crate) use resolved::ResolvedKeymap;
pub(crate) use scope_map::ScopeMap;
pub(crate) use tui_pane::KeyBind;

/// Cargo-port's [`tui_pane::KeyBind::canonicalize_code`] hook: collapses
/// `+` and `=` so a user binding to either key matches presses of
/// both. Apply at every load-time `KeyBind` construction and at the
/// crossterm-event dispatch boundary so storage and lookup agree.
pub(crate) const fn canonical_code(code: KeyCode) -> KeyCode {
    match code {
        KeyCode::Char('+') => KeyCode::Char('='),
        other => other,
    }
}
