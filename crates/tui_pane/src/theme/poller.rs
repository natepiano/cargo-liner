//! Background tracking for the OS appearance setting.
//!
//! Linux holds one session-bus connection, reads the startup setting through
//! it, and then waits for change signals. Failed subscriptions retry with
//! backoff. Other platforms retain their existing detection cadence.

#[cfg(target_os = "linux")]
use std::future::Future;
#[cfg(target_os = "linux")]
use std::pin::Pin;
use std::time::Duration;

#[cfg(not(target_os = "linux"))]
use dark_light::Error;
use dark_light::Mode;
#[cfg(target_os = "linux")]
use futures_lite::Stream;
#[cfg(target_os = "linux")]
use futures_lite::StreamExt;
#[cfg(target_os = "linux")]
use futures_lite::stream;
use tokio::runtime::Handle;
#[cfg(not(target_os = "linux"))]
use tokio::time::Interval;
#[cfg(not(target_os = "linux"))]
use tokio::time::MissedTickBehavior;
#[cfg(target_os = "linux")]
use zbus::Connection;
#[cfg(target_os = "linux")]
use zbus::Message;
#[cfg(target_os = "linux")]
use zbus::Proxy;
#[cfg(target_os = "linux")]
use zbus::zvariant::OwnedValue;

use super::Appearance;
#[cfg(target_os = "linux")]
use super::constants::APPEARANCE_NAMESPACE;
use super::constants::BACKOFF_INTERVAL;
use super::constants::BACKOFF_THRESHOLD;
#[cfg(target_os = "linux")]
use super::constants::COLOR_SCHEME_DARK;
#[cfg(target_os = "linux")]
use super::constants::COLOR_SCHEME_KEY;
#[cfg(target_os = "linux")]
use super::constants::COLOR_SCHEME_LIGHT;
use super::constants::POLL_INTERVAL;
#[cfg(target_os = "linux")]
use super::constants::PORTAL_DESTINATION;
#[cfg(target_os = "linux")]
use super::constants::PORTAL_PATH;
#[cfg(target_os = "linux")]
use super::constants::SETTINGS_CHANGED;
#[cfg(target_os = "linux")]
use super::constants::SETTINGS_INTERFACE;
#[cfg(target_os = "linux")]
use super::constants::SETTINGS_READ;

/// A setting reported by the OS; unspecified values never reach the callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppearanceObservation {
    Unspecified,
    Selected(Appearance),
}

impl From<Mode> for AppearanceObservation {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Light => Self::Selected(Appearance::Light),
            Mode::Dark => Self::Selected(Appearance::Dark),
            Mode::Unspecified => Self::Unspecified,
        }
    }
}

#[cfg(target_os = "linux")]
impl TryFrom<OwnedValue> for AppearanceObservation {
    type Error = zbus::zvariant::Error;

    fn try_from(value: OwnedValue) -> Result<Self, Self::Error> {
        Ok(match u32::try_from(value)? {
            COLOR_SCHEME_DARK => Self::Selected(Appearance::Dark),
            COLOR_SCHEME_LIGHT => Self::Selected(Appearance::Light),
            _ => Self::Unspecified,
        })
    }
}

/// Tracks observations, including unspecified values that were not delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppearanceHistory {
    Unobserved,
    Observed(AppearanceObservation),
}

impl AppearanceHistory {
    fn observe<F>(&mut self, observation: AppearanceObservation, on_change: F)
    where
        F: FnOnce(Appearance),
    {
        if *self == Self::Observed(observation) {
            return;
        }
        *self = Self::Observed(observation);
        if let AppearanceObservation::Selected(appearance) = observation {
            on_change(appearance);
        }
    }
}

/// Retains whether the backend ever supplied a startup value and live watch.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubscriptionRecovery {
    NeverConnected { failures: u32 },
    Live,
    Interrupted { failures: u32 },
}

#[cfg(target_os = "linux")]
impl SubscriptionRecovery {
    const fn failed(&mut self) -> Duration {
        let failures = match *self {
            Self::NeverConnected { failures } => {
                let failures = failures.saturating_add(1);
                *self = Self::NeverConnected { failures };
                failures
            },
            Self::Live => {
                *self = Self::Interrupted { failures: 1 };
                1
            },
            Self::Interrupted { failures } => {
                let failures = failures.saturating_add(1);
                *self = Self::Interrupted { failures };
                failures
            },
        };
        if failures >= BACKOFF_THRESHOLD {
            BACKOFF_INTERVAL
        } else {
            POLL_INTERVAL
        }
    }
}

/// Why a live subscription stopped supplying observations.
#[cfg(target_os = "linux")]
#[derive(Debug)]
enum SubscriptionEnd {
    Disconnected,
    OwnerChanged,
    InvalidSetting(zbus::Error),
}

