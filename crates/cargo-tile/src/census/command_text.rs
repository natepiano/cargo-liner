//! Cargo argument classification and command display.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use crate::config::DefaultArguments;
use crate::constants::ARGUMENT_ASSIGNMENT;
use crate::constants::ARGUMENT_SEPARATOR;
use crate::constants::CARGO_DEFAULT_PROFILE;
use crate::constants::CARGO_DISPLAY_NAME;
use crate::constants::CARGO_PROCESS_NAMES;
use crate::constants::CARGO_PROFILE_FLAG;
use crate::constants::CARGO_RELEASE_FLAGS;
use crate::constants::CARGO_RELEASE_PROFILE;
use crate::constants::CARGO_SUBCOMMAND_PREFIX;
use crate::constants::CARGO_TOOLCHAIN_SELECTOR;
use crate::constants::COMMAND_VALUED_OPTIONS;
use crate::constants::COMMAND_WORD_SEPARATOR;
use crate::constants::FLAG_MARK;
use crate::constants::HOME_ALIAS;
use crate::constants::RUSTC_LINT_OWNERS;
use crate::constants::SELF_PROCESS_NAME;
use crate::constants::ValuedOptionScope;

/// Convert the scanner's optional home at the external API boundary once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScannerHome<'home> {
    /// Only this prefix may be shortened to the scanner's tilde.
    Known(&'home Path),
    /// No scanner home was observed, so every directory stays absolute.
    Unavailable,
}

impl<'home> From<Option<&'home Path>> for ScannerHome<'home> {
    fn from(home: Option<&'home Path>) -> Self { home.map_or(Self::Unavailable, Self::Known) }
}

/// Validated arguments can only lack a retained subcommand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetainedSubcommandAbsence {
    /// No retained argument names a subcommand.
    NoSubcommand,
}

/// Raw arguments preserve why they cannot identify a subcommand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SubcommandAbsence {
    /// The process API supplied no argument list.
    ArgvUnavailable,
    /// Available arguments identify no accepted cargo program.
    ProgramRejected,
    /// Cargo is recognized, but its arguments name no subcommand.
    NoSubcommand,
}

/// An executable name that cannot identify an external cargo subcommand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExternalSubcommandAbsence {
    /// The program is cargo itself, this scanner, or lacks the external prefix.
    ProgramRejected,
}

/// A command line split into its program and the rest of its arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandText {
    /// Program name, path stripped.
    pub(crate) program: String,
    /// Remaining arguments, one entry per argv word. Held split rather
    /// than joined because a cell may leave one of them out;
    /// [`CommandText::line`] is what puts them back into a line.
    arguments:          Vec<String>,
    /// Parsed once, preserving each argv entry and every complete option value.
    words:              Vec<CommandWord>,
}

/// A parsed argv word; only this module can construct or classify it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandWord {
    kind: CommandWordKind,
}

/// Incomplete syntax is retained as a diagnostic word, never as a valued flag.
#[derive(Clone, Debug, Eq, PartialEq)]
enum CommandWordKind {
    Name(String),
    Selector(String),
    Flag(String),
    FlagWithValue { flag: String, value: String },
    IncompleteOption(String),
    Value(String),
    Separator,
}

impl CommandWord {
    /// Render without splitting an argv entry containing whitespace.
    fn text(&self) -> Cow<'_, str> {
        match &self.kind {
            CommandWordKind::Name(text)
            | CommandWordKind::Selector(text)
            | CommandWordKind::Flag(text)
            | CommandWordKind::IncompleteOption(text)
            | CommandWordKind::Value(text) => Cow::Borrowed(text),
            CommandWordKind::FlagWithValue { flag, value } => {
                Cow::Owned(format!("{flag}{COMMAND_WORD_SEPARATOR}{value}"))
            },
            CommandWordKind::Separator => Cow::Borrowed(ARGUMENT_SEPARATOR),
        }
    }

    /// Expose the parsed value boundary without classifying the text again.
    pub(crate) fn display_word(&self) -> CommandWordDisplay<'_> {
        match &self.kind {
            CommandWordKind::FlagWithValue { flag, value } => {
                CommandWordDisplay::FlagWithValue { flag, value }
            },
            _ => CommandWordDisplay::Independent(self.text()),
        }
    }

    fn is_selected(&self, selected: &DefaultArguments) -> bool {
        match &self.kind {
            CommandWordKind::Name(_) | CommandWordKind::Selector(_) => true,
            CommandWordKind::Flag(flag)
            | CommandWordKind::IncompleteOption(flag)
            | CommandWordKind::FlagWithValue { flag, .. } => selected.contains(flag),
            CommandWordKind::Value(_) | CommandWordKind::Separator => false,
        }
    }
}

/// A display unit either stands alone or retains its flag and value separately.
pub(crate) enum CommandWordDisplay<'a> {
    Independent(Cow<'a, str>),
    FlagWithValue { flag: &'a str, value: &'a str },
}

/// Which program owns the flags in the current argv segment.
#[derive(Clone, Copy)]
enum ArgumentOwner {
    CargoGlobal,
    Cargo,
    RustcLint,
    Other,
}

impl ArgumentOwner {
    /// Compiler frontends share the same lint argument language.
    fn compiler(program: &str) -> Self {
        if RUSTC_LINT_OWNERS.contains(&program_name(program)) {
            Self::RustcLint
        } else {
            Self::Other
        }
    }

