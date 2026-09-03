//! Keymap-editor controller: the input and persistence half of the
//! keymap overlay.
//!
//! [`KeymapPane`] owns the overlay's rendering and its `EditState`
//! machine; this module owns everything between the two — moving the
//! selection, starting an edit, validating a captured key against the
//! bindings already in force, and writing the result back to the
//! keymap TOML.
//!
//! An embedding app implements [`KeymapEditContext`] and routes the
//! keymap overlay's keys through [`handle_keymap_navigation_key`],
//! [`dispatch_keymap_action`], and [`handle_keymap_capture_command`].
//! Everything an app must supply is app-specific by nature: where the
//! TOML lives, how to rebuild the keymap after a write, and which
//! keys the app reserves for itself.

use std::fmt::Write as _;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;

use crate::Action;
use crate::GlobalAction;
use crate::Globals;
use crate::KeyBind;
use crate::KeySequence;
use crate::Keymap;
use crate::KeymapCaptureCommand;
use crate::KeymapHelpRow;
use crate::KeymapHelpRowKind;
use crate::KeymapPane;
use crate::KeymapUiContext;
use crate::NavAction;
use crate::OverlayAction;

/// TOML table shared by the framework globals and the app globals.
const GLOBAL_SCOPE: &str = "global";
/// TOML table holding the framework navigation scope.
const NAVIGATION_SCOPE: &str = "navigation";
/// TOML table holding the framework overlay bar.
const OVERLAY_SCOPE: &str = "overlay";

/// Keys the editor refuses to bind because the overlay itself needs
/// them to move around.
const RESERVED_NAVIGATION: [KeyCode; 8] = [
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Left,
    KeyCode::Right,
    KeyCode::Home,
    KeyCode::End,
    KeyCode::PageUp,
    KeyCode::PageDown,
];

/// What the app must supply for the framework to run the keymap
/// editor on its behalf.
pub trait KeymapEditContext: KeymapUiContext + Sized {
    /// This app's globals scope. Its actions share the `[global]`
    /// TOML table with the framework's own, so the writer has to ask
    /// the app which of the two owns a given action key.
    type AppGlobals: Globals<Self>;

    /// Comment block written above the generated tables.
    const KEYMAP_TOML_HEADER: &'static str;

    /// Where the keymap TOML lives. `None` disables persistence —
    /// rebinds then last only as long as the process does.
    fn keymap_file_path(&self) -> Option<PathBuf>;

    /// Record the message shown on the selected row after a rejected
    /// capture. Read back through
    /// [`KeymapUiContext::keymap_inline_error`].
    fn set_keymap_inline_error(&mut self, message: String);

    /// Clear whatever [`Self::set_keymap_inline_error`] recorded.
    fn clear_keymap_inline_error(&mut self);

    /// Rebuild this app's keymap from `content`, which has already
    /// been written to [`Self::keymap_file_path`]. An app that fails
    /// to rebuild should surface that itself; the editor has no
    /// recovery to offer beyond leaving the file in place.
    fn reload_keymap(&mut self, content: &str);

    /// Reject `bind` with the returned message, or accept it with
    /// `None`. The overlay's own navigation keys are rejected before
    /// this runs; override to add app reservations such as vim
    /// motion keys.
    fn keymap_reserved_bind(&self, _bind: KeyBind) -> Option<String> { None }

    /// Whether `bind` is generated rather than configured. Generated
    /// binds are left out of the written TOML — writing them back
    /// would freeze an alias that only exists while the mode
    /// generating it is on.
    fn keymap_generated_bind(&self, _scope: &str, _action_key: &str, _bind: &KeySequence) -> bool {
        false
    }
}

/// A rebind waiting to be written, replacing whatever the TOML
/// currently holds for its scope and action.
struct PendingRebind {
    /// TOML table the action lives in.
    scope:  &'static str,
    /// TOML action key being rebound.
    action: &'static str,
    /// The captured replacement.
    bind:   KeySequence,
}

