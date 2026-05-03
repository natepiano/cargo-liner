//! The `Background` subsystem.
//!
//! Owns the four mpsc channel pairs plus the watcher sender:
//! - `bg_tx` / `bg_rx` (replaced wholesale on every rescan — see [`Background::swap_bg_channel`])
//! - `ci_fetch_tx` / `ci_fetch_rx`
//! - `clean_tx` / `clean_rx`
//! - `example_tx` / `example_rx`
//! - `watch_tx`
//!
//! Spawn / poll facade methods live on `App` (and inside
//! [`Inflight`]) because they thread cross-subsystem dependencies
//! (`Scan`, `Net`, `ToastManager`).
//!
//! [`Inflight`]: super::inflight::Inflight

use std::sync::mpsc;

use super::terminal::CiFetchMsg;
use super::terminal::CleanMsg;
use super::terminal::ExampleMsg;
use crate::scan::BackgroundMsg;
use crate::watcher::WatcherMsg;

/// Bundle the four channel pairs plus the watcher sender that
/// [`Background`] owns. Single argument to [`Background::new`].
pub struct BackgroundChannels {
    pub bg:       (mpsc::Sender<BackgroundMsg>, mpsc::Receiver<BackgroundMsg>),
    pub ci_fetch: (mpsc::Sender<CiFetchMsg>, mpsc::Receiver<CiFetchMsg>),
    pub clean:    (mpsc::Sender<CleanMsg>, mpsc::Receiver<CleanMsg>),
    pub example:  (mpsc::Sender<ExampleMsg>, mpsc::Receiver<ExampleMsg>),
    pub watch_tx: mpsc::Sender<WatcherMsg>,
}

/// Owns every long-lived I/O channel App holds. App holds a single
/// `background: Background` field.
pub(super) struct Background {
    bg_tx:       mpsc::Sender<BackgroundMsg>,
    bg_rx:       mpsc::Receiver<BackgroundMsg>,
    ci_fetch_tx: mpsc::Sender<CiFetchMsg>,
    ci_fetch_rx: mpsc::Receiver<CiFetchMsg>,
    clean_tx:    mpsc::Sender<CleanMsg>,
    clean_rx:    mpsc::Receiver<CleanMsg>,
    example_tx:  mpsc::Sender<ExampleMsg>,
    example_rx:  mpsc::Receiver<ExampleMsg>,
    watch_tx:    mpsc::Sender<WatcherMsg>,
}

impl Background {
    pub(super) fn new(channels: BackgroundChannels) -> Self {
        let BackgroundChannels {
            bg: (bg_tx, bg_rx),
            ci_fetch: (ci_fetch_tx, ci_fetch_rx),
            clean: (clean_tx, clean_rx),
            example: (example_tx, example_rx),
            watch_tx,
        } = channels;
        Self {
            bg_tx,
            bg_rx,
            ci_fetch_tx,
            ci_fetch_rx,
            clean_tx,
            clean_rx,
            example_tx,
            example_rx,
            watch_tx,
        }
    }

    // ── Senders (cloned by spawn paths) ──────────────────────────────

    pub(super) fn bg_sender(&self) -> mpsc::Sender<BackgroundMsg> { self.bg_tx.clone() }

    pub(super) fn ci_fetch_sender(&self) -> mpsc::Sender<CiFetchMsg> { self.ci_fetch_tx.clone() }

    pub(super) fn clean_sender(&self) -> mpsc::Sender<CleanMsg> { self.clean_tx.clone() }

    pub(super) fn example_sender(&self) -> mpsc::Sender<ExampleMsg> { self.example_tx.clone() }

    // ── Receiver access ──────────────────────────────────────────────

    pub(super) const fn bg_rx(&self) -> &mpsc::Receiver<BackgroundMsg> { &self.bg_rx }

    pub(super) const fn ci_fetch_rx(&self) -> &mpsc::Receiver<CiFetchMsg> { &self.ci_fetch_rx }

    pub(super) const fn clean_rx(&self) -> &mpsc::Receiver<CleanMsg> { &self.clean_rx }

    pub(super) const fn example_rx(&self) -> &mpsc::Receiver<ExampleMsg> { &self.example_rx }

