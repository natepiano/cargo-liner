#![allow(
    clippy::expect_used,
    reason = "tests require valid fixture paths and queued messages"
)]

use std::io::Error;
use std::io::ErrorKind;
use std::time::Duration;
use std::time::Instant;

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::backend::ClearType;
use ratatui::backend::TestBackend;
use ratatui::backend::WindowSize;
use ratatui::buffer::Cell;
use ratatui::layout::Position;
use ratatui::layout::Size;
use tui_pane::Repaint;
use tui_pane::ToastCountdownState;
use tui_pane::ToastDuration;
use tui_pane::ToastLingerState;
use tui_pane::ToastVisualDeadline;
use tui_pane::Toasts;
use tui_pane::TrackedItem;
use tui_pane::TrackedItemLifecycle;
use tui_pane::TrackedItemTiming;

use super::FrameDraw;
use super::FrameKey;
use super::FrameOutput;
use super::FrameState;
use super::draw_frame;
use super::draw_frame_with_renderer;
use super::process_input_frame;
use super::status_wait;
use crate::channel;
use crate::project::AbsolutePath;
use crate::project::DiskMeasurement;
use crate::project::FileStamp;
use crate::project::ManifestFingerprint;
use crate::project::NonRustProject;
use crate::project::Package;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Workspace;
use crate::project::WorkspaceMetadata;
use crate::scan::BackgroundMsg;
use crate::scan::CargoMetadataError;
use crate::scan::ProjectStorage;
use crate::sccache::SccacheSummary;
use crate::tui::app::App;
use crate::tui::app::ConfirmAction;
use crate::tui::app::ROW_PRESENTATION_BUILDS;
use crate::tui::project_list::ProjectPathLookup::Found;
use crate::tui::test_support;

fn project(path: &str) -> RootItem {
    RootItem::Rust(RustProject::Package(Package {
        path: AbsolutePath::from(path),
        ..Package::default()
    }))
}

#[test]
fn idle_iteration_draws_only_the_first_frame() {
    let app = test_support::make_app(&[]);
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    assert!(!frames.needs_draw(&app, now));
}

#[test]
fn the_loop_wakes_and_ticks_at_the_status_line_second() {
    let mut app = test_support::make_app(&[]);
    let now = Instant::now();
    let wait = status_wait(&app, now, Duration::from_secs(10));
    assert!(wait <= Duration::from_secs(1));
    let due = now + wait;
    assert_eq!(app.framework.tick_status(due), Repaint::Needed);
    assert_eq!(app.framework.tick_status(due), Repaint::NotNeeded);
}

#[test]
fn running_scan_draws_only_when_the_supplied_time_reaches_its_deadline() -> Result<(), &'static str>
{
    let app = test_support::make_app(&[]);
    assert!(app.scan.needs_animation());
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    let FrameState::Drawn {
        next_visual_change: ToastVisualDeadline::At(deadline),
        ..
    } = &frames
    else {
        return Err("a running scan must schedule an animation frame");
    };
    let deadline = *deadline;
    assert!(deadline > now);
    let just_before = deadline
        .checked_sub(Duration::from_nanos(1))
        .ok_or("the animation deadline must allow one nanosecond before it")?;
    assert!(!frames.needs_draw(&app, just_before));
    assert!(frames.needs_draw(&app, deadline));
    assert!(!frames.needs_draw(&app, deadline));
    Ok(())
}

