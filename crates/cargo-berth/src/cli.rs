//! The frozen command line for `cargo-berth`.
//!
//! Cargo invokes this binary as `cargo berth <verb>` and passes the word
//! `berth` to it. [`Cli::parse_arguments`] removes only that injected word, so
//! `cargo berth <verb>` and `cargo-berth <verb>` have the same command line.

use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt::Arguments;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::process::ExitCode;
use std::process::Stdio;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use clap::ArgGroup;
use clap::Args;
use clap::Error;
use clap::Parser;
use clap::Subcommand;
use clap::error::ErrorKind;

use crate::answer::OverlapAuthorizationReason;
use crate::answer::OverlapAuthorizationRequest;
use crate::answer::OverlapProposalSubmission;
use crate::answer::OverlapProposalToken;
use crate::answer::PermissiveOverlapAnswer;
use crate::answer::PermissiveOverlapAuthorizationRequest;
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::constants::OVERLAP_WHY_ARGUMENT;
use crate::constants::OVERLAP_WHY_ARGUMENT_ID;
use crate::constants::OVERLAP_WHY_VALUE_NAME;
use crate::constants::PROPOSAL_ARGUMENT;
use crate::constants::PROPOSAL_VALUE_NAME;
use crate::coordination_identity::PresentedCoordinationRun;
use crate::coordination_identity::RecoveryCommandLine;
use crate::drift::DriftComparisonChoice;
use crate::drift::DriftRequest;
use crate::drift::DriftReservationSelection;
use crate::drift::PostCommitWideningSelection;
use crate::edge::OrderingReason;
use crate::exit::BerthExit;
use crate::gate;
use crate::gate::GateDecision;
use crate::gate::GateError;
use crate::gate::GateResult;
use crate::gate::IntegrationRequest;
use crate::gate::ManagedTrunkDeletion;
use crate::gate::REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT;
use crate::gate::ReferenceTransaction;
use crate::gate::ReferenceTransactionIssuingDirectory;
use crate::gate::ReferenceTransactionParseError;
use crate::gate::ReferenceTransactionPhase;
use crate::gate::TrunkReferencePresence;
use crate::gate::permit::EnvironmentBypassRetentionOutcome;
use crate::git;
use crate::git::LocalBranchRenameTargetResolution;
use crate::hook;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ids::WorkPlanPhase;
use crate::ledger::ClaimSource;
use crate::ledger::ForcedIntegrationReason;
use crate::ledger::FullRefName;
use crate::ledger::IncursionIncidentId;
use crate::ledger::Ledger;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::NonEmptyReservationPurpose;
use crate::ledger::OrderingDirection;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReservationPurpose;
use crate::ledger::WorkPlanReference;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::output::PostCommitRendering;
use crate::recovery;
use crate::recovery::IncursionAnswerScope;
use crate::recovery::RenewRequest;
use crate::recovery::ReservationRecoveryDecision;
use crate::recovery::ResolveDecision;
use crate::recovery::ResolveRequest;
use crate::reservation::AbandonmentReason;
use crate::reservation::OrphanRetirementReason;
use crate::reservation::RewrittenIntegrationTrunkCommit;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::ScopeKind;
use crate::verb::board;
use crate::verb::board::BoardDisplayOutcome;
use crate::verb::board::BoardOutputSelection;
use crate::verb::check;
use crate::verb::check::CheckRequest;
use crate::verb::claim;
use crate::verb::claim::CheckReservationSelection;
use crate::verb::claim::ClaimCoordinationRunSelection;
use crate::verb::claim::ClaimRequest;
use crate::verb::claim::PhaseStartSelection;
use crate::verb::drift;
use crate::verb::integrate;
use crate::verb::integrate::IntegrateRequest;
use crate::verb::release;
use crate::verb::release::ReleaseRequest;
use crate::verb::sequence;
use crate::verb::sequence::SequenceRequest;

const ABANDON_ARGUMENT: &str = "abandon";
const ABOUT: &str = "Reserve git-worktree paths before they overlap";
const BINARY_NAME: &str = "cargo-berth";
const BLOCKER_VALUE_NAME: &str = "BLOCKER";
const CARGO_SUBCOMMAND_NAME: &str = "berth";
const CLAIM_AFTER_ARGUMENT: &str = "after";
const CLAIM_BEFORE_ARGUMENT: &str = "before";
const CLAIM_DEFER_ARGUMENT: &str = "defer";
const CLAIM_OVERRIDE_ARGUMENT: &str = "override";
const CLAIM_OVERRIDE_ARGUMENT_ID: &str = "override_reservation";
const CLAIM_RESOLUTION_GROUP: &str = "claim-resolution";
const FORCE_ARGUMENT: &str = "force";
const FULL_ARGUMENT: &str = "full";
const HEAD_ARGUMENT: &str = "head";
const HEAD_VALUE_NAME: &str = "OID";
const INTEGRATED_AS_ARGUMENT: &str = "integrated-as";
const INTEGRATED_AS_ARGUMENT_ID: &str = "integrated_as";
const EVERY_INCURSION_ARGUMENT: &str = "every-incursion";
const INCURSION_ARGUMENT: &str = "incursion";
const EVERY_INCURSION_ARGUMENT_ID: &str = "every_incursion";
const INCURSION_VALUE_NAME: &str = "INCIDENT_ID";
const INIT_OPERATION_GROUP: &str = "init-operation";
const JSON_ARGUMENT: &str = "json";
const PATH_VALUE_NAME: &str = "PATH";
const PHASE_ARGUMENT: &str = "phase";
const PHASE_VALUE_NAME: &str = "PHASE";
const PLAN_ARGUMENT: &str = "plan";
const PLAN_VALUE_NAME: &str = "PLAN";
const RECOVERED_ARGUMENT: &str = "recovered";
const RESERVATION_ARGUMENT: &str = "reservation";
const RESERVATION_VALUE_NAME: &str = "RESERVATION_ID";
const REPAIR_PROJECTION_ARGUMENT: &str = "repair-projection";
const REPAIR_PROJECTION_ARGUMENT_ID: &str = "repair_projection";
const REINITIALIZE_AFTER_REVIEW_ARGUMENT: &str = "reinitialize-after-review";
const REINITIALIZE_AFTER_REVIEW_ARGUMENT_ID: &str = "reinitialize_after_review";
const RESOLVE_REASONED_DISPOSITION_GROUP: &str = "resolve-reasoned-disposition";
const RESOLVE_DISPOSITION_GROUP: &str = "resolve-disposition";
const RETIRE_ORPHAN_ARGUMENT: &str = "retire-orphan";
const RETIRE_ORPHAN_ARGUMENT_ID: &str = "retire_orphan";
const RUN_ARGUMENT: &str = "run";
const RUN_VALUE_NAME: &str = "COORDINATION_RUN_ID";
const POST_COMMIT_HOOK_ENVIRONMENT: &str = "CARGO_BERTH_POST_COMMIT";
const TRUNK_OID_VALUE_NAME: &str = "TRUNK_OID";
const WHY_ARGUMENT: &str = "why";
const WHY_VALUE_NAME: &str = "WHY";

const ABANDON_LONG_ABOUT: &str = "Use this only when the reservation's work is intentionally discarded. It records an irreversible abandonment and releases its coordination hold; choosing it for recoverable work loses the trail that identifies where the work went. --why is required so later readers can distinguish a deliberate decision from a lost worktree.";
const BOARD_LONG_ABOUT: &str = "Inspect current reservations and integration constraints. With both standard input and standard output attached to terminals, bare board opens the full-screen view; otherwise it prints a pointer to board --json. Use --json to emit board facts.";
const CHECK_LONG_ABOUT: &str = "Check proposed paths against foreign reservations. An unprefixed path means one exact file; prefix a path with file: to state that explicitly or tree: to include all component descendants.";
const INTEGRATED_AS_LONG_ABOUT: &str = "Use this when the reservation's work reached trunk through a squash, cherry-pick, or other rewritten integration that the tool cannot prove from its stored commit. This asserts the supplied trunk commit is evidence; choosing it without that evidence can incorrectly release an unresolved reservation.";
const RECOVERED_LONG_ABOUT: &str = "Use this when the reservation's work is still present but now belongs to this replacement worktree. It records a new worktree identity; choosing it when the work was actually integrated or discarded leaves an inaccurate live reservation blocking other work.";
const RETIRE_ORPHAN_LONG_ABOUT: &str = "Use this only after confirming an orphaned reservation can retire without classifying its work as deliberately discarded. It records a distinct orphan-retirement disposition and requires --why so later readers can audit that decision.";
const RENEW_LONG_ABOUT: &str = "Record that this still-live reservation remains active after inspection. Renewal changes neither its scopes nor any ordering edge; using it to hide abandoned work delays the user-confirmed recovery or abandonment decision that must eventually resolve it.";
const EVERY_INCURSION_LONG_ABOUT: &str = "Answer every incursion incident outstanding for this reservation in one disposition. A backlog reports one notice per incident, and answering a single member leaves the rest standing, so the notice keeps firing until the set is empty.";
const RESOLVE_LONG_ABOUT: &str = "Resolve a reservation recovery or an incursion incident. Choose exactly one disposition: --incursion <INCIDENT_ID> for an outstanding incident; --recovered when work survives in this replacement worktree; --integrated-as <TRUNK_OID> when work reached trunk in a form the tool could not prove; --abandon --why <WHY> only when work is deliberately discarded; or --retire-orphan --why <WHY> after confirming an orphan may retire without classifying its work as discarded. Choosing --abandon discards work. Choosing --integrated-as asserts evidence the tool could not prove for itself, so a wrong commit can release an unresolved reservation.";
/// The refusal earned by a resolve command line that names no single disposition.
const RESOLVE_DISPOSITION_REFUSAL: &str = "choose exactly one resolution disposition and provide --why only for --abandon or --retire-orphan";

/// `cargo-berth`, as the command line sees it.
#[derive(Debug, Parser)]
#[command(name = BINARY_NAME, version, about = ABOUT, subcommand_required = true)]
pub(crate) struct Cli {
    /// The reservation command to run.
    #[command(subcommand)]
    command: Command,
}

/// The command line after clap has either parsed it or classified its error.
pub(crate) enum CliInvocation {
    /// A valid command line.
    Command(Box<Cli>),
    /// A command line clap could not parse.
    Usage(Error),
}

/// The verbs available from the frozen `cargo-berth` interface.
#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize the shared reservation ledger.
    Init(InitArguments),
    /// Inspect reservations and integration constraints.
    #[command(long_about = BOARD_LONG_ABOUT)]
    Board(BoardArguments),
    /// Check exact files or explicitly prefixed trees for foreign reservations.
    #[command(long_about = CHECK_LONG_ABOUT)]
    Check(CheckArguments),
    /// Run one public harness hook entry point.
    Hook(HookArguments),
    /// Claim paths for a reservation.
    Claim(ClaimArguments),
    /// Compare observed worktree changes with an active reservation.
    Drift(DriftArguments),
    /// Release a reservation at a checkpoint.
    Release(ReservationArguments),
    /// Record an ordering relationship between reservations.
    Sequence(SequenceArguments),
    /// Integrate a reservation into trunk.
    Integrate(IntegrateArguments),
    /// Resolve a stuck reservation.
    #[command(about = "Resolve a stuck reservation", long_about = RESOLVE_LONG_ABOUT)]
    Resolve(ResolveArguments),
    /// Renew a reservation's activity record.
    #[command(about = "Renew a reservation", long_about = RENEW_LONG_ABOUT)]
    Renew(ReservationArguments),
    /// Manage the current process's disposable coordination identity.
    Identity(IdentityArguments),
    /// Private dispatch used only by the installed git hook.
    #[command(name = "__reference-transaction", hide = true)]
    ReferenceTransaction(ReferenceTransactionArguments),
    /// Private refresh worker scheduled after a committed trunk-ref deletion.
    #[command(name = "__refresh-managed-hook-after-trunk-deletion", hide = true)]
    RefreshManagedHookAfterTrunkDeletion(ManagedHookRefreshArguments),
}

/// Public harness-hook commands.
#[derive(Debug, Args)]
struct HookArguments {
    /// The hook entry point to run.
    #[command(subcommand)]
    command: HookCommand,
}

/// Harness hook entry points implemented by the engine.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Subcommand)]
enum HookCommand {
    /// Authorize one file-writing tool request read from standard input.
    PreToolUse,
    /// Report working-tree drift for one completed Bash call read from standard input.
    PostToolUse,
    /// Publish current coordination state for one session opening read from standard input.
    SessionStart,
}

impl HookCommand {
    /// Write this hook's protocol response and report the exit status the process publishes.
    ///
    /// Reading this process's standard input happens here, so a caller that only needs to know
    /// which hook answers an invocation never reaches it.
    fn write_response(self) -> ExitCode {
        match self {
            Self::PreToolUse => hook::pre_tool_use::execute(),
            Self::PostToolUse => hook::post_tool_use::execute(),
            Self::SessionStart => hook::session_start::execute(),
        }
    }
}

/// The `--json` flag shared by every verb.
#[derive(Debug, Args)]
struct JsonOutput {
    /// Emit the frozen JSON response envelope.
    #[arg(long = JSON_ARGUMENT)]
    json: bool,
}