    /// Send `msg` on the watcher channel. Convenience for the
    /// common `watch_tx.send(...)` pattern.
    pub(super) fn send_watcher(&self, msg: WatcherMsg) -> Result<(), mpsc::SendError<WatcherMsg>> {
        self.watch_tx.send(msg)
    }

    /// Replace the bg channel pair wholesale. Called from
    /// `App::rescan` — the bg channel is rebuilt for each scan run
    /// while the other three channel pairs outlive any single
    /// rescan. The asymmetry stays explicit in the API rather than
    /// getting smoothed over (see plan note "Background channel-
    /// rescan caveat").
    pub(super) fn swap_bg_channel(
        &mut self,
        tx: mpsc::Sender<BackgroundMsg>,
        rx: mpsc::Receiver<BackgroundMsg>,
    ) {
        self.bg_tx = tx;
        self.bg_rx = rx;
    }

    /// Replace the watcher sender, used by `App::respawn_watcher`
    /// after a config reload changes the watch roots.
    pub(super) fn replace_watcher_sender(&mut self, tx: mpsc::Sender<WatcherMsg>) {
        self.watch_tx = tx;
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    fn make_msg() -> BackgroundMsg {
        BackgroundMsg::RepoFetchQueued {
            repo: crate::ci::OwnerRepo::new("owner", "repo"),
        }
    }

    fn fresh() -> Background {
        let (watch_tx, _watch_rx) = mpsc::channel();
        Background::new(BackgroundChannels {
            bg: mpsc::channel(),
            ci_fetch: mpsc::channel(),
            clean: mpsc::channel(),
            example: mpsc::channel(),
            watch_tx,
        })
    }

    #[test]
    fn bg_sender_clone_round_trips_through_rx() {
        let bg = fresh();
        let sender = bg.bg_sender();
        sender
            .send(make_msg())
            .expect("send through cloned bg sender");
        let received = bg.bg_rx().recv().expect("recv on bg_rx");
        assert!(matches!(received, BackgroundMsg::RepoFetchQueued { .. }));
    }

    #[test]
    fn swap_bg_channel_routes_to_new_pair_only() {
        let mut bg = fresh();
        let original_sender = bg.bg_sender();

        let (new_tx, new_rx) = mpsc::channel();
        bg.swap_bg_channel(new_tx, new_rx);

        // Sender cloned before the swap can still send (it's tied to
        // the dropped receiver), but the swapped-in receiver must
        // not see anything from it.
        let _ = original_sender.send(make_msg());
        assert!(
            bg.bg_rx().try_recv().is_err(),
            "stale sender must not reach the swapped-in rx"
        );

        // A fresh send via the new sender DOES reach the new rx.
        bg.bg_sender()
            .send(make_msg())
            .expect("send through post-swap bg sender");
        let received = bg.bg_rx().recv().expect("recv on swapped bg_rx");
        assert!(matches!(received, BackgroundMsg::RepoFetchQueued { .. }));
    }

    #[test]
    fn send_watcher_delivers_to_watcher_channel() {
        let (watch_tx, watch_rx) = mpsc::channel();
        let bg = Background::new(BackgroundChannels {
            bg: mpsc::channel(),
            ci_fetch: mpsc::channel(),
            clean: mpsc::channel(),
            example: mpsc::channel(),
            watch_tx,
        });

        bg.send_watcher(WatcherMsg::InitialRegistrationComplete)
            .expect("send_watcher succeeds");
        let received = watch_rx.recv().expect("recv on watch_rx");
        assert!(matches!(received, WatcherMsg::InitialRegistrationComplete));
    }

    #[test]
    fn replace_watcher_sender_redirects_send_watcher() {
        let mut bg = fresh();
        let (new_watch_tx, new_watch_rx) = mpsc::channel();
        bg.replace_watcher_sender(new_watch_tx);
        bg.send_watcher(WatcherMsg::InitialRegistrationComplete)
            .expect("send_watcher succeeds post-replace");
        let received = new_watch_rx.recv().expect("recv on new watcher rx");
        assert!(matches!(received, WatcherMsg::InitialRegistrationComplete));
    }
}
