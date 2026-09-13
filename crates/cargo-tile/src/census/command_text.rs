//! Cargo argument classification and command display.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use crate::constants::ARGUMENT_SEPARATOR;
use crate::constants::CARGO_DISPLAY_NAME;
use crate::constants::CARGO_PROCESS_NAMES;
use crate::constants::CARGO_SUBCOMMAND_PREFIX;
use crate::constants::CARGO_TOOLCHAIN_SELECTOR;
use crate::constants::FLAG_MARK;
use crate::constants::HOME_ALIAS;
use crate::constants::SELF_PROCESS_NAME;
use crate::constants::SUMMARY_HIDDEN_VALUED_FLAGS;
use crate::render::SummaryDetail;

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
}

impl CommandText {
    /// A command line built from its parts, for the tests elsewhere in
    /// the crate that need an invocation to hand around.
    #[cfg(test)]
    pub(crate) fn of(program: &str, arguments: &[&str]) -> Self {
        Self {
            program:   program.to_string(),
            arguments: arguments.iter().map(|word| (*word).to_string()).collect(),
        }
    }

    /// The cargo subcommand this invocation names: `port` in
    /// `cargo port`, and in `cargo +nightly port` too, the toolchain
    /// selector being no part of it.
    ///
    /// [`command_text`] puts an external subcommand's own name back at
    /// the front of the arguments, so a command that became
    /// `cargo-port` answers this the same as one still spelled
    /// `cargo port`.
    fn subcommand(&self) -> Result<&str, RetainedSubcommandAbsence> {
        self.arguments
            .iter()
            .map(String::as_str)
            .find(|argument| !argument.starts_with(CARGO_TOOLCHAIN_SELECTOR))
            .ok_or(RetainedSubcommandAbsence::NoSubcommand)
    }

    /// Whether `commands.hidden_when_idle` names this command's
    /// subcommand.
    ///
    /// Half the answer to whether the grid gives the command a cell --
    /// the other half is whether anything is running under it, which
    /// [`crate::roster::TrackedGroup::deserves_a_cell`] puts together
    /// with this.
    pub(crate) fn is_hidden_when_idle(&self, hidden_when_idle: &[String]) -> bool {
        self.subcommand()
            .is_ok_and(|subcommand| hidden_when_idle.iter().any(|hidden| hidden == subcommand))
    }

    /// The arguments that still name what runs, with everything the
    /// command was called *with* taken off: `mend` out of `mend
    /// --manifest-path /tmp/x/Cargo.toml --json`, and `nextest run` out
    /// of the whole of `nextest run --workspace --all-features`.
    ///
    /// Keeps the toolchain selector, which is part of what runs rather
    /// than an argument to it -- `+nightly fmt` says something `fmt`
    /// alone does not.
    pub(crate) fn named(&self) -> String {
        self.arguments
            .iter()
            .map(String::as_str)
            .take_while(|word| names_the_command(word))
            .collect::<Vec<&str>>()
            .join(" ")
    }

    /// The arguments as one line, the summary's own flags in or out.
    pub(crate) fn line(&self, detail: SummaryDetail) -> String {
        if detail == SummaryDetail::Full {
            return self.arguments.join(" ");
        }
        let mut kept: Vec<&str> = Vec::with_capacity(self.arguments.len());
        let mut skipping = false;
        let mut handed_over = false;
        for argument in &self.arguments {
            // Everything past a bare `--` belongs to the program cargo
            // runs, which spells its flags however it likes. Nothing
            // there is cargo's to read, so nothing there is dropped.
            if handed_over {
                kept.push(argument);
                continue;
            }
            if argument == ARGUMENT_SEPARATOR {
                handed_over = true;
                kept.push(argument);
                continue;
            }
            // The word after a bare `--color` is the value it takes,
            // and goes wherever the flag goes.
            if std::mem::take(&mut skipping) {
                continue;
            }
            if SUMMARY_HIDDEN_VALUED_FLAGS.contains(&argument.as_str()) {
                skipping = true;
                continue;
            }
            if SUMMARY_HIDDEN_VALUED_FLAGS
                .iter()
                .any(|flag| is_assignment(argument, flag))
            {
                continue;
            }
            kept.push(argument);
        }
        kept.join(" ")
    }
}

