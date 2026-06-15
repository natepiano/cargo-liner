use super::App;
use super::AppPaneId;
use super::BarRegion;
use super::BarSlot;
use super::Bindings;
use super::CopyLabel;
use super::CopyPayload;
use super::CopySelection;
use super::CopySelectionResult;
use super::KeyBind;
use super::PROJECT_LIST_TAB_ORDER;
use super::Pane;
use super::ProjectListAction;
use super::Shortcuts;
use super::TabStop;
use super::input;
use super::project_list_is_tabbable;

/// `Pane<App>` + `Shortcuts<App>` host for the `ProjectList` pane.
pub struct ProjectListPane;

impl Pane<App> for ProjectListPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::ProjectList;

    fn tab_stop() -> TabStop<App> {
        TabStop::ordered(PROJECT_LIST_TAB_ORDER, project_list_is_tabbable)
    }
}

impl Shortcuts<App> for ProjectListPane {
    type Actions = ProjectListAction;

    const SCOPE_NAME: &'static str = "project_list";
    const SECTION_NAME: &'static str = "Project List";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Right => ProjectListAction::ExpandRow,
            crossterm::event::KeyCode::Left => ProjectListAction::CollapseRow,
            '=' => ProjectListAction::ExpandAll,
            '-' => ProjectListAction::CollapseAll,
        }
    }

    fn bar_slots(&self, _: &App) -> Vec<(BarRegion, BarSlot<Self::Actions>)> {
        vec![(
            BarRegion::Nav,
            BarSlot::Paired(
                ProjectListAction::ExpandAll,
                ProjectListAction::CollapseAll,
                "all",
            ),
        )]
    }

    fn vim_extras() -> &'static [(Self::Actions, KeyBind)] { &PROJECT_LIST_VIM_EXTRAS }

    fn dispatcher() -> fn(Self::Actions, &mut App) { input::dispatch_project_list_action }
}

impl CopySelection<App> for ProjectListPane {
    fn copy_selection(ctx: &App) -> CopySelectionResult {
        ctx.project_list
            .selected_project_path()
            .map_or(CopySelectionResult::Nothing, |path| {
                CopySelectionResult::Payload(CopyPayload::new(
                    path.display().to_string(),
                    CopyLabel::Path,
                ))
            })
    }
}

/// `'l'` / `'h'` extend the `ProjectList` scope with vim-style row
/// expand/collapse when `VimMode::Enabled`. Append-only — the keymap
/// builder's vim overlay skips letters already bound on the full
/// `KeyBind` (code + mods), so a TOML rebind to `'l'` for a different
/// action wins over this default.
static PROJECT_LIST_VIM_EXTRAS: [(ProjectListAction, KeyBind); 2] = [
    (ProjectListAction::ExpandRow, KeyBind::from_char('l')),
    (ProjectListAction::CollapseRow, KeyBind::from_char('h')),
];
