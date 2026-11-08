use anyhow::Context;
use cargo_metadata::PackageId;
use cargo_metadata::semver::Version;
use ratatui::buffer::Buffer;
use tui_pane::ColorMode;

use super::App;
use crate::channel;
use crate::config::NonRustInclusion;
use crate::http::ServiceKind;
use crate::project::AbsolutePath;
use crate::project::FileStamp;
use crate::project::FirstCommit;
use crate::project::ManifestFingerprint;
use crate::project::NonRustProject;
use crate::project::PackageRecord;
use crate::project::PublishPolicy;
use crate::project::RemoteInfo;
use crate::project::RemoteKind;
use crate::project::RemoteList;
use crate::project::RepoInfo;
use crate::project::ResolvedBranch;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::TrackedRef;
use crate::project::WorkflowPresence;
use crate::project::Workspace;
use crate::project::WorkspaceMetadata;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::tui::panes::PaneId;
use crate::tui::test_support;

fn application() -> (App, AbsolutePath) {
    let path = AbsolutePath::from("/tmp/test/loading-layout");
    let project = test_support::make_package("loading-layout", path.as_path());
    let mut app = test_support::make_app_with_color_mode(&[project], ColorMode::Colored);
    app.ensure_visible_rows_cached();
    app.ensure_detail_cached();
    (app, path)
}

fn workspace_application() -> (App, AbsolutePath) {
    let path = AbsolutePath::from("/tmp/test/loading-workspace");
    let project = RootItem::Rust(RustProject::Workspace(Workspace {
        path: path.clone(),
        name: Some("loading-workspace".to_string()),
        ..crate::project::Workspace::default()
    }));
    let mut app = test_support::make_app_with_color_mode(&[project], ColorMode::Colored);
    app.ensure_visible_rows_cached();
    app.ensure_detail_cached();
    (app, path)
}

fn cargo_description(
    app: &mut App,
    path: &AbsolutePath,
    description: Option<&str>,
) -> anyhow::Result<()> {
    let package = PackageRecord {
        name:          "loading-layout".to_string(),
        version:       Version::new(0, 1, 0),
        edition:       "2024".to_string(),
        description:   description.map(str::to_string),
        license:       None,
        homepage:      None,
        repository:    None,
        manifest_path: AbsolutePath::from(path.as_path().join("Cargo.toml")),
        targets:       Vec::new(),
        publish:       PublishPolicy::Never,
    };
    app.scan
        .metadata_store_handle()
        .lock()
        .map_err(|error| anyhow::anyhow!("metadata lock: {error}"))?
        .upsert(WorkspaceMetadata {
            declared_checkout_root:   path.clone(),
            target_directory:         AbsolutePath::from(path.as_path().join("target")),
            packages:                 std::iter::once((
                PackageId {
                    repr: "loading-layout".to_string(),
                },
                package,
            ))
            .collect(),
            fingerprint:              ManifestFingerprint {
                manifest:       FileStamp {
                    content_hash: [0; 32],
                },
                lockfile:       None,
                rust_toolchain: None,
                configs:        std::collections::BTreeMap::new(),
            },
            out_of_tree_target_bytes: None,
        });
    app.scan.manifest_metadata.record_success(path);
    app.scan.bump_generation();
    app.ensure_detail_cached();
    Ok(())
}

fn github_remote(app: &mut App, path: &AbsolutePath) {
    app.handle_bg_msg(BackgroundMsg::RepoInfo {
        path: path.clone(),
        info: RepoInfo {
            remotes:           RemoteList::Listed(vec![RemoteInfo {
                name:         "origin".to_string(),
                url:          Some("https://github.com/acme/loading-layout".to_string()),
                owner:        None,
                repo:         None,
                tracked_ref:  TrackedRef::Untracked,
                ahead_behind: crate::project::Comparison::NotCompared,
                kind:         RemoteKind::Clone,
                push:         crate::project::PushState::Enabled {
                    push_url: String::new(),
                },
            }]),
            workflows:         WorkflowPresence::Missing,
            first_commit:      FirstCommit::AwaitingProbe,
            last_fetched:      None,
            default_branch:    ResolvedBranch::NotFound,
            local_main_branch: ResolvedBranch::NotFound,
        },
    });
    app.ensure_detail_cached();
}

