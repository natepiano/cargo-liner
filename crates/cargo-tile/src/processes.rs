//! Discovery of the `cargo` invocations running on this machine.
//!
//! Scanning happens on a background thread and arrives over a channel, so
//! the render loop never pays for it. Each scan is two-phase: a cheap
//! full-system pass reading only pid, name, parent and start time, then a
//! targeted pass reading working directory and argv for the handful of
//! processes that turned out to be cargo. The expensive per-process reads
//! are therefore never spent on the `rustc` and `sccache` processes a
//! build churns through by the hundred.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

use chrono::DateTime;
use chrono::Local;
use sysinfo::Pid;
use sysinfo::Process;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;

use crate::constants::CARGO_DISPLAY_NAME;
use crate::constants::CARGO_PROCESS_NAMES;
use crate::constants::COMPILER_PROCESS_NAMES;
use crate::constants::HOME_ALIAS;
use crate::constants::PARENT_WALK_LIMIT;
use crate::constants::PROCESS_POLL_MILLIS;
use crate::constants::SECONDS_PER_HOUR;
use crate::constants::SECONDS_PER_MINUTE;
use crate::constants::START_TIME_FORMAT;
use crate::constants::UNRESOLVED_PATH;
use crate::constants::UNRESOLVED_TIME;

/// One running `cargo` invocation, preformatted for the table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CargoProcess {
    /// Working directory with the home prefix collapsed to `~`.
    pub(crate) path:     String,
    /// Process id.
    pub(crate) pid:      u32,
    /// Local wall-clock start time, `hh:mm`.
    pub(crate) start:    String,
    /// Elapsed run time, `mm:ss` until an hour and `hh:mm:ss` past it.
    pub(crate) duration: String,
    /// Compiler processes this invocation currently owns, if any.
    pub(crate) compiler: Option<Compiler>,
    /// The command line, split so program and arguments style apart.
    pub(crate) command:  CommandText,
}

/// The compiler driver an invocation is running, and how many at once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Compiler {
    /// Driver name, one of [`COMPILER_PROCESS_NAMES`].
    pub(crate) name:  &'static str,
    /// How many of it are running under this cargo invocation.
    pub(crate) count: usize,
}

/// A command line split into its program and the rest of its arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandText {
    /// Program name, path stripped.
    pub(crate) program:   String,
    /// Remaining arguments, space-joined.
    pub(crate) arguments: String,
}

/// Start the scanner thread and hand back the channel it publishes on.
///
/// The thread ends when the receiver is dropped.
pub(crate) fn spawn() -> Receiver<Vec<CargoProcess>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut system = System::new();
        let home = dirs::home_dir();
        loop {
            if sender.send(scan(&mut system, home.as_deref())).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(PROCESS_POLL_MILLIS));
        }
    });
    receiver
}

/// One two-phase scan, newest invocation first.
fn scan(system: &mut System, home: Option<&Path>) -> Vec<CargoProcess> {
    // Phase one: pid, name, parent and start time for everything. None of
    // the fields this asks for require a per-process read of the argument
    // area, which is what makes it cheap enough to poll continuously.
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());

    let mut census = Census::take(system);
    census.collapse_wrappers();

    // Phase two: the costly fields, for cargo processes only.
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&census.cargo),
        false,
        ProcessRefreshKind::nothing()
            .with_cwd(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );

    // A process can carry the name `cargo` without being one, so the argv
    // phase two just read is what settles it -- see `command_text`.
    // Pruning here rather than at render time keeps a mislabelled
    // `sccache` from claiming the compilers that belong to the cargo
    // above it, which `attribute_compilers` is about to hand out.
    census.cargo.retain(|pid| {
        system
            .process(*pid)
            .is_some_and(|process| cargo_argv_start(process.cmd()).is_some())
    });

    let counts = census.attribute_compilers();
    let mut dated: Vec<(u64, CargoProcess)> = census
        .cargo
        .iter()
        .filter_map(|pid| {
            let process = system.process(*pid)?;
            Some((process.start_time(), row(process, *pid, &counts, home)?))
        })
        .collect();
    // Newest first, so a cargo command just fired off lands at the top.
    dated.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.pid.cmp(&right.1.pid)));
    dated.into_iter().map(|(_, process)| process).collect()
}

