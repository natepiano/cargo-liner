#[cfg(test)]
mod readability_tests;
#[cfg(test)]
mod status_bar_contrast_tests;

use std::collections::HashSet;
use std::num::NonZeroU16;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use crossterm::cursor::MoveToColumn;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::buffer::CellDiffOption;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Margin;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use tui_pane::BLOCK_BORDER_WIDTH;
use tui_pane::BYTES_PER_GIB;
use tui_pane::BYTES_PER_KIB;
use tui_pane::BYTES_PER_MIB;
use tui_pane::BarPalette;
use tui_pane::ColorMode;
use tui_pane::FrameworkOverlayId;
use tui_pane::PaneBorders;
use tui_pane::PaneFocusState;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::ResolvedPaneLayout;
use tui_pane::ScanIndicator;
use tui_pane::StatusLine;
use tui_pane::StatusLineGlobal;
use tui_pane::StatusLineNote;
use tui_pane::ToastsRenderCtx;
use tui_pane::accent_color;
use tui_pane::error_color;
use tui_pane::label_color;
use tui_pane::render_status_line as render_framework_status_line;
use tui_pane::secondary_text_color;
use tui_pane::status_bar_color;
use tui_pane::success_color;
use tui_pane::text_default;
use tui_pane::title_color;
use unicode_width::UnicodeWidthStr;

use super::app::App;
use super::app::ConfirmAction;
use super::app::ConfirmationModalState;
use super::app_render_state::OverlayRenderInputs;
use super::columns::ProjectColumnLayout;
use super::constants::AFFECTED_EXTRAS_VISIBLE_CAP;
use super::constants::CONFIRM_DIALOG_HEIGHT;
use super::constants::CUT_TRAILING_SEPARATORS;
use super::constants::STRIP_SLOT_COUNT;
use super::integration::AppGlobalAction;
use super::interaction;
use super::overlays::PopupFrame;
use super::panes;
use super::panes::DescriptionState;
use super::panes::EmptyDescriptionBehavior;
use super::panes::OutputPaneVisibility;
use super::panes::PaneId;
use super::panes::SyncedDescriptionHeight;
use super::render_context::PaneRenderCtx;
use super::sccache;
use super::settings;
use super::state::CiStatusLookup;
use crate::ci::CiStatus;
use crate::constants::TARGET_DIR;
use crate::project;
use crate::project::AbsolutePath;

pub(super) fn format_bytes(bytes: u64) -> String { format_bytes_with_precision(bytes, 1) }

/// Round storage values to whole units for narrow project columns.
pub(super) fn format_bytes_compact(bytes: u64) -> String { format_bytes_with_precision(bytes, 0) }

