//! Invocation rows, origin clocks, and displayed ancestry.

#[cfg(test)]
#[path = "invocation_tests.rs"]
mod invocation_tests;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use chrono::DateTime;
use chrono::Local;
use sysinfo::Pid;

use super::observation::ProcessField;
use super::observation::ProcessObservation;
use super::observation::WorkingDirectoryObservation;
use super::resource_use;
use super::resource_use::InvocationResourceUse;
use crate::birth_stamp::LifetimeEvidence;
use crate::census::command_text;
use crate::census::command_text::CommandText;
use crate::census::command_text::RowAbsence;
use crate::census::command_text::ScannerHome;
use crate::census::direct_capture::DirectAssociation;
use crate::census::direct_capture::DirectCapture;
use crate::census::invocation_cpu_accounting::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::census::process_identity::CaptureMembership;
use crate::census::process_identity::InvocationId;
use crate::census::process_identity::ProcessIdentity;
use crate::census::process_identity::VisibleParent;
use crate::constants::ARGUMENT_SEPARATOR;
use crate::constants::CARGO_DISPLAY_NAME;
use crate::constants::CARGO_JSON_FORMAT_PREFIX;
use crate::constants::CARGO_MESSAGE_FORMAT_FLAG;
use crate::constants::CARGO_MESSAGE_FORMAT_JSON_PREFIX;
use crate::constants::CARGO_QUIET_FLAGS;
use crate::constants::SECONDS_PER_HOUR;
use crate::constants::SECONDS_PER_MINUTE;
use crate::constants::START_TIME_FORMAT;
use crate::constants::TRANSPARENT_PROCESS_NAMES;
use crate::constants::UNRESOLVED_PATH;
use crate::constants::UNRESOLVED_TIME;
use crate::progress::capture::Capture;
use crate::progress::capture::CaptureKey;
use crate::progress::capture_read::CaptureLookup;
use crate::registration::RegistrationCandidate;
use crate::registration::WorkingDirectoryIdentity;
use crate::registration::WriterHome;
use crate::render::Account;
use crate::render::CaptureContext;

/// A row's capture context states whether registration fields may describe it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RowProvenance {
    /// No verified capture qualifies this row's own working directory; an observed
    /// process owner still names the account it runs under.
    Uncaptured(ProcessOwner),
    /// This invocation owns its selected registration's metadata.
    Direct(CaptureContext),
    /// A nested invocation shares account and capture, retaining its own fields.
    Enclosing(CaptureContext),
}

impl RowProvenance {
    /// Verification came from this root; missing owner observations grant no qualification,
    /// leaving the row uncaptured under its process `owner`.
    pub(super) fn direct(capture: &Capture, key: &CaptureKey, owner: ProcessOwner) -> Self {
        let Some(status) = capture.root_status.get(key.root.0) else {
            return Self::Uncaptured(owner);
        };
        let uid = status.root.uid;
        Self::Direct(CaptureContext {
            root:        key.root,
            incarnation: key.incarnation,
            account:     Account {
                uid,
                name: status.account.clone(),
            },
        })
    }

    /// Enclosing membership qualifies a group without granting registration fields.
    pub(super) fn enclosing(capture: &Capture, key: &CaptureKey, owner: ProcessOwner) -> Self {
        match Self::direct(capture, key, owner) {
            Self::Direct(context) => Self::Enclosing(context),
            provenance => provenance,
        }
    }
}

/// The account an uncaptured invocation's process runs as, named on the scan worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessOwner {
    /// The process table reported this uid; the account table that named this
    /// scan's roots named it too, so a root of the same uid carries the same label.
    Observed(Account),
    /// An unreadable owner qualifies nothing, so the heading names no account.
    Unavailable,
}

impl ProcessOwner {
    /// Name an observed uid from this scan's account table; rendering performs no lookup.
    pub(super) fn observe(uid: ProcessField<u32>, capture: &Capture) -> Self {
        match uid {
            ProcessField::Observed(uid) => Self::Observed(Account {
                uid,
                name: capture.account_name(uid),
            }),
            ProcessField::Unavailable => Self::Unavailable,
        }
    }
}