/// Dispatch an [`OverlayAction`] the keymap overlay owns.
pub fn dispatch_keymap_action<Ctx: KeymapEditContext>(
    action: OverlayAction,
    ctx: &mut Ctx,
    keymap: &Keymap<Ctx>,
) {
    ctx.clear_keymap_inline_error();
    match action {
        OverlayAction::StartEdit => ctx.framework_mut().keymap_pane.enter_awaiting(),
        OverlayAction::Cancel => {
            ctx.framework_mut().keymap_pane.enter_browse();
            if ctx.framework().overlay().is_some() {
                keymap.dispatch_framework_global(GlobalAction::Dismiss, ctx);
            }
        },
    }
}

/// Move the selection inside the keymap overlay, or start editing the
/// selected row on Enter.
pub fn handle_keymap_navigation_key<Ctx: KeymapEditContext>(
    ctx: &mut Ctx,
    keymap: &Keymap<Ctx>,
    code: KeyCode,
) {
    match code {
        KeyCode::Enter => {
            ctx.clear_keymap_inline_error();
            ctx.framework_mut().keymap_pane.enter_awaiting();
        },
        KeyCode::End => {
            let last = selectable_rows(ctx, keymap).len().saturating_sub(1);
            ctx.framework_mut().keymap_pane.viewport_mut().set_pos(last);
        },
        _ => {
            let viewport = ctx.framework_mut().keymap_pane.viewport_mut();
            match code {
                KeyCode::Up => viewport.up(),
                KeyCode::Down => viewport.down(),
                KeyCode::Home => viewport.home(),
                KeyCode::PageUp => viewport.page_up(),
                KeyCode::PageDown => viewport.page_down(),
                _ => (),
            }
        },
    }
}

/// Act on the command [`KeymapPane::handle_capture_key`] produced.
pub fn handle_keymap_capture_command<Ctx: KeymapEditContext>(
    ctx: &mut Ctx,
    keymap: &Keymap<Ctx>,
    command: KeymapCaptureCommand,
) {
    match command {
        KeymapCaptureCommand::None => (),
        KeymapCaptureCommand::Cancel | KeymapCaptureCommand::ClearConflict => {
            ctx.clear_keymap_inline_error();
        },
        KeymapCaptureCommand::Captured(bind) => capture(ctx, keymap, bind),
    }
}

/// Begin remapping the row selected in the compact global-shortcuts
/// overlay, by opening the full editor on the matching row.
pub fn edit_selected_global_shortcut<Ctx: KeymapEditContext>(ctx: &mut Ctx, keymap: &Keymap<Ctx>) {
    let rows = keymap.global_shortcut_rows();
    let Some(selected) = rows.get(ctx.framework().global_shortcuts_pane.viewport().pos()) else {
        return;
    };
    let target = (selected.scope, selected.action);
    let Some(index) = selectable_rows(ctx, keymap)
        .iter()
        .position(|row| (row.scope, row.action) == target)
    else {
        return;
    };

    keymap.dispatch_framework_global(GlobalAction::OpenKeymap, ctx);
    ctx.clear_keymap_inline_error();
    let pane = &mut ctx.framework_mut().keymap_pane;
    pane.viewport_mut().set_pos(index);
    pane.enter_awaiting();
}

/// Write the keymap as it currently stands, then reload it. Apps call
/// this when something other than a rebind changes what the TOML
/// should say.
pub fn save_keymap_to_disk<Ctx: KeymapEditContext>(ctx: &mut Ctx, keymap: &Keymap<Ctx>) {
    if let Err(message) = write_keymap(ctx, keymap, None) {
        ctx.set_keymap_inline_error(message);
    }
}

/// The keymap TOML this context would write right now.
#[must_use]
pub fn keymap_toml<Ctx: KeymapEditContext>(ctx: &Ctx, keymap: &Keymap<Ctx>) -> String {
    keymap_toml_with_pending(ctx, keymap, None)
}

