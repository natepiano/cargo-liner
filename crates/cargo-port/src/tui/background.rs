//! The `Background` subsystem.
//!
//! Owns the four channel pairs plus the watcher handle:
//! - background `sender` / `receiver` (replaced wholesale on every rescan — see
//!   [`Background::swap_background_channel`])
//! - `ci_fetch_tx` / `ci_fetch_rx`
//! - `clean_tx` / `clean_rx`
//! - `example_tx` / `example_rx`
//! - `watcher`
//!
//! Spawn / poll facade methods live on `App` (and inside
//! [`Inflight`]) because they thread cross-subsystem dependencies
//! (`Scan`, `Net`, framework toasts).
//!
//! [`Inflight`]: super::state::Inflight

use tui_pane::PERF_LOG_TARGET;

use super::startup_services::WatcherHandle;
use super::terminal::CiFetchMsg;
use super::terminal::CleanMsg;
use super::terminal::ExampleMsg;
use crate::channel::Receiver;
use crate::channel::SendError;
use crate::channel::Sender;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::scan::BackgroundMsg;
use crate::watcher::WatchRequest;
use crate::watcher::WatcherMsg;

/// Bundle the four channel pairs plus the watcher handle that
/// [`Background`] owns. Single argument to [`Background::new`].
pub struct BackgroundChannels {
    pub background: (Sender<BackgroundMsg>, Receiver<BackgroundMsg>),
    pub ci_fetch:   (Sender<CiFetchMsg>, Receiver<CiFetchMsg>),
    pub clean:      (Sender<CleanMsg>, Receiver<CleanMsg>),
    pub example:    (Sender<ExampleMsg>, Receiver<ExampleMsg>),
    pub watcher:    WatcherHandle,
}

/// Owns every long-lived I/O channel App holds. App holds a single
/// `background: Background` field.
pub(super) struct Background {
    sender:      Sender<BackgroundMsg>,
    receiver:    Receiver<BackgroundMsg>,
    ci_fetch_tx: Sender<CiFetchMsg>,
    ci_fetch_rx: Receiver<CiFetchMsg>,
    clean_tx:    Sender<CleanMsg>,
    clean_rx:    Receiver<CleanMsg>,
    example_tx:  Sender<ExampleMsg>,
    example_rx:  Receiver<ExampleMsg>,
    watcher:     WatcherHandle,
}

impl Background {
    pub(super) fn new(channels: BackgroundChannels) -> Self {
        let BackgroundChannels {
            background: (background_tx, background_rx),
            ci_fetch: (ci_fetch_tx, ci_fetch_rx),
            clean: (clean_tx, clean_rx),
            example: (example_tx, example_rx),
            watcher,
        } = channels;
        Self {
            sender: background_tx,
            receiver: background_rx,
            ci_fetch_tx,
            ci_fetch_rx,
            clean_tx,
            clean_rx,
            example_tx,
            example_rx,
            watcher,
        }
    }

    // ── Senders (cloned by spawn paths) ──────────────────────────────

    pub(super) fn background_sender(&self) -> Sender<BackgroundMsg> { self.sender.clone() }

    pub(super) fn ci_fetch_sender(&self) -> Sender<CiFetchMsg> { self.ci_fetch_tx.clone() }

    pub(super) fn clean_sender(&self) -> Sender<CleanMsg> { self.clean_tx.clone() }

    pub(super) fn example_sender(&self) -> Sender<ExampleMsg> { self.example_tx.clone() }

    // ── Receiver access ──────────────────────────────────────────────

    pub(super) const fn background_receiver(&self) -> &Receiver<BackgroundMsg> { &self.receiver }

    pub(super) const fn ci_fetch_rx(&self) -> &Receiver<CiFetchMsg> { &self.ci_fetch_rx }

    pub(super) const fn clean_rx(&self) -> &Receiver<CleanMsg> { &self.clean_rx }

    pub(super) const fn example_rx(&self) -> &Receiver<ExampleMsg> { &self.example_rx }

    /// Send `msg` on the watcher channel. Convenience for the
    /// common watcher-registration pattern. Disabled watcher handles
    /// accept the message without starting a watcher thread.
    pub(super) fn send_watcher(&self, msg: WatcherMsg) -> Result<(), SendError<WatcherMsg>> {
        self.watcher.send(msg)
    }

    /// Replace the background channel pair wholesale. Called from
    /// `App::rescan` — the background channel is rebuilt for each scan run
    /// while the other three channel pairs outlive any single
    /// rescan. The asymmetry stays explicit in the API rather than
    /// getting smoothed over (see plan note "Background channel-
    /// rescan caveat").
    pub(super) fn swap_background_channel(
        &mut self,
        sender: Sender<BackgroundMsg>,
        receiver: Receiver<BackgroundMsg>,
    ) {
        self.sender = sender;
        self.receiver = receiver;
    }