/// An unavailable registration timestamp sorts after known starts in ascending order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum RunStart {
    /// Seconds since the epoch order both process and registration observations.
    Known(u64),
    /// A failed timestamp read cannot prevent a verified invocation's row.
    Unavailable,
}

/// Only the origin's process or its own publication can establish the start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum OriginStart {
    /// Kernel start and elapsed time belong to the observed origin.
    Process { epoch: u64, elapsed: Duration },
    /// The origin wrote this publication when it started.
    Registration { epoch: u64, elapsed: Duration },
    /// Neither source can establish the origin start.
    Unavailable,
}

impl OriginStart {
    fn registration(direct: &DirectCapture) -> Self {
        let Ok(time) = &direct.modified else {
            return Self::Unavailable;
        };
        let Ok(start) = time.duration_since(UNIX_EPOCH) else {
            return Self::Unavailable;
        };
        Self::Registration {
            epoch:   start.as_secs(),
            elapsed: SystemTime::now().duration_since(*time).unwrap_or_default(),
        }
    }

    /// Select whether the retained clock still owns the elapsed time.
    pub(crate) const fn retains(self, incoming: Self) -> bool {
        match (self, incoming) {
            (Self::Process { epoch, .. }, Self::Process { epoch: current, .. })
            | (Self::Registration { epoch, .. }, Self::Registration { epoch: current, .. })
                if epoch == current =>
            {
                false
            },
            (Self::Process { .. }, _) => true,
            (_, Self::Process { .. }) | (Self::Unavailable, _) => false,
            (Self::Registration { .. }, _) => true,
        }
    }

    /// Retained clocks include the full interval through the observation time.
    pub(crate) const fn advance(self, incoming: Self, elapsed: Duration) -> Self {
        if !self.retains(incoming) {
            return incoming;
        }
        match self {
            Self::Process {
                epoch,
                elapsed: previous,
            } => Self::Process {
                epoch,
                elapsed: previous.saturating_add(elapsed),
            },
            Self::Registration {
                epoch,
                elapsed: previous,
            } => Self::Registration {
                epoch,
                elapsed: previous.saturating_add(elapsed),
            },
            Self::Unavailable => incoming,
        }
    }

    /// Compare display values without allocating duration labels.
    pub(crate) const fn same_duration(self, other: Self) -> bool {
        match (self, other) {
            (
                Self::Process { elapsed: first, .. } | Self::Registration { elapsed: first, .. },
                Self::Process {
                    elapsed: second, ..
                }
                | Self::Registration {
                    elapsed: second, ..
                },
            ) => first.as_secs() == second.as_secs(),
            (Self::Unavailable, Self::Unavailable) => true,
            _ => false,
        }
    }

    /// Derive the sortable start from the origin's clock.
    const fn started(self) -> RunStart {
        match self {
            Self::Process { epoch, .. } | Self::Registration { epoch, .. } => {
                RunStart::Known(epoch)
            },
            Self::Unavailable => RunStart::Unavailable,
        }
    }

    /// Derive the local start label without storing another clock.
    fn start(self) -> String {
        match self.started() {
            RunStart::Known(epoch) => start_label(epoch),
            RunStart::Unavailable => UNRESOLVED_TIME.to_owned(),
        }
    }

    /// Derive the elapsed label from the origin's clock.
    fn duration(self) -> String {
        match self {
            Self::Process { elapsed, .. } | Self::Registration { elapsed, .. } => {
                duration_label(elapsed.as_secs())
            },
            Self::Unavailable => UNRESOLVED_TIME.to_owned(),
        }
    }
}

#[cfg(test)]
mod origin_start_tests {
    use std::time::Duration;

    use super::OriginStart;
    use super::RunStart;

