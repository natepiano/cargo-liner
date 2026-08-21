//! Discovery of the `cargo` invocations running on this machine, rolled
//! up into the groups the display is built from.
//!
//! Scanning happens on a background thread and arrives over a channel, so
//! the render loop never pays for it. Each scan is two-phase: a cheap
//! full-system pass reading only pid, name, parent and start time, then a
//! targeted pass reading working directory and argv for the handful of
//! processes that turned out to be cargo. The expensive per-process reads
//! are therefore never spent on the `rustc` and `sccache` processes a
//! build churns through by the hundred.
//!
//! What comes out is not a flat list. One command a developer typed can
//! be a whole tree of cargo processes -- `cargo mend` driving a
//! `cargo nextest` suite that runs `cargo check` per crate -- and a flat
//! list reports that as a dozen unrelated rows. [`CargoGroup`] keeps the
//! tree: the outermost invocation leads, everything running under it
//! follows, and the summary can show one row per command with a count
//! beside it.

use std::collections::HashMap;
use std::collections::HashSet;
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
use crate::constants::CARGO_SUBCOMMAND_PREFIX;
use crate::constants::COMPILER_PROCESS_NAMES;
use crate::constants::HOME_ALIAS;
use crate::constants::MANIFEST_PATH_FLAG;
use crate::constants::PARENT_WALK_LIMIT;
use crate::constants::PROCESS_POLL_MILLIS;
use crate::constants::SECONDS_PER_HOUR;
use crate::constants::SECONDS_PER_MINUTE;
use crate::constants::SELF_PROCESS_NAME;
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
    /// Compiler processes this invocation currently owns, if any. On the
    /// invocation leading a group this is the whole group's tally, so
    /// the summary reports the build rather than the driver process.
    pub(crate) compiler: Option<Compiler>,
    /// Cargo invocations running under this one. Zero for a plain
    /// command, which is what most rows are.
    pub(crate) managed:  usize,
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
    pub(crate) program: String,
    /// Remaining arguments, one entry per argv word. Held split rather
    /// than joined because a cell may leave one of them out;
    /// [`CommandText::line`] is what puts them back into a line.
    arguments:          Vec<String>,
}

/// Whether a cell shows the manifest path an invocation names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManifestPath {
    /// Show it, the way a command's own cell shows the whole line.
    Shown,
    /// Leave it out, the way the summary does. Every row there already
    /// sits under the working directory heading its group, and cargo is
    /// handed the manifest as an absolute path -- long enough to push
    /// the subcommand off the edge of a narrow cell to repeat what the
    /// header just said.
    Hidden,
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

    /// The arguments as one line, the manifest path in or out.
    pub(crate) fn line(&self, manifest: ManifestPath) -> String {
        if manifest == ManifestPath::Shown {
            return self.arguments.join(" ");
        }
        let mut kept: Vec<&str> = Vec::with_capacity(self.arguments.len());
        let mut skipping = false;
        for argument in &self.arguments {
            // The word after a bare `--manifest-path` is the path it
            // takes, and goes wherever the flag goes.
            if std::mem::take(&mut skipping) {
                continue;
            }
            if argument == MANIFEST_PATH_FLAG {
                skipping = true;
                continue;
            }
            if is_manifest_assignment(argument) {
                continue;
            }
            kept.push(argument);
        }
        kept.join(" ")
    }
}

/// Whether an argument is the `--manifest-path=<path>` spelling, which
/// carries the path in the same word instead of the next one.
fn is_manifest_assignment(argument: &str) -> bool {
    argument
        .strip_prefix(MANIFEST_PATH_FLAG)
        .is_some_and(|rest| rest.starts_with('='))
}

/// One command and every cargo invocation running under it.
///
/// A plain `cargo build` is a group of one. A command that drives other
/// cargo commands is one group holding all of them, which is what lets
/// the summary carry the command that was typed with a count beside it
/// instead of the fan-out it became.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CargoGroup {
    /// The outermost invocation: the command that was typed, and the row
    /// the summary carries.
    pub(crate) lead: CargoProcess,
    /// Everything running under [`lead`](Self::lead), newest first.
    /// Empty for a plain command.
    pub(crate) rest: Vec<CargoProcess>,
}

