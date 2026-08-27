//! App-owned modal for browsing attract-screen favorites.

use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;

use chrono::Datelike;
use chrono::Local;
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::Action;
use tui_pane::BandDirection;
use tui_pane::BandFraying;
use tui_pane::Bindings;
use tui_pane::ColumnSpec;
use tui_pane::ColumnWidths;
use tui_pane::KeySequence;
use tui_pane::Keymap;
use tui_pane::Mode;
use tui_pane::Pane;
use tui_pane::PaneFocusState;
use tui_pane::PopupFrame;
use tui_pane::Shortcuts;
use tui_pane::TabStop;
use tui_pane::TextDrift;
use tui_pane::TextFill;
use tui_pane::Viewport;
use tui_pane::ViewportOverflow;
use tui_pane::error_color;
use tui_pane::keep_visible_scroll_offset;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::selection_style;
use tui_pane::text_default;
use tui_pane::title_color;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::app::AppOverlay;
use crate::app::AppPaneId;
use crate::attract::AttractMode;
use crate::favorites;
use crate::favorites::Favorite;
use crate::favorites::FavoriteId;
use crate::favorites::FavoriteRowRecognition;
use crate::favorites::FavoriteRows;
use crate::favorites::FavoriteSettings;
use crate::favorites::FavoritesFileState;
use crate::favorites::UnrecognizedFavoriteValue;
use crate::globals::AppGlobalAction;

const FAVORITES_SCOPE_NAME: &str = "favorites";
const FAVORITES_SECTION_NAME: &str = "Favorites";
const POPUP_MAX_WIDTH: u16 = 110;
const POPUP_SIDE_MARGIN: u16 = 4;
const POPUP_BORDER_WIDTH: u16 = 2;
const POPUP_BORDER_HEIGHT: u16 = 2;
const FOOTER_HEIGHT: u16 = 1;
const COLUMN_GAP: usize = 2;
const MARKER_WIDTH: usize = 2;

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum FavoritesOverlayAction {
        SelectPrevious => ("select_previous", "Select the previous favorite");
        SelectNext => ("select_next", "Select the next favorite");
        PageColumnsLeft => ("page_columns_left", "Show the previous parameter column");
        PageColumnsRight => ("page_columns_right", "Show the next parameter column");
        Close => ("close", "Close favorites");
    }
}

/// Keymap host for the app-owned favorites modal.
pub(crate) struct FavoritesOverlayPane;

impl Pane<App> for FavoritesOverlayPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Favorites;

    fn mode() -> fn(&App) -> Mode<App> { |_app| Mode::Static }

    fn tab_stop() -> TabStop<App> { TabStop::never() }
}

impl Shortcuts<App> for FavoritesOverlayPane {
    type Actions = FavoritesOverlayAction;

    const SCOPE_NAME: &'static str = FAVORITES_SCOPE_NAME;
    const SECTION_NAME: &'static str = FAVORITES_SECTION_NAME;

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            [KeyCode::Up, 'k'] => FavoritesOverlayAction::SelectPrevious,
            [KeyCode::Down, 'j'] => FavoritesOverlayAction::SelectNext,
            [KeyCode::Left, 'h'] => FavoritesOverlayAction::PageColumnsLeft,
            [KeyCode::Right, 'l'] => FavoritesOverlayAction::PageColumnsRight,
            KeyCode::Esc => FavoritesOverlayAction::Close,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch }
}

fn dispatch(action: FavoritesOverlayAction, app: &mut App) {
    app.favorites_overlay.handle_action(action);
}

/// The content carried by an open favorites modal.
#[derive(Clone, Debug)]
pub(crate) enum FavoritesOverlayContent {
    /// At least one recognized favorite, with any unrecognized rows retained below it.
    Rows(FavoriteRowsView),
    /// No favorites file has been created yet, or its loaded row list is empty.
    NoneSaved,
    /// The file has rows, but this build recognizes none of them.
    OnlyUnrecognized(UnrecognizedFavoritesView),
    /// The operating system supplied no configuration directory.
    LocationUnavailable,
    /// The file exists but its TOML or row structure is invalid.
    Unparseable {
        /// Path holding the invalid content.
        path:  PathBuf,
        /// Parse failure text.
        error: String,
    },
    /// The file exists but could not be read.
    Unreadable {
        /// Path that could not be read.
        path:  PathBuf,
        /// File-system failure text.
        error: String,
    },
}

impl FavoritesOverlayContent {
    fn from_file_state(state: FavoritesFileState) -> Self {
        match state {
            FavoritesFileState::LocationUnavailable => Self::LocationUnavailable,
            FavoritesFileState::Missing { .. } => Self::NoneSaved,
            FavoritesFileState::Loaded { rows, .. } => {
                let view = FavoriteRowsView::from_rows(&rows);
                if view.saved_count() > 0 {
                    Self::Rows(view)
                } else if view.unrecognized.is_empty() {
                    Self::NoneSaved
                } else {
                    Self::OnlyUnrecognized(UnrecognizedFavoritesView {
                        rows: view.unrecognized,
                    })
                }
            },
            FavoritesFileState::Unparseable { path, error } => Self::Unparseable { path, error },
            FavoritesFileState::Unreadable { path, error } => Self::Unreadable { path, error },
        }
    }

    fn saved_count(&self) -> usize {
        match self {
            Self::Rows(rows) => rows.saved_count(),
            Self::NoneSaved
            | Self::OnlyUnrecognized(_)
            | Self::LocationUnavailable
            | Self::Unparseable { .. }
            | Self::Unreadable { .. } => 0,
        }
    }
}

/// Cached, display-ready recognized favorites and diagnostics.
#[derive(Clone, Debug)]
pub(crate) struct FavoriteRowsView {
    sections:     Vec<FavoriteModeSection>,
    unrecognized: Vec<UnrecognizedFavoriteView>,
}

