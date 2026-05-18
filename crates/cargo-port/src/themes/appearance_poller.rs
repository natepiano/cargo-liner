//! Background poller for the OS appearance setting.
//!
//! Spawned at startup on the tokio runtime. Wakes every 1500ms, calls
//! [`dark_light::detect`] on a blocking thread, and emits
//! [`BackgroundMsg::AppearanceChanged`] only when the value transitions.
//! After 10 consecutive detect errors the cadence drops to 30s — a
//! broken backend stops hammering syscalls without losing the recovery
//! path (the first successful detect after the slow tick restores the
//! 1500ms baseline). The poller exits silently when the background
//! channel closes.

use std::sync::mpsc::Sender;
use std::time::Duration;

use dark_light::Error;
use dark_light::Mode;
use tokio::runtime::Handle;
use tokio::time::Interval;
use tokio::time::MissedTickBehavior;
use tui_pane::Appearance;

use crate::scan::BackgroundMsg;

const POLL_INTERVAL: Duration = Duration::from_millis(1500);
const BACKOFF_INTERVAL: Duration = Duration::from_secs(30);
const BACKOFF_THRESHOLD: u32 = 10;

pub(crate) fn spawn_appearance_poller(handle: &Handle, tx: Sender<BackgroundMsg>) {
    handle.spawn(run(tx));
}

async fn run(tx: Sender<BackgroundMsg>) {
    // Start with `last = None` so the first tick emits the current OS
    // state. Otherwise startup resolves with `os = None` (which `Auto`
    // mode falls back to `Dark` for), and the user's actual appearance
    // is never delivered until the OS itself transitions during the
    // session.
    let mut last: Option<Appearance> = None;
    let mut current_interval = POLL_INTERVAL;
    let mut consecutive_errors: u32 = 0;
    let mut ticker = new_ticker(current_interval);

    loop {
        ticker.tick().await;
        match detect_blocking().await {
            Ok(mode) => {
                consecutive_errors = 0;
                if current_interval != POLL_INTERVAL {
                    current_interval = POLL_INTERVAL;
                    ticker = new_ticker(current_interval);
                }
                let next = to_appearance(mode);
                if next != last {
                    last = next;
                    if let Some(appearance) = next
                        && tx
                            .send(BackgroundMsg::AppearanceChanged(appearance))
                            .is_err()
                    {
                        return;
                    }
                }
            },
            Err(err) => {
                consecutive_errors = consecutive_errors.saturating_add(1);
                if consecutive_errors == BACKOFF_THRESHOLD {
                    tracing::warn!(
                        error = %err,
                        threshold = BACKOFF_THRESHOLD,
                        backoff_secs = BACKOFF_INTERVAL.as_secs(),
                        "dark_light_detect_backoff"
                    );
                    current_interval = BACKOFF_INTERVAL;
                    ticker = new_ticker(current_interval);
                }
            },
        }
    }
}

fn new_ticker(period: Duration) -> Interval {
    let mut ticker = tokio::time::interval(period);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ticker
}

async fn detect_blocking() -> Result<Mode, Error> {
    match tokio::task::spawn_blocking(dark_light::detect).await {
        Ok(result) => result,
        Err(join_err) => Err(Error::Io(std::io::Error::other(join_err.to_string()))),
    }
}

const fn to_appearance(mode: Mode) -> Option<Appearance> {
    match mode {
        Mode::Light => Some(Appearance::Light),
        Mode::Dark => Some(Appearance::Dark),
        Mode::Unspecified => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_appearance_maps_light_and_dark() {
        assert_eq!(to_appearance(Mode::Light), Some(Appearance::Light));
        assert_eq!(to_appearance(Mode::Dark), Some(Appearance::Dark));
        assert!(to_appearance(Mode::Unspecified).is_none());
    }
}
