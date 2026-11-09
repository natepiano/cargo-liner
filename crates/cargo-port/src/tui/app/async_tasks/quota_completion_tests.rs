use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use tui_pane::ColorMode;

use super::QuotaFetchAdmission;
use super::QuotaFetchCompletion;
use super::QuotaFetchReason;
use super::QuotaScreenUpdate;
use crate::config::GitHubRateLimitMode;
use crate::http::GitHubRateLimit;
use crate::http::RateLimitQuota;
use crate::tui::panes::PaneId;
use crate::tui::test_support;

fn quota(remaining: u64, reset_at: u64) -> GitHubRateLimit {
    GitHubRateLimit {
        core:    Some(RateLimitQuota {
            used: 5000 - remaining,
            remaining,
            limit: 5000,
            reset_at: Some(reset_at),
        }),
        graphql: None,
    }
}

#[test]
fn a_late_quota_window_retries_and_updates_the_drawn_count() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let now = SystemTime::now();
    let reset = now.duration_since(UNIX_EPOCH)?.as_secs();
    let now = UNIX_EPOCH + Duration::from_secs(reset);
    let old = quota(4714, reset);
    app.net.http_client.store_rate_limit_for_test(old);
    let QuotaFetchAdmission::Start(first) =
        app.net
            .quota_refresh
            .request(QuotaFetchReason::Reset, old, now)
    else {
        anyhow::bail!("the reset needs its first fetch");
    };
    first.complete(QuotaFetchCompletion::Received(old));
    app.poll_github_quota_at(now);
    assert_eq!(
        app.github_quota_timeout_at(now),
        Some(Duration::from_secs(10))
    );
    assert!(matches!(
        app.net
            .quota_refresh
            .request(QuotaFetchReason::Reset, old, now + Duration::from_secs(9)),
        QuotaFetchAdmission::ResetAlreadyRequested
    ));
    let retry_time = now + Duration::from_secs(10);
    let QuotaFetchAdmission::Start(retry) =
        app.net
            .quota_refresh
            .request(QuotaFetchReason::Reset, old, retry_time)
    else {
        anyhow::bail!("the old window needs one retry after ten seconds");
    };
    let new = quota(4988, reset + 3600);
    app.net.http_client.store_rate_limit_for_test(new);
    retry.complete(QuotaFetchCompletion::Received(new));
    assert_eq!(
        app.poll_github_quota_at(retry_time),
        QuotaScreenUpdate::Invalidate
    );
    app.ensure_detail_cached();
    let rows = test_support::rows(&test_support::draw_pane(&mut app, PaneId::Git, 80)?);
    let core = rows
        .iter()
        .find(|row| row.contains("Core"))
        .context("new Core row")?;
    assert!(core.contains("4988/5000 resets"), "{core}");
    assert!(!core.contains("4714/5000"), "{core}");
    assert!(matches!(
        app.net
            .quota_refresh
            .request(QuotaFetchReason::Reset, new, retry_time),
        QuotaFetchAdmission::ResetAlreadyRequested
    ));
    assert_eq!(
        app.github_quota_timeout_at(now),
        Some(Duration::from_secs(3600))
    );
    Ok(())
}

#[test]
fn stale_quota_windows_stop_after_three_timed_retries() -> anyhow::Result<()> {
    for completion in [
        QuotaFetchCompletion::Received(quota(4714, 200)),
        QuotaFetchCompletion::Failed,
    ] {
        let app = test_support::project_app();
        let old = quota(4714, 200);
        app.net.http_client.store_rate_limit_for_test(old);
        let reset = UNIX_EPOCH + Duration::from_secs(200);
        let mut fetches = 0;
        for seconds in [0, 10, 20, 30] {
            let now = reset + Duration::from_secs(seconds);
            let QuotaFetchAdmission::Start(permit) =
                app.net
                    .quota_refresh
                    .request(QuotaFetchReason::Reset, old, now)
            else {
                anyhow::bail!("a quota fetch must start at reset + {seconds} seconds");
            };
            fetches += 1;
            assert!(matches!(
                app.net
                    .quota_refresh
                    .request(QuotaFetchReason::Recovery, old, now),
                QuotaFetchAdmission::InFlight
            ));
            permit.complete(completion);
            assert!(matches!(
                app.net
                    .quota_refresh
                    .request(QuotaFetchReason::Reset, old, now),
                QuotaFetchAdmission::ResetAlreadyRequested
            ));
            if seconds < 30 {
                assert_eq!(
                    app.github_quota_timeout_at(now),
                    Some(Duration::from_secs(10))
                );
                assert!(matches!(
                    app.net.quota_refresh.request(
                        QuotaFetchReason::Reset,
                        old,
                        now + Duration::from_secs(9)
                    ),
                    QuotaFetchAdmission::ResetAlreadyRequested
                ));
            }
        }
        assert_eq!(fetches, 4);
        assert_eq!(
            app.github_quota_timeout_at(reset + Duration::from_secs(30)),
            None
        );
        assert!(matches!(
            app.net.quota_refresh.request(
                QuotaFetchReason::Reset,
                old,
                reset + Duration::from_secs(40)
            ),
            QuotaFetchAdmission::ResetAlreadyRequested
        ));
    }
    Ok(())
}

