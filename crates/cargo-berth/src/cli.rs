//! The frozen command line for `cargo-berth`.
//!
//! Cargo invokes this binary as `cargo berth <verb>` and passes the word
//! `berth` to it. [`Cli::parse_arguments`] removes only that injected word, so
//! `cargo berth <verb>` and `cargo-berth <verb>` have the same command line.

use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use clap::ArgGroup;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use clap::error::ErrorKind;

use crate::exit::BerthExit;
use crate::git;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WorkPlanPhase;
use crate::ledger::ClaimSource;
use crate::ledger::Ledger;
use crate::ledger::LedgerTransactionError;
use crate::ledger::NonEmptyReservationPurpose;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReservationPurpose;
use crate::ledger::WorkPlanReference;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::ScopeKind;
use crate::verb::check::CheckRequest;
use crate::verb::claim::ClaimCoordinationRunSelection;
use crate::verb::claim::ClaimRequest;
use crate::verb::claim::PhaseStartSelection;
use crate::verb::release::ReleaseRequest;

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
const HEAD_ARGUMENT: &str = "head";
const HEAD_VALUE_NAME: &str = "OID";
const INTEGRATED_AS_ARGUMENT: &str = "integrated-as";
const INTEGRATED_AS_ARGUMENT_ID: &str = "integrated_as";
const JSON_ARGUMENT: &str = "json";
const PATH_VALUE_NAME: &str = "PATH";
const PHASE_ARGUMENT: &str = "phase";
const PHASE_VALUE_NAME: &str = "PHASE";
const PLAN_ARGUMENT: &str = "plan";
const PLAN_VALUE_NAME: &str = "PLAN";
const RECOVERED_ARGUMENT: &str = "recovered";
const RESOLVE_DISPOSITION_GROUP: &str = "resolve-disposition";
const RUN_ARGUMENT: &str = "run";
const RUN_VALUE_NAME: &str = "COORDINATION_RUN_ID";
const TRUNK_OID_VALUE_NAME: &str = "TRUNK_OID";
const WHY_ARGUMENT: &str = "why";
const WHY_VALUE_NAME: &str = "WHY";

const ABANDON_LONG_ABOUT: &str = "Use this only when the reservation's work is intentionally discarded. It records an irreversible abandonment and releases its coordination hold; choosing it for recoverable work loses the trail that identifies where the work went. --why is required so later readers can distinguish a deliberate decision from a lost worktree.";
const INTEGRATED_AS_LONG_ABOUT: &str = "Use this when the reservation's work reached trunk through a squash, cherry-pick, or other rewritten integration that the tool cannot prove from its stored commit. This asserts the supplied trunk commit is evidence; choosing it without that evidence can incorrectly release an unresolved reservation.";
const RECOVERED_LONG_ABOUT: &str = "Use this when the reservation's work is still present but now belongs to this replacement worktree. It records a new worktree identity; choosing it when the work was actually integrated or discarded leaves an inaccurate live reservation blocking other work.";
const RENEW_LONG_ABOUT: &str = "Record that this still-live reservation remains active after inspection. Renewal changes neither its scopes nor any ordering edge; using it to hide abandoned work delays the user-confirmed recovery or abandonment decision that must eventually resolve it.";
const RESOLVE_LONG_ABOUT: &str = "Resolve a reservation that is stuck because its original worktree disappeared or its integration evidence changed. Choose exactly one disposition: --recovered when the work survives in this replacement worktree; --integrated-as <TRUNK_OID> when the work reached trunk in a form the tool could not prove; or --abandon --why <WHY> only when the work is deliberately discarded. Choosing --abandon discards work. Choosing --integrated-as asserts evidence the tool could not prove for itself, so a wrong commit can release an unresolved reservation.";

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
    Command(Cli),
    /// A command line clap could not parse.
    Usage(clap::Error),
}