impl FavoriteRowsView {
    fn from_rows(rows: &FavoriteRows) -> Self {
        let mut sections: Vec<FavoriteModeSection> = Vec::new();
        let mut unrecognized = Vec::new();
        for recognition in rows.iter() {
            match recognition {
                FavoriteRowRecognition::Recognized(favorite) => {
                    let mode = favorite.settings.mode();
                    if let Some(section) = sections.iter_mut().find(|section| section.mode == mode)
                    {
                        section.rows.push(FavoriteRowView::from_favorite(favorite));
                    } else {
                        sections.push(FavoriteModeSection {
                            mode,
                            rows: vec![FavoriteRowView::from_favorite(favorite)],
                        });
                    }
                },
                FavoriteRowRecognition::Unrecognized(value) => {
                    unrecognized.push(UnrecognizedFavoriteView::from(value));
                },
            }
        }
        Self {
            sections,
            unrecognized,
        }
    }

    fn saved_count(&self) -> usize { self.sections.iter().map(|section| section.rows.len()).sum() }
}

#[derive(Clone, Debug)]
struct FavoriteModeSection {
    mode: AttractMode,
    rows: Vec<FavoriteRowView>,
}

#[derive(Clone, Debug)]
struct FavoriteRowView {
    id:    FavoriteId,
    saved: String,
    cells: Vec<String>,
}

impl FavoriteRowView {
    fn from_favorite(favorite: &Favorite) -> Self {
        Self {
            id:    favorite.id,
            saved: format_timestamp(favorite),
            cells: favorite_cells(favorite.settings),
        }
    }
}

/// Display-ready rows a newer or misspelled file left unrecognized.
#[derive(Clone, Debug)]
pub(crate) struct UnrecognizedFavoritesView {
    rows: Vec<UnrecognizedFavoriteView>,
}

#[derive(Clone, Debug)]
struct UnrecognizedFavoriteView {
    key:      String,
    spelling: String,
}

impl From<&UnrecognizedFavoriteValue> for UnrecognizedFavoriteView {
    fn from(value: &UnrecognizedFavoriteValue) -> Self {
        Self {
            key:      value.key.clone(),
            spelling: value.spelling.clone(),
        }
    }
}

/// A live keymap lookup after domain meaning has replaced optionality.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum ResolvedBinding {
    /// The action has a primary binding.
    Bound(KeySequence),
    /// The action is deliberately unbound.
    #[default]
    Unbound,
}

impl From<Option<KeySequence>> for ResolvedBinding {
    fn from(binding: Option<KeySequence>) -> Self { binding.map_or(Self::Unbound, Self::Bound) }
}

