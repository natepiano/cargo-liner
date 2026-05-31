use toml::Table;

use super::overlay;
use crate::AppContext;
use crate::Bindings;
use crate::KeySequence;
use crate::KeymapError;
use crate::NavAction;
use crate::Shortcuts;
use crate::VimMode;
use crate::keymap::nav_action;

/// Apply TOML and vim overlay onto a pane's defaults.
///
/// Returns `Err(KeymapError)` for overlay failures; the per-state
/// `register::<P>` wrappers swallow that into a deferred record (see
/// `insert_pane`) so the chain stays `Self`-returning. The helper is
/// `Result`-typed so the loader can widen its error pathway later
/// without changing this signature.
pub(super) fn build_pane_bindings<Ctx: AppContext + 'static, P: Shortcuts<Ctx>>(
    toml_table: Option<&Table>,
    vim_mode: VimMode,
    unknown: Option<&mut Vec<String>>,
) -> Result<Bindings<P::Actions>, KeymapError> {
    let scope_name = <P as Shortcuts<Ctx>>::SCOPE_NAME;
    let mut bindings =
        overlay::apply_toml_overlay::<P::Actions>(scope_name, P::defaults(), toml_table, unknown)?;
    if matches!(vim_mode, VimMode::Enabled) {
        for (action, key) in P::vim_extras() {
            let sequence = KeySequence::from(*key);
            if !bindings.has_key(&sequence) {
                bindings.bind(sequence, *action);
            }
        }
    }
    overlay::check_cross_action_collision(scope_name, &bindings)?;
    Ok(bindings)
}

/// Append vim navigation extras to the navigation scope. Skips any
/// sequence already bound to another action.
pub(super) fn apply_vim_navigation_extras(bindings: &mut Bindings<NavAction>) {
    for (key, action) in nav_action::vim_letter_extras() {
        if !bindings.has_key(&key) {
            bindings.bind(key, action);
        }
    }
}

pub(super) fn reserved_vim_navigation_keys() -> Vec<KeySequence> {
    nav_action::vim_letter_extras()
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

pub(super) fn check_reserved_vim_navigation_keys<A: crate::Action>(
    scope_name: &str,
    bindings: &Bindings<A>,
    reserved_keys: &[KeySequence],
) -> Result<(), KeymapError> {
    for (key, action) in bindings.entries() {
        if let Some(reserved) = reserved_keys
            .iter()
            .find(|reserved| keys_conflict(key, reserved))
        {
            return Err(KeymapError::CrossScopeVimCollision {
                scope:          scope_name.to_string(),
                action:         action.toml_key().to_string(),
                key:            key.display(),
                navigation_key: reserved.display(),
            });
        }
    }
    Ok(())
}

fn keys_conflict(left: &KeySequence, right: &KeySequence) -> bool {
    left == right || left.starts_with_strict(right.keys()) || right.starts_with_strict(left.keys())
}