#[cfg(target_os = "linux")]
impl std::fmt::Display for SubscriptionEnd {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => formatter.write_str("appearance subscription ended"),
            Self::OwnerChanged => formatter.write_str("appearance portal owner changed"),
            Self::InvalidSetting(error) => write!(formatter, "invalid appearance setting: {error}"),
        }
    }
}

/// Spawn the OS appearance background task.
///
/// Delivers the first concrete OS appearance and subsequent changes. Linux
/// subscribes through one held session-bus connection and retries if it ends;
/// other platforms poll. Unspecified observations do not call `on_change`.
/// The callback runs on the tokio runtime and should do minimal work (e.g.
/// forward to a channel).
pub fn spawn_appearance_poller<F>(handle: &Handle, on_change: F)
where
    F: Fn(Appearance) + Send + 'static,
{
    handle.spawn(run(on_change));
}

#[cfg(target_os = "linux")]
type AppearanceChanges =
    Pin<Box<dyn Stream<Item = Result<AppearanceObservation, SubscriptionEnd>> + Send>>;

/// The three operations that establish an appearance subscription. Keeping
/// them separate lets tests verify that the startup read uses the watched
/// connection and that the task only repeats them after a failure.
#[cfg(target_os = "linux")]
trait AppearanceBackend {
    type Connection: Send + Sync;

    fn connect(&mut self) -> impl Future<Output = zbus::Result<Self::Connection>> + Send;
    fn watch(
        connection: &Self::Connection,
    ) -> impl Future<Output = zbus::Result<AppearanceChanges>> + Send;
    fn read(
        connection: &Self::Connection,
    ) -> impl Future<Output = zbus::Result<AppearanceObservation>> + Send;
}

#[cfg(target_os = "linux")]
struct PortalBackend;

#[cfg(target_os = "linux")]
impl AppearanceBackend for PortalBackend {
    type Connection = Proxy<'static>;

    async fn connect(&mut self) -> zbus::Result<Self::Connection> {
        let connection = Connection::session().await?;
        Proxy::new_owned(
            connection,
            PORTAL_DESTINATION,
            PORTAL_PATH,
            SETTINGS_INTERFACE,
        )
        .await
    }

    async fn watch(portal: &Self::Connection) -> zbus::Result<AppearanceChanges> {
        let owners = portal.receive_owner_changed().await?;
        let changes = portal
            .receive_signal_with_args(
                SETTINGS_CHANGED,
                &[(0, APPEARANCE_NAMESPACE), (1, COLOR_SCHEME_KEY)],
            )
            .await?;
        let changes = changes
            .map(|message| setting_observation(&message))
            .chain(stream::iter([Err(SubscriptionEnd::Disconnected)]));
        let owners = owners
            .map(|_| Err(SubscriptionEnd::OwnerChanged))
            .chain(stream::iter([Err(SubscriptionEnd::Disconnected)]));
        Ok(Box::pin(stream::or(changes, owners)))
    }

    async fn read(portal: &Self::Connection) -> zbus::Result<AppearanceObservation> {
        let initial: OwnedValue = portal
            .call(SETTINGS_READ, &(APPEARANCE_NAMESPACE, COLOR_SCHEME_KEY))
            .await?;
        Ok(AppearanceObservation::try_from(initial)?)
    }
}

#[cfg(target_os = "linux")]
async fn run<F>(on_change: F)
where
    F: Fn(Appearance) + Send + 'static,
{
    track(PortalBackend, on_change, |_, delay| {
        tokio::time::sleep(delay)
    })
    .await;
}

#[cfg(target_os = "linux")]
async fn track<B, F, R, D>(mut backend: B, mut on_change: F, mut retry: R)
where
    B: AppearanceBackend,
    F: Fn(Appearance),
    R: FnMut(SubscriptionRecovery, Duration) -> D,
    D: Future<Output = ()>,
{
    let mut history = AppearanceHistory::Unobserved;
    let mut recovery = SubscriptionRecovery::NeverConnected { failures: 0 };
    loop {
        let result = subscribe(&mut backend, &mut history, &mut recovery, &mut on_change).await;
        let delay = recovery.failed();
        match result {
            Ok(reason) => {
                tracing::warn!(%reason, ?recovery, retry_secs = delay.as_secs_f32(), "appearance_subscription_ended");
            },
            Err(error) => {
                tracing::warn!(%error, ?recovery, retry_secs = delay.as_secs_f32(), "appearance_subscription_failed");
            },
        }
        retry(recovery, delay).await;
    }
}

