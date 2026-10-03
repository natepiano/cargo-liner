//! Answers to reservation overlap conflicts, each bound to the holder it names.

mod approver;
mod conflict_authorization;
mod constants;
mod request;
mod scope_binding;

pub(crate) use approver::OverlapApprover;
pub(crate) use conflict_authorization::ConflictAuthorization;
pub(crate) use request::DeferAnswerRequest;
pub(crate) use request::OverlapAuthorizationReason;
pub(crate) use request::OverlapAuthorizationRequest;
pub(crate) use request::PermissiveOverlapAnswer;
pub(crate) use request::PermissiveOverlapAuthorizationRequest;
pub(crate) use scope_binding::AuthorizedOverlap;
#[cfg(test)]
pub(crate) use scope_binding::AuthorizedOverlapScopeSet;
pub(crate) use scope_binding::AuthorizedOverlapSet;
pub(crate) use scope_binding::OverlapScopeRevision;
