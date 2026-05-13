use toml::Table;

use super::overlay::apply_toml_overlay;
use crate::AppContext;
use crate::Bindings;
use crate::KeyBind;
use crate::KeymapError;
use crate::Navigation;
use crate::Shortcuts;
use crate::VimMode;

/// Apply TOML and vim overlay onto a pane's defaults.
///
/// Returns `Err(KeymapError)` for overlay failures; the per-state
/// `register::<P>` wrappers swallow that into a deferred record (see
/// `insert_pane`) so the chain stays `Self`-returning. Phase 10
/// silently propagates overlay failures via `build()` once we widen
/// the error pathway; today the helper is `Result`-typed in
/// preparation.
pub(super) fn build_pane_bindings<Ctx: AppContext + 'static, P: Shortcuts<Ctx>>(
    toml_table: Option<&Table>,
    vim_mode: VimMode,
) -> Result<Bindings<P::Actions>, KeymapError> {
    let scope_name = <P as Shortcuts<Ctx>>::SCOPE_NAME;
    let mut bindings = apply_toml_overlay::<P::Actions>(scope_name, P::defaults(), toml_table)?;
    if matches!(vim_mode, VimMode::Enabled) {
        for (action, key) in P::vim_extras() {
            if !bindings.has_key(key) {
                bindings.bind(*key, *action);
            }
        }
    }
    Ok(bindings)
}

/// Append vim navigation extras (`h` / `j` / `k` / `l`) to
/// `Navigation::LEFT` / `DOWN` / `UP` / `RIGHT`. Skips any letter
/// already bound to a different action on the full [`KeyBind`] (code
/// + mods).
pub(super) fn apply_vim_navigation_extras<Ctx: AppContext + 'static, N: Navigation<Ctx>>(
    bindings: &mut Bindings<N::Actions>,
) {
    let pairs: [(char, N::Actions); 4] = [
        ('h', <N as Navigation<Ctx>>::LEFT),
        ('j', <N as Navigation<Ctx>>::DOWN),
        ('k', <N as Navigation<Ctx>>::UP),
        ('l', <N as Navigation<Ctx>>::RIGHT),
    ];
    for (c, action) in pairs {
        let key = KeyBind::from(c);
        if !bindings.has_key(&key) {
            bindings.bind(key, action);
        }
    }
}