#[cfg(target_os = "linux")]
async fn subscribe<B, F>(
    backend: &mut B,
    history: &mut AppearanceHistory,
    recovery: &mut SubscriptionRecovery,
    on_change: &mut F,
) -> zbus::Result<SubscriptionEnd>
where
    B: AppearanceBackend,
    F: Fn(Appearance),
{
    let connection = backend.connect().await?;
    // Install both watches before reading so a change during startup is queued.
    let mut observations = B::watch(&connection).await?;
    let initial = B::read(&connection).await?;
    Ok(follow(initial, &mut observations, history, recovery, on_change).await)
}

#[cfg(target_os = "linux")]
fn setting_observation(message: &Message) -> Result<AppearanceObservation, SubscriptionEnd> {
    let (_, _, value): (String, String, OwnedValue) = message
        .body()
        .deserialize()
        .map_err(SubscriptionEnd::InvalidSetting)?;
    AppearanceObservation::try_from(value)
        .map_err(|error| SubscriptionEnd::InvalidSetting(error.into()))
}

#[cfg(target_os = "linux")]
async fn follow<S, F>(
    initial: AppearanceObservation,
    observations: &mut S,
    history: &mut AppearanceHistory,
    recovery: &mut SubscriptionRecovery,
    mut on_change: F,
) -> SubscriptionEnd
where
    S: Stream<Item = Result<AppearanceObservation, SubscriptionEnd>> + Unpin,
    F: FnMut(Appearance),
{
    *recovery = SubscriptionRecovery::Live;
    history.observe(initial, &mut on_change);
    while let Some(observation) = observations.next().await {
        match observation {
            Ok(observation) => history.observe(observation, &mut on_change),
            Err(reason) => return reason,
        }
    }
    SubscriptionEnd::Disconnected
}

