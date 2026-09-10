//! Parse bounded capture records and require process identity before admitting one.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use crate::birth_stamp::BirthStamp;
use crate::birth_stamp::IdentityEvidence;
use crate::birth_stamp::KernelObservation;
use crate::birth_stamp::Verification;
use crate::constants::CAPTURE_REGISTRATION_BYTES;
use crate::constants::REGISTRATION_FIELD_SEPARATOR;
use crate::constants::REGISTRATION_LEGACY_SEPARATOR;
use crate::constants::REGISTRATION_MAGIC;

/// Parsing preserves compatibility without promoting old display text to proof.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Registration {
    /// Framed fields retain argument boundaries and permit identity verification.
    Versioned(RegistrationCandidate),
    /// Older tab-separated text can only annotate an existing process-table row.
    Legacy(LegacyRegistration),
}

impl Registration {
    /// Reject one malformed record without invalidating readable siblings.
    /// Both phase-2 records and the writer-home extension use the same v2 magic.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        if u64::try_from(bytes.len()).map_err(|_| ParseError::TooLarge)?
            > CAPTURE_REGISTRATION_BYTES
        {
            return Err(ParseError::TooLarge);
        }
        if !bytes.contains(&REGISTRATION_FIELD_SEPARATOR) {
            return parse_legacy(bytes).map(Self::Legacy);
        }
        let framed = bytes
            .strip_suffix(&[REGISTRATION_FIELD_SEPARATOR])
            .ok_or(ParseError::Framing)?;
        let fields: Vec<_> = framed
            .split(|byte| *byte == REGISTRATION_FIELD_SEPARATOR)
            .collect();
        let [
            magic,
            generation,
            boot,
            birth,
            log,
            directory,
            remaining @ ..,
        ] = fields.as_slice()
        else {
            return Err(ParseError::Framing);
        };
        if *magic != REGISTRATION_MAGIC {
            return Err(ParseError::Magic);
        }
        let generation = text(generation)?;
        if !single_basename(generation) {
            return Err(ParseError::Generation);
        }
        let log_basename = text(log)?;
        if !single_basename(log_basename) {
            return Err(ParseError::LogBasename);
        }
        let (writer_home, arguments) = parse_arguments(remaining)?;
        Ok(Self::Versioned(RegistrationCandidate {
            generation: generation.to_owned(),
            identity: text(boot)
                .and_then(|boot| text(birth).map(|birth| BirthStamp::from_fields(boot, birth)))
                .unwrap_or(IdentityEvidence::Unavailable),
            log_basename: log_basename.to_owned(),
            directory: PathBuf::from(OsString::from_vec(directory.to_vec())),
            writer_home,
            arguments,
        }))
    }
}

/// A parsed record remains a candidate until verification compares its kernel stamp.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RegistrationCandidate {
    /// An invocation's opaque generation must match its published filename.
    generation:   String,
    /// Missing or nondecimal fields remain unknown, even if the pid is absent.
    identity:     IdentityEvidence,
    /// The only log this registration may read or authorize for paired removal.
    log_basename: String,
    /// Preserve raw Unix bytes; old collapsed paths have no directory identity.
    directory:    PathBuf,
    /// Display shortening must agree with both the writer and scanner's home.
    writer_home:  WriterHome,
    /// NUL framing recovers each original argument, including empty arguments.
    arguments:    Vec<OsString>,
}

impl RegistrationCandidate {
    /// A generation is a filename association, never process identity by itself.
    pub(crate) fn generation(&self) -> &str { &self.generation }

    /// Open this validated basename relative to the inspected capture root.
    pub(crate) fn log_basename(&self) -> &str { &self.log_basename }

    /// Preserve absolute directory bytes until the display boundary shortens them.
    pub(crate) fn directory(&self) -> &Path { &self.directory }

    /// An old `~` record cannot supply the absolute directory needed for grouping.
    pub(crate) fn directory_identity(&self) -> WorkingDirectoryIdentity {
        WorkingDirectoryIdentity::from(self.directory())
    }

    /// Missing home information prevents a display-only `~` from changing meaning.
    pub(crate) const fn writer_home(&self) -> &WriterHome { &self.writer_home }

    /// Argument boundaries and non-UTF-8 bytes survive parsing unchanged.
    pub(crate) fn arguments(&self) -> &[OsString] { &self.arguments }

    /// Cleanup compares this evidence again after rereading the registration.
    pub(crate) const fn identity(&self) -> &IdentityEvidence { &self.identity }