/// One whole command line with its arguments taken off, for the steps
/// of a chain, which are held as a line rather than split.
///
/// The first word is the program however it is spelled, path and all --
/// a chain step is often reached by its path, and dropping that would
/// leave a bare `node` or `sh` saying less than the row it heads.
/// Everything after it is kept only while it still names what runs.
pub(crate) fn command_name(line: &str) -> String {
    let mut words = line.split_whitespace();
    let Some(program) = words.next() else {
        return String::new();
    };
    std::iter::once(program)
        .chain(words.take_while(|word| names_the_command(word)))
        .collect::<Vec<&str>>()
        .join(" ")
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

/// Whether an argument is the `--flag=<value>` spelling, which carries
/// the value in the same word instead of the next one.
fn is_assignment(argument: &str, flag: &str) -> bool {
    argument
        .strip_prefix(flag)
        .is_some_and(|rest| rest.starts_with('='))
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
    Ok(CommandText {
        program: CARGO_DISPLAY_NAME.to_string(),
        arguments,
    })
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

/// The subcommand an argv names: the first word past the cargo binary
/// that is neither a flag nor a `+toolchain` selector.
pub(super) fn subcommand(argv: &[OsString]) -> Result<String, SubcommandAbsence> {
    let CargoArguments { start, layout } = cargo_split(argv).map_err(SubcommandAbsence::from)?;
    if let ArgumentLayout::External(subcommand) = layout {
        return Ok(subcommand);
    }
    argv.iter()
        .skip(start)
        .map(|argument| argument.to_string_lossy().into_owned())
        .find(|argument| !argument.starts_with('-') && !argument.starts_with('+'))
        .ok_or(SubcommandAbsence::NoSubcommand)
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
    use crate::constants::DEFAULT_HIDDEN_WHEN_IDLE;
    fn hidden_when_idle() -> Vec<String> {
        DEFAULT_HIDDEN_WHEN_IDLE
            .iter()
            .map(|subcommand| (*subcommand).to_string())
            .collect()
    }

    use super::*;
    use crate::constants::COORDINATION_SUBCOMMAND_NAME;
    use crate::constants::DEFAULT_EXCLUDED;
    use crate::constants::SIBLING_SUBCOMMAND_NAME;

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
        let selected = CommandText::of(CARGO_DISPLAY_NAME, &["+nightly", SIBLING_SUBCOMMAND_NAME]);
        assert!(selected.is_hidden_when_idle(&hidden_when_idle()));
    }

    #[test]
    fn a_subcommand_off_the_list_is_not_hidden() {
        let building = CommandText::of(CARGO_DISPLAY_NAME, &["build"]);
        assert!(!building.is_hidden_when_idle(&hidden_when_idle()));
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
        assert_eq!(text.line(SummaryDetail::Full), "build --release");
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
        assert_eq!(text.line(SummaryDetail::Full), "check --all-targets");
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

    #[test]
    fn the_summary_drops_the_manifest_path_and_the_flag_naming_it() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("check"),
            OsString::from("--manifest-path"),
            OsString::from("/opt/project/Cargo.toml"),
            OsString::from("--all-targets"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.line(SummaryDetail::Trimmed), "check --all-targets");
    }

    #[test]
    fn the_summary_drops_a_manifest_path_written_as_one_word() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("check"),
            OsString::from("--manifest-path=/opt/project/Cargo.toml"),
            OsString::from("--all-targets"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.line(SummaryDetail::Trimmed), "check --all-targets");
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
        assert_eq!(
            text.line(SummaryDetail::Trimmed),
            "mend --all-targets -p hana_clerestory"
        );
    }

    /// A rendering flag says how the caller wanted the output, which is
    /// the caller's business rather than the run's -- in either
    /// spelling, and wherever in the line it falls.
    #[test]
    fn the_summary_drops_the_rendering_flags() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("--color=auto"),
            OsString::from("test"),
            OsString::from("--no-run"),
            OsString::from("--message-format"),
            OsString::from("json-render-diagnostics"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.line(SummaryDetail::Trimmed), "test --no-run");
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
            text.line(SummaryDetail::Trimmed),
            "clippy -- -D warnings --color always"
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
        assert_eq!(
            text.line(SummaryDetail::Full),
            "build --bin hana --message-format=json"
        );
        assert_eq!(text.line(SummaryDetail::Trimmed), "build --bin hana");
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
        assert_eq!(
            text.line(SummaryDetail::Trimmed),
            "check --manifest-path-of-record"
        );
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
            text.line(SummaryDetail::Full),
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
            CommandText::of("cargo", &["nextest", "run", "--workspace"]).named(),
            "nextest run",
        );
        assert_eq!(
            CommandText::of("cargo", &["+nightly", "fmt", "--all"]).named(),
            "+nightly fmt",
        );
    }

    /// A chain step is held as one line rather than split, and its
    /// program is often reached by its path -- so the first word stands
    /// however it is spelled, and only what follows is read for
    /// arguments. A bare `node` would say less than the row it heads.
    #[test]
    fn a_named_chain_step_keeps_the_path_it_was_reached_by() {
        assert_eq!(
            command_name("~/.claude/local/claude --setting on --setting off"),
            "~/.claude/local/claude",
        );
        assert_eq!(command_name("zsh -c cargo nextest run"), "zsh");
        assert_eq!(command_name("zed"), "zed");
        assert_eq!(command_name(""), "");
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

        assert_eq!(mend.named(), "mend");
        assert!(
            mend.line(SummaryDetail::Full).contains("--json"),
            "and the long line still carries every one of them",
        );
    }
}
