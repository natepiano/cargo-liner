//! Atomic reservation acquisition.

mod acquisition;
mod authorization;
mod constants;
mod failure;
mod incomplete_answer;
mod phase_candidate;
mod recovery_command;
mod repository_facts;
mod request;

pub(super) use acquisition::CheckAcquisitionDecision;
pub(crate) use acquisition::CheckReservationSelection;
pub(crate) use acquisition::ClaimCapacityRecovery;
pub(crate) use acquisition::FirstTouchClaimExecution;
pub(crate) use acquisition::FirstTouchClaimRequest;
pub(crate) use acquisition::FirstTouchConflictHandling;
pub(crate) use acquisition::FirstTouchConflictOutcome;
pub(crate) use acquisition::ReservationAcquisition;
pub(crate) use acquisition::ReservationAcquisitionKind;
pub(crate) use acquisition::SiblingHeldScopes;
pub(crate) use acquisition::acquire_first_touch;
pub(super) use acquisition::acquire_first_touch_for_check;
pub(crate) use acquisition::execute;
pub(super) use acquisition::selected_reservation_check_acquisition;
pub(crate) use failure::ClaimError;
pub(crate) use phase_candidate::PhaseReservationCandidate;
pub(crate) use recovery_command::ClaimRecoveryCommand;
pub(crate) use recovery_command::ClaimRecoverySelector;
pub(super) use repository_facts::read_reference;
pub(crate) use repository_facts::reference_tip_from_files;
pub(crate) use request::ClaimAnswerPolicy;
pub(crate) use request::ClaimCoordinationRunSelection;
pub(crate) use request::ClaimRequest;
pub(crate) use request::ClaimReservationSelection;
pub(crate) use request::PhaseStartSelection;
