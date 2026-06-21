use super::Action;
use super::GlobalAction;
use super::Keymap;
use super::KeymapHelpRow;
use super::KeymapHelpRowKind;
use crate::AppContext;

/// Build one [`KeymapHelpRow`] for a framework global action. Free
/// fn so [`Keymap::keymap_help_rows`] can iterate without
/// monomorphizing.
pub(super) fn framework_global_help_row<Ctx: AppContext + 'static>(
    section: &'static str,
    action: GlobalAction,
    keymap: &Keymap<Ctx>,
) -> KeymapHelpRow {
    KeymapHelpRow {
        section,
        scope: "global",
        action: action.toml_key(),
        description: action.description(),
        bind: keymap.framework_globals.key_for(action).cloned(),
        row_kind: KeymapHelpRowKind::Action,
    }
}