/// Complete-board or named-reservation board arguments.
#[derive(Debug, Args)]
struct BoardArguments {
    /// Read one reservation's lifecycle independently of its board placement.
    #[arg(
        long = RESERVATION_ARGUMENT,
        value_name = RESERVATION_VALUE_NAME,
        requires = JSON_ARGUMENT
    )]
    reservation: Option<ReservationId>,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output: JsonOutput,
}

/// Coordination identity management commands.
#[derive(Debug, Args)]
struct IdentityArguments {
    /// The identity operation to run.
    #[command(subcommand)]
    command: IdentityCommand,
}

/// Operations over the current process's disposable identity sources.
#[derive(Debug, Subcommand)]
enum IdentityCommand {
    /// Remove only the current `CARGO_BERTH_SESSION_ID` mapping.
    ClearSession(JsonOutput),
}

/// Initialization arguments including explicit projection-only recovery.
#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new(INIT_OPERATION_GROUP)
        .args([
            REPAIR_PROJECTION_ARGUMENT_ID,
            REINITIALIZE_AFTER_REVIEW_ARGUMENT_ID,
        ])
        .multiple(false)
))]
struct InitArguments {
    /// Remove and rebuild only `reservations.json` from journal truth.
    #[arg(long = REPAIR_PROJECTION_ARGUMENT)]
    repair_projection:         bool,
    /// Discard journal state after confirming every pending order was reviewed.
    #[arg(long = REINITIALIZE_AFTER_REVIEW_ARGUMENT)]
    reinitialize_after_review: bool,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output:               JsonOutput,
}

/// Git's private hook lifecycle argument; update lines arrive on standard input.
#[derive(Debug, Args)]
struct ReferenceTransactionArguments {
    /// The preparing, prepared, committed, or aborted phase; an unknown word is a no-op.
    phase:           ReferenceTransactionPhase,
    /// The full configured trunk ref captured when the hook was installed.
    trunk_reference: FullRefName,
}

/// The deleted managed trunk ref and the object its replacement must still name.
#[derive(Debug, Args)]
struct ManagedHookRefreshArguments {
    /// The configured trunk ref whose committed deletion scheduled this worker.
    deleted_reference: FullRefName,
    /// The object tip the deleted trunk ref named.
    previous_tip:      GitObjectId,
}

/// Why git's reference-transaction input could not be classified.
enum ReferenceTransactionInputError {
    /// Standard input could not be read completely.
    StandardInputUnreadable(std::io::Error),
    /// The input did not satisfy git's reference-transaction record format.
    MalformedHookInput(ReferenceTransactionParseError),
}

impl Display for ReferenceTransactionInputError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StandardInputUnreadable(error) => {
                write!(formatter, "standard input was unreadable: {error}")
            },
            Self::MalformedHookInput(error) => {
                write!(formatter, "hook input was malformed: {error}")
            },
        }
    }
}

/// How a bypassed transaction relates to the configured trunk reference.
enum BypassTransactionTrunkRelation {
    /// The parsed transaction names the trunk reference.
    Named,
    /// The parsed transaction demonstrably does not name the trunk reference.
    NotNamed,
    /// Input failure prevented the transaction from being classified.
    Unconfirmed(ReferenceTransactionInputError),
}

/// Why this bypass invocation must attempt to retain an audit fact.
enum EnvironmentBypassAuditBasis {
    /// Parsed input confirmed that the transaction names the trunk reference.
    ConfirmedTrunkReference,
    /// Input failure means the transaction may name the trunk reference.
    UnconfirmedTrunkReference(ReferenceTransactionInputError),
}

/// The output representation requested at the command line boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CliOutputFormat {
    /// Print the frozen JSON envelope.
    Json,
    /// Print the envelope's message.
    Text,
}

/// Whether the installed post-commit hook requested all-reservation drift behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PostCommitHookRequest {
    /// Use the post-commit hook's all-reservation drift behavior.
    Requested,
    /// Use the ordinary command-line drift behavior.
    NotRequested,
}

/// How a completed command response is rendered for its caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandResponseRendering {
    /// Emit the frozen response envelope in the requested representation.
    OutputEnvelope(CliOutputFormat),
    /// Emit only the installed post-commit hook warning.
    PostCommitWarning,
}

impl From<bool> for CliOutputFormat {
    fn from(json: bool) -> Self { if json { Self::Json } else { Self::Text } }
}

impl JsonOutput {
    /// Convert clap's flag value into the command's output representation.
    fn output_format(&self) -> CliOutputFormat { self.json.into() }
}

impl BoardArguments {
    /// Convert clap's optional flag into the board's domain selection.
    fn into_output_selection(self) -> BoardOutputSelection {
        self.reservation.map_or(
            BoardOutputSelection::CompleteBoard,
            BoardOutputSelection::ReservationLifecycleFor,
        )
    }
}

/// A command whose first argument is one or more repository paths.
#[derive(Debug, Args)]
struct PathArguments {
    /// The paths to check; unprefixed paths are files, while `tree:` includes descendants.
    #[arg(required = true, value_name = PATH_VALUE_NAME)]
    paths:       Vec<PathBuf>,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output: JsonOutput,
}

/// Edit-check paths and the reservation the caller intends to continue.
#[derive(Debug, Args)]
struct CheckArguments {
    /// The paths and output representation for this check.
    #[command(flatten)]
    path_arguments: PathArguments,
    /// Continue the named active reservation held by the acting run and worktree.
    #[arg(long = RESERVATION_ARGUMENT, value_name = RESERVATION_VALUE_NAME)]
    reservation:    Option<ReservationId>,
}

/// Arguments that answer an overlap while claiming paths.
#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new(CLAIM_RESOLUTION_GROUP)
        .args([
            CLAIM_BEFORE_ARGUMENT,
            CLAIM_AFTER_ARGUMENT,
            CLAIM_DEFER_ARGUMENT,
            CLAIM_OVERRIDE_ARGUMENT_ID,
        ])
        .multiple(false)
))]
struct ClaimArguments {
    /// The repository paths to reserve.
    #[arg(required = true, value_name = PATH_VALUE_NAME)]
    paths:                Vec<PathBuf>,
    /// Sequence this reservation before the blocking reservation.
    #[arg(
        long = CLAIM_BEFORE_ARGUMENT,
        value_name = BLOCKER_VALUE_NAME,
        requires = OVERLAP_WHY_ARGUMENT_ID
    )]
    before:               Option<ReservationId>,
    /// Sequence this reservation after the blocking reservation.
    #[arg(
        long = CLAIM_AFTER_ARGUMENT,
        value_name = BLOCKER_VALUE_NAME,
        requires = OVERLAP_WHY_ARGUMENT_ID
    )]
    after:                Option<ReservationId>,
    /// Defer an answer about the blocking reservation.
    #[arg(
        long = CLAIM_DEFER_ARGUMENT,
        value_name = BLOCKER_VALUE_NAME,
        requires = OVERLAP_WHY_ARGUMENT_ID
    )]
    defer:                Option<ReservationId>,
    /// Override the blocking reservation.
    #[arg(
        long = CLAIM_OVERRIDE_ARGUMENT,
        value_name = BLOCKER_VALUE_NAME,
        requires = OVERLAP_WHY_ARGUMENT_ID
    )]
    override_reservation: Option<ReservationId>,
    /// Explain why these paths are being protected.
    #[arg(long = WHY_ARGUMENT, value_name = WHY_VALUE_NAME)]
    why:                  Option<String>,
    /// Explain why this specific overlap answer is authorized.
    #[arg(
        long = OVERLAP_WHY_ARGUMENT,
        value_name = OVERLAP_WHY_VALUE_NAME,
        requires = CLAIM_RESOLUTION_GROUP
    )]
    overlap_why:          Option<String>,
    /// Apply the exact overlap proposal returned by the preceding invocation.
    #[arg(
        long = PROPOSAL_ARGUMENT,
        value_name = PROPOSAL_VALUE_NAME,
        requires = CLAIM_RESOLUTION_GROUP
    )]
    proposal:             Option<String>,
    /// Name the external work plan that originated this claim.
    #[arg(
        long = PLAN_ARGUMENT,
        value_name = PLAN_VALUE_NAME,
        requires = PHASE_ARGUMENT
    )]
    plan:                 Option<WorkPlanReference>,
    /// Name the opaque phase label within the external work plan.
    #[arg(
        long = PHASE_ARGUMENT,
        value_name = PHASE_VALUE_NAME,
        requires = PLAN_ARGUMENT
    )]
    phase:                Option<WorkPlanPhase>,
    /// Use this UUID-v7 coordination run instead of creating one.
    #[arg(long = RUN_ARGUMENT, value_name = RUN_VALUE_NAME)]
    run:                  Option<CoordinationRunId>,
    /// Record the full phase-start commit used for later drift comparison.
    #[arg(long = HEAD_ARGUMENT, value_name = HEAD_VALUE_NAME)]
    head:                 Option<ProtectedPhaseStartHead>,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output:          JsonOutput,
}

/// A command whose first argument is one reservation identifier.
#[derive(Debug, Args)]
struct ReservationArguments {
    /// The reservation the command concerns.
    reservation_id: ReservationId,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output:    JsonOutput,
}

/// Arguments selecting a cheap delta or complete phase-start drift comparison.
#[derive(Debug, Args)]
struct DriftArguments {
    /// Name the active reservation to widen or receive an incursion record.
    #[arg(long = RESERVATION_ARGUMENT, value_name = RESERVATION_VALUE_NAME)]
    reservation: Option<ReservationId>,
    /// Run the complete four-command comparison against the protected phase-start HEAD.
    #[arg(long = FULL_ARGUMENT)]
    full:        bool,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output: JsonOutput,
}

/// Arguments for the `sequence` verb.
#[derive(Debug, Args)]
struct SequenceArguments {
    /// The reservation that must integrate first.
    first:       ReservationId,
    /// The reservation that must integrate afterward.
    then:        ReservationId,
    /// Explain why this ordering is needed.
    #[arg(long = WHY_ARGUMENT, value_name = WHY_VALUE_NAME)]
    why:         String,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output: JsonOutput,
}

/// Arguments for the `integrate` verb.
#[derive(Debug, Args)]
struct IntegrateArguments {
    /// The reservation to integrate.
    reservation_id: ReservationId,
    /// Permit integration past held ordering edges and unresolved deferrals.
    #[arg(long = FORCE_ARGUMENT, requires = WHY_ARGUMENT)]
    force:          bool,
    /// Explain why forced integration is authorized.
    #[arg(long = WHY_ARGUMENT, value_name = WHY_VALUE_NAME, requires = FORCE_ARGUMENT)]
    why:            Option<String>,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output:    JsonOutput,
}

/// Which of a reservation's outstanding incursion incidents this command line answers.
#[derive(Debug, Args)]
struct IncursionAnswerSelection {
    /// Answer this outstanding incursion incident.
    #[arg(long = INCURSION_ARGUMENT, value_name = INCURSION_VALUE_NAME)]
    incursion:       Option<IncursionIncidentId>,
    /// Answer every incursion incident outstanding for the reservation.
    #[arg(long = EVERY_INCURSION_ARGUMENT, long_help = EVERY_INCURSION_LONG_ABOUT)]
    every_incursion: bool,
}

/// How this command line disposes of the stuck reservation itself.
#[derive(Debug, Args)]
struct ReservationRecoverySelection {
    /// Record this worktree as the recovered holder of surviving work.
    #[arg(long = RECOVERED_ARGUMENT, long_help = RECOVERED_LONG_ABOUT)]
    recovered:     bool,
    /// Assert a trunk commit proves rewritten integration.
    #[arg(
        long = INTEGRATED_AS_ARGUMENT,
        value_name = TRUNK_OID_VALUE_NAME,
        long_help = INTEGRATED_AS_LONG_ABOUT
    )]
    integrated_as: Option<RewrittenIntegrationTrunkCommit>,
    /// Permanently discard this reservation's work and coordination hold.
    #[arg(
        long = ABANDON_ARGUMENT,
        requires = WHY_ARGUMENT,
        long_help = ABANDON_LONG_ABOUT
    )]
    abandon:       bool,
    /// Retire this confirmed orphan without classifying it as deliberate abandonment.
    #[arg(
        long = RETIRE_ORPHAN_ARGUMENT,
        requires = WHY_ARGUMENT,
        long_help = RETIRE_ORPHAN_LONG_ABOUT
    )]
    retire_orphan: bool,
    /// Explain the deliberate abandonment or orphan-retirement decision.
    #[arg(
        long = WHY_ARGUMENT,
        value_name = WHY_VALUE_NAME,
        requires = RESOLVE_REASONED_DISPOSITION_GROUP
    )]
    why:           Option<String>,
}

/// Whether the incursion-answer flags name a disposition for this resolve.
enum IncursionAnswerNomination {
    /// No incursion flag appeared on the command line.
    NoIncursionAnswered,
    /// The flags name exactly one scope of incursion answers.
    AnswerScope(IncursionAnswerScope),
}

/// Whether the reservation-recovery flags name a disposition for this resolve.
enum ReservationRecoveryNomination {
    /// No reservation-recovery flag appeared on the command line.
    NoRecoveryRequested,
    /// The flags name exactly one reservation recovery decision.
    Recovery(ReservationRecoveryDecision),
}