    /// Only kernel evidence taken for this pid can construct a verified registration.
    /// Test-only observation construction preserves deterministic sweep races.
    pub(crate) fn verify_observation(
        &self,
        pid: u32,
        observation: &KernelObservation,
    ) -> RegistrationVerification {
        match (observation.compare(pid, self.identity()), self.identity()) {
            (Verification::Confirmed, IdentityEvidence::Available(birth)) => {
                RegistrationVerification::Confirmed(VerifiedRegistration {
                    birth: birth.clone(),
                    pid,
                    record: self.clone(),
                })
            },
            (Verification::Ended, _) => RegistrationVerification::Ended,
            (Verification::Unknown | Verification::Confirmed, _) => {
                RegistrationVerification::Unknown
            },
        }
    }
}

/// The writer's home prefix is optional protocol evidence with an explicit absence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WriterHome {
    /// An absolute prefix may shorten display when the scanner uses the same home.
    Known(PathBuf),
    /// Older records and writers without an absolute HOME cannot supply a prefix.
    Unavailable,
}

/// Only absolute paths can identify a directory across different HOME settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorkingDirectoryIdentity {
    /// The raw absolute directory survives independently of its displayed text.
    Absolute(PathBuf),
    /// Empty and previously shortened paths provide display text only.
    Unavailable,
}

impl From<&Path> for WorkingDirectoryIdentity {
    fn from(directory: &Path) -> Self {
        if directory.is_absolute() {
            Self::Absolute(directory.to_path_buf())
        } else {
            Self::Unavailable
        }
    }
}

/// Legacy command text cannot recover argument boundaries or prove its writer lives.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LegacyRegistration {
    /// Retain the original path text, including a possible historical `~` prefix.
    directory: PathBuf,
    /// A single display string must never be treated as an argument vector.
    command:   OsString,
}

#[cfg(test)]
impl LegacyRegistration {
    /// Annotate a process-table row without treating a collapsed path as identity.
    pub(crate) fn directory(&self) -> &Path { &self.directory }

    /// Preserve display text; shell word boundaries are irrecoverable here.
    pub(crate) fn command(&self) -> &OsStr { &self.command }
}

/// Only one outcome contains the type accepted by registration-sourced rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RegistrationVerification {
    /// A completed equality check links the registration to the current process.
    Confirmed(VerifiedRegistration),
    /// Fresh evidence proves this registration's writer ended.
    Ended,
    /// Missing identity or observation permits display annotation but no active row.
    Unknown,
}

/// Proof of a registration's identity at its verification boundary.
/// Fields are private and there is no conversion from filenames or parsed records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedRegistration {
    /// The available comparison stamp that verification actually accepted.
    birth:  BirthStamp,
    /// The process whose current kernel birth matched the record.
    pid:    u32,
    /// Keep the fields bound to that comparison for later membership and row use.
    record: RegistrationCandidate,
}

impl VerifiedRegistration {
    /// Qualification for a generation-qualified invocation identity.
    pub(crate) const fn birth(&self) -> &BirthStamp { &self.birth }

    /// Membership attaches to the shim pid that was actually verified.
    pub(crate) const fn pid(&self) -> u32 { self.pid }

    /// Consumers retain the checked record instead of reconstructing it from a name.
    pub(crate) const fn record(&self) -> &RegistrationCandidate { &self.record }
}

/// Structural failures reject this record alone, preserving process-table rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParseError {
    /// Bound allocations even if a caller bypasses the descriptor read cap.
    TooLarge,
    /// Fields are missing or the final field is not NUL-terminated.
    Framing,
    /// A NUL-framed record uses an unsupported protocol version.
    Magic,
    /// A filename field is not valid UTF-8 text.
    Text,
    /// A generation cannot identify a single published filename.
    Generation,
    /// Logs must be single basenames within the inspected root.
    LogBasename,
    /// The decimal count is invalid or does not equal the actual argument count.
    ArgumentCount,
    /// Unframed text does not contain a directory and command separated by a tab.
    Legacy,
}

/// A slash, dot component, or empty string cannot be a root-relative log basename.
fn single_basename(value: &str) -> bool {
    let path = Path::new(value);
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
        && path.file_name() == Some(OsStr::new(value))
}

/// Identity and filename text must not be silently changed by lossy decoding.
fn text(bytes: &[u8]) -> Result<&str, ParseError> {
    std::str::from_utf8(bytes).map_err(|_| ParseError::Text)
}

