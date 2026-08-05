use super::Action;
use super::App;
use super::AppPaneId;
use super::Bindings;
use super::CopySelection;
use super::CopySelectionResult;
use super::KeyBind;
use super::OUTPUT_TAB_ORDER;
use super::OutputAction;
use super::Pane;
use super::ShortcutState;
use super::Shortcuts;
use super::TabStop;
use super::input;
use super::output_is_tabbable;

/// `key` held with Alt — the portable representation the keymap stores for the
/// termination shortcuts, spelled `alt-<key>` in TOML.
///
/// Pass an uppercase `key` for the Shift-held form: that is the normalized
/// dispatch spelling a live Alt-Shift keypress arrives as, and the one
/// `alt-shift-k` in a user's TOML parses to.
fn alt_key(key: char) -> KeyBind {
    KeyBind::from_parts(
        crossterm::event::KeyCode::Char(key),
        crossterm::event::KeyModifiers::ALT,
    )
}

/// `Pane<App>` + `Shortcuts<App>` host for the Output pane.
///
/// `Navigable`: the cursor scrolls the buffer (and the `Nav` region
/// shows in the bar), `V` starts a linewise selection, and `y` yanks the
/// selected range through the framework copy path.
pub struct OutputPane;

impl Pane<App> for OutputPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Output;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(OUTPUT_TAB_ORDER, output_is_tabbable) }
}

impl Shortcuts<App> for OutputPane {
    type Actions = OutputAction;

    const SCOPE_NAME: &'static str = "output";
    const SECTION_NAME: &'static str = "Output";

    /// The two kill actions are stored portably as `alt-k` and `alt-shift-k`
    /// (the latter round-trips through the canonical spelling `alt-K`, which is
    /// how a terminal reports the physical keypress). macOS terminals label that
    /// physical key `Option` and must be configured to report it as Meta/Alt for
    /// the binding to arrive at all; a user whose terminal cannot do that
    /// rebinds through the keymap overlay like any other action.
    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            KeyBind::ctrl('a')             => OutputAction::SelectAll,
            crossterm::event::KeyCode::Esc => OutputAction::Cancel,
            alt_key('k')                   => OutputAction::KillSelectedBuild,
            alt_key('K')                   => OutputAction::KillScopedBuilds,
        }
    }

    /// Both termination actions are declared and rebindable but do not fire
    /// yet: the confirmation transaction that authorizes them lands with the
    /// termination phases, so the bar greys them out rather than offering a
    /// key that does nothing.
    fn state(&self, action: OutputAction, _: &App) -> ShortcutState {
        match action {
            OutputAction::KillSelectedBuild | OutputAction::KillScopedBuilds => {
                ShortcutState::Disabled
            },
            OutputAction::SelectAll | OutputAction::Cancel => ShortcutState::Enabled,
        }
    }

    /// The cancel label tracks what the next Esc actually does, matching
    /// the output pane's own title: stop a running process, or close the
    /// pane.
    fn bar_label(&self, action: OutputAction, ctx: &App) -> &'static str {
        match action {
            OutputAction::Cancel => {
                if ctx.panes.output.selection().is_visual() {
                    // A visual selection is active: Esc collapses it back to
                    // the cursor row rather than stopping a run or closing
                    // the pane. Matches the title's "(y copy · Esc done)".
                    "done"
                } else if ctx.inflight.owned_run().is_running() {
                    "stop"
                } else {
                    "close"
                }
            },
            OutputAction::SelectAll
            | OutputAction::KillSelectedBuild
            | OutputAction::KillScopedBuilds => action.bar_label(),
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { input::dispatch_output_action }
}

impl CopySelection<App> for OutputPane {
    /// What the cursor is on decides what `y` copies: an activity row copies
    /// that row, and captured output copies the selected range.
    fn copy_selection(ctx: &App) -> CopySelectionResult { ctx.copy_output_selection() }
}

#[cfg(test)]
mod tests {
    use tui_pane::KeyBind;
    use tui_pane::Shortcuts;

    use super::OutputAction;
    use super::OutputPane;
    use super::alt_key;

    /// The kill defaults are stored portably. `alt-shift-k` is the spelling the
    /// plan documents for users; it parses to the same bind as the canonical
    /// `alt-K` that a terminal reports and that the generated keymap writes.
    #[test]
    fn kill_defaults_parse_from_their_portable_toml_spellings() {
        let defaults = OutputPane::defaults().into_scope_map();

        for (spelling, expected) in [
            ("alt-k", OutputAction::KillSelectedBuild),
            ("alt-shift-k", OutputAction::KillScopedBuilds),
            ("alt-K", OutputAction::KillScopedBuilds),
        ] {
            let resolved = KeyBind::parse(spelling)
                .ok()
                .and_then(|bind| defaults.action_for(&bind));
            assert_eq!(
                resolved,
                Some(expected),
                "{spelling} should parse and resolve to {expected:?}"
            );
        }
    }

    /// A macOS terminal that reports Option as Meta sends Alt-Shift-K as the
    /// normalized uppercase char, which must reach the same binding.
    #[test]
    fn alt_shift_keypress_normalizes_onto_the_scoped_kill_binding() {
        let defaults = OutputPane::defaults().into_scope_map();
        let pressed = KeyBind::from_parts(
            crossterm::event::KeyCode::Char('k'),
            crossterm::event::KeyModifiers::ALT | crossterm::event::KeyModifiers::SHIFT,
        );

        assert_eq!(pressed, alt_key('K'));
        assert_eq!(
            defaults.action_for(&pressed),
            Some(OutputAction::KillScopedBuilds),
        );
    }
}
