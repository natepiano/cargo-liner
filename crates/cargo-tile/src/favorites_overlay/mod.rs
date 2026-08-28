//! App-owned modal for browsing attract-screen favorites.

mod bindings;
mod content;
mod line_plan;

use std::mem;
use std::time::Duration;
use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use tui_pane::Bindings;
use tui_pane::Keymap;
use tui_pane::Mode;
use tui_pane::Pane;
use tui_pane::PopupFrame;
use tui_pane::Shortcuts;
use tui_pane::TabStop;
use tui_pane::ToastStyle;
use tui_pane::Viewport;
use tui_pane::ViewportOverflow;
use tui_pane::error_color;
use tui_pane::keep_visible_scroll_offset;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::title_color;
use tui_pane::warning_color;

use self::bindings::FavoritesSurfaceBindings;
use self::bindings::SelectedFavoriteActions;
use self::content::FavoriteRowLifecycle;
use self::content::FavoriteRowLookup;
use self::content::FavoriteRowLookupMut;
pub(crate) use self::content::FavoritesOverlayContent;
pub(crate) use self::content::UnrecognizedFavoritesView;
use self::content::direction_name;
use self::content::drift_name;
use self::content::fraying_name;
use self::content::pixel_fill_name;
use self::content::pixel_resolve_name;
use self::content::text_fill_name;
use self::line_plan::CachedLinePlan;
use self::line_plan::CachedOverlayLine;
use self::line_plan::CachedSurfaceWidth;
use self::line_plan::FavoriteRowIdentity;
use self::line_plan::FavoriteSelection;
use self::line_plan::build_line_plan;
use self::line_plan::popup_height_cap;
use self::line_plan::popup_width;
use self::line_plan::rendered_line;
use self::line_plan::row_lifecycle;
use self::line_plan::wrapped_notice_height;
use crate::app::App;
use crate::app::AppOverlay;
use crate::app::AppPaneId;
use crate::app::OpenFavoritesCurrentParameters;
use crate::app::OpenFavoritesOverlayState;
use crate::attract::SettingsApplicationOutcome;
use crate::constants::CONTENT_MIN_HEIGHT;
use crate::constants::FAVORITE_REMOVAL_FADE;
use crate::constants::FAVORITES_SCOPE;
use crate::constants::FAVORITES_SECTION;
use crate::constants::FOOTER_HEIGHT;
use crate::constants::NOTICE_TOAST_MIN_INTERIOR_LINES;
use crate::constants::NOTICE_TOAST_VISIBLE;
use crate::constants::POPUP_CHROME_HEIGHT;
use crate::constants::POPUP_CHROME_WIDTH;
use crate::favorites;
use crate::favorites::AttractSettings;
use crate::favorites::FavoriteRemovalTarget;
use crate::favorites::FavoritesFileState;
use crate::favorites::FavoritesMutation;
use crate::favorites::FavoritesMutationError;
use crate::favorites::FavoritesRetryInstruction;
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum FavoritesOverlayNotice {
    #[default]
    NoNotice,
    DeletionRefused {
        message: String,
    },
    DeletionConfirmation {
        message: String,
    },
    FavoriteAdjusted {
        message: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
enum FavoriteRemovalCommitState {
    #[default]
    NoCommitPending,
    Pending(FavoriteRowIdentity),
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

#[derive(Clone, Debug, Default, PartialEq)]
enum FavoriteDeletionConfirmationState {
    #[default]
    NoConfirmationArmed,
    AwaitingSecondPress(FavoriteRowIdentity),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FavoritesOverlayActionOutcome {
    Quiet,
    Load(AttractSettings),
    Close,
}

/// Time-driven work owed by the favorites overlay.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FavoritesOverlayFrameOutcome {
    /// No row is fading and no frame is owed.
    Quiet,
    /// A removal fade is in progress.
    Repaint,
    /// The fade finished and this row must be removed from the file.
    CommitRemoval(FavoriteRemovalTarget),
}

struct FavoritesOverlayCloseCommit {
    removal_targets: Vec<FavoriteRemovalTarget>,
    retry:           FavoritesRetryInstruction,
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
    deletion_confirmation:  FavoriteDeletionConfirmationState,
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
            cached_surface_width:   CachedSurfaceWidth::NeedsRebuild,
            notice:                 FavoritesOverlayNotice::NoNotice,
            deletion_confirmation:  FavoriteDeletionConfirmationState::NoConfirmationArmed,
            removal_commit:         FavoriteRemovalCommitState::NoCommitPending,
        }
    }
}

impl FavoritesOverlay {
    /// Whether the app modal is currently consuming input.
    pub(crate) const fn is_open(&self) -> bool { matches!(self.state, AppOverlay::Favorites(_)) }

    #[cfg(test)]
    pub(crate) const fn deletion_confirmation_is_armed_for_test(&self) -> bool {
        matches!(
            self.deletion_confirmation,
            FavoriteDeletionConfirmationState::AwaitingSecondPress(_)
        )
    }

    #[cfg(test)]
    pub(crate) const fn deletion_confirmation_notice_is_visible_for_test(&self) -> bool {
        matches!(
            self.notice,
            FavoritesOverlayNotice::DeletionConfirmation { .. }
        )
    }

    /// Reload favorites and open the matching content state.
    pub(crate) fn open(
        &mut self,
        keymap: &Keymap<App>,
        current_parameters: OpenFavoritesCurrentParameters,
    ) {
        self.open_with_loader(keymap, current_parameters, favorites::load);
    }

    fn open_with_loader(
        &mut self,
        keymap: &Keymap<App>,
        current_parameters: OpenFavoritesCurrentParameters,
        loader: impl FnOnce() -> FavoritesFileState,
    ) {
        self.open_file_state(loader(), current_parameters, keymap);
    }

    /// Open the modal at the content position represented by one complete file state.
    pub(crate) fn open_file_state(
        &mut self,
        state: FavoritesFileState,
        current_parameters: OpenFavoritesCurrentParameters,
        keymap: &Keymap<App>,
    ) {
        self.state = AppOverlay::Favorites(OpenFavoritesOverlayState {
            content: FavoritesOverlayContent::from(state),
            current_parameters,
        });
        self.surface_bindings = FavoritesSurfaceBindings::resolve(keymap);
        self.horizontal_column_page = 0;
        self.notice = FavoritesOverlayNotice::NoNotice;
        self.deletion_confirmation = FavoriteDeletionConfirmationState::NoConfirmationArmed;
        self.removal_commit = FavoriteRemovalCommitState::NoCommitPending;
        let selected_rows = match &self.state {
            AppOverlay::Closed => 0,
            AppOverlay::Favorites(open_state) => open_state.content.navigable_row_count(),
        };
        self.viewport.set_len(selected_rows);
        if let CachedSurfaceWidth::Rendered(width) = self.cached_surface_width {
            self.rebuild_line_plan(width);
        }
    }

    /// Replace the open modal's parameter snapshot after a coalesced terminal resize.
    pub(crate) const fn refresh_current_parameters(
        &mut self,
        current_parameters: OpenFavoritesCurrentParameters,
    ) {
        let AppOverlay::Favorites(open_state) = &mut self.state else {
            return;
        };
        open_state.current_parameters = current_parameters;
        self.cached_surface_width = CachedSurfaceWidth::NeedsRebuild;
    }

    /// Apply one resolved modal action.
    fn handle_action(&mut self, action: FavoritesOverlayAction) -> FavoritesOverlayActionOutcome {
        self.handle_action_at(action, Instant::now())
    }

    /// Cancel deletion confirmation when the modal consumes a key with no bound action.
    pub(crate) fn handle_unmapped_key(&mut self) {
        if self.is_open() {
            self.cancel_deletion_confirmation();
        }
    }

    fn handle_action_at(
        &mut self,
        action: FavoritesOverlayAction,
        now: Instant,
    ) -> FavoritesOverlayActionOutcome {
        if !self.is_open() {
            return FavoritesOverlayActionOutcome::Quiet;
        }
        if action != FavoritesOverlayAction::Delete {
            self.cancel_deletion_confirmation();
        }
        match action {
            FavoritesOverlayAction::SelectPrevious => {
                self.viewport.up();
                self.refresh_footer();
            },
            FavoritesOverlayAction::SelectNext => {
                self.viewport.down();
                self.refresh_footer();
            },
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
                if let FavoriteSelection::Row(FavoriteRowIdentity::Recognized(favorite_id)) =
                    self.favorite_selection()
                    && let AppOverlay::Favorites(open_state) = &self.state
                    && let FavoritesOverlayContent::Rows(rows) = &open_state.content
                    && let FavoriteRowLookup::Found(row) = rows.row(favorite_id)
                {
                    self.notice = FavoritesOverlayNotice::NoNotice;
                    return FavoritesOverlayActionOutcome::Load(row.settings);
                }
            },
            FavoritesOverlayAction::Delete => {
                self.request_removal(now);
            },
            FavoritesOverlayAction::Close => return FavoritesOverlayActionOutcome::Close,
        }
        FavoritesOverlayActionOutcome::Quiet
    }

    fn request_removal(&mut self, now: Instant) {
        let FavoriteSelection::Row(identity) = self.favorite_selection() else {
            self.cancel_deletion_confirmation();
            return;
        };
        if matches!(
            &self.deletion_confirmation,
            FavoriteDeletionConfirmationState::AwaitingSecondPress(armed) if *armed == identity
        ) {
            self.deletion_confirmation = FavoriteDeletionConfirmationState::NoConfirmationArmed;
            self.notice = FavoritesOverlayNotice::NoNotice;
            self.start_removal(&identity, now);
            return;
        }

        self.deletion_confirmation =
            FavoriteDeletionConfirmationState::AwaitingSecondPress(identity);
        self.notice = FavoritesOverlayNotice::DeletionConfirmation {
            message: self.surface_bindings.delete_confirmation_notice(),
        };
    }

    fn cancel_deletion_confirmation(&mut self) {
        if matches!(
            self.deletion_confirmation,
            FavoriteDeletionConfirmationState::NoConfirmationArmed
        ) {
            return;
        }
        self.deletion_confirmation = FavoriteDeletionConfirmationState::NoConfirmationArmed;
        if matches!(
            self.notice,
            FavoritesOverlayNotice::DeletionConfirmation { .. }
        ) {
            self.notice = FavoritesOverlayNotice::NoNotice;
        }
    }

    fn start_removal(&mut self, identity: &FavoriteRowIdentity, now: Instant) {
        self.notice = FavoritesOverlayNotice::NoNotice;
        let AppOverlay::Favorites(open_state) = &mut self.state else {
            return;
        };
        let started = match (&mut open_state.content, identity) {
            (FavoritesOverlayContent::Rows(rows), FavoriteRowIdentity::Recognized(favorite_id)) => {
                match rows.row_mut(*favorite_id) {
                    FavoriteRowLookupMut::Found(row) => {
                        row.lifecycle = FavoriteRowLifecycle::Removing { since: now };
                        true
                    },
                    FavoriteRowLookupMut::Missing => false,
                }
            },
            (
                FavoritesOverlayContent::Rows(rows),
                FavoriteRowIdentity::Unrecognized(removal_locator),
            ) => rows
                .unrecognized
                .iter_mut()
                .find(|row| row.removal_locator == *removal_locator)
                .is_some_and(|row| {
                    row.lifecycle = FavoriteRowLifecycle::Removing { since: now };
                    true
                }),
            (
                FavoritesOverlayContent::OnlyUnrecognized(rows),
                FavoriteRowIdentity::Unrecognized(removal_locator),
            ) => rows
                .rows
                .iter_mut()
                .find(|row| row.removal_locator == *removal_locator)
                .is_some_and(|row| {
                    row.lifecycle = FavoriteRowLifecycle::Removing { since: now };
                    true
                }),
            (
                FavoritesOverlayContent::NoneSaved
                | FavoritesOverlayContent::OnlyUnrecognized(_)
                | FavoritesOverlayContent::LocationUnavailable
                | FavoritesOverlayContent::Unparseable { .. }
                | FavoritesOverlayContent::Unreadable { .. },
                FavoriteRowIdentity::Recognized(_) | FavoriteRowIdentity::Unrecognized(_),
            ) => false,
        };
        if started {
            self.surface_bindings.invalidate_footer();
            self.rebuild_for_cached_width();
        }
    }

    fn begin_close(&mut self) -> FavoritesOverlayCloseCommit {
        let removal_targets = match &self.state {
            AppOverlay::Favorites(open_state) => match &open_state.content {
                FavoritesOverlayContent::Rows(rows) => {
                    let mut targets = rows
                        .removing_ids()
                        .into_iter()
                        .map(FavoriteRemovalTarget::Recognized)
                        .collect::<Vec<_>>();
                    targets.extend(
                        rows.unrecognized
                            .iter()
                            .filter(|row| {
                                matches!(row.lifecycle, FavoriteRowLifecycle::Removing { .. })
                            })
                            .map(|row| {
                                FavoriteRemovalTarget::Unrecognized(row.removal_locator.clone())
                            }),
                    );
                    targets
                },
                FavoritesOverlayContent::OnlyUnrecognized(rows) => rows
                    .rows
                    .iter()
                    .filter(|row| matches!(row.lifecycle, FavoriteRowLifecycle::Removing { .. }))
                    .map(|row| FavoriteRemovalTarget::Unrecognized(row.removal_locator.clone()))
                    .collect(),
                FavoritesOverlayContent::NoneSaved
                | FavoritesOverlayContent::LocationUnavailable
                | FavoritesOverlayContent::Unparseable { .. }
                | FavoritesOverlayContent::Unreadable { .. } => Vec::new(),
            },
            AppOverlay::Closed => Vec::new(),
        };
        self.state = AppOverlay::Closed;
        self.notice = FavoritesOverlayNotice::NoNotice;
        self.deletion_confirmation = FavoriteDeletionConfirmationState::NoConfirmationArmed;
        self.removal_commit = FavoriteRemovalCommitState::NoCommitPending;
        FavoritesOverlayCloseCommit {
            removal_targets,
            retry: self.surface_bindings.close_delete_retry(),
        }
    }

    fn finish_close(&mut self) {
        self.viewport.clear_surface();
        self.line_plan = CachedLinePlan::default();
    }

    /// Advance any row-removal fade and request a file mutation when one completes.
    pub(crate) fn advance(&mut self, now: Instant) -> FavoritesOverlayFrameOutcome {
        if !matches!(
            self.removal_commit,
            FavoriteRemovalCommitState::NoCommitPending
        ) {
            return FavoritesOverlayFrameOutcome::Quiet;
        }
        let mut fade_in_progress = false;
        let mut completed = None;
        let AppOverlay::Favorites(open_state) = &self.state else {
            return FavoritesOverlayFrameOutcome::Quiet;
        };
        match &open_state.content {
            FavoritesOverlayContent::Rows(rows) => {
                for row in rows.sections.iter().flat_map(|section| &section.rows) {
                    if let FavoriteRowLifecycle::Removing { since } = row.lifecycle {
                        if now.duration_since(since) >= FAVORITE_REMOVAL_FADE {
                            completed = Some(FavoriteRowIdentity::Recognized(row.id));
                            break;
                        }
                        fade_in_progress = true;
                    }
                }
                if completed.is_none() {
                    for row in &rows.unrecognized {
                        if let FavoriteRowLifecycle::Removing { since } = row.lifecycle {
                            if now.duration_since(since) >= FAVORITE_REMOVAL_FADE {
                                completed = Some(FavoriteRowIdentity::Unrecognized(
                                    row.removal_locator.clone(),
                                ));
                                break;
                            }
                            fade_in_progress = true;
                        }
                    }
                }
            },
            FavoritesOverlayContent::OnlyUnrecognized(rows) => {
                for row in &rows.rows {
                    if let FavoriteRowLifecycle::Removing { since } = row.lifecycle {
                        if now.duration_since(since) >= FAVORITE_REMOVAL_FADE {
                            completed = Some(FavoriteRowIdentity::Unrecognized(
                                row.removal_locator.clone(),
                            ));
                            break;
                        }
                        fade_in_progress = true;
                    }
                }
            },
            FavoritesOverlayContent::NoneSaved
            | FavoritesOverlayContent::LocationUnavailable
            | FavoritesOverlayContent::Unparseable { .. }
            | FavoritesOverlayContent::Unreadable { .. } => {},
        }
        if let Some(identity) = completed {
            self.removal_commit = FavoriteRemovalCommitState::Pending(identity.clone());
            return FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget::from(
                identity,
            ));
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
        match &self.state {
            AppOverlay::Favorites(open_state) => match &open_state.content {
                FavoritesOverlayContent::Rows(rows) => removal_visual_deadline(
                    rows.sections
                        .iter()
                        .flat_map(|section| &section.rows)
                        .map(|row| row.lifecycle)
                        .chain(rows.unrecognized.iter().map(|row| row.lifecycle)),
                    now,
                    frame_period,
                ),
                FavoritesOverlayContent::OnlyUnrecognized(rows) => removal_visual_deadline(
                    rows.rows.iter().map(|row| row.lifecycle),
                    now,
                    frame_period,
                ),
                FavoritesOverlayContent::NoneSaved
                | FavoritesOverlayContent::LocationUnavailable
                | FavoritesOverlayContent::Unparseable { .. }
                | FavoritesOverlayContent::Unreadable { .. } => {
                    VisualDeadline::NoVisualChangeScheduled
                },
            },
            AppOverlay::Closed => VisualDeadline::NoVisualChangeScheduled,
        }
    }

    /// Reconcile one completed fade with the result of its file mutation.
    pub(crate) fn finish_removal(
        &mut self,
        removal_target: FavoriteRemovalTarget,
        result: Result<(), FavoritesMutationError>,
    ) {
        let identity = FavoriteRowIdentity::from(removal_target);
        self.finish_removal_identity(&identity, result);
    }

    fn finish_removal_identity(
        &mut self,
        identity: &FavoriteRowIdentity,
        result: Result<(), FavoritesMutationError>,
    ) {
        if self.removal_commit != FavoriteRemovalCommitState::Pending(identity.clone()) {
            return;
        }
        self.removal_commit = FavoriteRemovalCommitState::NoCommitPending;
        match result {
            Ok(()) => {
                self.drop_removed_row(identity);
                self.surface_bindings.invalidate_footer();
                self.rebuild_for_cached_width();
            },
            Err(error) => {
                self.restore_row_after_refusal(identity);
                self.notice = FavoritesOverlayNotice::DeletionRefused {
                    message: deletion_refusal_message(
                        &self.surface_bindings.delete_retry(),
                        &error,
                    ),
                };
                self.surface_bindings.invalidate_footer();
                self.rebuild_for_cached_width();
                self.select_favorite(identity);
            },
        }
    }

    fn restore_row_after_refusal(&mut self, identity: &FavoriteRowIdentity) {
        let AppOverlay::Favorites(open_state) = &mut self.state else {
            return;
        };
        match (&mut open_state.content, identity) {
            (FavoritesOverlayContent::Rows(rows), FavoriteRowIdentity::Recognized(favorite_id)) => {
                if let FavoriteRowLookupMut::Found(row) = rows.row_mut(*favorite_id) {
                    row.lifecycle = FavoriteRowLifecycle::Active;
                }
            },
            (
                FavoritesOverlayContent::Rows(rows),
                FavoriteRowIdentity::Unrecognized(removal_locator),
            ) => {
                if let Some(row) = rows
                    .unrecognized
                    .iter_mut()
                    .find(|row| row.removal_locator == *removal_locator)
                {
                    row.lifecycle = FavoriteRowLifecycle::Active;
                }
            },
            (
                FavoritesOverlayContent::OnlyUnrecognized(rows),
                FavoriteRowIdentity::Unrecognized(removal_locator),
            ) => {
                if let Some(row) = rows
                    .rows
                    .iter_mut()
                    .find(|row| row.removal_locator == *removal_locator)
                {
                    row.lifecycle = FavoriteRowLifecycle::Active;
                }
            },
            (
                FavoritesOverlayContent::NoneSaved
                | FavoritesOverlayContent::OnlyUnrecognized(_)
                | FavoritesOverlayContent::LocationUnavailable
                | FavoritesOverlayContent::Unparseable { .. }
                | FavoritesOverlayContent::Unreadable { .. },
                FavoriteRowIdentity::Recognized(_) | FavoriteRowIdentity::Unrecognized(_),
            ) => {},
        }
    }

    fn drop_removed_row(&mut self, identity: &FavoriteRowIdentity) {
        let AppOverlay::Favorites(open_state) = &mut self.state else {
            return;
        };
        match (&mut open_state.content, identity) {
            (FavoritesOverlayContent::Rows(rows), FavoriteRowIdentity::Recognized(favorite_id)) => {
                rows.remove(*favorite_id);
            },
            (
                FavoritesOverlayContent::Rows(rows),
                FavoriteRowIdentity::Unrecognized(removal_locator),
            ) => {
                rows.remove_unrecognized(removal_locator);
            },
            (
                FavoritesOverlayContent::OnlyUnrecognized(rows),
                FavoriteRowIdentity::Unrecognized(removal_locator),
            ) => {
                rows.rows
                    .retain(|row| row.removal_locator != *removal_locator);
            },
            (
                FavoritesOverlayContent::NoneSaved
                | FavoritesOverlayContent::OnlyUnrecognized(_)
                | FavoritesOverlayContent::LocationUnavailable
                | FavoritesOverlayContent::Unparseable { .. }
                | FavoritesOverlayContent::Unreadable { .. },
                FavoriteRowIdentity::Recognized(_) | FavoriteRowIdentity::Unrecognized(_),
            ) => {},
        }
        normalize_content_after_removal(&mut open_state.content);
    }

    fn select_favorite(&mut self, identity: &FavoriteRowIdentity) {
        for (position, line_index) in self.line_plan.navigation_line_index.iter().enumerate() {
            if matches!(
                self.line_plan.lines.get(*line_index),
                Some(CachedOverlayLine::Row { identity: row, .. }) if row == identity
            ) {
                self.viewport.set_pos(position);
                self.refresh_footer();
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
            | FavoritesOverlayNotice::DeletionConfirmation { message }
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
            AppOverlay::Favorites(open_state) => open_state.content.saved_count(),
        };
        let popup = PopupFrame {
            title: Some(favorites_heading(saved_count)),
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
            .map(|line| rendered_line(line, &selected, row_lifecycle(state, line), now))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(visible), content_area);
        render_notice(frame, &self.notice, notice_area);
        frame.render_widget(
            Paragraph::new(self.surface_bindings.footer())
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
            return FavoriteSelection::NoRowSelected;
        };
        match self.line_plan.lines.get(line_index) {
            Some(CachedOverlayLine::Row { identity, .. }) => {
                FavoriteSelection::Row(identity.clone())
            },
            Some(CachedOverlayLine::NonRow(_)) | None => FavoriteSelection::NoRowSelected,
        }
    }

    fn refresh_footer(&mut self) {
        let selected_favorite_actions = match self.favorite_selection() {
            FavoriteSelection::NoRowSelected => SelectedFavoriteActions::NoFavoriteSelected,
            FavoriteSelection::Row(FavoriteRowIdentity::Recognized(_)) => {
                SelectedFavoriteActions::LoadAndDelete
            },
            FavoriteSelection::Row(FavoriteRowIdentity::Unrecognized(_)) => {
                SelectedFavoriteActions::DeleteOnly
            },
        };
        self.surface_bindings.refresh_footer(
            self.line_plan.navigation_line_index.len(),
            self.line_plan.last_horizontal_column_page,
            selected_favorite_actions,
        );
    }

    fn rebuild_line_plan(&mut self, width: u16) {
        self.line_plan = match &self.state {
            AppOverlay::Closed => CachedLinePlan::default(),
            AppOverlay::Favorites(open_state) => build_line_plan(
                &open_state.content,
                &open_state.current_parameters,
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
        self.refresh_footer();
    }
}

fn favorites_heading(saved_count: usize) -> String {
    format!(" Favorites -- {saved_count} saved -- ● matches the current parameters ")
}

fn removal_visual_deadline(
    lifecycles: impl Iterator<Item = FavoriteRowLifecycle>,
    now: Instant,
    frame_period: Duration,
) -> VisualDeadline {
    lifecycles.fold(
        VisualDeadline::NoVisualChangeScheduled,
        |deadline, lifecycle| {
            let FavoriteRowLifecycle::Removing { since } = lifecycle else {
                return deadline;
            };
            let removal_done = since + FAVORITE_REMOVAL_FADE;
            let next_frame = now + frame_period;
            deadline.earlier(VisualDeadline::At(removal_done.min(next_frame)))
        },
    )
}

fn deletion_refusal_message(
    retry: &FavoritesRetryInstruction,
    error: &FavoritesMutationError,
) -> String {
    if matches!(error, FavoritesMutationError::UnrecognizedFavoriteChanged) {
        return "The favorites file changed after this row was loaded; nothing was deleted"
            .to_string();
    }
    favorites::favorite_refusal_message(FavoritesMutation::Delete, retry, error)
}

fn render_notice(frame: &mut Frame<'_>, notice: &FavoritesOverlayNotice, area: Rect) {
    let (message, color) = match notice {
        FavoritesOverlayNotice::NoNotice => return,
        FavoritesOverlayNotice::DeletionRefused { message } => (message, error_color()),
        FavoritesOverlayNotice::DeletionConfirmation { message }
        | FavoritesOverlayNotice::FavoriteAdjusted { message } => (message, warning_color()),
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
    for removal_target in close_commit.removal_targets {
        if let Err(error) = remove(removal_target) {
            let message = deletion_refusal_message(&close_commit.retry, &error);
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
        FavoritesOverlayContent::OnlyUnrecognized(rows) if rows.rows.is_empty() => {
            FavoritesOverlayContent::NoneSaved
        },
        other => other,
    };
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
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use tempfile::TempDir;
    use tui_pane::BandDirection;
    use tui_pane::BandFraying;
    use tui_pane::FocusedPane;
    use tui_pane::Framework;
    use tui_pane::KeyBind;
    use tui_pane::PixelFill;
    use tui_pane::PixelResolve;
    use tui_pane::ToastVisualDeadline;
    use unicode_width::UnicodeWidthStr;

    use super::bindings::BAND_COLUMNS_FOR_TEST as BAND_COLUMNS;
    use super::content::FavoriteRowsView;
    use super::line_plan::FavoriteSectionTableLayoutForTest;
    use super::line_plan::favorite_section_table_layout_for_test;
    use super::line_plan::removal_alpha_for_test as removal_alpha;
    use super::*;
    use crate::app::Updates;
    use crate::attract::AttractGridPresentation;
    use crate::attract::AttractVisibilityInstruction;
    use crate::attract::Work;
    use crate::constants::COLUMN_GAP;
    use crate::constants::FAVORITE_ROW_PREFIX_WIDTH;
    use crate::favorites;
    use crate::favorites::FavoriteId;
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
                CachedOverlayLine::NonRow(line) => line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect(),
                CachedOverlayLine::Row { tail, .. } => {
                    format!("{}{tail}", " ".repeat(FAVORITE_ROW_PREFIX_WIDTH))
                },
            })
            .collect()
    }

    fn display_column(line: &str, value: &str) -> usize {
        let byte_index = line
            .find(value)
            .expect("rendered table value should be present");
        UnicodeWidthStr::width(&line[..byte_index])
    }

    fn moving_band_table_layout(keymap: &Keymap<App>) -> FavoriteSectionTableLayoutForTest {
        let rows = favorites::parse_rows_for_overlay_test(MOVING_BAND_ROW)
            .expect("moving-band fixture should parse");
        let view = FavoriteRowsView::from(&rows);
        let bindings = FavoritesSurfaceBindings::resolve(keymap);
        favorite_section_table_layout_for_test(&view.sections[0], &bindings)
    }

    fn current_parameters() -> OpenFavoritesCurrentParameters {
        favorites::parse_rows_for_overlay_test(MOVING_BAND_ROW)
            .expect("current-parameters fixture should parse")
            .recognized()
            .next()
            .expect("current-parameters fixture should have a recognized row")
            .settings
            .into()
    }

    fn open_at_width(
        state: FavoritesFileState,
        keymap: &Keymap<App>,
        width: u16,
    ) -> FavoritesOverlay {
        let mut overlay = FavoritesOverlay::default();
        overlay.open_file_state(state, current_parameters(), keymap);
        overlay.cached_surface_width = CachedSurfaceWidth::Rendered(width);
        overlay.rebuild_line_plan(width);
        overlay
    }

    fn selected(overlay: &FavoritesOverlay) -> (FavoriteId, AttractSettings) {
        let FavoriteSelection::Row(FavoriteRowIdentity::Recognized(favorite_id)) =
            overlay.favorite_selection()
        else {
            panic!("fixture should select a recognized favorite");
        };
        let AppOverlay::Favorites(open_state) = &overlay.state else {
            panic!("fixture should contain recognized rows");
        };
        let FavoritesOverlayContent::Rows(rows) = &open_state.content else {
            panic!("fixture should contain recognized rows");
        };
        let FavoriteRowLookup::Found(row) = rows.row(favorite_id) else {
            panic!("selected favorite should remain in the overlay");
        };
        (favorite_id, row.settings)
    }

    fn selected_identity(overlay: &FavoritesOverlay) -> FavoriteRowIdentity {
        let FavoriteSelection::Row(identity) = overlay.favorite_selection() else {
            panic!("fixture should select a favorite row");
        };
        identity
    }

    fn start_selected_removal(overlay: &mut FavoritesOverlay, now: Instant) {
        let identity = selected_identity(overlay);
        overlay.start_removal(&identity, now);
    }

    fn lifecycle(overlay: &FavoritesOverlay, favorite_id: FavoriteId) -> FavoriteRowLifecycle {
        let AppOverlay::Favorites(open_state) = &overlay.state else {
            panic!("fixture should contain recognized rows");
        };
        let FavoritesOverlayContent::Rows(rows) = &open_state.content else {
            panic!("fixture should contain recognized rows");
        };
        let FavoriteRowLookup::Found(row) = rows.row(favorite_id) else {
            panic!("favorite should remain in the overlay");
        };
        row.lifecycle
    }

    fn identity_lifecycle(
        overlay: &FavoritesOverlay,
        identity: &FavoriteRowIdentity,
    ) -> FavoriteRowLifecycle {
        let line = overlay
            .line_plan
            .lines
            .iter()
            .find(|line| {
                matches!(
                    line,
                    CachedOverlayLine::Row { identity: row, .. } if row == identity
                )
            })
            .expect("favorite identity should remain in the line plan");
        row_lifecycle(&overlay.state, line)
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
    fn heading_explains_the_current_parameters_mark() {
        assert_eq!(
            favorites_heading(2),
            " Favorites -- 2 saved -- ● matches the current parameters "
        );
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
    fn unknown_mode_and_misspelled_enum_are_visible_and_selectable() {
        let keymap = keymap_from("");
        let overlay = open_at_width(loaded_state(UNRECOGNIZED_ROWS), &keymap, 100);
        let rendered = plan_text(&overlay.line_plan).join("\n");

        assert!(matches!(
            overlay.state,
            AppOverlay::Favorites(OpenFavoritesOverlayState {
                content: FavoritesOverlayContent::OnlyUnrecognized(_),
                ..
            })
        ));
        assert!(rendered.contains("mode = \"future_mode\" is not recognized"));
        assert!(rendered.contains("resolve = \"mist\" is not recognized"));
        assert_eq!(overlay.line_plan.selectable_line_index().len(), 2);
        assert_eq!(overlay.viewport.len(), 2);
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
        assert!(matches!(
            overlay.favorite_selection(),
            FavoriteSelection::Row(FavoriteRowIdentity::Unrecognized(_))
        ));
    }

    #[test]
    fn mixed_rows_scroll_past_the_last_favorite_to_diagnostics() {
        let keymap = keymap_from("");
        let mixed_rows = format!("{RECOGNIZED_ROWS}\n{UNRECOGNIZED_ROWS}");
        let mut overlay = open_at_width(loaded_state(&mixed_rows), &keymap, 100);
        assert!(
            overlay
                .line_plan
                .navigation_line_index
                .iter()
                .all(|index| matches!(
                    overlay.line_plan.lines[*index],
                    CachedOverlayLine::Row { .. }
                ))
        );
        let last_recognized_line = overlay
            .line_plan
            .lines
            .iter()
            .rposition(|line| {
                matches!(
                    line,
                    CachedOverlayLine::Row {
                        identity: FavoriteRowIdentity::Recognized(_),
                        ..
                    }
                )
            })
            .expect("mixed fixture should contain recognized favorites");

        for _ in 1..overlay.viewport.len() {
            overlay.handle_action(FavoritesOverlayAction::SelectNext);
        }
        let active_line = overlay.update_vertical_viewport(Rect::new(0, 0, 100, 2));
        let rendered = plan_text(&overlay.line_plan);

        assert!(active_line > last_recognized_line);
        assert_eq!(active_line, overlay.line_plan.lines.len() - 1);
        assert_eq!(
            overlay.viewport.scroll_offset(),
            overlay.line_plan.lines.len() - 2
        );
        assert!(rendered[active_line].contains("resolve = \"mist\" is not recognized"));
        assert!(matches!(
            overlay.favorite_selection(),
            FavoriteSelection::Row(FavoriteRowIdentity::Unrecognized(_))
        ));
    }

    #[test]
    fn selection_walks_sections_and_scrolls_to_stay_visible() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(RECOGNIZED_ROWS), &keymap, 100);

        assert_eq!(overlay.viewport.len(), 3);
        let first_line = overlay.line_plan.selectable_line_index()[0];
        let second_line = overlay.line_plan.selectable_line_index()[1];
        let pixel_line = overlay.line_plan.selectable_line_index()[2];
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
    fn horizontal_pages_keep_saved_pinned_and_cells_aligned() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(RECOGNIZED_ROWS), &keymap, 40);
        let first_page = plan_text(&overlay.line_plan);
        let first_header = first_page
            .iter()
            .find(|line| line.contains("Saved") && line.contains("Direction"))
            .expect("first page should show Direction");
        assert!(first_header.starts_with("   Saved"));

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
        assert!(header.starts_with("   Saved"));
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
        assert!(!overlay.surface_bindings.footer().contains("page"));
    }

    #[test]
    fn moving_band_pages_end_at_its_last_column_and_left_reverses_right() {
        let keymap = keymap_from("");
        let table_layout = moving_band_table_layout(&keymap);
        let width = u16::try_from(
            FAVORITE_ROW_PREFIX_WIDTH
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
    fn load_uses_retained_settings_and_reverses_fade_out() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let (_, settings) = selected(&overlay);
        let AppOverlay::Favorites(open_state) = &mut overlay.state else {
            panic!("fixture should contain recognized rows");
        };
        let FavoritesOverlayContent::Rows(rows) = &mut open_state.content else {
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
    fn load_refuses_an_unrecognized_row() {
        let keymap = keymap_from("");
        let mixed = format!("{MOVING_BAND_ROW}\n{UNRECOGNIZED_ROWS}");
        let mut overlay = open_at_width(loaded_state(&mixed), &keymap, 100);
        while overlay.viewport.pos() + 1 < overlay.viewport.len() {
            overlay.handle_action(FavoritesOverlayAction::SelectNext);
        }
        assert!(matches!(
            overlay.favorite_selection(),
            FavoriteSelection::Row(FavoriteRowIdentity::Unrecognized(_))
        ));
        let refusal = FavoritesOverlayNotice::DeletionRefused {
            message: "keep this refusal visible".to_string(),
        };
        overlay.notice = refusal.clone();

        assert_eq!(
            overlay.handle_action(FavoritesOverlayAction::Load),
            FavoritesOverlayActionOutcome::Quiet
        );
        assert_eq!(overlay.notice, refusal);
    }

    #[test]
    fn load_without_a_selected_recognized_row_preserves_the_existing_undo_point() {
        let keymap = keymap_from("");
        let mixed = format!("{MOVING_BAND_ROW}\n{UNRECOGNIZED_ROWS}");
        let mut overlay = open_at_width(loaded_state(&mixed), &keymap, 100);
        while overlay.viewport.pos() + 1 < overlay.viewport.len() {
            overlay.handle_action(FavoritesOverlayAction::SelectNext);
        }
        assert!(matches!(
            overlay.favorite_selection(),
            FavoriteSelection::Row(FavoriteRowIdentity::Unrecognized(_))
        ));
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
    fn delete_requires_two_presses_before_fade_and_commit() {
        let keymap = keymap_from("");
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("favorites.toml");
        fs::write(&path, MOVING_BAND_ROW).expect("favorite fixture should be written");
        let mut overlay = open_at_width(loaded_state_at(&path, MOVING_BAND_ROW), &keymap, 100);
        let (favorite_id, _) = selected(&overlay);
        let identity = FavoriteRowIdentity::Recognized(favorite_id);
        let started = Instant::now();

        assert_eq!(
            overlay.handle_action_at(FavoritesOverlayAction::Delete, started),
            FavoritesOverlayActionOutcome::Quiet
        );
        assert_eq!(
            overlay.deletion_confirmation,
            FavoriteDeletionConfirmationState::AwaitingSecondPress(identity.clone())
        );
        assert_eq!(
            identity_lifecycle(&overlay, &identity),
            FavoriteRowLifecycle::Active
        );
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::Quiet
        );
        assert_eq!(
            fs::read(&path).expect("favorite fixture should remain readable"),
            MOVING_BAND_ROW.as_bytes()
        );
        let FavoritesOverlayNotice::DeletionConfirmation { message } = &overlay.notice else {
            panic!("first delete should show confirmation");
        };
        assert!(message.contains("Press x again"));

        assert_eq!(
            overlay.handle_action_at(
                FavoritesOverlayAction::Delete,
                started + Duration::from_millis(1),
            ),
            FavoritesOverlayActionOutcome::Quiet
        );
        assert_eq!(
            identity_lifecycle(&overlay, &identity),
            FavoriteRowLifecycle::Removing {
                since: started + Duration::from_millis(1),
            }
        );
        assert_eq!(
            overlay.advance(started + Duration::from_millis(1) + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget::Recognized(
                favorite_id
            ))
        );
        overlay.finish_removal(FavoriteRemovalTarget::Recognized(favorite_id), Ok(()));
        assert!(matches!(
            overlay.state,
            AppOverlay::Favorites(OpenFavoritesOverlayState {
                content: FavoritesOverlayContent::NoneSaved,
                ..
            })
        ));
    }

    #[test]
    fn unrecognized_delete_commits_the_loaded_locator_after_confirmation() {
        let keymap = keymap_from("");
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("favorites.toml");
        fs::write(&path, UNRECOGNIZED_ROWS).expect("favorite fixture should be written");
        let original = fs::read(&path).expect("favorite fixture should be readable");
        let mut overlay = open_at_width(loaded_state_at(&path, UNRECOGNIZED_ROWS), &keymap, 100);
        let identity = selected_identity(&overlay);
        let FavoriteRowIdentity::Unrecognized(expected_locator) = &identity else {
            panic!("fixture should select an unrecognized row");
        };
        let expected_target = FavoriteRemovalTarget::Unrecognized(expected_locator.clone());
        let started = Instant::now();

        overlay.handle_action_at(FavoritesOverlayAction::Delete, started);
        overlay.handle_action_at(
            FavoritesOverlayAction::Delete,
            started + Duration::from_millis(1),
        );
        assert!(matches!(
            identity_lifecycle(&overlay, &identity),
            FavoriteRowLifecycle::Removing { .. }
        ));

        let outcome = overlay.advance(started + Duration::from_millis(1) + FAVORITE_REMOVAL_FADE);

        assert_eq!(
            outcome,
            FavoritesOverlayFrameOutcome::CommitRemoval(expected_target.clone())
        );
        assert_eq!(
            fs::read(&path).expect("favorite fixture should remain readable"),
            original
        );
        assert_eq!(
            overlay.advance(started + Duration::from_millis(1) + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::Quiet
        );
        overlay.finish_removal(expected_target, Ok(()));
        assert_eq!(overlay.viewport.len(), 1);
        assert!(matches!(
            overlay.favorite_selection(),
            FavoriteSelection::Row(FavoriteRowIdentity::Unrecognized(_))
        ));
    }

    #[test]
    fn moving_after_arming_delete_cannot_start_removal_on_either_row() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(RECOGNIZED_ROWS), &keymap, 100);
        let first = selected_identity(&overlay);
        let started = Instant::now();

        overlay.handle_action_at(FavoritesOverlayAction::Delete, started);
        overlay.handle_action(FavoritesOverlayAction::SelectNext);
        let second = selected_identity(&overlay);
        assert_ne!(first, second);
        assert_eq!(
            overlay.deletion_confirmation,
            FavoriteDeletionConfirmationState::NoConfirmationArmed
        );

        overlay.handle_action_at(
            FavoritesOverlayAction::Delete,
            started + Duration::from_millis(1),
        );
        assert_eq!(
            identity_lifecycle(&overlay, &first),
            FavoriteRowLifecycle::Active
        );
        assert_eq!(
            identity_lifecycle(&overlay, &second),
            FavoriteRowLifecycle::Active
        );
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::Quiet
        );
    }

    #[test]
    fn confirmation_cancellation_events_leave_the_file_untouched() {
        let keymap = keymap_from("");
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("favorites.toml");
        fs::write(&path, RECOGNIZED_ROWS).expect("favorite fixture should be written");
        let original = fs::read(&path).expect("favorite fixture should be readable");

        let mut moved = open_at_width(loaded_state_at(&path, RECOGNIZED_ROWS), &keymap, 100);
        moved.handle_action(FavoritesOverlayAction::Delete);
        moved.handle_action(FavoritesOverlayAction::SelectNext);
        assert_eq!(
            moved.deletion_confirmation,
            FavoriteDeletionConfirmationState::NoConfirmationArmed
        );
        assert_eq!(
            fs::read(&path).expect("file should remain readable"),
            original
        );

        let mut reloaded = open_at_width(loaded_state_at(&path, RECOGNIZED_ROWS), &keymap, 100);
        reloaded.handle_action(FavoritesOverlayAction::Delete);
        reloaded.open_file_state(
            loaded_state_at(&path, RECOGNIZED_ROWS),
            current_parameters(),
            &keymap,
        );
        assert_eq!(
            reloaded.deletion_confirmation,
            FavoriteDeletionConfirmationState::NoConfirmationArmed
        );
        assert_eq!(
            fs::read(&path).expect("file should remain readable"),
            original
        );

        let mut reopened = open_at_width(loaded_state_at(&path, RECOGNIZED_ROWS), &keymap, 100);
        reopened.handle_action(FavoritesOverlayAction::Delete);
        let close_commit = reopened.begin_close();
        assert!(close_commit.removal_targets.is_empty());
        reopened.finish_close();
        reopened.open_file_state(
            loaded_state_at(&path, RECOGNIZED_ROWS),
            current_parameters(),
            &keymap,
        );
        assert_eq!(
            reopened.deletion_confirmation,
            FavoriteDeletionConfirmationState::NoConfirmationArmed
        );
        assert_eq!(
            fs::read(&path).expect("file should remain readable"),
            original
        );

        let mut other_key = open_at_width(loaded_state_at(&path, RECOGNIZED_ROWS), &keymap, 100);
        other_key.handle_action(FavoritesOverlayAction::Delete);
        other_key.handle_action(FavoritesOverlayAction::PageColumnsLeft);
        assert_eq!(
            other_key.deletion_confirmation,
            FavoriteDeletionConfirmationState::NoConfirmationArmed
        );
        assert_eq!(
            fs::read(&path).expect("file should remain readable"),
            original
        );
    }

    #[test]
    fn stale_unrecognized_locator_preserves_file_and_reports_nothing_deleted() {
        let keymap = keymap_from("");
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("favorites.toml");
        fs::write(&path, UNRECOGNIZED_ROWS).expect("favorite fixture should be written");
        let mut overlay = open_at_width(loaded_state_at(&path, UNRECOGNIZED_ROWS), &keymap, 100);
        let identity = selected_identity(&overlay);
        let started = Instant::now();
        overlay.handle_action_at(FavoritesOverlayAction::Delete, started);
        overlay.handle_action_at(
            FavoritesOverlayAction::Delete,
            started + Duration::from_millis(1),
        );
        let changed = UNRECOGNIZED_ROWS.replace("future_mode", "future_mode_changed");
        fs::write(&path, &changed).expect("changed fixture should be written");
        let before_attempt = fs::read(&path).expect("changed fixture should be readable");

        let outcome = overlay.advance(started + Duration::from_millis(1) + FAVORITE_REMOVAL_FADE);

        let FavoritesOverlayFrameOutcome::CommitRemoval(removal_target) = outcome else {
            panic!("completed unrecognized fade should request a file mutation");
        };
        assert!(matches!(
            &removal_target,
            FavoriteRemovalTarget::Unrecognized(_)
        ));
        assert_eq!(
            fs::read(&path).expect("file should remain readable"),
            before_attempt
        );
        overlay.finish_removal(
            removal_target,
            Err(FavoritesMutationError::UnrecognizedFavoriteChanged),
        );
        assert_eq!(
            identity_lifecycle(&overlay, &identity),
            FavoriteRowLifecycle::Active
        );
        let FavoritesOverlayNotice::DeletionRefused { message } = &overlay.notice else {
            panic!("stale locator should produce an overlay refusal");
        };
        assert!(message.contains("file changed"));
        assert!(message.contains("nothing was deleted"));
    }

    #[test]
    fn deletion_fade_uses_elapsed_time_and_commits_once() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let (favorite_id, _) = selected(&overlay);
        let started = Instant::now();

        start_selected_removal(&mut overlay, started);
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
            FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget::Recognized(
                favorite_id
            ))
        );
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::Quiet
        );

        overlay.finish_removal(FavoriteRemovalTarget::Recognized(favorite_id), Ok(()));
        assert!(matches!(
            overlay.state,
            AppOverlay::Favorites(OpenFavoritesOverlayState {
                content: FavoritesOverlayContent::NoneSaved,
                ..
            })
        ));
    }

    #[test]
    fn two_removals_can_fade_together_and_each_commit_once() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(RECOGNIZED_ROWS), &keymap, 100);
        let started = Instant::now();
        let (first, _) = selected(&overlay);
        start_selected_removal(&mut overlay, started);
        let (second, _) = selected(&overlay);
        assert_ne!(first, second);
        start_selected_removal(&mut overlay, started + Duration::from_millis(1));

        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget::Recognized(first))
        );
        overlay.finish_removal(FavoriteRemovalTarget::Recognized(first), Ok(()));
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE + Duration::from_millis(1)),
            FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget::Recognized(second))
        );
        overlay.finish_removal(FavoriteRemovalTarget::Recognized(second), Ok(()));
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE + Duration::from_millis(1)),
            FavoritesOverlayFrameOutcome::Quiet
        );
        assert_eq!(
            match &overlay.state {
                AppOverlay::Favorites(open_state) => open_state.content.saved_count(),
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
        start_selected_removal(&mut overlay, started);
        let (second, _) = selected(&overlay);
        start_selected_removal(&mut overlay, started + Duration::from_millis(1));

        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget::Recognized(first))
        );
        overlay.finish_removal(
            FavoriteRemovalTarget::Recognized(first),
            Err(FavoritesMutationError::LockUnavailable {
                path:  PathBuf::from("/tmp/favorites.lock"),
                error: "held".to_string(),
            }),
        );
        let refusal = overlay.notice.clone();

        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE + Duration::from_millis(1)),
            FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget::Recognized(second))
        );
        overlay.finish_removal(FavoriteRemovalTarget::Recognized(second), Ok(()));

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

        start_selected_removal(&mut overlay, started);
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget::Recognized(
                favorite_id
            ))
        );
        overlay.finish_removal(FavoriteRemovalTarget::Recognized(favorite_id), Ok(()));

        assert!(matches!(
            overlay.state,
            AppOverlay::Favorites(OpenFavoritesOverlayState {
                content: FavoritesOverlayContent::OnlyUnrecognized(_),
                ..
            })
        ));
    }

    #[test]
    fn refused_deletion_restores_the_row_and_notice_until_retry() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let (favorite_id, _) = selected(&overlay);
        let started = Instant::now();
        start_selected_removal(&mut overlay, started);
        assert_eq!(
            overlay.advance(started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget::Recognized(
                favorite_id
            ))
        );
        overlay.finish_removal(
            FavoriteRemovalTarget::Recognized(favorite_id),
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
        start_selected_removal(&mut overlay, retry_started);
        assert_eq!(overlay.notice, FavoritesOverlayNotice::NoNotice);
        assert_eq!(
            overlay.advance(retry_started + FAVORITE_REMOVAL_FADE),
            FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget::Recognized(
                favorite_id
            ))
        );
        overlay.finish_removal(FavoriteRemovalTarget::Recognized(favorite_id), Ok(()));
        assert_eq!(overlay.notice, FavoritesOverlayNotice::NoNotice);
    }

    #[test]
    fn wrapped_refusal_renders_trailing_lock_error_with_realistic_path() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 74);
        let (favorite_id, _) = selected(&overlay);
        let started = Instant::now();
        start_selected_removal(&mut overlay, started);
        let _ = overlay.advance(started + FAVORITE_REMOVAL_FADE);
        overlay.finish_removal(
            FavoriteRemovalTarget::Recognized(favorite_id),
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
        start_selected_removal(&mut overlay, started);
        let _ = overlay.advance(started + FAVORITE_REMOVAL_FADE);
        overlay.finish_removal(
            FavoriteRemovalTarget::Recognized(favorite_id),
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
        assert!(rendered.iter().any(|line| line.contains('▸')));
    }

    #[test]
    fn write_refusal_also_restores_the_row() {
        let keymap = keymap_from("");
        let mut overlay = open_at_width(loaded_state(MOVING_BAND_ROW), &keymap, 100);
        let (favorite_id, _) = selected(&overlay);
        let started = Instant::now();
        start_selected_removal(&mut overlay, started);
        let _ = overlay.advance(started + FAVORITE_REMOVAL_FADE);
        overlay.finish_removal(
            FavoriteRemovalTarget::Recognized(favorite_id),
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
        start_selected_removal(&mut overlay, Instant::now());
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
        start_selected_removal(&mut overlay, Instant::now());
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
        start_selected_removal(&mut overlay, now);

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
        overlay.open_with_loader(&keymap, current_parameters(), || {
            loads += 1;
            FavoritesFileState::Missing {
                path: PathBuf::from("/tmp/favorites.toml"),
            }
        });
        assert_eq!(loads, 1);
        assert_eq!(overlay.viewport.len(), 0);
        let _ = overlay.begin_close();
        overlay.finish_close();

        overlay.open_with_loader(&keymap, current_parameters(), || {
            loads += 1;
            loaded_state(RECOGNIZED_ROWS)
        });
        assert_eq!(loads, 2);
        assert_eq!(overlay.viewport.len(), 3);
    }
}