/// Old records start this suffix with argc; the extension starts with HOME.
/// An empty or absolute HOME distinguishes the extension without losing old v2.
fn parse_arguments(fields: &[&[u8]]) -> Result<(WriterHome, Vec<OsString>), ParseError> {
    let Some((first, rest)) = fields.split_first() else {
        return Err(ParseError::ArgumentCount);
    };
    let home = PathBuf::from(OsString::from_vec(first.to_vec()));
    let (writer_home, count, arguments) = if first.is_empty() || home.is_absolute() {
        let Some((count, arguments)) = rest.split_first() else {
            return Err(ParseError::ArgumentCount);
        };
        let writer_home = if home.is_absolute() {
            WriterHome::Known(home)
        } else {
            WriterHome::Unavailable
        };
        (writer_home, *count, arguments)
    } else {
        (WriterHome::Unavailable, *first, rest)
    };
    if count.is_empty() || !count.iter().all(u8::is_ascii_digit) {
        return Err(ParseError::ArgumentCount);
    }
    let count: usize = text(count)?
        .parse()
        .map_err(|_| ParseError::ArgumentCount)?;
    if count != arguments.len() {
        return Err(ParseError::ArgumentCount);
    }
    let arguments = arguments
        .iter()
        .map(|argument| OsString::from_vec(argument.to_vec()))
        .collect();
    Ok((writer_home, arguments))
}

