use std::path::Path;

use anyhow::Context;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use tui_pane::Appearance;
use tui_pane::CpuBreakdown;
use tui_pane::CpuCoreUsage;
use tui_pane::CpuUsage;
use tui_pane::PaneBorders;
use tui_pane::ResolvedPane;
use tui_pane::ResolvedPaneLayout;
use tui_pane::Theme;

use crate::project::DiskMeasurement;
use crate::project::Package;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::sccache::StatsResult;
use crate::tui::app::App;
use crate::tui::app_render_state::OverlayRenderInputs;
use crate::tui::panes::PaneId;
use crate::tui::panes::RunTargetKind;
use crate::tui::panes::SyncedDescriptionHeight;
use crate::tui::panes::TargetEntry;
use crate::tui::panes::TargetsData;
use crate::tui::render;
use crate::tui::sccache;
use crate::tui::test_support;
use crate::tui::theme_roles;

pub(in crate::tui) const PROJECT: &str = "readable-project";
pub(in crate::tui) const PROJECT_PATH: &str = "/tmp/test/readable-project";
pub(in crate::tui) const WIDTH: u16 = 160;
pub(in crate::tui) const HEIGHT: u16 = 45;

pub(in crate::tui) fn target_data() -> TargetsData {
    TargetsData {
        binaries: ["first", "second"]
            .map(|name| TargetEntry {
                name:              name.to_string(),
                display_name:      name.to_string(),
                run_target_kind:   RunTargetKind::Binary,
                source:            crate::tui::panes::TargetSource::workspace_root(
                    PROJECT.to_string(),
                ),
                project_path:      crate::project::AbsolutePath::from(PROJECT_PATH),
                package_path:      crate::project::AbsolutePath::from(PROJECT_PATH),
                package_name:      PROJECT.to_string(),
                src_path:          crate::project::AbsolutePath::from(format!(
                    "{PROJECT_PATH}/src/main.rs"
                )),
                required_features: Vec::new(),
            })
            .to_vec(),
        ..crate::tui::panes::TargetsData::default()
    }
}

pub(in crate::tui) fn project_app() -> App {
    let mut package = Package {
        path: crate::project::AbsolutePath::from(PROJECT_PATH),
        name: Some(PROJECT.to_string()),
        ..crate::project::Package::default()
    };
    package.disk = DiskMeasurement::TotalOnly(1_073_741_824);
    let project = RootItem::Rust(RustProject::Package(package));
    let mut app = test_support::make_app(&[project]);
    app.scan
        .manifest_metadata
        .record_success(&crate::project::AbsolutePath::from(PROJECT_PATH));
    app.ensure_visible_rows_cached();
    app.ensure_detail_cached();
    app
}

pub(in crate::tui) fn draw_ui(app: &mut App, width: u16) -> anyhow::Result<Buffer> {
    app.ensure_fit_widths_cached();
    app.ensure_disk_cache();
    let mut terminal = Terminal::new(TestBackend::new(width, HEIGHT))?;
    let frame = terminal.draw(|frame| render::ui(frame, app))?;
    Ok(frame.buffer.clone())
}

pub(in crate::tui) fn draw_pane(app: &mut App, pane: PaneId, width: u16) -> anyhow::Result<Buffer> {
    draw_pane_at_size(app, pane, width, HEIGHT)
}

pub(in crate::tui) fn draw_pane_at_size(
    app: &mut App,
    pane: PaneId,
    width: u16,
    height: u16,
) -> anyhow::Result<Buffer> {
    app.ensure_fit_widths_cached();
    app.ensure_disk_cache();
    let lookup = app.ci.status_lookup();
    let path = app
        .project_list
        .selected_project_path()
        .map(Path::to_path_buf);
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    terminal.draw(|frame| {
        frame.render_widget(
            ratatui::widgets::Block::default()
                .style(ratatui::style::Style::default().bg(tui_pane::theme().text.bg_focus.color)),
            frame.area(),
        );
        let mut split = app.split_for_render(
            path.as_deref(),
            std::time::Duration::ZERO,
            &lookup,
            OverlayRenderInputs::none(),
            SyncedDescriptionHeight::default(),
            app.empty_package_description(),
        );
        let layout = ResolvedPaneLayout::new(vec![ResolvedPane {
            pane,
            area: frame.area(),
        }]);
        tui_pane::render_panes(
            frame,
            &mut split.registry,
            &layout,
            &split.pane_render_ctx,
            tui_pane::default_pane_chrome(split.pane_render_ctx.color_mode),
            PaneBorders::Separate,
        );
    })?;
    Ok(terminal.backend().buffer().clone())
}

pub(in crate::tui) fn rows(buffer: &Buffer) -> Vec<String> {
    (buffer.area.y..buffer.area.bottom())
        .map(|y| {
            (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

pub(in crate::tui) fn position(buffer: &Buffer, text: &str) -> anyhow::Result<(u16, u16)> {
    test_support::find_text_cell(buffer, buffer.area, text)
        .with_context(|| format!("missing {text:?}: {:?}", rows(buffer)))
}

pub(in crate::tui) fn stats_app(lines: Vec<String>) -> App {
    let mut app = test_support::make_app(&[]);
    let request = app.overlays.sccache_pane.start_loading();
    app.overlays
        .sccache_pane
        .apply_result(request, StatsResult::Ready(lines));
    app
}

pub(in crate::tui) fn draw_stats(app: &mut App, height: u16) -> anyhow::Result<Buffer> {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, height))?;
    terminal.draw(|frame| sccache::render_sccache_popup(frame, app))?;
    Ok(terminal.backend().buffer().clone())
}

pub(in crate::tui) fn shipped_dark_theme() -> anyhow::Result<Theme> {
    let mut theme = crate::themes::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == crate::constants::DEFAULT_DARK_THEME)
        .context("the shipped dark theme")?
        .theme;
    theme_roles::apply_role_defaults_to_theme(
        &mut theme,
        Some(&tui_pane::ThemeId::new(
            crate::constants::DEFAULT_DARK_THEME,
        )),
        Appearance::Dark,
    );
    Ok(theme)
}

pub(in crate::tui) fn sampled_cpu_app(core_count: usize) -> App {
    let mut app = project_app();
    app.panes.cpu.install_sample_for_test(CpuUsage {
        total_percent: 50,
        cores:         (0..core_count)
            .map(|index| CpuCoreUsage {
                label:                               format!("CPU {}", index + 1),
                percent:                             50,
                #[cfg(target_os = "macos")]
                cluster:                             None,
            })
            .collect(),
        breakdown:     CpuBreakdown {
            system: 10,
            user:   40,
            idle:   50,
        },
        gpu:           tui_pane::GpuUsage::default(),
    });
    app
}