    /// Replace the watcher handle, used by `App::respawn_watcher`
    /// after a config reload changes the watch roots.
    pub(super) fn replace_watcher(&mut self, watcher: WatcherHandle) { self.watcher = watcher; }

    #[cfg(test)]
    pub(super) const fn watcher_is_active(&self) -> bool { self.watcher.is_active() }

    pub(super) fn register_item_background_services(&self, item: &RootItem) {
        let started = std::time::Instant::now();
        let abs_path = AbsolutePath::from(item.path().to_path_buf());
        let repo_root = project::git_repo_root(&abs_path);
        let has_repo_root = repo_root.is_some();
        let _ = self.send_watcher(WatcherMsg::Register(WatchRequest {
            project_label: abs_path.to_string_lossy().to_string(),
            abs_path: abs_path.clone(),
            repo_root,
        }));
        tracing::trace!(
            target: PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
            path = %item.display_path(),
            has_repo_root,
            "app_register_project_background_services"
        );
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::channel;

    fn make_msg() -> BackgroundMsg {
        BackgroundMsg::RepoFetchQueued {
            repo: crate::ci::OwnerRepo::new("owner", "repo"),
        }
    }

    fn fresh() -> Background {
        let (watch_tx, _watch_rx) = channel::unbounded();
        Background::new(BackgroundChannels {
            background: channel::unbounded(),
            ci_fetch:   channel::unbounded(),
            clean:      channel::unbounded(),
            example:    channel::unbounded(),
            watcher:    WatcherHandle::active(watch_tx),
        })
    }

    #[test]
    fn bg_sender_clone_round_trips_through_rx() {
        let background = fresh();
        let sender = background.background_sender();
        sender
            .send(make_msg())
            .expect("send through cloned bg sender");
        let received = background
            .background_receiver()
            .recv()
            .expect("recv on background_rx");
        assert!(matches!(received, BackgroundMsg::RepoFetchQueued { .. }));
    }

    #[test]
    fn swap_bg_channel_routes_to_new_pair_only() {
        let mut background = fresh();
        let original_sender = background.background_sender();

        let (new_tx, new_rx) = channel::unbounded();
        background.swap_background_channel(new_tx, new_rx);

        // Sender cloned before the swap can still send (it's tied to
        // the dropped receiver), but the swapped-in receiver must
        // not see anything from it.
        let _ = original_sender.send(make_msg());
        assert!(
            background.background_receiver().try_recv().is_err(),
            "stale sender must not reach the swapped-in rx"
        );

        // A fresh send via the new sender DOES reach the new rx.
        background
            .background_sender()
            .send(make_msg())
            .expect("send through post-swap bg sender");
        let received = background
            .background_receiver()
            .recv()
            .expect("recv on swapped background_rx");
        assert!(matches!(received, BackgroundMsg::RepoFetchQueued { .. }));
    }

    #[test]
    fn send_watcher_delivers_to_watcher_channel() {
        let (watch_tx, watch_rx) = channel::unbounded();
        let background = Background::new(BackgroundChannels {
            background: channel::unbounded(),
            ci_fetch:   channel::unbounded(),
            clean:      channel::unbounded(),
            example:    channel::unbounded(),
            watcher:    WatcherHandle::active(watch_tx),
        });

        background
            .send_watcher(WatcherMsg::InitialRegistrationComplete)
            .expect("send_watcher succeeds");
        let received = watch_rx.recv().expect("recv on watch_rx");
        assert!(matches!(received, WatcherMsg::InitialRegistrationComplete));
    }

    #[test]
    fn replace_watcher_handle_redirects_send_watcher() {
        let mut background = fresh();
        let (new_watch_tx, new_watch_rx) = channel::unbounded();
        background.replace_watcher(WatcherHandle::active(new_watch_tx));
        background
            .send_watcher(WatcherMsg::InitialRegistrationComplete)
            .expect("send_watcher succeeds post-replace");
        let received = new_watch_rx.recv().expect("recv on new watcher rx");
        assert!(matches!(received, WatcherMsg::InitialRegistrationComplete));
    }

    #[test]
    fn disabled_watcher_handle_ignores_registration_messages() {
        let background = Background::new(BackgroundChannels {
            background: channel::unbounded(),
            ci_fetch:   channel::unbounded(),
            clean:      channel::unbounded(),
            example:    channel::unbounded(),
            watcher:    WatcherHandle::disabled(),
        });

        background
            .send_watcher(WatcherMsg::InitialRegistrationComplete)
            .expect("disabled watcher accepts completion");

        assert!(!background.watcher_is_active());
    }
}
