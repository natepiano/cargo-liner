use std::time::Duration;
use std::time::Instant;
use std::time::UNIX_EPOCH;

use anyhow::Context;

use super::FrameState;
use crate::tui::app::QuotaFetchAdmission;
use crate::tui::app::QuotaFetchCompletion;
use crate::tui::app::QuotaFetchReason;
use crate::tui::app::QuotaScreenUpdate;
use crate::tui::test_support;

#[test]
fn quota_expiry_requests_one_fetch_without_input_and_then_stays_idle() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.net.set_force_github_rate_limit(true);
    let reset = app
        .net
        .rate_limit()
        .core
        .context("observed quota")?
        .reset_at
        .context("reset time")?;
    let before = UNIX_EPOCH + Duration::from_secs(reset - 1);
    let expiry = UNIX_EPOCH + Duration::from_secs(reset);
    assert_eq!(
        app.github_quota_timeout_at(before),
        Some(Duration::from_secs(1))
    );
    let requests = app
        .startup_services
        .counts()
        .github_rate_limit_prime()
        .suppressed();
    assert_eq!(
        app.poll_github_quota_at(before),
        QuotaScreenUpdate::Unchanged
    );
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    assert!(!frames.needs_draw(&app, now));
    assert_eq!(
        app.poll_github_quota_at(expiry),
        QuotaScreenUpdate::Invalidate
    );
    assert_eq!(
        app.startup_services
            .counts()
            .github_rate_limit_prime()
            .suppressed(),
        requests + 1
    );
    assert!(frames.needs_draw(&app, now));
    assert_eq!(
        app.poll_github_quota_at(expiry),
        QuotaScreenUpdate::Unchanged
    );
    assert!(!frames.needs_draw(&app, now));
    assert_eq!(
        app.startup_services
            .counts()
            .github_rate_limit_prime()
            .suppressed(),
        requests + 1
    );
    Ok(())
}

#[test]
fn expiry_during_recovery_draws_once_and_failed_completion_leaves_the_frame_idle()
-> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.net.set_force_github_rate_limit(true);
    let reset = app
        .net
        .rate_limit()
        .core
        .context("observed quota")?
        .reset_at
        .context("reset time")?;
    let before = UNIX_EPOCH + Duration::from_secs(reset - 1);
    let expiry = UNIX_EPOCH + Duration::from_secs(reset);
    assert_eq!(
        app.poll_github_quota_at(before),
        QuotaScreenUpdate::Unchanged
    );
    let QuotaFetchAdmission::Start(recovery) =
        app.net
            .quota_refresh
            .request(QuotaFetchReason::Recovery, app.net.rate_limit(), before)
    else {
        anyhow::bail!("the recovery request needs a permit");
    };
    let now = Instant::now();
    let mut frames = FrameState::default();
    assert!(frames.needs_draw(&app, now));
    assert!(!frames.needs_draw(&app, now));
    let requests = app
        .startup_services
        .counts()
        .github_rate_limit_prime()
        .suppressed();
    assert_eq!(
        app.poll_github_quota_at(expiry),
        QuotaScreenUpdate::Invalidate
    );
    assert!(frames.needs_draw(&app, now));
    assert_eq!(
        app.poll_github_quota_at(expiry),
        QuotaScreenUpdate::Unchanged
    );
    assert!(!frames.needs_draw(&app, now));
    recovery.complete(QuotaFetchCompletion::Failed);
    assert_eq!(
        app.poll_github_quota_at(expiry),
        QuotaScreenUpdate::Unchanged
    );
    assert!(!frames.needs_draw(&app, now));
    assert_eq!(
        app.poll_github_quota_at(expiry),
        QuotaScreenUpdate::Unchanged
    );
    assert!(!frames.needs_draw(&app, now));
    assert_eq!(
        app.startup_services
            .counts()
            .github_rate_limit_prime()
            .suppressed(),
        requests
    );
    Ok(())
}