/// Legacy parsing retains one uninterpreted command string for annotation only.
fn parse_legacy(bytes: &[u8]) -> Result<LegacyRegistration, ParseError> {
    let separator = bytes
        .iter()
        .position(|byte| *byte == REGISTRATION_LEGACY_SEPARATOR)
        .ok_or(ParseError::Legacy)?;
    let (directory, command) = (&bytes[..separator], &bytes[separator + 1..]);
    if directory.is_empty() || command.is_empty() {
        return Err(ParseError::Legacy);
    }
    Ok(LegacyRegistration {
        directory: PathBuf::from(OsString::from_vec(directory.to_vec())),
        command:   OsString::from_vec(command.to_vec()),
    })
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use super::ParseError;
    use super::Registration;
    use super::RegistrationCandidate;
    use super::RegistrationVerification;
    use super::WorkingDirectoryIdentity;
    use super::WriterHome;
    use crate::birth_stamp::BirthStamp;
    use crate::birth_stamp::IdentityEvidence;
    use crate::birth_stamp::KernelObservation;
    use crate::birth_stamp::Observation;
    use crate::constants::CAPTURE_REGISTRATION_BYTES;
    use crate::constants::REGISTRATION_FIELD_SEPARATOR;
    use crate::constants::REGISTRATION_MAGIC;

    /// Serialize the actual wire framing without relying on Rust string escapes.
    fn framed(fields: &[&[u8]]) -> Vec<u8> {
        fields
            .iter()
            .flat_map(|field| field.iter().copied().chain([REGISTRATION_FIELD_SEPARATOR]))
            .collect()
    }

    /// Supply the common v2 header; each test controls the meaningful suffix.
    fn record(birth: &[u8], log: &[u8], suffix: &[&[u8]]) -> Vec<u8> {
        let mut fields = vec![REGISTRATION_MAGIC, b"calendar-uuid", b"boot", birth, log];
        fields.extend_from_slice(suffix);
        framed(&fields)
    }

    /// Assert that a fixture exercises the versioned branch of the public parser.
    fn versioned(bytes: &[u8]) -> RegistrationCandidate {
        let Registration::Versioned(record) =
            Registration::parse(bytes).expect("valid versioned fixture")
        else {
            panic!("expected versioned registration");
        };
        record
    }

    #[test]
    fn fields_round_trip_raw_paths_home_and_original_arguments() {
        let bytes = record(
            b"101",
            b"exact.log",
            &[
                b"/home/writer/work\twith\nlines",
                b"/home/writer",
                b"4",
                b"build",
                b"",
                b"arg with spaces",
                b"\xff",
            ],
        );
        let record = versioned(&bytes);
        assert_eq!(record.generation(), "calendar-uuid");
        assert_eq!(record.log_basename(), "exact.log");
        assert_eq!(
            record.directory(),
            Path::new("/home/writer/work\twith\nlines")
        );
        assert_eq!(
            record.writer_home(),
            &WriterHome::Known("/home/writer".into())
        );
        assert_eq!(
            record.directory_identity(),
            WorkingDirectoryIdentity::Absolute(record.directory().to_path_buf())
        );
        assert_eq!(
            record
                .arguments()
                .iter()
                .map(|value| value.as_bytes())
                .collect::<Vec<_>>(),
            vec![b"build".as_slice(), b"", b"arg with spaces", b"\xff"]
        );
    }

    #[test]
    fn phase_two_records_have_no_writer_home_or_collapsed_directory_identity() {
        let record = versioned(&record(b"101", b"exact.log", &[b"~/work", b"1", b"build"]));
        assert_eq!(record.writer_home(), &WriterHome::Unavailable);
        assert_eq!(record.directory(), Path::new("~/work"));
        assert_eq!(
            record.directory_identity(),
            WorkingDirectoryIdentity::Unavailable
        );
    }

    #[test]
    fn legacy_command_remains_display_text() {
        let record = Registration::parse(b"~/work\tcargo build --flag with spaces")
            .expect("legacy display record");
        assert!(
            matches!(&record, Registration::Legacy(legacy) if legacy.directory() == Path::new("~/work") && legacy.command() == OsStr::new("cargo build --flag with spaces"))
        );
    }

    #[test]
    fn only_confirmed_comparison_constructs_verified_registration() {
        let record = versioned(&record(b"101", b"exact.log", &[b"/work", b"0"]));
        assert_eq!(
            record.verify_observation(123, &KernelObservation::for_test(123, Observation::Ended)),
            RegistrationVerification::Ended
        );
        assert_eq!(
            record.verify_observation(123, &KernelObservation::for_test(123, Observation::Unknown)),
            RegistrationVerification::Unknown
        );
        let IdentityEvidence::Available(stamp) = BirthStamp::from_fields("boot", "101") else {
            panic!("expected complete identity fields");
        };
        let observation = KernelObservation::for_test(123, Observation::Present(stamp));
        let verified = record.verify_observation(123, &observation);
        assert!(
            matches!(verified, RegistrationVerification::Confirmed(verified) if verified.pid() == 123 && verified.record() == &record)
        );
        assert_eq!(
            record.verify_observation(124, &observation),
            RegistrationVerification::Unknown
        );
    }

    #[test]
    fn another_pids_absence_cannot_authorize_registration_removal() {
        let record = versioned(&record(b"101", b"exact.log", &[b"/work", b"0"]));
        assert_eq!(
            record.verify_observation(123, &KernelObservation::for_test(124, Observation::Ended)),
            RegistrationVerification::Unknown
        );
    }

    #[test]
    fn unavailable_birth_never_constructs_verified_registration() {
        for birth in [
            b"".as_slice(),
            b"Wed Sep 9 20:00:00 2026",
            b"+101",
            b"101 ",
            b"\xff",
        ] {
            let record = versioned(&record(birth, b"exact.log", &[b"/work", b"0"]));
            assert_eq!(record.identity(), &IdentityEvidence::Unavailable);
            assert_eq!(
                record
                    .verify_observation(123, &KernelObservation::for_test(123, Observation::Ended)),
                RegistrationVerification::Unknown
            );
        }
    }

    #[test]
    fn record_equality_detects_replacement_identity_and_fields() {
        let first = Registration::parse(&record(b"101", b"exact.log", &[b"/work", b"0"]))
            .expect("valid first record");
        let replacement = Registration::parse(&record(b"102", b"exact.log", &[b"/work", b"0"]))
            .expect("valid replacement record");
        assert_ne!(first, replacement);
        assert_eq!(first, first.clone());
    }

    #[test]
    fn malformed_counts_and_trailing_fields_are_rejected() {
        for suffix in [
            vec![b"/work".as_slice()],
            vec![b"/work", b"+1", b"build"],
            vec![b"/work", b"0", b"build"],
            vec![b"/work", b"2", b"build"],
            vec![b"/work", b"/home", b"1"],
        ] {
            assert_eq!(
                Registration::parse(&record(b"101", b"exact.log", &suffix)),
                Err(ParseError::ArgumentCount)
            );
        }
    }

    #[test]
    fn log_names_cannot_escape_or_name_a_directory() {
        for log in [
            b"".as_slice(),
            b"/tmp/foreign",
            b"../foreign",
            b"a/b",
            b"a/",
            b".",
            b"..",
            b"./a",
        ] {
            assert_eq!(
                Registration::parse(&record(b"101", log, &[b"/work", b"0"])),
                Err(ParseError::LogBasename)
            );
        }
    }

    #[test]
    fn missing_terminator_unknown_magic_and_empty_records_are_rejected() {
        let mut bytes = record(b"101", b"exact.log", &[b"/work", b"0"]);
        bytes.pop();
        assert_eq!(Registration::parse(&bytes), Err(ParseError::Framing));
        assert_eq!(
            Registration::parse(&framed(&[
                b"different",
                b"generation",
                b"boot",
                b"101",
                b"log",
                b"/work",
                b"0"
            ])),
            Err(ParseError::Magic)
        );
        assert_eq!(Registration::parse(b""), Err(ParseError::Legacy));
    }

    #[test]
    fn parser_enforces_registration_byte_cap() {
        let bytes =
            vec![b'x'; usize::try_from(CAPTURE_REGISTRATION_BYTES).expect("cap fits usize") + 1];
        assert_eq!(Registration::parse(&bytes), Err(ParseError::TooLarge));
    }
}