#[cfg(not(target_os = "linux"))]
async fn run<F>(on_change: F)
where
    F: Fn(Appearance) + Send + 'static,
{
    let mut history = AppearanceHistory::Unobserved;
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
                history.observe(mode.into(), &on_change);
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

#[cfg(not(target_os = "linux"))]
fn new_ticker(period: Duration) -> Interval {
    let mut ticker = tokio::time::interval(period);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ticker
}

#[cfg(not(target_os = "linux"))]
async fn detect_blocking() -> Result<Mode, Error> {
    tokio::task::spawn_blocking(dark_light::detect)
        .await
        .unwrap_or_else(|join_err| Err(Error::Io(std::io::Error::other(join_err.to_string()))))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn external_modes_become_explicit_observations() {
        assert_eq!(
            AppearanceObservation::from(Mode::Light),
            AppearanceObservation::Selected(Appearance::Light)
        );
        assert_eq!(
            AppearanceObservation::from(Mode::Dark),
            AppearanceObservation::Selected(Appearance::Dark)
        );
        assert_eq!(
            AppearanceObservation::from(Mode::Unspecified),
            AppearanceObservation::Unspecified
        );
    }

    #[test]
    fn unspecified_startup_is_observed_without_delivery() {
        let delivered = RefCell::new(Vec::new());
        let mut history = AppearanceHistory::Unobserved;
        history.observe(AppearanceObservation::Unspecified, |appearance| {
            delivered.borrow_mut().push(appearance);
        });
        assert_eq!(
            history,
            AppearanceHistory::Observed(AppearanceObservation::Unspecified)
        );
        assert!(delivered.borrow().is_empty());
    }

    #[test]
    fn repeated_unspecified_then_first_concrete_delivers_once() {
        let delivered = RefCell::new(Vec::new());
        let on_change = |appearance| delivered.borrow_mut().push(appearance);
        let mut history = AppearanceHistory::Unobserved;
        history.observe(AppearanceObservation::Unspecified, on_change);
        history.observe(AppearanceObservation::Unspecified, on_change);
        assert!(delivered.borrow().is_empty());
        history.observe(
            AppearanceObservation::Selected(Appearance::Light),
            on_change,
        );
        history.observe(
            AppearanceObservation::Selected(Appearance::Light),
            on_change,
        );
        assert_eq!(*delivered.borrow(), [Appearance::Light]);
    }

    #[test]
    fn first_concrete_startup_is_delivered() {
        let delivered = RefCell::new(Vec::new());
        let mut history = AppearanceHistory::Unobserved;
        history.observe(
            AppearanceObservation::Selected(Appearance::Dark),
            |appearance| delivered.borrow_mut().push(appearance),
        );
        assert_eq!(*delivered.borrow(), [Appearance::Dark]);
    }

    #[test]
    fn concrete_unspecified_same_concrete_is_delivered_again() {
        let delivered = RefCell::new(Vec::new());
        let on_change = |appearance| delivered.borrow_mut().push(appearance);
        let mut history = AppearanceHistory::Unobserved;
        for observation in [
            AppearanceObservation::Selected(Appearance::Dark),
            AppearanceObservation::Unspecified,
            AppearanceObservation::Selected(Appearance::Dark),
        ] {
            history.observe(observation, on_change);
        }
        assert_eq!(*delivered.borrow(), [Appearance::Dark, Appearance::Dark]);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn idle_subscription_delivers_startup_once_and_keeps_waiting() {
        let delivered = RefCell::new(Vec::new());
        let on_change = |appearance| delivered.borrow_mut().push(appearance);
        let mut history = AppearanceHistory::Unobserved;
        let mut recovery = SubscriptionRecovery::NeverConnected { failures: 0 };
        let mut observations = stream::pending();
        let mut following = Box::pin(follow(
            AppearanceObservation::Selected(Appearance::Light),
            &mut observations,
            &mut history,
            &mut recovery,
            on_change,
        ));
        assert!(
            futures_lite::future::poll_once(&mut following)
                .await
                .is_none()
        );
        assert_eq!(*delivered.borrow(), [Appearance::Light]);
        assert!(
            futures_lite::future::poll_once(&mut following)
                .await
                .is_none()
        );
        assert_eq!(*delivered.borrow(), [Appearance::Light]);
        drop(following);
        assert_eq!(recovery, SubscriptionRecovery::Live);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn subscription_ended_and_retried_delivers_changed_startup() {
        let delivered = RefCell::new(Vec::new());
        let on_change = |appearance| delivered.borrow_mut().push(appearance);
        let mut history = AppearanceHistory::Unobserved;
        let mut recovery = SubscriptionRecovery::NeverConnected { failures: 0 };
        let reason = follow(
            AppearanceObservation::Selected(Appearance::Light),
            &mut stream::empty(),
            &mut history,
            &mut recovery,
            on_change,
        )
        .await;
        assert!(matches!(reason, SubscriptionEnd::Disconnected));
        assert_eq!(recovery.failed(), POLL_INTERVAL);
        assert_eq!(recovery, SubscriptionRecovery::Interrupted { failures: 1 });
        assert_eq!(recovery.failed(), POLL_INTERVAL);
        assert_eq!(recovery, SubscriptionRecovery::Interrupted { failures: 2 });
        let reason = follow(
            AppearanceObservation::Selected(Appearance::Dark),
            &mut stream::iter([Err(SubscriptionEnd::OwnerChanged)]),
            &mut history,
            &mut recovery,
            on_change,
        )
        .await;
        assert!(matches!(reason, SubscriptionEnd::OwnerChanged));
        assert_eq!(recovery, SubscriptionRecovery::Live);
        assert_eq!(*delivered.borrow(), [Appearance::Light, Appearance::Dark]);
        assert_eq!(recovery.failed(), POLL_INTERVAL);
        assert_eq!(recovery, SubscriptionRecovery::Interrupted { failures: 1 });
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn stream_observations_preserve_unspecified_transitions() {
        let delivered = RefCell::new(Vec::new());
        let on_change = |appearance| delivered.borrow_mut().push(appearance);
        let mut history = AppearanceHistory::Unobserved;
        let mut recovery = SubscriptionRecovery::NeverConnected { failures: 0 };
        let mut observations = stream::iter([
            Ok(AppearanceObservation::Unspecified),
            Ok(AppearanceObservation::Selected(Appearance::Light)),
            Ok(AppearanceObservation::Selected(Appearance::Light)),
            Ok(AppearanceObservation::Unspecified),
            Ok(AppearanceObservation::Selected(Appearance::Light)),
            Ok(AppearanceObservation::Selected(Appearance::Dark)),
        ]);
        let reason = follow(
            AppearanceObservation::Unspecified,
            &mut observations,
            &mut history,
            &mut recovery,
            on_change,
        )
        .await;
        assert!(matches!(reason, SubscriptionEnd::Disconnected));
        assert_eq!(
            *delivered.borrow(),
            [Appearance::Light, Appearance::Light, Appearance::Dark]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backend_that_never_worked_backs_off_without_becoming_interrupted() {
        let mut recovery = SubscriptionRecovery::NeverConnected { failures: 0 };
        for failures in 1..BACKOFF_THRESHOLD {
            assert_eq!(recovery.failed(), POLL_INTERVAL);
            assert_eq!(recovery, SubscriptionRecovery::NeverConnected { failures });
        }
        assert_eq!(recovery.failed(), BACKOFF_INTERVAL);
        assert_eq!(
            recovery,
            SubscriptionRecovery::NeverConnected {
                failures: BACKOFF_THRESHOLD,
            }
        );
        assert_eq!(recovery.failed(), BACKOFF_INTERVAL);
        assert_eq!(
            recovery,
            SubscriptionRecovery::NeverConnected {
                failures: BACKOFF_THRESHOLD + 1,
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn recovery_resets_backoff_without_redelivering_unchanged_appearance() {
        let delivered = RefCell::new(Vec::new());
        let on_change = |appearance| delivered.borrow_mut().push(appearance);
        let observation = AppearanceObservation::Selected(Appearance::Dark);
        let mut history = AppearanceHistory::Observed(observation);
        let mut recovery = SubscriptionRecovery::Interrupted {
            failures: BACKOFF_THRESHOLD,
        };
        assert_eq!(recovery.failed(), BACKOFF_INTERVAL);
        follow(
            observation,
            &mut stream::empty(),
            &mut history,
            &mut recovery,
            on_change,
        )
        .await;
        assert!(delivered.borrow().is_empty());
        assert_eq!(recovery, SubscriptionRecovery::Live);
        assert_eq!(recovery.failed(), POLL_INTERVAL);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn portal_values_convert_at_the_wire_boundary() -> Result<(), zbus::zvariant::Error> {
        assert_eq!(
            AppearanceObservation::try_from(OwnedValue::from(COLOR_SCHEME_DARK))?,
            AppearanceObservation::Selected(Appearance::Dark)
        );
        assert_eq!(
            AppearanceObservation::try_from(OwnedValue::from(COLOR_SCHEME_LIGHT))?,
            AppearanceObservation::Selected(Appearance::Light)
        );
        assert_eq!(
            AppearanceObservation::try_from(OwnedValue::from(0_u32))?,
            AppearanceObservation::Unspecified
        );
        assert!(AppearanceObservation::try_from(OwnedValue::from(true)).is_err());
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[allow(
        clippy::expect_used,
        reason = "tests should panic on unexpected values"
    )]
    mod orchestration {
        use std::collections::VecDeque;
        use std::future;
        use std::sync::Arc;
        use std::sync::Mutex;
        use std::task::Poll;
        use std::task::Waker;

        use super::*;

        #[derive(Debug, PartialEq, Eq)]
        enum BackendCall {
            Connect(usize),
            Watch(usize),
            Read(usize),
            Close(usize),
            Retry(SubscriptionRecovery, Duration),
        }

        enum StreamTail {
            Pending,
            Ended,
            Notifications(Arc<Mutex<MockNotifications>>),
        }

        enum MockNotification {
            Observed(AppearanceObservation),
            Ended,
        }

        enum NotificationListener {
            Unpolled,
            Waiting(Waker),
        }

        struct MockNotifications {
            pending:  VecDeque<MockNotification>,
            listener: NotificationListener,
        }

        enum BackendAttempt {
            Unavailable,
            Connected {
                initial:      AppearanceObservation,
                observations: Vec<Result<AppearanceObservation, SubscriptionEnd>>,
                tail:         StreamTail,
            },
        }

        struct MockBackend {
            attempts:    VecDeque<BackendAttempt>,
            calls:       Arc<Mutex<Vec<BackendCall>>>,
            connections: usize,
        }

        struct MockConnection {
            id:           usize,
            initial:      AppearanceObservation,
            observations: Mutex<AppearanceChanges>,
            calls:        Arc<Mutex<Vec<BackendCall>>>,
        }

        impl Drop for MockConnection {
            fn drop(&mut self) {
                self.calls
                    .lock()
                    .expect("backend call log")
                    .push(BackendCall::Close(self.id));
            }
        }

        impl AppearanceBackend for MockBackend {
            type Connection = MockConnection;

            fn connect(&mut self) -> impl Future<Output = zbus::Result<MockConnection>> + Send {
                self.connections += 1;
                let id = self.connections;
                self.calls
                    .lock()
                    .expect("backend call log")
                    .push(BackendCall::Connect(id));
                future::ready(
                    match self
                        .attempts
                        .pop_front()
                        .expect("only scripted connection attempts")
                    {
                        BackendAttempt::Unavailable => {
                            Err(std::io::Error::other("backend unavailable").into())
                        },
                        BackendAttempt::Connected {
                            initial,
                            observations,
                            tail,
                        } => {
                            let tail: AppearanceChanges = match tail {
                                StreamTail::Pending => Box::pin(stream::pending()),
                                StreamTail::Ended => Box::pin(stream::empty()),
                                StreamTail::Notifications(notifications) => {
                                    notification_stream(notifications)
                                },
                            };
                            Ok(MockConnection {
                                id,
                                initial,
                                observations: Mutex::new(Box::pin(
                                    stream::iter(observations).chain(tail),
                                )),
                                calls: Arc::clone(&self.calls),
                            })
                        },
                    },
                )
            }

            fn watch(
                connection: &MockConnection,
            ) -> impl Future<Output = zbus::Result<AppearanceChanges>> + Send {
                connection
                    .calls
                    .lock()
                    .expect("backend call log")
                    .push(BackendCall::Watch(connection.id));
                future::ready(Ok(std::mem::replace(
                    &mut *connection
                        .observations
                        .lock()
                        .expect("scripted observations"),
                    Box::pin(stream::empty()),
                )))
            }

            fn read(
                connection: &MockConnection,
            ) -> impl Future<Output = zbus::Result<AppearanceObservation>> + Send {
                connection
                    .calls
                    .lock()
                    .expect("backend call log")
                    .push(BackendCall::Read(connection.id));
                future::ready(Ok(connection.initial))
            }
        }

        fn connected(initial: AppearanceObservation, tail: StreamTail) -> BackendAttempt {
            BackendAttempt::Connected {
                initial,
                observations: Vec::new(),
                tail,
            }
        }

        fn notification_stream(notifications: Arc<Mutex<MockNotifications>>) -> AppearanceChanges {
            Box::pin(stream::poll_fn(move |context| {
                let mut notifications = notifications.lock().expect("scripted notifications");
                match notifications.pending.pop_front() {
                    Some(MockNotification::Observed(observation)) => {
                        Poll::Ready(Some(Ok(observation)))
                    },
                    Some(MockNotification::Ended) => Poll::Ready(None),
                    None => {
                        notifications.listener =
                            NotificationListener::Waiting(context.waker().clone());
                        Poll::Pending
                    },
                }
            }))
        }

        fn notify(notifications: &Mutex<MockNotifications>, notification: MockNotification) {
            let listener = {
                let mut notifications = notifications.lock().expect("scripted notifications");
                notifications.pending.push_back(notification);
                std::mem::replace(&mut notifications.listener, NotificationListener::Unpolled)
            };
            if let NotificationListener::Waiting(waker) = listener {
                waker.wake();
            }
        }

        #[tokio::test]
        async fn startup_reads_the_watched_connection_once_and_idle_never_reconnects() {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let delivered = RefCell::new(Vec::new());
            let backend = MockBackend {
                attempts:    VecDeque::from([connected(
                    AppearanceObservation::Selected(Appearance::Light),
                    StreamTail::Pending,
                )]),
                calls:       Arc::clone(&calls),
                connections: 0,
            };
            let mut task = Box::pin(track(
                backend,
                |appearance| delivered.borrow_mut().push(appearance),
                |recovery, delay| {
                    calls
                        .lock()
                        .expect("backend call log")
                        .push(BackendCall::Retry(recovery, delay));
                    future::ready(())
                },
            ));
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert_eq!(*delivered.borrow(), [Appearance::Light]);
            assert_eq!(
                *calls.lock().expect("backend call log"),
                [
                    BackendCall::Connect(1),
                    BackendCall::Watch(1),
                    BackendCall::Read(1)
                ]
            );
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert_eq!(*delivered.borrow(), [Appearance::Light]);
            assert_eq!(
                *calls.lock().expect("backend call log"),
                [
                    BackendCall::Connect(1),
                    BackendCall::Watch(1),
                    BackendCall::Read(1)
                ]
            );
            drop(task);
            assert_eq!(
                calls.lock().expect("backend call log").last(),
                Some(&BackendCall::Close(1))
            );
        }

        #[tokio::test]
        async fn pending_task_receives_later_change_then_reconnects_only_after_stream_end() {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let delivered = RefCell::new(Vec::new());
            let notifications = Arc::new(Mutex::new(MockNotifications {
                pending:  VecDeque::new(),
                listener: NotificationListener::Unpolled,
            }));
            let backend = MockBackend {
                attempts:    VecDeque::from([
                    connected(
                        AppearanceObservation::Selected(Appearance::Light),
                        StreamTail::Notifications(Arc::clone(&notifications)),
                    ),
                    connected(
                        AppearanceObservation::Selected(Appearance::Light),
                        StreamTail::Pending,
                    ),
                ]),
                calls:       Arc::clone(&calls),
                connections: 0,
            };
            let mut task = Box::pin(track(
                backend,
                |appearance| delivered.borrow_mut().push(appearance),
                |recovery, delay| {
                    calls
                        .lock()
                        .expect("backend call log")
                        .push(BackendCall::Retry(recovery, delay));
                    future::ready(())
                },
            ));
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert_eq!(*delivered.borrow(), [Appearance::Light]);
            assert_eq!(
                *calls.lock().expect("backend call log"),
                [
                    BackendCall::Connect(1),
                    BackendCall::Watch(1),
                    BackendCall::Read(1)
                ]
            );
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert_eq!(*delivered.borrow(), [Appearance::Light]);
            assert_eq!(
                *calls.lock().expect("backend call log"),
                [
                    BackendCall::Connect(1),
                    BackendCall::Watch(1),
                    BackendCall::Read(1)
                ]
            );

            notify(
                &notifications,
                MockNotification::Observed(AppearanceObservation::Selected(Appearance::Dark)),
            );
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert_eq!(*delivered.borrow(), [Appearance::Light, Appearance::Dark]);
            assert_eq!(
                *calls.lock().expect("backend call log"),
                [
                    BackendCall::Connect(1),
                    BackendCall::Watch(1),
                    BackendCall::Read(1)
                ]
            );

            notify(&notifications, MockNotification::Ended);
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert_eq!(
                *delivered.borrow(),
                [Appearance::Light, Appearance::Dark, Appearance::Light]
            );
            assert_eq!(
                *calls.lock().expect("backend call log"),
                [
                    BackendCall::Connect(1),
                    BackendCall::Watch(1),
                    BackendCall::Read(1),
                    BackendCall::Close(1),
                    BackendCall::Retry(
                        SubscriptionRecovery::Interrupted { failures: 1 },
                        POLL_INTERVAL
                    ),
                    BackendCall::Connect(2),
                    BackendCall::Watch(2),
                    BackendCall::Read(2),
                ]
            );
        }

        #[tokio::test]
        async fn changes_and_unspecified_redelivery_use_the_original_subscription() {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let delivered = RefCell::new(Vec::new());
            let backend = MockBackend {
                attempts:    VecDeque::from([BackendAttempt::Connected {
                    initial:      AppearanceObservation::Unspecified,
                    observations: vec![
                        Ok(AppearanceObservation::Unspecified),
                        Ok(AppearanceObservation::Selected(Appearance::Light)),
                        Ok(AppearanceObservation::Selected(Appearance::Light)),
                        Ok(AppearanceObservation::Unspecified),
                        Ok(AppearanceObservation::Selected(Appearance::Light)),
                        Ok(AppearanceObservation::Selected(Appearance::Dark)),
                    ],
                    tail:         StreamTail::Pending,
                }]),
                calls:       Arc::clone(&calls),
                connections: 0,
            };
            let mut task = Box::pin(track(
                backend,
                |appearance| delivered.borrow_mut().push(appearance),
                |recovery, delay| {
                    calls
                        .lock()
                        .expect("backend call log")
                        .push(BackendCall::Retry(recovery, delay));
                    future::ready(())
                },
            ));
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert_eq!(
                *delivered.borrow(),
                [Appearance::Light, Appearance::Light, Appearance::Dark]
            );
            assert_eq!(
                *calls.lock().expect("backend call log"),
                [
                    BackendCall::Connect(1),
                    BackendCall::Watch(1),
                    BackendCall::Read(1)
                ]
            );
        }

        #[tokio::test]
        async fn ended_stream_and_owner_restart_each_retry_with_one_new_startup_read() {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let delivered = RefCell::new(Vec::new());
            let backend = MockBackend {
                attempts:    VecDeque::from([
                    connected(
                        AppearanceObservation::Selected(Appearance::Light),
                        StreamTail::Ended,
                    ),
                    BackendAttempt::Connected {
                        initial:      AppearanceObservation::Selected(Appearance::Light),
                        observations: vec![Err(SubscriptionEnd::OwnerChanged)],
                        tail:         StreamTail::Pending,
                    },
                    connected(
                        AppearanceObservation::Selected(Appearance::Dark),
                        StreamTail::Pending,
                    ),
                ]),
                calls:       Arc::clone(&calls),
                connections: 0,
            };
            let mut task = Box::pin(track(
                backend,
                |appearance| delivered.borrow_mut().push(appearance),
                |recovery, delay| {
                    calls
                        .lock()
                        .expect("backend call log")
                        .push(BackendCall::Retry(recovery, delay));
                    future::ready(())
                },
            ));
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert_eq!(*delivered.borrow(), [Appearance::Light, Appearance::Dark]);
            assert_eq!(
                *calls.lock().expect("backend call log"),
                [
                    BackendCall::Connect(1),
                    BackendCall::Watch(1),
                    BackendCall::Read(1),
                    BackendCall::Close(1),
                    BackendCall::Retry(
                        SubscriptionRecovery::Interrupted { failures: 1 },
                        POLL_INTERVAL
                    ),
                    BackendCall::Connect(2),
                    BackendCall::Watch(2),
                    BackendCall::Read(2),
                    BackendCall::Close(2),
                    BackendCall::Retry(
                        SubscriptionRecovery::Interrupted { failures: 1 },
                        POLL_INTERVAL
                    ),
                    BackendCall::Connect(3),
                    BackendCall::Watch(3),
                    BackendCall::Read(3),
                ]
            );
        }

        #[tokio::test]
        async fn never_connected_attempts_back_off_without_subscribing_or_reading() {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let delivered = RefCell::new(Vec::new());
            let backend = MockBackend {
                attempts:    (0..BACKOFF_THRESHOLD)
                    .map(|_| BackendAttempt::Unavailable)
                    .collect(),
                calls:       Arc::clone(&calls),
                connections: 0,
            };
            let mut task = Box::pin(track(
                backend,
                |appearance| delivered.borrow_mut().push(appearance),
                |recovery, delay| {
                    calls
                        .lock()
                        .expect("backend call log")
                        .push(BackendCall::Retry(recovery, delay));
                    async move {
                        if recovery
                            == (SubscriptionRecovery::NeverConnected {
                                failures: BACKOFF_THRESHOLD,
                            })
                        {
                            future::pending::<()>().await;
                        }
                    }
                },
            ));
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert!(delivered.borrow().is_empty());
            let expected: Vec<_> = (1..=BACKOFF_THRESHOLD)
                .enumerate()
                .flat_map(|(index, failures)| {
                    [
                        BackendCall::Connect(index + 1),
                        BackendCall::Retry(
                            SubscriptionRecovery::NeverConnected { failures },
                            if failures == BACKOFF_THRESHOLD {
                                BACKOFF_INTERVAL
                            } else {
                                POLL_INTERVAL
                            },
                        ),
                    ]
                })
                .collect();
            assert_eq!(*calls.lock().expect("backend call log"), expected);
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert_eq!(*calls.lock().expect("backend call log"), expected);
        }

        #[tokio::test]
        async fn recovered_backend_delivers_startup_and_preserves_interrupted_history() {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let delivered = RefCell::new(Vec::new());
            let backend = MockBackend {
                attempts:    VecDeque::from([
                    BackendAttempt::Unavailable,
                    BackendAttempt::Unavailable,
                    connected(AppearanceObservation::Unspecified, StreamTail::Ended),
                    BackendAttempt::Unavailable,
                    BackendAttempt::Unavailable,
                    BackendAttempt::Connected {
                        initial:      AppearanceObservation::Selected(Appearance::Light),
                        observations: vec![
                            Ok(AppearanceObservation::Unspecified),
                            Ok(AppearanceObservation::Selected(Appearance::Light)),
                            Ok(AppearanceObservation::Selected(Appearance::Dark)),
                        ],
                        tail:         StreamTail::Pending,
                    },
                ]),
                calls:       Arc::clone(&calls),
                connections: 0,
            };
            let mut task = Box::pin(track(
                backend,
                |appearance| delivered.borrow_mut().push(appearance),
                |recovery, delay| {
                    calls
                        .lock()
                        .expect("backend call log")
                        .push(BackendCall::Retry(recovery, delay));
                    future::ready(())
                },
            ));
            assert!(futures_lite::future::poll_once(&mut task).await.is_none());
            assert_eq!(
                *delivered.borrow(),
                [Appearance::Light, Appearance::Light, Appearance::Dark]
            );
            assert_eq!(
                *calls.lock().expect("backend call log"),
                [
                    BackendCall::Connect(1),
                    BackendCall::Retry(
                        SubscriptionRecovery::NeverConnected { failures: 1 },
                        POLL_INTERVAL
                    ),
                    BackendCall::Connect(2),
                    BackendCall::Retry(
                        SubscriptionRecovery::NeverConnected { failures: 2 },
                        POLL_INTERVAL
                    ),
                    BackendCall::Connect(3),
                    BackendCall::Watch(3),
                    BackendCall::Read(3),
                    BackendCall::Close(3),
                    BackendCall::Retry(
                        SubscriptionRecovery::Interrupted { failures: 1 },
                        POLL_INTERVAL
                    ),
                    BackendCall::Connect(4),
                    BackendCall::Retry(
                        SubscriptionRecovery::Interrupted { failures: 2 },
                        POLL_INTERVAL
                    ),
                    BackendCall::Connect(5),
                    BackendCall::Retry(
                        SubscriptionRecovery::Interrupted { failures: 3 },
                        POLL_INTERVAL
                    ),
                    BackendCall::Connect(6),
                    BackendCall::Watch(6),
                    BackendCall::Read(6),
                ]
            );
        }
    }
}
