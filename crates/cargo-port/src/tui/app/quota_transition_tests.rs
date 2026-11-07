use anyhow::Context;
use ratatui::buffer::Buffer;
use tempfile::TempDir;
use tui_pane::ColorMode;

use super::App;
use crate::http::ServiceKind;
use crate::project::AbsolutePath;
use crate::project::FirstCommit;
use crate::project::RemoteInfo;
use crate::project::RemoteKind;
use crate::project::RemoteList;
use crate::project::RepoInfo;
use crate::project::ResolvedBranch;
use crate::project::TrackedRef;
use crate::project::WorkflowPresence;
use crate::scan::BackgroundMsg;
use crate::tui::panes::GithubStars;
use crate::tui::panes::PaneId;
use crate::tui::test_support;

fn application() -> anyhow::Result<(TempDir, App, AbsolutePath)> {
    let directory = TempDir::new()?;
    let path = AbsolutePath::from(directory.path());
    let project = test_support::make_package("quota-project", path.as_path());
    let mut app = test_support::make_app_with_color_mode(&[project], ColorMode::Colored);
    app.ensure_visible_rows_cached();
    app.ensure_detail_cached();
    Ok((directory, app, path))
}

fn deliver(app: &mut App, message: BackgroundMsg) -> anyhow::Result<()> {
    let sender = app.background.background_sender();
    sender
        .send(message)
        .map_err(|error| anyhow::anyhow!("application message failed: {error}"))?;
    app.poll_background();
    app.ensure_detail_cached();
    Ok(())
}

fn repository(app: &mut App, path: &AbsolutePath, url: &str) -> anyhow::Result<()> {
    deliver(
        app,
        BackgroundMsg::RepoInfo {
            path: path.clone(),
            info: RepoInfo {
                remotes:           RemoteList::Listed(vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some(url.to_string()),
                    owner:        None,
                    repo:         None,
                    tracked_ref:  TrackedRef::Untracked,
                    ahead_behind: crate::project::Comparison::NotCompared,
                    kind:         RemoteKind::Clone,
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
                }]),
                workflows:         WorkflowPresence::Present,
                first_commit:      FirstCommit::AwaitingProbe,
                last_fetched:      None,
                default_branch:    ResolvedBranch::NotFound,
                local_main_branch: ResolvedBranch::NotFound,
            },
        },
    )
}

fn draw(app: &mut App) -> anyhow::Result<Buffer> { test_support::draw_pane(app, PaneId::Git, 80) }

#[test]
fn a_non_github_project_has_no_stars_row_during_an_outage() -> anyhow::Result<()> {
    let (directory, mut app, path) = application()?;
    assert_eq!(
        app.panes.git.content().context("Git data")?.stars,
        GithubStars::NotGithub
    );
    for url in [None, Some("https://gitlab.com/acme/quota-project")] {
        if let Some(url) = url {
            repository(&mut app, &path, url)?;
        }
        deliver(
            &mut app,
            BackgroundMsg::ServiceUnreachable {
                service: ServiceKind::GitHub,
            },
        )?;
        let rows = test_support::rows(&draw(&mut app)?);
        assert!(!rows.iter().any(|row| row.contains("Stars")), "{rows:?}");
    }
    drop(directory);
    Ok(())
}

#[test]
fn github_stars_recover_from_one_shared_outage_message() -> anyhow::Result<()> {
    let (directory, mut app, path) = application()?;
    repository(&mut app, &path, "https://github.com/acme/quota-project")?;
    assert_eq!(
        app.panes.git.content().context("Git data")?.stars,
        GithubStars::Pending
    );
    deliver(
        &mut app,
        BackgroundMsg::ServiceUnreachable {
            service: ServiceKind::GitHub,
        },
    )?;
    assert_eq!(
        app.panes.git.content().context("Git data")?.stars,
        GithubStars::Unreachable
    );
    let buffer = draw(&mut app)?;
    let rows = test_support::rows(&buffer);
    let text = rows.join("\n");
    assert_eq!(text.matches("GitHub unreachable").count(), 1, "{text}");
    assert_eq!(
        text.to_lowercase().matches("github unreachable").count(),
        1,
        "{text}"
    );
    let (x, y) = test_support::position(&buffer, "GitHub unreachable")?;
    let colour = buffer[(x, y)].fg;
    for offset in 0..u16::try_from("GitHub unreachable".len())? {
        assert_eq!(buffer[(x + offset, y)].fg, colour);
    }
    deliver(
        &mut app,
        BackgroundMsg::ServiceRecovered {
            service: ServiceKind::GitHub,
        },
    )?;
    assert_eq!(
        app.panes.git.content().context("Git data")?.stars,
        GithubStars::Pending
    );
    deliver(
        &mut app,
        BackgroundMsg::RepoMeta {
            path:        path.clone(),
            stars:       0,
            description: None,
        },
    )?;
    for (message, expected) in [
        (
            BackgroundMsg::ServiceUnreachable {
                service: ServiceKind::GitHub,
            },
            "0",
        ),
        (
            BackgroundMsg::ServiceRecovered {
                service: ServiceKind::GitHub,
            },
            "0",
        ),
        (
            BackgroundMsg::RepoMeta {
                path,
                stars: 42,
                description: None,
            },
            "42",
        ),
    ] {
        deliver(&mut app, message)?;
        let rows = test_support::rows(&draw(&mut app)?);
        let stars = rows
            .iter()
            .find(|row| row.contains("Stars"))
            .context("measured Stars row")?;
        let mut value = stars
            .split_once("Stars")
            .context("Stars label")?
            .1
            .split_whitespace();
        assert_eq!(value.next(), Some("⭐"), "{stars}");
        assert_eq!(value.next(), Some(expected), "{stars}");
    }
    let rows = test_support::rows(&draw(&mut app)?);
    let stars = rows
        .iter()
        .find(|row| row.contains("Stars"))
        .context("recovered Stars row")?;
    assert!(stars.contains("42"), "{stars}");
    drop(directory);
    Ok(())
}