#[test]
fn supplied_time_controls_toast_countdown_fade_and_the_next_frame() -> anyhow::Result<()> {
    let mut app = test_support::make_app(&[]);
    queue(
        &mut app,
        BackgroundMsg::ScanResult {
            projects:     Vec::new(),
            disk_entries: Vec::new(),
        },
    );
    assert!(!app.scan.needs_animation());
    let mut settings = app.framework.toasts.settings().clone();
    settings.finished_task_visible = ToastDuration::try_from_secs("finished_task_visible", 4.0)?;
    app.framework.toasts = Toasts::with_settings(settings);
    let task = app.framework.toasts.start_task("scan", "finished");
    let completed_at = Instant::now() + Duration::from_secs(60);
    let mut item = TrackedItem::new("repository", "repository");
    item.timing = TrackedItemTiming::CompletedWithoutStart { completed_at };
    assert!(app.framework.toasts.set_tracked_items(task, &[item]));
    let now = completed_at + Duration::from_secs(2);
    let key = FrameKey::from_app(&app, now);
    assert_eq!(key.toasts.len(), 1);
    let toast = &key.toasts[0];
    assert_eq!(
        toast.countdown,
        ToastCountdownState::CountingDown { remaining_secs: 2 }
    );
    assert_eq!(toast.linger, ToastLingerState::Lingering { progress: 0.5 });
    assert_eq!(toast.items.len(), 1);
    assert_eq!(
        toast.items[0].lifecycle,
        TrackedItemLifecycle::CompletedFading { progress: 0.5 }
    );
    let real_time_view = app.framework.toasts.active_views(Instant::now());
    assert_ne!(toast.countdown, real_time_view[0].countdown());
    assert_ne!(toast.linger, real_time_view[0].linger());

    let ToastVisualDeadline::At(deadline) = app.framework.toasts.next_visual_change_deadline(now)
    else {
        return Err(anyhow::anyhow!(
            "a fading toast must schedule a visual change"
        ));
    };
    let mut frames = FrameState::default();
    frames.record_draw(&app, now);
    let FrameState::Drawn {
        next_visual_change, ..
    } = &frames
    else {
        return Err(anyhow::anyhow!(
            "record_draw must retain the toast deadline"
        ));
    };
    assert_eq!(*next_visual_change, ToastVisualDeadline::At(deadline));
    assert!(!frames.needs_draw(&app, now));
    let just_before = deadline
        .checked_sub(Duration::from_nanos(1))
        .ok_or_else(|| anyhow::anyhow!("the deadline must allow an earlier instant"))?;
    frames.record_draw(&app, just_before);
    assert!(!frames.needs_draw(&app, just_before));
    assert!(frames.needs_draw(&app, deadline));
    assert!(!frames.needs_draw(&app, deadline));
    Ok(())
}

#[test]
fn disk_cache_reuses_unchanged_inputs_and_rebuilds_after_size_change() {
    let path = "/tmp/phase3-cache-project";
    let mut app = test_support::make_app(&[project(path)]);
    app.ensure_visible_rows_cached();
    app.ensure_disk_cache();
    let builds = app.project_list.disk_cache_builds;
    app.ensure_disk_cache();
    assert_eq!(app.project_list.disk_cache_builds, builds);
    app.background
        .background_sender()
        .send(BackgroundMsg::DiskUsage {
            path:  AbsolutePath::from(path),
            bytes: 42,
        })
        .expect("disk update queued");
    app.poll_background();
    app.ensure_disk_cache();
    assert_eq!(app.project_list.cached_root_sorted, vec![42]);
    assert_eq!(app.project_list.disk_cache_builds, builds + 1);
}

#[test]
fn fit_width_cache_reuses_inputs_and_tracks_unselected_disk_and_storage() {
    let path = "/tmp/phase3-width-unselected";
    let mut app = test_support::make_app(&[project("/tmp/phase3-selected"), project(path)]);
    select_fixture_project(&mut app, "/tmp/phase3-selected");
    app.ensure_visible_rows_cached();
    app.ensure_fit_widths_cached();
    let builds = app.project_list.fit_width_builds.get();
    app.ensure_fit_widths_cached();
    assert_eq!(app.project_list.fit_width_builds.get(), builds);

    let detail_generation = app.scan.generation();
    queue(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  AbsolutePath::from(path),
            bytes: 42,
        },
    );
    assert_eq!(app.scan.generation(), detail_generation);
    app.ensure_visible_rows_cached();
    app.ensure_fit_widths_cached();
    assert_eq!(app.project_list.fit_width_builds.get(), builds + 1);
    app.ensure_fit_widths_cached();
    assert_eq!(app.project_list.fit_width_builds.get(), builds + 1);

    let paths = app.project_list.visible_project_storage_paths();
    // Storage is an independent cache input, even without a generation bump.
    app.project_list
        .set_project_storage(&paths, ProjectStorage::Available(12345));
    app.ensure_fit_widths_cached();
    assert_eq!(app.project_list.fit_width_builds.get(), builds + 2);
    app.project_list
        .set_project_storage(&paths, ProjectStorage::Available(12345));
    app.ensure_fit_widths_cached();
    assert_eq!(app.project_list.fit_width_builds.get(), builds + 2);
}

fn queue(app: &mut App, message: BackgroundMsg) {
    app.background
        .background_sender()
        .send(message)
        .expect("background update queued");
    assert_eq!(
        app.poll_background().bg_msgs,
        1,
        "only the queued reply is processed"
    );
}

