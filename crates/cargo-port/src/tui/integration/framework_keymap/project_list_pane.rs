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
pub(super) struct ProjectListPane;

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

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tui_pane::Action;
    use tui_pane::AppContext;
    use tui_pane::BarPalette;
    use tui_pane::FocusedPane;
    use tui_pane::render_status_bar;

    use super::*;
    use crate::tui::test_support;

    #[test]
    fn project_list_bar_keeps_its_compact_pair_and_actions_have_word_labels() {
        let app = test_support::make_app(&[]);
        let bar = render_status_bar(
            &FocusedPane::App(AppPaneId::ProjectList),
            &app,
            &app.framework_keymap,
            app.framework(),
            &BarPalette::default(),
        );
        let nav: String = bar.nav.iter().map(|span| span.content.as_ref()).collect();
        assert!(nav.contains("=/- all"));
        assert_eq!(bar.pane_action, [] as [ratatui::prelude::Span<'_>; 0]);
        let actions = ProjectListAction::ALL
            .iter()
            .map(|action| {
                let key = app
                    .framework_keymap
                    .key_for_toml_key(AppPaneId::ProjectList, action.toml_key())
                    .expect("bound action");
                format!("{} {}", key.display_short(), action.bar_label())
            })
            .collect::<Vec<_>>()
            .join(" · ");
        assert_eq!(
            actions,
            "= expand all · - collapse all · → expand row · ← collapse row"
        );
        println!("Project List compact bar: {nav}");
        println!("Project List action labels: {actions}");
    }
}