fn format_bytes_with_precision(bytes: u64, precision: usize) -> String {
    #[allow(
        clippy::cast_precision_loss,
        reason = "display-only — sub-byte precision is irrelevant"
    )]
    if bytes >= BYTES_PER_GIB {
        format!("{:.*} GiB", precision, bytes as f64 / BYTES_PER_GIB as f64)
    } else if bytes >= BYTES_PER_MIB {
        format!("{:.*} MiB", precision, bytes as f64 / BYTES_PER_MIB as f64)
    } else if bytes >= BYTES_PER_KIB {
        format!("{:.*} KiB", precision, bytes as f64 / BYTES_PER_KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

pub(super) fn ci_status_style(ci_status: Option<CiStatus>) -> Style {
    match ci_status {
        Some(CiStatus::Passed) => Style::default().fg(success_color()),
        Some(CiStatus::Failed) => Style::default().fg(error_color()),
        _ => Style::default(),
    }
}

/// Resolve the tiled pane layout for the top area. Sizes the Details/Git top
/// row to the tallest project's content (measured across all projects and
/// cached against the scan generation and top-pane widths) so the
/// Lang/CPU/Targets row grows into the space the previous fixed split left
/// empty above it.
/// The description floor the Package and Git panes share for this frame:
/// both wrap their description against their own pane width, and the
/// taller of the two sets the height both render at, so the rows below
/// them line up across the two panes.
fn sync_description_floor(
    app: &App,
    tiled: &ResolvedPaneLayout<PaneId>,
    empty_package_description: EmptyDescriptionBehavior,
) -> SyncedDescriptionHeight {
    let pkg_block = panes::DescriptionBlock::for_pane(
        app.panes
            .package
            .content()
            .map_or(&DescriptionState::Pending, |d| &d.description),
        tiled.area(PaneId::Package),
        empty_package_description,
    );
    let git_block = panes::DescriptionBlock::for_pane(
        app.panes
            .git
            .content()
            .map_or(&DescriptionState::Pending, |d| &d.description),
        tiled.area(PaneId::Git),
        EmptyDescriptionBehavior::RenderNothing,
    );
    panes::sync_floor(&[&pkg_block, &git_block])
}

/// What the tiled panes read off the app for one frame, gathered before
/// the render borrows it mutably.
#[derive(Clone, Copy)]
struct TiledRenderInputs<'a> {
    selected_path:             Option<&'a Path>,
    empty_package_description: EmptyDescriptionBehavior,
    animation_elapsed:         Duration,
    ci_status_lookup:          &'a CiStatusLookup,
    synced_description_height: SyncedDescriptionHeight,
}

/// Draw toast cards through the shared renderer.
fn render_toasts_over_panes(frame: &mut Frame, app: &mut App, area: Rect) {
    app.refresh_startup_panel_for_layout(area);
    let area = app.toast_area(area);
    let toasts_pane_focused = app.focus_is(PaneId::Toasts);
    let toasts_ctx = ToastsRenderCtx {
        now:              Instant::now(),
        pane_focus_state: if toasts_pane_focused {
            PaneFocusState::Active
        } else {
            PaneFocusState::Inactive
        },
    };
    Renderable::render(&mut app.framework.toasts, frame, area, &toasts_ctx);
}

impl App {
    /// Keep toast cards inside the pane at their placement corner.
    pub(in crate::tui) fn toast_area(&self, bounds: Rect) -> Rect {
        let corner = self.framework.toasts.placement_corner(bounds);
        self.panes
            .tiled_layout
            .panes
            .iter()
            .find(|pane| pane.area.contains(corner))
            .map_or(bounds, |pane| pane.area.inner(Margin::new(1, 1)))
    }
}

/// Render each pane and then draw its borders and titles.
fn render_tiled_panes(
    frame: &mut Frame,
    app: &mut App,
    layout: &ResolvedPaneLayout<PaneId>,
    inputs: TiledRenderInputs<'_>,
) {
    let mut split = app.split_for_render(
        inputs.selected_path,
        inputs.animation_elapsed,
        inputs.ci_status_lookup,
        OverlayRenderInputs::none(),
        inputs.synced_description_height,
        inputs.empty_package_description,
    );
    tui_pane::render_panes(
        frame,
        &mut split.registry,
        layout,
        &split.pane_render_ctx,
        tui_pane::default_pane_chrome(split.pane_render_ctx.color_mode),
        // Cargo Port's panes are separate visual units, so `PaneBorders::Separate`
        // closes each box and highlights its focused border independently.
        // cargo-tile uses `PaneBorders::Shared` for shared border cells.
        PaneBorders::Separate,
    );
}

fn resolve_tiled_layout(
    app: &mut App,
    top_area: Rect,
    left_width: u16,
    bottom_row: panes::BottomRow,
) -> ResolvedPaneLayout<PaneId> {
    let core_count = app.panes.cpu.core_count();
    let minimum_left_width = u16::try_from(
        app.project_list.cached_fit_widths.minimum_table_width()
            + BLOCK_BORDER_WIDTH
            + panes::PROJECT_LIST_RIGHT_PADDING,
    )
    .unwrap_or(u16::MAX);
    let shown_left_width = u16::try_from(
        app.project_list.cached_fit_widths.shown_table_width()
            + BLOCK_BORDER_WIDTH
            + panes::PROJECT_LIST_RIGHT_PADDING,
    )
    .unwrap_or(u16::MAX);
    let project_column = ProjectColumnLayout {
        preferred_width: left_width,
        minimum_width:   minimum_left_width,
        shown_width:     shown_left_width,
        lint_column:     app.config.current().lint.enabled.into(),
    };
    let targets_width = app.panes.targets.preferred_width();
    let middle =
        panes::selected_language_stats(&app.project_list, app.selected_project_path_for_render())
            .map_or_else(
                || panes::MiddlePaneNeeds::minimum(targets_width),
                |stats| panes::MiddlePaneNeeds::for_selected(stats, targets_width),
            );
    let (package_width, git_width) = panes::top_pane_widths(top_area, project_column, middle);
    let key = (app.scan.generation(), package_width, git_width);
    let rows = if let Some(rows) = app.panes.cached_top_row_height(key) {
        rows
    } else {
        let rows = panes::max_top_pane_inner_height(app, package_width, git_width);
        app.panes.store_top_row_height(key, rows);
        rows
    };
    panes::resolve_layout(
        top_area,
        project_column,
        core_count,
        bottom_row,
        rows.top_inner,
        middle.with_rows(
            app.project_list.most_language_rows(),
            app.panes
                .targets
                .content()
                .map_or(0, panes::TargetsData::inner_rows),
        ),
    )
}

fn clear_pane_surfaces(app: &mut App) {
    app.panes.project_list.body_rect = Rect::ZERO;
    // Hidden panes must not keep mouse areas from the previous frame.
    app.lint.viewport.set_content_area(Rect::ZERO);
    app.ci.viewport.set_content_area(Rect::ZERO);
    app.panes.output.viewport.set_content_area(Rect::ZERO);
    app.panes.package.viewport.set_content_area(Rect::ZERO);
    app.panes.package.clear_row_rects();
    app.panes.git.viewport.set_content_area(Rect::ZERO);
    app.panes.git.clear_row_layout();
    app.panes.lang.viewport.set_content_area(Rect::ZERO);
    app.panes.cpu.viewport.set_content_area(Rect::ZERO);
    app.panes.cpu.clear_row_rects();
    app.panes.targets.viewport.set_content_area(Rect::ZERO);
    app.panes.targets.clear_row_rects();
}

fn move_focus_from_hidden_pane(app: &mut App, bottom_row: panes::BottomRow) {
    let focused = app.focused_pane_id();
    if panes::tab_order(bottom_row).contains(&focused)
        && (matches!(
            panes::PaneVisibility::in_layout(&app.panes.tiled_layout, focused),
            panes::PaneVisibility::Hidden
        ) || (matches!(focused, PaneId::Targets | PaneId::Lang)
            && !app.is_pane_tabbable(focused)))
    {
        let order = panes::tab_order(bottom_row);
        let start = order.iter().position(|pane| *pane == focused).unwrap_or(0);
        if let Some(next) = order
            .iter()
            .cycle()
            .skip(start + 1)
            .take(order.len())
            .copied()
            .find(|pane| app.is_pane_tabbable(*pane))
        {
            app.set_focus_to_pane(next);
        }
    }
}

pub(super) fn ui(frame: &mut Frame, app: &mut App) {
    sync_hovered_pane_row(app);
    clear_pane_surfaces(app);
    app.prune_toasts();

    // `app.themes.frame_background()` supplies a background only when active-theme
    // appearance differs from the terminal (such as forced dark on light). Render
    // its `Block` before panes, leaving terminal transparency otherwise.
    if let Some(color) = app.themes.frame_background() {
        frame.render_widget(
            Block::default().style(Style::default().bg(color)),
            frame.area(),
        );
    }

    let outer_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    // `left_width` combines `cached_fit_widths.total_width()` with `BLOCK_BORDER_WIDTH`
    // for the project column's left border and right gutter. The neighboring pane
    // supplies the right border in the grid pass, so it is not charged again.
    let left_width = u16::try_from(
        app.project_list.cached_fit_widths.total_width()
            + BLOCK_BORDER_WIDTH
            + panes::PROJECT_LIST_RIGHT_PADDING,
    )
    .unwrap_or(u16::MAX);

    let bottom_row = match app.output_pane_visibility() {
        OutputPaneVisibility::Hidden => panes::BottomRow::Diagnostics,
        OutputPaneVisibility::Visible => panes::BottomRow::Output,
    };
    let tiled = resolve_tiled_layout(app, outer_layout[0], left_width, bottom_row);

    app.panes.tiled_layout = tiled;
    app.reconcile_bottom_row_focus();
    move_focus_from_hidden_pane(app, bottom_row);

    // `sync_pane_focus` records each pane's focus before `App::split_for_render`,
    // allowing the trait-dispatched render loop to read focus from each pane's `&mut self`.
    sync_pane_focus(app);

    // Build the CI lookup snapshot now, while we still hold `&app.ci`;
    // the upcoming split takes `&mut app.ci` for the registry, which
    // would alias an `&Ci` ref carried in the ctx.
    let ci_status_lookup = app.ci.status_lookup();

    // `selected_project_path_for_render` reads project selection and scan state;
    // own `selected_path` before `split_for_render` needs mutable access to those owners.
    let selected_path: Option<PathBuf> = app
        .selected_project_path_for_render()
        .map(std::path::Path::to_path_buf);
    let animation_elapsed = app.animation_started.elapsed();

    // Sync description-section heights so the Package and Git panes'
    // description blocks render with matching row counts (and bottom
    // edges align) when both panes have a description. Build the
    // pre-wrapped DescriptionBlocks here and ask `sync_floor` for the
    // shared height — the same block is rebuilt by each pane's render
    // path with the same inputs, so the sync floor matches what each
    // pane actually draws (the structural enforcement).
    let empty_package_description = app.empty_package_description();
    let synced_description_height =
        sync_description_floor(app, &app.panes.tiled_layout, empty_package_description);
    let tiled = std::mem::take(&mut app.panes.tiled_layout);

    render_tiled_panes(
        frame,
        app,
        &tiled,
        TiledRenderInputs {
            selected_path: selected_path.as_deref(),
            empty_package_description,
            animation_elapsed,
            ci_status_lookup: &ci_status_lookup,
            synced_description_height,
        },
    );

    app.panes.tiled_layout = tiled;
    render_status_bar(frame, app, outer_layout[1]);
    if app.framework.overlay().is_some()
        || app.overlays.is_finder_open()
        || app.overlays.is_sccache_open()
        || matches!(
            app.confirmation_modal_state(),
            ConfirmationModalState::Open { .. }
        )
    {
        frame.render_widget(Clear, outer_layout[0]);
    }
    render_toasts_over_panes(frame, app, outer_layout[0]);

    if app.framework.overlay() == Some(FrameworkOverlayId::Settings) {
        dispatch_settings_overlay(app, frame);
    }
    if app.framework.overlay() == Some(FrameworkOverlayId::Keymap) {
        dispatch_keymap_overlay(app, frame);
    }
    if app.framework.overlay() == Some(FrameworkOverlayId::GlobalShortcuts) {
        dispatch_global_shortcuts_overlay(app, frame);
    }
    if app.overlays.is_finder_open() {
        dispatch_finder_render(app, frame);
    }
    if app.overlays.is_sccache_open() {
        sccache::render_sccache_popup(frame, app);
    }
    if let ConfirmationModalState::Open { action, readiness } = app.confirmation_modal_state() {
        let body = confirm_action_body(app, action);
        let verifying = readiness.is_verifying();
        render_confirm_popup(frame, action, &body, verifying, app.framework.color_mode);
    }

    sync_hovered_pane_row(app);
    prepare_terminal_buffer(frame.buffer_mut(), app.framework.color_mode);
}

/// Body lines shown below the `Run cargo clean?` prompt — everything
/// from the resolved target dir to the "Also affected" list and the
/// nested-crate summary. Pre-formatted into strings so render stays
/// a dumb pass-through.
fn confirm_action_body(app: &App, action: &ConfirmAction) -> Vec<String> {
    match action {
        ConfirmAction::Clean(project_path) => {
            let target = app
                .scan
                .resolve_target_dir(project_path)
                .unwrap_or_else(|| AbsolutePath::from(project_path.as_path().join(TARGET_DIR)));
            let mut lines = vec![project::home_relative_path(target.as_path())];

            // `app.scan.target_dir_index.siblings` finds projects sharing this target
            // directory outside the current selection. `handle_cargo_metadata_msg`
            // populates the index incrementally, so early startup can yield no affected
            // siblings; then the clean dialog retains only its resolved-target body line.
            let selection = [project_path.clone()];
            append_sibling_lines(app, &target, &selection, &mut lines);

            lines
        },
        ConfirmAction::CleanGroup { primary, linked } => {
            let mut lines = vec!["Checkouts:".to_string()];
            // Render `all_paths` up to `AFFECTED_EXTRAS_VISIBLE_CAP` so a group clean
            // cannot overflow the popup; collapse remaining checkouts into `+N more`
            // using the same cap as sibling lines.
            let all_paths: Vec<&AbsolutePath> = std::iter::once(primary).chain(linked).collect();
            for path in all_paths.iter().take(AFFECTED_EXTRAS_VISIBLE_CAP) {
                lines.push(format!("  {}", project::home_relative_path(path.as_path())));
            }
            if all_paths.len() > AFFECTED_EXTRAS_VISIBLE_CAP {
                let extra = all_paths.len() - AFFECTED_EXTRAS_VISIBLE_CAP;
                lines.push(format!("  +{extra} more"));
            }

            // `seen_targets` deduplicates resolved target directories before
            // `append_sibling_lines` unions their siblings: `ConfirmAction::CleanGroup`
            // can affect projects outside `selection`, just like a single clean.
            let selection: Vec<AbsolutePath> = all_paths.iter().copied().cloned().collect();
            let mut seen_targets: HashSet<AbsolutePath> = std::collections::HashSet::new();
            for path in &all_paths {
                let target = app
                    .scan
                    .resolve_target_dir(path)
                    .unwrap_or_else(|| AbsolutePath::from(path.as_path().join(TARGET_DIR)));
                if seen_targets.insert(target.clone()) {
                    append_sibling_lines(app, &target, &selection, &mut lines);
                }
            }

            lines
        },
        ConfirmAction::PauseLintProject(project_root) => vec![
            project::home_relative_path(project_root.as_path()),
            "Kills running lint jobs for this project.".to_string(),
            "Holds new runs until you resume it.".to_string(),
        ],
        ConfirmAction::PauseAllLints => vec![
            "Kills running lint jobs.".to_string(),
            "Holds new runs until you resume.".to_string(),
        ],
    }
}

/// Append the "Also affects:" block (sibling project paths + optional
/// nested-crate summary) for a single resolved target dir. Shared
/// between the `Clean` and `CleanGroup` body builders.
fn append_sibling_lines(
    app: &App,
    target: &AbsolutePath,
    selection: &[AbsolutePath],
    lines: &mut Vec<String>,
) {
    let siblings = app.scan.target_dir_index.siblings(target, selection);
    let project_siblings = siblings;
    if !project_siblings.is_empty() {
        lines.push("Also affects:".to_string());
        for sibling in project_siblings.iter().take(AFFECTED_EXTRAS_VISIBLE_CAP) {
            lines.push(format!(
                "  {}",
                project::home_relative_path(sibling.as_path())
            ));
        }
        if project_siblings.len() > AFFECTED_EXTRAS_VISIBLE_CAP {
            let extra = project_siblings.len() - AFFECTED_EXTRAS_VISIBLE_CAP;
            lines.push(format!("  +{extra} more"));
        }
    }
}

fn render_confirm_popup(
    frame: &mut Frame,
    action: &ConfirmAction,
    body: &[String],
    verifying: bool,
    color_mode: ColorMode,
) {
    // While the fingerprint re-check is in flight we swap the prompt
    // + keys for a "Verifying target dir…" placeholder and drop the
    // (y/n) suffix — `y` is ignored by handle_confirm_key in that
    // state, and showing it enabled would mislead the user.
    let prompt = match action {
        ConfirmAction::Clean(_) => "Run cargo clean?",
        ConfirmAction::CleanGroup { .. } => "Run cargo clean on all checkouts?",
        ConfirmAction::PauseLintProject(_) => "Pause lints for selected project?",
        ConfirmAction::PauseAllLints => "Pause all lints?",
    };
    let keys_suffix = if verifying { "" } else { " (y/n)" };
    let prompt_text = if verifying {
        " Verifying target dir… ".to_string()
    } else {
        format!(" {prompt} {keys_suffix} ")
    };
    let prompt_width = prompt_text.len();
    let body_max = body.iter().map(String::len).max().unwrap_or(0);
    // `body_width` adds one leading and one trailing space around the widest `body` line.
    let body_width = if body_max == 0 { 0 } else { body_max + 2 };
    let width = u16::try_from(prompt_width.max(body_width) + 4).unwrap_or(u16::MAX);
    let body_height = u16::try_from(body.len()).unwrap_or(u16::MAX);
    let height = CONFIRM_DIALOG_HEIGHT.saturating_add(body_height);

    let inner = PopupFrame {
        color_mode,
        title: None,
        border_color: title_color(),
        width,
        height,
    }
    .render(frame);

    let mut lines = if verifying {
        vec![Line::from(vec![Span::styled(
            " Verifying target dir… ",
            Style::default()
                .fg(label_color())
                .add_modifier(Modifier::ITALIC),
        )])]
    } else {
        vec![Line::from(vec![
            Span::styled(format!(" {prompt}  "), Style::default().fg(text_default())),
            Span::styled(
                "(y/n)",
                Style::default()
                    .fg(title_color())
                    .add_modifier(Modifier::BOLD),
            ),
        ])]
    };
    for body_line in body {
        lines.push(Line::from(vec![Span::styled(
            format!(" {body_line} "),
            Style::default().fg(label_color()),
        )]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Dispatch the Keymap overlay popup via the framework's
/// [`tui_pane::KeymapPane::render_overlay`] path. Inputs are built
/// while `&App` is still available, then `&mut keymap_pane` draws.
fn dispatch_keymap_overlay(app: &mut App, frame: &mut Frame) {
    // `app.framework.keymap_pane.focus` uses `PaneFocusState::Active` whenever the Keymap popup is
    // open.
    app.framework.keymap_pane.focus = RenderFocus {
        pane_focus_state: PaneFocusState::Active,
    };
    let inputs = tui_pane::KeymapPane::prepare_overlay_inputs(app, &app.framework_keymap);
    app.framework
        .keymap_pane
        .render_overlay(frame, frame.area(), &inputs);
}

/// Dispatch the Settings overlay popup via [`tui_pane::Renderable`].
/// Mirror of [`dispatch_keymap_overlay`] — the precompute step calls
/// [`tui_pane::SettingsPane::render_rows`] (which mutates the pane)
/// before `App::split_for_render`, then the trait method draws the
/// popup.
fn dispatch_settings_overlay(app: &mut App, frame: &mut Frame) {
    app.framework.settings_pane.focus = RenderFocus {
        pane_focus_state: PaneFocusState::Active,
    };
    let inputs = settings::prepare_settings_render_inputs(app, frame.area());
    let animation_elapsed = app.animation_started.elapsed();
    let selected_path: Option<PathBuf> = app
        .selected_project_path_for_render()
        .map(std::path::Path::to_path_buf);
    let ci_status_lookup = app.ci.status_lookup();
    let split = app.split_for_render(
        selected_path.as_deref(),
        animation_elapsed,
        &ci_status_lookup,
        OverlayRenderInputs::settings(&inputs),
        panes::SyncedDescriptionHeight::default(),
        app.empty_package_description(),
    );
    Renderable::render(
        split.registry.settings_pane,
        frame,
        frame.area(),
        &split.pane_render_ctx,
    );
}

/// Dispatch the framework-owned Global Shortcuts overlay popup.
fn dispatch_global_shortcuts_overlay(app: &mut App, frame: &mut Frame) {
    app.framework.global_shortcuts_pane.focus = RenderFocus {
        pane_focus_state: PaneFocusState::Active,
    };
    app.framework.global_shortcuts_pane.render(
        frame,
        frame.area(),
        &app.framework_keymap,
        app.framework.color_mode,
    );
}

fn dispatch_finder_render(app: &mut App, frame: &mut Frame) {
    let finder_focus = RenderFocus {
        pane_focus_state: app.pane_focus_state(PaneId::Finder),
    };
    app.overlays.finder_pane.focus = finder_focus;
    let animation_elapsed = app.animation_started.elapsed();
    let selected_project_path: Option<PathBuf> = app
        .selected_project_path_for_render()
        .map(std::path::Path::to_path_buf);
    let ci_status_lookup = app.ci.status_lookup();
    let mode = app.framework.color_mode;
    let split = app.split_finder_for_render();
    let pane_render_context = PaneRenderCtx {
        color_mode: mode,
        animation_elapsed,
        config: split.config,
        project_list: split.project_list,
        selected_project_path: selected_project_path.as_deref(),
        scan: split.scan,
        ci_status_lookup: &ci_status_lookup,
        settings_render_inputs: None,
        synced_description_height: panes::SyncedDescriptionHeight::default(),
        empty_package_description: EmptyDescriptionBehavior::ShowPlaceholder,
        output_presentation: split.output_presentation,
    };
    // `split.finder_pane` computes its popup size in `Renderable::render`,
    // so the passed `frame.area()` is unused.
    Renderable::render(split.finder_pane, frame, frame.area(), &pane_render_context);
}

/// Stamp each renderable pane's [`tui_pane::RenderFocus`] snapshot
/// before [`tui_pane::render_panes`] dispatches the loop. After this,
/// every pane reads its own focus state from `&mut self` instead of
/// the shared [`PaneRenderCtx`] — which is what frees the ctx of any
/// per-pane field and lets the generic loop carry one ctx per frame.
fn sync_pane_focus(app: &mut App) {
    let ids = [
        PaneId::Package,
        PaneId::Lang,
        PaneId::Cpu,
        PaneId::Git,
        PaneId::Targets,
        PaneId::ProjectList,
        PaneId::Output,
        PaneId::Lints,
        PaneId::CiRuns,
    ];
    for id in ids {
        let focus = RenderFocus {
            pane_focus_state: if app.framework.overlay().is_some()
                || app.overlays.is_finder_open()
                || app.overlays.is_sccache_open()
                || matches!(
                    app.confirmation_modal_state(),
                    ConfirmationModalState::Open { .. }
                ) {
                PaneFocusState::Inactive
            } else {
                app.pane_focus_state(id)
            },
        };
        match id {
            PaneId::Package => app.panes.package.focus = focus,
            PaneId::Lang => app.panes.lang.focus = focus,
            PaneId::Cpu => app.panes.cpu.focus = focus,
            PaneId::Git => app.panes.git.focus = focus,
            PaneId::Targets => app.panes.targets.focus = focus,
            PaneId::ProjectList => app.panes.project_list.focus = focus,
            PaneId::Output => app.panes.output.focus = focus,
            PaneId::Lints => app.lint.focus = focus,
            PaneId::CiRuns => app.ci.focus = focus,
            PaneId::Toasts
            | PaneId::Settings
            | PaneId::Finder
            | PaneId::Keymap
            | PaneId::Sccache => {},
        }
    }
}

fn sync_hovered_pane_row(app: &mut App) {
    let hovered = app
        .mouse_pos
        .and_then(|pos| interaction::hovered_pane_row_at(app, pos));
    app.panes.set_hover(hovered);
    app.apply_hovered_pane_row();
}

/// Preserve attributes and cursor positions in terminal output.
fn prepare_terminal_buffer(buffer: &mut Buffer, mode: ColorMode) {
    mode.clear_colors(buffer);
    for (cell, position) in buffer.content.iter_mut().zip(buffer.area.positions()) {
        if cell.symbol().ends_with('\u{fe0f}') {
            let width = u16::try_from(cell.symbol().width()).unwrap_or(u16::MAX);
            if let Some(width) = NonZeroU16::new(width) {
                // Clear every reserved column before drawing a VS16 glyph.
                let next_column = position.x.saturating_add(width.get());
                let symbol = format!(
                    "{}{}{}{}",
                    " ".repeat(usize::from(width.get())),
                    MoveToColumn(position.x),
                    cell.symbol(),
                    MoveToColumn(next_column)
                );
                cell.set_symbol(&symbol)
                    .set_diff_option(CellDiffOption::ForcedWidth(width));
            }
        }
    }
}

pub(super) fn truncate_to_width(text: &str, max_width: usize) -> &str {
    if text.width() <= max_width {
        return text;
    }
    let span = Span::raw(text);
    let mut width = 0;
    let mut end = 0;
    for grapheme in span.styled_graphemes(Style::default()) {
        width += grapheme.symbol.width();
        if width > max_width {
            break;
        }
        let Some(offset) = text[end..].find(grapheme.symbol) else {
            break;
        };
        end += offset + grapheme.symbol.len();
    }
    &text[..end]
}

pub(super) fn truncate_with_ellipsis(text: &str, max_width: usize, ellipsis: &str) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    if ellipsis.is_empty() {
        return truncate_to_width(text, max_width).to_string();
    }
    if max_width <= ellipsis.width() {
        return truncate_to_width(ellipsis, max_width).to_string();
    }
    let mut prefix = truncate_to_width(text, max_width.saturating_sub(ellipsis.width())).trim_end();
    while let Some(rest) = CUT_TRAILING_SEPARATORS
        .iter()
        .find_map(|separator| prefix.strip_suffix(separator))
    {
        prefix = rest.trim_end();
    }
    let mut shortened = String::with_capacity(prefix.len() + ellipsis.len());
    shortened.push_str(prefix);
    shortened.push_str(ellipsis);
    shortened
}

/// Keep visible rows and mark omitted rows on the last retained line.
pub(super) fn clip_rows_with_ellipsis(
    rows: &[String],
    visible: usize,
    width: usize,
) -> Vec<String> {
    let mut clipped: Vec<String> = rows.iter().take(visible).cloned().collect();
    if rows.len() > clipped.len()
        && let Some(last) = clipped.last_mut()
    {
        *last = truncate_with_ellipsis(&format!("{last}…"), width, "…");
    }
    clipped
}

/// Palette wiring `accent_color()` / `secondary_text_color()` / `Modifier::BOLD`
/// to the framework bar so `tui_pane::render_status_line` output uses
/// cargo-port's key/label styling. The framework ships a theme-neutral
/// [`tui_pane::BarPalette::default`]; cargo-port supplies its own colors
/// here.
pub(super) fn cargo_port_bar_palette() -> BarPalette {
    let enabled_key_style = Style::default()
        .fg(accent_color())
        .add_modifier(Modifier::BOLD);
    let disabled_key_style = Style::default()
        .fg(secondary_text_color())
        .add_modifier(Modifier::BOLD);
    let disabled_label_style = Style::default().fg(secondary_text_color());
    BarPalette {
        status_line_style: Style::default().bg(status_bar_color()).fg(text_default()),
        status_activity_style: enabled_key_style,
        status_label_style: Style::default()
            .fg(title_color())
            .add_modifier(Modifier::BOLD),
        status_value_style: Style::default().fg(text_default()),
        enabled_key_style,
        enabled_label_style: Style::default(),
        disabled_key_style,
        disabled_label_style,
        separator_style: Style::default(),
    }
}

const fn cargo_port_status_line_globals(
    _: &App,
) -> [StatusLineGlobal<AppGlobalAction>; STRIP_SLOT_COUNT] {
    [StatusLineGlobal::global_shortcuts_help()]
}

#[cfg(test)]
pub(super) fn cargo_port_global_text_for_test(app: &App) -> String {
    let globals = cargo_port_status_line_globals(app);
    tui_pane::status_line_global_spans::<App, AppGlobalAction>(
        &app.framework_keymap,
        &globals,
        &BarPalette::default(),
    )
    .iter()
    .map(|span| span.content.as_ref())
    .collect()
}

#[cfg(test)]
pub(super) fn cargo_port_right_text_for_test(
    app: &App,
    framework_global_spans: &[Span<'static>],
) -> String {
    if framework_global_spans.is_empty() {
        String::new()
    } else {
        cargo_port_global_text_for_test(app)
    }
}

/// Informational right-side segments: the sccache summary while sccache is
/// reporting one, nothing otherwise.
fn cargo_port_status_line_notes(app: &App) -> Vec<StatusLineNote> {
    app.sccache_status_line.note().into_iter().collect()
}

pub(super) fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let notes = cargo_port_status_line_notes(app);
    let globals = cargo_port_status_line_globals(app);
    let status = StatusLine::new(
        app.animation_started.elapsed().as_secs(),
        if app.scan.is_complete() {
            ScanIndicator::Hidden
        } else {
            ScanIndicator::Shown
        },
        &notes,
        &globals,
    )
    .with_individual_note_hints();
    render_framework_status_line::<App, AppGlobalAction>(
        frame,
        area,
        app,
        &app.framework_keymap,
        &app.framework,
        &cargo_port_bar_palette(),
        &status,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use ratatui::style::Modifier;
    use ratatui::style::Style;
    use tui_pane::ColorMode;

    use crate::config::LintIndicator;
    use crate::tui::columns::LintColumnStatus;
    use crate::tui::columns::ProjectColumnLayout;
    use crate::tui::panes;
    use crate::tui::panes::BottomRow;
    use crate::tui::panes::CPU_PANE_WIDTH;
    use crate::tui::panes::PaneId;
    use crate::tui::test_support;

    #[test]
    fn terminal_buffer_uses_the_selected_color_mode() {
        for (mode, foreground, background) in [
            (ColorMode::Colored, Color::Red, Color::Blue),
            (ColorMode::NoColor, Color::Reset, Color::Reset),
        ] {
            let mut buffer = Buffer::empty(Rect::new(0, 0, 1, 1));
            buffer[(0, 0)].set_symbol("x").set_style(
                Style::default()
                    .fg(Color::Red)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            );
            super::prepare_terminal_buffer(&mut buffer, mode);
            assert_eq!(buffer[(0, 0)].fg, foreground);
            assert_eq!(buffer[(0, 0)].bg, background);
            assert_eq!(buffer[(0, 0)].modifier, Modifier::BOLD);
            assert_eq!(buffer[(0, 0)].symbol(), "x");
        }
    }

    #[test]
    fn clip_rows_without_overflow_leaves_rows_untouched() {
        let rows = vec!["first".to_string(), "second".to_string()];
        assert_eq!(super::clip_rows_with_ellipsis(&rows, 3, 1), rows);
    }

    #[test]
    fn clip_rows_overflow_appends_to_short_last_row() {
        let rows = vec!["first".to_string(), "a".to_string(), "omitted".to_string()];
        assert_eq!(super::clip_rows_with_ellipsis(&rows, 2, 5), ["first", "a…"]);
    }

    #[test]
    fn clip_rows_overflow_replaces_full_width_last_character() {
        let rows = vec!["abcde".to_string(), "omitted".to_string()];
        assert_eq!(super::clip_rows_with_ellipsis(&rows, 1, 5), ["abcd…"]);
    }

    #[test]
    fn clip_rows_zero_visible_returns_nothing() {
        let rows = vec!["first".to_string()];
        assert_eq!(
            super::clip_rows_with_ellipsis(&rows, 0, 5),
            Vec::<String>::new()
        );
    }

    #[test]
    fn clip_rows_overflow_at_zero_width_has_empty_last_row() {
        let rows = vec!["first".to_string(), "omitted".to_string()];
        assert_eq!(super::clip_rows_with_ellipsis(&rows, 1, 0), [""]);
    }

    #[test]
    fn clip_rows_overflow_at_one_column_shows_ellipsis() {
        let rows = vec!["first".to_string(), "omitted".to_string()];
        assert_eq!(super::clip_rows_with_ellipsis(&rows, 1, 1), ["…"]);
    }

    #[test]
    fn clip_rows_overflow_respects_wide_unicode_columns() {
        let rows = vec!["界界".to_string(), "omitted".to_string()];
        assert_eq!(super::clip_rows_with_ellipsis(&rows, 1, 4), ["界…"]);
    }

    #[test]
    fn resolved_layout_keeps_cpu_column_fixed() {
        let narrow = panes::resolve_layout(
            Rect::new(0, 0, 80, 30),
            ProjectColumnLayout {
                preferred_width: 30,
                minimum_width:   30,
                shown_width:     30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            20,
            panes::MiddlePaneNeeds::minimum(18),
        );
        let wide = panes::resolve_layout(
            Rect::new(0, 0, 150, 30),
            ProjectColumnLayout {
                preferred_width: 30,
                minimum_width:   30,
                shown_width:     30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            20,
            panes::MiddlePaneNeeds::minimum(18),
        );

        assert_eq!(narrow.area(PaneId::Cpu).width, CPU_PANE_WIDTH);
        assert_eq!(wide.area(PaneId::Cpu).width, CPU_PANE_WIDTH);
    }

    #[test]
    fn top_row_has_no_dead_space_above_targets() {
        let layout = panes::resolve_layout(
            Rect::new(0, 0, 120, 30),
            ProjectColumnLayout {
                preferred_width: 30,
                minimum_width:   30,
                shown_width:     30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            20,
            panes::MiddlePaneNeeds::minimum(18),
        );
        let package = layout.area(PaneId::Package);
        let git = layout.area(PaneId::Git);
        let targets = layout.area(PaneId::Targets);
        let right_col = Rect::new(30, 0, 90, 30);

        assert_eq!(package.x, right_col.x);
        assert_eq!(
            git.x.saturating_add(git.width),
            right_col.x.saturating_add(right_col.width)
        );
        assert_eq!(package.width.saturating_add(git.width), right_col.width);
        assert_eq!(
            targets.x.saturating_add(targets.width),
            right_col.x.saturating_add(right_col.width)
        );
    }

    #[test]
    fn detail_panes_take_rows_unused_by_selected_targets() -> anyhow::Result<()> {
        for mode in [ColorMode::Colored, ColorMode::NoColor] {
            for (height, target_count) in [(30, 1), (45, 1), (45, 5)] {
                let mut app = test_support::project_app();
                app.framework.color_mode = mode;
                app.config.current_mut().lint.enabled = LintIndicator::Disabled;
                app.ensure_fit_widths_cached();
                app.project_list
                    .cached_fit_widths
                    .observe(crate::tui::columns::COL_NAME, 100);
                let mut data = test_support::target_data();
                data.binaries = (0..target_count)
                    .map(|index| {
                        let mut entry = data.binaries[0].clone();
                        entry.name = format!("target-{index}");
                        entry.display_name.clone_from(&entry.name);
                        entry
                    })
                    .collect();
                app.panes.targets.set_content(data);
                let mut terminal = Terminal::new(TestBackend::new(80, height))?;
                terminal.draw(|frame| super::ui(frame, &mut app))?;
                let package = app.panes.tiled_layout.area(PaneId::Package);
                let git = app.panes.tiled_layout.area(PaneId::Git);
                let mut most = panes::max_top_pane_inner_height(&app, package.width, git.width);
                most.top_inner = 60;
                most.targets_inner = 20;
                app.panes
                    .store_top_row_height((app.scan.generation(), package.width, git.width), most);
                terminal.draw(|frame| super::ui(frame, &mut app))?;
                let buffer = terminal.backend().buffer();
                let targets = app.panes.tiled_layout.area(PaneId::Targets);
                let package = app.panes.tiled_layout.area(PaneId::Package);
                let git = app.panes.tiled_layout.area(PaneId::Git);
                assert_eq!((package.width, git.width), (18, 17));
                assert_eq!(targets.height, u16::try_from(target_count)? + 3);
                assert_eq!(package.bottom(), targets.y);
                assert_eq!(git.bottom(), targets.y);
                assert!(app.panes.tiled_layout.area(PaneId::Cpu).is_empty());
                for index in 0..target_count {
                    assert!(
                        test_support::find_text_cell(buffer, targets, &format!("target-{index}"))
                            .is_some()
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn middle_row_expands_to_fit_all_cpu_rows_when_height_allows() {
        // `panes::resolve_layout` with top-row inner height 4 gives `PaneId::Cpu`
        // more than `panes::cpu_required_pane_height(12)`, fitting every CPU row and
        // leaving more height for Targets.
        let layout = panes::resolve_layout(
            Rect::new(0, 0, 120, 40),
            ProjectColumnLayout {
                preferred_width: 30,
                minimum_width:   30,
                shown_width:     30,
                lint_column:     LintColumnStatus::Hidden,
            },
            12,
            BottomRow::Diagnostics,
            4,
            panes::MiddlePaneNeeds::minimum(18),
        );

        assert!(layout.area(PaneId::Cpu).height > super::panes::cpu_required_pane_height(12));
    }
}
