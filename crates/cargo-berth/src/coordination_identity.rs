//! Coordination-identity validation and executable recovery instructions.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::EditAuthorization;
use crate::ledger::ResolvedEditAuthorization;
use crate::ledger::WorktreeContext;
use crate::reservation::RetainedReservationSet;
use crate::reservation::WorktreeOccupancy;

/// Whether a caller presented the coordination identity a claim was recorded under.
///
/// The same-worktree occupancy refusal is a rule between two *presented* coordination runs.
/// An identity this process created for itself, because nothing identified the caller, is not
/// a coordination run and never refuses anyone.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CoordinationIdentityProvenance {
    /// An argument, environment value, or honored marker identified the claimant.
    Presented,
    /// Nothing identified the claimant, so this process issued an identity to stand in for it.
    NotPresented,
}

/// A coordination run whose identity the caller presented, and the acting side of occupancy.
///
/// The occupancy rule holds between two coordination runs that *both* presented an identity.
/// The holder's half is read from the record it was claimed under, as
/// [`CoordinationIdentityProvenance`]. The acting half has no record to read from, so it is
/// carried in the type instead: the field is private and the only two constructors each name
/// the one place a caller can present a run --- the `--run` argument
/// ([`Self::from_run_argument`]) and `CARGO_BERTH_RUN`
/// ([`Self::from_edit_authorization`], which yields `Some` for
/// [`EditAuthorization::Environment`] and nothing else).
///
/// That is the whole point of the type. A run read off a holder record, off a session
/// mapping, off a worktree marker, or issued by this process for an unidentified caller is a
/// bare [`CoordinationRunId`], and no bare id converts. So a fourth site that wants to ask the
/// occupancy question cannot ask it one-sidedly: it has to obtain the acting side through a
/// constructor that states where the identity came from, and the three that exist today ---
/// `ClaimRunValidation::validate`, `DriftRunValidation::authorize_scope_acquisition`, and
/// `check::validate_edit_worktree_occupancy` --- each do exactly that. Before this type the
/// guard was a variant match repeated at each site with a comment asking the next author to
/// repeat it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PresentedCoordinationRun(CoordinationRunId);

impl PresentedCoordinationRun {
    /// Accept the run named by the `--run` argument.
    ///
    /// Reached from one place, the claim argument conversion in [`crate::cli`]. A `--run` on
    /// the command line is a caller presenting an identity in the most literal sense the
    /// engine has.
    pub(crate) const fn from_run_argument(coordination_run_id: CoordinationRunId) -> Self {
        Self(coordination_run_id)
    }

    /// Accept the run named by `CARGO_BERTH_RUN`, and refuse every other authorization source.
    ///
    /// A session mapping and a worktree marker each require an active reservation of their own
    /// run in this worktree, which this same rule stops a second run from ever acquiring, and
    /// an unidentified caller presented nothing at all --- refusing it would refuse the
    /// engine's own markerless post-commit work. All three answer `None`, and a caller that
    /// gets `None` has its answer: there is no occupancy question to ask.
    pub(crate) const fn from_edit_authorization(
        edit_authorization: EditAuthorization,
    ) -> Option<Self> {
        match edit_authorization {
            EditAuthorization::Environment {
                coordination_run_id,
                ..
            } => Some(Self(coordination_run_id)),
            EditAuthorization::Session { .. }
            | EditAuthorization::Marker { .. }
            | EditAuthorization::Unidentified => None,
        }
    }

    /// Return the run itself, for recording it and for comparing it against a holder's.
    ///
    /// Reading is unguarded on purpose: the invariant is about where a presented run comes
    /// from, not about what may be done with one once a constructor has admitted it.
    pub(crate) const fn coordination_run_id(self) -> CoordinationRunId { self.0 }
}

/// A complete process argument vector that always contains an executable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryCommandLine(Vec<OsString>);

impl RecoveryCommandLine {
    pub(crate) fn current_process() -> Self {
        let arguments = std::env::args_os().collect::<Vec<_>>();
        Self::try_from(arguments).unwrap_or_else(|_| Self(vec![OsString::from("cargo-berth")]))
    }

    fn runnable_arguments(
        &self,
    ) -> Result<RunnableRecoveryCommandLine, RecoveryCommandContainsNonTextArgument> {
        let arguments = self
            .0
            .iter()
            .map(|argument| {
                argument
                    .to_str()
                    .map(str::to_owned)
                    .ok_or(RecoveryCommandContainsNonTextArgument)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RunnableRecoveryCommandLine(arguments))
    }
}

impl TryFrom<Vec<OsString>> for RecoveryCommandLine {
    type Error = EmptyRecoveryCommandLine;

