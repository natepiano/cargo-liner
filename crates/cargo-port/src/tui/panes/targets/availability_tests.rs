use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use cargo_metadata::PackageId;
use cargo_metadata::TargetKind;
use cargo_metadata::semver::Version;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use tempfile::TempDir;
use tui_pane::ColorMode;

use crate::project::AbsolutePath;
use crate::project::Cargo;
use crate::project::ManifestFingerprint;
use crate::project::Package;
use crate::project::PackageRecord;
use crate::project::PublishPolicy;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::TargetRecord;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::project::WorkspaceMetadata;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::tui::dismiss_target::DismissTarget;
use crate::tui::finder;
use crate::tui::input;
use crate::tui::interaction;
use crate::tui::interaction::ClickMode;
use crate::tui::keymap::FinderAction;
use crate::tui::keymap::TargetsAction;
use crate::tui::panes;
use crate::tui::panes::PaneId;
use crate::tui::terminal;
use crate::tui::test_support;

struct TargetFixture {
    app:     App,
    root:    PathBuf,
    owners:  Vec<PathBuf>,
    scratch: TempDir,
}

fn package_record(path: &Path, name: &str) -> anyhow::Result<PackageRecord> {
    fs::create_dir_all(path.join("src"))?;
    fs::write(path.join("src/main.rs"), "fn main() {}")?;
    fs::write(
        path.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )?;
    Ok(PackageRecord {
        name:          name.to_string(),
        version:       Version::new(0, 1, 0),
        edition:       "2024".to_string(),
        description:   None,
        license:       None,
        homepage:      None,
        repository:    None,
        manifest_path: AbsolutePath::from(path.join("Cargo.toml")),
        targets:       vec![TargetRecord {
            name:              name.to_string(),
            kinds:             vec![TargetKind::Bin],
            src_path:          AbsolutePath::from(path.join("src/main.rs")),
            required_features: Vec::new(),
        }],
        publish:       PublishPolicy::Never,
    })
}

fn package(path: &Path, record: &PackageRecord) -> Package {
    let mut package = Package {
        path: AbsolutePath::from(path),
        name: Some(record.name.clone()),
        ..Package::default()
    };
    package.cargo = Cargo::from(record);
    package
}

impl TargetFixture {
    fn new(workspace: bool) -> anyhow::Result<Self> {
        let scratch = tempfile::tempdir()?;
        let root = scratch.path().join("project");
        let names = if workspace {
            vec!["alpha", "zeta"]
        } else {
            vec!["demo"]
        };
        let owners: Vec<_> = names
            .iter()
            .map(|name| {
                if workspace {
                    root.join(name)
                } else {
                    root.clone()
                }
            })
            .collect();
        let records = owners
            .iter()
            .zip(&names)
            .map(|(path, name)| package_record(path, name))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let packages: Vec<_> = owners
            .iter()
            .zip(&records)
            .map(|(path, record)| package(path, record))
            .collect();
        let items = if workspace {
            fs::write(
                root.join("Cargo.toml"),
                "[workspace]\nmembers = [\"alpha\", \"zeta\"]\n",
            )?;
            let mut items = vec![RootItem::Rust(RustProject::Workspace(Workspace {
                path: AbsolutePath::from(root.as_path()),
                ..Workspace::default()
            }))];
            items.extend(
                packages
                    .into_iter()
                    .map(|package| RootItem::Rust(RustProject::Package(package))),
            );
            items
        } else {
            vec![RootItem::Rust(RustProject::Package(packages[0].clone()))]
        };
        let mut app = test_support::make_app(&items);
        app.scan
            .metadata_store_handle()
            .lock()
            .map_err(|error| anyhow::anyhow!("{error}"))?
            .upsert(WorkspaceMetadata {
                declared_checkout_root:   AbsolutePath::from(root.as_path()),
                target_directory:         AbsolutePath::from(root.join("target")),
                packages:                 records
                    .into_iter()
                    .map(|record| {
                        (
                            PackageId {
                                repr: record.name.clone(),
                            },
                            record,
                        )
                    })
                    .collect(),
                fingerprint:              ManifestFingerprint::capture(&root)?,
                out_of_tree_target_bytes: None,
            });
        app.ensure_detail_cached();
        Ok(Self {
            app,
            root,
            owners,
            scratch,
        })
    }

