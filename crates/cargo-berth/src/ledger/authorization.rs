//! Edit authorization: the run identity a process can prove, and where it came from.

use std::ffi::OsString;
use std::fs;
use std::path::Path;

use super::constants::COORDINATION_RUN_ENVIRONMENT;
use super::constants::COORDINATION_RUN_MARKER_FILE_NAME;
use super::worktree_context::WorktreeContext;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::session;
use crate::session::SessionIdentityLookup;

/// The coordination identity and authorization resolved from one process-context read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedEditAuthorization {
    pub(crate) worktree_id:         WorktreeId,
    pub(crate) coordination_run_id: CoordinationRunId,
    edit_authorization:             EditAuthorization,
}

impl ResolvedEditAuthorization {
    /// Build one coherent result from an edit authorization and its issuing worktree.
    pub(crate) fn for_edit_authorization(
        worktree_id: WorktreeId,
        edit_authorization: EditAuthorization,
    ) -> Self {
        let coordination_run_id = match edit_authorization {
            EditAuthorization::Session {
                coordination_run_id,
                ..
            }
            | EditAuthorization::Environment {
                coordination_run_id,
                ..
            }
            | EditAuthorization::Marker {
                coordination_run_id,
                ..
            } => coordination_run_id,
            EditAuthorization::Unidentified => CoordinationRunId::new(),
        };
        Self {
            worktree_id,
            coordination_run_id,
            edit_authorization,
        }
    }

    /// Return the authorization resolved in the same read as this identity.
    pub(crate) const fn edit_authorization(self) -> EditAuthorization { self.edit_authorization }

    /// Select the run recorded by a command-owned journal mutation.
    pub(crate) const fn journal_mutation_actor_for(
        self,
        coordination_run_id: CoordinationRunId,
    ) -> ResolvedJournalMutationActor {
        ResolvedJournalMutationActor {
            worktree_id: self.worktree_id,
            coordination_run_id,
        }
    }
}

/// The worktree and coordination run recorded by one journal mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedJournalMutationActor {
    pub(crate) worktree_id:         WorktreeId,
    pub(crate) coordination_run_id: CoordinationRunId,
}

/// The coordination identity an edit check can prove for its current process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditAuthorization {
    /// The harness session mapping supplied one active reservation identity.
    Session {
        /// The run recorded on the mapped reservation.
        coordination_run_id: CoordinationRunId,
        /// The reservation selected for this harness session.
        reservation_id:      ReservationId,
        /// The worktree from which this mapping is being used.
        worktree_id:         WorktreeId,
    },
    /// The process environment explicitly supplied the coordination run.
    Environment {
        /// The run named by `CARGO_BERTH_RUN`.
        coordination_run_id: CoordinationRunId,
        /// The worktree the invocation runs in, which decides the same-worktree exemption.
        worktree_id:         WorktreeId,
    },
    /// The worktree marker supplied a run paired with its issued worktree identity.
    Marker {
        /// The run named by the marker.
        coordination_run_id: CoordinationRunId,
        /// The opaque identity from the same administrative directory.
        worktree_id:         WorktreeId,
    },
    /// The caller has no run identity and must not receive a same-worktree exemption.
    Unidentified,
}

/// The coordination run selected from the current process environment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnvironmentCoordinationRunSelection {
    /// The environment did not supply a coordination run.
    NotSupplied,
    /// The supplied value was unusable, so authorization falls back to the marker.
    UnusableFallbackToMarker,
    /// The environment identified one validated coordination run.
    Identified(CoordinationRunId),
}

impl EnvironmentCoordinationRunSelection {
    fn from_current_process() -> Self {
        let Some(value) = std::env::var_os(COORDINATION_RUN_ENVIRONMENT) else {
            return Self::NotSupplied;
        };
        Self::from(value)
    }
}

impl From<OsString> for EnvironmentCoordinationRunSelection {
    fn from(value: OsString) -> Self {
        value
            .into_string()
            .ok()
            .and_then(|value| value.parse().ok())
            .map_or(Self::UnusableFallbackToMarker, Self::Identified)
    }
}

impl EditAuthorization {
    pub(super) fn resolve_for_worktree(
        worktree_context: &WorktreeContext,
        worktree_id: WorktreeId,
    ) -> Self {
        Self::resolve_from_sources(
            session::resolve(&worktree_context.ledger_directory()),
            EnvironmentCoordinationRunSelection::from_current_process(),
            worktree_context.administrative_directory(),
            worktree_id,
        )
    }

