//! App-owned modal for browsing attract-screen favorites.

use std::mem;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

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
use ratatui::widgets::Wrap;
use tui_pane::Action;
use tui_pane::BandDirection;
use tui_pane::BandFraying;
use tui_pane::Bindings;
use tui_pane::ColumnSpec;
use tui_pane::ColumnWidths;
use tui_pane::Keymap;
use tui_pane::Mode;
use tui_pane::Pane;
use tui_pane::PaneFocusState;
use tui_pane::PixelFill;
use tui_pane::PixelResolve;
use tui_pane::PopupFrame;
use tui_pane::Shortcuts;
use tui_pane::TabStop;
use tui_pane::TextDrift;
use tui_pane::TextFill;
use tui_pane::ToastStyle;
use tui_pane::Viewport;
use tui_pane::ViewportOverflow;
use tui_pane::blend_color;
use tui_pane::error_color;
use tui_pane::keep_visible_scroll_offset;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::selection_style;
use tui_pane::text_default;
use tui_pane::title_color;
use tui_pane::warning_color;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::app::AppOverlay;
use crate::app::AppPaneId;
use crate::attract;
use crate::attract::AttractMode;
use crate::attract::SettingsApplicationOutcome;
use crate::constants::COLUMN_GAP;
use crate::constants::CONTENT_MIN_HEIGHT;
use crate::constants::CURSOR_WIDTH;
use crate::constants::FAVORITE_REMOVAL_FADE;
use crate::constants::FAVORITES_SCOPE;
use crate::constants::FAVORITES_SECTION;
use crate::constants::FOOTER_HEIGHT;
use crate::constants::NOTICE_TOAST_MIN_INTERIOR_LINES;
use crate::constants::NOTICE_TOAST_VISIBLE;
use crate::constants::POPUP_CHROME_HEIGHT;
use crate::constants::POPUP_CHROME_WIDTH;
use crate::constants::POPUP_MAX_WIDTH;
use crate::constants::POPUP_SIDE_MARGIN;
use crate::favorites;
use crate::favorites::AttractSettings;
use crate::favorites::Favorite;
use crate::favorites::FavoriteId;
use crate::favorites::FavoriteRemovalTarget;
use crate::favorites::FavoriteRowRecognition;
use crate::favorites::FavoriteRows;
use crate::favorites::FavoritesFileState;
use crate::favorites::FavoritesMutation;
use crate::favorites::FavoritesMutationError;
use crate::favorites::FavoritesRetryInstruction;
use crate::favorites::ResolvedBinding;
use crate::favorites::UnrecognizedFavoriteValue;
use crate::globals::AppGlobalAction;
use crate::terminal::VisualDeadline;

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum FavoritesOverlayAction {
        SelectPrevious => ("select_previous", "Select the previous favorite");
        SelectNext => ("select_next", "Select the next favorite");
        PageColumnsLeft => ("page_columns_left", "Show the previous parameter column");
        PageColumnsRight => ("page_columns_right", "Show the next parameter column");
        Load => ("load", "Load the selected favorite");
        Delete => ("delete", "Delete the selected favorite");
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

    const SCOPE_NAME: &'static str = FAVORITES_SCOPE;
    const SECTION_NAME: &'static str = FAVORITES_SECTION;

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            [KeyCode::Up, 'k'] => FavoritesOverlayAction::SelectPrevious,
            [KeyCode::Down, 'j'] => FavoritesOverlayAction::SelectNext,
            [KeyCode::Left, 'h'] => FavoritesOverlayAction::PageColumnsLeft,
            [KeyCode::Right, 'l'] => FavoritesOverlayAction::PageColumnsRight,
            KeyCode::Enter => FavoritesOverlayAction::Load,
            'x' => FavoritesOverlayAction::Delete,
            KeyCode::Esc => FavoritesOverlayAction::Close,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch }
}