impl ResolvedBinding {
    fn display_short(&self) -> String {
        match self {
            Self::Bound(sequence) => sequence.display_short(),
            Self::Unbound => String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParameterColumnDescriptor {
    heading:      &'static str,
    action_names: &'static [&'static str],
    separator:    &'static str,
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

const fn column_descriptors(mode: AttractMode) -> &'static [ParameterColumnDescriptor] {
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

#[derive(Clone, Debug, Default)]
struct FavoritesSurfaceBindings {
    columns:  Vec<ModeColumnBindings>,
    previous: ResolvedBinding,
    next:     ResolvedBinding,
    left:     ResolvedBinding,
    right:    ResolvedBinding,
    close:    ResolvedBinding,
    save:     ResolvedBinding,
}

impl FavoritesSurfaceBindings {
    fn resolve(keymap: &Keymap<App>) -> Self {
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
            close: resolve_pane_binding(keymap, AppPaneId::Favorites, "close"),
            save: resolve_global_binding(keymap, "save_favorite"),
        }
    }

    fn column_labels(&self, mode: AttractMode) -> &[String] {
        self.columns
            .iter()
            .find(|bindings| bindings.mode == mode)
            .map_or(&[], |bindings| bindings.labels.as_slice())
    }

    fn footer(&self, last_horizontal_column_page: usize) -> String {
        let movement = format!(
            "{}/{} move",
            self.previous.display_short(),
            self.next.display_short(),
        );
        let close = format!("{} close", self.close.display_short());
        if last_horizontal_column_page == 0 {
            format!("{movement}   {close}")
        } else {
            format!(
                "{movement}   {}/{} page   {close}",
                self.left.display_short(),
                self.right.display_short(),
            )
        }
    }

    fn empty_notice(&self) -> String {
        format!(
            "No favorites saved -- press {}, then {} while the attract screen is up",
            self.close.display_short(),
            self.save.display_short(),
        )
    }
}

fn resolve_pane_binding(
    keymap: &Keymap<App>,
    pane: AppPaneId,
    action_name: &str,
) -> ResolvedBinding {
    ResolvedBinding::from(keymap.key_for_toml_key(pane, action_name))
}

fn resolve_global_binding(keymap: &Keymap<App>, action_name: &str) -> ResolvedBinding {
    let binding = AppGlobalAction::from_toml_key(action_name).and_then(|action| {
        keymap
            .globals::<AppGlobalAction>()
            .and_then(|scope| scope.key_for(action))
            .cloned()
    });
    ResolvedBinding::from(binding)
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

#[derive(Clone, Debug)]
enum CachedOverlayLine {
    Static(Line<'static>),
    Favorite { id: FavoriteId, tail: String },
}

#[derive(Clone, Debug, Default)]
struct CachedLinePlan {
    lines:                       Vec<CachedOverlayLine>,
    selectable_line_index:       Vec<usize>,
    navigation_line_index:       Vec<usize>,
    last_horizontal_column_page: usize,
}

impl CachedLinePlan {
    fn finish_navigation(&mut self) {
        self.navigation_line_index
            .clone_from(&self.selectable_line_index);
        if let Some(last_selectable_line) = self.selectable_line_index.last().copied() {
            self.navigation_line_index
                .extend(last_selectable_line.saturating_add(1)..self.lines.len());
        } else {
            self.navigation_line_index.extend(0..self.lines.len());
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum CachedSurfaceWidth {
    #[default]
    NeverRendered,
    Rendered(u16),
}

/// The complete app-owned favorites modal controller.
pub(crate) struct FavoritesOverlay {
    state:                  AppOverlay,
    viewport:               Viewport,
    horizontal_column_page: usize,
    surface_bindings:       FavoritesSurfaceBindings,
    line_plan:              CachedLinePlan,
    cached_surface_width:   CachedSurfaceWidth,
}

impl Default for FavoritesOverlay {
    fn default() -> Self {
        Self {
            state:                  AppOverlay::Closed,
            viewport:               Viewport::new(),
            horizontal_column_page: 0,
            surface_bindings:       FavoritesSurfaceBindings::default(),
            line_plan:              CachedLinePlan::default(),
            cached_surface_width:   CachedSurfaceWidth::NeverRendered,
        }
    }
}

impl FavoritesOverlay {
    /// Whether the app modal is currently consuming input.
    pub(crate) const fn is_open(&self) -> bool { matches!(self.state, AppOverlay::Favorites(_)) }

    /// Reload favorites and open the matching content state.
    pub(crate) fn open(&mut self, keymap: &Keymap<App>) {
        self.open_with_loader(keymap, favorites::load);
    }

    fn open_with_loader(
        &mut self,
        keymap: &Keymap<App>,
        loader: impl FnOnce() -> FavoritesFileState,
    ) {
        self.open_file_state(loader(), keymap);
    }

    fn open_file_state(&mut self, state: FavoritesFileState, keymap: &Keymap<App>) {
        self.state = AppOverlay::Favorites(FavoritesOverlayContent::from_file_state(state));
        self.surface_bindings = FavoritesSurfaceBindings::resolve(keymap);
        self.horizontal_column_page = 0;
        let selected_rows = match &self.state {
            AppOverlay::Closed => 0,
            AppOverlay::Favorites(content) => content.saved_count(),
        };
        self.viewport.set_len(selected_rows);
        if let CachedSurfaceWidth::Rendered(width) = self.cached_surface_width {
            self.rebuild_line_plan(width);
        }
    }

    /// Apply one resolved modal action.
    pub(crate) fn handle_action(&mut self, action: FavoritesOverlayAction) {
        if !self.is_open() {
            return;
        }
        match action {
            FavoritesOverlayAction::SelectPrevious => self.viewport.up(),
            FavoritesOverlayAction::SelectNext => self.viewport.down(),
            FavoritesOverlayAction::PageColumnsLeft => {
                self.horizontal_column_page = self.horizontal_column_page.saturating_sub(1);
                self.rebuild_for_cached_width();
            },
            FavoritesOverlayAction::PageColumnsRight => {
                if self.horizontal_column_page < self.line_plan.last_horizontal_column_page {
                    self.horizontal_column_page = self.horizontal_column_page.saturating_add(1);
                    self.rebuild_for_cached_width();
                }
            },
            FavoritesOverlayAction::Close => self.close(),
        }
    }

    fn close(&mut self) {
        self.state = AppOverlay::Closed;
        self.viewport.clear_surface();
        self.line_plan = CachedLinePlan::default();
    }

    fn rebuild_for_cached_width(&mut self) {
        if let CachedSurfaceWidth::Rendered(width) = self.cached_surface_width {
            self.rebuild_line_plan(width);
        }
    }

    /// Draw only the cached lines intersecting the current viewport.
    pub(crate) fn render(&mut self, frame: &mut Frame<'_>) {
        if !self.is_open() {
            return;
        }
        let area = frame.area();
        let width = popup_width(area);
        let surface_width = width.saturating_sub(POPUP_BORDER_WIDTH);
        if self.cached_surface_width != CachedSurfaceWidth::Rendered(surface_width) {
            self.cached_surface_width = CachedSurfaceWidth::Rendered(surface_width);
            self.rebuild_line_plan(surface_width);
        }

        let desired_height = u16::try_from(self.line_plan.lines.len())
            .unwrap_or(u16::MAX)
            .saturating_add(FOOTER_HEIGHT)
            .saturating_add(POPUP_BORDER_HEIGHT);
        let height = desired_height.min(popup_height_cap(area)).min(area.height);
        let saved_count = match &self.state {
            AppOverlay::Closed => 0,
            AppOverlay::Favorites(content) => content.saved_count(),
        };
        let popup = PopupFrame {
            title: Some(format!(" Favorites -- {saved_count} saved ")),
            border_color: title_color(),
            width,
            height,
        }
        .render_with_areas(frame);
        let content_height = popup.inner.height.saturating_sub(FOOTER_HEIGHT);
        let content_area = Rect::new(
            popup.inner.x,
            popup.inner.y,
            popup.inner.width,
            content_height,
        );
        let footer_area = Rect::new(
            popup.inner.x,
            popup.inner.y.saturating_add(content_height),
            popup.inner.width,
            popup.inner.height.saturating_sub(content_height),
        );

        let active_line = self.update_vertical_viewport(content_area);
        let visible_rows = self.viewport.visible_rows();
        let line_count = self.line_plan.lines.len();
        let scroll_offset = self.viewport.scroll_offset();

        let end = scroll_offset.saturating_add(visible_rows).min(line_count);
        let selected_id = self.selected_id();
        let visible = self.line_plan.lines[scroll_offset..end]
            .iter()
            .map(|line| rendered_line(line, selected_id))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(visible), content_area);
        frame.render_widget(
            Paragraph::new(
                self.surface_bindings
                    .footer(self.line_plan.last_horizontal_column_page),
            )
            .style(Style::default().fg(label_color())),
            footer_area,
        );
        render_overflow_affordance(
            frame,
            popup.outer,
            ViewportOverflow::new(line_count, scroll_offset, visible_rows, active_line),
            Style::default().fg(label_color()),
        );
    }

    fn update_vertical_viewport(&mut self, content_area: Rect) -> usize {
        let active_line = self
            .line_plan
            .navigation_line_index
            .get(self.viewport.pos())
            .copied()
            .unwrap_or(0);
        let visible_rows = usize::from(content_area.height);
        let line_count = self.line_plan.lines.len();
        let scroll_offset = keep_visible_scroll_offset(active_line, visible_rows, line_count);
        self.viewport.set_content_area(content_area);
        self.viewport.set_viewport_rows(visible_rows);
        self.viewport.set_scroll_offset(scroll_offset);
        self.viewport.set_content_height(line_count);
        active_line
    }

    fn selected_id(&self) -> SelectedFavorite {
        let Some(&line_index) = self
            .line_plan
            .navigation_line_index
            .get(self.viewport.pos())
        else {
            return SelectedFavorite::NoFavoriteSelected;
        };
        match self.line_plan.lines.get(line_index) {
            Some(CachedOverlayLine::Favorite { id, .. }) => SelectedFavorite::Selected(*id),
            Some(CachedOverlayLine::Static(_)) | None => SelectedFavorite::NoFavoriteSelected,
        }
    }

    fn rebuild_line_plan(&mut self, width: u16) {
        self.line_plan = match &self.state {
            AppOverlay::Closed => CachedLinePlan::default(),
            AppOverlay::Favorites(content) => build_line_plan(
                content,
                &self.surface_bindings,
                width,
                self.horizontal_column_page,
            ),
        };
        self.horizontal_column_page = self
            .horizontal_column_page
            .min(self.line_plan.last_horizontal_column_page);
        self.viewport
            .set_len(self.line_plan.navigation_line_index.len());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectedFavorite {
    NoFavoriteSelected,
    Selected(FavoriteId),
}

fn rendered_line(line: &CachedOverlayLine, selected: SelectedFavorite) -> Line<'static> {
    match line {
        CachedOverlayLine::Static(line) => line.clone(),
        CachedOverlayLine::Favorite { id, tail } => {
            let is_selected = selected == SelectedFavorite::Selected(*id);
            let marker = if is_selected { "▸ " } else { "  " };
            let line = Line::from(vec![Span::raw(marker), Span::raw(tail.clone())]);
            if is_selected {
                line.style(selection_style(PaneFocusState::Active))
            } else {
                line.style(Style::default().fg(text_default()))
            }
        },
    }
}

fn build_line_plan(
    content: &FavoritesOverlayContent,
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
                    width,
                    horizontal_page,
                );
            }
            append_unrecognized(&mut plan, &rows.unrecognized);
        },
        FavoritesOverlayContent::NoneSaved => plan.lines.push(static_line(
            bindings.empty_notice(),
            Style::default().fg(text_default()),
        )),
        FavoritesOverlayContent::OnlyUnrecognized(rows) => {
            append_unrecognized(&mut plan, &rows.rows);
        },
        FavoritesOverlayContent::LocationUnavailable => plan.lines.push(static_line(
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
    plan.lines.push(static_line(
        heading.to_string(),
        Style::default()
            .fg(error_color())
            .add_modifier(Modifier::BOLD),
    ));
    plan.lines.push(static_line(
        format!("  {}: {error}", path.display()),
        Style::default().fg(text_default()),
    ));
}

fn append_section(
    plan: &mut CachedLinePlan,
    section: &FavoriteModeSection,
    table_layout: &FavoriteSectionTableLayout,
    bindings: &FavoritesSurfaceBindings,
    width: u16,
    horizontal_page: usize,
) {
    if !plan.lines.is_empty() {
        plan.lines
            .push(static_line(String::new(), Style::default()));
    }
    plan.lines.push(static_line(
        format!("Attract: {}", mode_label(section.mode)),
        Style::default()
            .fg(title_color())
            .add_modifier(Modifier::BOLD),
    ));

    let descriptors = column_descriptors(section.mode);
    let key_labels = bindings.column_labels(section.mode);
    let visible = table_layout.visible_parameter_columns(horizontal_page, width);
    let headings = descriptors
        .iter()
        .map(|descriptor| descriptor.heading)
        .collect::<Vec<_>>();
    plan.lines.push(static_line(
        format_table_line(
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
    plan.lines.push(static_line(
        format_table_line(
            "",
            key_labels,
            table_layout.saved_width,
            &table_layout.parameter_widths,
            visible.clone(),
        ),
        Style::default().fg(label_color()),
    ));
    for row in &section.rows {
        let line_index = plan.lines.len();
        plan.selectable_line_index.push(line_index);
        plan.lines.push(CachedOverlayLine::Favorite {
            id:   row.id,
            tail: format_table_tail(
                &row.saved,
                &row.cells,
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
            .push(static_line(String::new(), Style::default()));
    }
    plan.lines.push(static_line(
        "Unrecognized favorites".to_string(),
        Style::default()
            .fg(error_color())
            .add_modifier(Modifier::BOLD),
    ));
    plan.lines.extend(rows.iter().map(|row| {
        static_line(
            format!("  {} = {:?} is not recognized", row.key, row.spelling),
            Style::default().fg(error_color()),
        )
    }));
}

fn static_line(text: String, style: Style) -> CachedOverlayLine {
    CachedOverlayLine::Static(Line::from(text).style(style))
}

#[derive(Clone, Debug)]
struct FavoriteSectionTableLayout {
    saved_width:      u16,
    parameter_widths: ColumnWidths,
}

impl FavoriteSectionTableLayout {
    fn measure(section: &FavoriteModeSection, bindings: &FavoritesSurfaceBindings) -> Self {
        let descriptors = column_descriptors(section.mode);
        let key_labels = bindings.column_labels(section.mode);
        Self {
            saved_width:      measured_saved_width(&section.rows),
            parameter_widths: measured_parameter_widths(descriptors, key_labels, &section.rows),
        }
    }

    fn visible_parameter_columns(&self, horizontal_page: usize, width: u16) -> Range<usize> {
        visible_parameter_columns(
            horizontal_page,
            width,
            self.saved_width,
            &self.parameter_widths,
        )
    }

    fn last_horizontal_column_page(&self, width: u16) -> usize {
        let column_count = self.parameter_widths.to_constraints().len();
        (0..column_count)
            .find(|page| self.visible_parameter_columns(*page, width).end == column_count)
            .unwrap_or_else(|| column_count.saturating_sub(1))
    }
}

fn measured_saved_width(rows: &[FavoriteRowView]) -> u16 {
    rows.iter().fold(5_u16, |width, row| {
        width.max(u16::try_from(UnicodeWidthStr::width(row.saved.as_str())).unwrap_or(u16::MAX))
    })
}

fn measured_parameter_widths(
    descriptors: &[ParameterColumnDescriptor],
    key_labels: &[String],
    rows: &[FavoriteRowView],
) -> ColumnWidths {
    let specs = descriptors
        .iter()
        .map(|descriptor| {
            ColumnSpec::fit(
                u16::try_from(UnicodeWidthStr::width(descriptor.heading)).unwrap_or(u16::MAX),
            )
        })
        .collect();
    let mut widths = ColumnWidths::new(specs);
    for (column, label) in key_labels.iter().enumerate() {
        widths.observe_cell_usize(column, UnicodeWidthStr::width(label.as_str()));
    }
    for row in rows {
        for (column, cell) in row.cells.iter().enumerate() {
            widths.observe_cell_usize(column, UnicodeWidthStr::width(cell.as_str()));
        }
    }
    widths
}

fn visible_parameter_columns(
    horizontal_page: usize,
    width: u16,
    saved_width: u16,
    parameter_widths: &ColumnWidths,
) -> Range<usize> {
    let column_count = parameter_widths.to_constraints().len();
    if column_count == 0 {
        return 0..0;
    }
    let start = horizontal_page.min(column_count - 1);
    let pinned = MARKER_WIDTH.saturating_add(usize::from(saved_width));
    let available = usize::from(width).saturating_sub(pinned);
    let mut used: usize = 0;
    let mut end = start;
    for column in start..column_count {
        let cost = usize::from(parameter_widths.get(column)).saturating_add(COLUMN_GAP);
        if end > start && used.saturating_add(cost) > available {
            break;
        }
        used = used.saturating_add(cost);
        end = column + 1;
    }
    start..end
}

fn format_table_line<T: AsRef<str>>(
    saved: &str,
    cells: &[T],
    saved_width: u16,
    parameter_widths: &ColumnWidths,
    visible: Range<usize>,
) -> String {
    format!(
        "  {}",
        format_table_tail(saved, cells, saved_width, parameter_widths, visible)
    )
}

fn format_table_tail<T: AsRef<str>>(
    saved: &str,
    cells: &[T],
    saved_width: u16,
    parameter_widths: &ColumnWidths,
    visible: Range<usize>,
) -> String {
    let mut line = String::new();
    push_display_padded(&mut line, saved, usize::from(saved_width));
    for column in visible {
        line.push_str(&" ".repeat(COLUMN_GAP));
        if let Some(cell) = cells.get(column) {
            push_display_padded(
                &mut line,
                cell.as_ref(),
                usize::from(parameter_widths.get(column)),
            );
        }
    }
    line
}

fn push_display_padded(line: &mut String, value: &str, width: usize) {
    line.push_str(value);
    line.push_str(&" ".repeat(width.saturating_sub(UnicodeWidthStr::width(value))));
}

fn popup_width(area: Rect) -> u16 {
    area.width
        .saturating_sub(POPUP_SIDE_MARGIN)
        .min(POPUP_MAX_WIDTH)
        .max(area.width.min(POPUP_BORDER_WIDTH))
}

fn popup_height_cap(area: Rect) -> u16 {
    let eighty_percent = u32::from(area.height).saturating_mul(80) / 100;
    u16::try_from(eighty_percent)
        .unwrap_or(u16::MAX)
        .max((POPUP_BORDER_HEIGHT + FOOTER_HEIGHT).min(area.height))
}

fn format_timestamp(favorite: &Favorite) -> String {
    if favorite.saved.year() == Local::now().year() {
        favorite.saved.format("%d %b %H:%M:%S").to_string()
    } else {
        favorite.saved.format("%d %b %Y %H:%M:%S").to_string()
    }
}

fn favorite_cells(settings: FavoriteSettings) -> Vec<String> {
    match settings {
        FavoriteSettings::MovingBand(settings) => vec![
            direction_name(settings.direction).to_string(),
            settings.width.to_string(),
            settings.speed.to_string(),
            settings.tail_speed.to_string(),
            fraying_name(settings.fraying).to_string(),
        ],
        FavoriteSettings::MovingText(settings) => vec![
            direction_name(settings.direction).to_string(),
            settings.speed.to_string(),
            settings.spread.to_string(),
            drift_name(settings.drift).to_string(),
            text_fill_name(settings.fill).to_string(),
        ],
        FavoriteSettings::Pixelate(settings) => vec![
            direction_name(settings.direction).to_string(),
            settings.speed.to_string(),
            settings.wave_percent.to_string(),
            settings.block_columns.to_string(),
            pixel_resolve_name(settings.resolve).to_string(),
            pixel_fill_name(settings.fill).to_string(),
        ],
    }
}

const fn mode_label(mode: AttractMode) -> &'static str {
    match mode {
        AttractMode::MovingBand => "Moving Band",
        AttractMode::MovingText => "Moving Text",
        AttractMode::Pixelate => "Pixelate",
    }
}

const fn direction_name(direction: BandDirection) -> &'static str {
    match direction {
        BandDirection::Left => "left",
        BandDirection::Right => "right",
        BandDirection::Up => "up",
        BandDirection::Down => "down",
    }
}

const fn fraying_name(fraying: BandFraying) -> &'static str {
    match fraying {
        BandFraying::Trailing => "trailing",
        BandFraying::Both => "both",
        BandFraying::Leading => "leading",
        BandFraying::Neither => "neither",
    }
}

const fn drift_name(drift: TextDrift) -> &'static str {
    match drift {
        TextDrift::Together => "together",
        TextDrift::Apart => "apart",
    }
}

const fn text_fill_name(fill: TextFill) -> &'static str {
    match fill {
        TextFill::Bars => "bars",
        TextFill::Glyphs => "glyphs",
    }
}

const fn pixel_resolve_name(resolve: tui_pane::PixelResolve) -> &'static str {
    match resolve {
        tui_pane::PixelResolve::Blend => "blend",
        tui_pane::PixelResolve::Step => "step",
        tui_pane::PixelResolve::Scatter => "scatter",
    }
}

const fn pixel_fill_name(fill: tui_pane::PixelFill) -> &'static str {
    match fill {
        tui_pane::PixelFill::Solid => "solid",
        tui_pane::PixelFill::Shades => "shades",
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

    use super::*;
    use crate::favorites::parse_rows_for_overlay_test;
    use crate::keymap::build_keymap;

    const RECOGNIZED_ROWS: &str = r#"
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
id = "01a03f5e-9c14-7b41-8a02-1de4c7c9b330"
saved = "2026-08-26T09:02:44-07:00"
mode = "moving_band"
direction = "right"
width = 12
speed = 40
tail_speed = 96
fraying = "both"

[[favorite]]
id = "01a03f61-9c14-7b41-8a02-1de4c7c9b333"
saved = "2026-08-26T14:31:05-07:00"
mode = "pixelate"
direction = "left"
speed = 24
wave_percent = 145
block_columns = 6
resolve = "scatter"
fill = "solid"
"#;

    const UNRECOGNIZED_ROWS: &str = r#"
[[favorite]]
id = "01a03f62-9c14-7b41-8a02-1de4c7c9b334"
saved = "2026-08-26T14:31:05-07:00"
mode = "future_mode"

[[favorite]]
id = "01a03f63-9c14-7b41-8a02-1de4c7c9b335"
saved = "2026-08-26T14:32:05-07:00"
mode = "pixelate"
direction = "left"
speed = 24
wave_percent = 145
block_columns = 6
resolve = "mist"
fill = "solid"
"#;

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

    fn keymap_from(toml: &str) -> Keymap<App> {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("keymap.toml");
        if !toml.is_empty() {
            fs::write(&path, toml).expect("test keymap should be written");
        }
        let mut framework = Framework::new(FocusedPane::App(AppPaneId::Main));
        build_keymap(&mut framework, (!toml.is_empty()).then_some(path))
            .expect("test keymap should resolve")
    }

    fn loaded_state(text: &str) -> FavoritesFileState {
        FavoritesFileState::Loaded {
            path: PathBuf::from("/tmp/favorites.toml"),
            rows: parse_rows_for_overlay_test(text).expect("favorites fixture should parse"),
        }
    }

    fn plan_text(plan: &CachedLinePlan) -> Vec<String> {
        plan.lines
            .iter()
            .map(|line| match line {
                CachedOverlayLine::Static(line) => line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect(),
                CachedOverlayLine::Favorite { tail, .. } => format!("  {tail}"),
            })
            .collect()
    }

    fn display_column(line: &str, value: &str) -> usize {
        let byte_index = line
            .find(value)
            .expect("rendered table value should be present");
        UnicodeWidthStr::width(&line[..byte_index])
    }

    fn moving_band_table_layout(keymap: &Keymap<App>) -> FavoriteSectionTableLayout {
        let rows =
            parse_rows_for_overlay_test(MOVING_BAND_ROW).expect("moving-band fixture should parse");
        let view = FavoriteRowsView::from_rows(&rows);
        let bindings = FavoritesSurfaceBindings::resolve(keymap);
        FavoriteSectionTableLayout::measure(&view.sections[0], &bindings)
    }

    fn open_at_width(
        state: FavoritesFileState,
        keymap: &Keymap<App>,
        width: u16,
    ) -> FavoritesOverlay {
        let mut overlay = FavoritesOverlay::default();
        overlay.open_file_state(state, keymap);
        overlay.cached_surface_width = CachedSurfaceWidth::Rendered(width);
        overlay.rebuild_line_plan(width);
        overlay
    }

    #[test]
    fn modal_scope_has_only_the_phase_four_actions() {
        let scope = FavoritesOverlayPane::defaults().into_scope_map();
        let cases = [
            (
                KeyBind::from(KeyCode::Up),
                FavoritesOverlayAction::SelectPrevious,
            ),
            (KeyBind::from('k'), FavoritesOverlayAction::SelectPrevious),
            (
                KeyBind::from(KeyCode::Down),
                FavoritesOverlayAction::SelectNext,
            ),
            (KeyBind::from('j'), FavoritesOverlayAction::SelectNext),
            (
                KeyBind::from(KeyCode::Left),
                FavoritesOverlayAction::PageColumnsLeft,
            ),
            (KeyBind::from('h'), FavoritesOverlayAction::PageColumnsLeft),
            (
                KeyBind::from(KeyCode::Right),
                FavoritesOverlayAction::PageColumnsRight,
            ),
            (KeyBind::from('l'), FavoritesOverlayAction::PageColumnsRight),
            (KeyBind::from(KeyCode::Esc), FavoritesOverlayAction::Close),
        ];
        for (binding, action) in cases {
            assert_eq!(scope.action_for(&binding), Some(action));
        }
        assert_eq!(scope.action_for(&KeyBind::from(KeyCode::Enter)), None);
        assert_eq!(scope.action_for(&KeyBind::from('x')), None);
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
        assert_eq!(bindings.footer(1), "w/s move   a/d page   z close");
        assert_eq!(bindings.footer(0), "w/s move   z close");
        assert_eq!(
            bindings.empty_notice(),
            "No favorites saved -- press z, then y while the attract screen is up"
        );
        assert!(!bindings.footer(1).contains("enter"));
        assert!(!bindings.footer(1).contains("x delete"));
    }

    #[test]
    fn every_file_state_maps_to_a_distinct_rendered_position() {
        let keymap = keymap_from("");
        let cases = [
            (
                FavoritesFileState::Missing {
                    path: PathBuf::from("/tmp/missing.toml"),
                },
                "No favorites saved",
            ),
            (
                FavoritesFileState::LocationUnavailable,
                "location unavailable",
            ),
            (
                FavoritesFileState::Unparseable {
                    path:  PathBuf::from("/tmp/bad.toml"),
                    error: "bad TOML".to_string(),
                },
                "/tmp/bad.toml: bad TOML",
            ),
            (
                FavoritesFileState::Unreadable {
                    path:  PathBuf::from("/tmp/closed.toml"),
                    error: "permission denied".to_string(),
                },
                "/tmp/closed.toml: permission denied",
            ),
            (loaded_state(RECOGNIZED_ROWS), "Attract: Moving Band"),
        ];

        for (state, expected) in cases {
            let overlay = open_at_width(state, &keymap, 100);
            let rendered = plan_text(&overlay.line_plan).join("\n");
            assert!(
                rendered.contains(expected),
                "{rendered:?} should contain {expected:?}"
            );
        }
    }

    #[test]
    fn unknown_mode_and_misspelled_enum_are_visible_but_not_selectable() {
        let keymap = keymap_from("");
        let overlay = open_at_width(loaded_state(UNRECOGNIZED_ROWS), &keymap, 100);
        let rendered = plan_text(&overlay.line_plan).join("\n");

        assert!(matches!(
            overlay.state,
            AppOverlay::Favorites(FavoritesOverlayContent::OnlyUnrecognized(_))
        ));
        assert!(rendered.contains("mode = \"future_mode\" is not recognized"));
        assert!(rendered.contains("resolve = \"mist\" is not recognized"));
        assert!(overlay.line_plan.selectable_line_index.is_empty());
        assert_eq!(overlay.viewport.len(), overlay.line_plan.lines.len());
        assert!(!rendered.contains("No favorites saved"));
    }

    #[test]
    fn all_unrecognized_rows_scroll_to_the_last_diagnostic() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(UNRECOGNIZED_ROWS), &keymap, 100);

        for _ in 1..overlay.viewport.len() {
            overlay.handle_action(FavoritesOverlayAction::SelectNext);
        }
        let active_line = overlay.update_vertical_viewport(Rect::new(0, 0, 100, 1));
        let rendered = plan_text(&overlay.line_plan);

        assert_eq!(active_line, overlay.line_plan.lines.len() - 1);
        assert_eq!(overlay.viewport.scroll_offset(), active_line);
        assert!(rendered[active_line].contains("resolve = \"mist\" is not recognized"));
        assert_eq!(overlay.selected_id(), SelectedFavorite::NoFavoriteSelected);
    }

    #[test]
    fn mixed_rows_scroll_past_the_last_favorite_to_diagnostics() {
        let keymap = keymap_from("");
        let mixed_rows = format!("{RECOGNIZED_ROWS}\n{UNRECOGNIZED_ROWS}");
        let mut overlay = open_at_width(loaded_state(&mixed_rows), &keymap, 100);
        let last_favorite_line = overlay
            .line_plan
            .selectable_line_index
            .last()
            .copied()
            .expect("mixed fixture should contain recognized favorites");

        for _ in 1..overlay.viewport.len() {
            overlay.handle_action(FavoritesOverlayAction::SelectNext);
        }
        let active_line = overlay.update_vertical_viewport(Rect::new(0, 0, 100, 2));
        let rendered = plan_text(&overlay.line_plan);

        assert!(active_line > last_favorite_line);
        assert_eq!(active_line, overlay.line_plan.lines.len() - 1);
        assert_eq!(
            overlay.viewport.scroll_offset(),
            overlay.line_plan.lines.len() - 2
        );
        assert!(rendered[active_line].contains("resolve = \"mist\" is not recognized"));
        assert_eq!(overlay.selected_id(), SelectedFavorite::NoFavoriteSelected);
    }

    #[test]
    fn selection_walks_sections_and_scrolls_to_stay_visible() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(RECOGNIZED_ROWS), &keymap, 100);

        assert_eq!(overlay.viewport.len(), 3);
        let first_line = overlay.line_plan.selectable_line_index[0];
        let second_line = overlay.line_plan.selectable_line_index[1];
        let pixel_line = overlay.line_plan.selectable_line_index[2];
        assert!(first_line < second_line && second_line < pixel_line);
        let rendered = plan_text(&overlay.line_plan);
        assert!(rendered[first_line].contains("10"));
        assert!(rendered[second_line].contains("12"));
        overlay.handle_action(FavoritesOverlayAction::SelectNext);
        overlay.handle_action(FavoritesOverlayAction::SelectNext);
        assert_eq!(overlay.viewport.pos(), 2);
        let active_line = overlay.update_vertical_viewport(Rect::new(0, 0, 100, 2));
        assert_eq!(active_line, pixel_line);
        assert!(overlay.viewport.scroll_offset() > 0);
        assert!(active_line < overlay.viewport.scroll_offset() + overlay.viewport.visible_rows());
        overlay.handle_action(FavoritesOverlayAction::SelectPrevious);
        assert_eq!(overlay.viewport.pos(), 1);
    }

    #[test]
    fn timestamps_keep_seconds_and_add_the_year_only_when_needed() {
        let current_year = Local::now().year();
        let old_year = current_year - 1;
        let rows = parse_rows_for_overlay_test(&format!(
            r#"
[[favorite]]
id = "01a03f64-9c14-7b41-8a02-1de4c7c9b336"
saved = "{current_year}-01-02T03:04:05-05:00"
mode = "moving_band"
direction = "left"
width = 10
speed = 32
tail_speed = 72
fraying = "leading"

[[favorite]]
id = "01a03f65-9c14-7b41-8a02-1de4c7c9b337"
saved = "{old_year}-01-02T03:04:05-05:00"
mode = "moving_band"
direction = "right"
width = 12
speed = 40
tail_speed = 96
fraying = "both"
"#
        ))
        .expect("timestamp fixture should parse");
        let view = FavoriteRowsView::from_rows(&rows);
        let saved = view.sections[0]
            .rows
            .iter()
            .map(|row| row.saved.as_str())
            .collect::<Vec<_>>();

        assert!(saved.contains(&"02 Jan 03:04:05"));
        assert!(saved.contains(&format!("02 Jan {old_year} 03:04:05").as_str()));
    }

    #[test]
    fn horizontal_pages_keep_saved_pinned_and_cells_aligned() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(RECOGNIZED_ROWS), &keymap, 40);
        let first_page = plan_text(&overlay.line_plan);
        let first_header = first_page
            .iter()
            .find(|line| line.contains("Saved") && line.contains("Direction"))
            .expect("first page should show Direction");
        assert!(first_header.starts_with("  Saved"));

        overlay.handle_action(FavoritesOverlayAction::PageColumnsRight);
        let second_page = plan_text(&overlay.line_plan);
        let band_heading = second_page
            .iter()
            .position(|line| line == "Attract: Moving Band")
            .expect("band section should remain present");
        let header = &second_page[band_heading + 1];
        let key_line = &second_page[band_heading + 2];
        let row = &second_page[band_heading + 3];
        let column_start = header.find("Width").expect("second page should show Width");
        assert!(header.starts_with("  Saved"));
        assert_eq!(
            key_line.find("-/+").expect("width keys should align"),
            column_start
        );
        assert_eq!(
            row.find("10").expect("width cell should align"),
            column_start
        );
        assert!(!header.contains("Direction"));
    }