fn select_fixture_project(app: &mut App, path: &str) {
    let path = std::path::Path::new(path);
    app.project_list.select_project_in_tree(path, true);
    assert_eq!(app.project_list.selected_project_path(), Some(path));
}

#[test]
fn unselected_disk_change_draws_once_and_updates_its_row() {
    let path = "/tmp/phase3-unselected";
    let mut app = test_support::make_app(&[project("/tmp/phase3-selected"), project(path)]);
    select_fixture_project(&mut app, "/tmp/phase3-selected");
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    assert!(!frames.needs_draw(&app, now));
    queue(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  AbsolutePath::from(path),
            bytes: 4096,
        },
    );
    assert!(frames.needs_draw(&app, now));
    app.ensure_visible_rows_cached();
    app.ensure_disk_cache();
    assert_eq!(
        app.project_list
            .visible_row_disk_usage(crate::tui::project_list::VisibleRow::Root { node_index: 1 }),
        Some(4096)
    );
    assert!(!frames.needs_draw(&app, now));
    let row_generation = app.project_list.row_generation;
    let detail_generation = app.scan.generation();
    queue(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  AbsolutePath::from(path),
            bytes: 4096,
        },
    );
    assert_eq!(app.project_list.row_generation, row_generation);
    assert_eq!(app.scan.generation(), detail_generation);
    assert!(!frames.needs_draw(&app, now));
}

#[test]
fn delayed_storage_reply_after_identical_disk_reply_changes_the_row_frame_input() {
    let path = "/tmp/phase3-storage-delayed";
    let mut app = test_support::make_app(&[project("/tmp/phase3-selected"), project(path)]);
    select_fixture_project(&mut app, "/tmp/phase3-selected");
    let disk_reply = || BackgroundMsg::DiskUsage {
        path:  AbsolutePath::from(path),
        bytes: 4096,
    };
    queue(&mut app, disk_reply());
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    let row_generation = app.project_list.row_generation;
    let detail_generation = app.scan.generation();
    let paths = app.project_list.visible_project_storage_paths();
    let sender = app.background.background_sender();
    sender
        .send(disk_reply())
        .expect("queue identical disk reply");
    sender
        .send(BackgroundMsg::ProjectStorage {
            paths,
            storage: ProjectStorage::Available(12345),
        })
        .expect("queue delayed storage reply");
    let stats = app.poll_background();
    assert_eq!(
        stats.bg_msgs, 2,
        "the poll includes DiskUsage and ProjectStorage"
    );
    assert_eq!(stats.disk_usage_msgs, 1);
    assert_eq!(app.project_list.row_generation, row_generation + 1);
    assert_eq!(app.scan.generation(), detail_generation);
    assert!(frames.needs_draw(&app, now));
    assert!(!frames.needs_draw(&app, now));
}

#[test]
fn row_message_burst_builds_row_presentation_at_most_twice() {
    let paths = ["/tmp/phase13b-first", "/tmp/phase13b-second"];
    let mut app = test_support::make_app(&[project(paths[0]), project(paths[1])]);
    let sender = app.background.background_sender();
    for path in paths.iter().cycle().take(20) {
        sender
            .send(BackgroundMsg::NoRepository {
                path: AbsolutePath::from(*path),
            })
            .expect("queue row message");
    }
    ROW_PRESENTATION_BUILDS.with(|builds| builds.set(0));

    let stats = app.poll_background();

    assert_eq!(stats.bg_msgs, 20);
    assert!(
        ROW_PRESENTATION_BUILDS.with(std::cell::Cell::get) <= 2,
        "one frame batch compares the row presentation once"
    );
}

#[test]
fn equal_zero_bytes_still_draws_deletion_once() {
    let path = AbsolutePath::from("/tmp/phase3-nonexistent/deleted");
    let mut item = project(path.as_path().to_str().expect("fixture path"));
    item.at_path_mut(&path).expect("fixture project").disk = DiskMeasurement::TotalOnly(0);
    let mut app = test_support::make_app(&[project("/tmp/phase3-selected"), item]);
    select_fixture_project(&mut app, "/tmp/phase3-selected");
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    queue(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  path.clone(),
            bytes: 0,
        },
    );
    assert!(frames.needs_draw(&app, now));
    assert_eq!(
        app.project_list
            .at_path(&path)
            .expect_found("deleted row")
            .visibility,
        crate::project::Visibility::Deleted
    );
    assert!(!frames.needs_draw(&app, now));
    queue(&mut app, BackgroundMsg::DiskUsage { path, bytes: 0 });
    assert!(!frames.needs_draw(&app, now));
}