    fn takes_value(self, flag: &str) -> bool {
        COMMAND_VALUED_OPTIONS.iter().any(|(name, scope)| {
            *name == flag
                && matches!(
                    (self, scope),
                    (
                        Self::Cargo,
                        ValuedOptionScope::Global | ValuedOptionScope::Cargo
                    ) | (Self::CargoGlobal, ValuedOptionScope::Global)
                        | (Self::RustcLint, ValuedOptionScope::RustcLint)
                )
        })
    }
}

/// Parse only at the argv boundary; consumers never infer arity from text.
fn command_words(program: &str, arguments: &[String]) -> Vec<CommandWord> {
    let cargo = CARGO_PROCESS_NAMES.contains(&program_name(program))
        || program_name(program).starts_with(CARGO_SUBCOMMAND_PREFIX);
    let mut owner = if cargo {
        ArgumentOwner::CargoGlobal
    } else {
        ArgumentOwner::compiler(program)
    };
    let mut position = if cargo {
        CommandNamePosition::BeforeCommand
    } else {
        CommandNamePosition::Arguments
    };
    let mut child_owner = ArgumentOwner::Other;
    let mut words = arguments.iter().peekable();
    let mut parsed = Vec::new();
    while let Some(word) = words.next() {
        let kind = if word == ARGUMENT_SEPARATOR {
            position = CommandNamePosition::Arguments;
            owner = child_owner;
            CommandWordKind::Separator
        } else if word.starts_with(FLAG_MARK) {
            if matches!(position, CommandNamePosition::Naming) {
                position = CommandNamePosition::Arguments;
            }
            if !word.contains(ARGUMENT_ASSIGNMENT) && owner.takes_value(word) {
                words
                    .next_if(|value| !value.starts_with(FLAG_MARK))
                    .map_or_else(
                        || CommandWordKind::IncompleteOption(word.clone()),
                        |value| CommandWordKind::FlagWithValue {
                            flag:  word.clone(),
                            value: value.clone(),
                        },
                    )
            } else {
                CommandWordKind::Flag(word.clone())
            }
        } else if !matches!(position, CommandNamePosition::Arguments) && names_the_command(word) {
            if word.starts_with(CARGO_TOOLCHAIN_SELECTOR) {
                CommandWordKind::Selector(word.clone())
            } else {
                if matches!(position, CommandNamePosition::BeforeCommand) {
                    child_owner = ArgumentOwner::compiler(word);
                    owner = ArgumentOwner::Cargo;
                }
                position = CommandNamePosition::Naming;
                CommandWordKind::Name(word.clone())
            }
        } else {
            position = CommandNamePosition::Arguments;
            CommandWordKind::Value(word.clone())
        };
        parsed.push(CommandWord { kind });
    }
    parsed
}

/// Which argv words can still name the invocation before its option tail.
#[derive(Clone, Copy)]
enum CommandNamePosition {
    BeforeCommand,
    Naming,
    Arguments,
}

/// Cargo's build profile, independent of arguments passed to its child program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CargoProfile {
    /// The usual development profile needs no label.
    Default,
    /// Cargo's optimized release profile.
    Release,
    /// A profile explicitly named by the invocation.
    Named(String),
}

impl CargoProfile {
    /// Normalize Cargo's two built-in profile names.
    fn named(name: &str) -> Self {
        match name {
            CARGO_DEFAULT_PROFILE => Self::Default,
            CARGO_RELEASE_PROFILE => Self::Release,
            name => Self::Named(name.to_string()),
        }
    }
}

impl CommandText {
    /// A command line built from its parts, for the tests elsewhere in
    /// the crate that need an invocation to hand around.
    #[cfg(test)]
    pub(crate) fn of(program: &str, arguments: &[&str]) -> Self {
        Self::new(
            program.to_string(),
            arguments.iter().map(|word| (*word).to_string()).collect(),
        )
    }

    pub(super) fn new(program: String, arguments: Vec<String>) -> Self {
        let words = command_words(&program, &arguments);
        Self {
            program,
            arguments,
            words,
        }
    }

    /// Parsed argument units in their original argv order.
    pub(crate) fn words(&self) -> &[CommandWord] { &self.words }