/// What phase one learned: the parent links, the cargo processes, and the
/// compiler processes waiting to be attributed to one of them.
struct Census {
    /// Every process's parent, for walking a compiler up to its cargo.
    parents:   HashMap<Pid, Pid>,
    /// Processes whose own name is `cargo`.
    cargo:     Vec<Pid>,
    /// Compiler processes paired with which driver they are.
    compilers: Vec<(Pid, &'static str)>,
}

impl Census {
    /// Classify every process the last refresh saw.
    fn take(system: &System) -> Self {
        let mut census = Self {
            parents:   HashMap::new(),
            cargo:     Vec::new(),
            compilers: Vec::new(),
        };
        for (&pid, process) in system.processes() {
            if let Some(parent) = process.parent() {
                census.parents.insert(pid, parent);
            }
            let name = process.name();
            if CARGO_PROCESS_NAMES
                .iter()
                .any(|cargo| name == OsStr::new(*cargo))
            {
                census.cargo.push(pid);
            } else if let Some(driver) = COMPILER_PROCESS_NAMES
                .iter()
                .find(|driver| name == OsStr::new(**driver))
            {
                census.compilers.push((pid, driver));
            }
        }
        census
    }

    /// Count each cargo invocation's compiler descendants, reporting the
    /// highest-priority driver present in [`COMPILER_PROCESS_NAMES`].
    ///
    /// `sccache` outranks `rustc` because when a wrapper is in use every
    /// `rustc` is a child of one, and reporting both would double-count
    /// the same compile.
    fn attribute_compilers(&self) -> HashMap<Pid, Compiler> {
        let mut tallies: HashMap<Pid, HashMap<&'static str, usize>> = HashMap::new();
        for &(pid, driver) in &self.compilers {
            if let Some(owner) = self.owning_cargo(pid) {
                *tallies.entry(owner).or_default().entry(driver).or_default() += 1;
            }
        }
        tallies
            .into_iter()
            .filter_map(|(owner, tally)| {
                let driver = COMPILER_PROCESS_NAMES
                    .iter()
                    .find_map(|driver| tally.get(driver).map(|count| (*driver, *count)))?;
                Some((
                    owner,
                    Compiler {
                        name:  driver.0,
                        count: driver.1,
                    },
                ))
            })
            .collect()
    }

    /// Drop any candidate that has another candidate beneath it.
    ///
    /// A shim that wraps cargo is itself named `cargo` — that is the whole
    /// point of a shim — so one command can present as two processes: the
    /// wrapper and the real cargo it spawned. The inner one is the process
    /// actually doing the work, so the outer one goes.
    fn collapse_wrappers(&mut self) {
        let candidates = self.cargo.clone();
        let mut wrappers: Vec<Pid> = Vec::new();
        for &pid in &candidates {
            let mut current = pid;
            for _ in 0..PARENT_WALK_LIMIT {
                let Some(&parent) = self.parents.get(&current) else {
                    break;
                };
                if candidates.contains(&parent) {
                    wrappers.push(parent);
                }
                current = parent;
            }
        }
        self.cargo.retain(|pid| !wrappers.contains(pid));
    }

    /// Walk `pid` up its parent chain to the cargo invocation that owns
    /// it, bounded by [`PARENT_WALK_LIMIT`] so a reparented process whose
    /// chain loops back on itself cannot spin here.
    fn owning_cargo(&self, pid: Pid) -> Option<Pid> {
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            let parent = *self.parents.get(&current)?;
            if self.cargo.contains(&parent) {
                return Some(parent);
            }
            current = parent;
        }
        None
    }
}

/// Format one cargo process into its table row.
fn row(
    process: &Process,
    pid: Pid,
    counts: &HashMap<Pid, Compiler>,
    home: Option<&Path>,
) -> Option<CargoProcess> {
    Some(CargoProcess {
        path:     process.cwd().map_or_else(
            || UNRESOLVED_PATH.to_string(),
            |cwd| home_relative(cwd, home),
        ),
        pid:      pid.as_u32(),
        start:    start_label(process.start_time()),
        duration: duration_label(process.run_time()),
        compiler: counts.get(&pid).cloned(),
        command:  command_text(process.cmd(), home)?,
    })
}