/// The verbs available from the frozen `cargo-berth` interface.
#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize the shared reservation ledger.
    Init(JsonOutput),
    /// Display the reservation board.
    Board(JsonOutput),
    /// Check whether paths would be blocked.
    Check(PathArguments),
    /// Claim paths for a reservation.
    Claim(ClaimArguments),
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
}

/// The `--json` flag shared by every verb.
#[derive(Debug, Args)]
struct JsonOutput {
    /// Emit the frozen JSON response envelope.
    #[arg(long = JSON_ARGUMENT)]
    json: bool,
}

/// The output representation requested at the command line boundary.
#[derive(Clone, Copy)]
enum CliOutputFormat {
    /// Print the frozen JSON envelope.
    Json,
    /// Print the envelope's message.
    Text,
}

impl From<bool> for CliOutputFormat {
    fn from(json: bool) -> Self { if json { Self::Json } else { Self::Text } }
}

impl JsonOutput {
    /// Convert clap's flag value into the command's output representation.
    fn output_format(&self) -> CliOutputFormat { self.json.into() }
}

/// A command whose first argument is one or more repository paths.
#[derive(Debug, Args)]
struct PathArguments {
    /// The repository paths the command concerns.
    #[arg(required = true, value_name = PATH_VALUE_NAME)]
    paths:       Vec<std::path::PathBuf>,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output: JsonOutput,
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
    paths:                Vec<std::path::PathBuf>,
    /// Sequence this reservation before the blocking reservation.
    #[arg(long = CLAIM_BEFORE_ARGUMENT, value_name = BLOCKER_VALUE_NAME)]
    before:               Option<ReservationId>,
    /// Sequence this reservation after the blocking reservation.
    #[arg(long = CLAIM_AFTER_ARGUMENT, value_name = BLOCKER_VALUE_NAME)]
    after:                Option<ReservationId>,
    /// Defer an answer about the blocking reservation.
    #[arg(long = CLAIM_DEFER_ARGUMENT, value_name = BLOCKER_VALUE_NAME)]
    defer:                Option<ReservationId>,
    /// Override the blocking reservation.
    #[arg(long = CLAIM_OVERRIDE_ARGUMENT, value_name = BLOCKER_VALUE_NAME)]
    override_reservation: Option<ReservationId>,
    /// Explain why the overlap answer is requested.
    #[arg(long = WHY_ARGUMENT, value_name = WHY_VALUE_NAME)]
    why:                  Option<String>,
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
    /// Use this UUID-v7 coordination run instead of minting one.
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
    /// Permit integration past a held ordering edge.
    #[arg(long = FORCE_ARGUMENT, requires = WHY_ARGUMENT)]
    force:          bool,
    /// Explain why forced integration is authorized.
    #[arg(long = WHY_ARGUMENT, value_name = WHY_VALUE_NAME, requires = FORCE_ARGUMENT)]
    why:            Option<String>,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output:    JsonOutput,
}

/// A user-confirmed recovery decision for a stuck reservation.
#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new(RESOLVE_DISPOSITION_GROUP)
        .args([RECOVERED_ARGUMENT, INTEGRATED_AS_ARGUMENT_ID, ABANDON_ARGUMENT])
        .required(true)
        .multiple(false)
))]
struct ResolveArguments {
    /// The stuck reservation to resolve.
    reservation_id: ReservationId,
    /// Record this worktree as the recovered holder of surviving work.
    #[arg(long = RECOVERED_ARGUMENT, long_help = RECOVERED_LONG_ABOUT)]
    recovered:      bool,
    /// Assert a trunk commit proves rewritten integration.
    #[arg(
        long = INTEGRATED_AS_ARGUMENT,
        value_name = TRUNK_OID_VALUE_NAME,
        long_help = INTEGRATED_AS_LONG_ABOUT
    )]
    integrated_as:  Option<String>,
    /// Permanently discard this reservation's work and coordination hold.
    #[arg(
        long = ABANDON_ARGUMENT,
        requires = WHY_ARGUMENT,
        long_help = ABANDON_LONG_ABOUT
    )]
    abandon:        bool,
    /// Explain the deliberate abandonment decision.
    #[arg(long = WHY_ARGUMENT, value_name = WHY_VALUE_NAME, requires = ABANDON_ARGUMENT)]
    why:            Option<String>,
    /// The output representation requested for this command.
    #[command(flatten)]
    json_output:    JsonOutput,
}

