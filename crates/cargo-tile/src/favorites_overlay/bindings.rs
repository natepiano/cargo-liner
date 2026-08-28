//! Resolved favorites bindings, parameter columns, and footer labels.

use tui_pane::Action;
use tui_pane::Keymap;

use crate::app::App;
use crate::app::AppPaneId;
use crate::attract::AttractMode;
use crate::favorites::FavoritesRetryInstruction;
use crate::favorites::ResolvedBinding;
use crate::globals::AppGlobalAction;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ParameterColumnDescriptor {
    pub(super) heading: &'static str,
    action_names:       &'static [&'static str],
    separator:          &'static str,
}

const BAND_COLUMNS: [ParameterColumnDescriptor; 5] = [
    ParameterColumnDescriptor {
        heading:      "Direction",
        action_names: &["travel_left", "travel_up", "travel_down", "travel_right"],
        separator:    "",
    },
    ParameterColumnDescriptor {
        heading:      "Width",
        action_names: &["thinner", "wider"],
        separator:    "/",
    },
    ParameterColumnDescriptor {
        heading:      "Speed",
        action_names: &["slower", "faster"],
        separator:    "/",
    },
    ParameterColumnDescriptor {
        heading:      "Tail",
        action_names: &["tail_slower", "tail_faster"],
        separator:    "/",
    },
    ParameterColumnDescriptor {
        heading:      "Fraying",
        action_names: &["cycle_fraying"],
        separator:    "",
    },
];
#[cfg(test)]
pub(super) const BAND_COLUMNS_FOR_TEST: [ParameterColumnDescriptor; 5] = BAND_COLUMNS;

const TEXT_COLUMNS: [ParameterColumnDescriptor; 5] = [
    ParameterColumnDescriptor {
        heading:      "Direction",
        action_names: &["travel_left", "travel_up", "travel_down", "travel_right"],
        separator:    "",
    },
    ParameterColumnDescriptor {
        heading:      "Speed",
        action_names: &["slower", "faster"],
        separator:    "/",
    },
    ParameterColumnDescriptor {
        heading:      "Spread",
        action_names: &["spread_narrower", "spread_wider"],
        separator:    "/",
    },
    ParameterColumnDescriptor {
        heading:      "Drift",
        action_names: &["cycle_drift"],
        separator:    "",
    },
    ParameterColumnDescriptor {
        heading:      "Fill",
        action_names: &["cycle_fill"],
        separator:    "",
    },
];

const PIXEL_COLUMNS: [ParameterColumnDescriptor; 6] = [
    ParameterColumnDescriptor {
        heading:      "Direction",
        action_names: &["sweep_left", "sweep_up", "sweep_down", "sweep_right"],
        separator:    "",
    },
    ParameterColumnDescriptor {
        heading:      "Speed",
        action_names: &["slower", "faster"],
        separator:    "/",
    },
    ParameterColumnDescriptor {
        heading:      "Wave",
        action_names: &["wave_narrower", "wave_wider"],
        separator:    "/",
    },
    ParameterColumnDescriptor {
        heading:      "Block",
        action_names: &["sharper", "coarser"],
        separator:    "/",
    },
    ParameterColumnDescriptor {
        heading:      "Resolve",
        action_names: &["cycle_resolve"],
        separator:    "",
    },
    ParameterColumnDescriptor {
        heading:      "Fill",
        action_names: &["cycle_fill"],
        separator:    "",
    },
];

pub(super) const fn column_descriptors(mode: AttractMode) -> &'static [ParameterColumnDescriptor] {
    match mode {
        AttractMode::MovingBand => &BAND_COLUMNS,
        AttractMode::MovingText => &TEXT_COLUMNS,
        AttractMode::Pixelate => &PIXEL_COLUMNS,
    }
}

#[derive(Clone, Debug)]
struct ModeColumnBindings {
    mode:   AttractMode,
    labels: Vec<String>,
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
            labels: column_descriptors(mode)
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
        }
    }

    pub(super) fn column_labels(&self, mode: AttractMode) -> &[String] {
        self.columns
            .iter()
            .find(|bindings| bindings.mode == mode)
            .map_or(&[], |bindings| bindings.labels.as_slice())
    }

    pub(super) fn footer(&self, last_horizontal_column_page: usize) -> String {
        let movement = format!(
            "{}/{} move",
            self.previous.display_short(),
            self.next.display_short(),
        );
        let mutations = format!(
            "{} load   {} delete",
            self.load.display_short(),
            self.delete.display_short(),
        );
        let close = format!("{} close", self.close.display_short());
        if last_horizontal_column_page == 0 {
            format!("{movement}   {mutations}   {close}")
        } else {
            format!(
                "{movement}   {}/{} page   {mutations}   {close}",
                self.left.display_short(),
                self.right.display_short(),
            )
        }
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

pub(super) const fn mode_label(mode: AttractMode) -> &'static str {
    match mode {
        AttractMode::MovingBand => "Moving Band",
        AttractMode::MovingText => "Moving Text",
        AttractMode::Pixelate => "Pixelate",
    }
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
        assert_eq!(PIXEL_COLUMNS[0].action_names[0], "sweep_left");
        assert_eq!(BAND_COLUMNS[0].action_names[0], "travel_left");
        assert_eq!(TEXT_COLUMNS[0].action_names[0], "travel_left");
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
        let bindings = FavoritesSurfaceBindings::resolve(&keymap);

        assert_eq!(bindings.column_labels(AttractMode::Pixelate)[0], "aunr");
        assert_eq!(
            bindings.footer(1),
            "w/s move   a/d page   enter load   x delete   z close"
        );
        assert_eq!(
            bindings.footer(0),
            "w/s move   enter load   x delete   z close"
        );
        assert_eq!(
            bindings.empty_notice(),
            "No favorites saved -- press z, then y while the attract screen is up"
        );
        assert!(bindings.footer(1).contains("enter load"));
        assert!(bindings.footer(1).contains("x delete"));
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
