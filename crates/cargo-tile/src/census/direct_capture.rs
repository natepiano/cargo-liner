//! Verified registrations directly representing cargo invocations.

use std::time::SystemTime;

use super::process_identity::InvocationId;
use super::process_identity::RunId;
use crate::progress::capture::CaptureKey;
use crate::progress::capture::ConfirmedCapture;
use crate::progress::capture_diagnostic::CaptureFailure;
use crate::registration::VerifiedRegistration;

/// Selection and verification are independent facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectedProof {
    /// The selected key has a retained verification proof.
    Confirmed,
    /// The selected reading has no proof and cannot source a row.
    Unconfirmed,
}

/// The nearest registered ancestor retains its root for every annotation lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum NearestRegistration {
    /// No registration exists along the bounded parent walk.
    Unregistered,
    /// Root precedence selects one registration without mixing its sibling roots.
    Registered(CaptureKey),
    /// Competition stops the ancestry walk and retains the competing identities.
    Ambiguous(Vec<CaptureKey>),
}

/// A verified registration established to directly represent this invocation.
/// Only membership resolution can construct this value; an enclosing capture cannot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirectCapture {
    /// The same identity is used by a process row and a registration row.
    pub(super) run_id:       RunId,
    /// Metadata and log lookup use the exact selected publication.
    pub(super) key:          CaptureKey,
    /// The descriptor-bound timestamp survives independently of verification.
    pub(super) modified:     Result<SystemTime, CaptureFailure>,
    /// Keep the existing verifier's proof, including its permitted directory and argv.
    pub(super) registration: VerifiedRegistration,
}

impl From<&ConfirmedCapture> for DirectCapture {
    fn from(confirmed: &ConfirmedCapture) -> Self {
        Self {
            run_id:       RunId::verified(&confirmed.key, &confirmed.registration),
            key:          confirmed.key.clone(),
            modified:     confirmed.modified.clone(),
            registration: confirmed.registration.clone(),
        }
    }
}

impl DirectCapture {
    /// Row construction consumes this proof rather than an enclosing membership.
    pub(crate) const fn registration(&self) -> &VerifiedRegistration { &self.registration }

    /// A change of row source cannot change the registered invocation's identity.
    pub(super) fn invocation_id(&self) -> InvocationId {
        InvocationId::Captured(self.run_id.clone())
    }
}

/// Only the direct arm permits access to verified row metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DirectAssociation {
    /// The registration represents this row's command, rather than an ancestor command.
    Direct(Box<DirectCapture>),
    /// No verified registration directly describes this process.
    None,
}
