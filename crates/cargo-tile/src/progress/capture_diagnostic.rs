//! Path-qualified capture and identity diagnostics retained by the scanner.

use std::io::Error;
use std::io::ErrorKind;
use std::path::PathBuf;

/// A cloneable scan observation retains the actual I/O kind and diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureFailure {
    /// Allows later rendering to distinguish denial, absence, and other failures.
    pub(crate) kind:    ErrorKind,
    /// The error is observed once, without reopening the path during rendering.
    pub(crate) message: String,
}

impl From<Error> for CaptureFailure {
    fn from(error: Error) -> Self {
        Self {
            kind:    error.kind(),
            message: error.to_string(),
        }
    }
}

/// Retain the pathname at the failed operation, rather than only its root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PathFailure {
    /// Absolute artifact path, or a kernel interface name for boot observations.
    pub(crate) path:    PathBuf,
    /// Original kind and message survive transport to the display thread.
    pub(crate) failure: CaptureFailure,
}

/// Path-qualified observations supplement the count of verified, readable captures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CaptureDiagnostic {
    /// A short directory inventory can omit runs from this scan.
    EnumerationIncomplete(PathBuf),
    /// An inaccessible directory must never read as an empty inventory.
    EnumerationFailed(PathFailure),
    /// An individual registration failed to read; sibling evidence stays available.
    RegistrationUnreadable(PathFailure),
    /// Malformed bytes or mismatched generation cannot establish an association.
    RegistrationInvalid(PathBuf),
    /// A newer framing version stays outside identity verification and cleanup.
    UnsupportedRegistrationVersion {
        /// The retained registration that this reader cannot decode.
        path:        PathBuf,
        /// The framing version declared by the writer.
        encountered: u64,
        /// The newest framing version this reader understands.
        supported:   u64,
    },
    /// A legacy record can annotate a process row but supplies no verifiable identity.
    AnnotationOnly(PathBuf),
    /// Missing identity fields cannot authorize cleanup, even after the pid ends.
    Unverifiable(PathBuf),
    /// The record supplies identity, but this scan could not observe the live process.
    IdentityUnknown(PathBuf),
    /// The cached boot failure prevents checking this record until a restart.
    IdentityBlockedByBoot(PathBuf),
    /// An unpublished artifact stays outside the active capture count.
    Staging(PathBuf),
    /// Retain the exact named log and its I/O failure even without a process row.
    LogUnreadable(PathFailure),
    /// The cached boot read disables verification until a restart retries it.
    BootUnavailable(PathFailure),
}