    /// Both default text and wrapping select the same parsed units.
    pub(crate) fn selected_words<'a>(
        &'a self,
        selected: &DefaultArguments,
    ) -> Vec<&'a CommandWord> {
        self.words
            .iter()
            .filter(|word| word.is_selected(selected))
            .collect()
    }

    /// Legacy display-line fixtures have no argv boundaries to preserve.
    #[cfg(test)]
    pub(crate) fn from_display_line(line: &str) -> Self {
        let mut words = line.split_whitespace();
        Self::new(
            words.next().unwrap_or_default().to_string(),
            words.map(str::to_string).collect(),
        )
    }

    /// Keep selected parsed units without parsing their display text again.
    pub(crate) fn with_default_arguments(&self, selected: &DefaultArguments) -> Self {
        let words: Vec<_> = self.selected_words(selected).into_iter().cloned().collect();
        let arguments = words
            .iter()
            .flat_map(|word| match &word.kind {
                CommandWordKind::FlagWithValue { flag, value } => vec![flag.clone(), value.clone()],
                _ => vec![word.text().into_owned()],
            })
            .collect();
        Self {
            program: self.program_name().to_string(),
            arguments,
            words,
        }
    }

    /// A display line assembled from retained argv entries, preserving their whitespace.
    pub(crate) fn display_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.arguments.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(COMMAND_WORD_SEPARATOR)
    }

    /// The cargo subcommand this invocation names: `port` in
    /// `cargo port`, and in `cargo +nightly --offline port` too.
    /// Leading global options, their values and toolchain selectors
    /// are no part of the subcommand.
    ///
    /// [`command_text`] puts an external subcommand's own name back at
    /// the front of the arguments, so a command that became
    /// `cargo-port` answers this the same as one still spelled
    /// `cargo port`.
    fn subcommand(&self) -> Result<&str, RetainedSubcommandAbsence> {
        cargo_subcommand(&self.words)
    }

    /// Whether the hidden command policy names this invocation's subcommand.
    pub(crate) fn is_hidden(&self, hidden_commands: &[String]) -> bool {
        self.subcommand()
            .is_ok_and(|subcommand| hidden_commands.iter().any(|hidden| hidden == subcommand))
    }

    /// The command's subcommands and toolchain, without its argument tail.
    ///
    /// Keeps the toolchain selector, which is part of what runs rather
    /// than an argument to it -- `+nightly fmt` says something `fmt`
    /// alone does not.
    pub(crate) fn named(&self) -> CommandName {
        let mut kept = Vec::new();
        let mut omission = ArgumentOmission::Kept;
        for word in &self.words {
            match &word.kind {
                CommandWordKind::Name(name) | CommandWordKind::Selector(name) => {
                    kept.push(name.as_str());
                },
                _ => omission = ArgumentOmission::Dropped,
            }
        }
        CommandName {
            words:     kept.join(COMMAND_WORD_SEPARATOR),
            arguments: omission,
        }
    }

    /// Subcommand words and configured flags in their original argv order.
    pub(crate) fn selected_arguments(&self, selected: &DefaultArguments) -> String {
        self.selected_words(selected)
            .into_iter()
            .map(CommandWord::text)
            .collect::<Vec<_>>()
            .join(COMMAND_WORD_SEPARATOR)
    }

    /// The program's file name for Short-view rows.
    pub(crate) fn program_name(&self) -> &str { program_name(&self.program) }

    /// Read only Cargo's own profile flags, stopping at its argument separator.
    pub(crate) fn profile(&self) -> CargoProfile {
        let mut profile = CargoProfile::Default;
        let mut arguments = self
            .arguments
            .iter()
            .map(String::as_str)
            .take_while(|argument| *argument != ARGUMENT_SEPARATOR);
        while let Some(argument) = arguments.next() {
            if CARGO_RELEASE_FLAGS.contains(&argument) {
                profile = CargoProfile::Release;
            } else if argument == CARGO_PROFILE_FLAG {
                if let Some(name) = arguments.next() {
                    profile = CargoProfile::named(name);
                }
            } else if let Some(name) = argument
                .strip_prefix(CARGO_PROFILE_FLAG)
                .and_then(|rest| rest.strip_prefix(ARGUMENT_ASSIGNMENT))
            {
                profile = CargoProfile::named(name);
            }
        }
        profile
    }

    /// All arguments, preserving the whitespace inside each argv entry.
    pub(crate) fn line(&self) -> String { self.arguments.join(COMMAND_WORD_SEPARATOR) }
}

/// One whole command line with its arguments taken off, for the steps
/// of a chain, which are held as a line rather than split.
///
/// Short view names executables by file name. Cargo retains its subcommands;
/// other session runners omit arguments.
#[cfg(test)]
fn command_name(line: &str, selected: &DefaultArguments) -> CommandName {
    let command = CommandText::from_display_line(line);
    let arguments = command.selected_arguments(selected);
    let words = if arguments.is_empty() {
        command.program_name().to_string()
    } else {
        format!("{} {arguments}", command.program_name())
    };
    CommandName {
        words,
        arguments: if command.arguments.join(COMMAND_WORD_SEPARATOR) == arguments {
            ArgumentOmission::Kept
        } else {
            ArgumentOmission::Dropped
        },
    }
}

/// Whether the Short name omitted arguments from its source command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArgumentOmission {
    Kept,
    Dropped,
}

/// Kept command words together with the meaning of their terminal mark.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandName {
    pub(crate) words:     String,
    pub(crate) arguments: ArgumentOmission,
}

/// Convert a path-bearing executable name at the path API boundary.
fn program_name(program: &str) -> &str {
    Path::new(program)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(program)
}

/// Whether a word standing after the program still names the command
/// rather than arguing with it.
///
/// Two answers rule a word out: a leading dash, which is a flag, and a
/// path separator, which is a manifest or a target directory or a
/// binary reached by its path. What survives is the subcommands and the
/// toolchain selector, which is the name of what runs.
fn names_the_command(word: &str) -> bool {
    !word.starts_with(FLAG_MARK) && !word.contains(std::path::MAIN_SEPARATOR)
}

/// Render `path` with the home directory collapsed to `~`.
pub(super) fn home_relative(path: &Path, home: ScannerHome<'_>) -> String {
    let full = path.display().to_string();
    let ScannerHome::Known(home) = home else {
        return full;
    };
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => HOME_ALIAS.to_string(),
        Ok(rest) => format!("{HOME_ALIAS}/{}", rest.display()),
        Err(_) => full,
    }
}

