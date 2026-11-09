use std::collections::HashSet;

use toml::Table;

use super::KeymapBuilder;
use super::overlay;
use super::registration;
use crate::AppContext;
use crate::GlobalAction;
use crate::Keymap;
use crate::KeymapError;

/// Drop the `Configuring`/`Registering` state and return a finalized
/// [`Keymap`]. Validates the parsed TOML against registered scope
/// names and emits the scopes, singletons, and lifecycle hooks the
/// builder has collected.
pub(super) fn finalize<Ctx: AppContext + 'static, State>(
    mut keymap_builder: KeymapBuilder<Ctx, State>,
) -> Result<Keymap<Ctx>, KeymapError> {
    if let Some(keymap_error) = keymap_builder.deferred_error {
        return Err(keymap_error);
    }
    if let Some(type_name) = keymap_builder.duplicate_scope {
        return Err(KeymapError::DuplicateScope { type_name });
    }
    if keymap_builder.navigation_scope.is_none() && !keymap_builder.scopes.is_empty() {
        return Err(KeymapError::NavigationMissing);
    }
    if keymap_builder.globals_scope.is_none() && !keymap_builder.scopes.is_empty() {
        return Err(KeymapError::GlobalsMissing);
    }
    validate_toml_scopes(
        keymap_builder.toml_table.as_ref(),
        &keymap_builder.registered_scopes,
        keymap_builder
            .unknown_entry_policy
            .warnings(&mut keymap_builder.unknown_warnings),
    )?;

    let mut keymap = Keymap::<Ctx>::new(keymap_builder.config_path);
    for (id, runtime_scope) in keymap_builder.scopes {
        keymap.insert_scope(id, runtime_scope);
    }
    if let Some(navigation_scope_map) = keymap_builder.navigation_scope {
        keymap.set_navigation(navigation_scope_map);
    }
    if let Some(navigation_scope_render_fn) = keymap_builder.navigation_render_fn {
        keymap.set_navigation_render_fn(navigation_scope_render_fn);
    }
    if let Some(globals_singleton) = keymap_builder.globals_scope {
        keymap.set_globals(globals_singleton);
    }
    if let Some(app_globals_scope_render_fn) = keymap_builder.globals_render_fn {
        keymap.set_app_globals_render_fn(app_globals_scope_render_fn);
    }
    if let Some(scope_shortcut_rows_fn) = keymap_builder.globals_shortcut_rows_fn {
        keymap.set_app_globals_shortcut_rows_fn(scope_shortcut_rows_fn);
    }
    if let Some(navigation_scope_help_rows_fn) = keymap_builder.navigation_help_rows_fn {
        keymap.set_navigation_help_rows_fn(navigation_scope_help_rows_fn);
    }
    if let Some(app_globals_scope_help_rows_fn) = keymap_builder.app_globals_help_rows_fn {
        keymap.set_app_globals_help_rows_fn(app_globals_scope_help_rows_fn);
    }
    if let Some(navigation_scope_toml_action_keys_fn) = keymap_builder.navigation_toml_keys_fn {
        keymap.set_navigation_toml_keys_fn(navigation_scope_toml_action_keys_fn);
    }
    if let Some(app_globals_scope_toml_action_keys_fn) = keymap_builder.app_globals_toml_keys_fn {
        keymap.set_app_globals_toml_keys_fn(app_globals_scope_toml_action_keys_fn);
    }
    let framework_global_bindings = overlay::apply_toml_overlay_with_peer::<GlobalAction>(
        "global",
        GlobalAction::defaults(),
        keymap_builder.toml_table.as_ref(),
        keymap_builder.globals_action_keys.as_ref(),
        keymap_builder
            .unknown_entry_policy
            .warnings(&mut keymap_builder.unknown_warnings),
        false,
    )?;
    registration::check_reserved_vim_navigation_keys(
        "global",
        &framework_global_bindings,
        &keymap_builder.vim_reserved_keys,
    )?;
    keymap.set_framework_globals(framework_global_bindings.into_scope_map());
    if let Some(overlay_scope_map) = keymap_builder.overlay_scope {
        keymap.set_overlay_scope(overlay_scope_map);
    }
    if let Some(hook) = keymap_builder.on_quit {
        keymap.set_on_quit(hook);
    }
    if let Some(hook) = keymap_builder.on_restart {
        keymap.set_on_restart(hook);
    }
    if let Some(hook) = keymap_builder.dismiss_fallback {
        keymap.set_dismiss_fallback(hook);
    }
    keymap.set_framework_global_shortcut_presentation(
        keymap_builder.framework_global_shortcut_presentation,
    );
    keymap.set_unknown_warnings(keymap_builder.unknown_warnings);
    Ok(keymap)
}

/// Reject TOML scope keys that do not match any registered
/// `SCOPE_NAME`. The shared `[global]` table is also accepted because
/// the framework globals read it alongside the binary globals.
///
/// `unknown` mirrors [`apply_toml_overlay`](overlay::apply_toml_overlay):
/// `None` returns [`KeymapError::UnknownScope`] (strict); `Some(sink)`
/// records a warning and skips the scope (lenient).
fn validate_toml_scopes(
    table: Option<&Table>,
    registered: &HashSet<&'static str>,
    mut unknown: Option<&mut Vec<String>>,
) -> Result<(), KeymapError> {
    let Some(table) = table else {
        return Ok(());
    };
    for key in table.keys() {
        if registered.contains(key.as_str()) {
            continue;
        }
        if key == "global" {
            continue;
        }
        if let Some(sink) = unknown.as_deref_mut() {
            sink.push(format!("unknown scope [{key}] (ignored)"));
            continue;
        }
        return Err(KeymapError::UnknownScope { scope: key.clone() });
    }
    Ok(())
}