/// A user-confirmed recovery decision for a stuck reservation.
#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new(RESOLVE_DISPOSITION_GROUP)
        .args([
            RECOVERED_ARGUMENT,
            INTEGRATED_AS_ARGUMENT_ID,
            ABANDON_ARGUMENT,
            RETIRE_ORPHAN_ARGUMENT_ID,
            INCURSION_ARGUMENT,
            EVERY_INCURSION_ARGUMENT_ID,
        ])
        .required(true)
        .multiple(false)
))]
#[command(group(
    ArgGroup::new(RESOLVE_REASONED_DISPOSITION_GROUP)
        .args([ABANDON_ARGUMENT, RETIRE_ORPHAN_ARGUMENT_ID])
        .multiple(false)
))]
struct ResolveArguments {
    /// The stuck reservation to resolve.
    reservation_id:       ReservationId,
    /// The incursion incidents this command line answers, if it answers any.
    #[command(flatten)]
    incursion_answer:     IncursionAnswerSelection,
    /// The recovery this command line applies to the reservation, if it applies one.
    #[command(flatten)]
    reservation_recovery: ReservationRecoverySelection,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output:          JsonOutput,
}

impl Cli {
    /// Read the command line, whether cargo invoked this binary or a shell did.
    pub(crate) fn parse_arguments() -> CliInvocation {
        Self::try_parse_from(without_subcommand_name(env::args_os().collect()))
            .map_or_else(CliInvocation::Usage, |cli| {
                CliInvocation::Command(Box::new(cli))
            })
    }

    /// Execute the parsed command and return its published process exit status.
    fn run(self) -> ExitCode {
        let command = match self.command {
            Command::ReferenceTransaction(arguments) => {
                return run_reference_transaction(arguments.phase, arguments.trunk_reference);
            },
            Command::RefreshManagedHookAfterTrunkDeletion(arguments) => {
                return run_managed_hook_refresh_worker(&arguments);
            },
            command => command,
        };
        let output_format = command.output_format();
        let recovery_command_line = RecoveryCommandLine::current_process();
        match command.execute(output_format, &recovery_command_line) {
            CommandOutputOwnership::CallerRendersResponse(output_envelope) => {
                publish_envelope_response(output_format, &output_envelope).into()
            },
            CommandOutputOwnership::BoardPresentedAndTerminalRestored => BerthExit::Clear.into(),
            CommandOutputOwnership::HookOwnsItsResponse(hook_command) => {
                hook_command.write_response()
            },
        }
    }
}

impl CliInvocation {
    /// Print a parser error or execute a valid command.
    pub(crate) fn run(self) -> ExitCode {
        match self {
            Self::Command(cli) => (*cli).run(),
            Self::Usage(error) => {
                let berth_exit = exit_for_parser_error(&error);
                if let Err(print_error) = error.print() {
                    eprintln!("{BINARY_NAME}: {print_error}");
                }
                berth_exit.into()
            },
        }
    }
}

impl Command {
    /// Execute this command after its output representation is resolved.
    fn execute(
        self,
        output_format: CliOutputFormat,
        recovery_command_line: &RecoveryCommandLine,
    ) -> CommandOutputOwnership {
        let output_envelope = match self {
            Self::Init(init_arguments) => {
                initialize_ledger(init_arguments.initialization_request())
            },
            Self::Board(board_arguments) => {
                return match board::execute(board_arguments.into_output_selection(), output_format)
                {
                    BoardDisplayOutcome::HeadlessResponse(output_envelope)
                    | BoardDisplayOutcome::TerminalDidNotOpen(output_envelope)
                    | BoardDisplayOutcome::TerminalFailedAfterOpening(output_envelope)
                    | BoardDisplayOutcome::FactsUnavailable(output_envelope) => {
                        CommandOutputOwnership::CallerRendersResponse(Box::new(output_envelope))
                    },
                    BoardDisplayOutcome::TerminalRestored => {
                        CommandOutputOwnership::BoardPresentedAndTerminalRestored
                    },
                };
            },
            Self::Check(check_arguments) => match check_arguments.into_check_request() {
                Ok(check_request) => check::execute(check_request, recovery_command_line),
                Err(error) => OutputEnvelope::invalid_input(CommandVerb::Check, &error),
            },
            Self::Hook(HookArguments { command }) => {
                return CommandOutputOwnership::HookOwnsItsResponse(command);
            },
            Self::Claim(claim_arguments) => match claim_arguments.into_claim_request() {
                Ok(claim_request) => claim::execute(claim_request, recovery_command_line),
                Err(error) => OutputEnvelope::invalid_input(CommandVerb::Claim, &error),
            },
            Self::Drift(drift_arguments) => {
                drift::execute(drift_arguments.into_drift_request(), recovery_command_line)
            },
            Self::Release(reservation_arguments) => {
                release::execute(reservation_arguments.into_release_request())
            },
            Self::Sequence(sequence_arguments) => {
                match sequence_arguments.into_sequence_request() {
                    Ok(sequence_request) => {
                        sequence::execute(&sequence_request, recovery_command_line)
                    },
                    Err(error) => OutputEnvelope::invalid_input(CommandVerb::Sequence, &error),
                }
            },
            Self::Integrate(integrate_arguments) => {
                match integrate_arguments.into_integrate_request() {
                    Ok(integrate_request) => {
                        integrate::execute(integrate_request, recovery_command_line)
                    },
                    Err(error) => OutputEnvelope::invalid_input(CommandVerb::Integrate, &error),
                }
            },
            Self::Resolve(resolve_arguments) => match resolve_arguments.into_resolve_request() {
                Ok(resolve_request) => recovery::resolve(resolve_request),
                Err(error) => OutputEnvelope::invalid_input(CommandVerb::Resolve, &error),
            },
            Self::Renew(reservation_arguments) => {
                recovery::renew(reservation_arguments.into_renew_request())
            },
            Self::Identity(identity_arguments) => {
                execute_identity_command(&identity_arguments.command)
            },
            Self::ReferenceTransaction(_) | Self::RefreshManagedHookAfterTrunkDeletion(_) => {
                OutputEnvelope::invalid_input(
                    CommandVerb::Integrate,
                    "a private hook dispatch cannot use the public envelope path",
                )
            },
        };
        CommandOutputOwnership::CallerRendersResponse(Box::new(output_envelope))
    }

    /// Return this command's requested output representation.
    fn output_format(&self) -> CliOutputFormat {
        match self {
            Self::Init(init_arguments) => init_arguments.json_output.output_format(),
            Self::Board(board_arguments) => board_arguments.json_output.output_format(),
            Self::Check(check_arguments) => {
                check_arguments.path_arguments.json_output.output_format()
            },
            Self::Hook(_) => CliOutputFormat::Text,
            Self::Claim(claim_arguments) => claim_arguments.json_output.output_format(),
            Self::Drift(drift_arguments) => drift_arguments.json_output.output_format(),
            Self::Release(reservation_arguments) | Self::Renew(reservation_arguments) => {
                reservation_arguments.json_output.output_format()
            },
            Self::Sequence(sequence_arguments) => sequence_arguments.json_output.output_format(),
            Self::Integrate(integrate_arguments) => integrate_arguments.json_output.output_format(),
            Self::Resolve(resolve_arguments) => resolve_arguments.json_output.output_format(),
            Self::Identity(identity_arguments) => match &identity_arguments.command {
                IdentityCommand::ClearSession(json_output) => json_output.output_format(),
            },
            Self::ReferenceTransaction(_) | Self::RefreshManagedHookAfterTrunkDeletion(_) => {
                CliOutputFormat::Text
            },
        }
    }

    /// Report how this command publishes its result, without executing its engine.
    #[cfg(test)]
    const fn result_reporting(&self) -> CommandResultReporting {
        match self {
            Self::Init(_) => CommandResultReporting::Envelope(CommandVerb::Init),
            Self::Board(_) => CommandResultReporting::Envelope(CommandVerb::Board),
            Self::Check(_) => CommandResultReporting::Envelope(CommandVerb::Check),
            Self::Hook(HookArguments { command }) => CommandResultReporting::HookProtocol(*command),
            Self::Claim(_) => CommandResultReporting::Envelope(CommandVerb::Claim),
            Self::Drift(_) => CommandResultReporting::Envelope(CommandVerb::Drift),
            Self::Release(_) => CommandResultReporting::Envelope(CommandVerb::Release),
            Self::Sequence(_) => CommandResultReporting::Envelope(CommandVerb::Sequence),
            Self::Integrate(_) => CommandResultReporting::Envelope(CommandVerb::Integrate),
            Self::Resolve(_) => CommandResultReporting::Envelope(CommandVerb::Resolve),
            Self::Renew(_) => CommandResultReporting::Envelope(CommandVerb::Renew),
            Self::Identity(_) => CommandResultReporting::Envelope(CommandVerb::Identity),
            Self::ReferenceTransaction(_) | Self::RefreshManagedHookAfterTrunkDeletion(_) => {
                CommandResultReporting::GitHookProtocol
            },
        }
    }

    /// Name the coverage row this parsed command belongs to.
    #[cfg(test)]
    const fn route(&self) -> CommandLineRoute {
        match self {
            Self::Init(_) => CommandLineRoute::Init,
            Self::Board(_) => CommandLineRoute::Board,
            Self::Check(_) => CommandLineRoute::Check,
            Self::Hook(HookArguments {
                command: HookCommand::PreToolUse,
            }) => CommandLineRoute::HookPreToolUse,
            Self::Hook(HookArguments {
                command: HookCommand::PostToolUse,
            }) => CommandLineRoute::HookPostToolUse,
            Self::Hook(HookArguments {
                command: HookCommand::SessionStart,
            }) => CommandLineRoute::HookSessionStart,
            Self::Claim(_) => CommandLineRoute::Claim,
            Self::Drift(_) => CommandLineRoute::Drift,
            Self::Release(_) => CommandLineRoute::Release,
            Self::Sequence(_) => CommandLineRoute::Sequence,
            Self::Integrate(_) => CommandLineRoute::Integrate,
            Self::Resolve(_) => CommandLineRoute::Resolve,
            Self::Renew(_) => CommandLineRoute::Renew,
            Self::Identity(IdentityArguments {
                command: IdentityCommand::ClearSession(_),
            }) => CommandLineRoute::IdentityClearSession,
            Self::ReferenceTransaction(_) => CommandLineRoute::ReferenceTransaction,
            Self::RefreshManagedHookAfterTrunkDeletion(_) => {
                CommandLineRoute::RefreshManagedHookAfterTrunkDeletion
            },
        }
    }
}

/// Where one parsed command's result reaches the caller.
#[cfg(test)]
#[derive(Debug, Eq, PartialEq)]
enum CommandResultReporting {
    /// The command renders one response envelope recorded under this verb.
    Envelope(CommandVerb),
    /// The command answers with this harness hook's protocol response instead of an envelope.
    HookProtocol(HookCommand),
    /// The command answers a git hook invocation: any diagnostic goes to standard error
    /// and the process exit status is the whole answer, so no envelope is ever rendered.
    GitHookProtocol,
}

/// One command line for every route the frozen interface publishes a result through.
///
/// A new [`Command`] variant forces a new member here through [`Command::route`], and a new
/// member forces [`CommandLineRoute::ALL`] to grow, so no route can enter the parser without
/// a row in the coverage the tests below assert over.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandLineRoute {
    /// `init` initializes the shared ledger.
    Init,
    /// `board` inspects reservations and integration constraints.
    Board,
    /// `check` asks whether proposed paths collide with a foreign reservation.
    Check,
    /// `hook pre-tool-use` authorizes one file-writing tool request.
    HookPreToolUse,
    /// `hook post-tool-use` reports drift for one completed Bash call.
    HookPostToolUse,
    /// `hook session-start` publishes coordination state for one session opening.
    HookSessionStart,
    /// `claim` reserves paths for a reservation.
    Claim,
    /// `drift` compares observed changes with reservation scopes.
    Drift,
    /// `release` walks a reservation through its lifecycle.
    Release,
    /// `sequence` records an ordering relationship.
    Sequence,
    /// `integrate` moves trunk to this worktree's head.
    Integrate,
    /// `resolve` answers an incursion or records a recovery disposition.
    Resolve,
    /// `renew` refreshes a reservation's activity record.
    Renew,
    /// `identity clear-session` drops the process's disposable session mapping.
    IdentityClearSession,
    /// The private dispatch git's installed `reference-transaction` hook invokes.
    ReferenceTransaction,
    /// The private worker scheduled after a committed trunk-ref deletion.
    RefreshManagedHookAfterTrunkDeletion,
}

/// Who owns presenting a command's result once the command has run.
enum CommandOutputOwnership {
    /// The caller renders this response through the resolved output representation.
    CallerRendersResponse(Box<OutputEnvelope>),
    /// The board TUI presented itself and restored the terminal before returning.
    BoardPresentedAndTerminalRestored,
    /// A public hook command owns its protocol response and the exit status it publishes.
    ///
    /// Writing that response reads this process's standard input, so the write belongs to
    /// [`Cli::run`], which owns the process, rather than to the dispatch that selected the
    /// hook. Nothing has been written when this value is returned.
    HookOwnsItsResponse(HookCommand),
}