    fn try_from(arguments: Vec<OsString>) -> Result<Self, Self::Error> {
        if arguments.is_empty() {
            Err(EmptyRecoveryCommandLine)
        } else {
            Ok(Self(arguments))
        }
    }
}

/// A complete recovery argv whose arguments are executable from the JSON contract.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq)]
#[schemars(rename = "runnable_recovery_argv")]
#[schemars(transparent)]
pub(crate) struct RunnableRecoveryCommandLine(#[schemars(length(min = 1))] Vec<String>);

impl RunnableRecoveryCommandLine {
    const BOARD_ARGUMENTS: [&str; 3] = ["cargo-berth", "board", "--json"];
    const CLEAR_SESSION_ARGUMENTS: [&str; 4] =
        ["cargo-berth", "identity", "clear-session", "--json"];

    pub(crate) fn board() -> Self { Self::from_static(Self::BOARD_ARGUMENTS) }

    pub(crate) fn clear_session_mapping() -> Self {
        Self::from_static(Self::CLEAR_SESSION_ARGUMENTS)
    }

    /// The command that releases one named reservation from the worktree holding it.
    ///
    /// `release` checkpoints an `Active` reservation into `Outstanding`, and occupancy is an
    /// `Active`-only rule, so running this in the incumbent's own worktree ends the occupancy
    /// the refusal named. It is built from the incumbent's reservation id rather than taken
    /// from a recovery-command set because the same argv is executable from an ordinary
    /// command and from a git gate alike.
    fn release_reservation(reservation_id: ReservationId) -> Self {
        Self(vec![
            "cargo-berth".to_owned(),
            "release".to_owned(),
            reservation_id.to_string(),
            "--json".to_owned(),
        ])
    }

    fn from_static<const ARGUMENT_COUNT: usize>(arguments: [&str; ARGUMENT_COUNT]) -> Self {
        const { assert!(ARGUMENT_COUNT > 0) }
        Self(arguments.into_iter().map(str::to_owned).collect())
    }

    fn render(&self) -> String {
        self.0
            .iter()
            .map(|argument| shell_quote(argument))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl TryFrom<Vec<String>> for RunnableRecoveryCommandLine {
    type Error = EmptyRecoveryCommandLine;

    fn try_from(arguments: Vec<String>) -> Result<Self, Self::Error> {
        if arguments.is_empty() {
            Err(EmptyRecoveryCommandLine)
        } else {
            Ok(Self(arguments))
        }
    }
}

impl Serialize for RunnableRecoveryCommandLine {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RunnableRecoveryCommandLine {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let arguments = Vec::<String>::deserialize(deserializer)?;
        Self::try_from(arguments).map_err(serde::de::Error::custom)
    }
}

/// A process command cannot be published because one argument is not text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RecoveryCommandContainsNonTextArgument;

/// A recovery command line omitted its executable.
#[derive(Debug)]
pub(crate) struct EmptyRecoveryCommandLine;

impl Display for EmptyRecoveryCommandLine {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a recovery command line must contain an executable")
    }
}

impl Error for EmptyRecoveryCommandLine {}

/// The identity facts and executable recovery policy for one validation.
#[derive(Clone)]
pub(crate) struct CoordinationIdentityValidationContext {
    resolved_edit_authorization: ResolvedEditAuthorization,
    worktree_context:            WorktreeContext,
    recovery_commands:           CoordinationIdentityRecoveryCommands,
}

/// Commands that remain executable after an ordinary command or git hook is rejected.
#[derive(Clone)]
enum CoordinationIdentityRecoveryCommands {
    /// The user-facing command may be repeated in another worktree.
    UserCommand(RecoveryCommandLine),
    /// Git supplied transaction records on stdin, so only standalone repairs are replayable.
    GitGate {
        clear_session_mapping: RunnableRecoveryCommandLine,
        reconcile_marker:      RunnableRecoveryCommandLine,
    },
}

impl CoordinationIdentityValidationContext {
    /// Validate an ordinary command whose complete argv can be repeated.
    pub(crate) fn for_user_command(
        resolved_edit_authorization: ResolvedEditAuthorization,
        worktree_context: &WorktreeContext,
        recovery_command_line: &RecoveryCommandLine,
    ) -> Self {
        Self {
            resolved_edit_authorization,
            worktree_context: worktree_context.clone(),
            recovery_commands: CoordinationIdentityRecoveryCommands::UserCommand(
                recovery_command_line.clone(),
            ),
        }
    }

    /// Validate a git hook using standalone repairs instead of git's private stdin protocol.
    pub(crate) fn for_git_gate(
        resolved_edit_authorization: ResolvedEditAuthorization,
        worktree_context: &WorktreeContext,
        clear_session_mapping: RunnableRecoveryCommandLine,
        reconcile_marker: RunnableRecoveryCommandLine,
    ) -> Self {
        Self {
            resolved_edit_authorization,
            worktree_context: worktree_context.clone(),
            recovery_commands: CoordinationIdentityRecoveryCommands::GitGate {
                clear_session_mapping,
                reconcile_marker,
            },
        }
    }

    /// Return the identity and authorization resolved from this context's worktree.
    pub(crate) const fn resolved_edit_authorization(&self) -> ResolvedEditAuthorization {
        self.resolved_edit_authorization
    }
}

/// One executable response to a rejected coordination identity.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "coordination_identity_recovery_action")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CoordinationIdentityRecoveryAction {
    /// Remove only the harness-session mapping supplied to this process.
    ClearSessionMapping {
        /// The complete recovery command.
        argv: RunnableRecoveryCommandLine,
        /// The canonical directory in which to run the command.
        cwd:  CanonicalWorktreeRoot,
    },
    /// Reconcile retained state and remove an inactive worktree marker.
    ReconcileAndSweepMarker {
        /// The complete recovery command.
        argv: RunnableRecoveryCommandLine,
        /// The canonical directory in which to run the command.
        cwd:  CanonicalWorktreeRoot,
    },
    /// Repeat the original command from the reservation holder's checkout.
    RerunFromHoldingWorktree {
        /// The complete original command.
        argv: RunnableRecoveryCommandLine,
        /// The canonical holder root in which to run the command.
        cwd:  CanonicalWorktreeRoot,
    },
    /// Release the incumbent reservation occupying the issuing worktree.
    ReleaseIncumbentReservation {
        /// The complete recovery command.
        argv: RunnableRecoveryCommandLine,
        /// The canonical incumbent worktree in which to run the command.
        cwd:  CanonicalWorktreeRoot,
    },
    /// Remove the foreign session mapping before starting independent work here.
    ClaimSeparatelyHere {
        /// The complete clear-session command.
        argv: RunnableRecoveryCommandLine,
        /// The canonical issuing root in which to run the command.
        cwd:  CanonicalWorktreeRoot,
    },
}

impl Display for CoordinationIdentityRecoveryAction {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let (argv, cwd) = match self {
            Self::ClearSessionMapping { argv, cwd }
            | Self::ReconcileAndSweepMarker { argv, cwd }
            | Self::ReleaseIncumbentReservation { argv, cwd }
            | Self::RerunFromHoldingWorktree { argv, cwd }
            | Self::ClaimSeparatelyHere { argv, cwd } => (argv, cwd),
        };
        write!(
            formatter,
            "cd {} && {}",
            shell_quote(&cwd.to_string()),
            argv.render()
        )
    }
}

/// The non-empty executable recovery choices for one identity rejection.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq)]
#[schemars(rename = "coordination_identity_recovery_actions")]
#[schemars(transparent)]
pub(crate) struct CoordinationIdentityRecoveryActions(
    #[schemars(length(min = 1))] Vec<CoordinationIdentityRecoveryAction>,
);

impl CoordinationIdentityRecoveryActions {
    fn one(action: CoordinationIdentityRecoveryAction) -> Self { Self(vec![action]) }

