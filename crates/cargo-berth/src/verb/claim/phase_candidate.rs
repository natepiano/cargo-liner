//! Eligible phase reservations with truthful session labels.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use crate::ids::ReservationId;
use crate::session::HarnessSessionId;

/// One eligible phase reservation, paired with the sessions that select it.
#[derive(Debug)]
pub(crate) struct PhaseReservationCandidate {
    pub(crate) reservation_id: ReservationId,
    sessions:                  PhaseReservationSessions,
}

/// The session label of an eligible phase, with plurality represented explicitly.
#[derive(Debug)]
enum PhaseReservationSessions {
    IndexUnavailable,
    NoSession,
    One(HarnessSessionId),
    Several {
        first:     HarnessSessionId,
        second:    HarnessSessionId,
        remaining: Vec<HarnessSessionId>,
    },
}

impl PhaseReservationCandidate {
    pub(super) const fn without_session_label(reservation_id: ReservationId) -> Self {
        Self {
            reservation_id,
            sessions: PhaseReservationSessions::IndexUnavailable,
        }
    }

    pub(crate) fn from_sessions(
        reservation_id: ReservationId,
        mut sessions: Vec<HarnessSessionId>,
    ) -> Self {
        sessions.sort();
        sessions.dedup();
        let mut sessions = sessions.into_iter();
        let sessions = match (sessions.next(), sessions.next()) {
            (None, _) => PhaseReservationSessions::NoSession,
            (Some(session), None) => PhaseReservationSessions::One(session),
            (Some(first), Some(second)) => PhaseReservationSessions::Several {
                first,
                second,
                remaining: sessions.collect(),
            },
        };
        Self {
            reservation_id,
            sessions,
        }
    }
}

impl Display for PhaseReservationCandidate {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "`{}`", self.reservation_id)?;
        if matches!(self.sessions, PhaseReservationSessions::IndexUnavailable) {
            return Ok(());
        }
        formatter.write_str(" (")?;
        match &self.sessions {
            PhaseReservationSessions::IndexUnavailable => return Ok(()),
            PhaseReservationSessions::NoSession => formatter.write_str("no session")?,
            PhaseReservationSessions::One(session) => {
                write!(formatter, "session {}", session.as_str())?;
            },
            PhaseReservationSessions::Several {
                first,
                second,
                remaining,
            } => {
                write!(
                    formatter,
                    "sessions {}, {}",
                    first.as_str(),
                    second.as_str()
                )?;
                for session in remaining {
                    write!(formatter, ", {}", session.as_str())?;
                }
            },
        }
        formatter.write_str(")")
    }
}