impl CheckArguments {
    fn into_check_request(self) -> Result<CheckRequest, String> {
        let reservation_selection = self.reservation.map_or(
            CheckReservationSelection::SessionMappingOrSingleActive,
            CheckReservationSelection::Explicit,
        );
        DeclaredReservationScopeSet::parse(self.path_arguments.paths, ScopeKind::File)
            .map(|declared_scopes| CheckRequest {
                declared_scopes,
                reservation_selection,
            })
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy)]
enum InitializationRequest {
    Initialize,
    RepairProjection,
    ReinitializeAfterReview,
}

impl InitArguments {
    const fn initialization_request(&self) -> InitializationRequest {
        match (self.repair_projection, self.reinitialize_after_review) {
            (true, false) => InitializationRequest::RepairProjection,
            (false, true) => InitializationRequest::ReinitializeAfterReview,
            (false, false) | (true, true) => InitializationRequest::Initialize,
        }
    }
}

impl ClaimArguments {
    fn into_claim_request(self) -> Result<ClaimRequest, String> {
        let Self {
            paths,
            before,
            after,
            defer,
            override_reservation,
            why,
            overlap_why,
            proposal,
            plan,
            phase,
            run,
            head,
            json_output: _,
        } = self;
        let declared_scopes = DeclaredReservationScopeSet::parse(paths, ScopeKind::Tree)
            .map_err(|error| error.to_string())?;
        let source = match (plan, phase) {
            (Some(plan), Some(phase)) => ClaimSource::WorkPlan { plan, phase },
            (None, None) => ClaimSource::Explicit,
            (Some(_), None) | (None, Some(_)) => {
                return Err("--plan and --phase must be supplied together".to_owned());
            },
        };
        let purpose = match why {
            Some(explanation) => explanation
                .parse::<NonEmptyReservationPurpose>()
                .map(ReservationPurpose::Explained)
                .map_err(|error| error.to_string())?,
            None => ReservationPurpose::NotProvidedByCaller,
        };
        let phase_start = head.map_or(
            PhaseStartSelection::CurrentHead,
            PhaseStartSelection::Protected,
        );
        let overlap_selection = overlap_selection(
            before,
            after,
            defer,
            override_reservation,
            overlap_why.as_deref(),
            proposal.as_deref(),
        )?;
        let overlap_authorization = overlap_authorization_request(overlap_selection);
        Ok(ClaimRequest {
            declared_scopes,
            source,
            purpose,
            coordination_run_selection: run.map_or(
                ClaimCoordinationRunSelection::ContinueOrStart,
                |coordination_run_id| {
                    ClaimCoordinationRunSelection::Specified(
                        PresentedCoordinationRun::from_run_argument(coordination_run_id),
                    )
                },
            ),
            phase_start,
            overlap_authorization,
        })
    }
}

/// Convert the clap-owned overlap arguments into the one answer they encode.
fn overlap_selection(
    before: Option<ReservationId>,
    after: Option<ReservationId>,
    defer: Option<ReservationId>,
    override_reservation: Option<ReservationId>,
    overlap_why: Option<&str>,
    proposal: Option<&str>,
) -> Result<OverlapSelection, String> {
    let permissive_overlap_details = || {
        let authorization_reason = overlap_why
            .ok_or_else(|| "a permissive overlap answer requires --overlap-why".to_owned())?
            .parse::<OverlapAuthorizationReason>()
            .map_err(|error| error.to_string())?;
        let proposal_submission = match proposal {
            Some(token) => OverlapProposalSubmission::Apply(Box::new(
                token
                    .parse::<OverlapProposalToken>()
                    .map_err(|error| error.to_string())?,
            )),
            None => OverlapProposalSubmission::Issue,
        };
        Ok::<_, String>((authorization_reason, proposal_submission))
    };
    Ok(match (before, after, defer, override_reservation) {
        (None, None, None, None) => {
            if overlap_why.is_some() || proposal.is_some() {
                return Err(
                    "--overlap-why and --proposal require --before, --after, --defer, or --override"
                        .to_owned(),
                );
            }
            OverlapSelection::NoOverlapRequested
        },
        (Some(blocker_reservation_id), None, None, None) => {
            let (authorization_reason, proposal_submission) = permissive_overlap_details()?;
            OverlapSelection::RequesterBeforeHolder {
                blocker_reservation_id,
                authorization_reason,
                proposal_submission,
            }
        },
        (None, Some(blocker_reservation_id), None, None) => {
            let (authorization_reason, proposal_submission) = permissive_overlap_details()?;
            OverlapSelection::RequesterAfterHolder {
                blocker_reservation_id,
                authorization_reason,
                proposal_submission,
            }
        },
        (None, None, Some(blocker_reservation_id), None) => {
            let (authorization_reason, proposal_submission) = permissive_overlap_details()?;
            OverlapSelection::Defer {
                blocker_reservation_id,
                authorization_reason,
                proposal_submission,
            }
        },
        (None, None, None, Some(blocker_reservation_id)) => {
            let (authorization_reason, proposal_submission) = permissive_overlap_details()?;
            OverlapSelection::Override {
                blocker_reservation_id,
                authorization_reason,
                proposal_submission,
            }
        },
        _ => return Err("choose only one overlap answer".to_owned()),
    })
}

/// Whether and how the caller permits one reservation overlap.
enum OverlapSelection {
    /// The caller did not request a permissive overlap answer.
    NoOverlapRequested,
    /// The requester must integrate before the current reservation holder.
    RequesterBeforeHolder {
        blocker_reservation_id: ReservationId,
        authorization_reason:   OverlapAuthorizationReason,
        proposal_submission:    OverlapProposalSubmission,
    },
    /// The current reservation holder must integrate before the requester.
    RequesterAfterHolder {
        blocker_reservation_id: ReservationId,
        authorization_reason:   OverlapAuthorizationReason,
        proposal_submission:    OverlapProposalSubmission,
    },
    /// The requester defers its integration until the overlap is resolved.
    Defer {
        blocker_reservation_id: ReservationId,
        authorization_reason:   OverlapAuthorizationReason,
        proposal_submission:    OverlapProposalSubmission,
    },
    /// The requester proceeds despite the current reservation holder's overlap.
    Override {
        blocker_reservation_id: ReservationId,
        authorization_reason:   OverlapAuthorizationReason,
        proposal_submission:    OverlapProposalSubmission,
    },
}

fn overlap_authorization_request(selection: OverlapSelection) -> OverlapAuthorizationRequest {
    let (answer, reason, proposal_submission) = match selection {
        OverlapSelection::NoOverlapRequested => return OverlapAuthorizationRequest::Absent,
        OverlapSelection::RequesterBeforeHolder {
            blocker_reservation_id,
            authorization_reason,
            proposal_submission,
        } => (
            PermissiveOverlapAnswer::Sequence {
                blocker:   blocker_reservation_id,
                direction: OrderingDirection::RequesterBeforeHolder,
            },
            authorization_reason,
            proposal_submission,
        ),
        OverlapSelection::RequesterAfterHolder {
            blocker_reservation_id,
            authorization_reason,
            proposal_submission,
        } => (
            PermissiveOverlapAnswer::Sequence {
                blocker:   blocker_reservation_id,
                direction: OrderingDirection::HolderBeforeRequester,
            },
            authorization_reason,
            proposal_submission,
        ),
        OverlapSelection::Defer {
            blocker_reservation_id,
            authorization_reason,
            proposal_submission,
        } => (
            PermissiveOverlapAnswer::Defer {
                blocker: blocker_reservation_id,
            },
            authorization_reason,
            proposal_submission,
        ),
        OverlapSelection::Override {
            blocker_reservation_id,
            authorization_reason,
            proposal_submission,
        } => (
            PermissiveOverlapAnswer::Override {
                blocker: blocker_reservation_id,
            },
            authorization_reason,
            proposal_submission,
        ),
    };
    OverlapAuthorizationRequest::Permissive(Box::new(PermissiveOverlapAuthorizationRequest {
        answer,
        reason,
        proposal_submission,
    }))
}

impl ReservationArguments {
    const fn into_release_request(self) -> ReleaseRequest {
        ReleaseRequest {
            reservation_id: self.reservation_id,
        }
    }

    const fn into_renew_request(self) -> RenewRequest {
        RenewRequest {
            reservation_id: self.reservation_id,
        }
    }
}

impl DriftArguments {
    fn into_drift_request(self) -> DriftRequest {
        let comparison = if self.full {
            DriftComparisonChoice::FullPhaseStart
        } else {
            DriftComparisonChoice::CheapDelta
        };
        let reservation = match post_commit_hook_request() {
            PostCommitHookRequest::Requested => {
                DriftReservationSelection::EveryActiveForPostCommit {
                    widening: self.reservation.map_or(
                        PostCommitWideningSelection::SessionMappingOrSingleCandidate,
                        PostCommitWideningSelection::Explicit,
                    ),
                }
            },
            PostCommitHookRequest::NotRequested => self.reservation.map_or(
                DriftReservationSelection::SessionMappingOrSingleActive,
                DriftReservationSelection::Explicit,
            ),
        };
        DriftRequest {
            comparison,
            reservation,
        }
    }
}

impl SequenceArguments {
    fn into_sequence_request(self) -> Result<SequenceRequest, String> {
        let reason = self
            .why
            .parse::<OrderingReason>()
            .map_err(|error| error.to_string())?;
        Ok(SequenceRequest {
            first: self.first,
            then: self.then,
            reason,
        })
    }
}

impl IntegrateArguments {
    fn into_integrate_request(self) -> Result<IntegrateRequest, String> {
        let integration = match (self.force, self.why) {
            (false, None) => IntegrationRequest::EnforceOrdering,
            (true, Some(reason)) => reason
                .parse::<ForcedIntegrationReason>()
                .map(IntegrationRequest::ForceOnce)
                .map_err(|error| error.to_string())?,
            (true, None) | (false, Some(_)) => {
                return Err("--force and --why must be supplied together".to_owned());
            },
        };
        Ok(IntegrateRequest {
            reservation_id: self.reservation_id,
            integration,
        })
    }
}

impl IncursionAnswerSelection {
    /// Convert clap's incursion flags into the answer scope they name, if any.
    fn nomination(self) -> Result<IncursionAnswerNomination, String> {
        let Self {
            incursion,
            every_incursion,
        } = self;
        match (incursion, every_incursion) {
            (Some(incident_id), false) => Ok(IncursionAnswerNomination::AnswerScope(
                IncursionAnswerScope::One(incident_id),
            )),
            (None, true) => Ok(IncursionAnswerNomination::AnswerScope(
                IncursionAnswerScope::Every,
            )),
            (None, false) => Ok(IncursionAnswerNomination::NoIncursionAnswered),
            (Some(_), true) => Err(RESOLVE_DISPOSITION_REFUSAL.to_owned()),
        }
    }
}

impl ReservationRecoverySelection {
    /// Convert clap's recovery flags into the decision they name, if any.
    fn nomination(self) -> Result<ReservationRecoveryNomination, String> {
        let Self {
            recovered,
            integrated_as,
            abandon,
            retire_orphan,
            why,
        } = self;
        match (recovered, integrated_as, abandon, retire_orphan, why) {
            (false, None, false, false, None) => {
                Ok(ReservationRecoveryNomination::NoRecoveryRequested)
            },
            (true, None, false, false, None) => Ok(ReservationRecoveryNomination::Recovery(
                ReservationRecoveryDecision::Recovered,
            )),
            (false, Some(trunk_commit), false, false, None) => {
                Ok(ReservationRecoveryNomination::Recovery(
                    ReservationRecoveryDecision::IntegratedAs(trunk_commit),
                ))
            },
            (false, None, true, false, Some(reason)) => reason
                .parse::<AbandonmentReason>()
                .map(ReservationRecoveryDecision::Abandon)
                .map(ReservationRecoveryNomination::Recovery)
                .map_err(|error| error.to_string()),
            (false, None, false, true, Some(reason)) => reason
                .parse::<OrphanRetirementReason>()
                .map(ReservationRecoveryDecision::RetireOrphan)
                .map(ReservationRecoveryNomination::Recovery)
                .map_err(|error| error.to_string()),
            _ => Err(RESOLVE_DISPOSITION_REFUSAL.to_owned()),
        }
    }
}

impl ResolveArguments {
    fn into_resolve_request(self) -> Result<ResolveRequest, String> {
        let Self {
            reservation_id,
            incursion_answer,
            reservation_recovery,
            json_output: _,
        } = self;
        let decision = match (
            incursion_answer.nomination()?,
            reservation_recovery.nomination()?,
        ) {
            (
                IncursionAnswerNomination::AnswerScope(scope),
                ReservationRecoveryNomination::NoRecoveryRequested,
            ) => ResolveDecision::Incursion(scope),
            (
                IncursionAnswerNomination::NoIncursionAnswered,
                ReservationRecoveryNomination::Recovery(recovery),
            ) => ResolveDecision::Reservation(recovery),
            _ => return Err(RESOLVE_DISPOSITION_REFUSAL.to_owned()),
        };
        Ok(ResolveRequest {
            reservation_id,
            decision,
        })
    }
}

