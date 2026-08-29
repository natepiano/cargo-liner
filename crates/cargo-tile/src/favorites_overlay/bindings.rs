//! Resolved favorites bindings and footer labels.

use tui_pane::Action;
use tui_pane::Keymap;

use super::parameter_column;
use super::parameter_column::ParameterColumnDescriptor;
use crate::app::App;
use crate::app::AppPaneId;
use crate::attract::AttractMode;
use crate::favorites::FavoritesRetryInstruction;
use crate::favorites::ResolvedBinding;
use crate::globals::AppGlobalAction;

#[derive(Clone, Debug)]
struct ModeColumnBindings {
    mode:   AttractMode,
    labels: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SelectedFavoriteActions {
    NoFavoriteSelected,
    DeleteOnly,
    LoadAndDelete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FavoritesFooterRequest {
    has_multiple_navigation_positions: bool,
    last_horizontal_column_page:       usize,
    selected_favorite_actions:         SelectedFavoriteActions,
}

#[derive(Clone, Debug, Default)]
enum CachedFavoritesFooter {
    #[default]
    NeedsRebuild,
    Current {
        request: FavoritesFooterRequest,
        text:    String,
    },
}

#[derive(Clone, Debug)]
pub(super) struct FavoritesSurfaceBindings {
    columns:  Vec<ModeColumnBindings>,
    previous: ResolvedBinding,
    next:     ResolvedBinding,
    left:     ResolvedBinding,
    right:    ResolvedBinding,
    load:     ResolvedBinding,
    delete:   ResolvedBinding,
    close:    ResolvedBinding,
    save:     ResolvedBinding,
    open:     ResolvedBinding,
    footer:   CachedFavoritesFooter,
}

impl Default for FavoritesSurfaceBindings {
    fn default() -> Self {
        Self {
            columns:  Vec::new(),
            previous: ResolvedBinding::for_action("select_previous", None),
            next:     ResolvedBinding::for_action("select_next", None),
            left:     ResolvedBinding::for_action("page_columns_left", None),
            right:    ResolvedBinding::for_action("page_columns_right", None),
            load:     ResolvedBinding::for_action("load", None),
            delete:   ResolvedBinding::for_action("delete", None),
            close:    ResolvedBinding::for_action("close", None),
            save:     ResolvedBinding::for_action("save_favorite", None),
            open:     ResolvedBinding::for_action("open_favorites", None),
            footer:   CachedFavoritesFooter::NeedsRebuild,
        }
    }
}

impl FavoritesSurfaceBindings {
    pub(super) fn resolve(keymap: &Keymap<App>) -> Self {
        let columns = [
            AttractMode::MovingBand,
            AttractMode::MovingText,
            AttractMode::Pixelate,
        ]
        .into_iter()
        .map(|mode| ModeColumnBindings {
            mode,
            labels: parameter_column::column_descriptors(mode)
                .iter()
                .map(|descriptor| resolve_column_label(keymap, mode, *descriptor))
                .collect(),
        })
        .collect();
        Self {
            columns,
            previous: resolve_pane_binding(keymap, AppPaneId::Favorites, "select_previous"),
            next: resolve_pane_binding(keymap, AppPaneId::Favorites, "select_next"),
            left: resolve_pane_binding(keymap, AppPaneId::Favorites, "page_columns_left"),
            right: resolve_pane_binding(keymap, AppPaneId::Favorites, "page_columns_right"),
            load: resolve_pane_binding(keymap, AppPaneId::Favorites, "load"),
            delete: resolve_pane_binding(keymap, AppPaneId::Favorites, "delete"),
            close: resolve_pane_binding(keymap, AppPaneId::Favorites, "close"),
            save: resolve_global_binding(keymap, "save_favorite"),
            open: resolve_global_binding(keymap, "open_favorites"),
            footer: CachedFavoritesFooter::NeedsRebuild,
        }
    }

    pub(super) fn column_labels(&self, mode: AttractMode) -> &[String] {
        self.columns
            .iter()
            .find(|bindings| bindings.mode == mode)
            .map_or(&[], |bindings| bindings.labels.as_slice())
    }

    pub(super) fn invalidate_footer(&mut self) {
        self.footer = CachedFavoritesFooter::NeedsRebuild;
    }

    pub(super) fn refresh_footer(
        &mut self,
        navigation_position_count: usize,
        last_horizontal_column_page: usize,
        selected_favorite_actions: SelectedFavoriteActions,
    ) {
        let request = FavoritesFooterRequest {
            has_multiple_navigation_positions: navigation_position_count > 1,
            last_horizontal_column_page,
            selected_favorite_actions,
        };
        if matches!(
            &self.footer,
            CachedFavoritesFooter::Current { request: current, .. } if *current == request
        ) {
            return;
        }

        let mut segments = Vec::with_capacity(5);
        if request.has_multiple_navigation_positions
            && let (
                ResolvedBinding::Bound {
                    sequence: previous, ..
                },
                ResolvedBinding::Bound { sequence: next, .. },
            ) = (&self.previous, &self.next)
        {
            segments.push(format!(
                "{}/{} move",
                previous.display_short(),
                next.display_short(),
            ));
        }
        if request.last_horizontal_column_page > 0
            && let (
                ResolvedBinding::Bound { sequence: left, .. },
                ResolvedBinding::Bound {
                    sequence: right, ..
                },
            ) = (&self.left, &self.right)
        {
            segments.push(format!(
                "{}/{} page",
                left.display_short(),
                right.display_short(),
            ));
        }
        match request.selected_favorite_actions {
            SelectedFavoriteActions::NoFavoriteSelected => {},
            SelectedFavoriteActions::DeleteOnly => {
                if let ResolvedBinding::Bound { sequence, .. } = &self.delete {
                    segments.push(format!("{} delete", sequence.display_short()));
                }
            },
            SelectedFavoriteActions::LoadAndDelete => {
                if let ResolvedBinding::Bound { sequence, .. } = &self.load {
                    segments.push(format!("{} load", sequence.display_short()));
                }
                if let ResolvedBinding::Bound { sequence, .. } = &self.delete {
                    segments.push(format!("{} delete", sequence.display_short()));
                }
            },
        }
        if let ResolvedBinding::Bound { sequence, .. } = &self.close {
            segments.push(format!("{} close", sequence.display_short()));
        }
        self.footer = CachedFavoritesFooter::Current {
            request,
            text: segments.join("   "),
        };
    }

    pub(super) fn footer(&self) -> &str {
        match &self.footer {
            CachedFavoritesFooter::NeedsRebuild => "",
            CachedFavoritesFooter::Current { text, .. } => text,
        }
    }

    pub(super) fn delete_confirmation_notice(&self) -> String {
        format!(
            "Press {} again to confirm deletion",
            self.delete.display_short()
        )
    }

    pub(super) fn empty_notice(&self) -> String {
        format!(
            "No favorites saved -- press {}, then {} while the attract screen is up",
            self.close.display_short(),
            self.save.display_short(),
        )
    }

    pub(super) fn delete_retry(&self) -> FavoritesRetryInstruction {
        FavoritesRetryInstruction::Press(self.delete.clone())
    }

    pub(super) fn close_delete_retry(&self) -> FavoritesRetryInstruction {
        FavoritesRetryInstruction::ReopenThenPress {
            open:  self.open.clone(),
            retry: self.delete.clone(),
        }
    }
}

fn resolve_pane_binding(
    keymap: &Keymap<App>,
    pane: AppPaneId,
    action_name: &'static str,
) -> ResolvedBinding {
    ResolvedBinding::for_action(action_name, keymap.key_for_toml_key(pane, action_name))
}

fn resolve_global_binding(keymap: &Keymap<App>, action_name: &'static str) -> ResolvedBinding {
    let binding = AppGlobalAction::from_toml_key(action_name).and_then(|action| {
        keymap
            .globals::<AppGlobalAction>()
            .and_then(|scope| scope.key_for(action))
            .cloned()
    });
    ResolvedBinding::for_action(action_name, binding)
}

fn resolve_column_label(
    keymap: &Keymap<App>,
    mode: AttractMode,
    descriptor: ParameterColumnDescriptor,
) -> String {
    descriptor
        .action_names
        .iter()
        .map(|action| {
            resolve_pane_binding(keymap, AppPaneId::Attract(mode), action).display_short()
        })
        .collect::<Vec<_>>()
        .join(descriptor.separator)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use tui_pane::FocusedPane;
    use tui_pane::Framework;
    use tui_pane::KeyBind;
    use tui_pane::KeySequence;

    use super::*;
    use crate::keymap;

    fn keymap_from(toml: &str) -> Keymap<App> {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("keymap.toml");
        if !toml.is_empty() {
            fs::write(&path, toml).expect("test keymap should be written");
        }
        let mut framework = Framework::new(FocusedPane::App(AppPaneId::Main));
        keymap::build_keymap(&mut framework, (!toml.is_empty()).then_some(path))
            .expect("test keymap should resolve")
    }

    #[test]
    fn column_descriptors_resolve_the_complete_default_matrix() {
        let keymap = keymap_from("");
        let bindings = FavoritesSurfaceBindings::resolve(&keymap);

        assert_eq!(
            bindings.column_labels(AttractMode::MovingBand),
            ["←↑↓→", "-/+", "</>", "[/]", "v"]
        );
        assert_eq!(
            bindings.column_labels(AttractMode::MovingText),
            ["←↑↓→", "</>", "[/]", "v", "t"]
        );
        assert_eq!(
            bindings.column_labels(AttractMode::Pixelate),
            ["←↑↓→", "</>", "[/]", "-/+", "v", "t"]
        );
        assert_eq!(
            parameter_column::column_descriptors(AttractMode::Pixelate)[0].action_names[0],
            "sweep_left"
        );
        assert_eq!(
            parameter_column::column_descriptors(AttractMode::MovingBand)[0].action_names[0],
            "travel_left"
        );
        assert_eq!(
            parameter_column::column_descriptors(AttractMode::MovingText)[0].action_names[0],
            "travel_left"
        );
    }

    #[test]
    fn column_footer_and_empty_labels_follow_rebinding() {
        let keymap = keymap_from(
            r#"
[global]
save_favorite = "y"

[favorites]
select_previous = "w"
select_next = "s"
page_columns_left = "a"
page_columns_right = "d"
close = "z"

[attract_pixelate]
sweep_left = "a"
sweep_up = "u"
sweep_down = "n"
sweep_right = "r"
"#,
        );
        let mut bindings = FavoritesSurfaceBindings::resolve(&keymap);

        assert_eq!(bindings.column_labels(AttractMode::Pixelate)[0], "aunr");
        bindings.refresh_footer(2, 1, SelectedFavoriteActions::LoadAndDelete);
        assert_eq!(
            bindings.footer(),
            "w/s move   a/d page   enter load   x delete   z close"
        );
        bindings.refresh_footer(2, 0, SelectedFavoriteActions::LoadAndDelete);
        assert_eq!(
            bindings.footer(),
            "w/s move   enter load   x delete   z close"
        );
        assert_eq!(
            bindings.empty_notice(),
            "No favorites saved -- press z, then y while the attract screen is up"
        );
        assert!(bindings.footer().contains("enter load"));
        assert!(bindings.footer().contains("x delete"));
    }

    #[test]
    fn footer_names_only_actions_the_selection_can_run() {
        let keymap = keymap_from("");
        let mut bindings = FavoritesSurfaceBindings::resolve(&keymap);

        bindings.refresh_footer(2, 0, SelectedFavoriteActions::DeleteOnly);
        assert_eq!(bindings.footer(), "↑/↓ move   x delete   Esc close");
        assert!(!bindings.footer().contains("load"));

        bindings.refresh_footer(0, 0, SelectedFavoriteActions::NoFavoriteSelected);
        assert_eq!(bindings.footer(), "Esc close");
        assert!(!bindings.footer().contains("load"));
        assert!(!bindings.footer().contains("delete"));

        bindings.refresh_footer(1, 0, SelectedFavoriteActions::LoadAndDelete);
        assert_eq!(bindings.footer(), "enter load   x delete   Esc close");
        assert!(!bindings.footer().contains("move"));
    }

    #[test]
    fn footer_omits_every_segment_with_an_unbound_action() {
        let keymap = keymap_from(
            r#"
[favorites]
select_previous = ""
page_columns_right = ""
load = ""
delete = ""
close = ""
"#,
        );
        let mut bindings = FavoritesSurfaceBindings::resolve(&keymap);

        bindings.refresh_footer(2, 1, SelectedFavoriteActions::LoadAndDelete);

        assert_eq!(bindings.footer(), "");
    }

    #[test]
    fn unbound_labels_cross_the_same_named_boundary_as_bound_labels() {
        let unbound = ResolvedBinding::for_action("save_favorite", None);
        let bound = ResolvedBinding::for_action(
            "save_favorite",
            Some(KeySequence::from(KeyBind::ctrl('s'))),
        );

        assert_eq!(
            unbound,
            ResolvedBinding::Unbound {
                action_name: "save_favorite",
            }
        );
        assert_eq!(unbound.display_short(), "");
        assert_eq!(bound.display_short(), "⌃s");
    }
}