#[test]
fn project_discovered_draws_one_new_frame() {
    let mut app = test_support::make_app(&[]);
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    // A non-Rust root starts no `cargo metadata` task, so its reply cannot join this poll.
    let discovered = NonRustProject::new(AbsolutePath::from("/tmp/phase3-discovered"), None);
    queue(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: RootItem::NonRust(discovered),
        },
    );
    assert!(frames.needs_draw(&app, now));
    assert!(matches!(
        app.project_list
            .at_path(std::path::Path::new("/tmp/phase3-discovered")),
        Found(_)
    ));
}

#[test]
fn row_reply_comparison_never_recomputes_fitted_widths() {
    let root = AbsolutePath::from("/tmp/phase3-width-error");
    let mut app = test_support::make_app(&[
        project("/tmp/phase3-selected"),
        project("/tmp/phase3-width-error"),
    ]);
    select_fixture_project(&mut app, "/tmp/phase3-selected");
    app.ensure_visible_rows_cached();
    app.ensure_fit_widths_cached();
    let calls = app.project_list.fit_width_builds.get();
    let generation = app
        .scan
        .metadata_store_handle()
        .lock()
        .expect("metadata store")
        .next_generation(&root);
    let reply = || BackgroundMsg::CargoMetadata {
        workspace_root: root.clone(),
        generation,
        fingerprint: fixture_fingerprint(),
        result: Err(CargoMetadataError::ManifestRejected {
            first_line: "invalid manifest".into(),
        }),
    };
    queue(&mut app, reply());
    assert_eq!(
        app.project_list.fit_width_builds.get(),
        calls,
        "comparison must not measure columns"
    );
    app.ensure_fit_widths_cached();
    assert_eq!(app.project_list.fit_width_builds.get(), calls + 1);
    queue(&mut app, reply());
    assert_eq!(app.project_list.fit_width_builds.get(), calls + 1);
    app.ensure_fit_widths_cached();
    assert_eq!(
        app.project_list.fit_width_builds.get(),
        calls + 1,
        "identical row inputs reuse fitted widths"
    );
}

#[test]
fn collapsed_descendant_disk_text_invalidates_width_inputs() {
    let root = AbsolutePath::from("/tmp/phase3-width-workspace");
    let member = AbsolutePath::from(root.join("member"));
    let mut app = test_support::make_app(&[
        RootItem::Rust(RustProject::Workspace(Workspace {
            path: root,
            ..crate::project::Workspace::default()
        })),
        project(member.as_path().to_str().expect("member path")),
    ]);
    app.project_list.expanded.reset();
    app.ensure_visible_rows_cached();
    assert!(
        !app.visible_rows()
            .iter()
            .any(|row| app.project_list.abs_path_for_row(*row).as_ref() == Some(&member))
    );
    app.ensure_fit_widths_cached();
    let calls = app.project_list.fit_width_builds.get();
    let before = app
        .project_list
        .cached_fit_widths
        .get(crate::tui::columns::COL_DISK);
    queue(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  member.clone(),
            bytes: u64::MAX,
        },
    );
    app.ensure_fit_widths_cached();
    assert_eq!(app.project_list.fit_width_builds.get(), calls + 1);
    assert!(
        app.project_list
            .cached_fit_widths
            .get(crate::tui::columns::COL_DISK)
            > before
    );
    queue(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  member,
            bytes: u64::MAX,
        },
    );
    app.ensure_fit_widths_cached();
    assert_eq!(app.project_list.fit_width_builds.get(), calls + 1);
}

fn fixture_fingerprint() -> ManifestFingerprint {
    ManifestFingerprint {
        manifest:       FileStamp {
            content_hash: [0; 32],
        },
        lockfile:       None,
        rust_toolchain: None,
        configs:        std::collections::BTreeMap::new(),
    }
}