fn no_github_remote(app: &mut App, path: &AbsolutePath) {
    app.handle_bg_msg(BackgroundMsg::RepoInfo {
        path: path.clone(),
        info: RepoInfo::default(),
    });
    app.ensure_detail_cached();
}

fn description_frame(app: &mut App, window: u16, width: u16) -> anyhow::Result<Buffer> {
    test_support::draw_pane_in_window_at_size(app, PaneId::Package, window, width, 14)
}

fn assert_no_description(buffer: &Buffer) {
    let rows = test_support::rows(buffer);
    assert!(
        !rows.iter().any(|row| row.contains("description")),
        "{rows:?}"
    );
}

fn assert_empty_description(buffer: &Buffer, window: u16) -> anyhow::Result<()> {
    test_support::position(
        buffer,
        if window == 80 {
            "description"
        } else {
            "No description"
        },
    )?;
    Ok(())
}

#[test]
fn completed_no_repository_probe_resolves_empty_descriptions() -> anyhow::Result<()> {
    for (window, width) in [(80, 17), (160, 80)] {
        for rust_project in [false, true] {
            let directory = tempfile::tempdir()?;
            let path = AbsolutePath::from(directory.path());
            let project = if rust_project {
                test_support::make_package("outside-git", path.as_path())
            } else {
                RootItem::NonRust(NonRustProject::new(
                    path.clone(),
                    Some("outside-git".to_string()),
                ))
            };
            let mut config = crate::config::CargoPortConfig::default();
            config.tui.include_non_rust = NonRustInclusion::Include;
            let mut app = test_support::make_app_with_config(&[project], &config);
            app.ensure_visible_rows_cached();
            app.ensure_detail_cached();
            if rust_project {
                cargo_description(&mut app, &path, None)?;
            }
            assert_no_description(&description_frame(&mut app, window, width)?);

            let (sender, receiver) = channel::unbounded();
            scan::emit_git_info(&sender, &path);
            let result = receiver.try_recv().context("Completed Git probe")?;
            assert!(
                matches!(&result, BackgroundMsg::NoRepository { path: result_path } if result_path == &path)
            );
            app.handle_bg_msg(result);
            app.ensure_detail_cached();
            assert_empty_description(&description_frame(&mut app, window, width)?, window)?;
            assert!(matches!(
                app.project_list
                    .git_repo_for(path.as_path())
                    .context("Repository probe")?
                    .repo_info,
                crate::project::RepositoryProbe::NoRepository
            ));
            assert!(receiver.try_recv().is_err());
        }
    }
    Ok(())
}

#[test]
fn completed_no_repository_probe_keeps_cargo_description_pending() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = AbsolutePath::from(directory.path());
    let project = test_support::make_package("pending-cargo", path.as_path());
    let mut app = test_support::make_app_with_color_mode(&[project], ColorMode::Colored);
    app.ensure_visible_rows_cached();
    app.ensure_detail_cached();
    let (sender, receiver) = channel::unbounded();
    scan::emit_git_info(&sender, &path);
    app.handle_bg_msg(receiver.try_recv().context("Completed Git probe")?);
    app.ensure_detail_cached();
    assert_no_description(&description_frame(&mut app, 160, 80)?);
    cargo_description(&mut app, &path, None)?;
    assert_empty_description(&description_frame(&mut app, 160, 80)?, 160)?;
    Ok(())
}