    fn two(
        first: CoordinationIdentityRecoveryAction,
        second: CoordinationIdentityRecoveryAction,
    ) -> Self {
        Self(vec![first, second])
    }

    fn render(&self) -> String {
        self.0
            .iter()
            .map(ToString::to_string)
            .map(|action| format!("`{action}`"))
            .collect::<Vec<_>>()
            .join(" or ")
    }

    fn original_command_recovery(&self) -> OriginalCommandRecovery {
        self.0
            .iter()
            .find_map(|action| match action {
                CoordinationIdentityRecoveryAction::RerunFromHoldingWorktree { argv, .. } => {
                    Some(OriginalCommandRecovery::Runnable(argv.render()))
                },
                CoordinationIdentityRecoveryAction::ClaimSeparatelyHere { .. } => {
                    Some(OriginalCommandRecovery::ContainsNonTextArgument)
                },
                CoordinationIdentityRecoveryAction::ClearSessionMapping { .. }
                | CoordinationIdentityRecoveryAction::ReconcileAndSweepMarker { .. }
                | CoordinationIdentityRecoveryAction::ReleaseIncumbentReservation { .. } => None,
            })
            .unwrap_or(OriginalCommandRecovery::GitPrivateTransaction)
    }
}

enum OriginalCommandRecovery {
    Runnable(String),
    ContainsNonTextArgument,
    GitPrivateTransaction,
}

impl TryFrom<Vec<CoordinationIdentityRecoveryAction>> for CoordinationIdentityRecoveryActions {
    type Error = EmptyCoordinationIdentityRecoveryActions;

    fn try_from(actions: Vec<CoordinationIdentityRecoveryAction>) -> Result<Self, Self::Error> {
        if actions.is_empty() {
            Err(EmptyCoordinationIdentityRecoveryActions)
        } else {
            Ok(Self(actions))
        }
    }
}

impl Serialize for CoordinationIdentityRecoveryActions {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CoordinationIdentityRecoveryActions {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let actions = Vec::<CoordinationIdentityRecoveryAction>::deserialize(deserializer)?;
        Self::try_from(actions).map_err(serde::de::Error::custom)
    }
}

/// A coordination-identity rejection omitted every executable recovery choice.
#[derive(Debug)]
pub(crate) struct EmptyCoordinationIdentityRecoveryActions;

impl Display for EmptyCoordinationIdentityRecoveryActions {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a coordination identity rejection requires a recovery action")
    }
}

impl Error for EmptyCoordinationIdentityRecoveryActions {}