/// Render `path` with the home directory collapsed to `~`.
fn home_relative(path: &Path, home: Option<&Path>) -> String {
    let full = path.display().to_string();
    let Some(home) = home else {
        return full;
    };
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => HOME_ALIAS.to_string(),
        Ok(rest) => format!("{HOME_ALIAS}/{}", rest.display()),
        Err(_) => full,
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

/// Split argv into the program's bare name and the rest of the line,
/// answering `None` when argv does not name a cargo binary anywhere.
///
/// A cargo binary installed under an alias still reads as `cargo`: the
/// name on disk is an artifact of how it was wrapped, not of what the
/// user typed.
///
/// The `None` case is what keeps the table honest about what a process
/// is. [`Census::take`] classifies on [`sysinfo::Process::name`], and
/// macOS does not always let sysinfo read a process's executable: when
/// it cannot, the name reported is the parent's. Every `sccache` a build
/// spawns is a child of cargo, so a whole burst of them can present as
/// cargo at once. Their argv still reads `sccache /path/to/rustc …`,
/// which names no cargo binary, and that is what settles it.
fn command_text(argv: &[OsString], home: Option<&Path>) -> Option<CommandText> {
    let start = cargo_argv_start(argv)?;
    let arguments = argv
        .iter()
        .skip(start + 1)
        .map(|argument| home_relative(Path::new(argument), home))
        .collect::<Vec<_>>()
        .join(" ");
    Some(CommandText {
        program: CARGO_DISPLAY_NAME.to_string(),
        arguments,
    })
}

/// Where the cargo binary sits in argv, or `None` when none of it names
/// one.
///
/// A shim caught before it hands off still has its interpreter at
/// argv[0] — `zsh /path/to/cargo check …`. Starting at the cargo binary
/// instead renders that identically to the same command a moment later,
/// once the real cargo is running it.
fn cargo_argv_start(argv: &[OsString]) -> Option<usize> { argv.iter().position(is_cargo_binary) }

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
    use super::*;

    #[test]
    fn duration_stays_minutes_and_seconds_under_an_hour() {
        assert_eq!(duration_label(57), "00:57");
        assert_eq!(duration_label(3599), "59:59");
    }

    #[test]
    fn duration_widens_to_hours_once_pathological() {
        assert_eq!(duration_label(3600), "01:00:00");
        assert_eq!(duration_label(45_296), "12:34:56");
    }

    #[test]
    fn home_prefix_collapses_to_tilde() {
        let home = PathBuf::from("/Users/someone");
        let path = PathBuf::from("/Users/someone/rust/project");
        assert_eq!(home_relative(&path, Some(&home)), "~/rust/project");
    }

    #[test]
    fn home_itself_renders_as_bare_tilde() {
        let home = PathBuf::from("/Users/someone");
        assert_eq!(home_relative(&home, Some(&home)), "~");
    }

    #[test]
    fn path_outside_home_is_left_alone() {
        let home = PathBuf::from("/Users/someone");
        let path = PathBuf::from("/opt/build");
        assert_eq!(home_relative(&path, Some(&home)), "/opt/build");
    }

    #[test]
    fn command_splits_program_from_arguments() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from("build"),
            OsString::from("--release"),
        ];
        let text = command_text(&argv, None).expect("argv names a cargo binary");
        assert_eq!(text.program, "cargo");
        assert_eq!(text.arguments, "build --release");
    }

    #[test]
    fn a_shim_caught_before_handoff_still_reads_as_cargo() {
        let argv = vec![
            OsString::from("/bin/zsh"),
            OsString::from("/Users/someone/.rustup/toolchains/stable/bin/cargo"),
            OsString::from("check"),
            OsString::from("--all-targets"),
        ];
        let text = command_text(&argv, None).expect("argv names a cargo binary");
        assert_eq!(text.program, "cargo");
        assert_eq!(text.arguments, "check --all-targets");
    }

    #[test]
    fn a_wrapped_cargo_still_reads_as_cargo() {
        let argv = vec![
            OsString::from("/Users/someone/.rustup/toolchains/stable/bin/cargo-tile-real"),
            OsString::from("build"),
        ];
        assert_eq!(
            command_text(&argv, None)
                .expect("argv names a cargo binary")
                .program,
            "cargo"
        );
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
        assert!(command_text(&argv, None).is_none());
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
        let text = command_text(&argv, Some(&home)).expect("argv names a cargo binary");
        assert_eq!(
            text.arguments,
            "check --manifest-path ~/rust/project/Cargo.toml"
        );
    }
}