impl CargoGroup {
    /// The group's identity, stable for as long as the command runs.
    pub(crate) const fn id(&self) -> u32 { self.lead.pid }
}

/// Start the scanner thread and hand back the channel it publishes on.
///
/// The thread ends when the receiver is dropped.
pub(crate) fn spawn() -> Receiver<Vec<CargoGroup>> {
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

/// One two-phase scan, newest group first.
fn scan(system: &mut System, home: Option<&Path>) -> Vec<CargoGroup> {
    // Phase one: pid, name, parent and start time for everything. None of
    // the fields this asks for require a per-process read of the argument
    // area, which is what makes it cheap enough to poll continuously.
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());

    let mut census = Census::take(system);

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
            .is_some_and(|process| names_cargo(process.cmd()))
    });

    // Shims are separated from managers on argv, so this waits for phase
    // two rather than running on names alone. The cost is reading argv
    // for the wrappers too, which is a handful of processes against the
    // hundreds phase two already skips.
    census.collapse_shims(system);

    let counts = census.attribute_compilers();
    census.groups(system, &counts, home)
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
            if is_cargo_name(name) {
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

    /// Drop every cargo that is only a shim in front of another cargo.
    ///
    /// A shim that wraps cargo is itself named `cargo` -- that is the
    /// whole point of a shim -- so one command can present as two
    /// processes. What separates that from a command *managing* other
    /// cargo commands is whether the child is the same command: a shim
    /// hands its line straight on, so the subcommand and the working
    /// directory both match, while `cargo mend` running a `cargo nextest`
    /// suite matches neither. The shim goes and the process doing the
    /// work stays; the manager stays and keeps its children.
    ///
    /// Looping rather than one pass because a shim can stand in front of
    /// a shim, and dropping the outer one is what reveals the next.
    fn collapse_shims(&mut self, system: &System) {
        loop {
            let children = self.cargo_children();
            let shims: Vec<Pid> = self
                .cargo
                .iter()
                .copied()
                .filter(|&pid| Self::is_shim(system, &children, pid))
                .collect();
            if shims.is_empty() {
                return;
            }
            self.cargo.retain(|pid| !shims.contains(pid));
        }
    }

    /// Whether `pid` is a shim in front of the one cargo beneath it.
    fn is_shim(system: &System, children: &HashMap<Pid, Vec<Pid>>, pid: Pid) -> bool {
        let Some(kids) = children.get(&pid) else {
            return false;
        };
        let [child] = kids[..] else {
            return false;
        };
        let (Some(outer), Some(inner)) = (system.process(pid), system.process(child)) else {
            return false;
        };
        subcommand(outer.cmd()) == subcommand(inner.cmd()) && outer.cwd() == inner.cwd()
    }

    /// Each cargo's direct cargo children -- the tree the groups are cut
    /// from, with a process that came out as its own ancestor dropped so
    /// a walk of it cannot loop.
    fn cargo_children(&self) -> HashMap<Pid, Vec<Pid>> {
        let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for &pid in &self.cargo {
            if let Some(owner) = self.owning_cargo(pid)
                && owner != pid
            {
                children.entry(owner).or_default().push(pid);
            }
        }
        children
    }

    /// Every cargo running under `root`, at any depth.
    fn descendants(children: &HashMap<Pid, Vec<Pid>>, root: Pid) -> Vec<Pid> {
        let mut seen: HashSet<Pid> = HashSet::from([root]);
        let mut queue = vec![root];
        let mut out = Vec::new();
        while let Some(pid) = queue.pop() {
            let Some(kids) = children.get(&pid) else {
                continue;
            };
            for &kid in kids {
                if seen.insert(kid) {
                    out.push(kid);
                    queue.push(kid);
                }
            }
        }
        out
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

    /// Every group the surviving cargo set forms, newest lead first.
    ///
    /// A lead ties with another when both started inside the same second
    /// -- the start time is whole seconds, since sysinfo reads the
    /// kernel's `pbi_start_tvsec` and drops the microseconds beside it.
    /// Pid breaks the tie in the same direction: macOS hands them out in
    /// order, so within one second the higher pid is the later start.
    fn groups(
        &self,
        system: &System,
        counts: &HashMap<Pid, Compiler>,
        home: Option<&Path>,
    ) -> Vec<CargoGroup> {
        let children = self.cargo_children();
        let mut dated: Vec<(u64, CargoGroup)> = self
            .cargo
            .iter()
            .filter(|&&pid| self.owning_cargo(pid).is_none_or(|owner| owner == pid))
            .filter_map(|&pid| {
                let start = system.process(pid)?.start_time();
                Some((start, Self::group(system, counts, home, &children, pid)?))
            })
            .collect();
        dated.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then(right.1.lead.pid.cmp(&left.1.lead.pid))
        });
        dated.into_iter().map(|(_, group)| group).collect()
    }

    /// Build the group led by `root`.
    fn group(
        system: &System,
        counts: &HashMap<Pid, Compiler>,
        home: Option<&Path>,
        children: &HashMap<Pid, Vec<Pid>>,
        root: Pid,
    ) -> Option<CargoGroup> {
        let managed = Self::descendants(children, root);
        let mut lead = row(
            system.process(root)?,
            root,
            counts.get(&root).cloned(),
            managed.len(),
            home,
        )?;
        // The lead reports the whole group's compilers: what a developer
        // wants off the summary row is how much work the command they
        // typed is doing, and for a manager none of that work is running
        // under the manager's own pid.
        lead.compiler = aggregate_compilers(counts, std::iter::once(root).chain(managed.clone()));

        let mut dated: Vec<(u64, CargoProcess)> = managed
            .into_iter()
            .filter_map(|pid| {
                let process = system.process(pid)?;
                let under = Self::descendants(children, pid).len();
                Some((
                    process.start_time(),
                    row(process, pid, counts.get(&pid).cloned(), under, home)?,
                ))
            })
            .collect();
        dated.sort_by(|left, right| right.0.cmp(&left.0).then(right.1.pid.cmp(&left.1.pid)));
        Some(CargoGroup {
            lead,
            rest: dated.into_iter().map(|(_, process)| process).collect(),
        })
    }
}