impl Cli {
    /// Read the command line, whether cargo invoked this binary or a shell did.
    pub(crate) fn parse_arguments() -> CliInvocation {
        Self::try_parse_from(without_subcommand_name(env::args_os().collect()))
            .map_or_else(CliInvocation::Usage, CliInvocation::Command)
    }

    /// Execute the parsed command and return its published process exit status.
    fn run(self) -> ExitCode {
        let output_format = self.command.output_format();
        let output_envelope = self.command.execute();
        let berth_exit = output_envelope.exit_code;
        emit_response(output_format, &output_envelope);
        berth_exit.into()
    }
}

impl CliInvocation {
    /// Print a parser error or execute a valid command.
    pub(crate) fn run(self) -> ExitCode {
        match self {
            Self::Command(cli) => cli.run(),
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
    /// Execute this command's available engine or return its typed placeholder.
    fn execute(self) -> OutputEnvelope {
        match self {
            Self::Init(_) => initialize_ledger(),
            Self::Board(_) => OutputEnvelope::unimplemented(CommandVerb::Board),
            Self::Check(path_arguments) => match path_arguments.into_check_request() {
                Ok(check_request) => crate::verb::check::execute(check_request),
                Err(error) => OutputEnvelope::invalid_input(CommandVerb::Check, &error),
            },
            Self::Claim(claim_arguments) => match claim_arguments.into_claim_request() {
                Ok(claim_request) => crate::verb::claim::execute(claim_request),
                Err(error) => OutputEnvelope::invalid_input(CommandVerb::Claim, &error),
            },
            Self::Release(reservation_arguments) => {
                crate::verb::release::execute(reservation_arguments.into_release_request())
            },
            Self::Sequence(_) => OutputEnvelope::unimplemented(CommandVerb::Sequence),
            Self::Integrate(_) => OutputEnvelope::unimplemented(CommandVerb::Integrate),
            Self::Resolve(_) => OutputEnvelope::unimplemented(CommandVerb::Resolve),
            Self::Renew(_) => OutputEnvelope::unimplemented(CommandVerb::Renew),
        }
    }

    /// Return this command's requested output representation.
    fn output_format(&self) -> CliOutputFormat {
        match self {
            Self::Init(json_output) | Self::Board(json_output) => json_output.output_format(),
            Self::Check(path_arguments) => path_arguments.json_output.output_format(),
            Self::Claim(claim_arguments) => claim_arguments.json_output.output_format(),
            Self::Release(reservation_arguments) | Self::Renew(reservation_arguments) => {
                reservation_arguments.json_output.output_format()
            },
            Self::Sequence(sequence_arguments) => sequence_arguments.json_output.output_format(),
            Self::Integrate(integrate_arguments) => integrate_arguments.json_output.output_format(),
            Self::Resolve(resolve_arguments) => resolve_arguments.json_output.output_format(),
        }
    }

    /// Return this command's envelope verb without executing its engine.
    #[cfg(test)]
    const fn verb(&self) -> CommandVerb {
        match self {
            Self::Init(_) => CommandVerb::Init,
            Self::Board(_) => CommandVerb::Board,
            Self::Check(_) => CommandVerb::Check,
            Self::Claim(_) => CommandVerb::Claim,
            Self::Release(_) => CommandVerb::Release,
            Self::Sequence(_) => CommandVerb::Sequence,
            Self::Integrate(_) => CommandVerb::Integrate,
            Self::Resolve(_) => CommandVerb::Resolve,
            Self::Renew(_) => CommandVerb::Renew,
        }
    }
}

impl PathArguments {
    fn into_check_request(self) -> Result<CheckRequest, String> {
        DeclaredReservationScopeSet::parse(self.paths, ScopeKind::File)
            .map(|declared_scopes| CheckRequest { declared_scopes })
            .map_err(|error| error.to_string())
    }
}

impl ClaimArguments {
    fn into_claim_request(self) -> Result<ClaimRequest, String> {
        let Self {
            paths,
            before: _,
            after: _,
            defer: _,
            override_reservation: _,
            why,
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
        Ok(ClaimRequest {
            declared_scopes,
            source,
            purpose,
            coordination_run_selection: run.map_or(
                ClaimCoordinationRunSelection::ContinueOrStart,
                ClaimCoordinationRunSelection::Specified,
            ),
            phase_start,
        })
    }
}

impl ReservationArguments {
    const fn into_release_request(self) -> ReleaseRequest {
        ReleaseRequest {
            reservation_id: self.reservation_id,
        }
    }
}

fn initialize_ledger() -> OutputEnvelope {
    match env::current_dir() {
        Ok(invocation_directory) => match git::repository_root(&invocation_directory) {
            Ok(repository_root) => match Ledger::initialize(&repository_root) {
                Ok(initialization) => OutputEnvelope::initialized(initialization),
                Err(error) => match LedgerTransactionError::from(error) {
                    LedgerTransactionError::LockContention => OutputEnvelope::contention(
                        CommandVerb::Init,
                        &LedgerTransactionError::LockContention.to_string(),
                    ),
                    LedgerTransactionError::LedgerUnreadable(error) => {
                        OutputEnvelope::ledger_unreadable(CommandVerb::Init, &error.to_string())
                    },
                    LedgerTransactionError::CorrectableInput(error) => {
                        OutputEnvelope::invalid_input(CommandVerb::Init, &error.to_string())
                    },
                },
            },
            Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Init, &error.to_string()),
        },
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Init, &error.to_string()),
    }
}

fn emit_response(output_format: CliOutputFormat, output_envelope: &OutputEnvelope) {
    let rendered = match output_format {
        CliOutputFormat::Json => match serde_json::to_string(output_envelope) {
            Ok(rendered) => rendered,
            Err(_) => return,
        },
        CliOutputFormat::Text => output_envelope.message.clone(),
    };
    write_line(rendered);
}

fn write_line(mut rendered: String) {
    rendered.push('\n');
    let standard_output = std::io::stdout();
    let mut standard_output = standard_output.lock();
    std::mem::drop(standard_output.write_all(rendered.as_bytes()));
}

/// Decide the exit status required by a clap parser error.
fn exit_for_parser_error(error: &clap::Error) -> BerthExit {
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
    use std::ffi::OsString;

    use clap::Parser;

    use super::BINARY_NAME;
    use super::CARGO_SUBCOMMAND_NAME;
    use super::Cli;
    use super::CommandVerb;
    use super::exit_for_parser_error;
    use super::without_subcommand_name;
    use crate::exit::BerthExit;

    const RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";

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

    fn assert_verb_parses(
        direct_arguments: &[&str],
        cargo_arguments: &[&str],
        command_verb: CommandVerb,
    ) {
        assert!(parsed_verb(direct_arguments).is_ok_and(|parsed_verb| parsed_verb == command_verb));
        assert!(parsed_verb(cargo_arguments).is_ok_and(|parsed_verb| parsed_verb == command_verb));
    }

    fn parsed_verb(arguments: &[&str]) -> Result<CommandVerb, clap::Error> {
        Cli::try_parse_from(without_subcommand_name(
            arguments.iter().map(OsString::from).collect(),
        ))
        .map(|cli| cli.command.verb())
    }

    fn help_for(verb: &str) -> String {
        Cli::try_parse_from([BINARY_NAME, verb, "--help"])
            .map_or_else(|error| error.render().to_string(), |_| String::new())
    }
}
