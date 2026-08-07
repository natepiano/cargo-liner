//! Frozen build-termination authority, observation, transaction ownership, and lifecycle.

mod authority;
mod lifecycle;
mod observation;
mod transaction;

pub(crate) use authority::BuildTerminationAuthority;
pub(crate) use authority::BuildTerminationAuthorizationConstruction;
pub(crate) use authority::ClassifiedExternalTerminationSupport;
pub(crate) use authority::ClassifiedExternalTerminationSupports;
pub(crate) use authority::ExternalBuildTerminationAuthority;
pub(crate) use authority::OwnedBuildTerminationAuthority;
pub(crate) use authority::OwnedTerminationSupport;
pub(crate) use authority::ScopeTerminationAuthorization;
pub(in crate::build_monitor) use authority::ScopeTerminationAuthorizationCurrency;
pub(crate) use authority::SelectedBuildTerminationAuthorization;
pub(in crate::build_monitor) use authority::SelectedBuildTerminationAuthorizationCurrency;
pub(crate) use authority::SelectedBuildTerminationAvailability;
pub(crate) use lifecycle::BuildTerminationLifecycle;
pub(crate) use lifecycle::BuildTerminationLifecycleRegistry;
pub(crate) use lifecycle::BuildTerminationSessionCompletion;
pub(crate) use lifecycle::BuildTerminationTerminalRecord;
pub(crate) use lifecycle::BuildTerminationTransactionCompletion;
pub(crate) use observation::BuildTerminationObservationDemand;
pub(crate) use observation::BuildTerminationObservationExecution;
pub(crate) use observation::observe_build_termination_demand;
pub(crate) use transaction::BUILD_TERMINATION_TIMEOUT;
pub(crate) use transaction::BuildTerminationCompletionTransition;
pub(crate) use transaction::BuildTerminationDeadline;
pub(crate) use transaction::BuildTerminationState;
pub(crate) use transaction::BuildTerminationSubmission;
pub(crate) use transaction::BuildTerminationSubmissionRefusal;
pub(crate) use transaction::BuildTerminationTransactionId;
