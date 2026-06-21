use core::marker::PhantomData;

use super::Configuring;
use super::KeymapBuilder;
use super::Registering;
use crate::AppContext;

/// Move from the [`Configuring`] type to `Registering`. Field-by-
/// field move so the typestate transition is purely a type change at
/// runtime.
pub(super) fn transition<Ctx: AppContext + 'static>(
    src: KeymapBuilder<Ctx, Configuring>,
) -> KeymapBuilder<Ctx, Registering> {
    KeymapBuilder {
        scopes:                   src.scopes,
        pane_registrations:       src.pane_registrations,
        copy_registrations:       src.copy_registrations,
        registered_scopes:        src.registered_scopes,
        duplicate_scope:          src.duplicate_scope,
        config_path:              src.config_path,
        toml_table:               src.toml_table,
        vim_mode:                 src.vim_mode,
        on_quit:                  src.on_quit,
        on_restart:               src.on_restart,
        dismiss_fallback:         src.dismiss_fallback,
        navigation_scope:         src.navigation_scope,
        navigation_scope_name:    src.navigation_scope_name,
        navigation_render_fn:     src.navigation_render_fn,
        globals_scope:            src.globals_scope,
        globals_scope_name:       src.globals_scope_name,
        globals_action_keys:      src.globals_action_keys,
        globals_render_fn:        src.globals_render_fn,
        globals_shortcut_rows_fn: src.globals_shortcut_rows_fn,
        navigation_help_rows_fn:  src.navigation_help_rows_fn,
        app_globals_help_rows_fn: src.app_globals_help_rows_fn,
        navigation_toml_keys_fn:  src.navigation_toml_keys_fn,
        app_globals_toml_keys_fn: src.app_globals_toml_keys_fn,
        overlay_scope:            src.overlay_scope,
        vim_reserved_keys:        src.vim_reserved_keys,
        deferred_error:           src.deferred_error,
        unknown_entry_policy:     src.unknown_entry_policy,
        unknown_warnings:         src.unknown_warnings,
        _state:                   PhantomData,
    }
}