#[test]
fn unselected_manifest_error_and_recovery_each_draw() {
    let root = AbsolutePath::from("/tmp/phase3-nonexistent/manifest");
    let mut app = test_support::make_app(&[
        project("/tmp/phase3-selected"),
        project(root.as_path().to_str().expect("fixture path")),
    ]);
    select_fixture_project(&mut app, "/tmp/phase3-selected");
    let fingerprint = fixture_fingerprint();
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    let generation = app
        .scan
        .metadata_store_handle()
        .lock()
        .expect("metadata store")
        .next_generation(&root);
    queue(
        &mut app,
        BackgroundMsg::CargoMetadata {
            workspace_root: root.clone(),
            generation,
            fingerprint: fingerprint.clone(),
            result: Err(CargoMetadataError::ManifestRejected {
                first_line: "invalid manifest".into(),
            }),
        },
    );
    assert!(frames.needs_draw(&app, now));
    assert!(matches!(
        app.scan.manifest_metadata.for_project(&root),
        crate::tui::app::ProjectMetadataState::Failed { .. }
    ));
    assert!(!frames.needs_draw(&app, now));
    let generation = app
        .scan
        .metadata_store_handle()
        .lock()
        .expect("metadata store")
        .next_generation(&root);
    let metadata = WorkspaceMetadata {
        declared_checkout_root:   root.clone(),
        target_directory:         AbsolutePath::from(root.join("target")),
        packages:                 std::collections::HashMap::new(),
        fingerprint:              fingerprint.clone(),
        out_of_tree_target_bytes: None,
    };
    queue(
        &mut app,
        BackgroundMsg::CargoMetadata {
            workspace_root: root.clone(),
            generation,
            fingerprint,
            result: Ok(metadata),
        },
    );
    assert!(frames.needs_draw(&app, now));
    assert_eq!(
        app.scan.manifest_metadata.for_project(&root),
        &crate::tui::app::ProjectMetadataState::Available
    );
}

#[test]
fn identical_storage_reply_does_not_draw() {
    let mut app = test_support::make_app(&[project("/tmp/phase3-storage")]);
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    let paths = app.project_list.visible_project_storage_paths();
    queue(
        &mut app,
        BackgroundMsg::ProjectStorage {
            paths:   paths.clone(),
            storage: ProjectStorage::Available(12345),
        },
    );
    assert!(frames.needs_draw(&app, now));
    queue(
        &mut app,
        BackgroundMsg::ProjectStorage {
            paths,
            storage: ProjectStorage::Available(12345),
        },
    );
    assert!(!frames.needs_draw(&app, now));
}

#[test]
fn disk_batch_changes_breakdown_once_and_preserves_identical_detail_generation() {
    let path = AbsolutePath::from("/tmp/phase3-batch");
    let mut app =
        test_support::make_app(&[project(path.as_path().to_str().expect("fixture path"))]);
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    let batch = |target| BackgroundMsg::DiskUsageBatch {
        root_path: path.clone(),
        entries:   vec![(
            path.clone(),
            crate::scan::DirSizes {
                total:                 100,
                in_project_target:     target,
                in_project_non_target: 100 - target,
                max_source_mtime:      None,
            },
        )],
    };
    queue(&mut app, batch(40));
    assert!(frames.needs_draw(&app, now));
    let generation = app.scan.generation();
    queue(&mut app, batch(40));
    assert_eq!(app.scan.generation(), generation);
    assert!(!frames.needs_draw(&app, now));
    queue(&mut app, batch(60));
    assert!(frames.needs_draw(&app, now));
    assert!(app.scan.generation() > generation);
    let info = app.project_list.at_path(&path).expect_found("batch row");
    assert_eq!(info.disk.total(), Some(100));
    assert!(matches!(
        info.disk,
        crate::project::DiskMeasurement::Measured {
            total:  100,
            target: 60,
            other:  40,
        }
    ));
}

#[test]
fn unselected_disk_batch_does_not_invalidate_selected_details() {
    let path = AbsolutePath::from("/tmp/phase3-batch-unselected");
    let mut app = test_support::make_app(&[
        project("/tmp/phase3-selected"),
        project(path.as_path().to_str().expect("fixture path")),
    ]);
    select_fixture_project(&mut app, "/tmp/phase3-selected");
    let generation = app.scan.generation();
    queue(
        &mut app,
        BackgroundMsg::DiskUsageBatch {
            root_path: path.clone(),
            entries:   vec![(
                path,
                crate::scan::DirSizes {
                    total:                 100,
                    in_project_target:     40,
                    in_project_non_target: 60,
                    max_source_mtime:      None,
                },
            )],
        },
    );
    assert_eq!(app.scan.generation(), generation);
}