/// Validate a captured bind against every rule in force, then either
/// apply it or park the overlay on the conflict.
fn capture<Ctx: KeymapEditContext>(ctx: &mut Ctx, keymap: &Keymap<Ctx>, bind: KeyBind) {
    let rows = KeymapPane::ordered_help_rows(ctx, keymap);
    let selected = rows
        .iter()
        .filter(|row| row.row_kind != KeymapHelpRowKind::Header)
        .nth(ctx.framework().keymap_pane.viewport().pos())
        .cloned();
    let Some(row) = selected else {
        return;
    };

    if let Some(message) = rejection(ctx, &rows, &row, bind) {
        ctx.set_keymap_inline_error(message);
        ctx.framework_mut().keymap_pane.enter_conflict();
        return;
    }

    let pending = PendingRebind {
        scope:  row.scope,
        action: row.action,
        bind:   bind.into(),
    };
    if let Err(message) = write_keymap(ctx, keymap, Some(&pending)) {
        ctx.set_keymap_inline_error(message);
        ctx.framework_mut().keymap_pane.enter_conflict();
        return;
    }
    ctx.clear_keymap_inline_error();
    ctx.framework_mut().keymap_pane.enter_browse();
}

/// Why `bind` cannot be bound to `row`, or `None` if it can.
///
/// A global that shadows a pane scope steals that pane's key
/// silently, so the check runs in both directions rather than only
/// within the row's own scope.
fn rejection<Ctx: KeymapEditContext>(
    ctx: &Ctx,
    rows: &[KeymapHelpRow],
    row: &KeymapHelpRow,
    bind: KeyBind,
) -> Option<String> {
    if bind.mods == KeyModifiers::NONE && RESERVED_NAVIGATION.contains(&bind.code) {
        return Some(format!("\"{}\" reserved for navigation", bind.display()));
    }
    if let Some(message) = ctx.keymap_reserved_bind(bind) {
        return Some(message);
    }
    let cross_scope = if row.scope == GLOBAL_SCOPE {
        conflict(rows, row, bind, |other| other.scope != GLOBAL_SCOPE)
    } else {
        conflict(rows, row, bind, |other| other.scope == GLOBAL_SCOPE)
    };
    cross_scope.or_else(|| conflict(rows, row, bind, |other| other.scope == row.scope))
}

/// The first row matching `predicate` that already holds `bind`.
fn conflict(
    rows: &[KeymapHelpRow],
    current: &KeymapHelpRow,
    bind: KeyBind,
    predicate: impl Fn(&KeymapHelpRow) -> bool,
) -> Option<String> {
    rows.iter()
        .filter(|row| row.row_kind != KeymapHelpRowKind::Header)
        .filter(|row| predicate(row))
        .filter(|row| row.bind.as_ref().and_then(KeySequence::single_key) == Some(bind))
        .find(|row| row.scope != current.scope || row.action != current.action)
        .map(|row| {
            format!(
                "\"{}\" used by {} → {}",
                bind.display(),
                row.section,
                row.action,
            )
        })
}

/// Every selectable (non-header) row, in overlay display order.
fn selectable_rows<Ctx: KeymapEditContext>(ctx: &Ctx, keymap: &Keymap<Ctx>) -> Vec<KeymapHelpRow> {
    KeymapPane::ordered_help_rows(ctx, keymap)
        .into_iter()
        .filter(|row| row.row_kind != KeymapHelpRowKind::Header)
        .collect()
}

/// Render the TOML, write it, and hand it back to the app to reload.
///
/// An app with no [`KeymapEditContext::keymap_file_path`] keeps the
/// rebind in memory: there is nowhere to persist it, which is not an
/// error.
fn write_keymap<Ctx: KeymapEditContext>(
    ctx: &mut Ctx,
    keymap: &Keymap<Ctx>,
    pending: Option<&PendingRebind>,
) -> Result<(), String> {
    let Some(path) = ctx.keymap_file_path() else {
        return Ok(());
    };
    let content = keymap_toml_with_pending(ctx, keymap, pending);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("creating {}: {error}", parent.display()))?;
    }
    std::fs::write(&path, &content)
        .map_err(|error| format!("writing {}: {error}", path.display()))?;
    ctx.reload_keymap(&content);
    Ok(())
}