    /// Resolve one authorization from the three sources, in a fixed precedence.
    ///
    /// A live harness session mapping first, then `CARGO_BERTH_RUN`, then the worktree marker.
    /// That order is a **precondition of the same-worktree occupancy rule**, not an arbitrary
    /// tie-break. Occupancy is asked only of a run the caller presented, and
    /// [`crate::coordination_identity::PresentedCoordinationRun::from_edit_authorization`]
    /// admits [`Self::Environment`] alone --- so a caller inside a mapped session cannot present
    /// a second coordination run here however it sets the environment, and is never asked the
    /// occupancy question at all. Moving the environment above the session mapping would put
    /// every mapped session one exported variable away from refusing itself in its own
    /// worktree.
    fn resolve_from_sources(
        session_identity: SessionIdentityLookup,
        environment_run_selection: EnvironmentCoordinationRunSelection,
        worktree_administrative_directory: &Path,
        worktree_id: WorktreeId,
    ) -> Self {
        if let SessionIdentityLookup::Mapped(identity) = session_identity {
            return Self::Session {
                coordination_run_id: identity.coordination_run_id(),
                reservation_id: identity.reservation_id(),
                worktree_id,
            };
        }
        if let EnvironmentCoordinationRunSelection::Identified(coordination_run_id) =
            environment_run_selection
        {
            return Self::Environment {
                coordination_run_id,
                worktree_id,
            };
        }
        let marker_path = worktree_administrative_directory.join(COORDINATION_RUN_MARKER_FILE_NAME);
        if let Some(marker) = fs::read_to_string(marker_path)
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .map(|coordination_run_id| Self::Marker {
                coordination_run_id,
                worktree_id,
            })
        {
            return marker;
        }
        Self::Unidentified
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::ffi::OsString;
    use std::fs;

    use tempfile::tempdir;

    use super::EditAuthorization;
    use super::EnvironmentCoordinationRunSelection;
    use crate::coordination_identity::PresentedCoordinationRun;
    use crate::ids::CoordinationRunId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeKind;
    use crate::ledger::constants::COORDINATION_RUN_MARKER_FILE_NAME;
    use crate::ledger::identity;
    use crate::ledger::test_support;
    use crate::ledger::worktree_context::WorktreeContext;
    use crate::session::SessionIdentityLookup;

    /// A live session mapping outranks `CARGO_BERTH_RUN`, so a mapped caller presents nothing.
    ///
    /// This is the precondition the occupancy refusal rests on. Occupancy is a rule between two
    /// runs a caller presented, and only [`EditAuthorization::Environment`] counts as
    /// presenting one. Because the mapping is consulted first, a caller inside a mapped session
    /// resolves to [`EditAuthorization::Session`] no matter what it exports, so it can never
    /// present a second run in a worktree its own session already occupies --- and the refusal
    /// never fires against a session holding the reservation that authorized it.
    ///
    /// Asserted through `PresentedCoordinationRun` rather than on the variant alone, because
    /// that is what the occupancy rule actually consults.
    #[test]
    fn a_live_session_mapping_outranks_a_presented_run() {
        let administrative_directory = tempdir().expect("administrative directory should exist");
        let administrative_worktree =
            identity::worktree_identity(administrative_directory.path(), WorktreeKind::Linked)
                .expect("worktree identity should be created")
                .id;
        let environment_run = CoordinationRunId::new();
        let session_run = CoordinationRunId::new();
        let session_reservation = ReservationId::new();

        let resolved = EditAuthorization::resolve_from_sources(
            SessionIdentityLookup::Mapped(crate::session::SessionReservationIdentity::new(
                session_run,
                session_reservation,
            )),
            EnvironmentCoordinationRunSelection::Identified(environment_run),
            administrative_directory.path(),
            administrative_worktree,
        );

        assert_eq!(
            resolved,
            EditAuthorization::Session {
                coordination_run_id: session_run,
                reservation_id:      session_reservation,
                worktree_id:         administrative_worktree,
            },
            "the environment run must not survive a live session mapping"
        );
        assert!(
            PresentedCoordinationRun::from_edit_authorization(resolved).is_none(),
            "a mapped caller presents no run to the occupancy rule"
        );
        assert_eq!(
            PresentedCoordinationRun::from_edit_authorization(EditAuthorization::Environment {
                coordination_run_id: environment_run,
                worktree_id:         administrative_worktree,
            })
            .map(PresentedCoordinationRun::coordination_run_id),
            Some(environment_run),
            "the same environment run does present one once no mapping outranks it"
        );
    }

    #[test]
    fn edit_authorization_prefers_environment_then_marker_then_unidentified() {
        let administrative_directory = tempdir().expect("administrative directory should exist");
        let environment_run = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b"
            .parse::<CoordinationRunId>()
            .expect("environment run should parse");
        let marker_run = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c"
            .parse::<CoordinationRunId>()
            .expect("marker run should parse");
        let administrative_worktree =
            identity::worktree_identity(administrative_directory.path(), WorktreeKind::Linked)
                .expect("marker worktree identity should be created")
                .id;
        fs::write(
            administrative_directory
                .path()
                .join(COORDINATION_RUN_MARKER_FILE_NAME),
            format!("{marker_run}\n"),
        )
        .expect("coordination marker should write");

        assert_eq!(
            EditAuthorization::resolve_from_sources(
                crate::session::SessionIdentityLookup::Unavailable,
                EnvironmentCoordinationRunSelection::Identified(environment_run),
                administrative_directory.path(),
                administrative_worktree,
            ),
            EditAuthorization::Environment {
                coordination_run_id: environment_run,
                worktree_id:         administrative_worktree,
            }
        );

        let session_run = CoordinationRunId::new();
        let session_reservation = ReservationId::new();
        assert_eq!(
            EditAuthorization::resolve_from_sources(
                crate::session::SessionIdentityLookup::Mapped(
                    crate::session::SessionReservationIdentity::new(
                        session_run,
                        session_reservation,
                    ),
                ),
                EnvironmentCoordinationRunSelection::Identified(environment_run),
                administrative_directory.path(),
                administrative_worktree,
            ),
            EditAuthorization::Session {
                coordination_run_id: session_run,
                reservation_id:      session_reservation,
                worktree_id:         administrative_worktree,
            }
        );

        assert_eq!(
            EditAuthorization::resolve_from_sources(
                crate::session::SessionIdentityLookup::Unavailable,
                EnvironmentCoordinationRunSelection::NotSupplied,
                administrative_directory.path(),
                administrative_worktree,
            ),
            EditAuthorization::Marker {
                coordination_run_id: marker_run,
                worktree_id:         administrative_worktree,
            }
        );
        fs::remove_file(
            administrative_directory
                .path()
                .join(COORDINATION_RUN_MARKER_FILE_NAME),
        )
        .expect("coordination marker should remove");
        assert_eq!(
            EditAuthorization::resolve_from_sources(
                crate::session::SessionIdentityLookup::Unavailable,
                EnvironmentCoordinationRunSelection::Identified(environment_run),
                administrative_directory.path(),
                administrative_worktree,
            ),
            EditAuthorization::Environment {
                coordination_run_id: environment_run,
                worktree_id:         administrative_worktree,
            }
        );
        assert_eq!(
            EditAuthorization::resolve_from_sources(
                crate::session::SessionIdentityLookup::Unavailable,
                EnvironmentCoordinationRunSelection::NotSupplied,
                administrative_directory.path(),
                administrative_worktree,
            ),
            EditAuthorization::Unidentified
        );
        let repository = test_support::scratch_repository();
        let worktree_context = WorktreeContext::discover(repository.path())
            .expect("scratch worktree should be discovered");
        assert!(identity::resolve_identity(&worktree_context).is_ok());
    }

    #[test]
    fn unusable_environment_coordination_run_falls_back_to_marker_then_unidentified() {
        let administrative_directory = tempdir().expect("administrative directory should exist");
        let marker_run = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c"
            .parse::<CoordinationRunId>()
            .expect("marker run should parse");
        let administrative_worktree =
            identity::worktree_identity(administrative_directory.path(), WorktreeKind::Linked)
                .expect("marker worktree identity should resolve")
                .id;
        fs::write(
            administrative_directory
                .path()
                .join(COORDINATION_RUN_MARKER_FILE_NAME),
            format!("{marker_run}\n"),
        )
        .expect("coordination marker should write");
        let unusable_environment_run_selections = ["", "01900a1b-not-a-valid-uuid"]
            .map(|value| EnvironmentCoordinationRunSelection::from(OsString::from(value)));

        for environment_run_selection in unusable_environment_run_selections {
            assert_eq!(
                environment_run_selection,
                EnvironmentCoordinationRunSelection::UnusableFallbackToMarker
            );
            assert_eq!(
                EditAuthorization::resolve_from_sources(
                    crate::session::SessionIdentityLookup::Unavailable,
                    environment_run_selection,
                    administrative_directory.path(),
                    administrative_worktree,
                ),
                EditAuthorization::Marker {
                    coordination_run_id: marker_run,
                    worktree_id:         administrative_worktree,
                }
            );
        }

        fs::remove_file(
            administrative_directory
                .path()
                .join(COORDINATION_RUN_MARKER_FILE_NAME),
        )
        .expect("coordination marker should remove");
        for environment_run_selection in unusable_environment_run_selections {
            assert_eq!(
                EditAuthorization::resolve_from_sources(
                    crate::session::SessionIdentityLookup::Unavailable,
                    environment_run_selection,
                    administrative_directory.path(),
                    administrative_worktree,
                ),
                EditAuthorization::Unidentified
            );
        }
    }
}