#[test]
fn identical_sccache_replies_preserve_frame_and_width_inputs() {
    let mut app = test_support::make_app(&[project("/tmp/phase3-sccache")]);
    app.ensure_visible_rows_cached();
    app.ensure_fit_widths_cached();
    let builds = app.project_list.fit_width_builds.get();
    let generation = app.project_list.row_generation;
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    let reported = || {
        BackgroundMsg::SccacheSummary(SccacheSummary::Reported {
            cache_size: "2 GiB".into(),
            hit_rate:   "50%".into(),
        })
    };
    queue(&mut app, reported());
    assert!(frames.needs_draw(&app, now));
    queue(&mut app, reported());
    assert!(!frames.needs_draw(&app, now));
    queue(
        &mut app,
        BackgroundMsg::SccacheSummary(SccacheSummary::Unavailable),
    );
    assert!(frames.needs_draw(&app, now));
    queue(
        &mut app,
        BackgroundMsg::SccacheSummary(SccacheSummary::Unavailable),
    );
    assert!(!frames.needs_draw(&app, now));
    app.ensure_fit_widths_cached();
    assert_eq!(app.project_list.row_generation, generation);
    assert_eq!(app.project_list.fit_width_builds.get(), builds);
}

#[test]
fn production_frame_decision_ignores_mouse_in_confirmation_and_draws_dismissal() {
    let mut app = test_support::make_app(&[project("/tmp/phase3-confirmation")]);
    app.ensure_visible_rows_cached();
    app.ensure_disk_cache();
    app.ensure_fit_widths_cached();
    app.ensure_detail_cached();
    app.open_ready_confirmation_for_test(ConfirmAction::Clean(AbsolutePath::from(
        "/tmp/phase3-confirmation",
    )));
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
    let mut output = FrameOutput::default();
    assert!(matches!(
        draw_frame(&mut terminal, &mut app, &mut output).expect("first frame"),
        FrameDraw::Drawn(_)
    ));
    let (tx, rx) = channel::unbounded();
    tx.send(Event::Mouse(MouseEvent {
        kind:      MouseEventKind::Moved,
        column:    1,
        row:       1,
        modifiers: KeyModifiers::NONE,
    }))
    .expect("queue ignored input");
    let input = process_input_frame(&mut app, &rx);
    assert_eq!(input.count, 1);
    assert!(matches!(
        draw_frame(&mut terminal, &mut app, &mut output).expect("ignored input frame decision"),
        FrameDraw::Unchanged(_)
    ));
    assert!(app.confirmation_is_open());
    tx.send(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
        .expect("queue dismissal");
    assert_eq!(process_input_frame(&mut app, &rx).count, 1);
    assert!(matches!(
        draw_frame(&mut terminal, &mut app, &mut output).expect("dismissal frame"),
        FrameDraw::Drawn(_)
    ));
    assert!(!app.confirmation_is_open());
}

#[test]
fn valid_keymap_reload_changes_the_frame_key() {
    let mut app = test_support::make_app(&[]);
    for toast in app.framework.toasts.active_now() {
        app.framework.toasts.dismiss(toast.id());
    }
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    let path = app
        .keymap
        .path()
        .expect("fixture keymap path")
        .to_path_buf();
    std::fs::write(&path, "[output]\ncancel = \"q\"\n").expect("write new keymap");
    app.maybe_reload_keymap_from_disk();
    // Reload diagnostics are separate visible inputs. A clean label update
    // must request a frame even after those notifications are dismissed.
    for toast in app.framework.toasts.active_now() {
        app.framework.toasts.dismiss(toast.id());
    }
    assert!(frames.needs_draw(&app, now));
    assert!(!frames.needs_draw(&app, now));
}

#[test]
fn drawn_frame_renders_ui_once_in_the_real_terminal() {
    let mut app = test_support::make_app(&[project("/tmp/phase3b-render")]);
    app.ensure_visible_rows_cached();
    app.ensure_disk_cache();
    app.ensure_fit_widths_cached();
    app.ensure_detail_cached();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
    let mut output = FrameOutput::default();
    let mut render_counts = Vec::new();
    assert!(matches!(
        draw_frame_with_renderer(&mut terminal, &mut app, &mut output, |frame, app| {
            render_counts.push(frame.count());
            crate::tui::render::ui(frame, app);
        })
        .expect("first frame"),
        FrameDraw::Drawn(_)
    ));
    app.open_ready_confirmation_for_test(ConfirmAction::Clean(AbsolutePath::from(
        "/tmp/phase3b-render",
    )));
    assert!(matches!(
        draw_frame_with_renderer(&mut terminal, &mut app, &mut output, |frame, app| {
            render_counts.push(frame.count());
            crate::tui::render::ui(frame, app);
        })
        .expect("confirmation frame"),
        FrameDraw::Drawn(_)
    ));
    assert_eq!(render_counts, [0, 1]);
}

#[test]
fn changed_frames_reuse_terminal_and_retained_cell_buffers() {
    let mut app = test_support::make_app(&[]);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
    let mut output = FrameOutput::default();
    assert!(matches!(
        draw_frame_with_renderer(&mut terminal, &mut app, &mut output, |frame, _| {
            frame.buffer_mut()[(0, 0)].set_symbol("A");
        })
        .expect("first frame"),
        FrameDraw::Drawn(_)
    ));
    let (changed, changed_allocations) = crate::test_support::count_allocations(|| {
        draw_frame_with_renderer(&mut terminal, &mut app, &mut output, |frame, _| {
            frame.buffer_mut()[(0, 0)].set_symbol("B");
        })
        .expect("changed frame")
    });
    assert!(matches!(changed, FrameDraw::Drawn(_)));
    assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), "B");
    assert_eq!(changed_allocations, 0);
    let (unchanged, unchanged_allocations) = crate::test_support::count_allocations(|| {
        draw_frame_with_renderer(&mut terminal, &mut app, &mut output, |frame, _| {
            frame.buffer_mut()[(0, 0)].set_symbol("B");
        })
        .expect("unchanged frame")
    });
    assert!(matches!(unchanged, FrameDraw::Unchanged(_)));
    assert_eq!(unchanged_allocations, 0);
}

