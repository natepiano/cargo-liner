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
    mut builder: KeymapBuilder<Ctx, State>,
) -> Result<Keymap<Ctx>, KeymapError> {
    if let Some(err) = builder.deferred_error {
        return Err(err);
    }
    if let Some(type_name) = builder.duplicate_scope {
        return Err(KeymapError::DuplicateScope { type_name });
    }
    if builder.navigation_scope.is_none() && !builder.scopes.is_empty() {
        return Err(KeymapError::NavigationMissing);
    }
    if builder.globals_scope.is_none() && !builder.scopes.is_empty() {
        return Err(KeymapError::GlobalsMissing);
    }
    validate_toml_scopes(
        builder.toml_table.as_ref(),
        &builder.registered_scopes,
        builder
            .ignore_unknown
            .then_some(&mut builder.unknown_warnings),
    )?;

    let mut keymap = Keymap::<Ctx>::new(builder.config_path);
    for (id, scope) in builder.scopes {
        keymap.insert_scope(id, scope);
    }
    if let Some(nav) = builder.navigation_scope {
        keymap.set_navigation(nav);
    }
    if let Some(render) = builder.navigation_render_fn {
        keymap.set_navigation_render_fn(render);
    }
    if let Some(g) = builder.globals_scope {
        keymap.set_globals(g);
    }
    if let Some(render) = builder.globals_render_fn {
        keymap.set_app_globals_render_fn(render);
    }
    if let Some(render) = builder.globals_shortcut_rows_fn {
        keymap.set_app_globals_shortcut_rows_fn(render);
    }
    if let Some(render) = builder.navigation_help_rows_fn {
        keymap.set_navigation_help_rows_fn(render);
    }
    if let Some(render) = builder.app_globals_help_rows_fn {
        keymap.set_app_globals_help_rows_fn(render);
    }
    if let Some(render) = builder.navigation_toml_keys_fn {
        keymap.set_navigation_toml_keys_fn(render);
    }
    if let Some(render) = builder.app_globals_toml_keys_fn {
        keymap.set_app_globals_toml_keys_fn(render);
    }
    let framework_globals = overlay::apply_toml_overlay_with_peer::<GlobalAction>(
        "global",
        GlobalAction::defaults(),
        builder.toml_table.as_ref(),
        builder.globals_action_keys.as_ref(),
        builder
            .ignore_unknown
            .then_some(&mut builder.unknown_warnings),
    )?;
    registration::check_reserved_vim_navigation_keys(
        "global",
        &framework_globals,
        &builder.vim_reserved_keys,
    )?;
    keymap.set_framework_globals(framework_globals.into_scope_map());
    if let Some(overlay_scope) = builder.overlay_scope {
        keymap.set_overlay_scope(overlay_scope);
    }
    if let Some(hook) = builder.on_quit {
        keymap.set_on_quit(hook);
    }
    if let Some(hook) = builder.on_restart {
        keymap.set_on_restart(hook);
    }
    if let Some(hook) = builder.dismiss_fallback {
        keymap.set_dismiss_fallback(hook);
    }
    keymap.set_unknown_warnings(builder.unknown_warnings);
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
