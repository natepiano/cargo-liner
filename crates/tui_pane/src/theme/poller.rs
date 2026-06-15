//! Background poller for the OS appearance setting.
//!
//! Spawned on a `tokio` runtime. Wakes every 1500ms, calls
//! [`dark_light::detect`] on a blocking thread, and invokes the
//! caller-supplied closure whenever the value transitions.  After 10
//! consecutive detect errors the cadence drops to 30s — a broken
//! backend stops hammering syscalls without losing the recovery path
//! (the first successful detect after the slow tick restores the
//! 1500ms baseline).

use std::time::Duration;

use dark_light::Error;
use dark_light::Mode;
use tokio::runtime::Handle;
use tokio::time::Interval;
use tokio::time::MissedTickBehavior;

use super::Appearance;
use super::constants::BACKOFF_INTERVAL;
use super::constants::BACKOFF_THRESHOLD;
use super::constants::POLL_INTERVAL;

/// Spawn the OS appearance background task.
///
/// Watches the OS appearance setting and calls `on_change` whenever
/// the value transitions between `Light` and `Dark`. `on_change` runs
/// on the tokio runtime; do minimal work inside it (e.g. forward to a
/// channel).
pub fn spawn_appearance_poller<F>(handle: &Handle, on_change: F)
where
    F: Fn(Appearance) + Send + 'static,
{
    handle.spawn(run(on_change));
}

async fn run<F>(on_change: F)
where
    F: Fn(Appearance) + Send + 'static,
{
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
                    if let Some(appearance) = next {
                        on_change(appearance);
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
    tokio::task::spawn_blocking(dark_light::detect)
        .await
        .unwrap_or_else(|join_err| Err(Error::Io(std::io::Error::other(join_err.to_string()))))
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