#[test]
fn unchanged_frame_key_skips_ui_render() {
    let mut app = test_support::make_app(&[]);
    let now = Instant::now();
    let mut frames = FrameState::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
    let mut output = FrameOutput::default();
    let mut renders = 0;
    for _ in 0..2 {
        if frames.needs_draw(&app, now) {
            assert!(matches!(
                draw_frame_with_renderer(&mut terminal, &mut app, &mut output, |frame, app| {
                    renders += 1;
                    crate::tui::render::ui(frame, app);
                })
                .expect("allowed frame"),
                FrameDraw::Drawn(_)
            ));
        }
    }
    assert_eq!(renders, 1);
}

struct CursorQueryUnsupported(TestBackend);

impl Backend for CursorQueryUnsupported {
    type Error = Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.0.draw(content).map_err(std::io::Error::other)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.0.hide_cursor().map_err(std::io::Error::other)
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.0.show_cursor().map_err(std::io::Error::other)
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        Err(std::io::Error::new(
            ErrorKind::Unsupported,
            "the terminal does not answer cursor queries",
        ))
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.0
            .set_cursor_position(position)
            .map_err(std::io::Error::other)
    }

    fn clear(&mut self) -> Result<(), Self::Error> { self.0.clear().map_err(std::io::Error::other) }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.0
            .clear_region(clear_type)
            .map_err(std::io::Error::other)
    }

    fn size(&self) -> Result<Size, Self::Error> { self.0.size().map_err(std::io::Error::other) }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.0.window_size().map_err(std::io::Error::other)
    }

    fn flush(&mut self) -> Result<(), Self::Error> { self.0.flush().map_err(std::io::Error::other) }
}

#[test]
fn frames_draw_without_terminal_cursor_replies() -> std::io::Result<()> {
    let mut app = test_support::make_app(&[]);
    let mut terminal = Terminal::new(CursorQueryUnsupported(TestBackend::new(100, 30)))?;
    let mut output = FrameOutput::default();
    for (symbol, changes) in [("A", true), ("A", false), ("B", true)] {
        let result = draw_frame_with_renderer(&mut terminal, &mut app, &mut output, |frame, _| {
            frame.buffer_mut()[(0, 0)].set_symbol(symbol);
        })?;
        assert_eq!(matches!(result, FrameDraw::Drawn(_)), changes);
        assert_eq!(terminal.backend().0.buffer()[(0, 0)].symbol(), symbol);
    }
    Ok(())
}