    #[test]
    fn wide_table_does_not_page_or_advertise_paging() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(RECOGNIZED_ROWS), &keymap, 100);
        let before = plan_text(&overlay.line_plan);

        overlay.handle_action(FavoritesOverlayAction::PageColumnsRight);

        assert_eq!(overlay.horizontal_column_page, 0);
        assert_eq!(plan_text(&overlay.line_plan), before);
        assert_eq!(overlay.line_plan.last_horizontal_column_page, 0);
        assert!(
            !overlay
                .surface_bindings
                .footer(overlay.line_plan.last_horizontal_column_page)
                .contains("page")
        );
    }

    #[test]
    fn moving_band_pages_end_at_its_last_column_and_left_reverses_right() {
        let keymap = keymap_from("");
        let table_layout = moving_band_table_layout(&keymap);
        let width = u16::try_from(
            MARKER_WIDTH
                + usize::from(table_layout.saved_width)
                + COLUMN_GAP
                + usize::from(table_layout.parameter_widths.get(0)),
        )
        .expect("exact table width should fit u16");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, width);

        for _ in 0..BAND_COLUMNS.len() {
            overlay.handle_action(FavoritesOverlayAction::PageColumnsRight);
        }
        let last_page = overlay.horizontal_column_page;
        let last_page_text = plan_text(&overlay.line_plan);

        assert_eq!(last_page, BAND_COLUMNS.len() - 1);
        assert_eq!(overlay.line_plan.last_horizontal_column_page, last_page);
        assert!(last_page_text[1].contains("Fraying"));
        overlay.handle_action(FavoritesOverlayAction::PageColumnsRight);
        assert_eq!(overlay.horizontal_column_page, last_page);

        overlay.handle_action(FavoritesOverlayAction::PageColumnsLeft);
        assert_eq!(overlay.horizontal_column_page, last_page - 1);
        assert!(plan_text(&overlay.line_plan)[1].contains("Tail"));
    }

    #[test]
    fn exactly_fitting_parameter_column_is_visible() {
        let keymap = keymap_from("");
        let table_layout = moving_band_table_layout(&keymap);
        let width = u16::try_from(
            MARKER_WIDTH
                + usize::from(table_layout.saved_width)
                + COLUMN_GAP
                + usize::from(table_layout.parameter_widths.get(0)),
        )
        .expect("exact table width should fit u16");

        assert_eq!(table_layout.visible_parameter_columns(0, width), 0..1);
        let header = format_table_line(
            "Saved",
            &BAND_COLUMNS.map(|descriptor| descriptor.heading),
            table_layout.saved_width,
            &table_layout.parameter_widths,
            0..1,
        );
        assert_eq!(UnicodeWidthStr::width(header.as_str()), usize::from(width));
    }

    #[test]
    fn wide_binding_keeps_headers_keys_and_cells_aligned() {
        let keymap = keymap_from(
            r#"
[attract_moving_band]
travel_left = "界"
"#,
        );
        let overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let rendered = plan_text(&overlay.line_plan);
        let header = &rendered[1];
        let key_line = &rendered[2];
        let row = &rendered[3];

        assert_eq!(
            display_column(header, "Direction"),
            display_column(key_line, "界")
        );
        assert_eq!(
            display_column(header, "Direction"),
            display_column(row, "left")
        );
        assert_eq!(
            display_column(header, "Width"),
            display_column(key_line, "-/+")
        );
        assert_eq!(display_column(header, "Width"), display_column(row, "10"));
    }

    #[test]
    fn too_narrow_table_still_renders_one_clipped_parameter_column() {
        let keymap = keymap_from("");
        let table_layout = moving_band_table_layout(&keymap);
        let exact_width = usize::from(table_layout.saved_width)
            + MARKER_WIDTH
            + COLUMN_GAP
            + usize::from(table_layout.parameter_widths.get(0));
        let width = u16::try_from(exact_width - 1).expect("narrow table width should fit u16");
        let visible = table_layout.visible_parameter_columns(0, width);

        assert_eq!(visible, 0..1);
        let headings = BAND_COLUMNS.map(|descriptor| descriptor.heading);
        let labels = FavoritesSurfaceBindings::resolve(&keymap)
            .column_labels(AttractMode::MovingBand)
            .to_vec();
        let rows =
            parse_rows_for_overlay_test(MOVING_BAND_ROW).expect("moving-band fixture should parse");
        let view = FavoriteRowsView::from_rows(&rows);
        let row = &view.sections[0].rows[0];
        let header = format_table_line(
            "Saved",
            &headings,
            table_layout.saved_width,
            &table_layout.parameter_widths,
            visible.clone(),
        );
        let key_line = format_table_line(
            "",
            &labels,
            table_layout.saved_width,
            &table_layout.parameter_widths,
            visible.clone(),
        );
        let cell_line = format_table_line(
            &row.saved,
            &row.cells,
            table_layout.saved_width,
            &table_layout.parameter_widths,
            visible,
        );

        assert!(UnicodeWidthStr::width(header.as_str()) > usize::from(width));
        assert_eq!(
            display_column(&header, "Direction"),
            display_column(&key_line, "←↑↓→")
        );
        assert_eq!(
            display_column(&header, "Direction"),
            display_column(&cell_line, "left")
        );
    }

    #[test]
    fn reopening_runs_the_loader_again_and_sees_an_appended_row() {
        let keymap = keymap_from("");
        let mut overlay = FavoritesOverlay::default();
        let mut loads = 0;
        overlay.open_with_loader(&keymap, || {
            loads += 1;
            FavoritesFileState::Missing {
                path: PathBuf::from("/tmp/favorites.toml"),
            }
        });
        assert_eq!(loads, 1);
        assert_eq!(overlay.viewport.len(), 0);
        overlay.handle_action(FavoritesOverlayAction::Close);

        overlay.open_with_loader(&keymap, || {
            loads += 1;
            loaded_state(RECOGNIZED_ROWS)
        });
        assert_eq!(loads, 2);
        assert_eq!(overlay.viewport.len(), 3);
    }

    #[test]
    fn unbound_labels_cross_the_same_named_boundary_as_bound_labels() {
        let unbound = ResolvedBinding::from(None);
        let bound = ResolvedBinding::from(Some(KeySequence::from(KeyBind::ctrl('s'))));

        assert_eq!(unbound, ResolvedBinding::Unbound);
        assert_eq!(unbound.display_short(), "");
        assert_eq!(bound.display_short(), "⌃s");
    }
}