fn dispatch(action: FavoritesOverlayAction, app: &mut App) {
    let mut overlay = mem::take(&mut app.favorites_overlay);
    match overlay.handle_action(action) {
        FavoritesOverlayActionOutcome::Quiet => {},
        FavoritesOverlayActionOutcome::Load(settings) => {
            let application = app.attract.apply_settings(settings);
            close_overlay(&mut overlay, app);
            app.attract.request_show();
            report_application_outcome(&mut overlay, app, application);
        },
        FavoritesOverlayActionOutcome::Close => close_overlay(&mut overlay, app),
    }
    app.favorites_overlay = overlay;
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

impl From<FavoritesFileState> for FavoritesOverlayContent {
    fn from(state: FavoritesFileState) -> Self {
        match state {
            FavoritesFileState::LocationUnavailable => Self::LocationUnavailable,
            FavoritesFileState::Missing { .. } => Self::NoneSaved,
            FavoritesFileState::Loaded { rows, .. } => {
                let view = FavoriteRowsView::from(&rows);
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
}

impl FavoritesOverlayContent {
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

impl From<&FavoriteRows> for FavoriteRowsView {
    fn from(rows: &FavoriteRows) -> Self {
        let mut sections: Vec<FavoriteModeSection> = Vec::new();
        let mut unrecognized = Vec::new();
        for recognition in rows.iter() {
            match recognition {
                FavoriteRowRecognition::Recognized(favorite) => {
                    let mode = favorite.settings.mode();
                    if let Some(section) = sections.iter_mut().find(|section| section.mode == mode)
                    {
                        section.rows.push(FavoriteRowView::from(favorite));
                    } else {
                        sections.push(FavoriteModeSection {
                            mode,
                            rows: vec![FavoriteRowView::from(favorite)],
                        });
                    }
                },
                FavoriteRowRecognition::Unrecognized { diagnostic, .. } => {
                    unrecognized.push(UnrecognizedFavoriteView::from(diagnostic));
                },
            }
        }
        Self {
            sections,
            unrecognized,
        }
    }
}

impl FavoriteRowsView {
    fn saved_count(&self) -> usize { self.sections.iter().map(|section| section.rows.len()).sum() }

    fn row(&self, favorite_id: FavoriteId) -> FavoriteRowLookup<'_> {
        self.sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.id == favorite_id)
            .map_or(FavoriteRowLookup::Missing, FavoriteRowLookup::Found)
    }

    fn row_mut(&mut self, favorite_id: FavoriteId) -> FavoriteRowLookupMut<'_> {
        self.sections
            .iter_mut()
            .flat_map(|section| &mut section.rows)
            .find(|row| row.id == favorite_id)
            .map_or(FavoriteRowLookupMut::Missing, FavoriteRowLookupMut::Found)
    }

    fn remove(&mut self, favorite_id: FavoriteId) {
        for section in &mut self.sections {
            section.rows.retain(|row| row.id != favorite_id);
        }
        self.sections.retain(|section| !section.rows.is_empty());
    }

    fn removing_ids(&self) -> Vec<FavoriteId> {
        self.sections
            .iter()
            .flat_map(|section| &section.rows)
            .filter_map(|row| match row.lifecycle {
                FavoriteRowLifecycle::Active => None,
                FavoriteRowLifecycle::Removing { .. } => Some(row.id),
            })
            .collect()
    }
}

enum FavoriteRowLookup<'a> {
    Found(&'a FavoriteRowView),
    Missing,
}

enum FavoriteRowLookupMut<'a> {
    Found(&'a mut FavoriteRowView),
    Missing,
}

#[derive(Clone, Debug)]
struct FavoriteModeSection {
    mode: AttractMode,
    rows: Vec<FavoriteRowView>,
}

#[derive(Clone, Debug)]
struct FavoriteRowView {
    id:        FavoriteId,
    settings:  AttractSettings,
    saved:     String,
    cells:     Vec<String>,
    lifecycle: FavoriteRowLifecycle,
}

impl From<&Favorite> for FavoriteRowView {
    fn from(favorite: &Favorite) -> Self {
        Self {
            id:        favorite.id,
            settings:  favorite.settings,
            saved:     format_timestamp(favorite),
            cells:     favorite_cells(favorite.settings),
            lifecycle: FavoriteRowLifecycle::Active,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FavoriteRowLifecycle {
    Active,
    Removing { since: Instant },
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

#[derive(Clone, Debug)]
struct FavoritesSurfaceBindings {
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
            load: resolve_pane_binding(keymap, AppPaneId::Favorites, "load"),
            delete: resolve_pane_binding(keymap, AppPaneId::Favorites, "delete"),
            close: resolve_pane_binding(keymap, AppPaneId::Favorites, "close"),
            save: resolve_global_binding(keymap, "save_favorite"),
            open: resolve_global_binding(keymap, "open_favorites"),
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

    fn empty_notice(&self) -> String {
        format!(
            "No favorites saved -- press {}, then {} while the attract screen is up",
            self.close.display_short(),
            self.save.display_short(),
        )
    }

    fn delete_retry(&self) -> FavoritesRetryInstruction {
        FavoritesRetryInstruction::Press(self.delete.clone())
    }

    fn close_delete_retry(&self) -> FavoritesRetryInstruction {
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
        if let Some(last_favorite_line) = self
            .lines
            .iter()
            .rposition(|line| matches!(line, CachedOverlayLine::Favorite { .. }))
        {
            self.navigation_line_index
                .extend(last_favorite_line.saturating_add(1)..self.lines.len());
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum FavoritesOverlayNotice {
    #[default]
    NoNotice,
    DeletionRefused {
        message: String,
    },
    FavoriteAdjusted {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FavoriteRemovalCommitState {
    #[default]
    NoCommitPending,
    Pending(FavoriteId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FavoritesOverlayActionOutcome {
    Quiet,
    Load(AttractSettings),
    Close,
}

/// Time-driven work owed by the favorites overlay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FavoritesOverlayFrameOutcome {
    /// No row is fading and no frame is owed.
    Quiet,
    /// A removal fade is in progress.
    Repaint,
    /// The fade finished and this row must be removed from the file.
    CommitRemoval(FavoriteId),
}

struct FavoritesOverlayCloseCommit {
    favorite_ids: Vec<FavoriteId>,
    retry:        FavoritesRetryInstruction,
}

/// The complete app-owned favorites modal controller.
pub(crate) struct FavoritesOverlay {
    state:                  AppOverlay,
    viewport:               Viewport,
    horizontal_column_page: usize,
    surface_bindings:       FavoritesSurfaceBindings,
    line_plan:              CachedLinePlan,
    cached_surface_width:   CachedSurfaceWidth,
    notice:                 FavoritesOverlayNotice,
    removal_commit:         FavoriteRemovalCommitState,
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
            notice:                 FavoritesOverlayNotice::NoNotice,
            removal_commit:         FavoriteRemovalCommitState::NoCommitPending,
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

    /// Open the modal at the content position represented by one complete file state.
    pub(crate) fn open_file_state(&mut self, state: FavoritesFileState, keymap: &Keymap<App>) {
        self.state = AppOverlay::Favorites(FavoritesOverlayContent::from(state));
        self.surface_bindings = FavoritesSurfaceBindings::resolve(keymap);
        self.horizontal_column_page = 0;
        self.notice = FavoritesOverlayNotice::NoNotice;
        self.removal_commit = FavoriteRemovalCommitState::NoCommitPending;
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
    fn handle_action(&mut self, action: FavoritesOverlayAction) -> FavoritesOverlayActionOutcome {
        if !self.is_open() {
            return FavoritesOverlayActionOutcome::Quiet;
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
            FavoritesOverlayAction::Load => {
                if let FavoriteSelection::Row { settings, .. } = self.favorite_selection() {
                    self.notice = FavoritesOverlayNotice::NoNotice;
                    return FavoritesOverlayActionOutcome::Load(settings);
                }
            },
            FavoritesOverlayAction::Delete => {
                self.start_removal(Instant::now());
            },
            FavoritesOverlayAction::Close => return FavoritesOverlayActionOutcome::Close,
        }
        FavoritesOverlayActionOutcome::Quiet
    }

    fn start_removal(&mut self, now: Instant) {
        let FavoriteSelection::Row { id, .. } = self.favorite_selection() else {
            return;
        };
        self.notice = FavoritesOverlayNotice::NoNotice;
        if let AppOverlay::Favorites(FavoritesOverlayContent::Rows(rows)) = &mut self.state
            && let FavoriteRowLookupMut::Found(row) = rows.row_mut(id)
        {
            row.lifecycle = FavoriteRowLifecycle::Removing { since: now };
            self.rebuild_for_cached_width();
        }
    }

    fn begin_close(&mut self) -> FavoritesOverlayCloseCommit {
        let favorite_ids = match &self.state {
            AppOverlay::Favorites(FavoritesOverlayContent::Rows(rows)) => rows.removing_ids(),
            AppOverlay::Closed
            | AppOverlay::Favorites(
                FavoritesOverlayContent::NoneSaved
                | FavoritesOverlayContent::OnlyUnrecognized(_)
                | FavoritesOverlayContent::LocationUnavailable
                | FavoritesOverlayContent::Unparseable { .. }
                | FavoritesOverlayContent::Unreadable { .. },
            ) => Vec::new(),
        };
        self.state = AppOverlay::Closed;
        self.notice = FavoritesOverlayNotice::NoNotice;
        self.removal_commit = FavoriteRemovalCommitState::NoCommitPending;
        FavoritesOverlayCloseCommit {
            favorite_ids,
            retry: self.surface_bindings.close_delete_retry(),
        }
    }

    fn finish_close(&mut self) {
        self.viewport.clear_surface();
        self.line_plan = CachedLinePlan::default();
    }

    /// Advance any row-removal fade without writing the favorites file.
    pub(crate) fn advance(&mut self, now: Instant) -> FavoritesOverlayFrameOutcome {
        if !matches!(
            self.removal_commit,
            FavoriteRemovalCommitState::NoCommitPending
        ) {
            return FavoritesOverlayFrameOutcome::Quiet;
        }
        let AppOverlay::Favorites(FavoritesOverlayContent::Rows(rows)) = &self.state else {
            return FavoritesOverlayFrameOutcome::Quiet;
        };
        let mut fade_in_progress = false;
        for row in rows.sections.iter().flat_map(|section| &section.rows) {
            if let FavoriteRowLifecycle::Removing { since } = row.lifecycle {
                if now.duration_since(since) >= FAVORITE_REMOVAL_FADE {
                    self.removal_commit = FavoriteRemovalCommitState::Pending(row.id);
                    return FavoritesOverlayFrameOutcome::CommitRemoval(row.id);
                }
                fade_in_progress = true;
            }
        }
        if fade_in_progress {
            FavoritesOverlayFrameOutcome::Repaint
        } else {
            FavoritesOverlayFrameOutcome::Quiet
        }
    }

    /// Earliest wake needed to continue a row-removal fade.
    pub(crate) fn visual_deadline(&self, now: Instant, frame_period: Duration) -> VisualDeadline {
        if !matches!(
            self.removal_commit,
            FavoriteRemovalCommitState::NoCommitPending
        ) {
            return VisualDeadline::NoVisualChangeScheduled;
        }
        let AppOverlay::Favorites(FavoritesOverlayContent::Rows(rows)) = &self.state else {
            return VisualDeadline::NoVisualChangeScheduled;
        };
        rows.sections.iter().flat_map(|section| &section.rows).fold(
            VisualDeadline::NoVisualChangeScheduled,
            |deadline, row| {
                let FavoriteRowLifecycle::Removing { since } = row.lifecycle else {
                    return deadline;
                };
                let removal_done = since + FAVORITE_REMOVAL_FADE;
                let next_frame = now + frame_period;
                deadline.earlier(VisualDeadline::At(removal_done.min(next_frame)))
            },
        )
    }

    /// Reconcile one completed fade with the result of its file mutation.
    pub(crate) fn finish_removal(
        &mut self,
        favorite_id: FavoriteId,
        result: Result<(), FavoritesMutationError>,
    ) {
        if self.removal_commit != FavoriteRemovalCommitState::Pending(favorite_id) {
            return;
        }
        self.removal_commit = FavoriteRemovalCommitState::NoCommitPending;
        match result {
            Ok(()) => {
                self.drop_removed_row(favorite_id);
                self.rebuild_for_cached_width();
            },
            Err(error) => {
                if let AppOverlay::Favorites(FavoritesOverlayContent::Rows(rows)) = &mut self.state
                    && let FavoriteRowLookupMut::Found(row) = rows.row_mut(favorite_id)
                {
                    row.lifecycle = FavoriteRowLifecycle::Active;
                }
                self.notice = FavoritesOverlayNotice::DeletionRefused {
                    message: favorites::favorite_refusal_message(
                        FavoritesMutation::Delete,
                        &self.surface_bindings.delete_retry(),
                        &error,
                    ),
                };
                self.rebuild_for_cached_width();
                self.select_favorite(favorite_id);
            },
        }
    }

    fn drop_removed_row(&mut self, favorite_id: FavoriteId) {
        let AppOverlay::Favorites(content) = &mut self.state else {
            return;
        };
        if let FavoritesOverlayContent::Rows(rows) = content {
            rows.remove(favorite_id);
        }
        normalize_content_after_removal(content);
    }

    fn select_favorite(&mut self, favorite_id: FavoriteId) {
        for (position, line_index) in self.line_plan.navigation_line_index.iter().enumerate() {
            if matches!(
                self.line_plan.lines.get(*line_index),
                Some(CachedOverlayLine::Favorite { id, .. }) if *id == favorite_id
            ) {
                self.viewport.set_pos(position);
                return;
            }
        }
    }

    fn set_adjustment_notice(&mut self, message: String) {
        self.notice = FavoritesOverlayNotice::FavoriteAdjusted { message };
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
        let surface_width = width.saturating_sub(POPUP_CHROME_WIDTH);
        if self.cached_surface_width != CachedSurfaceWidth::Rendered(surface_width) {
            self.cached_surface_width = CachedSurfaceWidth::Rendered(surface_width);
            self.rebuild_line_plan(surface_width);
        }

        let notice_height = match &self.notice {
            FavoritesOverlayNotice::NoNotice => 0,
            FavoritesOverlayNotice::DeletionRefused { message }
            | FavoritesOverlayNotice::FavoriteAdjusted { message } => {
                wrapped_notice_height(message, surface_width)
            },
        };
        let desired_height = u16::try_from(self.line_plan.lines.len())
            .unwrap_or(u16::MAX)
            .saturating_add(FOOTER_HEIGHT)
            .saturating_add(notice_height)
            .saturating_add(POPUP_CHROME_HEIGHT);
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
        let footer_height = FOOTER_HEIGHT.min(popup.inner.height);
        let notice_height = notice_height.min(
            popup
                .inner
                .height
                .saturating_sub(footer_height)
                .saturating_sub(CONTENT_MIN_HEIGHT),
        );
        let content_height = popup
            .inner
            .height
            .saturating_sub(footer_height)
            .saturating_sub(notice_height);
        let content_area = Rect::new(
            popup.inner.x,
            popup.inner.y,
            popup.inner.width,
            content_height,
        );
        let notice_area = Rect::new(
            popup.inner.x,
            popup.inner.y.saturating_add(content_height),
            popup.inner.width,
            notice_height,
        );
        let footer_area = Rect::new(
            popup.inner.x,
            popup
                .inner
                .y
                .saturating_add(content_height)
                .saturating_add(notice_height),
            popup.inner.width,
            footer_height,
        );

        let active_line = self.update_vertical_viewport(content_area);
        let visible_rows = self.viewport.visible_rows();
        let line_count = self.line_plan.lines.len();
        let scroll_offset = self.viewport.scroll_offset();

        let end = scroll_offset.saturating_add(visible_rows).min(line_count);
        let selected = self.favorite_selection();
        let now = Instant::now();
        let state = &self.state;
        let visible = self.line_plan.lines[scroll_offset..end]
            .iter()
            .map(|line| rendered_line(line, selected, row_lifecycle(state, line), now))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(visible), content_area);
        render_notice(frame, &self.notice, notice_area);
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

    fn favorite_selection(&self) -> FavoriteSelection {
        let Some(&line_index) = self
            .line_plan
            .navigation_line_index
            .get(self.viewport.pos())
        else {
            return FavoriteSelection::Nothing;
        };
        match self.line_plan.lines.get(line_index) {
            Some(CachedOverlayLine::Favorite { id, .. }) => match &self.state {
                AppOverlay::Favorites(FavoritesOverlayContent::Rows(rows)) => match rows.row(*id) {
                    FavoriteRowLookup::Found(row) => FavoriteSelection::Row {
                        id:       *id,
                        settings: row.settings,
                    },
                    FavoriteRowLookup::Missing => FavoriteSelection::Nothing,
                },
                AppOverlay::Closed
                | AppOverlay::Favorites(
                    FavoritesOverlayContent::NoneSaved
                    | FavoritesOverlayContent::OnlyUnrecognized(_)
                    | FavoritesOverlayContent::LocationUnavailable
                    | FavoritesOverlayContent::Unparseable { .. }
                    | FavoritesOverlayContent::Unreadable { .. },
                ) => FavoriteSelection::Nothing,
            },
            Some(CachedOverlayLine::Static(_)) | None => FavoriteSelection::Nothing,
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

fn render_notice(frame: &mut Frame<'_>, notice: &FavoritesOverlayNotice, area: Rect) {
    let (message, color) = match notice {
        FavoritesOverlayNotice::NoNotice => return,
        FavoritesOverlayNotice::DeletionRefused { message } => (message, error_color()),
        FavoritesOverlayNotice::FavoriteAdjusted { message } => (message, warning_color()),
    };
    frame.render_widget(
        Paragraph::new(message.as_str())
            .style(Style::default().fg(color))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn close_overlay(overlay: &mut FavoritesOverlay, app: &mut App) {
    close_overlay_with(overlay, app, favorites::remove);
}

fn close_overlay_with(
    overlay: &mut FavoritesOverlay,
    app: &mut App,
    mut remove: impl FnMut(FavoriteRemovalTarget) -> Result<(), FavoritesMutationError>,
) {
    let close_commit = overlay.begin_close();
    for favorite_id in close_commit.favorite_ids {
        if let Err(error) = remove(FavoriteRemovalTarget::Recognized(favorite_id)) {
            let message = favorites::favorite_refusal_message(
                FavoritesMutation::Delete,
                &close_commit.retry,
                &error,
            );
            push_scheduled_toast(app, "Favorite not deleted", &message, ToastStyle::Error);
        }
    }
    overlay.finish_close();
}

fn report_application_outcome(
    overlay: &mut FavoritesOverlay,
    app: &mut App,
    outcome: SettingsApplicationOutcome,
) {
    if !overlay.is_open() {
        report_closed_overlay_adjustment(app, outcome);
        return;
    }
    let SettingsApplicationOutcome::AppliedWithAdjustments {
        requested,
        effective,
    } = outcome
    else {
        return;
    };
    let message = favorite_adjustment_message(requested, effective);
    overlay.set_adjustment_notice(message);
}

/// Report an adjusted favorite after its modal has closed.
pub(crate) fn report_closed_overlay_adjustment(app: &mut App, outcome: SettingsApplicationOutcome) {
    let SettingsApplicationOutcome::AppliedWithAdjustments {
        requested,
        effective,
    } = outcome
    else {
        return;
    };
    let message = favorite_adjustment_message(requested, effective);
    push_scheduled_toast(app, "Favorite adjusted", &message, ToastStyle::Warning);
}

fn push_scheduled_toast(app: &mut App, title: &str, body: &str, style: ToastStyle) {
    app.framework.toasts.push_timed_styled(
        title,
        body,
        NOTICE_TOAST_VISIBLE,
        NOTICE_TOAST_MIN_INTERIOR_LINES,
        style,
    );
}

fn favorite_adjustment_message(requested: AttractSettings, effective: AttractSettings) -> String {
    let mut fields = Vec::new();
    match (requested, effective) {
        (AttractSettings::MovingBand(requested), AttractSettings::MovingBand(effective)) => {
            record_adjustment(
                &mut fields,
                "direction",
                direction_name(requested.direction),
                direction_name(effective.direction),
            );
            record_numeric_adjustment(&mut fields, "width", &requested.width, &effective.width);
            record_numeric_adjustment(&mut fields, "speed", &requested.speed, &effective.speed);
            record_numeric_adjustment(
                &mut fields,
                "tail_speed",
                &requested.tail_speed,
                &effective.tail_speed,
            );
            record_adjustment(
                &mut fields,
                "fraying",
                fraying_name(requested.fraying),
                fraying_name(effective.fraying),
            );
        },
        (AttractSettings::MovingText(requested), AttractSettings::MovingText(effective)) => {
            record_adjustment(
                &mut fields,
                "direction",
                direction_name(requested.direction),
                direction_name(effective.direction),
            );
            record_numeric_adjustment(&mut fields, "speed", &requested.speed, &effective.speed);
            record_numeric_adjustment(&mut fields, "spread", &requested.spread, &effective.spread);
            record_adjustment(
                &mut fields,
                "drift",
                drift_name(requested.drift),
                drift_name(effective.drift),
            );
            record_adjustment(
                &mut fields,
                "fill",
                text_fill_name(requested.fill),
                text_fill_name(effective.fill),
            );
        },
        (AttractSettings::Pixelate(requested), AttractSettings::Pixelate(effective)) => {
            record_adjustment(
                &mut fields,
                "direction",
                direction_name(requested.direction),
                direction_name(effective.direction),
            );
            record_numeric_adjustment(&mut fields, "speed", &requested.speed, &effective.speed);
            record_numeric_adjustment(
                &mut fields,
                "wave_percent",
                &requested.wave_percent,
                &effective.wave_percent,
            );
            record_numeric_adjustment(
                &mut fields,
                "block_columns",
                &requested.block_columns,
                &effective.block_columns,
            );
            record_adjustment(
                &mut fields,
                "resolve",
                pixel_resolve_name(requested.resolve),
                pixel_resolve_name(effective.resolve),
            );
            record_adjustment(
                &mut fields,
                "fill",
                pixel_fill_name(requested.fill),
                pixel_fill_name(effective.fill),
            );
        },
        (
            AttractSettings::MovingBand(_)
            | AttractSettings::MovingText(_)
            | AttractSettings::Pixelate(_),
            AttractSettings::MovingBand(_)
            | AttractSettings::MovingText(_)
            | AttractSettings::Pixelate(_),
        ) => fields.push("mode changed unexpectedly".to_string()),
    }
    format!("Adjusted favorite for this terminal: {}", fields.join(", "))
}

fn record_adjustment(fields: &mut Vec<String>, name: &str, requested: &str, effective: &str) {
    if requested != effective {
        fields.push(format!("{name} {requested} -> {effective}"));
    }
}

fn record_numeric_adjustment<T: std::fmt::Display + Eq>(
    fields: &mut Vec<String>,
    name: &str,
    requested: &T,
    effective: &T,
) {
    if requested != effective {
        fields.push(format!("{name} {requested} -> {effective}"));
    }
}

fn normalize_content_after_removal(content: &mut FavoritesOverlayContent) {
    let current = mem::replace(content, FavoritesOverlayContent::NoneSaved);
    *content = match current {
        FavoritesOverlayContent::Rows(rows) if rows.saved_count() == 0 => {
            if rows.unrecognized.is_empty() {
                FavoritesOverlayContent::NoneSaved
            } else {
                FavoritesOverlayContent::OnlyUnrecognized(UnrecognizedFavoritesView {
                    rows: rows.unrecognized,
                })
            }
        },
        other => other,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FavoriteSelection {
    Nothing,
    Row {
        id:       FavoriteId,
        settings: AttractSettings,
    },
}

fn rendered_line(
    line: &CachedOverlayLine,
    selected: FavoriteSelection,
    lifecycle: FavoriteRowLifecycle,
    now: Instant,
) -> Line<'static> {
    match line {
        CachedOverlayLine::Static(line) => line.clone(),
        CachedOverlayLine::Favorite { id, tail } => {
            let is_selected =
                matches!(selected, FavoriteSelection::Row { id: selected, .. } if selected == *id);
            let marker = if is_selected { "▸ " } else { "  " };
            let line = Line::from(vec![Span::raw(marker), Span::raw(tail.clone())]);
            if is_selected {
                line.style(selection_style(PaneFocusState::Active))
            } else {
                line.style(Style::default().fg(blend_color(
                    text_default(),
                    attract::ground(),
                    removal_alpha(lifecycle, now),
                )))
            }
        },
    }
}

fn row_lifecycle(state: &AppOverlay, line: &CachedOverlayLine) -> FavoriteRowLifecycle {
    let CachedOverlayLine::Favorite { id, .. } = line else {
        return FavoriteRowLifecycle::Active;
    };
    let AppOverlay::Favorites(FavoritesOverlayContent::Rows(rows)) = state else {
        return FavoriteRowLifecycle::Active;
    };
    match rows.row(*id) {
        FavoriteRowLookup::Found(row) => row.lifecycle,
        FavoriteRowLookup::Missing => FavoriteRowLifecycle::Active,
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
        if row.lifecycle == FavoriteRowLifecycle::Active {
            plan.selectable_line_index.push(line_index);
        }
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
    let pinned = CURSOR_WIDTH.saturating_add(usize::from(saved_width));
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
        .max(area.width.min(POPUP_CHROME_WIDTH))
}

fn popup_height_cap(area: Rect) -> u16 {
    let eighty_percent = u32::from(area.height).saturating_mul(80) / 100;
    u16::try_from(eighty_percent)
        .unwrap_or(u16::MAX)
        .max((POPUP_CHROME_HEIGHT + FOOTER_HEIGHT).min(area.height))
}

fn wrapped_notice_height(message: &str, surface_width: u16) -> u16 {
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

fn format_timestamp(favorite: &Favorite) -> String {
    if favorite.saved.year() == Local::now().year() {
        favorite.saved.format("%d %b %H:%M:%S").to_string()
    } else {
        favorite.saved.format("%d %b %Y %H:%M:%S").to_string()
    }
}

fn favorite_cells(settings: AttractSettings) -> Vec<String> {
    match settings {
        AttractSettings::MovingBand(settings) => vec![
            direction_name(settings.direction).to_string(),
            settings.width.to_string(),
            settings.speed.to_string(),
            settings.tail_speed.to_string(),
            fraying_name(settings.fraying).to_string(),
        ],
        AttractSettings::MovingText(settings) => vec![
            direction_name(settings.direction).to_string(),
            settings.speed.to_string(),
            settings.spread.to_string(),
            drift_name(settings.drift).to_string(),
            text_fill_name(settings.fill).to_string(),
        ],
        AttractSettings::Pixelate(settings) => vec![
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

const fn pixel_resolve_name(resolve: PixelResolve) -> &'static str {
    match resolve {
        PixelResolve::Blend => "blend",
        PixelResolve::Step => "step",
        PixelResolve::Scatter => "scatter",
    }
}

const fn pixel_fill_name(fill: PixelFill) -> &'static str {
    match fill {
        PixelFill::Solid => "solid",
        PixelFill::Shades => "shades",
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    clippy::panic,
    clippy::unchecked_time_subtraction,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use tempfile::TempDir;
    use tui_pane::FocusedPane;
    use tui_pane::Framework;
    use tui_pane::KeyBind;
    use tui_pane::KeySequence;
    use tui_pane::PixelFill;
    use tui_pane::PixelResolve;
    use tui_pane::ToastVisualDeadline;

    use super::*;
    use crate::app::Updates;
    use crate::attract::AttractGridPresentation;
    use crate::attract::AttractVisibilityInstruction;
    use crate::attract::Work;
    use crate::favorites;
    use crate::keymap;

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
        keymap::build_keymap(&mut framework, (!toml.is_empty()).then_some(path))
            .expect("test keymap should resolve")
    }

    fn loaded_state(text: &str) -> FavoritesFileState {
        loaded_state_at("/tmp/favorites.toml", text)
    }

    fn loaded_state_at(path: impl Into<PathBuf>, text: &str) -> FavoritesFileState {
        FavoritesFileState::Loaded {
            path: path.into(),
            rows: favorites::parse_rows_for_overlay_test(text)
                .expect("favorites fixture should parse"),
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
        let rows = favorites::parse_rows_for_overlay_test(MOVING_BAND_ROW)
            .expect("moving-band fixture should parse");
        let view = FavoriteRowsView::from(&rows);
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

    fn selected(overlay: &FavoritesOverlay) -> (FavoriteId, AttractSettings) {
        let FavoriteSelection::Row { id, settings } = overlay.favorite_selection() else {
            panic!("fixture should select a recognized favorite");
        };
        (id, settings)
    }

    fn lifecycle(overlay: &FavoritesOverlay, favorite_id: FavoriteId) -> FavoriteRowLifecycle {
        let AppOverlay::Favorites(FavoritesOverlayContent::Rows(rows)) = &overlay.state else {
            panic!("fixture should contain recognized rows");
        };
        let FavoriteRowLookup::Found(row) = rows.row(favorite_id) else {
            panic!("favorite should remain in the overlay");
        };
        row.lifecycle
    }

    fn rendered_buffer_lines(buffer: &Buffer) -> Vec<String> {
        (buffer.area.y..buffer.area.bottom())
            .map(|y| {
                (buffer.area.x..buffer.area.right()).fold(String::new(), |mut line, x| {
                    line.push_str(buffer[(x, y)].symbol());
                    line
                })
            })
            .collect()
    }

    #[test]
    fn modal_scope_includes_load_and_delete() {
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
            (KeyBind::from(KeyCode::Enter), FavoritesOverlayAction::Load),
            (KeyBind::from('x'), FavoritesOverlayAction::Delete),
            (KeyBind::from(KeyCode::Esc), FavoritesOverlayAction::Close),
        ];
        for (binding, action) in cases {
            assert_eq!(scope.action_for(&binding), Some(action));
        }
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
        assert_eq!(overlay.favorite_selection(), FavoriteSelection::Nothing);
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
        assert_eq!(overlay.favorite_selection(), FavoriteSelection::Nothing);
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
        let rows = favorites::parse_rows_for_overlay_test(&format!(
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
        let view = FavoriteRowsView::from(&rows);
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
            CURSOR_WIDTH
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
            CURSOR_WIDTH
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
            + CURSOR_WIDTH
            + COLUMN_GAP
            + usize::from(table_layout.parameter_widths.get(0));
        let width = u16::try_from(exact_width - 1).expect("narrow table width should fit u16");
        let visible = table_layout.visible_parameter_columns(0, width);

        assert_eq!(visible, 0..1);
        let headings = BAND_COLUMNS.map(|descriptor| descriptor.heading);
        let labels = FavoritesSurfaceBindings::resolve(&keymap)
            .column_labels(AttractMode::MovingBand)
            .to_vec();
        let rows = favorites::parse_rows_for_overlay_test(MOVING_BAND_ROW)
            .expect("moving-band fixture should parse");
        let view = FavoriteRowsView::from(&rows);
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
    fn load_uses_retained_settings_and_reverses_fade_out() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let (_, settings) = selected(&overlay);
        let AppOverlay::Favorites(FavoritesOverlayContent::Rows(rows)) = &mut overlay.state else {
            panic!("fixture should contain recognized rows");
        };
        rows.sections[0].rows[0].cells[0] = "lossy-display-value".to_string();

        let mut app = App::new_for_test().expect("test app should build");
        let now = Instant::now();
        app.attract.request_show();
        app.attract
            .advance(Rect::new(0, 0, 80, 24), Work::Idle, Updates::Live, now);
        app.attract.advance(
            Rect::new(0, 0, 80, 24),
            Work::Idle,
            Updates::Live,
            now + Duration::from_millis(8),
        );
        app.attract.toggle();
        app.attract.advance(
            Rect::new(0, 0, 80, 24),
            Work::Idle,
            Updates::Live,
            now + Duration::from_millis(16),
        );
        assert!(app.attract.showing());
        assert!(!app.attract.asked_for());

        app.favorites_overlay = overlay;
        dispatch(FavoritesOverlayAction::Load, &mut app);

        assert!(!app.favorites_overlay.is_open());
        assert!(app.attract.asked_for());
        assert_eq!(app.attract.current_settings(), settings);
    }

    #[test]
    fn mutation_keys_do_nothing_on_diagnostic_lines() {
        let keymap = keymap_from("");
        let mixed = format!("{MOVING_BAND_ROW}\n{UNRECOGNIZED_ROWS}");
        let mut overlay = open_at_width(loaded_state(&mixed), &keymap, 100);
        while overlay.viewport.pos() + 1 < overlay.viewport.len() {
            overlay.handle_action(FavoritesOverlayAction::SelectNext);
        }
        assert_eq!(overlay.favorite_selection(), FavoriteSelection::Nothing);
        let refusal = FavoritesOverlayNotice::DeletionRefused {
            message: "keep this refusal visible".to_string(),
        };
        overlay.notice = refusal.clone();

        assert_eq!(
            overlay.handle_action(FavoritesOverlayAction::Load),
            FavoritesOverlayActionOutcome::Quiet
        );
        assert_eq!(overlay.notice, refusal);
        assert_eq!(
            overlay.handle_action(FavoritesOverlayAction::Delete),
            FavoritesOverlayActionOutcome::Quiet
        );
        assert_eq!(overlay.notice, refusal);
        assert!(matches!(
            overlay.advance(Instant::now() + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::Quiet
        ));
    }

    #[test]
    fn load_without_a_selected_recognized_row_preserves_the_existing_undo_point() {
        let keymap = keymap_from("");
        let mixed = format!("{MOVING_BAND_ROW}\n{UNRECOGNIZED_ROWS}");
        let mut overlay = open_at_width(loaded_state(&mixed), &keymap, 100);
        while overlay.viewport.pos() + 1 < overlay.viewport.len() {
            overlay.handle_action(FavoritesOverlayAction::SelectNext);
        }
        assert_eq!(overlay.favorite_selection(), FavoriteSelection::Nothing);
        let (_, replacement) =
            selected(&open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100));
        let mut app = App::new_for_test().expect("test app should build");
        app.attract.record_terminal_resize(Rect::new(0, 0, 80, 24));
        let before = app.attract.current_settings();
        app.attract.apply_settings(replacement);
        app.favorites_overlay = overlay;

        dispatch(FavoritesOverlayAction::Load, &mut app);
        app.attract.restore_configuration_before_last_replacement();

        assert_eq!(app.attract.current_settings(), before);
    }

    #[test]
    fn deletion_fade_uses_elapsed_time_and_commits_once() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let (favorite_id, _) = selected(&overlay);
        let started = Instant::now();

        overlay.start_removal(started);
        assert_eq!(
            lifecycle(&overlay, favorite_id),
            FavoriteRowLifecycle::Removing { since: started }
        );
        let halfway = started + FAVORITE_REMOVAL_FADE / 2;
        let alpha = removal_alpha(lifecycle(&overlay, favorite_id), halfway);
        for _ in 0..20 {
            assert_eq!(
                removal_alpha(lifecycle(&overlay, favorite_id), halfway),
                alpha
            );
        }
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE - Duration::from_nanos(1)),
            FavoritesOverlayFrameOutcome::Repaint
        );
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(favorite_id)
        );
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::Quiet
        );

        overlay.finish_removal(favorite_id, Ok(()));
        assert!(matches!(
            overlay.state,
            AppOverlay::Favorites(FavoritesOverlayContent::NoneSaved)
        ));
    }

    #[test]
    fn two_removals_can_fade_together_and_each_commit_once() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(RECOGNIZED_ROWS), &keymap, 100);
        let started = Instant::now();
        let (first, _) = selected(&overlay);
        overlay.start_removal(started);
        let (second, _) = selected(&overlay);
        assert_ne!(first, second);
        overlay.start_removal(started + Duration::from_millis(1));

        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(first)
        );
        overlay.finish_removal(first, Ok(()));
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE + Duration::from_millis(1)),
            FavoritesOverlayFrameOutcome::CommitRemoval(second)
        );
        overlay.finish_removal(second, Ok(()));
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE + Duration::from_millis(1)),
            FavoritesOverlayFrameOutcome::Quiet
        );
        assert_eq!(
            match &overlay.state {
                AppOverlay::Favorites(content) => content.saved_count(),
                AppOverlay::Closed => 0,
            },
            1
        );
    }

    #[test]
    fn successful_removal_preserves_another_rows_refusal() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(RECOGNIZED_ROWS), &keymap, 100);
        let started = Instant::now();
        let (first, _) = selected(&overlay);
        overlay.start_removal(started);
        let (second, _) = selected(&overlay);
        overlay.start_removal(started + Duration::from_millis(1));

        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(first)
        );
        overlay.finish_removal(
            first,
            Err(FavoritesMutationError::LockUnavailable {
                path:  PathBuf::from("/tmp/favorites.lock"),
                error: "held".to_string(),
            }),
        );
        let refusal = overlay.notice.clone();

        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE + Duration::from_millis(1)),
            FavoritesOverlayFrameOutcome::CommitRemoval(second)
        );
        overlay.finish_removal(second, Ok(()));

        assert_eq!(overlay.notice, refusal);
        assert!(matches!(
            overlay.notice,
            FavoritesOverlayNotice::DeletionRefused { .. }
        ));
    }

    #[test]
    fn last_removal_normalizes_to_unrecognized_content() {
        let keymap = keymap_from("");
        let mixed = format!("{MOVING_BAND_ROW}\n{UNRECOGNIZED_ROWS}");
        let mut overlay = open_at_width(loaded_state(&mixed), &keymap, 100);
        let (favorite_id, _) = selected(&overlay);
        let started = Instant::now();

        overlay.start_removal(started);
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(favorite_id)
        );
        overlay.finish_removal(favorite_id, Ok(()));

        assert!(matches!(
            overlay.state,
            AppOverlay::Favorites(FavoritesOverlayContent::OnlyUnrecognized(_))
        ));
    }

    #[test]
    fn refused_deletion_restores_the_row_and_notice_until_retry() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let (favorite_id, _) = selected(&overlay);
        let started = Instant::now();
        overlay.start_removal(started);
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(favorite_id)
        );
        overlay.finish_removal(
            favorite_id,
            Err(FavoritesMutationError::LockUnavailable {
                path:  PathBuf::from("/tmp/favorites.lock"),
                error: "held".to_string(),
            }),
        );

        assert_eq!(
            lifecycle(&overlay, favorite_id),
            FavoriteRowLifecycle::Active
        );
        assert_eq!(selected(&overlay).0, favorite_id);
        let FavoritesOverlayNotice::DeletionRefused { message } = &overlay.notice else {
            panic!("refusal should be rendered inside the open overlay");
        };
        assert!(message.contains("deletion"));
        assert!(message.contains("press x to try again"));

        overlay.handle_action(FavoritesOverlayAction::SelectPrevious);
        assert!(matches!(
            overlay.notice,
            FavoritesOverlayNotice::DeletionRefused { .. }
        ));
        let retry_started = started + FAVORITE_REMOVAL_FADE + Duration::from_millis(1);
        overlay.start_removal(retry_started);
        assert_eq!(overlay.notice, FavoritesOverlayNotice::NoNotice);
        assert_eq!(
            overlay.advance(retry_started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(favorite_id)
        );
        overlay.finish_removal(favorite_id, Ok(()));
        assert_eq!(overlay.notice, FavoritesOverlayNotice::NoNotice);
    }

    #[test]
    fn wrapped_refusal_renders_trailing_lock_error_with_realistic_path() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 74);
        let (favorite_id, _) = selected(&overlay);
        let started = Instant::now();
        overlay.start_removal(started);
        let _ = overlay.advance(started + FAVORITE_REMOVAL_FADE);
        overlay.finish_removal(
            favorite_id,
            Err(FavoritesMutationError::LockUnavailable {
                path:  PathBuf::from(
                    "/Users/testuser/Library/Application Support/cargo-tile/favorites.toml",
                ),
                error: "Resource temporarily unavailable (os error 35)".to_string(),
            }),
        );
        let mut terminal =
            Terminal::new(TestBackend::new(80, 24)).expect("test terminal should build");

        terminal
            .draw(|frame| overlay.render(frame))
            .expect("favorites overlay should render");

        let rendered = rendered_buffer_lines(terminal.backend().buffer());
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("temporarily unavailable (os error 35)"))
        );
    }

    #[test]
    fn oversized_refusal_keeps_one_favorite_row_visible() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 74);
        let (favorite_id, _) = selected(&overlay);
        let started = Instant::now();
        overlay.start_removal(started);
        let _ = overlay.advance(started + FAVORITE_REMOVAL_FADE);
        overlay.finish_removal(
            favorite_id,
            Err(FavoritesMutationError::LockUnavailable {
                path:  PathBuf::from(
                    "/Users/testuser/Library/Application Support/cargo-tile/favorites.toml",
                ),
                error: "Resource temporarily unavailable (os error 35); ".repeat(12),
            }),
        );
        let mut terminal =
            Terminal::new(TestBackend::new(80, 8)).expect("test terminal should build");

        terminal
            .draw(|frame| overlay.render(frame))
            .expect("favorites overlay should render");

        let rendered = rendered_buffer_lines(terminal.backend().buffer());
        assert!(rendered.iter().any(|line| line.contains("▸ ")));
    }

    #[test]
    fn write_refusal_also_restores_the_row() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let (favorite_id, _) = selected(&overlay);
        let started = Instant::now();
        overlay.start_removal(started);
        let _ = overlay.advance(started + FAVORITE_REMOVAL_FADE);
        overlay.finish_removal(
            favorite_id,
            Err(FavoritesMutationError::WriteFailed {
                path:  PathBuf::from("/tmp/favorites.toml"),
                error: "read-only disk".to_string(),
            }),
        );

        assert_eq!(
            lifecycle(&overlay, favorite_id),
            FavoriteRowLifecycle::Active
        );
        let FavoritesOverlayNotice::DeletionRefused { message } = &overlay.notice else {
            panic!("write refusal should remain visible");
        };
        assert!(message.contains("cannot write favorites"));
    }

    #[test]
    fn close_mid_fade_commits_before_resetting_the_controller() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let (favorite_id, _) = selected(&overlay);
        overlay.start_removal(Instant::now());
        let mut removed = Vec::new();
        let mut app = App::new_for_test().expect("test app should build");

        close_overlay_with(&mut overlay, &mut app, |id| {
            removed.push(id);
            Ok(())
        });

        assert_eq!(removed, [FavoriteRemovalTarget::Recognized(favorite_id)]);
        assert!(!overlay.is_open());
        assert!(overlay.line_plan.lines.is_empty());
    }

    #[test]
    fn close_mid_fade_refusal_uses_reopen_retry_and_a_scheduled_toast() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        overlay.start_removal(Instant::now());
        let mut app = App::new_for_test().expect("test app should build");
        let now = Instant::now();

        close_overlay_with(&mut overlay, &mut app, |_| {
            Err(FavoritesMutationError::LockUnavailable {
                path:  PathBuf::from("/tmp/favorites.lock"),
                error: "held".to_string(),
            })
        });

        let toasts = app.framework.toasts.active_views(Instant::now());
        assert_eq!(toasts.len(), 1);
        assert!(toasts[0].body().contains("press ⌃o to reopen favorites"));
        assert!(toasts[0].body().contains("press x to retry the deletion"));
        assert_eq!(toasts[0].style(), ToastStyle::Error);
        assert!(matches!(
            app.framework.toasts.next_visual_change_deadline(now),
            ToastVisualDeadline::At(_)
        ));
    }

    #[test]
    fn adjustment_warning_is_scheduled_after_load_and_exact_load_is_quiet() {
        let keymap = keymap_from("");
        let oversized = MOVING_BAND_ROW.replace("width = 10", "width = 10000");
        let directory = TempDir::new().expect("temporary directory should be created");
        let adjusted_path = directory.path().join("adjusted-favorites.toml");
        fs::write(&adjusted_path, &oversized).expect("favorites fixture should be written");
        let adjusted_file_before =
            fs::read(&adjusted_path).expect("favorites fixture should be readable");
        let mut app = App::new_for_test().expect("test app should build");
        app.attract.record_terminal_resize(Rect::new(0, 0, 10, 5));
        let initial_settings = app.attract.current_settings();
        app.attract.apply_settings(initial_settings);
        app.attract.request_show();
        app.attract.toggle();
        let before_adjusted_load = app.attract.configuration();
        assert_eq!(
            before_adjusted_load.presentation.visibility_instruction,
            AttractVisibilityInstruction::Hide
        );
        assert_eq!(
            before_adjusted_load.presentation.grid_presentation,
            AttractGridPresentation::ReplacesGrid
        );
        app.favorites_overlay =
            open_at_width(loaded_state_at(&adjusted_path, &oversized), &keymap, 100);
        let now = Instant::now();

        dispatch(FavoritesOverlayAction::Load, &mut app);

        let toasts = app.framework.toasts.active_views(Instant::now());
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].title(), "Favorite adjusted");
        assert!(toasts[0].body().contains("width"));
        assert!(toasts[0].body().contains("10000"));
        assert_eq!(toasts[0].style(), ToastStyle::Warning);
        assert!(matches!(
            app.framework.toasts.next_visual_change_deadline(now),
            ToastVisualDeadline::At(_)
        ));
        assert_eq!(
            fs::read(&adjusted_path).expect("favorites fixture should remain readable"),
            adjusted_file_before
        );
        app.attract.restore_configuration_before_last_replacement();
        assert_eq!(app.attract.configuration(), before_adjusted_load);
        assert_eq!(
            app.framework.toasts.active_views(Instant::now()).len(),
            1,
            "undo leaves the favorite adjustment warning in place"
        );

        let exact_path = directory.path().join("exact-favorites.toml");
        fs::write(&exact_path, MOVING_BAND_ROW).expect("favorites fixture should be written");
        let exact_file_before =
            fs::read(&exact_path).expect("favorites fixture should be readable");
        let mut exact_app = App::new_for_test().expect("test app should build");
        exact_app
            .attract
            .record_terminal_resize(Rect::new(0, 0, 80, 24));
        let initial_settings = exact_app.attract.current_settings();
        exact_app.attract.apply_settings(initial_settings);
        exact_app.attract.request_show();
        exact_app.attract.toggle();
        let before_exact_load = exact_app.attract.configuration();
        assert_eq!(
            before_exact_load.presentation.visibility_instruction,
            AttractVisibilityInstruction::Hide
        );
        assert_eq!(
            before_exact_load.presentation.grid_presentation,
            AttractGridPresentation::ReplacesGrid
        );
        exact_app.favorites_overlay =
            open_at_width(loaded_state_at(&exact_path, MOVING_BAND_ROW), &keymap, 100);
        dispatch(FavoritesOverlayAction::Load, &mut exact_app);
        assert!(exact_app.framework.toasts.active_now().is_empty());
        assert_eq!(
            exact_app
                .framework
                .toasts
                .next_visual_change_deadline(Instant::now()),
            ToastVisualDeadline::NoVisualChangeScheduled
        );
        assert_eq!(
            fs::read(&exact_path).expect("favorites fixture should remain readable"),
            exact_file_before
        );
        exact_app
            .attract
            .restore_configuration_before_last_replacement();
        assert_eq!(exact_app.attract.configuration(), before_exact_load);
    }

    #[test]
    fn adjustment_warning_uses_lowercase_file_spellings() {
        let keymap = keymap_from("");
        let overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let (_, requested) = selected(&overlay);
        let AttractSettings::MovingBand(mut effective) = requested else {
            panic!("fixture should contain moving-band settings");
        };
        effective.direction = BandDirection::Right;
        effective.fraying = BandFraying::Both;
        let mut app = App::new_for_test().expect("test app should build");

        report_closed_overlay_adjustment(
            &mut app,
            SettingsApplicationOutcome::AppliedWithAdjustments {
                requested,
                effective: AttractSettings::MovingBand(effective),
            },
        );

        let toasts = app.framework.toasts.active_now();
        assert_eq!(toasts.len(), 1);
        let body = toasts[0].body();
        assert!(body.contains("direction left -> right"));
        assert!(body.contains("fraying leading -> both"));
        assert!(!body.contains("Left"));
        assert!(!body.contains("Leading"));
    }

    #[test]
    fn pixel_adjustment_toast_uses_lowercase_resolve_and_fill_spellings() {
        let rows = favorites::parse_rows_for_overlay_test(RECOGNIZED_ROWS)
            .expect("recognized favorites fixture should parse");
        let mut requested = rows
            .recognized()
            .find_map(|favorite| match favorite.settings {
                AttractSettings::Pixelate(settings) => Some(settings),
                AttractSettings::MovingBand(_) | AttractSettings::MovingText(_) => None,
            })
            .expect("fixture should contain pixel settings");
        requested.resolve = PixelResolve::Blend;
        requested.fill = PixelFill::Solid;
        let mut effective = requested;
        effective.resolve = PixelResolve::Step;
        effective.fill = PixelFill::Shades;
        let mut app = App::new_for_test().expect("test app should build");

        report_closed_overlay_adjustment(
            &mut app,
            SettingsApplicationOutcome::AppliedWithAdjustments {
                requested: AttractSettings::Pixelate(requested),
                effective: AttractSettings::Pixelate(effective),
            },
        );

        let toasts = app.framework.toasts.active_now();
        assert_eq!(toasts.len(), 1);
        let body = toasts[0].body();
        assert!(body.contains("resolve blend -> step"));
        assert!(body.contains("fill solid -> shades"));
        assert!(!body.contains("Blend"));
        assert!(!body.contains("Solid"));
    }

    #[test]
    fn removal_deadline_requests_frames_without_attract_or_input() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let now = Instant::now();
        let frame = Duration::from_millis(8);
        overlay.start_removal(now);

        assert_eq!(
            overlay.visual_deadline(now, frame),
            VisualDeadline::At(now + frame)
        );
        assert_eq!(
            overlay.advance(now + frame),
            FavoritesOverlayFrameOutcome::Repaint
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
        let _ = overlay.begin_close();
        overlay.finish_close();

        overlay.open_with_loader(&keymap, || {
            loads += 1;
            loaded_state(RECOGNIZED_ROWS)
        });
        assert_eq!(loads, 2);
        assert_eq!(overlay.viewport.len(), 3);
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
