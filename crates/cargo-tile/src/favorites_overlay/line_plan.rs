//! Cached favorites lines, navigation, and rendering.

use std::path::Path;
use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use tui_pane::PaneFocusState;
use tui_pane::blend_color;
use tui_pane::error_color;
use tui_pane::label_color;
use tui_pane::selection_style;
use tui_pane::text_default;
use tui_pane::title_color;
use unicode_width::UnicodeWidthStr;

use super::bindings::FavoritesSurfaceBindings;
use super::constants::FAVORITE_REMOVAL_FADE;
use super::constants::FOOTER_HEIGHT;
use super::constants::POPUP_MAX_WIDTH;
use super::constants::POPUP_SIDE_MARGIN;
use super::content::FavoriteModeSection;
use super::content::FavoriteRowLifecycle;
use super::content::FavoriteRowLookup;
use super::content::FavoritesOverlayContent;
use super::content::UnrecognizedFavoriteView;
use super::parameter_column;
use super::table_layout;
use super::table_layout::FavoriteSectionTableLayout;
use crate::app::AppOverlay;
use crate::app::OpenFavoritesCurrentParameters;
use crate::attract;
use crate::constants::POPUP_CHROME_HEIGHT;
use crate::constants::POPUP_CHROME_WIDTH;
use crate::favorites::FavoriteId;
use crate::favorites::FavoriteRemovalTarget;
use crate::favorites::UnrecognizedFavoriteRemovalLocator;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FavoriteRowCurrentParameters {
    Unrecognized,
    Different,
    Matching,
}