/// Why a process-resolved coordination identity cannot authorize this command.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "coordination_identity_rejection")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CoordinationIdentityRejection {
    /// A session mapping names a missing, inactive, or differently-owned reservation.
    StaleSessionMapping {
        /// The run recorded by the stale mapping.
        coordination_run_id: CoordinationRunId,
        /// The reservation recorded by the stale mapping.
        reservation_id:      ReservationId,
        /// The executable repair for this stale mapping.
        recovery_actions:    CoordinationIdentityRecoveryActions,
    },
    /// A worktree marker names no active reservation in its issuing worktree.
    StaleMarkerRun {
        /// The run recorded by the inactive marker.
        coordination_run_id: CoordinationRunId,
        /// The worktree that supplied the marker.
        issuing_worktree_id: WorktreeId,
        /// The canonical root that owns the marker.
        issuing_root:        CanonicalWorktreeRoot,
        /// The executable repair for this stale marker.
        recovery_actions:    CoordinationIdentityRecoveryActions,
    },
    /// Another coordination run already holds active work in the issuing worktree.
    WorktreeHeldByAnotherRun {
        /// The run already holding active work here.
        incumbent_coordination_run_id: CoordinationRunId,
        /// The incumbent's active reservation.
        incumbent_reservation_id:      ReservationId,
        /// The run this command presented.
        issuing_coordination_run_id:   CoordinationRunId,
        /// The worktree both runs name.
        issuing_worktree_id:           WorktreeId,
        /// The canonical checkout both runs name.
        issuing_root:                  CanonicalWorktreeRoot,
        /// The executable repair that ends the incumbent's occupancy of this worktree.
        recovery_actions:              CoordinationIdentityRecoveryActions,
    },
    /// A live session reservation belongs to another worktree.
    SessionWorktreeMismatch(Box<SessionWorktreeMismatchRejection>),
}

/// The coordination run whose active reservation already occupies a worktree.
#[derive(Clone, Copy, Debug)]
struct IncumbentWorktreeRun {
    /// The run holding the active reservation.
    coordination_run_id: CoordinationRunId,
    /// The active reservation that run holds in this worktree.
    reservation_id:      ReservationId,
}

/// The coordination run and worktree a command presented when it was refused.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IssuingWorktreeRun {
    /// The run this command presented.
    pub(crate) coordination_run_id: CoordinationRunId,
    /// The worktree this command ran in.
    pub(crate) worktree_id:         WorktreeId,
}

/// The complete holder and issuer facts for a session-to-worktree mismatch.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "session_worktree_mismatch_rejection")]
pub(crate) struct SessionWorktreeMismatchRejection {
    /// The run recorded by the session mapping and reservation.
    coordination_run_id: CoordinationRunId,
    /// The live reservation recorded by the session mapping.
    reservation_id:      ReservationId,
    /// The worktree that holds the reservation.
    holding_worktree_id: WorktreeId,
    /// The worktree that issued this command.
    issuing_worktree_id: WorktreeId,
    /// The canonical holder checkout.
    holding_root:        CanonicalWorktreeRoot,
    /// The canonical issuing checkout.
    issuing_root:        CanonicalWorktreeRoot,
    /// The complete alternatives for continuing or claiming separately.
    recovery_actions:    CoordinationIdentityRecoveryActions,
}

impl CoordinationIdentityRejection {
    /// Return the stable wire discriminator used by recovery consumers.
    pub(crate) const fn wire_kind(&self) -> &'static str {
        match self {
            Self::StaleSessionMapping { .. } => "stale_session_mapping",
            Self::StaleMarkerRun { .. } => "stale_marker_run",
            Self::WorktreeHeldByAnotherRun { .. } => "worktree_held_by_another_run",
            Self::SessionWorktreeMismatch(_) => "session_worktree_mismatch",
        }
    }

    /// Return reservation ids directly named by this rejection.
    pub(crate) fn reservation_ids(&self) -> Vec<ReservationId> {
        match self {
            Self::StaleSessionMapping { reservation_id, .. } => vec![*reservation_id],
            Self::StaleMarkerRun { .. } => Vec::new(),
            Self::WorktreeHeldByAnotherRun {
                incumbent_reservation_id,
                ..
            } => vec![*incumbent_reservation_id],
            Self::SessionWorktreeMismatch(rejection) => vec![rejection.reservation_id],
        }
    }

    /// Refuse a command whose run is not the one already holding active work here.
    ///
    /// The worktree is the coordination unit, so one run holds it at a time. The offered
    /// repair is [`CoordinationIdentityRecoveryAction::ReleaseIncumbentReservation`], which
    /// checkpoints the incumbent out of `Active` and so ends the occupancy this refusal
    /// names. A marker sweep cannot repair it: a sweep keeps every marker whose run still holds
    /// an active reservation, which is exactly the state being refused, so running it returns
    /// the caller to this same refusal. When
    /// the incumbent is still working the answer is a separate checkout, which the rendered
    /// message states and no command can perform.
    ///
    /// The rendered message closes on acquisition alone rather than on a blanket claim that
    /// nothing changed. The two paths that reach this refusal differ in what has already
    /// happened when it is raised: the pre-edit hook refuses before any decision, while
    /// post-commit drift observes and classifies first and refuses only the acquisition step,
    /// so the same invocation can report an incursion beside this refusal. One `Display` serves
    /// both, so it claims only what holds on both — no reservation was taken and none was
    /// widened — and leaves what else the invocation reported to the envelope carrying it.
    fn worktree_held_by_another_run(
        incumbent: IncumbentWorktreeRun,
        issuing: IssuingWorktreeRun,
        worktree_context: &WorktreeContext,
    ) -> Result<Self, CoordinationIdentityValidationError> {
        let issuing_root = canonical_issuing_root(worktree_context)?;
        Ok(Self::WorktreeHeldByAnotherRun {
            incumbent_coordination_run_id: incumbent.coordination_run_id,
            incumbent_reservation_id:      incumbent.reservation_id,
            issuing_coordination_run_id:   issuing.coordination_run_id,
            issuing_worktree_id:           issuing.worktree_id,
            issuing_root:                  issuing_root.clone(),
            recovery_actions:              CoordinationIdentityRecoveryActions::one(
                CoordinationIdentityRecoveryAction::ReleaseIncumbentReservation {
                    argv: RunnableRecoveryCommandLine::release_reservation(
                        incumbent.reservation_id,
                    ),
                    cwd:  issuing_root,
                },
            ),
        })
    }

    /// Render only the executable recovery actions selected for this rejection.
    pub(crate) fn rendered_recovery_actions(&self) -> String {
        match self {
            Self::StaleSessionMapping {
                recovery_actions, ..
            }
            | Self::StaleMarkerRun {
                recovery_actions, ..
            }
            | Self::WorktreeHeldByAnotherRun {
                recovery_actions, ..
            } => recovery_actions.render(),
            Self::SessionWorktreeMismatch(rejection) => rejection.recovery_actions.render(),
        }
    }
}

