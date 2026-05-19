use toml::Table;

use super::overlay;
use crate::AppContext;
use crate::Bindings;
use crate::KeyBind;
use crate::KeySequence;
use crate::KeymapError;
use crate::Navigation;
use crate::Shortcuts;
use crate::VimMode;

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
) -> Result<Bindings<P::Actions>, KeymapError> {
    let scope_name = <P as Shortcuts<Ctx>>::SCOPE_NAME;
    let mut bindings = P::defaults();
    if matches!(vim_mode, VimMode::Enabled) {
        for (action, key) in P::vim_extras() {
            let sequence = KeySequence::from(*key);
            if !bindings.has_key(&sequence) {
                bindings.bind(sequence, *action);
            }
        }
    }
    overlay::apply_toml_overlay::<P::Actions>(scope_name, bindings, toml_table)
}

/// Append vim navigation extras to the navigation scope. Skips any
/// sequence already bound to another action.
pub(super) fn apply_vim_navigation_extras<Ctx: AppContext + 'static, N: Navigation<Ctx>>(
    bindings: &mut Bindings<N::Actions>,
) {
    for (key, action) in vim_navigation_extras::<Ctx, N>() {
        if !bindings.has_key(&key) {
            bindings.bind(key, action);
        }
    }
}

pub(super) fn reserved_vim_navigation_keys<Ctx: AppContext + 'static, N: Navigation<Ctx>>()
-> Vec<KeySequence> {
    vim_navigation_extras::<Ctx, N>()
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

fn vim_navigation_extras<Ctx: AppContext + 'static, N: Navigation<Ctx>>()
-> [(KeySequence, N::Actions); 10] {
    let pairs: [(KeySequence, N::Actions); 10] = [
        (KeySequence::from('h'), <N as Navigation<Ctx>>::LEFT),
        (KeySequence::from('j'), <N as Navigation<Ctx>>::DOWN),
        (KeySequence::from('k'), <N as Navigation<Ctx>>::UP),
        (KeySequence::from('l'), <N as Navigation<Ctx>>::RIGHT),
        (
            KeySequence::new(vec![KeyBind::from('g'), KeyBind::from('g')]),
            <N as Navigation<Ctx>>::HOME,
        ),
        (KeySequence::from('G'), <N as Navigation<Ctx>>::END),
        (
            KeySequence::from(KeyBind::ctrl('u')),
            <N as Navigation<Ctx>>::HALF_PAGE_UP,
        ),
        (
            KeySequence::from(KeyBind::ctrl('d')),
            <N as Navigation<Ctx>>::HALF_PAGE_DOWN,
        ),
        (
            KeySequence::from(KeyBind::ctrl('b')),
            <N as Navigation<Ctx>>::PAGE_UP,
        ),
        (
            KeySequence::from(KeyBind::ctrl('f')),
            <N as Navigation<Ctx>>::PAGE_DOWN,
        ),
    ];
    pairs
}