fn execute_identity_command(identity_command: &IdentityCommand) -> OutputEnvelope {
    match identity_command {
        IdentityCommand::ClearSession(_) => {
            let invocation_directory = match env::current_dir() {
                Ok(invocation_directory) => invocation_directory,
                Err(error) => {
                    return OutputEnvelope::ledger_unreadable(
                        CommandVerb::Identity,
                        &error.to_string(),
                    );
                },
            };
            let worktree_context =
                match crate::ledger::WorktreeContext::discover(&invocation_directory) {
                    Ok(worktree_context) => worktree_context,
                    Err(error) => {
                        return OutputEnvelope::ledger_error(CommandVerb::Identity, &error);
                    },
                };
            let ledger = match Ledger::open_from_discovered_worktree(&worktree_context) {
                Ok(ledger) => ledger,
                Err(error) => {
                    return OutputEnvelope::ledger_error(CommandVerb::Identity, &error);
                },
            };
            match ledger.remove_current_session_mapping() {
                Ok(removal) => OutputEnvelope::current_session_mapping_removed(removal),
                Err(error) => match LedgerTransactionError::from(error) {
                    LedgerTransactionError::LockContention => OutputEnvelope::contention(
                        CommandVerb::Identity,
                        &LedgerTransactionError::LockContention.to_string(),
                    ),
                    LedgerTransactionError::LedgerUnreadable(error) => {
                        OutputEnvelope::ledger_error(CommandVerb::Identity, &error)
                    },
                    LedgerTransactionError::CorrectableInput(error) => {
                        OutputEnvelope::invalid_input(CommandVerb::Identity, &error.to_string())
                    },
                },
            }
        },
    }
}

fn initialize_ledger(initialization_request: InitializationRequest) -> OutputEnvelope {
    match env::current_dir() {
        Ok(invocation_directory) => match git::repository_root(&invocation_directory) {
            Ok(repository_root) => match initialization_request {
                InitializationRequest::Initialize => match Ledger::initialize(&repository_root) {
                    Ok(initialization) => {
                        let worktree_context =
                            match crate::ledger::WorktreeContext::discover(&repository_root) {
                                Ok(worktree_context) => worktree_context,
                                Err(error) => return initialization_error(error),
                            };
                        let berth_config = match BerthConfig::read(&repository_root) {
                            Ok(Enrollment::Enrolled(berth_config)) => berth_config,
                            Ok(Enrollment::Unconfigured {
                                expected_configuration_path,
                            }) => {
                                return OutputEnvelope::unconfigured(
                                    CommandVerb::Init,
                                    &expected_configuration_path,
                                );
                            },
                            Err(error) => {
                                return OutputEnvelope::ledger_error(
                                    CommandVerb::Init,
                                    &LedgerError::Config(error),
                                );
                            },
                        };
                        let trunk_reference = format!("refs/heads/{}", berth_config.trunk);
                        let hook_installations = gate::install::install_managed_hooks(
                            worktree_context.common_git_directory(),
                            worktree_context.repository_root(),
                            &trunk_reference,
                        );
                        OutputEnvelope::initialized(initialization, &hook_installations)
                    },
                    Err(error) => initialization_error(error),
                },
                InitializationRequest::RepairProjection => {
                    match Ledger::repair_projection(&repository_root) {
                        Ok(()) => OutputEnvelope::projection_repaired(),
                        Err(error) => initialization_error(error),
                    }
                },
                InitializationRequest::ReinitializeAfterReview => {
                    let worktree_context =
                        match crate::ledger::WorktreeContext::discover(&repository_root) {
                            Ok(worktree_context) => worktree_context,
                            Err(error) => return initialization_error(error),
                        };
                    let pending_environment_bypasses =
                        match gate::permit::pending_environment_bypass_count(
                            worktree_context.common_git_directory(),
                        ) {
                            Ok(count) => count,
                            Err(error) => {
                                return OutputEnvelope::ledger_unreadable(
                                    CommandVerb::Init,
                                    &error.to_string(),
                                );
                            },
                        };
                    match Ledger::reinitialize_after_review(&repository_root) {
                        Ok(reinitialization) => OutputEnvelope::reinitialized(
                            reinitialization.discarded_bytes,
                            reinitialization.discarded_complete_records,
                            pending_environment_bypasses,
                        ),
                        Err(error) => initialization_error(error),
                    }
                },
            },
            Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Init, &error.to_string()),
        },
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Init, &error.to_string()),
    }
}

/// Writes a reference-transaction diagnostic as best effort so a stderr failure
/// cannot change the gate's exit code.
fn write_reference_transaction_diagnostic(arguments: Arguments<'_>) {
    let _ = writeln!(std::io::stderr().lock(), "{arguments}");
}

fn run_reference_transaction(
    phase: ReferenceTransactionPhase,
    trunk_reference: FullRefName,
) -> ExitCode {
    const TOTAL_GATE_DEADLINE: Duration = Duration::from_secs(10);

    if gate::permit::environment_bypass_requested() {
        return run_environment_bypassed_reference_transaction(phase, &trunk_reference);
    }
    let started_at = std::time::Instant::now();
    let transaction = match read_reference_transaction(phase) {
        Ok(transaction) => transaction,
        Err(ReferenceTransactionInputError::StandardInputUnreadable(error)) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth trunk gate could not read git's transaction: {error}. To proceed anyway, rerun the git command with CARGO_BERTH_BYPASS=1."
            ));
            return BerthExit::LedgerUnreadable.into();
        },
        Err(ReferenceTransactionInputError::MalformedHookInput(error)) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth trunk gate rejected malformed hook input: {error}. To proceed anyway, rerun the git command with CARGO_BERTH_BYPASS=1."
            ));
            return BerthExit::UsageError.into();
        },
    };
    let managed_trunk_deletion = transaction.managed_trunk_deletion(&trunk_reference);
    let invocation_directory = match env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth trunk gate could not resolve its invocation directory: {error}. To proceed anyway, rerun the git command with CARGO_BERTH_BYPASS=1."
            ));
            return BerthExit::LedgerUnreadable.into();
        },
    };
    let issuing_directory = env::var_os(REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT)
        .map_or(
            ReferenceTransactionIssuingDirectory::MissingFromLegacyHook,
            |issuing_directory| {
                ReferenceTransactionIssuingDirectory::CapturedByManagedHook(PathBuf::from(
                    issuing_directory,
                ))
            },
        );
    let remaining = TOTAL_GATE_DEADLINE.saturating_sub(started_at.elapsed());
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let gate_invocation_directory = invocation_directory.clone();
    let gate_worker = std::thread::spawn(move || {
        let result = gate::evaluate_reference_transaction(
            &gate_invocation_directory,
            &issuing_directory,
            &transaction,
            &trunk_reference,
        );
        let _ = sender.send(result);
    });
    let results = match receiver.recv_timeout(remaining) {
        Ok(Ok(results)) => results,
        Ok(Err(error)) => return reference_transaction_error(&error),
        Err(RecvTimeoutError::Timeout) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth trunk gate exhausted its 10-second total deadline; no integration decision was made. Retry the git command, or set CARGO_BERTH_BYPASS=1 to proceed immediately."
            ));
            return BerthExit::BlockedByContention.into();
        },
        Err(RecvTimeoutError::Disconnected) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth trunk gate stopped before making a decision. Retry the git command, or set CARGO_BERTH_BYPASS=1 to proceed immediately."
            ));
            return BerthExit::LedgerUnreadable.into();
        },
    };
    if gate_worker.join().is_err() {
        write_reference_transaction_diagnostic(format_args!(
            "cargo-berth trunk gate stopped after reporting its decision. Retry the git command, or set CARGO_BERTH_BYPASS=1 to proceed immediately."
        ));
        return BerthExit::LedgerUnreadable.into();
    }
    schedule_managed_hook_refresh_after_trunk_deletion(
        &invocation_directory,
        managed_trunk_deletion,
    );
    exit_for_reference_transaction_results(results)
}

fn schedule_managed_hook_refresh_after_trunk_deletion(
    invocation_directory: &Path,
    deletion: ManagedTrunkDeletion,
) {
    let ManagedTrunkDeletion::Deleted {
        reference: deleted_reference,
        previous_tip,
    } = deletion
    else {
        return;
    };
    let executable = match env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth could not schedule its managed-hook refresh after {deleted_reference} was deleted: {error}. The stale hook will invoke cargo-berth defensively until cargo berth init refreshes it."
            ));
            return;
        },
    };
    let deleted_reference = deleted_reference.to_string();
    let previous_tip = previous_tip.to_string();
    match ProcessCommand::new(executable)
        .args([
            "__refresh-managed-hook-after-trunk-deletion",
            &deleted_reference,
            &previous_tip,
        ])
        .current_dir(invocation_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(refresh_worker) => drop(refresh_worker),
        Err(error) => write_reference_transaction_diagnostic(format_args!(
            "cargo-berth could not schedule its managed-hook refresh after {deleted_reference} was deleted: {error}. The stale hook will invoke cargo-berth defensively until cargo berth init refreshes it."
        )),
    }
}

fn run_managed_hook_refresh_worker(arguments: &ManagedHookRefreshArguments) -> ExitCode {
    let invocation_directory = match env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth could not refresh its managed hook after {} was deleted: {error}",
                arguments.deleted_reference
            ));
            return BerthExit::Clear.into();
        },
    };
    refresh_managed_hook_after_trunk_deletion(
        &invocation_directory,
        &arguments.deleted_reference,
        &arguments.previous_tip,
    );
    BerthExit::Clear.into()
}

fn refresh_managed_hook_after_trunk_deletion(
    invocation_directory: &Path,
    deleted_reference: &FullRefName,
    previous_tip: &GitObjectId,
) {
    const MAXIMUM_REPLACEMENT_LOOKUP_ATTEMPTS: usize = 20;
    const REPLACEMENT_LOOKUP_RETRY_INTERVAL: Duration = Duration::from_millis(10);

    let worktree_context = match crate::ledger::WorktreeContext::discover(invocation_directory) {
        Ok(worktree_context) => worktree_context,
        Err(error) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth could not inspect the repository after {deleted_reference} was deleted: {error}. The stale hook will invoke cargo-berth defensively until cargo berth init refreshes it."
            ));
            return;
        },
    };
    let mut renamed_reference = LocalBranchRenameTargetResolution::NotProven;
    for attempt in 0..MAXIMUM_REPLACEMENT_LOOKUP_ATTEMPTS {
        renamed_reference = match git::local_branch_rename_target_resolution(
            worktree_context.repository_root(),
            previous_tip,
            deleted_reference,
        ) {
            Ok(LocalBranchRenameTargetResolution::NotProven)
                if attempt + 1 < MAXIMUM_REPLACEMENT_LOOKUP_ATTEMPTS =>
            {
                std::thread::sleep(REPLACEMENT_LOOKUP_RETRY_INTERVAL);
                continue;
            },
            Ok(matches) => matches,
            Err(error) => {
                write_reference_transaction_diagnostic(format_args!(
                    "cargo-berth could not verify a reflog rename from {deleted_reference}: {error}. The stale hook will invoke cargo-berth defensively until cargo berth init refreshes it."
                ));
                return;
            },
        };
        break;
    }
    let renamed_reference = match renamed_reference {
        LocalBranchRenameTargetResolution::Unique(reference) => reference,
        LocalBranchRenameTargetResolution::NotProven => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth found no local branch with reflog proof that it was renamed from {deleted_reference}. The stale hook will invoke cargo-berth defensively until cargo berth init refreshes it."
            ));
            return;
        },
        LocalBranchRenameTargetResolution::Ambiguous => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth found multiple local branches with reflog proof that they were renamed from {deleted_reference}. The stale hook will invoke cargo-berth defensively until cargo berth init refreshes it."
            ));
            return;
        },
    };
    let installations = gate::install::install_managed_hooks(
        worktree_context.common_git_directory(),
        worktree_context.repository_root(),
        &renamed_reference.to_string(),
    );
    let refresh_failed = installations.iter().find(|installation| {
        installation.name() == "reference-transaction"
            && !matches!(
                installation.activation(),
                gate::install::ManagedHookActivationOutcome::Active { .. }
            )
    });
    if let Some(refresh_failed) = refresh_failed {
        write_reference_transaction_diagnostic(format_args!(
            "cargo-berth could not refresh its {} hook after the trunk ref was renamed to {renamed_reference}: {:?}. The existing hook was left unchanged; rerun cargo berth init after correcting the installation failure.",
            refresh_failed.name(),
            refresh_failed.activation(),
        ));
    }
}

fn read_reference_transaction(
    phase: ReferenceTransactionPhase,
) -> Result<ReferenceTransaction, ReferenceTransactionInputError> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(ReferenceTransactionInputError::StandardInputUnreadable)?;
    gate::parse_reference_transaction(phase, &input)
        .map_err(ReferenceTransactionInputError::MalformedHookInput)
}