impl Display for CoordinationIdentityRejection {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleSessionMapping {
                coordination_run_id,
                reservation_id,
                recovery_actions,
            } => write!(
                formatter,
                "Harness session mapping points to inactive reservation {reservation_id} for coordination run {coordination_run_id}. Run {}, then retry the rejected command. For a managed hook, retry the original git command. No reservation or edit decision changed.",
                recovery_actions.render()
            ),
            Self::StaleMarkerRun {
                coordination_run_id,
                issuing_root,
                recovery_actions,
                ..
            } => write!(
                formatter,
                "Worktree {issuing_root} has an inactive marker for coordination run {coordination_run_id}. Run {} to reconcile and sweep the marker, then retry the rejected command. For a managed hook, retry the original git command. Retrying first will repeat this rejection.",
                recovery_actions.render()
            ),
            Self::WorktreeHeldByAnotherRun {
                incumbent_coordination_run_id,
                incumbent_reservation_id,
                issuing_coordination_run_id,
                issuing_root,
                recovery_actions,
                ..
            } => write!(
                formatter,
                "Worktree {issuing_root} already holds active reservation {incumbent_reservation_id} for coordination run {incumbent_coordination_run_id}, so coordination run {issuing_coordination_run_id} cannot take work here. If coordination run {incumbent_coordination_run_id} is finished, run {} to release reservation {incumbent_reservation_id}, then retry the rejected command. If it is still working, run coordination run {issuing_coordination_run_id} from a separate checkout instead. Acquisition is all this refuses: no reservation was taken or widened for coordination run {issuing_coordination_run_id}. Whatever else this invocation reports, it observed and recorded regardless.",
                recovery_actions.render()
            ),
            Self::SessionWorktreeMismatch(rejection) => {
                let original_command = rejection.recovery_actions.original_command_recovery();
                write!(
                    formatter,
                    "Reservation {} for coordination run {} is active in {} ({}), but this command ran in {} ({}). ",
                    rejection.reservation_id,
                    rejection.coordination_run_id,
                    rejection.holding_root,
                    rejection.holding_worktree_id,
                    rejection.issuing_root,
                    rejection.issuing_worktree_id,
                )?;
                match original_command {
                    OriginalCommandRecovery::Runnable(original_command) => write!(
                        formatter,
                        "Run {}. The claim-separately action clears this session mapping; after it succeeds, start a separate harness session, claim work in {}, and rerun `{original_command}` there. No state changed.",
                        rejection.recovery_actions.render(),
                        rejection.issuing_root,
                    ),
                    OriginalCommandRecovery::ContainsNonTextArgument => write!(
                        formatter,
                        "The original command cannot be reproduced automatically because it contains an argument that is not text. Run {}. The offered action clears the misrouted session so work can start here instead. No state changed.",
                        rejection.recovery_actions.render(),
                    ),
                    OriginalCommandRecovery::GitPrivateTransaction => write!(
                        formatter,
                        "Run {}, then retry the original git command. No state changed.",
                        rejection.recovery_actions.render(),
                    ),
                }
            },
        }
    }
}

impl Error for CoordinationIdentityRejection {}

/// Identity validation either found a caller-repairable rejection or an unusable root.
#[derive(Debug)]
pub(crate) enum CoordinationIdentityValidationError {
    /// The identity is stale or belongs to another worktree.
    Rejected(CoordinationIdentityRejection),
    /// The issuing worktree root cannot be represented by the canonical-root wire type.
    InvalidCanonicalWorktreeRoot,
}