    #[test]
    fn retained_origin_clocks_include_every_fractional_interval() {
        let process = OriginStart::Process {
            epoch:   100,
            elapsed: Duration::ZERO,
        };
        let registration = OriginStart::Registration {
            epoch:   100,
            elapsed: Duration::ZERO,
        };
        let different_process = OriginStart::Process {
            epoch:   103,
            elapsed: Duration::from_secs(50),
        };
        let different_registration = OriginStart::Registration {
            epoch:   103,
            elapsed: Duration::from_secs(50),
        };
        for (mut clock, incoming) in [
            (process, OriginStart::Unavailable),
            (registration, OriginStart::Unavailable),
            (process, different_process),
            (registration, different_registration),
            (process, different_registration),
        ] {
            for _ in 0..10 {
                clock = clock.advance(incoming, Duration::from_millis(100));
            }
            assert_eq!(clock.started(), RunStart::Known(100));
            assert_eq!(clock.duration(), "00:01");
            assert!(matches!(clock,
                OriginStart::Process { elapsed, .. } | OriginStart::Registration { elapsed, .. }
                    if elapsed == Duration::from_secs(1)));
        }
    }

    #[test]
    fn accepted_origin_clocks_replace_elapsed_time_without_adding_an_interval() {
        let registration = OriginStart::Registration {
            epoch:   103,
            elapsed: Duration::from_millis(900),
        };
        let process = OriginStart::Process {
            epoch:   100,
            elapsed: Duration::from_secs(7),
        };
        assert_eq!(
            registration.advance(process, Duration::from_secs(3)),
            process
        );
        let updated_process = OriginStart::Process {
            epoch:   100,
            elapsed: Duration::from_secs(8),
        };
        assert_eq!(
            process.advance(updated_process, Duration::from_secs(3)),
            updated_process
        );
        let updated_registration = OriginStart::Registration {
            epoch:   103,
            elapsed: Duration::from_millis(950),
        };
        assert_eq!(
            registration.advance(updated_registration, Duration::from_secs(3)),
            updated_registration
        );
    }
}

/// Execution evidence for the origin whose process supplies the row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum InvocationExecution {
    /// An ordinary command is its own executor.
    Command,
    /// A registration remains without an observed origin process.
    RegistrationOnly,
    /// A capture entry is present before or after its executor's lifetime.
    AwaitingExecutor(LifetimeEvidence),
    /// A proved forwarding chain carries work under this capture entry.
    Executing {
        /// The origin's lifetime distinguishes reuse of its displayed pid.
        origin:   LifetimeEvidence,
        /// The observed executor proves continuity when an origin becomes verified.
        executor: ProcessIdentity,
    },
}

/// One running cargo invocation with numeric resource contributions.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CargoProcess {
    /// Row source and displayed pid do not change invocation continuity.
    pub(crate) invocation_id:        InvocationId,
    /// Origin lifetime and execution evidence do not change with registration publication.
    pub(crate) execution:            InvocationExecution,
    /// The origin supplies this clock before display formatting.
    pub(crate) origin_start:         OriginStart,
    /// Nested invocations share progress without inheriting registration metadata.
    pub(crate) capture_membership:   CaptureMembership,
    /// Account and root qualification do not grant enclosing rows metadata ownership.
    pub(crate) provenance:           RowProvenance,
    /// Working directory display, shortened only when the home prefix is unambiguous.
    pub(crate) path:                 String,
    /// Raw absolute directory for grouping; display formatting cannot change membership.
    pub(crate) directory_identity:   WorkingDirectoryIdentity,
    /// Process id.
    pub(crate) pid:                  u32,
    /// The nearest ancestor the cell draws: the cargo above this one
    /// where there is one, since that is a row of the same table, and
    /// otherwise the step of the chain block the command was started
    /// from. Never the immediate parent, which is the pty and shim the
    /// capture opened and is drawn nowhere.
    ///
    /// The named parent state distinguishes invocation identity from chain display.
    pub(crate) parent:               VisibleParent,
    /// Disjoint work attributed to this invocation, excluding nested cargo buckets.
    pub(crate) resource_use:         InvocationResourceUse,
    /// Numeric total of every assembled invocation below this row, including itself.
    pub(crate) subtree_resource_use: InvocationResourceUse,
    /// What the command is doing, when a capture of its output is there
    /// to read it from. Read off the nearest capture at or above the
    /// invocation, so a cargo the enclosing run started -- which the
    /// shim declines to capture a second time -- reports the run it is
    /// inside rather than nothing at all.
    pub(crate) state:                CaptureLookup,
    /// Cargo invocations running under this one. A measured zero names
    /// a plain command; an unavailable count cannot establish that it is idle.
    pub(crate) managed:              Measurement<usize>,
    /// Whether another cargo stands between this invocation and the
    /// lead of its group. False for the lead itself and for the
    /// invocations it started directly.
    ///
    /// What the summary keeps out. A command's own cell lists its whole
    /// tree, which is where the tree is worth reading; gathered into
    /// one table with every other command's, the deeper levels bury the
    /// runs they came from -- one `cargo nextest run` puts a `cargo
    /// mend` in the table for every test it runs.
    pub(crate) nested:               bool,
    /// The command line, split so program and arguments style apart.
    pub(crate) command:              CommandText,
}