/// Render the whole keymap TOML, substituting `pending` for whatever
/// the keymap currently resolves for that one action.
fn keymap_toml_with_pending<Ctx: KeymapEditContext>(
    ctx: &Ctx,
    keymap: &Keymap<Ctx>,
    pending: Option<&PendingRebind>,
) -> String {
    let mut out = String::from(Ctx::KEYMAP_TOML_HEADER);
    for (scope, action_keys) in keymap.keymap_toml_scope_keys(ctx.keymap_pane_display_order()) {
        let _ = writeln!(out, "[{scope}]");
        let mut entries: Vec<(&'static str, Vec<KeySequence>)> = action_keys
            .into_iter()
            .map(|action_key| (action_key, binds_for(ctx, keymap, scope, action_key)))
            .collect();
        entries.sort_by_key(|(name, _)| *name);
        let width = entries
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0);
        for (action_key, binds) in &entries {
            let value = pending
                .filter(|pending| pending.scope == scope && pending.action == *action_key)
                .map_or_else(
                    || toml_value(binds),
                    |pending| toml_value(std::slice::from_ref(&pending.bind)),
                );
            let _ = writeln!(out, "{action_key:<width$} = {value}");
        }
        out.push('\n');
    }
    if out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Every bind currently resolving to one scope's action key, with the
/// app's generated aliases stripped.
fn binds_for<Ctx: KeymapEditContext>(
    ctx: &Ctx,
    keymap: &Keymap<Ctx>,
    scope: &str,
    action_key: &str,
) -> Vec<KeySequence> {
    let binds = match scope {
        GLOBAL_SCOPE => global_binds::<Ctx>(keymap, action_key),
        NAVIGATION_SCOPE => NavAction::from_toml_key(action_key)
            .zip(keymap.navigation())
            .map(|(action, nav)| nav.display_keys_for(action).to_vec())
            .unwrap_or_default(),
        OVERLAY_SCOPE => OverlayAction::from_toml_key(action_key)
            .map(|action| keymap.overlay().display_keys_for(action).to_vec())
            .unwrap_or_default(),
        _ => pane_binds(ctx, keymap, scope, action_key),
    };
    binds
        .into_iter()
        .filter(|bind| !ctx.keymap_generated_bind(scope, action_key, bind))
        .collect()
}

/// Binds under `[global]`, which the framework globals and the app
/// globals share. The framework's own action set is tried first.
fn global_binds<Ctx: KeymapEditContext>(
    keymap: &Keymap<Ctx>,
    action_key: &str,
) -> Vec<KeySequence> {
    if let Some(action) = GlobalAction::from_toml_key(action_key)
        && let Some(bind) = keymap.framework_globals().key_for(action)
    {
        return vec![bind.clone()];
    }
    if let Some(action) = <Ctx::AppGlobals as Globals<Ctx>>::Actions::from_toml_key(action_key)
        && let Some(app_globals) = keymap.globals::<Ctx::AppGlobals>()
    {
        return app_globals.display_keys_for(action).to_vec();
    }
    Vec::new()
}

/// Binds under an app pane's scope, found by matching the TOML scope
/// name the keymap registered for each pane in display order.
fn pane_binds<Ctx: KeymapEditContext>(
    ctx: &Ctx,
    keymap: &Keymap<Ctx>,
    scope: &str,
    action_key: &str,
) -> Vec<KeySequence> {
    ctx.keymap_pane_display_order()
        .iter()
        .find(|id| keymap.scope_toml_name_for(**id) == Some(scope))
        .map(|id| keymap.keys_for_toml_key(*id, action_key))
        .unwrap_or_default()
}

/// Render binds as a TOML value: a bare string for one, an array for
/// several, and an empty string for an action left unbound.
fn toml_value(binds: &[KeySequence]) -> String {
    match binds {
        [] => "\"\"".to_string(),
        [bind] => format!("\"{}\"", bind.display()),
        _ => {
            let values = binds
                .iter()
                .map(|bind| format!("\"{}\"", bind.display()))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{values}]")
        },
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;

    use super::KeymapEditContext;
    use crate::AppContext;
    use crate::Bindings;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::FrameworkGlobalShortcutPresentation;
    use crate::FrameworkGlobalShortcutVisibility;
    use crate::FrameworkOverlayId;
    use crate::GlobalAction;
    use crate::Globals;
    use crate::KeyBind;
    use crate::Keymap;
    use crate::KeymapUiContext;
    use crate::NoToastAction;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Main,
    }

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum TestAppGlobalAction {
            Inspect => ("inspect", "inspect", "Inspect item");
        }
    }

    struct TestApp {
        framework:    Framework<Self>,
        inline_error: Option<String>,
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;
        type ToastAction = NoToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    impl KeymapUiContext for TestApp {
        fn keymap_inline_error(&self) -> Option<&str> { self.inline_error.as_deref() }

        fn keymap_pane_display_order(&self) -> &[TestPaneId] { &[] }
    }

    struct TestAppGlobals;

    impl Globals<TestApp> for TestAppGlobals {
        type Actions = TestAppGlobalAction;

        fn render_order() -> &'static [Self::Actions] { &[TestAppGlobalAction::Inspect] }

        fn defaults() -> Bindings<Self::Actions> {
            crate::bindings! { 'i' => TestAppGlobalAction::Inspect }
        }

        fn dispatcher() -> fn(Self::Actions, &mut TestApp) {
            |_action, _ctx| { /* no-op */ }
        }
    }

    impl KeymapEditContext for TestApp {
        type AppGlobals = TestAppGlobals;

        const KEYMAP_TOML_HEADER: &'static str = "";

        fn keymap_file_path(&self) -> Option<PathBuf> { None }

        fn set_keymap_inline_error(&mut self, message: String) {
            self.inline_error = Some(message);
        }

        fn clear_keymap_inline_error(&mut self) { self.inline_error = None; }

        fn reload_keymap(&mut self, _content: &str) {}

        fn keymap_reserved_bind(&self, _bind: KeyBind) -> Option<String> { None }
    }

    #[test]
    fn compact_selection_opens_the_row_shown_after_a_prior_row_is_hidden() {
        const fn hide_next_pane(action: GlobalAction) -> FrameworkGlobalShortcutVisibility {
            match action {
                GlobalAction::NextPane => FrameworkGlobalShortcutVisibility::Hidden,
                _ => FrameworkGlobalShortcutVisibility::Shown,
            }
        }

        let keymap = Keymap::<TestApp>::builder()
            .framework_global_shortcut_presentation(FrameworkGlobalShortcutPresentation::new(
                hide_next_pane,
            ))
            .build()
            .expect("keymap with presentation policy must build");
        let compact_rows = keymap.global_shortcut_rows();
        let expected = compact_rows
            .first()
            .expect("the compact overlay must retain a first row");
        assert_eq!(expected.action, "prev_pane");

        let mut app = TestApp {
            framework:    Framework::new(FocusedPane::App(TestPaneId::Main)),
            inline_error: None,
        };
        super::edit_selected_global_shortcut(&mut app, &keymap);

        let editor_rows = super::selectable_rows(&app, &keymap);
        let selected = editor_rows
            .get(app.framework().keymap_pane.viewport().pos())
            .expect("the full editor must select the compact row");
        assert_eq!(
            (selected.scope, selected.action),
            (expected.scope, expected.action)
        );
        assert_eq!(app.framework().overlay(), Some(FrameworkOverlayId::Keymap));
        assert!(app.framework().keymap_pane.is_awaiting());
    }
}
