use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::Mutex;
use std::time::Instant;

use zbus::blocking::Connection;

use super::constants::DESKTOP_RETRY_INTERVAL;

/// The shared session-bus connection used by the capture and position workers.
static SESSION_CONNECTION: Mutex<SessionConnection<Connection>> =
    Mutex::new(SessionConnection::Unconnected);

/// Access to the held session connection, cloned without opening another socket.
pub(super) enum SessionBus<C = Connection> {
    /// The bus is available for a desktop query.
    Connected(C),
    /// No usable connection exists yet, or the previously held connection closed.
    Unavailable(ConnectionFailure),
}

/// Why the desktop session bus cannot be accessed.
#[derive(Clone, Debug)]
pub(super) enum ConnectionFailure {
    /// The session bus has never accepted this process's connection.
    NeverConnected(String),
    /// A previously working connection closed and reconnection failed.
    Disconnected(String),
    /// Another worker panicked while accessing the connection state.
    StatePoisoned,
}

/// Lifetime and retry deadline of the shared desktop session connection.
enum SessionConnection<C> {
    /// No connection attempt has been made.
    Unconnected,
    /// A single connection serves all desktop workers.
    Connected(C),
    /// An unavailable bus is retried independently of the capture cadence.
    Retrying {
        /// Whether the bus has ever connected and the latest error.
        failure:  ConnectionFailure,
        /// Earliest next attempt.
        retry_at: Instant,
    },
}

impl<C: Clone> SessionConnection<C> {
    /// Reuse a live handle, or reconnect once the retry deadline permits it.
    fn access(
        &mut self,
        now: Instant,
        is_closed: impl FnOnce(&C) -> bool,
        connect: impl FnOnce() -> Result<C, String>,
    ) -> SessionBus<C> {
        match self {
            Self::Connected(connection) if !is_closed(connection) => {
                return SessionBus::Connected(connection.clone());
            },
            Self::Retrying { failure, retry_at } if now < *retry_at => {
                return SessionBus::Unavailable(failure.clone());
            },
            _ => {},
        }
        match connect() {
            Ok(connection) => {
                *self = Self::Connected(connection.clone());
                SessionBus::Connected(connection)
            },
            Err(error) => {
                let failure = match self {
                    Self::Unconnected
                    | Self::Retrying {
                        failure: ConnectionFailure::NeverConnected(_),
                        ..
                    } => ConnectionFailure::NeverConnected(error),
                    Self::Connected(_) | Self::Retrying { .. } => {
                        ConnectionFailure::Disconnected(error)
                    },
                };
                *self = Self::Retrying {
                    failure:  failure.clone(),
                    retry_at: now + DESKTOP_RETRY_INTERVAL,
                };
                SessionBus::Unavailable(failure)
            },
        }
    }
}

impl Display for ConnectionFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeverConnected(error) => write!(formatter, "session bus unavailable: {error}"),
            Self::Disconnected(error) => write!(formatter, "session bus disconnected: {error}"),
            Self::StatePoisoned => formatter.write_str("session connection state poisoned"),
        }
    }
}

/// The process-wide connection to the desktop session bus.
pub(super) fn session_connection() -> SessionBus {
    let Ok(mut connection) = SESSION_CONNECTION.lock() else {
        return SessionBus::Unavailable(ConnectionFailure::StatePoisoned);
    };
    connection.access(Instant::now(), Connection::is_closed, || {
        Connection::session().map_err(|error| error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn session_bus_unavailable_retries_only_after_deadline() {
        let mut connection = SessionConnection::<u32>::Unconnected;
        let now = Instant::now();
        let attempts = Cell::new(0);
        let connect = || {
            attempts.set(attempts.get() + 1);
            Err("bus absent".to_owned())
        };
        assert!(
            matches!(connection.access(now, |_| false, connect), SessionBus::Unavailable(ConnectionFailure::NeverConnected(error)) if error == "bus absent")
        );
        assert!(matches!(
            connection.access(now, |_| false, connect),
            SessionBus::Unavailable(ConnectionFailure::NeverConnected(_))
        ));
        assert_eq!(attempts.get(), 1);
        assert!(matches!(
            connection.access(now + DESKTOP_RETRY_INTERVAL, |_| false, connect),
            SessionBus::Unavailable(ConnectionFailure::NeverConnected(_))
        ));
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn session_bus_recovered_connection_is_retained() {
        let mut connection = SessionConnection::<u32>::Unconnected;
        let now = Instant::now();
        let attempts = Cell::new(0);
        assert!(matches!(
            connection.access(
                now,
                |_| false,
                || {
                    attempts.set(attempts.get() + 1);
                    Err("bus absent".to_owned())
                }
            ),
            SessionBus::Unavailable(ConnectionFailure::NeverConnected(_))
        ));
        let connect = || {
            attempts.set(attempts.get() + 1);
            Ok(7)
        };
        for _ in 0..60 {
            assert!(matches!(
                connection.access(now + DESKTOP_RETRY_INTERVAL / 2, |_| false, connect),
                SessionBus::Unavailable(ConnectionFailure::NeverConnected(_))
            ));
        }
        assert_eq!(attempts.get(), 1);
        let recovered_at = now + DESKTOP_RETRY_INTERVAL;
        assert!(matches!(
            connection.access(recovered_at, |_| false, connect),
            SessionBus::Connected(7)
        ));
        for _ in 0..60 {
            assert!(matches!(
                connection.access(recovered_at, |_| false, connect),
                SessionBus::Connected(7)
            ));
        }
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn session_bus_closed_connection_reports_disconnection_and_recovers() {
        let mut connection = SessionConnection::Connected(7);
        let now = Instant::now();
        let attempts = Cell::new(0);
        assert!(matches!(connection.access(now, |_| true, || {
            attempts.set(attempts.get() + 1);
            Err("bus stopped".to_owned())
        }), SessionBus::Unavailable(ConnectionFailure::Disconnected(error)) if error == "bus stopped"));
        let connect = || {
            attempts.set(attempts.get() + 1);
            Ok(8)
        };
        for _ in 0..60 {
            assert!(matches!(
                connection.access(now + DESKTOP_RETRY_INTERVAL / 2, |_| false, connect),
                SessionBus::Unavailable(ConnectionFailure::Disconnected(_))
            ));
        }
        assert_eq!(attempts.get(), 1);
        assert!(matches!(
            connection.access(now + DESKTOP_RETRY_INTERVAL, |_| false, connect),
            SessionBus::Connected(8)
        ));
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn session_bus_closed_connection_can_reconnect_immediately() {
        let mut connection = SessionConnection::Connected(7);
        assert!(matches!(
            connection.access(Instant::now(), |_| true, || Ok(8)),
            SessionBus::Connected(8)
        ));
    }
}