#[test]
fn cargo_description_before_remotes_never_draws_a_false_empty_row() -> anyhow::Result<()> {
    for (window, width) in [(80, 17), (160, 80)] {
        let (mut app, path) = application();
        cargo_description(&mut app, &path, None)?;
        let before = description_frame(&mut app, window, width)?;
        assert_no_description(&before);
        let path_position = test_support::position(&before, "Path")?;
        github_remote(&mut app, &path);
        let after = description_frame(&mut app, window, width)?;
        assert_no_description(&after);
        assert_eq!(path_position, test_support::position(&after, "Path")?);
        assert_eq!(
            test_support::rows(&before).last(),
            test_support::rows(&after).last()
        );
        app.handle_bg_msg(BackgroundMsg::RepoMeta {
            path,
            stars: 0,
            description: None,
        });
        app.ensure_detail_cached();
        assert_empty_description(&description_frame(&mut app, window, width)?, window)?;
    }
    Ok(())
}

#[test]
fn remotes_before_cargo_description_never_draw_a_false_empty_row() -> anyhow::Result<()> {
    for (window, width) in [(80, 17), (160, 80)] {
        let (mut app, path) = application();
        let before = description_frame(&mut app, window, width)?;
        assert_no_description(&before);
        no_github_remote(&mut app, &path);
        let after = description_frame(&mut app, window, width)?;
        assert_no_description(&after);
        assert_eq!(
            test_support::position(&before, "Path")?,
            test_support::position(&after, "Path")?
        );
        assert_eq!(
            test_support::rows(&before).last(),
            test_support::rows(&after).last()
        );
        cargo_description(&mut app, &path, None)?;
        assert_empty_description(&description_frame(&mut app, window, width)?, window)?;
    }
    Ok(())
}

#[test]
fn empty_github_description_does_not_resolve_pending_cargo_text() -> anyhow::Result<()> {
    for (window, width) in [(80, 17), (160, 80)] {
        let (mut app, path) = application();
        github_remote(&mut app, &path);
        let before = description_frame(&mut app, window, width)?;
        assert_no_description(&before);
        app.handle_bg_msg(BackgroundMsg::RepoMeta {
            path:        path.clone(),
            stars:       0,
            description: None,
        });
        app.ensure_detail_cached();
        let after = description_frame(&mut app, window, width)?;
        assert_no_description(&after);
        assert_eq!(
            test_support::position(&before, "Path")?,
            test_support::position(&after, "Path")?
        );
        assert_eq!(
            test_support::rows(&before).last(),
            test_support::rows(&after).last()
        );
        cargo_description(&mut app, &path, Some("Cargo words"))?;
        let known = description_frame(&mut app, window, width)?;
        assert_no_description(&known);
        test_support::position(&known, "Cargo words")?;
    }
    Ok(())
}

fn non_rust_application() -> (App, AbsolutePath) {
    let path = AbsolutePath::from("/tmp/test/non-rust-project");
    let project = RootItem::NonRust(NonRustProject::new(
        path.clone(),
        Some("non-rust".to_string()),
    ));
    let mut config = crate::config::CargoPortConfig::default();
    config.tui.include_non_rust = NonRustInclusion::Include;
    let mut app = test_support::make_app_with_config(&[project], &config);
    app.ensure_visible_rows_cached();
    app.ensure_detail_cached();
    (app, path)
}

#[test]
fn non_rust_description_resolves_when_no_github_remote_is_known() -> anyhow::Result<()> {
    for (window, width) in [(80, 17), (160, 80)] {
        let (mut app, path) = non_rust_application();
        assert_no_description(&description_frame(&mut app, window, width)?);
        no_github_remote(&mut app, &path);
        assert_empty_description(&description_frame(&mut app, window, width)?, window)?;
    }
    Ok(())
}

#[test]
fn non_rust_description_waits_for_the_github_reply() -> anyhow::Result<()> {
    for (window, width) in [(80, 17), (160, 80)] {
        for description in [None, Some("Repo words")] {
            let (mut app, path) = non_rust_application();
            let before = description_frame(&mut app, window, width)?;
            assert_no_description(&before);
            github_remote(&mut app, &path);
            let after = description_frame(&mut app, window, width)?;
            assert_no_description(&after);
            assert_eq!(
                test_support::position(&before, "Path")?,
                test_support::position(&after, "Path")?
            );
            assert_eq!(
                test_support::rows(&before).last(),
                test_support::rows(&after).last()
            );
            app.handle_bg_msg(BackgroundMsg::RepoMeta {
                path,
                stars: 0,
                description: description.map(str::to_string),
            });
            app.ensure_detail_cached();
            let known = description_frame(&mut app, window, width)?;
            if description.is_some() {
                assert_no_description(&known);
                let git = test_support::draw_pane_in_window(&mut app, PaneId::Git, window, width)?;
                test_support::position(&git, "Repo words")?;
            } else {
                assert_empty_description(&known, window)?;
            }
        }
    }
    Ok(())
}