impl Display for CoordinationIdentityValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(rejection) => rejection.fmt(formatter),
            Self::InvalidCanonicalWorktreeRoot => {
                formatter.write_str("the issuing worktree root is not canonical absolute UTF-8")
            },
        }
    }
}

impl Error for CoordinationIdentityValidationError {}

/// Refuse a presented run while another presented run holds an active reservation here.
///
/// The one same-worktree occupancy rule, in one place. Every caller that can reach it — the
/// `claim` and `check` paths through [`crate::verb`], and post-commit drift — asks the same
/// question through the same predicate, so the
/// [`CoordinationIdentityProvenance`] term is read in exactly one place and no path can grow a
/// second foreignness answer of its own.
///
/// Both terms of the rule arrive here in the types: the holder's provenance on the record
/// [`RetainedReservationSet::worktree_occupancy`] reads, and the acting side as
/// [`PresentedCoordinationRun`], which a caller holding only a bare [`CoordinationRunId`]
/// cannot produce.
pub(crate) fn validate_worktree_occupancy(
    reservations: &RetainedReservationSet,
    worktree_context: &WorktreeContext,
    worktree_id: WorktreeId,
    acting_run: PresentedCoordinationRun,
) -> Result<(), CoordinationIdentityValidationError> {
    let WorktreeOccupancy::Incumbent(incumbent) =
        reservations.worktree_occupancy(worktree_id, acting_run)
    else {
        return Ok(());
    };
    let rejection = CoordinationIdentityRejection::worktree_held_by_another_run(
        IncumbentWorktreeRun {
            coordination_run_id: incumbent.actor().run,
            reservation_id:      incumbent.id(),
        },
        IssuingWorktreeRun {
            coordination_run_id: acting_run.coordination_run_id(),
            worktree_id,
        },
        worktree_context,
    )?;
    Err(CoordinationIdentityValidationError::Rejected(rejection))
}

/// Validate one coherently resolved authorization against retained reservations.
pub(crate) fn validate_coordination_identity(
    reservations: &RetainedReservationSet,
    context: &CoordinationIdentityValidationContext,
) -> Result<(), CoordinationIdentityValidationError> {
    let resolved_edit_authorization = context.resolved_edit_authorization;
    let worktree_context = &context.worktree_context;
    let issuing_worktree_id = resolved_edit_authorization.worktree_id;
    match resolved_edit_authorization.edit_authorization() {
        EditAuthorization::Session {
            coordination_run_id,
            reservation_id,
            ..
        } => validate_session_mapping(
            reservations,
            coordination_run_id,
            reservation_id,
            issuing_worktree_id,
            worktree_context,
            &context.recovery_commands,
        ),
        EditAuthorization::Marker {
            coordination_run_id,
            ..
        } => validate_marker(
            reservations,
            coordination_run_id,
            issuing_worktree_id,
            worktree_context,
            &context.recovery_commands,
        ),
        EditAuthorization::Environment { .. } | EditAuthorization::Unidentified => Ok(()),
    }
}

fn validate_session_mapping(
    reservations: &RetainedReservationSet,
    coordination_run_id: CoordinationRunId,
    reservation_id: ReservationId,
    issuing_worktree_id: WorktreeId,
    worktree_context: &WorktreeContext,
    recovery_commands: &CoordinationIdentityRecoveryCommands,
) -> Result<(), CoordinationIdentityValidationError> {
    let Some(reservation) = reservations
        .iter()
        .find(|reservation| reservation.id() == reservation_id)
    else {
        let rejection = build_stale_session_mapping(
            coordination_run_id,
            reservation_id,
            worktree_context,
            recovery_commands,
        )?;
        return Err(CoordinationIdentityValidationError::Rejected(rejection));
    };
    if !reservation.is_active_for_coordination_run(coordination_run_id) {
        let rejection = build_stale_session_mapping(
            coordination_run_id,
            reservation_id,
            worktree_context,
            recovery_commands,
        )?;
        return Err(CoordinationIdentityValidationError::Rejected(rejection));
    }
    if reservation.actor().worktree == issuing_worktree_id {
        return Ok(());
    }
    let issuing_root = canonical_issuing_root(worktree_context)?;
    let holding_root = reservation.worktree_root().clone();
    let recovery_actions = match recovery_commands {
        CoordinationIdentityRecoveryCommands::UserCommand(original_command) => {
            user_command_mismatch_recovery_actions(original_command, &holding_root, &issuing_root)
        },
        CoordinationIdentityRecoveryCommands::GitGate {
            clear_session_mapping,
            ..
        } => CoordinationIdentityRecoveryActions::one(
            CoordinationIdentityRecoveryAction::ClearSessionMapping {
                argv: clear_session_mapping.clone(),
                cwd:  issuing_root.clone(),
            },
        ),
    };
    Err(CoordinationIdentityValidationError::Rejected(
        CoordinationIdentityRejection::SessionWorktreeMismatch(Box::new(
            SessionWorktreeMismatchRejection {
                coordination_run_id,
                reservation_id,
                holding_worktree_id: reservation.actor().worktree,
                issuing_worktree_id,
                holding_root,
                issuing_root,
                recovery_actions,
            },
        )),
    ))
}