#[test]
fn completed_quota_fetch_replaces_the_drawn_counts_without_input() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let now = UNIX_EPOCH + Duration::from_secs(100);
    let observed = app.net.rate_limit();
    let QuotaFetchAdmission::Start(permit) =
        app.net
            .quota_refresh
            .request(QuotaFetchReason::Recovery, observed, now)
    else {
        anyhow::bail!("the first quota request needs a permit");
    };
    app.net
        .http_client
        .store_rate_limit_for_test(quota(4739, 200));
    permit.complete(QuotaFetchCompletion::Received(quota(4739, 200)));
    assert_eq!(app.poll_github_quota_at(now), QuotaScreenUpdate::Invalidate);
    app.ensure_detail_cached();
    let first = test_support::rows(&test_support::draw_pane(&mut app, PaneId::Git, 80)?);
    assert!(
        first.iter().any(|row| row.contains("4739/5000")),
        "{first:?}"
    );
    let QuotaFetchAdmission::Start(permit) = app.net.quota_refresh.request(
        QuotaFetchReason::Reset,
        app.net.rate_limit(),
        UNIX_EPOCH + Duration::from_secs(200),
    ) else {
        anyhow::bail!("expiry needs one quota request");
    };
    app.net
        .http_client
        .store_rate_limit_for_test(quota(4999, 400));
    permit.complete(QuotaFetchCompletion::Received(quota(4999, 400)));
    assert_eq!(
        app.poll_github_quota_at(UNIX_EPOCH + Duration::from_secs(201)),
        QuotaScreenUpdate::Invalidate
    );
    app.ensure_detail_cached();
    let refreshed = test_support::rows(&test_support::draw_pane(&mut app, PaneId::Git, 80)?);
    assert!(
        refreshed.iter().any(|row| row.contains("4999/5000")),
        "{refreshed:?}"
    );
    assert!(
        !refreshed.iter().any(|row| row.contains("4739/5000")),
        "{refreshed:?}"
    );
    assert_eq!(
        app.poll_github_quota_at(UNIX_EPOCH + Duration::from_secs(201)),
        QuotaScreenUpdate::Unchanged
    );
    Ok(())
}

#[test]
fn failed_and_stale_quota_replies_schedule_a_retry_without_an_immediate_fetch() -> anyhow::Result<()>
{
    for completion in [
        QuotaFetchCompletion::Failed,
        QuotaFetchCompletion::Received(quota(4629, 200)),
    ] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        let seed_time = UNIX_EPOCH + Duration::from_secs(100);
        let QuotaFetchAdmission::Start(seed) = app.net.quota_refresh.request(
            QuotaFetchReason::Recovery,
            app.net.rate_limit(),
            seed_time,
        ) else {
            anyhow::bail!("the initial quota request needs a permit");
        };
        app.net
            .http_client
            .store_rate_limit_for_test(quota(4629, 200));
        seed.complete(QuotaFetchCompletion::Received(quota(4629, 200)));
        assert_eq!(
            app.poll_github_quota_at(seed_time),
            QuotaScreenUpdate::Invalidate
        );
        app.ensure_detail_cached();
        let now = UNIX_EPOCH + Duration::from_secs(201);
        let QuotaFetchAdmission::Start(permit) =
            app.net
                .quota_refresh
                .request(QuotaFetchReason::Reset, app.net.rate_limit(), now)
        else {
            anyhow::bail!("the expired quota needs one request");
        };
        assert!(matches!(
            app.net
                .quota_refresh
                .request(QuotaFetchReason::Recovery, app.net.rate_limit(), now),
            QuotaFetchAdmission::InFlight
        ));
        permit.complete(completion);
        app.poll_github_quota_at(now);
        app.ensure_detail_cached();
        let rows = test_support::rows(&test_support::draw_pane(&mut app, PaneId::Git, 80)?);
        let core = rows
            .iter()
            .find(|row| row.contains("Core"))
            .context("retained quota row")?;
        assert!(core.contains("4629/5000"), "{core}");
        assert!(!core.contains("resets"), "{core}");
        assert_eq!(
            app.github_quota_timeout_at(now),
            Some(Duration::from_secs(10))
        );
        assert_eq!(app.poll_github_quota_at(now), QuotaScreenUpdate::Unchanged);
        assert!(matches!(
            app.net
                .quota_refresh
                .request(QuotaFetchReason::Reset, app.net.rate_limit(), now),
            QuotaFetchAdmission::ResetAlreadyRequested
        ));
    }
    Ok(())
}