/// One process standing above a command in the process tree.
///
/// What a cell lists to say where the command came from: a shell, an
/// editor, the agent or script that typed it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Ancestor {
    /// Process id.
    pub(crate) pid:            u32,
    /// What the process is, as [`describe`] reads it.
    pub(crate) command:        CommandText,
    /// Whether the process passed a command through rather than
    /// starting it, per [`is_transparent`]. Whether a cell draws one of
    /// these is settled where the chain is drawn rather than here: the
    /// exception is the foot of the chain, and a driver's cell closes
    /// its own chain with the driver, which moves where the foot is.
    pub(crate) passes_through: bool,
}

impl CargoProcess {
    /// Sort invocations by the origin's start.
    pub(crate) const fn started(&self) -> RunStart { self.origin_start.started() }

    /// Format the origin's local start time.
    pub(crate) fn start(&self) -> String { self.origin_start.start() }

    /// Format the elapsed time from the origin's clock.
    pub(crate) fn duration(&self) -> String { self.origin_start.duration() }
}

/// One command and every cargo invocation running under it.
///
/// A plain `cargo build` is a group of one. A command that drives other
/// cargo commands is one group holding all of them, which is what lets
/// the summary carry the command that was typed with a count beside it
/// instead of the fan-out it became.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CargoGroup {
    /// The outermost invocation: the command that was typed, and the row
    /// the summary carries.
    pub(crate) lead:     CargoProcess,
    /// Everything running under [`lead`](Self::lead), newest first.
    /// Empty for a plain command.
    pub(crate) rest:     Vec<CargoProcess>,
    /// What stands above [`lead`](Self::lead), outermost first, ending
    /// at the process that started it. Empty for a command whose
    /// parents cannot be read.
    pub(crate) ancestry: AncestryWalk<Ancestor>,
}

/// Known ancestors retain whether the bounded parent walk reached its end.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AncestryWalk<T> {
    /// No further readable, non-root, unvisited parent remains.
    Ended(Vec<T>),
    /// Another parent remains beyond the walk's limit.
    LimitCut(Vec<T>),
}

impl<T> AncestryWalk<T> {
    /// Known levels, ordered from the outermost ancestor to the nearest.
    pub(crate) fn as_slice(&self) -> &[T] {
        match self {
            Self::Ended(levels) | Self::LimitCut(levels) => levels,
        }
    }

    /// Unavailable process metadata may omit levels without changing why the walk stopped.
    pub(super) fn filter_map<U>(self, filter: impl FnMut(T) -> Option<U>) -> AncestryWalk<U> {
        match self {
            Self::Ended(levels) => {
                AncestryWalk::Ended(levels.into_iter().filter_map(filter).collect())
            },
            Self::LimitCut(levels) => {
                AncestryWalk::LimitCut(levels.into_iter().filter_map(filter).collect())
            },
        }
    }
}

#[cfg(test)]
impl CargoGroup {
    /// The group's identity, stable for as long as the command runs.
    pub(crate) fn id(&self) -> InvocationId { self.lead.invocation_id.clone() }
}