/// Split argv into the program's bare name and the rest of the line,
/// retaining whether unavailable metadata or exclusion prevents a cargo row.
///
/// A cargo binary installed under an alias still reads as `cargo`: the
/// name on disk is an artifact of how it was wrapped, not of what the
/// user typed.
///
/// [`RowAbsence::ProgramRejected`] rejects readable argv belonging to another program.
/// The initial census classifies on [`sysinfo::Process::name`], and
/// macOS does not always let sysinfo read a process's executable: when
/// it cannot, the name reported is the parent's. Every `sccache` a build
/// spawns is a child of cargo, so a whole burst of them can present as
/// cargo at once. Their argv still reads `sccache /path/to/rustc …`,
/// which names no cargo binary, and that is what settles it.
pub(super) fn command_text(
    argv: &[OsString],
    home: ScannerHome<'_>,
) -> Result<CommandText, RowAbsence> {
    let CargoArguments { start, layout } = cargo_split(argv)?;
    let mut arguments: Vec<String> = argv
        .iter()
        .skip(start)
        .map(|argument| home_relative(Path::new(argument), home))
        .collect();
    // An external subcommand's binary is usually handed its own name
    // back as the first argument -- `cargo-nextest nextest run` -- but
    // a caller invoking the binary directly skips that. Putting it back
    // is what makes both spell the command that was typed.
    if let ArgumentLayout::External(subcommand) = layout
        && arguments.first() != Some(&subcommand)
    {
        arguments.insert(0, subcommand);
    }
    Ok(CommandText::new(CARGO_DISPLAY_NAME.to_string(), arguments))
}

/// Why a process-table candidate cannot become a cargo row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RowAbsence {
    /// The external process API could not supply argv.
    ArgvUnavailable,
    /// The available arguments do not identify a cargo program.
    ProgramRejected,
    /// The configured exclusions name this command.
    PolicyExcluded,
}

/// Raw argv inspection can fail before command policy is consulted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CargoArgvAbsence {
    /// The process API supplied no argument list.
    ArgvUnavailable,
    /// Available arguments identify no accepted cargo program.
    ProgramRejected,
}

impl From<CargoArgvAbsence> for RowAbsence {
    fn from(absence: CargoArgvAbsence) -> Self {
        match absence {
            CargoArgvAbsence::ArgvUnavailable => Self::ArgvUnavailable,
            CargoArgvAbsence::ProgramRejected => Self::ProgramRejected,
        }
    }
}

impl From<CargoArgvAbsence> for SubcommandAbsence {
    fn from(absence: CargoArgvAbsence) -> Self {
        match absence {
            CargoArgvAbsence::ArgvUnavailable => Self::ArgvUnavailable,
            CargoArgvAbsence::ProgramRejected => Self::ProgramRejected,
        }
    }
}

/// The two layouts carry different rules for rebuilding the displayed command.
#[derive(Debug, Eq, PartialEq)]
enum ArgumentLayout {
    /// Arguments immediately follow a cargo executable within argv.
    Cargo,
    /// The executable itself supplies the cargo subcommand name.
    External(String),
}

/// A recognized cargo argv retains where arguments begin and their interpretation.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct CargoArguments {
    /// Skip wrappers and the cargo executable when displaying arguments.
    pub(super) start: usize,
    /// External subcommands may need their name inserted into the display.
    layout:           ArgumentLayout,
}

impl CargoArguments {
    /// The forwarded slice follows cargo itself, including an interpreted shim.
    pub(super) const fn follows_cargo_executable(&self) -> bool {
        matches!(self.layout, ArgumentLayout::Cargo)
    }
}

/// Wrappers may precede the cargo binary; empty argv and a rejected program differ.
pub(super) fn cargo_split(argv: &[OsString]) -> Result<CargoArguments, CargoArgvAbsence> {
    if let Some(start) = argv.iter().position(is_cargo_binary) {
        return Ok(CargoArguments {
            start:  start + 1,
            layout: ArgumentLayout::Cargo,
        });
    }
    let program = argv.first().ok_or(CargoArgvAbsence::ArgvUnavailable)?;
    let subcommand = external_subcommand(program)
        .map_err(|ExternalSubcommandAbsence::ProgramRejected| CargoArgvAbsence::ProgramRejected)?;
    Ok(CargoArguments {
        start:  1,
        layout: ArgumentLayout::External(subcommand),
    })
}

/// User exclusions and unavailable process metadata retain separate reasons.
pub(super) fn select_cargo(
    argv: &[OsString],
    excluded: &[String],
) -> Result<CargoArguments, RowAbsence> {
    let arguments = cargo_split(argv)?;
    if is_excluded(argv, excluded) {
        return Err(RowAbsence::PolicyExcluded);
    }
    Ok(arguments)
}

/// Whether an argv belongs to a cargo invocation at all.
///
/// The initial census classifies on the process's own name, and a process
/// can wear one without being one -- see [`command_text`] -- so this is
/// what settles it.
pub(super) fn names_cargo(argv: &[OsString]) -> bool { cargo_split(argv).is_ok() }

/// Read the subcommand from the shared argv classification.
fn cargo_subcommand(words: &[CommandWord]) -> Result<&str, RetainedSubcommandAbsence> {
    words
        .iter()
        .find_map(|word| match &word.kind {
            CommandWordKind::Name(name) => Some(name.as_str()),
            _ => None,
        })
        .ok_or(RetainedSubcommandAbsence::NoSubcommand)
}

/// The subcommand after Cargo's global options and toolchain selectors.
pub(super) fn subcommand(argv: &[OsString]) -> Result<String, SubcommandAbsence> {
    let CargoArguments { start, layout } = cargo_split(argv).map_err(SubcommandAbsence::from)?;
    if let ArgumentLayout::External(subcommand) = layout {
        return Ok(subcommand);
    }
    let arguments = argv
        .iter()
        .skip(start)
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    cargo_subcommand(&command_words(CARGO_DISPLAY_NAME, &arguments))
        .map(str::to_owned)
        .map_err(|RetainedSubcommandAbsence::NoSubcommand| SubcommandAbsence::NoSubcommand)
}