fn run_environment_bypassed_reference_transaction(
    phase: ReferenceTransactionPhase,
    trunk_reference: &FullRefName,
) -> ExitCode {
    if phase == ReferenceTransactionPhase::Committed {
        if let Ok(transaction) = read_reference_transaction(phase) {
            let deletion = transaction.managed_trunk_deletion(trunk_reference);
            if let Ok(invocation_directory) = env::current_dir() {
                schedule_managed_hook_refresh_after_trunk_deletion(&invocation_directory, deletion);
            }
        }
        return BerthExit::Clear.into();
    }
    if phase != ReferenceTransactionPhase::Prepared {
        return BerthExit::Clear.into();
    }
    match bypass_transaction_trunk_relation(phase, trunk_reference) {
        BypassTransactionTrunkRelation::Named => {
            retain_environment_bypass_audit(EnvironmentBypassAuditBasis::ConfirmedTrunkReference)
        },
        BypassTransactionTrunkRelation::NotNamed => BerthExit::Clear.into(),
        BypassTransactionTrunkRelation::Unconfirmed(error) => retain_environment_bypass_audit(
            EnvironmentBypassAuditBasis::UnconfirmedTrunkReference(error),
        ),
    }
}

fn bypass_transaction_trunk_relation(
    phase: ReferenceTransactionPhase,
    trunk_reference: &FullRefName,
) -> BypassTransactionTrunkRelation {
    match read_reference_transaction(phase) {
        Ok(transaction) => match transaction.trunk_reference_presence(trunk_reference) {
            TrunkReferencePresence::Named => BypassTransactionTrunkRelation::Named,
            TrunkReferencePresence::NotNamed => BypassTransactionTrunkRelation::NotNamed,
        },
        Err(error) => BypassTransactionTrunkRelation::Unconfirmed(error),
    }
}

fn retain_environment_bypass_audit(audit_basis: EnvironmentBypassAuditBasis) -> ExitCode {
    let invocation_directory = match env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            match audit_basis {
                EnvironmentBypassAuditBasis::ConfirmedTrunkReference => {
                    write_reference_transaction_diagnostic(format_args!(
                        "cargo-berth took the CARGO_BERTH_BYPASS=1 override but could not resolve its invocation directory: {error}. The override could not be recorded here; this ref transaction remains permitted, and a marker is being left to report it later. Enter the repository from an existing directory, then rerun cargo berth init."
                    ));
                },
                EnvironmentBypassAuditBasis::UnconfirmedTrunkReference(input_error) => {
                    write_reference_transaction_diagnostic(format_args!(
                        "cargo-berth took the CARGO_BERTH_BYPASS=1 override but could not confirm whether the transaction named the trunk reference because {input_error}. It also could not resolve its invocation directory: {error}. The override could not be recorded here; this ref transaction remains permitted, and a marker is being left to report it later. Enter the repository from an existing directory, then rerun cargo berth init."
                    ));
                },
            }
            return BerthExit::LedgerUnreadable.into();
        },
    };
    let retention = gate::permit::record_environment_bypass(&invocation_directory);
    match (audit_basis, retention) {
        (
            EnvironmentBypassAuditBasis::ConfirmedTrunkReference,
            EnvironmentBypassRetentionOutcome::Journalled
            | EnvironmentBypassRetentionOutcome::PendingMarker
            | EnvironmentBypassRetentionOutcome::Unenrolled,
        ) => BerthExit::Clear.into(),
        (
            EnvironmentBypassAuditBasis::ConfirmedTrunkReference,
            EnvironmentBypassRetentionOutcome::Unrecorded,
        ) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth took the CARGO_BERTH_BYPASS=1 override, but neither the journal nor a pending marker retained its audit fact. This ref transaction remains permitted. Restore repository write access, then rerun cargo berth init."
            ));
            BerthExit::LedgerUnreadable.into()
        },
        (
            EnvironmentBypassAuditBasis::UnconfirmedTrunkReference(input_error),
            EnvironmentBypassRetentionOutcome::Journalled
            | EnvironmentBypassRetentionOutcome::PendingMarker,
        ) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth took the CARGO_BERTH_BYPASS=1 override but could not confirm whether the transaction named the trunk reference because {input_error}. The audit fact was retained without confirming the ref; this ref transaction remains permitted."
            ));
            BerthExit::Clear.into()
        },
        (
            EnvironmentBypassAuditBasis::UnconfirmedTrunkReference(input_error),
            EnvironmentBypassRetentionOutcome::Unenrolled,
        ) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth took the CARGO_BERTH_BYPASS=1 override but could not confirm whether the transaction named the trunk reference because {input_error}. The repository is not enrolled, so no shared audit destination applies; this ref transaction remains permitted."
            ));
            BerthExit::Clear.into()
        },
        (
            EnvironmentBypassAuditBasis::UnconfirmedTrunkReference(input_error),
            EnvironmentBypassRetentionOutcome::Unrecorded,
        ) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth took the CARGO_BERTH_BYPASS=1 override but could not confirm whether the transaction named the trunk reference because {input_error}, and neither the journal nor a pending marker retained the audit fact. The override could not be recorded here; this ref transaction remains permitted, and a marker is being left to report it later. Restore repository write access, then rerun cargo berth init."
            ));
            BerthExit::LedgerUnreadable.into()
        },
    }
}

fn exit_for_reference_transaction_results(results: Vec<GateResult>) -> ExitCode {
    let mut blocked = false;
    for result in results {
        match result.decision {
            GateDecision::Observed {
                generation,
                violations,
            } => {
                for violation in violations {
                    let reservation_id = violation.reservation.reservation_id;
                    let rendered = OutputEnvelope::integration_blocked(
                        reservation_id,
                        generation,
                        vec![violation],
                    )
                    .with_alerts(result.alerts.clone())
                    .render_text();
                    write_reference_transaction_diagnostic(format_args!(
                        "Observe-only cargo-berth trunk gate: {rendered}"
                    ));
                }
            },
            GateDecision::Blocked {
                generation,
                violations,
            } => {
                blocked = true;
                for violation in violations {
                    let reservation_id = violation.reservation.reservation_id;
                    write_reference_transaction_diagnostic(format_args!(
                        "{}",
                        OutputEnvelope::integration_blocked(
                            reservation_id,
                            generation,
                            vec![violation],
                        )
                        .with_alerts(result.alerts.clone())
                        .render_text()
                    ));
                }
            },
            GateDecision::Clear { .. }
            | GateDecision::PermitIssued { .. }
            | GateDecision::Forced { .. } => {},
        }
    }
    if blocked {
        BerthExit::BlockedByOrdering.into()
    } else {
        BerthExit::Clear.into()
    }
}

fn reference_transaction_error(error: &GateError) -> ExitCode {
    match error {
        GateError::Config(_) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth trunk gate could not read its configuration, so it could not check this possible trunk update: {error}. The ref transaction was permitted. Restore the configuration; CARGO_BERTH_BYPASS=1 remains the explicit override."
            ));
            BerthExit::Clear.into()
        },
        GateError::Transaction(LedgerTransactionError::LockContention) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth trunk gate exhausted its 10-second lock deadline; the ledger was busy and no integration decision was made. Retry the git command, or set CARGO_BERTH_BYPASS=1 to proceed immediately."
            ));
            BerthExit::BlockedByContention.into()
        },
        GateError::LegacyReferenceTransactionHook => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth trunk gate refused this ref transaction because the managed reference-transaction hook did not report the issuing worktree. Run cargo-berth init to reinstall the hook, then retry the git command. To proceed immediately, rerun the git command with CARGO_BERTH_BYPASS=1."
            ));
            BerthExit::UsageError.into()
        },
        GateError::CoordinationIdentity(_)
        | GateError::ReservationNotEntering(_)
        | GateError::NoHoldToForce(_)
        | GateError::MissingSkippedHold
        | GateError::Transaction(LedgerTransactionError::CorrectableInput(_)) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth trunk gate rejected invalid input: {error}. To proceed anyway, rerun the git command with CARGO_BERTH_BYPASS=1."
            ));
            BerthExit::UsageError.into()
        },
        GateError::Ledger(_)
        | GateError::Transaction(LedgerTransactionError::LedgerUnreadable(_))
        | GateError::Reconciliation(_)
        | GateError::Planning(_)
        | GateError::MissingConstraintFact(_)
        | GateError::UnsupportedSymbolicTrunkUpdate
        | GateError::Git(_)
        | GateError::PermitReplay(_) => {
            write_reference_transaction_diagnostic(format_args!(
                "cargo-berth trunk gate could not prove this integration safe: {error}. To proceed anyway, rerun the git command with CARGO_BERTH_BYPASS=1."
            ));
            BerthExit::LedgerUnreadable.into()
        },
    }
}

fn initialization_error(error: LedgerError) -> OutputEnvelope {
    match LedgerTransactionError::from(error) {
        LedgerTransactionError::LockContention => OutputEnvelope::contention(
            CommandVerb::Init,
            &LedgerTransactionError::LockContention.to_string(),
        ),
        LedgerTransactionError::LedgerUnreadable(error) => {
            OutputEnvelope::ledger_error(CommandVerb::Init, &error)
        },
        LedgerTransactionError::CorrectableInput(error) => {
            OutputEnvelope::invalid_input(CommandVerb::Init, &error.to_string())
        },
    }
}

/// Render one command's response and report the exit status its process publishes.
///
/// The envelope's own exit status is what the process returns, so the rendering and the
/// status it publishes are decided in one place rather than agreeing by convention.
fn publish_envelope_response(
    output_format: CliOutputFormat,
    output_envelope: &OutputEnvelope,
) -> BerthExit {
    match command_response_rendering(output_format, post_commit_hook_request()) {
        CommandResponseRendering::OutputEnvelope(output_format) => {
            emit_response(output_format, output_envelope);
        },
        CommandResponseRendering::PostCommitWarning => {
            emit_post_commit_response(output_envelope);
        },
    }
    output_envelope.exit_code
}

fn emit_response(output_format: CliOutputFormat, output_envelope: &OutputEnvelope) {
    let rendered = match output_format {
        CliOutputFormat::Json => match serde_json::to_string(output_envelope) {
            Ok(rendered) => rendered,
            Err(_) => return,
        },
        CliOutputFormat::Text => output_envelope.render_text(),
    };
    write_line(rendered);
}

fn emit_post_commit_response(output_envelope: &OutputEnvelope) {
    match output_envelope.post_commit_rendering() {
        PostCommitRendering::Silent => {},
        PostCommitRendering::Warning(warning) => {
            let _ = writeln!(std::io::stderr().lock(), "{warning}");
        },
    }
}

const fn command_response_rendering(
    output_format: CliOutputFormat,
    post_commit_hook_request: PostCommitHookRequest,
) -> CommandResponseRendering {
    match (output_format, post_commit_hook_request) {
        (CliOutputFormat::Text, PostCommitHookRequest::Requested) => {
            CommandResponseRendering::PostCommitWarning
        },
        (output_format, _) => CommandResponseRendering::OutputEnvelope(output_format),
    }
}

fn post_commit_hook_request() -> PostCommitHookRequest {
    if env::var_os(POST_COMMIT_HOOK_ENVIRONMENT).is_some_and(|value| value == "1") {
        PostCommitHookRequest::Requested
    } else {
        PostCommitHookRequest::NotRequested
    }
}

fn write_line(mut rendered: String) {
    rendered.push('\n');
    let standard_output = std::io::stdout();
    let mut standard_output = standard_output.lock();
    std::mem::drop(standard_output.write_all(rendered.as_bytes()));
}

/// Decide the exit status required by a clap parser error.
fn exit_for_parser_error(error: &Error) -> BerthExit {
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => BerthExit::Clear,
        _ => BerthExit::UsageError,
    }
}