#[derive(Clone, Debug)]
pub(super) enum CachedOverlayLine {
    NonRow(Line<'static>),
    Row {
        identity:           FavoriteRowIdentity,
        current_parameters: FavoriteRowCurrentParameters,
        tail:               String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FavoriteRowIdentity {
    Recognized(FavoriteId),
    Unrecognized(UnrecognizedFavoriteRemovalLocator),
}

impl From<FavoriteRowIdentity> for FavoriteRemovalTarget {
    fn from(identity: FavoriteRowIdentity) -> Self {
        match identity {
            FavoriteRowIdentity::Recognized(favorite_id) => Self::Recognized(favorite_id),
            FavoriteRowIdentity::Unrecognized(removal_locator) => {
                Self::Unrecognized(removal_locator)
            },
        }
    }
}

impl From<FavoriteRemovalTarget> for FavoriteRowIdentity {
    fn from(removal_target: FavoriteRemovalTarget) -> Self {
        match removal_target {
            FavoriteRemovalTarget::Recognized(favorite_id) => Self::Recognized(favorite_id),
            FavoriteRemovalTarget::Unrecognized(removal_locator) => {
                Self::Unrecognized(removal_locator)
            },
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct CachedLinePlan {
    pub(super) lines:                       Vec<CachedOverlayLine>,
    selectable_line_index:                  Vec<usize>,
    pub(super) navigation_line_index:       Vec<usize>,
    pub(super) last_horizontal_column_page: usize,
}

impl CachedLinePlan {
    #[cfg(test)]
    pub(super) fn selectable_line_index(&self) -> &[usize] { &self.selectable_line_index }

    fn finish_navigation(&mut self) {
        self.navigation_line_index
            .clone_from(&self.selectable_line_index);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum CachedSurfaceWidth {
    #[default]
    NeedsRebuild,
    Rendered(u16),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FavoriteSelection {
    NoRowSelected,
    Row(FavoriteRowIdentity),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FavoriteRowMarker {
    Neither,
    Selected,
    Current,
    SelectedAndCurrent,
}

impl FavoriteRowMarker {
    fn for_row(
        selected: &FavoriteSelection,
        identity: &FavoriteRowIdentity,
        current_parameters: FavoriteRowCurrentParameters,
    ) -> Self {
        match selected {
            FavoriteSelection::Row(selected_identity) if selected_identity == identity => {
                match current_parameters {
                    FavoriteRowCurrentParameters::Matching => Self::SelectedAndCurrent,
                    FavoriteRowCurrentParameters::Unrecognized
                    | FavoriteRowCurrentParameters::Different => Self::Selected,
                }
            },
            FavoriteSelection::NoRowSelected | FavoriteSelection::Row(_) => {
                match current_parameters {
                    FavoriteRowCurrentParameters::Matching => Self::Current,
                    FavoriteRowCurrentParameters::Unrecognized
                    | FavoriteRowCurrentParameters::Different => Self::Neither,
                }
            },
        }
    }

    const fn prefix(self) -> &'static str {
        match self {
            Self::Neither => "   ",
            Self::Selected => "▸  ",
            Self::Current => " ● ",
            Self::SelectedAndCurrent => "▸● ",
        }
    }

    const fn is_selected(self) -> bool { matches!(self, Self::Selected | Self::SelectedAndCurrent) }
}

pub(super) fn rendered_line(
    line: &CachedOverlayLine,
    selected: &FavoriteSelection,
    lifecycle: FavoriteRowLifecycle,
    now: Instant,
) -> Line<'static> {
    match line {
        CachedOverlayLine::NonRow(line) => line.clone(),
        CachedOverlayLine::Row {
            identity,
            current_parameters,
            tail,
        } => {
            let marker = FavoriteRowMarker::for_row(selected, identity, *current_parameters);
            let line = Line::from(vec![Span::raw(marker.prefix()), Span::raw(tail.clone())]);
            if marker.is_selected() {
                line.style(selection_style(PaneFocusState::Active))
            } else {
                let color = match identity {
                    FavoriteRowIdentity::Recognized(_) => blend_color(
                        text_default(),
                        attract::ground(),
                        removal_alpha(lifecycle, now),
                    ),
                    FavoriteRowIdentity::Unrecognized(_) => blend_color(
                        error_color(),
                        attract::ground(),
                        removal_alpha(lifecycle, now),
                    ),
                };
                line.style(Style::default().fg(color))
            }
        },
    }
}

pub(super) fn row_lifecycle(state: &AppOverlay, line: &CachedOverlayLine) -> FavoriteRowLifecycle {
    let CachedOverlayLine::Row { identity, .. } = line else {
        return FavoriteRowLifecycle::Active;
    };
    let AppOverlay::Favorites(open_state) = state else {
        return FavoriteRowLifecycle::Active;
    };
    match (&open_state.content, identity) {
        (FavoritesOverlayContent::Rows(rows), FavoriteRowIdentity::Recognized(favorite_id)) => {
            match rows.row(*favorite_id) {
                FavoriteRowLookup::Found(row) => row.lifecycle,
                FavoriteRowLookup::Missing => FavoriteRowLifecycle::Active,
            }
        },
        (
            FavoritesOverlayContent::Rows(rows),
            FavoriteRowIdentity::Unrecognized(removal_locator),
        ) => rows
            .unrecognized
            .iter()
            .find(|row| row.removal_locator == *removal_locator)
            .map_or(FavoriteRowLifecycle::Active, |row| row.lifecycle),
        (
            FavoritesOverlayContent::OnlyUnrecognized(rows),
            FavoriteRowIdentity::Unrecognized(removal_locator),
        ) => rows
            .rows
            .iter()
            .find(|row| row.removal_locator == *removal_locator)
            .map_or(FavoriteRowLifecycle::Active, |row| row.lifecycle),
        (
            FavoritesOverlayContent::NoneSaved
            | FavoritesOverlayContent::OnlyUnrecognized(_)
            | FavoritesOverlayContent::LocationUnavailable
            | FavoritesOverlayContent::Unparseable { .. }
            | FavoritesOverlayContent::Unreadable { .. },
            FavoriteRowIdentity::Recognized(_) | FavoriteRowIdentity::Unrecognized(_),
        ) => FavoriteRowLifecycle::Active,
    }
}

fn removal_alpha(lifecycle: FavoriteRowLifecycle, now: Instant) -> u8 {
    let FavoriteRowLifecycle::Removing { since } = lifecycle else {
        return 0;
    };
    let elapsed = now.duration_since(since);
    if elapsed >= FAVORITE_REMOVAL_FADE {
        return u8::MAX;
    }
    let scaled =
        elapsed.as_nanos().saturating_mul(u128::from(u8::MAX)) / FAVORITE_REMOVAL_FADE.as_nanos();
    u8::try_from(scaled).unwrap_or(u8::MAX)
}

#[cfg(test)]
pub(super) fn removal_alpha_for_test(lifecycle: FavoriteRowLifecycle, now: Instant) -> u8 {
    removal_alpha(lifecycle, now)
}

pub(super) fn build_line_plan(
    content: &FavoritesOverlayContent,
    current_parameters: &OpenFavoritesCurrentParameters,
    bindings: &FavoritesSurfaceBindings,
    width: u16,
    horizontal_page: usize,
) -> CachedLinePlan {
    let mut plan = CachedLinePlan::default();
    match content {
        FavoritesOverlayContent::Rows(rows) => {
            let table_layouts = rows
                .sections
                .iter()
                .map(|section| FavoriteSectionTableLayout::measure(section, bindings))
                .collect::<Vec<_>>();
            plan.last_horizontal_column_page = table_layouts
                .iter()
                .map(|layout| layout.last_horizontal_column_page(width))
                .max()
                .unwrap_or(0);
            let horizontal_page = horizontal_page.min(plan.last_horizontal_column_page);
            for (section, table_layout) in rows.sections.iter().zip(&table_layouts) {
                append_section(
                    &mut plan,
                    section,
                    table_layout,
                    bindings,
                    current_parameters,
                    width,
                    horizontal_page,
                );
            }
            append_unrecognized(&mut plan, &rows.unrecognized);
        },
        FavoritesOverlayContent::NoneSaved => plan.lines.push(non_row_line(
            bindings.empty_notice(),
            Style::default().fg(text_default()),
        )),
        FavoritesOverlayContent::OnlyUnrecognized(rows) => {
            append_unrecognized(&mut plan, &rows.rows);
        },
        FavoritesOverlayContent::LocationUnavailable => plan.lines.push(non_row_line(
            "Favorites location unavailable -- no OS configuration directory".to_string(),
            Style::default().fg(error_color()),
        )),
        FavoritesOverlayContent::Unparseable { path, error } => {
            append_failure(&mut plan, "Favorites file is unparseable", path, error);
        },
        FavoritesOverlayContent::Unreadable { path, error } => {
            append_failure(&mut plan, "Favorites file is unreadable", path, error);
        },
    }
    plan.finish_navigation();
    plan
}

fn append_failure(plan: &mut CachedLinePlan, heading: &str, path: &Path, error: &str) {
    plan.lines.push(non_row_line(
        heading.to_string(),
        Style::default()
            .fg(error_color())
            .add_modifier(Modifier::BOLD),
    ));
    plan.lines.push(non_row_line(
        format!("  {}: {error}", path.display()),
        Style::default().fg(text_default()),
    ));
}

fn append_section(
    plan: &mut CachedLinePlan,
    section: &FavoriteModeSection,
    table_layout: &FavoriteSectionTableLayout,
    bindings: &FavoritesSurfaceBindings,
    current_parameters: &OpenFavoritesCurrentParameters,
    width: u16,
    horizontal_page: usize,
) {
    if !plan.lines.is_empty() {
        plan.lines
            .push(non_row_line(String::new(), Style::default()));
    }
    plan.lines.push(non_row_line(
        format!("Attract: {}", parameter_column::mode_label(section.mode)),
        Style::default()
            .fg(title_color())
            .add_modifier(Modifier::BOLD),
    ));

    let descriptors = parameter_column::column_descriptors(section.mode);
    let key_labels = bindings.column_labels(section.mode);
    let visible = table_layout.visible_parameter_columns(horizontal_page, width);
    let headings = descriptors
        .iter()
        .map(|descriptor| descriptor.heading)
        .collect::<Vec<_>>();
    plan.lines.push(non_row_line(
        table_layout::format_table_line(
            "Saved",
            &headings,
            table_layout.saved_width,
            &table_layout.parameter_widths,
            visible.clone(),
        ),
        Style::default()
            .fg(label_color())
            .add_modifier(Modifier::BOLD),
    ));
    plan.lines.push(non_row_line(
        table_layout::format_table_line(
            "",
            key_labels,
            table_layout.saved_width,
            &table_layout.parameter_widths,
            visible.clone(),
        ),
        Style::default().fg(label_color()),
    ));
    for row in &section.rows {
        let cells = descriptors
            .iter()
            .map(|descriptor| descriptor.render_value(row.settings))
            .collect::<Vec<_>>();
        let line_index = plan.lines.len();
        if row.lifecycle == FavoriteRowLifecycle::Active {
            plan.selectable_line_index.push(line_index);
        }
        plan.lines.push(CachedOverlayLine::Row {
            identity:           FavoriteRowIdentity::Recognized(row.id),
            current_parameters: if current_parameters.matches(row.settings) {
                FavoriteRowCurrentParameters::Matching
            } else {
                FavoriteRowCurrentParameters::Different
            },
            tail:               table_layout::format_table_tail(
                &row.saved,
                &cells,
                table_layout.saved_width,
                &table_layout.parameter_widths,
                visible.clone(),
            ),
        });
    }
}

fn append_unrecognized(plan: &mut CachedLinePlan, rows: &[UnrecognizedFavoriteView]) {
    if rows.is_empty() {
        return;
    }
    if !plan.lines.is_empty() {
        plan.lines
            .push(non_row_line(String::new(), Style::default()));
    }
    plan.lines.push(non_row_line(
        "Unrecognized favorites".to_string(),
        Style::default()
            .fg(error_color())
            .add_modifier(Modifier::BOLD),
    ));
    for row in rows {
        let line_index = plan.lines.len();
        if row.lifecycle == FavoriteRowLifecycle::Active {
            plan.selectable_line_index.push(line_index);
        }
        plan.lines.push(CachedOverlayLine::Row {
            identity:           FavoriteRowIdentity::Unrecognized(row.removal_locator.clone()),
            current_parameters: FavoriteRowCurrentParameters::Unrecognized,
            tail:               format!("{} = {:?} is not recognized", row.key, row.spelling),
        });
    }
}

fn non_row_line(text: String, style: Style) -> CachedOverlayLine {
    CachedOverlayLine::NonRow(Line::from(text).style(style))
}

pub(super) fn popup_width(area: Rect) -> u16 {
    area.width
        .saturating_sub(POPUP_SIDE_MARGIN)
        .min(POPUP_MAX_WIDTH)
        .max(area.width.min(POPUP_CHROME_WIDTH))
}

pub(super) fn popup_height_cap(area: Rect) -> u16 {
    let eighty_percent = u32::from(area.height).saturating_mul(80) / 100;
    u16::try_from(eighty_percent)
        .unwrap_or(u16::MAX)
        .max((POPUP_CHROME_HEIGHT + FOOTER_HEIGHT).min(area.height))
}

pub(super) fn wrapped_notice_height(message: &str, surface_width: u16) -> u16 {
    let available_width = usize::from(surface_width);
    if message.is_empty() || available_width == 0 {
        return 0;
    }

    let mut wrapped_lines = 0_usize;
    for logical_line in message.split('\n') {
        wrapped_lines = wrapped_lines.saturating_add(1);
        let mut used_width = 0_usize;
        for word in logical_line.split_whitespace() {
            let word_width = UnicodeWidthStr::width(word);
            let separator_width = usize::from(used_width > 0);
            if used_width
                .saturating_add(separator_width)
                .saturating_add(word_width)
                <= available_width
            {
                used_width = used_width
                    .saturating_add(separator_width)
                    .saturating_add(word_width);
                continue;
            }

            if used_width > 0 {
                wrapped_lines = wrapped_lines.saturating_add(1);
            }
            let full_lines = word_width / available_width;
            let remainder = word_width % available_width;
            if remainder == 0 && full_lines > 0 {
                wrapped_lines = wrapped_lines.saturating_add(full_lines.saturating_sub(1));
                used_width = available_width;
            } else {
                wrapped_lines = wrapped_lines.saturating_add(full_lines);
                used_width = remainder;
            }
        }
    }

    u16::try_from(wrapped_lines).unwrap_or(u16::MAX)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use tui_pane::FocusedPane;
    use tui_pane::Framework;
    use tui_pane::Keymap;

    use super::*;
    use crate::app::App;
    use crate::app::AppPaneId;
    use crate::app::OpenFavoritesCurrentParameters;
    use crate::favorites;
    use crate::favorites::FavoriteRowRecognition;
    use crate::favorites_overlay::content::FavoriteRowsView;
    use crate::keymap;

    const MOVING_BAND_ROW: &str = r#"
[[favorite]]
id = "01a03f60-9c14-7b41-8a02-1de4c7c9b332"
saved = "2026-08-26T11:02:44-07:00"
mode = "moving_band"
direction = "left"
width = 10
speed = 32
tail_speed = 72
fraying = "leading"
"#;

    const UNRECOGNIZED_ROW: &str = r#"
[[favorite]]
id = "01a03f62-9c14-7b41-8a02-1de4c7c9b334"
saved = "2026-08-26T14:31:05-07:00"
mode = "future_mode"
"#;

    const TWO_MATCHING_ROWS: &str = r#"
[[favorite]]
id = "01a03f60-9c14-7b41-8a02-1de4c7c9b332"
saved = "2026-08-26T11:02:44-07:00"
mode = "moving_band"
direction = "left"
width = 10
speed = 32
tail_speed = 72
fraying = "leading"

[[favorite]]
id = "01a03f64-9c14-7b41-8a02-1de4c7c9b336"
saved = "2026-08-26T15:02:44-07:00"
mode = "moving_band"
direction = "left"
width = 10
speed = 32
tail_speed = 72
fraying = "leading"
"#;

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

    fn current_parameters(text: &str) -> OpenFavoritesCurrentParameters {
        favorites::parse_rows_for_overlay_test(text)
            .expect("current-parameters fixture should parse")
            .recognized()
            .next()
            .expect("current-parameters fixture should have a recognized row")
            .settings
            .into()
    }

    #[test]
    fn navigation_indexes_only_rows_and_unrecognized_selection_uses_cursor_style() {
        let keymap = keymap_from("");
        let rows = favorites::parse_rows_for_overlay_test(&format!(
            "{MOVING_BAND_ROW}\n{UNRECOGNIZED_ROW}"
        ))
        .expect("mixed fixture should parse");
        let content = FavoritesOverlayContent::Rows(FavoriteRowsView::from(&rows));
        let current_parameters = current_parameters(MOVING_BAND_ROW);
        let bindings = FavoritesSurfaceBindings::resolve(&keymap);
        let plan = build_line_plan(&content, &current_parameters, &bindings, 100, 0);

        assert_eq!(plan.navigation_line_index.len(), 2);
        assert_eq!(
            plan.navigation_line_index,
            plan.selectable_line_index().to_vec()
        );
        assert!(
            plan.navigation_line_index
                .iter()
                .all(|index| matches!(plan.lines[*index], CachedOverlayLine::Row { .. }))
        );

        let unrecognized_line = plan
            .navigation_line_index
            .iter()
            .find_map(|index| match &plan.lines[*index] {
                CachedOverlayLine::Row {
                    identity: FavoriteRowIdentity::Unrecognized(_),
                    ..
                } => Some(&plan.lines[*index]),
                CachedOverlayLine::NonRow(_)
                | CachedOverlayLine::Row {
                    identity: FavoriteRowIdentity::Recognized(_),
                    ..
                } => None,
            })
            .expect("fixture should include an unrecognized row");
        let CachedOverlayLine::Row { identity, .. } = unrecognized_line else {
            panic!("selected line should be a row");
        };
        let rendered = rendered_line(
            unrecognized_line,
            &FavoriteSelection::Row(identity.clone()),
            FavoriteRowLifecycle::Active,
            Instant::now(),
        );

        assert_eq!(rendered.spans[0].content, "▸  ");
        assert!(!rendered.spans[0].content.contains('●'));
        assert_eq!(rendered.style, selection_style(PaneFocusState::Active));
    }

    #[test]
    fn unrecognized_row_is_never_marked_current() {
        let keymap = keymap_from("");
        let rows = favorites::parse_rows_for_overlay_test(UNRECOGNIZED_ROW)
            .expect("unrecognized fixture should parse");
        let content = FavoritesOverlayContent::Rows(FavoriteRowsView::from(&rows));
        let current_parameters = current_parameters(MOVING_BAND_ROW);
        let bindings = FavoritesSurfaceBindings::resolve(&keymap);
        let plan = build_line_plan(&content, &current_parameters, &bindings, 100, 0);
        let unrecognized_line = plan
            .lines
            .iter()
            .find(|line| {
                matches!(
                    line,
                    CachedOverlayLine::Row {
                        identity: FavoriteRowIdentity::Unrecognized(_),
                        ..
                    }
                )
            })
            .expect("fixture should include an unrecognized row");
        let CachedOverlayLine::Row { identity, .. } = unrecognized_line else {
            panic!("unrecognized line should be a row");
        };
        let prefixes = [
            FavoriteSelection::NoRowSelected,
            FavoriteSelection::Row(identity.clone()),
        ]
        .map(|selection| {
            rendered_line(
                unrecognized_line,
                &selection,
                FavoriteRowLifecycle::Active,
                Instant::now(),
            )
            .spans[0]
                .content
                .clone()
                .into_owned()
        });

        assert_eq!(prefixes, ["   ", "▸  "]);
        assert!(prefixes.iter().all(|prefix| !prefix.contains('●')));
    }

    #[test]
    fn row_marker_renders_every_selected_and_current_combination() {
        let rows = favorites::parse_rows_for_overlay_test(MOVING_BAND_ROW)
            .expect("moving-band fixture should parse");
        let favorite_id = rows
            .recognized()
            .next()
            .expect("moving-band fixture should have a recognized row")
            .id;
        let identity = FavoriteRowIdentity::Recognized(favorite_id);
        let cases = [
            (
                FavoriteSelection::NoRowSelected,
                FavoriteRowCurrentParameters::Different,
                "   ",
            ),
            (
                FavoriteSelection::Row(identity.clone()),
                FavoriteRowCurrentParameters::Different,
                "▸  ",
            ),
            (
                FavoriteSelection::NoRowSelected,
                FavoriteRowCurrentParameters::Matching,
                " ● ",
            ),
            (
                FavoriteSelection::Row(identity.clone()),
                FavoriteRowCurrentParameters::Matching,
                "▸● ",
            ),
        ];

        for (selection, current_parameters, expected_prefix) in cases {
            let line = CachedOverlayLine::Row {
                identity: identity.clone(),
                current_parameters,
                tail: "favorite".to_string(),
            };
            let rendered = rendered_line(
                &line,
                &selection,
                FavoriteRowLifecycle::Active,
                Instant::now(),
            );

            assert_eq!(rendered.spans[0].content, expected_prefix);
        }
    }

    #[test]
    fn every_row_with_matching_settings_is_current() {
        let keymap = keymap_from("");
        let rows = favorites::parse_rows_for_overlay_test(TWO_MATCHING_ROWS)
            .expect("matching favorites fixture should parse");
        let content = FavoritesOverlayContent::Rows(FavoriteRowsView::from(&rows));
        let current_parameters = current_parameters(MOVING_BAND_ROW);
        let bindings = FavoritesSurfaceBindings::resolve(&keymap);
        let plan = build_line_plan(&content, &current_parameters, &bindings, 100, 0);
        let now = Instant::now();
        let matching_prefixes = plan
            .lines
            .iter()
            .filter_map(|line| match line {
                CachedOverlayLine::Row {
                    identity: FavoriteRowIdentity::Recognized(_),
                    current_parameters: FavoriteRowCurrentParameters::Matching,
                    ..
                } => Some(
                    rendered_line(
                        line,
                        &FavoriteSelection::NoRowSelected,
                        FavoriteRowLifecycle::Active,
                        now,
                    )
                    .spans[0]
                        .content
                        .clone()
                        .into_owned(),
                ),
                CachedOverlayLine::NonRow(_)
                | CachedOverlayLine::Row {
                    identity: FavoriteRowIdentity::Unrecognized(_),
                    ..
                }
                | CachedOverlayLine::Row {
                    identity: FavoriteRowIdentity::Recognized(_),
                    current_parameters:
                        FavoriteRowCurrentParameters::Different
                        | FavoriteRowCurrentParameters::Unrecognized,
                    ..
                } => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(matching_prefixes, [" ● ", " ● "]);
    }

    #[test]
    fn removing_unrecognized_row_fades_from_its_error_color() {
        let rows = favorites::parse_rows_for_overlay_test(UNRECOGNIZED_ROW)
            .expect("unrecognized fixture should parse");
        let removal_locator = rows
            .iter()
            .find_map(|recognition| match recognition {
                FavoriteRowRecognition::Unrecognized {
                    removal_locator, ..
                } => Some(removal_locator.clone()),
                FavoriteRowRecognition::Recognized(_) => None,
            })
            .expect("fixture should include an unrecognized row");
        let line = CachedOverlayLine::Row {
            identity:           FavoriteRowIdentity::Unrecognized(removal_locator),
            current_parameters: FavoriteRowCurrentParameters::Unrecognized,
            tail:               "unrecognized".to_string(),
        };
        let started = Instant::now();
        let active = rendered_line(
            &line,
            &FavoriteSelection::NoRowSelected,
            FavoriteRowLifecycle::Active,
            started,
        );
        let removing = rendered_line(
            &line,
            &FavoriteSelection::NoRowSelected,
            FavoriteRowLifecycle::Removing { since: started },
            started + FAVORITE_REMOVAL_FADE / 2,
        );

        assert_eq!(
            active.style.fg,
            Some(blend_color(error_color(), attract::ground(), 0))
        );
        assert_ne!(removing.style.fg, active.style.fg);
    }
}