/// Whether `commands.excluded` names the subcommand an argv carries.
///
/// Keyed on the subcommand rather than the binary so one entry covers
/// both spellings of the same command: `cargo berth claim` run through
/// cargo and `cargo-berth berth claim` run as its own binary answer
/// [`subcommand`] the same.
fn is_excluded(argv: &[OsString], excluded: &[String]) -> bool {
    subcommand(argv).is_ok_and(|subcommand| excluded.contains(&subcommand))
}

/// Whether a process's own name is one a cargo invocation wears.
///
/// Three spellings reach here: `cargo` itself, the name a shim's
/// wrapped binary was renamed to, and `cargo-<subcommand>` for every
/// tool installed as an external subcommand. This binary is the one
/// `cargo-` name left out -- cargo-tile watching the builds is not one
/// of the builds.
pub(super) fn is_cargo_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    name != SELF_PROCESS_NAME
        && (CARGO_PROCESS_NAMES.contains(&name) || name.starts_with(CARGO_SUBCOMMAND_PREFIX))
}

/// The subcommand an external cargo tool's binary name carries, or
/// a rejected program when the name is not one.
fn external_subcommand(argument: &OsString) -> Result<String, ExternalSubcommandAbsence> {
    let name = base_name(argument);
    if name == SELF_PROCESS_NAME || CARGO_PROCESS_NAMES.contains(&name.as_str()) {
        return Err(ExternalSubcommandAbsence::ProgramRejected);
    }
    name.strip_prefix(CARGO_SUBCOMMAND_PREFIX)
        .map(str::to_string)
        .ok_or(ExternalSubcommandAbsence::ProgramRejected)
}

/// Whether an argv entry names a cargo binary, under any of the names one
/// gets installed as.
fn is_cargo_binary(argument: &OsString) -> bool {
    CARGO_PROCESS_NAMES.contains(&base_name(argument).as_str())
}