/// Remove cargo's injected subcommand name from the first argument position.
fn without_subcommand_name(mut arguments: Vec<OsString>) -> Vec<OsString> {
    if arguments
        .get(1)
        .is_some_and(|argument| argument.as_os_str() == OsStr::new(CARGO_SUBCOMMAND_NAME))
    {
        arguments.remove(1);
    }
    arguments
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::ffi::OsString;
    use std::path::Path;
    use std::path::PathBuf;

    use clap::Error;
    use clap::Parser;
    use clap::error::ErrorKind;

    use super::BINARY_NAME;
    use super::CARGO_SUBCOMMAND_NAME;
    use super::ClaimArguments;
    use super::Cli;
    use super::CliOutputFormat;
    use super::Command;
    use super::CommandLineRoute;
    use super::CommandOutputOwnership;
    use super::CommandResponseRendering;
    use super::CommandResultReporting;
    use super::CommandVerb;
    use super::HookCommand;
    use super::PostCommitHookRequest;
    use super::command_response_rendering;
    use super::exit_for_parser_error;
    use super::without_subcommand_name;
    use crate::coordination_identity::RecoveryCommandLine;
    use crate::exit::BerthExit;
    use crate::output;
    use crate::verb::board::BoardOutputSelection;
    use crate::verb::claim::ClaimRequest;

    const RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
    const SECOND_RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
    const TRUNK_REFERENCE: &str = "refs/heads/main";
    const TRUNK_TIP: &str = "292471ef2254a985665228c46355571f54e4148a";

    impl CommandLineRoute {
        /// Every route the frozen interface publishes a result through, one row each.
        const ALL: [Self; 16] = [
            Self::Init,
            Self::Board,
            Self::Check,
            Self::HookPreToolUse,
            Self::HookPostToolUse,
            Self::HookSessionStart,
            Self::Claim,
            Self::Drift,
            Self::Release,
            Self::Sequence,
            Self::Integrate,
            Self::Resolve,
            Self::Renew,
            Self::IdentityClearSession,
            Self::ReferenceTransaction,
            Self::RefreshManagedHookAfterTrunkDeletion,
        ];

        /// One accepted command line that selects this route.
        fn arguments(self) -> Vec<&'static str> {
            match self {
                Self::Init => vec![BINARY_NAME, "init", "--json"],
                Self::Board => vec![BINARY_NAME, "board", "--json"],
                Self::Check => vec![BINARY_NAME, "check", "src/lib.rs", "--json"],
                Self::HookPreToolUse => vec![BINARY_NAME, "hook", "pre-tool-use"],
                Self::HookPostToolUse => vec![BINARY_NAME, "hook", "post-tool-use"],
                Self::HookSessionStart => vec![BINARY_NAME, "hook", "session-start"],
                Self::Claim => vec![
                    BINARY_NAME,
                    "claim",
                    "src/lib.rs",
                    "--why",
                    "protect the implementation",
                    "--json",
                ],
                Self::Drift => vec![BINARY_NAME, "drift", "--json"],
                Self::Release => vec![BINARY_NAME, "release", RESERVATION_ID, "--json"],
                Self::Sequence => vec![
                    BINARY_NAME,
                    "sequence",
                    RESERVATION_ID,
                    SECOND_RESERVATION_ID,
                    "--why",
                    "the holder API must land first",
                    "--json",
                ],
                Self::Integrate => vec![BINARY_NAME, "integrate", RESERVATION_ID, "--json"],
                Self::Resolve => {
                    vec![
                        BINARY_NAME,
                        "resolve",
                        RESERVATION_ID,
                        "--recovered",
                        "--json",
                    ]
                },
                Self::Renew => vec![BINARY_NAME, "renew", RESERVATION_ID, "--json"],
                Self::IdentityClearSession => {
                    vec![BINARY_NAME, "identity", "clear-session", "--json"]
                },
                Self::ReferenceTransaction => vec![
                    BINARY_NAME,
                    "__reference-transaction",
                    "committed",
                    TRUNK_REFERENCE,
                ],
                Self::RefreshManagedHookAfterTrunkDeletion => vec![
                    BINARY_NAME,
                    "__refresh-managed-hook-after-trunk-deletion",
                    TRUNK_REFERENCE,
                    TRUNK_TIP,
                ],
            }
        }

        /// Where this route states that its result reaches the caller.
        const fn declared_reporting(self) -> CommandResultReporting {
            match self {
                Self::Init => CommandResultReporting::Envelope(CommandVerb::Init),
                Self::Board => CommandResultReporting::Envelope(CommandVerb::Board),
                Self::Check => CommandResultReporting::Envelope(CommandVerb::Check),
                Self::HookPreToolUse => {
                    CommandResultReporting::HookProtocol(HookCommand::PreToolUse)
                },
                Self::HookPostToolUse => {
                    CommandResultReporting::HookProtocol(HookCommand::PostToolUse)
                },
                Self::HookSessionStart => {
                    CommandResultReporting::HookProtocol(HookCommand::SessionStart)
                },
                Self::Claim => CommandResultReporting::Envelope(CommandVerb::Claim),
                Self::Drift => CommandResultReporting::Envelope(CommandVerb::Drift),
                Self::Release => CommandResultReporting::Envelope(CommandVerb::Release),
                Self::Sequence => CommandResultReporting::Envelope(CommandVerb::Sequence),
                Self::Integrate => CommandResultReporting::Envelope(CommandVerb::Integrate),
                Self::Resolve => CommandResultReporting::Envelope(CommandVerb::Resolve),
                Self::Renew => CommandResultReporting::Envelope(CommandVerb::Renew),
                Self::IdentityClearSession => {
                    CommandResultReporting::Envelope(CommandVerb::Identity)
                },
                Self::ReferenceTransaction | Self::RefreshManagedHookAfterTrunkDeletion => {
                    CommandResultReporting::GitHookProtocol
                },
            }
        }
    }

    #[derive(Clone, Copy)]
    enum ExpectedClaimResolution {
        Before,
        After,
        Defer,
        Override,
    }

    impl ExpectedClaimResolution {
        const ALL: [Self; 4] = [Self::Before, Self::After, Self::Defer, Self::Override];

        const fn flag(self) -> &'static str {
            match self {
                Self::Before => "--before",
                Self::After => "--after",
                Self::Defer => "--defer",
                Self::Override => "--override",
            }
        }

        fn is_the_only_selected_resolution(self, claim_arguments: &ClaimArguments) -> bool {
            let before_is_selected = claim_arguments.before.is_some();
            let after_is_selected = claim_arguments.after.is_some();
            let defer_is_selected = claim_arguments.defer.is_some();
            let override_is_selected = claim_arguments.override_reservation.is_some();

            match self {
                Self::Before => {
                    before_is_selected
                        && !after_is_selected
                        && !defer_is_selected
                        && !override_is_selected
                },
                Self::After => {
                    !before_is_selected
                        && after_is_selected
                        && !defer_is_selected
                        && !override_is_selected
                },
                Self::Defer => {
                    !before_is_selected
                        && !after_is_selected
                        && defer_is_selected
                        && !override_is_selected
                },
                Self::Override => {
                    !before_is_selected
                        && !after_is_selected
                        && !defer_is_selected
                        && override_is_selected
                },
            }
        }
    }

    #[derive(Clone, Copy, Eq, PartialEq)]
    enum ShellQuoteState {
        Unquoted,
        SingleQuoted,
        DoubleQuoted,
    }

    #[test]
    fn post_commit_request_renders_text_as_warning_and_json_as_envelope() {
        assert_eq!(
            command_response_rendering(CliOutputFormat::Text, PostCommitHookRequest::Requested,),
            CommandResponseRendering::PostCommitWarning,
        );
        assert_eq!(
            command_response_rendering(CliOutputFormat::Json, PostCommitHookRequest::Requested,),
            CommandResponseRendering::OutputEnvelope(CliOutputFormat::Json),
        );
    }

    #[test]
    fn every_verb_parses_under_both_spellings() {
        assert_verb_parses(
            &[BINARY_NAME, "init", "--json"],
            &[BINARY_NAME, CARGO_SUBCOMMAND_NAME, "init", "--json"],
            CommandVerb::Init,
        );
        assert_verb_parses(
            &[BINARY_NAME, "board", "--json"],
            &[BINARY_NAME, CARGO_SUBCOMMAND_NAME, "board", "--json"],
            CommandVerb::Board,
        );
        assert_verb_parses(
            &[BINARY_NAME, "check", "src", "--json"],
            &[BINARY_NAME, CARGO_SUBCOMMAND_NAME, "check", "src", "--json"],
            CommandVerb::Check,
        );
        assert_verb_parses(
            &[BINARY_NAME, "claim", "src", "--json"],
            &[BINARY_NAME, CARGO_SUBCOMMAND_NAME, "claim", "src", "--json"],
            CommandVerb::Claim,
        );
        assert_verb_parses(
            &[BINARY_NAME, "drift", "--full", "--json"],
            &[
                BINARY_NAME,
                CARGO_SUBCOMMAND_NAME,
                "drift",
                "--full",
                "--json",
            ],
            CommandVerb::Drift,
        );
        assert_verb_parses(
            &[BINARY_NAME, "release", RESERVATION_ID, "--json"],
            &[
                BINARY_NAME,
                CARGO_SUBCOMMAND_NAME,
                "release",
                RESERVATION_ID,
                "--json",
            ],
            CommandVerb::Release,
        );
        assert_verb_parses(
            &[
                BINARY_NAME,
                "sequence",
                RESERVATION_ID,
                RESERVATION_ID,
                "--why",
                "order",
                "--json",
            ],
            &[
                BINARY_NAME,
                CARGO_SUBCOMMAND_NAME,
                "sequence",
                RESERVATION_ID,
                RESERVATION_ID,
                "--why",
                "order",
                "--json",
            ],
            CommandVerb::Sequence,
        );
        assert_verb_parses(
            &[
                BINARY_NAME,
                "integrate",
                RESERVATION_ID,
                "--force",
                "--why",
                "authorized",
                "--json",
            ],
            &[
                BINARY_NAME,
                CARGO_SUBCOMMAND_NAME,
                "integrate",
                RESERVATION_ID,
                "--force",
                "--why",
                "authorized",
                "--json",
            ],
            CommandVerb::Integrate,
        );
    }

    #[test]
    fn named_reservation_board_requires_json_and_becomes_a_domain_selection() {
        assert!(
            board_output_selection(&[BINARY_NAME, "board", "--reservation", RESERVATION_ID,])
                .is_err()
        );
        assert!(matches!(
            board_output_selection(&[
                BINARY_NAME,
                "board",
                "--reservation",
                RESERVATION_ID,
                "--json",
            ]),
            Ok(BoardOutputSelection::ReservationLifecycleFor(reservation_id))
                if reservation_id.to_string() == RESERVATION_ID
        ));
        assert!(matches!(
            board_output_selection(&[BINARY_NAME, "board", "--json"]),
            Ok(BoardOutputSelection::CompleteBoard)
        ));
    }

    #[test]
    fn recovery_verbs_parse_under_both_spellings() {
        assert_verb_parses(
            &[
                BINARY_NAME,
                "resolve",
                RESERVATION_ID,
                "--recovered",
                "--json",
            ],
            &[
                BINARY_NAME,
                CARGO_SUBCOMMAND_NAME,
                "resolve",
                RESERVATION_ID,
                "--recovered",
                "--json",
            ],
            CommandVerb::Resolve,
        );
        assert_verb_parses(
            &[BINARY_NAME, "renew", RESERVATION_ID, "--json"],
            &[
                BINARY_NAME,
                CARGO_SUBCOMMAND_NAME,
                "renew",
                RESERVATION_ID,
                "--json",
            ],
            CommandVerb::Renew,
        );
    }

    #[test]
    fn resolve_requires_exactly_one_complete_disposition() {
        assert!(parsed_verb(&[BINARY_NAME, "resolve", RESERVATION_ID]).is_err());
        assert!(
            parsed_verb(&[
                BINARY_NAME,
                "resolve",
                RESERVATION_ID,
                "--recovered",
                "--integrated-as",
                "abc123",
            ])
            .is_err()
        );
        assert!(parsed_verb(&[BINARY_NAME, "resolve", RESERVATION_ID, "--abandon"]).is_err());
        assert!(
            parsed_verb(&[
                BINARY_NAME,
                "resolve",
                RESERVATION_ID,
                "--every-incursion",
                "--recovered",
            ])
            .is_err()
        );
    }

    #[test]
    fn recovery_help_explains_every_disposition_and_cost() {
        let resolve_help = help_for("resolve");
        let renew_help = help_for("renew");

        for required_text in [
            "--recovered",
            "--integrated-as",
            "--abandon",
            "discards work",
            "asserts evidence",
        ] {
            assert!(resolve_help.contains(required_text));
        }
        assert!(renew_help.contains("changes neither its scopes nor any ordering edge"));
    }

    #[test]
    fn operational_help_explains_board_streams_check_defaults_and_force_scope() {
        let board_help = help_for("board");
        let check_help = help_for("check");
        let integrate_help = help_for("integrate");

        assert!(board_help.contains("both standard input and standard output"));
        assert!(board_help.contains("Use --json to emit board facts"));
        assert!(check_help.contains("An unprefixed path means one exact file"));
        assert!(check_help.contains("tree: to include all component descendants"));
        assert!(integrate_help.contains("held ordering edges and unresolved deferrals"));
    }

    #[test]
    fn every_permissive_claim_answer_requires_its_distinct_overlap_reason() {
        for answer in ["--before", "--after", "--defer", "--override"] {
            assert!(parsed_verb(&[BINARY_NAME, "claim", "src", answer, RESERVATION_ID]).is_err());
            assert!(
                parsed_verb(&[
                    BINARY_NAME,
                    "claim",
                    "src",
                    answer,
                    RESERVATION_ID,
                    "--why",
                    "protect the implementation",
                ])
                .is_err()
            );
            assert!(
                claim_request(&[
                    BINARY_NAME,
                    "claim",
                    "src",
                    answer,
                    RESERVATION_ID,
                    "--why",
                    "protect the implementation",
                    "--overlap-why",
                    "the two changes are coordinated",
                ])
                .is_ok()
            );
        }
    }

    #[test]
    fn rendered_overlap_answer_commands_select_the_documented_resolution() -> Result<(), String> {
        let mut rendered_commands = output::blocked_edit_answer_guidance()
            .lines()
            .filter(|line| {
                line.chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_digit())
            })
            .filter_map(|line| line.split('`').nth(1))
            .filter(|command| command.split_whitespace().nth(1) == Some("claim"));

        for expected_resolution in ExpectedClaimResolution::ALL {
            let rendered_command = rendered_commands.next().ok_or_else(|| {
                format!(
                    "rendered guidance omitted the `{}` overlap answer command",
                    expected_resolution.flag()
                )
            })?;
            let claim_arguments = parse_rendered_claim_command(rendered_command)?;

            assert!(
                claim_arguments
                    .overlap_why
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty()),
                "rendered overlap answer command `{rendered_command}` does not contain a non-empty \
                 `--overlap-why` value"
            );
            assert!(
                expected_resolution.is_the_only_selected_resolution(&claim_arguments),
                "rendered overlap answer command `{rendered_command}` must select only `{}`; found \
                 --before={}, --after={}, --defer={}, --override={}",
                expected_resolution.flag(),
                claim_arguments.before.is_some(),
                claim_arguments.after.is_some(),
                claim_arguments.defer.is_some(),
                claim_arguments.override_reservation.is_some()
            );
        }

        if let Some(extra_command) = rendered_commands.next() {
            return Err(format!(
                "rendered guidance contains an extra overlap answer command `{extra_command}`"
            ));
        }
        Ok(())
    }

    #[test]
    fn proposal_tokens_are_parsed_at_the_cli_boundary() {
        assert!(
            claim_request(&[
                BINARY_NAME,
                "claim",
                "src",
                "--override",
                RESERVATION_ID,
                "--overlap-why",
                "the overlap is accepted",
                "--proposal",
                "not-a-proposal",
            ])
            .is_err()
        );
        assert!(
            parsed_verb(&[BINARY_NAME, "claim", "src", "--proposal", "not-a-proposal",]).is_err()
        );
    }

    #[test]
    fn parser_errors_have_their_published_exit_statuses() {
        let invalid_command_exit = Cli::try_parse_from([BINARY_NAME, "unknown"])
            .map_or_else(|error| exit_for_parser_error(&error), |_| BerthExit::Clear);
        let help_exit = Cli::try_parse_from([BINARY_NAME, "--help"]).map_or_else(
            |error| exit_for_parser_error(&error),
            |_| BerthExit::UsageError,
        );
        let version_exit = Cli::try_parse_from([BINARY_NAME, "--version"]).map_or_else(
            |error| exit_for_parser_error(&error),
            |_| BerthExit::UsageError,
        );

        assert_eq!(invalid_command_exit, BerthExit::UsageError);
        assert_eq!(help_exit, BerthExit::Clear);
        assert_eq!(version_exit, BerthExit::Clear);
    }

    /// Every route states where its result reaches the caller, and the parser agrees.
    ///
    /// The round trip through [`Command::route`] is what makes the coverage exact: a row
    /// whose command line selects some other variant fails here rather than filling a slot
    /// that its own variant then never occupies.
    #[test]
    fn every_command_line_route_declares_the_reporting_its_parser_selects() -> Result<(), String> {
        for route in CommandLineRoute::ALL {
            let command = parsed_command(&route.arguments())?;

            assert_eq!(
                command.route(),
                route,
                "the command line for {route:?} selects a different command"
            );
            assert_eq!(
                command.result_reporting(),
                route.declared_reporting(),
                "{route:?} reports its result through a route it does not declare"
            );
        }
        Ok(())
    }

    /// Only the harness hook and git hook routes answer a protocol instead of an envelope.
    ///
    /// The exception is asserted rather than skipped: a hook verb
    /// writes its own response object and owns its exit status through
    /// [`CommandOutputOwnership::HookOwnsItsResponse`], and the two private git-invoked
    /// commands return from [`Cli::run`] before any envelope exists.
    ///
    /// `__reference-transaction` has its exit statuses proved end to end against the built
    /// binary in `tests/gate.rs`. No test in this crate invokes
    /// `__refresh-managed-hook-after-trunk-deletion` as a command line, so the parser
    /// coverage here is all this crate asserts about that command.
    #[test]
    fn only_the_hook_routes_answer_a_protocol_instead_of_an_envelope() {
        for route in CommandLineRoute::ALL {
            let answers_a_protocol = !matches!(
                route.declared_reporting(),
                CommandResultReporting::Envelope(_)
            );
            let is_a_hook_route = matches!(
                route,
                CommandLineRoute::HookPreToolUse
                    | CommandLineRoute::HookPostToolUse
                    | CommandLineRoute::HookSessionStart
                    | CommandLineRoute::ReferenceTransaction
                    | CommandLineRoute::RefreshManagedHookAfterTrunkDeletion
            );

            assert_eq!(
                answers_a_protocol, is_a_hook_route,
                "{route:?} disagrees with itself about whether it renders an envelope"
            );
        }
    }

    /// Every route answers through the output ownership its reporting declares.
    ///
    /// The engine runs against an empty scratch directory, so each envelope command fails on
    /// the absent repository rather than reaching a ledger. What is under test is the
    /// agreement itself: the ownership [`Command::execute`] hands back for every route, and
    /// the verb an envelope route records its response under.
    ///
    /// No route is skipped. The three harness hook routes are reached without reading this
    /// process's standard input, because [`Command::execute`] selects the hook and leaves the
    /// write to [`Cli::run`]. The two private git-invoked routes return from [`Cli::run`]
    /// before [`Command::execute`] runs at all; what this asserts of them is the refusal that
    /// guards the public envelope path against ever carrying one.
    ///
    /// That an envelope command's own exit status becomes the process exit status is proved
    /// against the built binary in `tests/drift.rs` and `tests/overlap.rs`. A unit test cannot
    /// witness it: [`Cli::run`] answers in [`ExitCode`], which carries no equality to assert
    /// over.
    #[test]
    fn every_command_line_route_answers_through_the_output_ownership_it_declares()
    -> Result<(), String> {
        let scratch_directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let _restored_directory = PreviousWorkingDirectory::left_for(scratch_directory.path())?;

        for route in CommandLineRoute::ALL {
            let output_ownership = executed_ownership(route)?;

            match route.declared_reporting() {
                CommandResultReporting::Envelope(declared_verb) => {
                    let CommandOutputOwnership::CallerRendersResponse(output_envelope) =
                        output_ownership
                    else {
                        return Err(format!("{route:?} rendered no response envelope"));
                    };

                    assert_eq!(
                        output_envelope.verb(),
                        declared_verb,
                        "{route:?} recorded its response under a verb it does not declare"
                    );
                },
                CommandResultReporting::HookProtocol(declared_hook) => {
                    let CommandOutputOwnership::HookOwnsItsResponse(selected_hook) =
                        output_ownership
                    else {
                        return Err(format!("{route:?} did not keep ownership of its response"));
                    };

                    assert_eq!(
                        selected_hook, declared_hook,
                        "{route:?} selected a hook it does not declare"
                    );
                },
                CommandResultReporting::GitHookProtocol => {
                    let CommandOutputOwnership::CallerRendersResponse(refusal) = output_ownership
                    else {
                        return Err(format!(
                            "{route:?} answered no refusal to the envelope path"
                        ));
                    };

                    assert_eq!(
                        refusal.exit_code,
                        BerthExit::UsageError,
                        "{route:?} let a private git dispatch through the public envelope path"
                    );
                },
            }
        }
        Ok(())
    }

    fn executed_ownership(route: CommandLineRoute) -> Result<CommandOutputOwnership, String> {
        let command = parsed_command(&route.arguments())?;
        let output_format = command.output_format();

        Ok(command.execute(output_format, &RecoveryCommandLine::current_process()))
    }

    /// The working directory this process returns to when a scratch directory is done with.
    ///
    /// `cargo-berth` has no `lib.rs`, so every `#[cfg(test)]` module in `src/` compiles into
    /// one test binary sharing one process working directory. A test that leaves the process
    /// standing in a directory that has since been removed breaks whichever test resolves the
    /// working directory next, so the entry is undone here rather than left to the reader,
    /// including when an assertion panics.
    struct PreviousWorkingDirectory {
        previous_directory: PathBuf,
    }

    impl PreviousWorkingDirectory {
        /// Enter `scratch_directory`, remembering the directory to come back to.
        fn left_for(scratch_directory: &Path) -> Result<Self, String> {
            let previous_directory = env::current_dir().map_err(|error| error.to_string())?;
            env::set_current_dir(scratch_directory).map_err(|error| error.to_string())?;

            Ok(Self { previous_directory })
        }
    }

    impl Drop for PreviousWorkingDirectory {
        fn drop(&mut self) {
            if env::set_current_dir(&self.previous_directory).is_err() {
                eprintln!(
                    "the working directory before the scratch directory could not be restored"
                );
            }
        }
    }

    fn parsed_command(arguments: &[&str]) -> Result<Command, String> {
        Cli::try_parse_from(without_subcommand_name(
            arguments.iter().map(OsString::from).collect(),
        ))
        .map(|cli| cli.command)
        .map_err(|error| error.to_string())
    }

    fn assert_verb_parses(
        direct_arguments: &[&str],
        cargo_arguments: &[&str],
        command_verb: CommandVerb,
    ) {
        assert!(
            parsed_verb(direct_arguments).is_ok_and(
                |parsed_verb| parsed_verb == CommandResultReporting::Envelope(command_verb)
            )
        );
        assert!(
            parsed_verb(cargo_arguments).is_ok_and(
                |parsed_verb| parsed_verb == CommandResultReporting::Envelope(command_verb)
            )
        );
    }

    fn parsed_verb(arguments: &[&str]) -> Result<CommandResultReporting, Error> {
        Cli::try_parse_from(without_subcommand_name(
            arguments.iter().map(OsString::from).collect(),
        ))
        .map(|cli| cli.command.result_reporting())
    }

    fn claim_request(arguments: &[&str]) -> Result<ClaimRequest, String> {
        let cli = Cli::try_parse_from(without_subcommand_name(
            arguments.iter().map(OsString::from).collect(),
        ))
        .map_err(|error| error.to_string())?;
        match cli.command {
            Command::Claim(claim_arguments) => claim_arguments.into_claim_request(),
            _ => Err("expected claim command".to_owned()),
        }
    }

    fn parse_rendered_claim_command(rendered_command: &str) -> Result<ClaimArguments, String> {
        let arguments = split_shell_arguments(rendered_command)?;
        let Some(executable) = arguments.first() else {
            return Err(format!(
                "rendered overlap answer command `{rendered_command}` has no executable"
            ));
        };
        if executable != BINARY_NAME {
            return Err(format!(
                "rendered overlap answer command `{rendered_command}` invokes executable \
                 `{executable}`, expected `{BINARY_NAME}`"
            ));
        }

        let arguments = arguments
            .into_iter()
            .map(|argument| match argument.as_str() {
                "<paths...>" => "src/lib.rs".to_owned(),
                "<holder-reservation-id>" => RESERVATION_ID.to_owned(),
                "<reason>" => "the overlap is coordinated".to_owned(),
                _ => argument,
            })
            .collect::<Vec<_>>();
        let cli = Cli::try_parse_from(arguments).map_err(|error| {
            format!("rendered overlap answer command `{rendered_command}` did not parse: {error}")
        })?;
        match cli.command {
            Command::Claim(claim_arguments) => Ok(claim_arguments),
            _ => Err(format!(
                "rendered overlap answer command `{rendered_command}` did not select `claim`"
            )),
        }
    }

    fn split_shell_arguments(rendered_command: &str) -> Result<Vec<String>, String> {
        let mut arguments = Vec::new();
        let mut current_argument = String::new();
        let mut argument_started = false;
        let mut quote_state = ShellQuoteState::Unquoted;

        for character in rendered_command.chars() {
            match (quote_state, character) {
                (ShellQuoteState::Unquoted, character) if character.is_whitespace() => {
                    if argument_started {
                        arguments.push(std::mem::take(&mut current_argument));
                        argument_started = false;
                    }
                },
                (ShellQuoteState::Unquoted, '\'') => {
                    quote_state = ShellQuoteState::SingleQuoted;
                    argument_started = true;
                },
                (ShellQuoteState::Unquoted, '"') => {
                    quote_state = ShellQuoteState::DoubleQuoted;
                    argument_started = true;
                },
                (ShellQuoteState::SingleQuoted, '\'') | (ShellQuoteState::DoubleQuoted, '"') => {
                    quote_state = ShellQuoteState::Unquoted;
                },
                (_, character) => {
                    current_argument.push(character);
                    argument_started = true;
                },
            }
        }

        if quote_state != ShellQuoteState::Unquoted {
            return Err(format!(
                "rendered overlap answer command `{rendered_command}` has an unterminated quote"
            ));
        }
        if argument_started {
            arguments.push(current_argument);
        }

        Ok(arguments)
    }

    fn board_output_selection(arguments: &[&str]) -> Result<BoardOutputSelection, Error> {
        let cli = Cli::try_parse_from(without_subcommand_name(
            arguments.iter().map(OsString::from).collect(),
        ))?;
        match cli.command {
            Command::Board(board_arguments) => Ok(board_arguments.into_output_selection()),
            _ => Err(Error::raw(
                ErrorKind::InvalidSubcommand,
                "test arguments must select board",
            )),
        }
    }

    fn help_for(verb: &str) -> String {
        let rendered_help = Cli::try_parse_from([BINARY_NAME, verb, "--help"])
            .map_or_else(|error| error.render().to_string(), |_| String::new());

        normalize_help_whitespace(&rendered_help)
    }

    fn normalize_help_whitespace(rendered_help: &str) -> String {
        rendered_help
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}