#[test]
fn pending_and_failed_github_descriptions_keep_package_rows_and_pages() -> anyhow::Result<()> {
    for (window, width) in [(80, 17), (160, 80)] {
        let (mut app, path) = workspace_application();
        cargo_description(&mut app, &path, None)?;
        github_remote(&mut app, &path);
        let pending = test_support::draw_pane_in_window_at_size(
            &mut app,
            PaneId::Package,
            window,
            width,
            14,
        )?;
        let pending_rows = test_support::rows(&pending);
        let path_position = test_support::position(&pending, "Path")?;
        assert!(
            !pending_rows.iter().any(|row| row.contains("description")),
            "{pending_rows:?}"
        );
        app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
            service: ServiceKind::GitHub,
        });
        app.ensure_detail_cached();
        let failed = test_support::draw_pane_in_window_at_size(
            &mut app,
            PaneId::Package,
            window,
            width,
            14,
        )?;
        let failed_rows = test_support::rows(&failed);
        assert!(
            !failed_rows.iter().any(|row| row.contains("description")),
            "{failed_rows:?}"
        );
        assert_eq!(path_position, test_support::position(&failed, "Path")?);
        assert_eq!(pending_rows.last(), failed_rows.last());
        app.handle_bg_msg(BackgroundMsg::RepoMeta {
            path:        path.clone(),
            stars:       0,
            description: Some("Repo words".to_string()),
        });
        app.ensure_detail_cached();
        let repository = test_support::draw_pane_in_window(&mut app, PaneId::Git, window, width)?;
        test_support::position(&repository, "Repo words")?;
        let loaded = test_support::draw_pane_in_window_at_size(
            &mut app,
            PaneId::Package,
            window,
            width,
            14,
        )?;
        assert_eq!(path_position, test_support::position(&loaded, "Path")?);
        assert_eq!(pending_rows.last(), test_support::rows(&loaded).last());
        app.handle_bg_msg(BackgroundMsg::RepoMeta {
            path,
            stars: 0,
            description: None,
        });
        app.ensure_detail_cached();
        let known = test_support::draw_pane_in_window(&mut app, PaneId::Package, window, width)?;
        test_support::position(
            &known,
            if window == 80 {
                "description"
            } else {
                "No description"
            },
        )?;
    }
    Ok(())
}

#[test]
fn cargo_description_stays_unknown_until_metadata_arrives_without_a_github_remote()
-> anyhow::Result<()> {
    for (window, width) in [(80, 17), (160, 80)] {
        let (mut app, path) = application();
        no_github_remote(&mut app, &path);
        let pending = test_support::draw_pane_in_window(&mut app, PaneId::Package, window, width)?;
        let rows = test_support::rows(&pending);
        assert!(
            !rows.iter().any(|row| row.contains("description")),
            "{rows:?}"
        );
        cargo_description(&mut app, &path, Some("Cargo words"))?;
        let known = test_support::draw_pane_in_window(&mut app, PaneId::Package, window, width)?;
        test_support::position(&known, "Cargo words")?;
        cargo_description(&mut app, &path, None)?;
        let empty = test_support::draw_pane_in_window(&mut app, PaneId::Package, window, width)?;
        test_support::position(
            &empty,
            if window == 80 {
                "description"
            } else {
                "No description"
            },
        )
        .with_context(|| {
            format!(
                "known empty Cargo description: {:?}",
                test_support::rows(&empty)
            )
        })?;
    }
    Ok(())
}