/// An argv entry's trailing path component.
fn base_name(argument: &OsString) -> String {
    PathBuf::from(argument)
        .file_name()
        .unwrap_or(argument.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    #[test]
    fn kept_words_and_argument_omissions_have_distinct_states() {
        let kept = CommandText::of("cargo", &["…"]).named();
        assert_eq!(kept.words, "…");
        assert_eq!(kept.arguments, ArgumentOmission::Kept);
        let dropped = CommandText::of("cargo", &["check", "--workspace"]).named();
        assert_eq!(dropped.words, "check");
        assert_eq!(dropped.arguments, ArgumentOmission::Dropped);
    }
    #[test]
    fn the_short_name_ends_before_subcommand_options() {
        assert_eq!(
            command_name(
                "cargo check --locked --offline -j 1",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "cargo check"
        );
        assert_eq!(
            command_name(
                "cargo nextest run --test-threads 32 --bin hana -E 'package(hana) & (test(arrange_glyph) | …)'"
            , &crate::config::DefaultArguments::default()).words,
            "cargo nextest run"
        );
        assert_eq!(
            CommandText::of(
                "cargo",
                &[
                    "nextest",
                    "run",
                    "--test-threads",
                    "32",
                    "--bin",
                    "hana",
                    "-E",
                    "package(hana) & (test(arrange_glyph) | …)",
                ],
            )
            .named()
            .words,
            "nextest run"
        );
        assert_eq!(
            command_name(
                "cargo --config profile.p19.inherits=\"dev\" build --profile p19 -j 1",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "cargo build"
        );
        assert_eq!(
            command_name(
                "cargo test -p cargo-tile --test cli_lifecycle --test shim_modes --test unit_tests",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "cargo test"
        );
        assert_eq!(
            command_name(
                "cargo --config x=1 -Z unstable-options build --release",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "cargo build"
        );
        assert_eq!(
            command_name(
                "cargo --color always -C /tmp/project check",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "cargo check"
        );
        assert_eq!(
            command_name(
                "cargo --config=x=1 -Zunstable-options -C/tmp/project +nightly fmt",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "cargo +nightly fmt"
        );
    }

    #[test]
    fn long_arguments_keep_the_release_selector() {
        let text = CommandText::of("cargo", &["build", "--release", "--locked"]);
        assert_eq!(text.line(), "build --release --locked");

        assert_eq!(text.profile(), CargoProfile::Release);
        assert_eq!(text.named().words, "build");
    }

    #[test]
    fn long_arguments_keep_a_separate_profile_name() {
        let text = CommandText::of("cargo", &["build", "--profile", "p19c", "--config", "x"]);
        assert_eq!(text.line(), "build --profile p19c --config x");

        assert_eq!(text.profile(), CargoProfile::Named("p19c".to_string()));
        assert_eq!(text.named().words, "build");
    }

    #[test]
    fn long_arguments_keep_an_assigned_profile_name() {
        let text = CommandText::of("cargo", &["build", "--profile=p19c"]);
        assert_eq!(text.line(), "build --profile=p19c");

        assert_eq!(text.profile(), CargoProfile::Named("p19c".to_string()));
        assert_eq!(text.named().words, "build");
    }

    #[test]
    fn cargo_profiles_are_read_before_the_child_arguments() {
        for arguments in [
            vec!["build", "--release"],
            vec!["build", "-r"],
            vec!["build", "--profile", "release"],
            vec!["build", "--profile=release"],
        ] {
            let text = CommandText::of("cargo", &arguments);
            assert_eq!(text.profile(), CargoProfile::Release);
            assert_eq!(text.named().words, "build");
            assert_eq!(text.line(), arguments.join(COMMAND_WORD_SEPARATOR));
        }
        for arguments in [
            vec!["run", "--", "--release"],
            vec!["run", "--", "-r"],
            vec!["run", "--", "--profile", "custom"],
            vec!["build", "--profile", "dev"],
            vec!["build", "--profile=dev"],
            vec!["build"],
        ] {
            assert_eq!(
                CommandText::of("cargo", &arguments).profile(),
                CargoProfile::Default
            );
        }
        for arguments in [
            vec!["build", "--profile", "bench"],
            vec!["build", "--profile=bench"],
        ] {
            assert_eq!(
                CommandText::of("cargo", &arguments).profile(),
                CargoProfile::Named("bench".to_string())
            );
        }
    }

    #[test]
    fn cargo_profile_selectors_stay_in_both_argument_details() {
        for arguments in [
            vec!["build", "--release", "--workspace"],
            vec!["build", "-r", "--workspace"],
            vec!["build", "--profile", "release", "--workspace"],
            vec!["build", "--profile=release", "--workspace"],
            vec!["build", "--profile", "dev", "--workspace"],
            vec!["build", "--profile=dev", "--workspace"],
            vec!["build", "--profile", "bench", "--workspace"],
            vec!["build", "--profile=bench", "--workspace"],
        ] {
            assert_eq!(
                CommandText::of("cargo", &arguments).line(),
                arguments.join(COMMAND_WORD_SEPARATOR)
            );
        }
        assert_eq!(
            CommandText::of(
                "cargo",
                &[
                    "run",
                    "--release",
                    "--",
                    "--release",
                    "-r",
                    "--profile",
                    "dev",
                    "--profile=bench"
                ]
            )
            .line(),
            "run --release -- --release -r --profile dev --profile=bench"
        );
        assert_eq!(
            CommandText::of("cargo", &["build", "--profile-of-record", "bench"]).line(),
            "build --profile-of-record bench"
        );

        let text = CommandText::of(
            "cargo",
            &[
                "build",
                "--color",
                "always",
                "--profile",
                "dev",
                "--workspace",
            ],
        );
        assert_eq!(
            text.line(),
            "build --color always --profile dev --workspace"
        );
    }

    #[test]
    fn the_short_name_keeps_a_subcommand_after_a_flag() {
        assert_eq!(
            CommandText::of("cargo", &["--color", "always", "nextest", "run"])
                .named()
                .words,
            "nextest run"
        );
        assert_eq!(
            CommandText::of("cargo", &["--locked", "check"])
                .named()
                .words,
            "check"
        );
        assert_eq!(
            CommandText::of("cargo", &["--color=always", "+nightly", "nextest", "run"])
                .named()
                .words,
            "+nightly nextest run"
        );
    }

    #[test]
    fn ancestor_names_keep_selected_arguments_without_an_omission_mark() {
        assert_eq!(
            command_name(
                "cargo check --workspace",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "cargo check --workspace"
        );
        assert_eq!(
            command_name("cargo check", &crate::config::DefaultArguments::default()).words,
            "cargo check"
        );
        assert_eq!(
            CommandText::of("cargo", &["check", "--", "child", "--release"])
                .named()
                .words,
            "check"
        );
    }

    #[test]
    fn a_short_view_program_is_named_by_its_file_name() {
        let cargo = CommandText::of("/nix/store/cargo/bin/cargo", &["check", "--workspace"]);
        assert_eq!(cargo.program_name(), "cargo");
        assert_eq!(cargo.named().words, "check");
        assert_eq!(
            command_name(
                "/nix/store/runner/bin/Runner.Worker spawnclient 166 175",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "Runner.Worker"
        );
        assert_eq!(
            command_name(
                "/nix/store/cargo/bin/cargo check --workspace",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "cargo check --workspace"
        );
        assert_eq!(
            command_name(
                "/nix/store/runner/bin/Runner.Listener",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "Runner.Listener"
        );
    }

    use crate::constants::DEFAULT_HIDDEN_COMMANDS;
    fn hidden_commands() -> Vec<String> {
        DEFAULT_HIDDEN_COMMANDS
            .iter()
            .map(|subcommand| (*subcommand).to_string())
            .collect()
    }

    use super::*;
    use crate::constants::COORDINATION_SUBCOMMAND_NAME;
    use crate::constants::DEFAULT_EXCLUDED;
    use crate::constants::PORT_SUBCOMMAND_NAME;

    #[test]
    fn unavailable_argv_and_deliberately_excluded_rows_have_different_outcomes() {
        assert_eq!(cargo_split(&[]), Err(CargoArgvAbsence::ArgvUnavailable));
        let excluded = vec![OsString::from("cargo"), OsString::from("build")];
        assert_eq!(
            select_cargo(&excluded, &[String::from("build")]),
            Err(RowAbsence::PolicyExcluded)
        );
        assert!(matches!(
            cargo_split(&excluded),
            Ok(CargoArguments {
                layout: ArgumentLayout::Cargo,
                ..
            })
        ));
        assert!(
            matches!(cargo_split(&[OsString::from("cargo-nextest"), OsString::from("run")]), Ok(CargoArguments { layout: ArgumentLayout::External(name), .. }) if name == "nextest")
        );
    }

    #[test]
    fn the_subcommand_is_the_first_argument_past_a_toolchain_selector() {
        assert_eq!(
            CommandText::of(CARGO_DISPLAY_NAME, &["+nightly", "build"]).subcommand(),
            Ok("build")
        );
        assert_eq!(
            CommandText::of(CARGO_DISPLAY_NAME, &["build"]).subcommand(),
            Ok("build")
        );
    }

    #[test]
    fn a_subcommand_on_the_list_is_recognised_past_a_toolchain_selector() {
        let selected = CommandText::of(CARGO_DISPLAY_NAME, &["+nightly", PORT_SUBCOMMAND_NAME]);
        assert!(selected.is_hidden(&hidden_commands()));
    }

    #[test]
    fn a_subcommand_off_the_list_is_not_hidden() {
        let building = CommandText::of(CARGO_DISPLAY_NAME, &["build"]);
        assert!(!building.is_hidden(&hidden_commands()));
    }

    #[test]
    fn home_prefix_collapses_to_tilde() {
        let home = PathBuf::from("/Users/someone");
        let path = PathBuf::from("/Users/someone/rust/project");
        assert_eq!(
            home_relative(&path, ScannerHome::Known(&home)),
            "~/rust/project"
        );
    }

    #[test]
    fn home_itself_renders_as_bare_tilde() {
        let home = PathBuf::from("/Users/someone");
        assert_eq!(home_relative(&home, ScannerHome::Known(&home)), "~");
    }

    #[test]
    fn path_outside_home_is_left_alone() {
        let home = PathBuf::from("/Users/someone");
        let path = PathBuf::from("/opt/build");
        assert_eq!(
            home_relative(&path, ScannerHome::Known(&home)),
            "/opt/build"
        );
    }

    #[test]
    fn command_splits_program_from_arguments() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from("build"),
            OsString::from("--release"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.program, "cargo");
        assert_eq!(text.line(), "build --release");
        assert_eq!(text.profile(), CargoProfile::Release);
    }

    #[test]
    fn a_shim_caught_before_handoff_still_reads_as_cargo() {
        let argv = vec![
            OsString::from("/bin/zsh"),
            OsString::from("/Users/someone/.rustup/toolchains/stable/bin/cargo"),
            OsString::from("check"),
            OsString::from("--all-targets"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.program, "cargo");
        assert_eq!(text.line(), "check --all-targets");
    }

    #[test]
    fn a_wrapped_cargo_still_reads_as_cargo() {
        let argv = vec![
            OsString::from("/Users/someone/.rustup/toolchains/stable/bin/cargo-tile-real"),
            OsString::from("build"),
        ];
        assert_eq!(
            command_text(&argv, ScannerHome::Unavailable)
                .expect("argv names a cargo binary")
                .program,
            "cargo"
        );
    }

    /// The list as it reaches [`is_excluded`] once the config has
    /// turned it into owned strings.
    fn excluded() -> Vec<String> {
        DEFAULT_EXCLUDED
            .iter()
            .map(|subcommand| (*subcommand).to_string())
            .collect()
    }

    #[test]
    fn an_excluded_command_after_a_valued_global_option_is_excluded() -> Result<(), RowAbsence> {
        for (option, scope) in COMMAND_VALUED_OPTIONS {
            if !matches!(scope, ValuedOptionScope::Global) {
                continue;
            }
            let argv = ["cargo", "+nightly", option, "k=v", "berth", "claim"].map(OsString::from);
            assert!(is_excluded(&argv, &excluded()), "{option}");
            let command = command_text(&argv, ScannerHome::Unavailable)?;
            assert!(command.is_hidden(&excluded()), "{option}");
        }
        Ok(())
    }

    #[test]
    fn an_excluded_subcommand_run_through_cargo_is_dropped() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from(COORDINATION_SUBCOMMAND_NAME),
            OsString::from("claim"),
        ];
        assert!(is_excluded(&argv, &excluded()));
    }

    /// The same command reached as its own binary, which is how a hook
    /// with the path already resolved runs it. Keying the list on the
    /// subcommand rather than the binary is what makes one entry cover
    /// both.
    #[test]
    fn an_excluded_subcommand_run_as_its_own_binary_is_dropped() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo-berth"),
            OsString::from("claim"),
        ];
        assert!(is_excluded(&argv, &excluded()));
    }

    #[test]
    fn an_excluded_subcommand_is_dropped_past_a_toolchain_selector() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from("+nightly"),
            OsString::from(COORDINATION_SUBCOMMAND_NAME),
            OsString::from("drift"),
        ];
        assert!(is_excluded(&argv, &excluded()));
    }

    #[test]
    fn a_subcommand_off_the_list_is_kept() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from("build"),
            OsString::from("--release"),
        ];
        assert!(!is_excluded(&argv, &excluded()));
    }

    /// An emptied list is the setting turned off, not a list that
    /// matches everything.
    #[test]
    fn an_empty_list_excludes_nothing() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from(COORDINATION_SUBCOMMAND_NAME),
            OsString::from("renew"),
        ];
        assert!(!is_excluded(&argv, &[]));
    }

    /// macOS can report an `sccache` with the name of the cargo that
    /// spawned it, which is how one reaches [`command_text`] at all.
    #[test]
    fn a_compiler_wrapper_wearing_cargos_name_is_not_a_cargo_command() {
        let argv = vec![
            OsString::from("sccache"),
            OsString::from("/Users/someone/.rustup/toolchains/stable/bin/rustc"),
            OsString::from("--crate-name"),
            OsString::from("bevy_transform"),
        ];
        assert!(command_text(&argv, ScannerHome::Unavailable).is_err());
    }

    /// What is being built is what the row is there to say: which
    /// member of a workspace, and how much of it.
    #[test]
    fn the_summary_keeps_what_names_the_work() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("mend"),
            OsString::from("--all-targets"),
            OsString::from("-p"),
            OsString::from("hana_clerestory"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.line(), "mend --all-targets -p hana_clerestory");
    }

    /// Past a bare `--` the arguments are the other program's. It
    /// spells its flags however it likes, and none of them are cargo's
    /// to drop -- a `--color` there is the other program's setting.
    #[test]
    fn the_summary_keeps_everything_handed_to_another_program() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("clippy"),
            OsString::from("--color"),
            OsString::from("never"),
            OsString::from("--"),
            OsString::from("-D"),
            OsString::from("warnings"),
            OsString::from("--color"),
            OsString::from("always"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(
            text.line(),
            "clippy --color never -- -D warnings --color always"
        );
    }

    /// A command's own cell shows the line as it was typed, however
    /// much of it the summary leaves out.
    #[test]
    fn a_cell_of_its_own_keeps_the_whole_line() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("build"),
            OsString::from("--bin"),
            OsString::from("hana"),
            OsString::from("--message-format=json"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.line(), "build --bin hana --message-format=json");
    }

    /// The flag is matched whole: an argument that merely starts the
    /// same way names something else and stays.
    #[test]
    fn an_argument_that_only_starts_like_the_manifest_flag_stays() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("check"),
            OsString::from("--manifest-path-of-record"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.line(), "check --manifest-path-of-record");
    }

    #[test]
    fn arguments_collapse_the_home_prefix() {
        let home = PathBuf::from("/Users/someone");
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from("check"),
            OsString::from("--manifest-path"),
            OsString::from("/Users/someone/rust/project/Cargo.toml"),
        ];
        let text =
            command_text(&argv, ScannerHome::Known(&home)).expect("argv names a cargo binary");
        assert_eq!(
            text.line(),
            "check --manifest-path ~/rust/project/Cargo.toml"
        );
    }

    /// A subcommand of a subcommand is still the name of what runs, so
    /// `nextest run` keeps both words -- and the toolchain selector
    /// keeps its place ahead of them, `+nightly fmt` saying something
    /// `fmt` alone does not.
    #[test]
    fn a_named_command_keeps_its_subcommands_and_its_toolchain() {
        assert_eq!(
            CommandText::of("cargo", &["nextest", "run", "--workspace"])
                .named()
                .words,
            "nextest run",
        );
        assert_eq!(
            CommandText::of("cargo", &["+nightly", "fmt", "--all"])
                .named()
                .words,
            "+nightly fmt",
        );
    }

    /// A chain step names the executable without its argument tail.
    #[test]
    fn a_named_chain_step_uses_the_program_file_name() {
        assert_eq!(
            command_name(
                "~/.claude/local/claude --setting on --setting off",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "claude",
        );
        assert_eq!(
            command_name(
                "zsh -c cargo nextest run",
                &crate::config::DefaultArguments::default()
            )
            .words,
            "zsh"
        );
        assert_eq!(
            command_name("zed", &crate::config::DefaultArguments::default()).words,
            "zed"
        );
        assert_eq!(
            command_name("", &crate::config::DefaultArguments::default()).words,
            ""
        );
    }

    #[test]
    fn unavailable_argv_does_not_identify_a_subcommand() {
        assert_eq!(cargo_split(&[]), Err(CargoArgvAbsence::ArgvUnavailable));
        assert_eq!(subcommand(&[]), Err(SubcommandAbsence::ArgvUnavailable));
    }

    #[test]
    fn a_rejected_program_does_not_identify_a_subcommand() {
        let argv = [OsString::from("rustc"), OsString::from("build")];
        assert_eq!(cargo_split(&argv), Err(CargoArgvAbsence::ProgramRejected));
        assert_eq!(subcommand(&argv), Err(SubcommandAbsence::ProgramRejected));
        for program in ["rustc", "cargo", SELF_PROCESS_NAME] {
            assert_eq!(
                external_subcommand(&OsString::from(program)),
                Err(ExternalSubcommandAbsence::ProgramRejected)
            );
        }
        assert_eq!(
            external_subcommand(&OsString::from("cargo-nextest")),
            Ok(String::from("nextest"))
        );
    }

    #[test]
    fn recognized_cargo_can_name_no_subcommand() {
        for argv in [
            vec![OsString::from("cargo")],
            vec![OsString::from("cargo"), OsString::from("+nightly")],
        ] {
            assert!(cargo_split(&argv).is_ok());
            assert_eq!(subcommand(&argv), Err(SubcommandAbsence::NoSubcommand));
            assert_eq!(
                command_text(&argv, ScannerHome::Unavailable)
                    .expect("recognized cargo")
                    .subcommand(),
                Err(RetainedSubcommandAbsence::NoSubcommand)
            );
        }
        assert_eq!(
            subcommand(&[OsString::from("cargo"), OsString::from("--version")]),
            Err(SubcommandAbsence::NoSubcommand)
        );
        assert_eq!(
            subcommand(&[
                OsString::from("cargo"),
                OsString::from("+nightly"),
                OsString::from("build")
            ]),
            Ok(String::from("build"))
        );
    }
    /// The short display keeps the words that name what runs and stops
    /// at the first argument. A manifest path is what makes these rows
    /// unreadable -- every one of a test suite's cases carries a
    /// different temporary directory, and none of them says anything the
    /// row's own pid does not.
    #[test]
    fn a_named_command_stops_at_its_first_argument() {
        let mend = CommandText::of(
            "cargo",
            &[
                "mend",
                "--manifest-path",
                "/var/folders/T/x/Cargo.toml",
                "--json",
            ],
        );

        assert_eq!(mend.named().words, "mend");
        assert!(
            mend.line().contains("--json"),
            "and the long line still carries every one of them",
        );
    }
}
