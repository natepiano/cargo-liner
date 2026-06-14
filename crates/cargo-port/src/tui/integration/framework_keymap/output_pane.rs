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
use super::Shortcuts;
use super::TabStop;
use super::input;
use super::output_is_tabbable;

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

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            KeyBind::ctrl('a')             => OutputAction::SelectAll,
            crossterm::event::KeyCode::Esc => OutputAction::Cancel,
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
                } else if ctx.inflight.example_running().is_some() {
                    "stop"
                } else {
                    "close"
                }
            },
            OutputAction::SelectAll => action.bar_label(),
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { input::dispatch_output_action }
}

impl CopySelection<App> for OutputPane {
    fn copy_selection(ctx: &App) -> CopySelectionResult {
        ctx.panes.output.copy_payload(ctx.inflight.example_output())
    }
}