/// Unknown timestamps remain last; invocation identity breaks ties without hash order.
pub(super) fn newest_first(left: &CargoProcess, right: &CargoProcess) -> Ordering {
    match (left.started(), right.started()) {
        (RunStart::Known(left), RunStart::Known(right)) => right.cmp(&left),
        (RunStart::Known(_), RunStart::Unavailable) => Ordering::Less,
        (RunStart::Unavailable, RunStart::Known(_)) => Ordering::Greater,
        (RunStart::Unavailable, RunStart::Unavailable) => Ordering::Equal,
    }
    .then_with(|| right.invocation_id.cmp(&left.invocation_id))
}

/// Format one cargo process into its table row.
pub(super) fn row(
    process: &ProcessObservation<'_>,
    arguments: &[OsString],
    invocation_id: InvocationId,
    managed: Measurement<usize>,
    home: ScannerHome<'_>,
    resource_use: InvocationResourceUse,
    direct: &DirectAssociation,
) -> Result<CargoProcess, RowAbsence> {
    let (directory_identity, path, command) = row_fields(process.cwd(), arguments, direct, home)?;
    let pid = match &invocation_id {
        InvocationId::Captured(run) => run.shim_pid,
        InvocationId::Process(_) => process.pid().as_u32(),
    };
    let origin_start = if pid == process.pid().as_u32() {
        OriginStart::Process {
            epoch:   process.start_time(),
            elapsed: Duration::from_secs(process.run_time()),
        }
    } else {
        match direct {
            DirectAssociation::Direct(direct) => OriginStart::registration(direct),
            DirectAssociation::None => OriginStart::Unavailable,
        }
    };
    Ok(CargoProcess {
        invocation_id,
        origin_start,
        execution: InvocationExecution::Command,
        capture_membership: CaptureMembership::Outside,
        provenance: RowProvenance::Uncaptured(ProcessOwner::Unavailable),
        path,
        directory_identity,
        pid,
        parent: VisibleParent::None,
        resource_use,
        subtree_resource_use: InvocationResourceUse::unavailable(MeasurementAbsence::Unproven),
        state: CaptureLookup::Unregistered,
        managed,
        nested: false,
        command,
    })
}

/// Resolve each unavailable field independently while observations retain their meaning.
pub(super) fn row_fields(
    directory: WorkingDirectoryObservation<'_>,
    argv: &[OsString],
    direct: &DirectAssociation,
    home: ScannerHome<'_>,
) -> Result<(WorkingDirectoryIdentity, String, CommandText), RowAbsence> {
    let command = match (command_text::command_text(argv, home), direct) {
        (Err(RowAbsence::ArgvUnavailable), DirectAssociation::Direct(direct)) => {
            command_text::command_text(&registration_argv(direct.registration().record()), home)?
        },
        (Ok(_), DirectAssociation::Direct(direct))
            if command_text::cargo_split(argv).is_ok_and(|arguments| {
                forwards_json_capture_arguments(
                    &argv[arguments.start..],
                    direct.registration().record().arguments(),
                )
            }) =>
        {
            // The shim's quiet rewrite changes execution, not the user's command heading.
            command_text::command_text(&registration_argv(direct.registration().record()), home)?
        },
        (command, _) => command?,
    };
    let (identity, display) = match (directory, direct) {
        (WorkingDirectoryObservation::Unavailable, DirectAssociation::Direct(direct)) => {
            let record = direct.registration().record();
            (
                record.directory_identity(),
                registration_directory(record, home),
            )
        },
        (WorkingDirectoryObservation::Observed(path), DirectAssociation::Direct(direct))
            if DirectoryComparison::between(direct.registration().record().directory(), path)
                == DirectoryComparison::Same =>
        {
            (
                direct.registration().record().directory_identity(),
                registration_directory(direct.registration().record(), home),
            )
        },
        (WorkingDirectoryObservation::Observed(path), _) => {
            (path.into(), command_text::home_relative(path, home))
        },
        (WorkingDirectoryObservation::Unavailable, _) => (
            WorkingDirectoryIdentity::Unavailable,
            UNRESOLVED_PATH.to_owned(),
        ),
    };
    Ok((identity, display, command))
}