#[test]
fn identical_completion_after_expiry_keeps_the_generation() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let now = UNIX_EPOCH + Duration::from_secs(100);
    let previous = quota(4720, 101);
    app.net.http_client.store_rate_limit_for_test(previous);
    let QuotaFetchAdmission::Start(seed) =
        app.net
            .quota_refresh
            .request(QuotaFetchReason::Recovery, app.net.rate_limit(), now)
    else {
        anyhow::bail!("initial request needs a permit");
    };
    seed.complete(QuotaFetchCompletion::Received(previous));
    app.poll_github_quota_at(now);
    let expiry = UNIX_EPOCH + Duration::from_secs(101);
    let QuotaFetchAdmission::Start(permit) =
        app.net
            .quota_refresh
            .request(QuotaFetchReason::Recovery, app.net.rate_limit(), now)
    else {
        anyhow::bail!("recovery request needs a permit");
    };
    assert_eq!(
        app.poll_github_quota_at(expiry),
        QuotaScreenUpdate::Invalidate
    );
    let generation = app.scan.generation();
    permit.complete(QuotaFetchCompletion::Received(previous));
    assert_eq!(
        app.poll_github_quota_at(expiry),
        QuotaScreenUpdate::Unchanged
    );
    assert_eq!(app.scan.generation(), generation);
    Ok(())
}

#[test]
fn new_window_survives_a_late_completion_and_reaches_the_git_pane() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let now = SystemTime::now();
    let seconds = now.duration_since(UNIX_EPOCH)?.as_secs();
    let previous = quota(4720, seconds - 1);
    let refreshed = quota(4999, seconds + 3600);
    app.net.http_client.store_rate_limit_for_test(previous);
    let QuotaFetchAdmission::Start(permit) = app.net.quota_refresh.request(
        QuotaFetchReason::Recovery,
        app.net.rate_limit(),
        now - Duration::from_secs(2),
    ) else {
        anyhow::bail!("recovery request needs a permit");
    };
    assert_eq!(app.poll_github_quota_at(now), QuotaScreenUpdate::Invalidate);
    app.ensure_detail_cached();
    app.net.http_client.store_rate_limit_for_test(refreshed);
    assert_eq!(app.net.rate_limit(), refreshed);
    let generation = app.scan.generation();
    assert_eq!(app.poll_github_quota_at(now), QuotaScreenUpdate::Unchanged);
    assert_eq!(app.scan.generation(), generation);
    permit.complete(QuotaFetchCompletion::Received(previous));
    assert_eq!(app.poll_github_quota_at(now), QuotaScreenUpdate::Invalidate);
    app.ensure_detail_cached();
    let rows = test_support::rows(&test_support::draw_pane(&mut app, PaneId::Git, 80)?);
    let core = rows
        .iter()
        .find(|row| row.contains("Core"))
        .context("Core row")?;
    assert!(core.contains("4999/5000 resets"), "{rows:?}");
    assert!(!core.contains("4720/5000"), "{core}");
    assert_eq!(app.net.rate_limit(), refreshed);
    assert_eq!(app.poll_github_quota_at(now), QuotaScreenUpdate::Unchanged);
    Ok(())
}

#[test]
fn completed_fetch_keeps_the_forced_core_quota() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.net
        .set_force_github_rate_limit(GitHubRateLimitMode::Forced);
    let before = app.net.rate_limit();
    let now = SystemTime::now();
    let QuotaFetchAdmission::Start(permit) =
        app.net
            .quota_refresh
            .request(QuotaFetchReason::Recovery, before, now)
    else {
        anyhow::bail!("recovery request needs a permit");
    };
    let response = quota(4999, u64::MAX);
    app.net.http_client.store_rate_limit_for_test(response);
    permit.complete(QuotaFetchCompletion::Received(response));
    app.poll_github_quota_at(now);
    app.ensure_detail_cached();
    let rows = test_support::rows(&test_support::draw_pane(&mut app, PaneId::Git, 80)?);
    let core = rows
        .iter()
        .find(|row| row.contains("Core"))
        .context("Core row")?;
    assert!(core.contains("0/5000"), "{core}");
    assert!(!core.contains("4999/5000"), "{core}");
    assert_eq!(app.net.rate_limit().core, before.core);
    Ok(())
}
