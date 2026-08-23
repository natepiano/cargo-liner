//! The frozen command line for `cargo-berth`.
//!
//! Cargo invokes this binary as `cargo berth <verb>` and passes the word
//! `berth` to it. [`Cli::parse_arguments`] removes only that injected word, so
//! `cargo berth <verb>` and `cargo-berth <verb>` have the same command line.

use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::process::ExitCode;

use clap::ArgGroup;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use clap::error::ErrorKind;

use crate::exit::BerthExit;
use crate::ids::ReservationId;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;

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
const JSON_ARGUMENT: &str = "json";
const PATH_VALUE_NAME: &str = "PATH";
const WHY_ARGUMENT: &str = "why";
const WHY_VALUE_NAME: &str = "WHY";

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
}

/// The `--json` flag shared by every verb.
#[derive(Debug, Args)]
struct JsonOutput {
    /// Emit the frozen JSON response envelope.
    #[arg(long = JSON_ARGUMENT)]
    json: bool,
}

/// The output representation requested at the command line boundary.
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

impl Cli {
    /// Read the command line, whether cargo invoked this binary or a shell did.
    pub(crate) fn parse_arguments() -> CliInvocation {
        Self::try_parse_from(without_subcommand_name(env::args_os().collect()))
            .map_or_else(CliInvocation::Usage, CliInvocation::Command)
    }

    /// Return the parsed verb's frozen response.
    fn run(self) -> ExitCode {
        let command_verb = self.command.verb();
        let cli_output_format = self.command.output_format();
        let output_envelope = OutputEnvelope::unimplemented(command_verb);

        match cli_output_format {
            CliOutputFormat::Json => {
                println!("{}", OutputEnvelope::unimplemented_json(command_verb));
            },
            CliOutputFormat::Text => println!("{}", output_envelope.message),
        }

        BerthExit::Clear.into()
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

/// Decide the exit status required by a clap parser error.
fn exit_for_parser_error(error: &clap::Error) -> BerthExit {
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => BerthExit::Clear,
        _ => BerthExit::UsageError,
    }
}

impl Command {
    /// Return this command's envelope verb.
    const fn verb(&self) -> CommandVerb {
        match self {
            Self::Init(_) => CommandVerb::Init,
            Self::Board(_) => CommandVerb::Board,
            Self::Check(_) => CommandVerb::Check,
            Self::Claim(_) => CommandVerb::Claim,
            Self::Release(_) => CommandVerb::Release,
            Self::Sequence(_) => CommandVerb::Sequence,
            Self::Integrate(_) => CommandVerb::Integrate,
        }
    }

    /// Return this command's requested output representation.
    fn output_format(&self) -> CliOutputFormat {
        match self {
            Self::Init(json_output) | Self::Board(json_output) => json_output.output_format(),
            Self::Check(path_arguments) => path_arguments.json_output.output_format(),
            Self::Claim(claim_arguments) => claim_arguments.json_output.output_format(),
            Self::Release(reservation_arguments) => {
                reservation_arguments.json_output.output_format()
            },
            Self::Sequence(sequence_arguments) => sequence_arguments.json_output.output_format(),
            Self::Integrate(integrate_arguments) => integrate_arguments.json_output.output_format(),
        }
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
    use super::BerthExit;
    use super::CARGO_SUBCOMMAND_NAME;
    use super::Cli;
    use super::CommandVerb;
    use super::exit_for_parser_error;
    use super::without_subcommand_name;

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
            &[BINARY_NAME, "release", "reservation", "--json"],
            &[
                BINARY_NAME,
                CARGO_SUBCOMMAND_NAME,
                "release",
                "reservation",
                "--json",
            ],
            CommandVerb::Release,
        );
        assert_verb_parses(
            &[
                BINARY_NAME,
                "sequence",
                "first",
                "then",
                "--why",
                "order",
                "--json",
            ],
            &[
                BINARY_NAME,
                CARGO_SUBCOMMAND_NAME,
                "sequence",
                "first",
                "then",
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
                "reservation",
                "--force",
                "--why",
                "authorized",
                "--json",
            ],
            &[
                BINARY_NAME,
                CARGO_SUBCOMMAND_NAME,
                "integrate",
                "reservation",
                "--force",
                "--why",
                "authorized",
                "--json",
            ],
            CommandVerb::Integrate,
        );
    }

    fn assert_verb_parses(
        direct_arguments: &[&str],
        cargo_arguments: &[&str],
        command_verb: CommandVerb,
    ) {
        assert!(
            parsed_verb(direct_arguments).is_ok_and(|parsed_verb| { parsed_verb == command_verb })
        );
        assert!(
            parsed_verb(cargo_arguments).is_ok_and(|parsed_verb| { parsed_verb == command_verb })
        );
    }

    fn parsed_verb(arguments: &[&str]) -> Result<CommandVerb, clap::Error> {
        Cli::try_parse_from(without_subcommand_name(
            arguments.iter().map(OsString::from).collect(),
        ))
        .map(|cli| cli.command.verb())
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
}