    fn disk_reply(&mut self, owner: usize, bytes: u64) -> anyhow::Result<()> {
        send(
            &mut self.app,
            BackgroundMsg::DiskUsage {
                path: AbsolutePath::from(self.owners[owner].as_path()),
                bytes,
            },
        )
    }

    fn delete(&mut self, owner: usize) -> anyhow::Result<()> {
        fs::remove_dir_all(&self.owners[owner])?;
        self.disk_reply(owner, 0)
    }
}

fn send(app: &mut App, message: BackgroundMsg) -> anyhow::Result<()> {
    app.background.background_sender().send(message)?;
    app.poll_background();
    Ok(())
}

fn enter(app: &mut App) {
    input::handle_event(
        app,
        &Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
}

fn target_text(app: &mut App) -> anyhow::Result<String> {
    Ok(test_support::rows(&test_support::draw_pane(app, PaneId::Targets, 70)?).join("\n"))
}

fn run_hint(app: &mut App) -> anyhow::Result<String> {
    let buffer = test_support::draw_ui(app, 160)?;
    Ok(test_support::rows(&buffer)
        .last()
        .context("status row")?
        .clone())
}

#[test]
fn a_queued_target_cannot_start_after_its_owner_becomes_deleted() -> anyhow::Result<()> {
    let mut fixture = TargetFixture::new(true)?;
    fixture.app.set_focus_to_pane(PaneId::Targets);
    let buffer = test_support::draw_ui(&mut fixture.app, 160)?;
    let targets = fixture.app.panes.tiled_layout.area(PaneId::Targets);
    test_support::find_text_cell(&buffer, targets, "alpha").context("live owner target")?;
    enter(&mut fixture.app);
    fixture
        .app
        .project_list
        .at_path_mut(&fixture.owners[0])
        .expect_found("the owning package")
        .visibility = Visibility::Deleted;
    terminal::start_queued_target_for_test(&mut fixture.app);
    assert!(!fixture.app.inflight.owned_run().is_running());
    assert!(matches!(
        fixture.app.inflight.owned_run_termination(),
        crate::tui::state::OwnedRunTermination::NotRunning
    ));
    assert_eq!(
        fixture.app.inflight.example_output(),
        &["Failed to start: target owner is Deleted"]
    );
    assert!(matches!(
        fixture.app.inflight.begin_owned_run_launch(),
        crate::tui::state::OwnedRunLaunchStart::NotQueued
    ));
    Ok(())
}

#[test]
fn target_rows_and_runs_follow_owner_deletion_and_recreation() -> anyhow::Result<()> {
    let mut fixture = TargetFixture::new(false)?;
    fixture.app.set_focus_to_pane(PaneId::Targets);
    assert!(run_hint(&mut fixture.app)?.contains("release"));
    let buffer = test_support::draw_ui(&mut fixture.app, 160)?;
    let area = fixture.app.panes.tiled_layout.area(PaneId::Targets);
    let (x, y) = test_support::find_text_cell(&buffer, area, "demo").context("drawn target")?;
    fixture.delete(0)?;
    enter(&mut fixture.app);
    assert!(fixture.app.inflight.take_pending_example_run().is_none());
    assert!(!target_text(&mut fixture.app)?.contains("demo"));
    assert!(!fixture.app.is_pane_tabbable(PaneId::Targets));
    assert!(!run_hint(&mut fixture.app)?.contains("release"));
    interaction::handle_click(&mut fixture.app, Position::new(x, y), ClickMode::Dispatch);
    panes::dispatch_targets_action(TargetsAction::ReleaseBuild, &mut fixture.app);
    assert!(fixture.app.inflight.take_pending_example_run().is_none());
    package_record(&fixture.owners[0], "demo")?;
    fixture.disk_reply(0, 64)?;
    fixture.app.ensure_detail_cached();
    fixture.app.set_focus_to_pane(PaneId::Targets);
    assert!(run_hint(&mut fixture.app)?.contains("release"));
    assert!(target_text(&mut fixture.app)?.contains("demo"));
    enter(&mut fixture.app);
    let queued = fixture
        .app
        .inflight
        .take_pending_example_run()
        .context("restored target run")?;
    assert_eq!(queued.target_name, "demo");
    assert_eq!(
        queued.abs_path,
        fixture.scratch.path().join("project").display().to_string()
    );
    Ok(())
}

#[test]
fn workspace_targets_keep_live_owners_and_clamp_the_drawn_cursor() -> anyhow::Result<()> {
    let mut fixture = TargetFixture::new(true)?;
    fixture.app.set_focus_to_pane(PaneId::Targets);
    let text = target_text(&mut fixture.app)?;
    assert!(text.contains("alpha") && text.contains("zeta"));
    fixture.app.panes.targets.viewport.set_pos(1);
    fixture.delete(1)?;
    panes::dispatch_targets_action(TargetsAction::ReleaseBuild, &mut fixture.app);
    assert!(fixture.app.inflight.take_pending_example_run().is_none());
    let text = target_text(&mut fixture.app)?;
    assert!(text.contains("alpha"));
    assert!(!text.contains("zeta"));
    assert_eq!(fixture.app.panes.targets.viewport.pos(), 0);
    assert!(run_hint(&mut fixture.app)?.contains("release"));
    enter(&mut fixture.app);
    let queued = fixture
        .app
        .inflight
        .take_pending_example_run()
        .context("live owner run")?;
    assert_eq!(queued.target_name, "alpha");
    assert_eq!(queued.abs_path, fixture.root.display().to_string());
    Ok(())
}

#[test]
fn dismissed_and_rescanned_owners_leave_no_target_row_or_run() -> anyhow::Result<()> {
    let mut fixture = TargetFixture::new(false)?;
    assert!(target_text(&mut fixture.app)?.contains("demo"));
    fixture.delete(0)?;
    fixture
        .app
        .dismiss(DismissTarget::DeletedProject(AbsolutePath::from(
            fixture.root.as_path(),
        )));
    fixture.app.ensure_detail_cached();
    assert!(!target_text(&mut fixture.app)?.contains("demo"));
    panes::dispatch_targets_action(TargetsAction::ReleaseBuild, &mut fixture.app);
    assert!(fixture.app.inflight.take_pending_example_run().is_none());
    fixture.app.rescan();
    send(
        &mut fixture.app,
        BackgroundMsg::ScanResult {
            projects:     Vec::new(),
            disk_entries: Vec::new(),
        },
    )?;
    fixture.app.ensure_detail_cached();
    assert!(!target_text(&mut fixture.app)?.contains("demo"));
    assert!(!run_hint(&mut fixture.app)?.contains("release"));
    panes::dispatch_targets_action(TargetsAction::ReleaseBuild, &mut fixture.app);
    assert!(fixture.app.inflight.take_pending_example_run().is_none());
    Ok(())
}

#[test]
fn finder_target_selection_uses_the_selected_owner_and_holds_deleted_targets() -> anyhow::Result<()>
{
    let mut fixture = TargetFixture::new(true)?;
    input::open_finder(&mut fixture.app);
    let item = fixture
        .app
        .project_list
        .finder
        .index
        .iter()
        .position(|item| item.target_name.as_deref() == Some("zeta"))
        .context("indexed member target")?;
    fixture.app.project_list.finder.results = vec![item];
    finder::dispatch_finder_action(FinderAction::Activate, &mut fixture.app);
    let text = target_text(&mut fixture.app)?;
    assert!(text.contains("zeta"));
    assert!(!text.contains("alpha"));
    assert_eq!(
        fixture.app.project_list.selected_project_path(),
        Some(fixture.owners[1].as_path())
    );
    input::open_finder(&mut fixture.app);
    let item = fixture
        .app
        .project_list
        .finder
        .index
        .iter()
        .position(|item| item.target_name.as_deref() == Some("zeta"))
        .context("saved member target")?;
    fixture.app.project_list.finder.results = vec![item];
    fixture.delete(1)?;
    finder::dispatch_finder_action(FinderAction::Activate, &mut fixture.app);
    assert!(!target_text(&mut fixture.app)?.contains("zeta"));
    assert!(!fixture.app.is_pane_tabbable(PaneId::Targets));
    panes::dispatch_targets_action(TargetsAction::ReleaseBuild, &mut fixture.app);
    assert!(fixture.app.inflight.take_pending_example_run().is_none());
    Ok(())
}

#[test]
fn deleted_workspace_holds_every_cached_member_target() -> anyhow::Result<()> {
    let mut fixture = TargetFixture::new(true)?;
    fixture.app.set_focus_to_pane(PaneId::Targets);
    assert!(target_text(&mut fixture.app)?.contains("alpha"));
    fs::remove_dir_all(&fixture.root)?;
    send(
        &mut fixture.app,
        BackgroundMsg::DiskUsage {
            path:  AbsolutePath::from(fixture.root.as_path()),
            bytes: 0,
        },
    )?;
    enter(&mut fixture.app);
    assert!(fixture.app.inflight.take_pending_example_run().is_none());
    let text = target_text(&mut fixture.app)?;
    assert!(!text.contains("alpha"));
    assert!(!text.contains("zeta"));
    assert!(!run_hint(&mut fixture.app)?.contains("release"));
    assert!(!fixture.app.is_pane_tabbable(PaneId::Targets));
    Ok(())
}

fn assert_one_focused_border(app: &App, buffer: &Buffer) {
    let focused = [
        PaneId::ProjectList,
        PaneId::Package,
        PaneId::Git,
        PaneId::Lang,
        PaneId::Cpu,
        PaneId::Targets,
        PaneId::Lints,
        PaneId::CiRuns,
        PaneId::Output,
    ]
    .into_iter()
    .filter(|pane| {
        let area = app.panes.tiled_layout.area(*pane);
        !area.is_empty() && (area.x..area.right()).any(|x| buffer[(x, area.y)].symbol() == "━")
    })
    .collect::<Vec<_>>();
    assert_eq!(focused, vec![app.focused_pane_id()]);
}

#[test]
fn deleting_the_last_target_moves_focus_to_a_drawn_pane() -> anyhow::Result<()> {
    let mut fixture = TargetFixture::new(false)?;
    fixture.app.framework.color_mode = ColorMode::NoColor;
    fixture.app.set_focus_to_pane(PaneId::Targets);
    test_support::draw_ui(&mut fixture.app, 160)?;
    fixture.delete(0)?;
    let buffer = test_support::draw_ui(&mut fixture.app, 160)?;
    assert_ne!(fixture.app.focused_pane_id(), PaneId::Targets);
    assert_one_focused_border(&fixture.app, &buffer);
    Ok(())
}

#[test]
fn hidden_output_with_no_target_moves_focus_to_a_drawn_pane() -> anyhow::Result<()> {
    let mut fixture = TargetFixture::new(false)?;
    fixture.app.framework.color_mode = ColorMode::NoColor;
    fixture.delete(0)?;
    fixture.app.set_focus_to_pane(PaneId::Output);
    let buffer = test_support::draw_ui(&mut fixture.app, 160)?;
    assert_ne!(fixture.app.focused_pane_id(), PaneId::Targets);
    assert_ne!(fixture.app.focused_pane_id(), PaneId::Output);
    assert_one_focused_border(&fixture.app, &buffer);
    Ok(())
}

#[test]
fn a_live_workspace_target_keeps_focus_after_another_owner_is_deleted() -> anyhow::Result<()> {
    let mut fixture = TargetFixture::new(true)?;
    fixture.app.framework.color_mode = ColorMode::NoColor;
    fixture.app.set_focus_to_pane(PaneId::Targets);
    test_support::draw_ui(&mut fixture.app, 160)?;
    fixture.delete(0)?;
    let buffer = test_support::draw_ui(&mut fixture.app, 160)?;
    assert_eq!(fixture.app.focused_pane_id(), PaneId::Targets);
    let area = fixture.app.panes.tiled_layout.area(PaneId::Targets);
    test_support::find_text_cell(&buffer, area, "zeta").context("live target")?;
    assert_one_focused_border(&fixture.app, &buffer);
    Ok(())
}
