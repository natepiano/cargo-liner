//! Proposal-bound answers to reservation overlap conflicts.

mod conflict_authorization;
mod proposal;
mod scope_binding;

pub(crate) use conflict_authorization::ConflictAuthorization;
pub(crate) use proposal::OverlapAuthorizationReason;
pub(crate) use proposal::OverlapAuthorizationRequest;
pub(crate) use proposal::OverlapEscalationPayload;
pub(crate) use proposal::OverlapProposal;
pub(crate) use proposal::OverlapProposalSubmission;
pub(crate) use proposal::OverlapProposalToken;
pub(crate) use proposal::OverlapRequester;
pub(crate) use proposal::PermissiveOverlapAnswer;
pub(crate) use proposal::PermissiveOverlapAuthorizationRequest;
pub(crate) use proposal::RequesterCoordinationIdentity;
#[cfg(test)]
pub(crate) use scope_binding::AuthorizedOverlap;
#[cfg(test)]
pub(crate) use scope_binding::AuthorizedOverlapScopeSet;
pub(crate) use scope_binding::AuthorizedOverlapSet;
pub(crate) use scope_binding::OverlapScopeRevision;