/// One compiler tally across a whole group: the highest-priority driver
/// any member is running, totalled over all of them.
fn aggregate_compilers(
    counts: &HashMap<Pid, Compiler>,
    members: impl Iterator<Item = Pid>,
) -> Option<Compiler> {
    let running: Vec<&Compiler> = members.filter_map(|pid| counts.get(&pid)).collect();
    let name = COMPILER_PROCESS_NAMES
        .iter()
        .find(|driver| running.iter().any(|compiler| compiler.name == **driver))?;
    Some(Compiler {
        name,
        count: running
            .iter()
            .filter(|compiler| compiler.name == *name)
            .map(|compiler| compiler.count)
            .sum(),
    })
}

/// Format one cargo process into its table row.
fn row(
    process: &Process,
    pid: Pid,
    compiler: Option<Compiler>,
    managed: usize,
    home: Option<&Path>,
) -> Option<CargoProcess> {
    Some(CargoProcess {
        path: process.cwd().map_or_else(
            || UNRESOLVED_PATH.to_string(),
            |cwd| home_relative(cwd, home),
        ),
        pid: pid.as_u32(),
        start: start_label(process.start_time()),
        duration: duration_label(process.run_time()),
        compiler,
        managed,
        command: command_text(process.cmd(), home)?,
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
    let (start, subcommand) = cargo_split(argv)?;
    let mut arguments: Vec<String> = argv
        .iter()
        .skip(start)
        .map(|argument| home_relative(Path::new(argument), home))
        .collect();
    // An external subcommand's binary is usually handed its own name
    // back as the first argument -- `cargo-nextest nextest run` -- but
    // a caller invoking the binary directly skips that. Putting it back
    // is what makes both spell the command that was typed.
    if let Some(subcommand) = subcommand
        && arguments.first() != Some(&subcommand)
    {
        arguments.insert(0, subcommand);
    }
    Some(CommandText {
        program: CARGO_DISPLAY_NAME.to_string(),
        arguments,
    })
}

/// Where a cargo invocation's arguments start in its argv, and the
/// subcommand its binary name carries when it is an external one.
///
/// Two layouts reach here. A cargo binary somewhere in argv -- the
/// common case, and the one a shim caught mid-handoff also takes --
/// puts the arguments straight after it. An external subcommand carries
/// no cargo binary at all: `cargo mend` *becomes* `cargo-mend`, so the
/// subcommand is in the process's own name and the arguments are
/// everything after argv\[0\].
fn cargo_split(argv: &[OsString]) -> Option<(usize, Option<String>)> {
    if let Some(start) = cargo_argv_start(argv) {
        return Some((start + 1, None));
    }
    let subcommand = external_subcommand(argv.first()?)?;
    Some((1, Some(subcommand)))
}

/// Whether an argv belongs to a cargo invocation at all.
///
/// [`Census::take`] classifies on the process's own name, and a process
/// can wear one without being one -- see [`command_text`] -- so this is
/// what settles it.
fn names_cargo(argv: &[OsString]) -> bool { cargo_split(argv).is_some() }

/// The subcommand an argv names: the first word past the cargo binary
/// that is neither a flag nor a `+toolchain` selector.
fn subcommand(argv: &[OsString]) -> Option<String> {
    let (start, external) = cargo_split(argv)?;
    if external.is_some() {
        return external;
    }
    argv.iter()
        .skip(start)
        .map(|argument| argument.to_string_lossy().into_owned())
        .find(|argument| !argument.starts_with('-') && !argument.starts_with('+'))
}

/// Whether a process's own name is one a cargo invocation wears.
///
/// Three spellings reach here: `cargo` itself, the name a shim's
/// wrapped binary was renamed to, and `cargo-<subcommand>` for every
/// tool installed as an external subcommand. This binary is the one
/// `cargo-` name left out -- cargo-tile watching the builds is not one
/// of the builds.
fn is_cargo_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    name != SELF_PROCESS_NAME
        && (CARGO_PROCESS_NAMES.contains(&name) || name.starts_with(CARGO_SUBCOMMAND_PREFIX))
}

/// The subcommand an external cargo tool's binary name carries, or
/// `None` when the name is not one.
fn external_subcommand(argument: &OsString) -> Option<String> {
    let name = base_name(argument);
    if name == SELF_PROCESS_NAME || CARGO_PROCESS_NAMES.contains(&name.as_str()) {
        return None;
    }
    Some(name.strip_prefix(CARGO_SUBCOMMAND_PREFIX)?.to_string())
}

/// Where the cargo binary sits in argv, or `None` when none of it names
/// one.
///
/// A shim caught before it hands off still has its interpreter at
/// argv\[0\] — `zsh /path/to/cargo check …`. Starting at the cargo binary
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
        assert_eq!(text.line(ManifestPath::Shown), "build --release");
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
        assert_eq!(text.line(ManifestPath::Shown), "check --all-targets");
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
    fn the_summary_drops_the_manifest_path_and_the_flag_naming_it() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("check"),
            OsString::from("--manifest-path"),
            OsString::from("/opt/project/Cargo.toml"),
            OsString::from("--all-targets"),
        ];
        let text = command_text(&argv, None).expect("argv names a cargo binary");
        assert_eq!(text.line(ManifestPath::Hidden), "check --all-targets");
    }

    #[test]
    fn the_summary_drops_a_manifest_path_written_as_one_word() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("check"),
            OsString::from("--manifest-path=/opt/project/Cargo.toml"),
            OsString::from("--all-targets"),
        ];
        let text = command_text(&argv, None).expect("argv names a cargo binary");
        assert_eq!(text.line(ManifestPath::Hidden), "check --all-targets");
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
        let text = command_text(&argv, None).expect("argv names a cargo binary");
        assert_eq!(
            text.line(ManifestPath::Hidden),
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
        let text = command_text(&argv, Some(&home)).expect("argv names a cargo binary");
        assert_eq!(
            text.line(ManifestPath::Shown),
            "check --manifest-path ~/rust/project/Cargo.toml"
        );
    }
}