/// Preserve the writer's argument boundaries when applying the same command policy.
fn registration_argv(record: &RegistrationCandidate) -> Vec<OsString> {
    std::iter::once(OsString::from(CARGO_DISPLAY_NAME))
        .chain(record.arguments().iter().cloned())
        .collect()
}

/// Verified metadata can supply a complete row without a measurable process.
pub(super) fn registration_row(
    direct: &DirectCapture,
    capture: &Capture,
    home: ScannerHome<'_>,
) -> Result<CargoProcess, RowAbsence> {
    let record = direct.registration().record();
    let origin_start = OriginStart::registration(direct);
    Ok(CargoProcess {
        invocation_id: direct.invocation_id(),
        execution: InvocationExecution::RegistrationOnly,
        origin_start,
        capture_membership: CaptureMembership::Outside,
        // No process is observed here, so there is no owner to fall back on.
        provenance: RowProvenance::direct(capture, &direct.key, ProcessOwner::Unavailable),
        path: registration_directory(record, home),
        directory_identity: record.directory_identity(),
        pid: direct.key.pid,
        parent: VisibleParent::None,
        resource_use: InvocationResourceUse::unavailable(MeasurementAbsence::Unproven),
        subtree_resource_use: InvocationResourceUse::unavailable(MeasurementAbsence::Unproven),
        state: capture.read(&direct.key),
        managed: Measurement::Unavailable(MeasurementAbsence::Unproven),
        nested: false,
        command: command_text::command_text(&registration_argv(record), home)?,
    })
}

/// Whether a process only passed a command through rather than being
/// what launched it, per [`TRANSPARENT_PROCESS_NAMES`].
pub(super) fn is_transparent(name: &OsStr) -> bool {
    Path::new(name)
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| TRANSPARENT_PROCESS_NAMES.contains(&name))
}

/// What an ancestor row calls a process: the command line it is
/// running, the executable behind it when its arguments cannot be read,
/// or the name the kernel reports when neither can.
///
/// macOS lets a process read the argument area of processes its own
/// user owns and of nothing else, so a root-owned ancestor -- a login
/// process, a launch agent -- arrives with an empty argv and falls
/// through to one of the other two.
pub(super) fn describe(process: &ProcessObservation<'_>, home: ScannerHome<'_>) -> CommandText {
    let mut words = process
        .cmd()
        .iter()
        .map(|word| command_text::home_relative(Path::new(word), home));
    if let Some(program) = words.next() {
        return CommandText::new(program, words.collect());
    }
    let program = match process.exe() {
        ProcessField::Observed(exe) => command_text::home_relative(exe, home),
        ProcessField::Unavailable => process.name().to_string_lossy().into_owned(),
    };
    CommandText::new(program, Vec::new())
}

/// A tilde is meaningful only when the writer and scanner agree on its prefix.
pub(super) fn registration_directory(
    record: &RegistrationCandidate,
    scanner_home: ScannerHome<'_>,
) -> String {
    match record.writer_home() {
        WriterHome::Known(writer_home)
            if scanner_home == ScannerHome::Known(writer_home.as_path()) =>
        {
            command_text::home_relative(record.directory(), scanner_home)
        },
        WriterHome::Known(_) | WriterHome::Unavailable => record.directory().display().to_string(),
    }
}

/// Local `hh:mm` for a UNIX timestamp in seconds.
fn start_label(epoch_seconds: u64) -> String {
    let seconds = i64::try_from(epoch_seconds).unwrap_or_default();
    DateTime::from_timestamp(seconds, 0).map_or_else(
        || UNRESOLVED_TIME.to_string(),
        |stamp| {
            stamp
                .with_timezone(&Local)
                .format(START_TIME_FORMAT)
                .to_string()
        },
    )
}

/// `mm:ss`, widening to `hh:mm:ss` once a run passes an hour.
fn duration_label(seconds: u64) -> String {
    let hours = seconds / SECONDS_PER_HOUR;
    let minutes = seconds % SECONDS_PER_HOUR / SECONDS_PER_MINUTE;
    let remainder = seconds % SECONDS_PER_MINUTE;
    if hours == 0 {
        format!("{minutes:02}:{remainder:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{remainder:02}")
    }
}