fn user_command_mismatch_recovery_actions(
    original_command: &RecoveryCommandLine,
    holding_root: &CanonicalWorktreeRoot,
    issuing_root: &CanonicalWorktreeRoot,
) -> CoordinationIdentityRecoveryActions {
    let claim_separately = CoordinationIdentityRecoveryAction::ClaimSeparatelyHere {
        argv: RunnableRecoveryCommandLine::clear_session_mapping(),
        cwd:  issuing_root.clone(),
    };
    match original_command.runnable_arguments() {
        Ok(original_command) => CoordinationIdentityRecoveryActions::two(
            CoordinationIdentityRecoveryAction::RerunFromHoldingWorktree {
                argv: original_command,
                cwd:  holding_root.clone(),
            },
            claim_separately,
        ),
        Err(RecoveryCommandContainsNonTextArgument) => {
            CoordinationIdentityRecoveryActions::one(claim_separately)
        },
    }
}

fn build_stale_session_mapping(
    coordination_run_id: CoordinationRunId,
    reservation_id: ReservationId,
    worktree_context: &WorktreeContext,
    recovery_commands: &CoordinationIdentityRecoveryCommands,
) -> Result<CoordinationIdentityRejection, CoordinationIdentityValidationError> {
    let issuing_root = canonical_issuing_root(worktree_context)?;
    let clear_session_mapping = match recovery_commands {
        CoordinationIdentityRecoveryCommands::UserCommand(_) => {
            RunnableRecoveryCommandLine::clear_session_mapping()
        },
        CoordinationIdentityRecoveryCommands::GitGate {
            clear_session_mapping,
            ..
        } => clear_session_mapping.clone(),
    };
    Ok(CoordinationIdentityRejection::StaleSessionMapping {
        coordination_run_id,
        reservation_id,
        recovery_actions: CoordinationIdentityRecoveryActions::one(
            CoordinationIdentityRecoveryAction::ClearSessionMapping {
                argv: clear_session_mapping,
                cwd:  issuing_root,
            },
        ),
    })
}

fn validate_marker(
    reservations: &RetainedReservationSet,
    coordination_run_id: CoordinationRunId,
    issuing_worktree_id: WorktreeId,
    worktree_context: &WorktreeContext,
    recovery_commands: &CoordinationIdentityRecoveryCommands,
) -> Result<(), CoordinationIdentityValidationError> {
    if reservations.iter().any(|reservation| {
        reservation
            .is_active_for_coordination_run_and_worktree(coordination_run_id, issuing_worktree_id)
    }) {
        return Ok(());
    }
    let issuing_root = canonical_issuing_root(worktree_context)?;
    let reconcile_marker = match recovery_commands {
        CoordinationIdentityRecoveryCommands::UserCommand(_) => {
            RunnableRecoveryCommandLine::board()
        },
        CoordinationIdentityRecoveryCommands::GitGate {
            reconcile_marker, ..
        } => reconcile_marker.clone(),
    };
    Err(CoordinationIdentityValidationError::Rejected(
        CoordinationIdentityRejection::StaleMarkerRun {
            coordination_run_id,
            issuing_worktree_id,
            issuing_root: issuing_root.clone(),
            recovery_actions: CoordinationIdentityRecoveryActions::one(
                CoordinationIdentityRecoveryAction::ReconcileAndSweepMarker {
                    argv: reconcile_marker,
                    cwd:  issuing_root,
                },
            ),
        },
    ))
}

fn canonical_issuing_root(
    worktree_context: &WorktreeContext,
) -> Result<CanonicalWorktreeRoot, CoordinationIdentityValidationError> {
    worktree_context
        .repository_root()
        .to_str()
        .ok_or(CoordinationIdentityValidationError::InvalidCanonicalWorktreeRoot)?
        .parse()
        .map_err(|_| CoordinationIdentityValidationError::InvalidCanonicalWorktreeRoot)
}

