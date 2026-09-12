//! Descriptor-bound registration samples and filename classification.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use super::capture_diagnostic::CaptureDiagnostic;
use super::capture_diagnostic::CaptureFailure;
use super::capture_diagnostic::PathFailure;
use crate::birth_stamp::IdentityEvidence;
use crate::birth_stamp::KernelObservation;
use crate::constants::CAPTURE_LIVE_RUNS_DIR;
use crate::constants::REGISTRATION_SEPARATOR;
use crate::constants::REGISTRATION_TEMP_SUFFIX;
use crate::constants::SUPPORTED_REGISTRATION_VERSION;
use crate::registration::ParseError;
use crate::registration::Registration;
use crate::registration::RegistrationVerification;
use crate::root_scan::Enumeration;
use crate::root_scan::RootScan;

/// Keep every readable generation and each independent read diagnostic.
pub(super) struct RegisteredRuns {
    /// A pid can retain several ended, unknown, or confirmed generations.
    pub(super) generations: BTreeMap<u32, Vec<RegisteredRun>>,
    /// Failed reads and records without proof remain visible independently of rows.
    pub(super) diagnostics: Vec<CaptureDiagnostic>,
}

/// The scan's original record is the comparison target for pending removal.
pub(super) struct RegisteredRun {
    /// Never infer process identity from this candidate filename.
    pub(super) name:         PathBuf,
    /// Retain parsed contents for exact log association and final rereading.
    pub(super) record:       Registration,
    /// Only the Confirmed variant carries a verified registration.
    pub(super) verification: RegistrationVerification,
    /// Taken from the descriptor that supplied the record bytes.
    pub(super) modified:     Result<SystemTime, CaptureFailure>,
}

/// Filename classification only; generation text never verifies process identity.
pub(super) enum RegistrationName<'name> {
    /// Older shims publish only their pid.
    Legacy(u32),
    /// A published filename also supplies a candidate log generation.
    Generated {
        /// The shim pid preceding the first separator.
        pid:        u32,
        /// The nonempty suffix, accepted without claiming a birth stamp.
        generation: &'name str,
    },
    /// An unpublished record supplies cleanup evidence without capture membership.
    Staging {
        /// The shim pid is still a candidate until its birth is verified.
        pid:        u32,
        /// Strip only the staging suffix when checking the record's generation.
        generation: &'name str,
    },
    /// Unrelated or malformed names do not establish liveness.
    Unrelated,
}

/// Directory enumeration failure is separate from a record that failed after enumeration.
pub(super) fn enumeration_diagnostic(
    outcome: &Enumeration,
    path: PathBuf,
    diagnostics: &mut Vec<CaptureDiagnostic>,
) {
    match outcome {
        Enumeration::Complete => {},
        Enumeration::Incomplete => diagnostics.push(CaptureDiagnostic::EnumerationIncomplete(path)),
        Enumeration::Failed(error) => {
            diagnostics.push(CaptureDiagnostic::EnumerationFailed(PathFailure {
                path,
                failure: std::io::Error::new(error.kind(), error.to_string()).into(),
            }));
        },
    }
}

/// Parse each registration independently and retain every generation's identity result.
pub(super) fn registered_runs(
    scan: &RootScan,
    observe: &impl Fn(u32) -> KernelObservation,
) -> RegisteredRuns {
    let mut diagnostics = Vec::new();
    enumeration_diagnostic(
        scan.registration_outcome(),
        scan.registration_path(),
        &mut diagnostics,
    );
    let mut generations: BTreeMap<u32, Vec<RegisteredRun>> = BTreeMap::new();
    let mut entries: Vec<_> = scan.registration_entries().collect();
    entries.sort_by(|left, right| left.name().cmp(right.name()));
    for entry in entries {
        let path = scan.path().join(CAPTURE_LIVE_RUNS_DIR).join(entry.name());
        let pid = match registration_name(entry.name()) {
            RegistrationName::Legacy(pid) | RegistrationName::Generated { pid, .. } => pid,
            RegistrationName::Staging { pid, .. } => {
                diagnostics.push(CaptureDiagnostic::Staging(path.clone()));
                pid
            },
            RegistrationName::Unrelated => continue,
        };
        let observation = match entry.read_registration() {
            Ok(observation) => observation,
            Err(error) => {
                diagnostics.push(CaptureDiagnostic::RegistrationUnreadable(PathFailure {
                    path,
                    failure: error.into(),
                }));
                continue;
            },
        };
        let record = match Registration::parse(&observation.bytes) {
            Ok(record) => record,
            Err(ParseError::UnsupportedVersion { encountered }) => {
                diagnostics.push(CaptureDiagnostic::UnsupportedRegistrationVersion {
                    path,
                    encountered,
                    supported: SUPPORTED_REGISTRATION_VERSION,
                });
                continue;
            },
            Err(_) => {
                diagnostics.push(CaptureDiagnostic::RegistrationInvalid(path));
                continue;
            },
        };
        let verification = match (&record, registration_name(entry.name())) {
            (
                Registration::Versioned(record),
                RegistrationName::Generated { generation, .. }
                | RegistrationName::Staging { generation, .. },
            ) if record.generation() == generation => record.verify_observation(pid, &observe(pid)),
            (Registration::Legacy(_), _) => RegistrationVerification::Unknown,
            _ => {
                diagnostics.push(CaptureDiagnostic::RegistrationInvalid(path));
                continue;
            },
        };
        if !matches!(
            registration_name(entry.name()),
            RegistrationName::Staging { .. }
        ) && matches!(verification, RegistrationVerification::Unknown)
        {
            diagnostics.push(match &record {
                Registration::Legacy(_) => CaptureDiagnostic::AnnotationOnly(path),
                Registration::Versioned(record)
                    if matches!(record.identity(), IdentityEvidence::Unavailable) =>
                {
                    CaptureDiagnostic::Unverifiable(path)
                },
                Registration::Versioned(_) => CaptureDiagnostic::IdentityUnknown(path),
            });
        }
        generations.entry(pid).or_default().push(RegisteredRun {
            name: entry.name().to_owned(),
            record,
            verification,
            modified: observation
                .metadata
                .modified()
                .map_err(CaptureFailure::from),
        });
    }
    RegisteredRuns {
        generations,
        diagnostics,
    }
}

/// Accept legacy pids and any nonempty generation suffix without interpreting
/// the suffix as process identity. Staging names supply cleanup candidates only.
pub(super) fn registration_name(path: &Path) -> RegistrationName<'_> {
    let Some(name) = path.to_str() else {
        return RegistrationName::Unrelated;
    };
    let published = name.strip_suffix(REGISTRATION_TEMP_SUFFIX).unwrap_or(name);
    let (pid, generation) = published
        .split_once(REGISTRATION_SEPARATOR)
        .unwrap_or((published, ""));
    if !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return RegistrationName::Unrelated;
    }
    let Ok(pid) = pid.parse() else {
        return RegistrationName::Unrelated;
    };
    if !generation.is_empty() {
        if published == name {
            RegistrationName::Generated { pid, generation }
        } else {
            RegistrationName::Staging { pid, generation }
        }
    } else if published != name || published.contains(REGISTRATION_SEPARATOR) {
        RegistrationName::Unrelated
    } else {
        RegistrationName::Legacy(pid)
    }
}