/// Path spelling alone cannot distinguish a directory from one of its aliases.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DirectoryComparison {
    /// Both observations name the same directory.
    Same,
    /// Readable metadata identifies distinct directories.
    Different,
    /// Missing metadata cannot establish whether different spellings agree.
    Unavailable,
}

impl DirectoryComparison {
    /// Equal observed paths need no extra access; aliases compare filesystem identity.
    /// A relative spelling would resolve against this process's own directory, so it
    /// never establishes identity.
    pub(super) fn between(left: &Path, right: &Path) -> Self {
        if left == right {
            return Self::Same;
        }
        if !left.is_absolute() || !right.is_absolute() {
            return Self::Unavailable;
        }
        match (fs::metadata(left), fs::metadata(right)) {
            (Ok(left), Ok(right)) if left.is_dir() && right.is_dir() => {
                if left.dev() == right.dev() && left.ino() == right.ino() {
                    Self::Same
                } else {
                    Self::Different
                }
            },
            _ => Self::Unavailable,
        }
    }
}

/// Match only the shim's complete removal of quiet flags before `--` in JSON mode.
/// All remaining words, including non-UTF-8 bytes and arguments after `--`, stay exact.
pub(super) fn forwards_json_capture_arguments(argv: &[OsString], registered: &[OsString]) -> bool {
    let separator = registered
        .iter()
        .position(|argument| argument == ARGUMENT_SEPARATOR)
        .unwrap_or(registered.len());
    let (cargo_arguments, passthrough) = registered.split_at(separator);
    if !cargo_arguments.iter().any(|argument| {
        argument
            .as_bytes()
            .starts_with(CARGO_MESSAGE_FORMAT_JSON_PREFIX.as_bytes())
    }) && !cargo_arguments.windows(2).any(|pair| {
        pair[0] == CARGO_MESSAGE_FORMAT_FLAG
            && pair[1]
                .as_bytes()
                .starts_with(CARGO_JSON_FORMAT_PREFIX.as_bytes())
    }) {
        return false;
    }
    cargo_arguments
        .iter()
        .filter(|argument| {
            !CARGO_QUIET_FLAGS
                .iter()
                .any(|quiet| argument.as_os_str() == OsStr::new(quiet))
        })
        .chain(passthrough)
        .eq(argv)
}

/// Each pid contributes once to every ancestor's numeric subtree total.
pub(crate) fn subtree_resource_totals(members: &[&CargoProcess]) -> Vec<InvocationResourceUse> {
    let indices: HashMap<_, _> = members
        .iter()
        .enumerate()
        .map(|(index, row)| (&row.invocation_id, index))
        .collect();
    let mut contributions: Vec<_> = members
        .iter()
        .map(|row| vec![(Pid::from_u32(row.pid), row.resource_use.clone())])
        .collect();
    let mut remaining_children = vec![0_usize; members.len()];
    for row in members {
        if let VisibleParent::Invocation { id, .. } = &row.parent
            && let Some(&parent) = indices.get(id)
        {
            remaining_children[parent] += 1;
        }
    }
    let mut ready: Vec<_> = remaining_children
        .iter()
        .enumerate()
        .filter_map(|(index, &children)| (children == 0).then_some(index))
        .collect();
    while let Some(index) = ready.pop() {
        if let VisibleParent::Invocation { id, .. } = &members[index].parent
            && let Some(&parent) = indices.get(id)
        {
            let descendants = contributions[index].clone();
            contributions[parent].extend(descendants);
            remaining_children[parent] -= 1;
            if remaining_children[parent] == 0 {
                ready.push(parent);
            }
        }
    }
    contributions
        .into_iter()
        .zip(remaining_children)
        .map(|(resources, children)| {
            if children > 0 {
                InvocationResourceUse::unavailable(MeasurementAbsence::Unproven)
            } else {
                resource_use::sum_resource_use_once_per_pid(resources)
            }
        })
        .collect()
}