fn shell_quote(argument: &str) -> String {
    if !argument.is_empty()
        && argument.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '-' | '_' | '/' | '.' | ':' | '=' | '@' | '%' | '+'
                )
        })
    {
        argument.to_owned()
    } else {
        format!("'{}'", argument.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    use serde_json::Value;

    use super::CoordinationIdentityRecoveryAction;
    use super::CoordinationIdentityRecoveryActions;
    use super::CoordinationIdentityRejection;
    use super::OriginalCommandRecovery;
    use super::PresentedCoordinationRun;
    use super::RecoveryCommandLine;
    use super::RunnableRecoveryCommandLine;
    use super::SessionWorktreeMismatchRejection;
    use super::shell_quote;
    use super::user_command_mismatch_recovery_actions;
    use crate::ids::CoordinationRunId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger::CanonicalWorktreeRoot;
    use crate::ledger::EditAuthorization;

    /// Only `CARGO_BERTH_RUN` presents a run to the occupancy rule's acting side.
    ///
    /// The other three authorization sources are not merely uninteresting here --- each is a
    /// reason the rule must not fire. A session mapping and a marker both require an active
    /// reservation of their own run in this worktree, which this same rule stops a second run
    /// from acquiring, so asking about them would refuse a run for holding the very thing that
    /// authorized it. `Unidentified` carries a run this process issued to stand in for a caller
    /// that named none, and post-commit drift first-touches under exactly that, so refusing it
    /// refuses the engine's own work.
    ///
    /// This pins the constructor rather than a variant match at a call site, which is the whole
    /// change: the three sites that ask the occupancy question now read their acting side from
    /// here, and a fourth cannot obtain one any other way --- the field is private and the only
    /// other constructor is the `--run` argument.
    #[test]
    fn only_an_environment_authorization_presents_a_run_for_the_occupancy_rule() {
        let coordination_run_id = CoordinationRunId::new();
        let worktree_id = WorktreeId::new();

        assert_eq!(
            PresentedCoordinationRun::from_edit_authorization(EditAuthorization::Environment {
                coordination_run_id,
                worktree_id,
            })
            .map(PresentedCoordinationRun::coordination_run_id),
            Some(coordination_run_id)
        );
        for unpresented in [
            EditAuthorization::Session {
                coordination_run_id,
                reservation_id: ReservationId::new(),
                worktree_id,
            },
            EditAuthorization::Marker {
                coordination_run_id,
                worktree_id,
            },
            EditAuthorization::Unidentified,
        ] {
            assert!(
                PresentedCoordinationRun::from_edit_authorization(unpresented).is_none(),
                "{unpresented:?} must not reach the occupancy question"
            );
        }
        assert_eq!(
            PresentedCoordinationRun::from_run_argument(coordination_run_id).coordination_run_id(),
            coordination_run_id
        );
    }

    #[test]
    fn recovery_domain_rejects_empty_commands_and_action_sets() {
        assert!(RecoveryCommandLine::try_from(Vec::<OsString>::new()).is_err());
        assert!(RunnableRecoveryCommandLine::try_from(Vec::<String>::new()).is_err());
        assert!(CoordinationIdentityRecoveryActions::try_from(Vec::new()).is_err());
    }

    #[test]
    fn recovery_action_serializes_complete_argv_and_canonical_cwd() {
        let cwd: CanonicalWorktreeRoot = std::env::current_dir()
            .expect("current directory should resolve")
            .to_str()
            .expect("current directory should be UTF-8")
            .parse()
            .expect("current directory should be canonical");
        let expected_rendering = format!(
            "cd {} && cargo-berth identity clear-session --json",
            shell_quote(&cwd.to_string())
        );
        let action = CoordinationIdentityRecoveryAction::ClearSessionMapping {
            argv: RunnableRecoveryCommandLine::clear_session_mapping(),
            cwd,
        };
        let serialized = serde_json::to_value(&action).expect("recovery action should serialize");

        assert_eq!(serialized["kind"], "clear_session_mapping");
        assert_eq!(
            serialized["argv"],
            Value::from(vec!["cargo-berth", "identity", "clear-session", "--json"])
        );
        assert!(
            serialized["cwd"]
                .as_str()
                .is_some_and(|cwd| cwd.starts_with('/'))
        );
        assert_eq!(action.to_string(), expected_rendering);
    }

    #[test]
    fn recovery_shell_tokens_leave_safe_text_bare_and_escape_apostrophes() {
        assert_eq!(shell_quote("cargo-berth"), "cargo-berth");
        assert_eq!(shell_quote("--json"), "--json");
        assert_eq!(shell_quote("holder's checkout"), "'holder'\\''s checkout'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn non_text_process_arguments_omit_the_rerun_action() {
        let command = RecoveryCommandLine::try_from(vec![
            OsString::from("cargo-berth"),
            OsString::from_vec(vec![b'f', 0x80]),
        ])
        .expect("non-empty recovery command should construct");
        let cwd = std::env::current_dir()
            .expect("current directory should resolve")
            .to_str()
            .expect("current directory should be UTF-8")
            .parse()
            .expect("current directory should be canonical");

        assert!(command.runnable_arguments().is_err());
        let recovery_actions = user_command_mismatch_recovery_actions(&command, &cwd, &cwd);
        assert!(matches!(
            recovery_actions.original_command_recovery(),
            OriginalCommandRecovery::ContainsNonTextArgument
        ));
        let serialized = serde_json::to_value(&recovery_actions)
            .expect("representable recovery action should serialize");
        assert_eq!(
            serialized,
            serde_json::json!([{
                "kind": "claim_separately_here",
                "argv": ["cargo-berth", "identity", "clear-session", "--json"],
                "cwd": cwd,
            }])
        );
        let rejection = CoordinationIdentityRejection::SessionWorktreeMismatch(Box::new(
            SessionWorktreeMismatchRejection {
                coordination_run_id: CoordinationRunId::new(),
                reservation_id: ReservationId::new(),
                holding_worktree_id: WorktreeId::new(),
                issuing_worktree_id: WorktreeId::new(),
                holding_root: cwd.clone(),
                issuing_root: cwd,
                recovery_actions,
            },
        ));
        let diagnostic = rejection.to_string();
        assert!(diagnostic.contains("cannot be reproduced automatically"));
        assert!(diagnostic.contains("an argument that is not text"));
        assert!(diagnostic.contains("clears the misrouted session"));
    }
}
