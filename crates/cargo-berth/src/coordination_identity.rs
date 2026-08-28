//! Coordination-identity validation and executable recovery instructions.

use std::ffi::OsString;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

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
use crate::reservation::ReservationLifecycle;
use crate::reservation::RetainedReservationSet;

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
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunnableRecoveryCommandLine(Vec<String>);

impl RunnableRecoveryCommandLine {
    const BOARD_ARGUMENTS: [&str; 3] = ["cargo-berth", "board", "--json"];
    const CLEAR_SESSION_ARGUMENTS: [&str; 4] =
        ["cargo-berth", "identity", "clear-session", "--json"];

    pub(crate) fn board() -> Self { Self::from_static(Self::BOARD_ARGUMENTS) }

    pub(crate) fn clear_session_mapping() -> Self {
        Self::from_static(Self::CLEAR_SESSION_ARGUMENTS)
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

impl std::error::Error for EmptyRecoveryCommandLine {}

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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoordinationIdentityRecoveryActions(Vec<CoordinationIdentityRecoveryAction>);

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
                | CoordinationIdentityRecoveryAction::ReconcileAndSweepMarker { .. } => None,
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

impl std::error::Error for EmptyCoordinationIdentityRecoveryActions {}

/// Why a process-resolved coordination identity cannot authorize this command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
    /// A live session reservation belongs to another worktree.
    SessionWorktreeMismatch(Box<SessionWorktreeMismatchRejection>),
}

/// The complete holder and issuer facts for a session-to-worktree mismatch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
    /// Return reservation ids directly named by this rejection.
    pub(crate) fn reservation_ids(&self) -> Vec<ReservationId> {
        match self {
            Self::StaleSessionMapping { reservation_id, .. } => vec![*reservation_id],
            Self::StaleMarkerRun { .. } => Vec::new(),
            Self::SessionWorktreeMismatch(rejection) => vec![rejection.reservation_id],
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

impl std::error::Error for CoordinationIdentityRejection {}

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

impl std::error::Error for CoordinationIdentityValidationError {}

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
    if !matches!(reservation.lifecycle(), ReservationLifecycle::Active)
        || reservation.actor().run != coordination_run_id
    {
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
        matches!(reservation.lifecycle(), ReservationLifecycle::Active)
            && reservation.actor().run == coordination_run_id
            && reservation.actor().worktree == issuing_worktree_id
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
    use super::RecoveryCommandLine;
    use super::RunnableRecoveryCommandLine;
    use super::SessionWorktreeMismatchRejection;
    use super::user_command_mismatch_recovery_actions;
    use crate::ids::CoordinationRunId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;

    #[test]
    fn recovery_domain_rejects_empty_commands_and_action_sets() {
        assert!(RecoveryCommandLine::try_from(Vec::<OsString>::new()).is_err());
        assert!(RunnableRecoveryCommandLine::try_from(Vec::<String>::new()).is_err());
        assert!(CoordinationIdentityRecoveryActions::try_from(Vec::new()).is_err());
    }

    #[test]
    fn recovery_action_serializes_complete_argv_and_canonical_cwd() {
        let cwd = std::env::current_dir()
            .expect("current directory should resolve")
            .to_str()
            .expect("current directory should be UTF-8")
            .parse()
            .expect("current directory should be canonical");
        let action = CoordinationIdentityRecoveryAction::ClearSessionMapping {
            argv: RunnableRecoveryCommandLine::clear_session_mapping(),
            cwd,
        };
        let serialized = serde_json::to_value(action).expect("recovery action should serialize");

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
