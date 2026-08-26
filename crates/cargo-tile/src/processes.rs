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
use std::time::Instant;

use chrono::DateTime;
use chrono::Local;
use sysinfo::Pid;
use sysinfo::Process;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;
use tui_pane::kernel_parent;

use crate::constants::ARGUMENT_SEPARATOR;
use crate::constants::CARGO_DISPLAY_NAME;
use crate::constants::CARGO_PROCESS_NAMES;
use crate::constants::CARGO_SUBCOMMAND_PREFIX;
use crate::constants::CARGO_TOOLCHAIN_SELECTOR;
use crate::constants::COMPILER_PROCESS_NAMES;
use crate::constants::CPU_REPORT_MILLIS;
use crate::constants::CPU_SMOOTHING_SECONDS;
use crate::constants::FLAG_MARK;
use crate::constants::HOME_ALIAS;
use crate::constants::PARENT_WALK_LIMIT;
use crate::constants::PROCESS_POLL_MILLIS;
use crate::constants::ROOT_PROCESS_PID;
use crate::constants::SCCACHE_BINARY;
use crate::constants::SECONDS_PER_HOUR;
use crate::constants::SECONDS_PER_MINUTE;
use crate::constants::SELF_PROCESS_NAME;
use crate::constants::START_TIME_FORMAT;
use crate::constants::SUMMARY_HIDDEN_VALUED_FLAGS;
use crate::constants::TRANSPARENT_PROCESS_NAMES;
use crate::constants::UNRESOLVED_PATH;
use crate::constants::UNRESOLVED_TIME;
use crate::progress::Capture;
use crate::progress::RunState;
use crate::sccache::SccacheServer;

/// One running `cargo` invocation, preformatted for the table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CargoProcess {
    /// Working directory with the home prefix collapsed to `~`.
    pub(crate) path:     String,
    /// Process id.
    pub(crate) pid:      u32,
    /// The nearest ancestor the cell draws: the cargo above this one
    /// where there is one, since that is a row of the same table, and
    /// otherwise the step of the chain block the command was started
    /// from. Never the immediate parent, which is the pty and shim the
    /// capture opened and is drawn nowhere.
    ///
    /// `None` only where the walk reaches the top having found nothing
    /// on screen.
    pub(crate) parent:   Option<u32>,
    /// Local wall-clock start time, `hh:mm`.
    pub(crate) start:    String,
    /// The same instant as seconds since the epoch, which is what
    /// orders one invocation against another. The label above it is
    /// only accurate to the minute and turns over at midnight, so it
    /// reads well and sorts badly.
    pub(crate) started:  u64,
    /// Elapsed run time, `mm:ss` until an hour and `hh:mm:ss` past it.
    pub(crate) duration: String,
    /// Share of a core this invocation and everything running under it
    /// are using, as a whole-number percent. `top`'s scale rather than a
    /// share of the machine, so a build across eight cores reads past
    /// 100% instead of flattening to a tenth of one.
    pub(crate) cpu:      String,
    /// Compiler processes this invocation currently owns, if any. On the
    /// invocation leading a group this is the whole group's tally, so
    /// the summary reports the build rather than the driver process.
    pub(crate) compiler: Option<Compiler>,
    /// What the command is doing, when a capture of its output is there
    /// to read it from. Read off the nearest capture at or above the
    /// invocation, so a cargo the enclosing run started -- which the
    /// shim declines to capture a second time -- reports the run it is
    /// inside rather than nothing at all.
    pub(crate) state:    Option<RunState>,
    /// Cargo invocations running under this one. Zero for a plain
    /// command, which is what most rows are.
    pub(crate) managed:  usize,
    /// Whether another cargo stands between this invocation and the
    /// lead of its group. False for the lead itself and for the
    /// invocations it started directly.
    ///
    /// What the summary keeps out. A command's own cell lists its whole
    /// tree, which is where the tree is worth reading; gathered into
    /// one table with every other command's, the deeper levels bury the
    /// runs they came from -- one `cargo nextest run` puts a `cargo
    /// mend` in the table for every test it runs.
    pub(crate) nested:   bool,
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

    /// The cargo subcommand this invocation names: `port` in
    /// `cargo port`, and in `cargo +nightly port` too, the toolchain
    /// selector being no part of it.
    ///
    /// [`command_text`] puts an external subcommand's own name back at
    /// the front of the arguments, so a command that became
    /// `cargo-port` answers this the same as one still spelled
    /// `cargo port`.
    fn subcommand(&self) -> Option<&str> {
        self.arguments
            .iter()
            .map(String::as_str)
            .find(|argument| !argument.starts_with(CARGO_TOOLCHAIN_SELECTOR))
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
            .is_some_and(|subcommand| hidden_when_idle.iter().any(|hidden| hidden == subcommand))
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
    pub(crate) fn line(&self, manifest: ManifestPath) -> String {
        if manifest == ManifestPath::Shown {
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

/// One process standing above a command in the process tree.
///
/// What a cell lists to say where the command came from: a shell, an
/// editor, the agent or script that typed it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Ancestor {
    /// Process id.
    pub(crate) pid:            u32,
    /// What the process is, as [`describe`] reads it.
    pub(crate) command:        String,
    /// Whether the process passed a command through rather than
    /// starting it, per [`is_transparent`]. Whether a cell draws one of
    /// these is settled where the chain is drawn rather than here: the
    /// exception is the foot of the chain, and a driver's cell closes
    /// its own chain with the driver, which moves where the foot is.
    pub(crate) passes_through: bool,
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
    pub(crate) lead:     CargoProcess,
    /// Everything running under [`lead`](Self::lead), newest first.
    /// Empty for a plain command.
    pub(crate) rest:     Vec<CargoProcess>,
    /// What stands above [`lead`](Self::lead), outermost first, ending
    /// at the process that started it. Empty for a command whose
    /// parents cannot be read.
    pub(crate) ancestry: Vec<Ancestor>,
}

impl CargoGroup {
    /// The group's identity, stable for as long as the command runs.
    pub(crate) const fn id(&self) -> u32 { self.lead.pid }
}

/// One scan's account of the machine: the cargo commands running, and
/// whether an sccache server is up behind them.
///
/// The two travel together because they are read together. Phase one
/// already names every process to find the compilers under each cargo,
/// and a running server is one more name in that same pass -- which is
/// what makes the answer free, and what keeps the summary's stats read
/// from having to start a server to discover whether one is running.
pub(crate) struct Scan {
    /// The commands running, newest first.
    pub(crate) groups:  Vec<CargoGroup>,
    /// Whether a process named [`SCCACHE_BINARY`] was among them.
    pub(crate) sccache: SccacheServer,
}

/// Start the scanner thread and hand back the channel it publishes on.
///
/// The thread ends when the receiver is dropped.
pub(crate) fn spawn() -> Receiver<Scan> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut system = System::new();
        let mut smoothing = CpuSmoothing::default();
        let home = dirs::home_dir();
        loop {
            if sender
                .send(scan(
                    &mut system,
                    &mut smoothing,
                    Instant::now(),
                    home.as_deref(),
                ))
                .is_err()
            {
                return;
            }
            thread::sleep(Duration::from_millis(PROCESS_POLL_MILLIS));
        }
    });
    receiver
}

/// One two-phase scan, newest group first.
///
/// `smoothing` outlives the scan because a CPU share is settled across
/// scans rather than read out of one, and `now` is what tells it when
/// the table is due a fresh reading.
fn scan(
    system: &mut System,
    smoothing: &mut CpuSmoothing,
    now: Instant,
    home: Option<&Path>,
) -> Scan {
    // Phase one: pid, name, parent and start time for everything. None of
    // the fields this asks for require a per-process read of the argument
    // area, which is what makes it cheap enough to poll continuously.
    // `with_cpu` on the cheap pass rather than the targeted one: the
    // work a cargo command is doing runs in the `rustc` and `sccache`
    // processes under it, and phase two never looks at those. sysinfo
    // reads a share as the delta between two refreshes of the same
    // process, so the first scan reports nought and every scan after it
    // reports the `PROCESS_POLL_MILLIS` just gone.
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu(),
    );

    let mut census = Census::take(system);

    // Phase two: the costly fields, for the cargo processes and for the
    // handful standing above each of them. The ancestors are read for
    // the same reason the invocations are -- a cell says what launched
    // the command -- and a chain is a few processes long, against the
    // hundreds this pass still skips.
    let detailed = census.detailed();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&detailed),
        false,
        ProcessRefreshKind::nothing()
            .with_cwd(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_exe(UpdateKind::OnlyIfNotSet),
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

    let attributed = census.attribute(smoothing, now);
    Scan {
        sccache: census.sccache(),
        // Phase one refreshed every process, so whether a registered
        // run is still going is a lookup rather than a fresh read of
        // the process table.
        groups:  census.groups(
            system,
            &attributed,
            home,
            &Capture::take(|pid| system.process(Pid::from_u32(pid)).is_some().into()),
        ),
    }
}

/// What the census worked out per cargo invocation, once every process
/// under one has been walked up to it.
struct Attributed {
    /// The compiler driver each invocation directly owns, and how many
    /// of it. Absent for an invocation compiling nothing.
    compilers: HashMap<Pid, Compiler>,
    /// The settled CPU share each invocation and everything under it
    /// add up to. Absent for an invocation using none.
    cpu:       HashMap<Pid, f32>,
}

/// Each cargo invocation's CPU share as the table reports it, carried
/// between scans.
///
/// Two separate things happen here, and they run at different speeds.
/// One scan's sample is a quarter second of a process's life, which for
/// anything that works in bursts says more about where the sample landed
/// than about what the command is doing, so each invocation's reading is
/// carried part way toward its latest sample rather than replaced by it,
/// over the window [`CPU_SMOOTHING_SECONDS`] names. That happens on every
/// scan. What the table is given, though, is held for
/// [`CPU_REPORT_MILLIS`] at a time: a smooth figure redrawn four times a
/// second is still a figure nobody can read.
#[derive(Default)]
struct CpuSmoothing {
    /// Where each invocation's reading has settled, moved on every scan.
    /// Keyed by the cargo pid, so an invocation that ends takes its
    /// history with it.
    settled:  HashMap<Pid, f32>,
    /// What the table is carrying, taken from
    /// [`settled`](Self::settled) when a reading falls due.
    reported: HashMap<Pid, f32>,
    /// When [`reported`](Self::reported) was last taken, or `None`
    /// before the first scan has taken one.
    taken:    Option<Instant>,
}

impl CpuSmoothing {
    /// Carry every invocation's reading toward what this scan sampled,
    /// and hand back what the table should show at `now`.
    ///
    /// An invocation the scan sampled nothing for is settled toward
    /// nought rather than left where it was: absent from the sample
    /// means it used no CPU, which is a reading like any other. One that
    /// has ended is let go of by both maps.
    fn settle(
        &mut self,
        sampled: &HashMap<Pid, f32>,
        cargo: &[Pid],
        now: Instant,
    ) -> HashMap<Pid, f32> {
        self.settled.retain(|pid, _| cargo.contains(pid));
        self.reported.retain(|pid, _| cargo.contains(pid));
        let alpha = smoothing_alpha();
        for &pid in cargo {
            let sample = sampled.get(&pid).copied().unwrap_or_default();
            // A pid met for the first time opens at its own sample, so a
            // command that starts busy is not drawn climbing to it.
            let settled = self.settled.entry(pid).or_insert(sample);
            *settled = (sample - *settled).mul_add(alpha, *settled);
        }
        if self.is_due(now) {
            self.reported.clone_from(&self.settled);
            self.taken = Some(now);
        } else {
            // An invocation that has only just started has nothing being
            // held for it, and waiting out the rest of somebody else's
            // second would draw it idle. Its opening reading goes
            // straight through.
            for (&pid, &settled) in &self.settled {
                self.reported.entry(pid).or_insert(settled);
            }
        }
        self.reported.clone()
    }

    /// Whether the table is due a fresh reading at `now`.
    fn is_due(&self, now: Instant) -> bool {
        self.taken.is_none_or(|taken| {
            now.duration_since(taken) >= Duration::from_millis(CPU_REPORT_MILLIS)
        })
    }
}

/// How much of a fresh sample a settled reading takes on.
///
/// Worked out from the scan interval rather than stated, so the window
/// stays the one [`CPU_SMOOTHING_SECONDS`] names however often the scan
/// runs.
fn smoothing_alpha() -> f32 {
    let interval = Duration::from_millis(PROCESS_POLL_MILLIS).as_secs_f32();
    1.0 - (-interval / CPU_SMOOTHING_SECONDS).exp()
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
    /// What each process that is using any CPU at all is using, ready to
    /// be attributed to the cargo above it. Processes reading nought are
    /// left out: they are the great majority of a machine, and they add
    /// nothing to the sum.
    cpu:       HashMap<Pid, f32>,
}

impl Census {
    /// Classify every process the last refresh saw.
    fn take(system: &System) -> Self {
        let mut census = Self {
            parents:   HashMap::new(),
            cargo:     Vec::new(),
            compilers: Vec::new(),
            cpu:       HashMap::new(),
        };
        for (&pid, process) in system.processes() {
            if let Some(parent) = process.parent().or_else(|| kernel_parent(pid)) {
                census.parents.insert(pid, parent);
            }
            let cpu = process.cpu_usage();
            if cpu > 0.0 {
                census.cpu.insert(pid, cpu);
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

    /// Whether phase one saw an sccache server.
    ///
    /// The server is a process named [`SCCACHE_BINARY`] like any other,
    /// so it lands in `compilers` whether or not a build is running --
    /// which is what lets this answer while the machine is idle, when a
    /// hit rate is exactly what a developer is looking at.
    fn sccache(&self) -> SccacheServer {
        if self
            .compilers
            .iter()
            .any(|&(_, driver)| driver == SCCACHE_BINARY)
        {
            SccacheServer::Running
        } else {
            SccacheServer::Stopped
        }
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

    /// Everything the census has to say about each cargo invocation.
    ///
    /// The two walks are one call because they run over the same parent
    /// chains and are both spent by the same pass over the groups.
    fn attribute(&self, smoothing: &mut CpuSmoothing, now: Instant) -> Attributed {
        Attributed {
            compilers: self.attribute_compilers(),
            cpu:       smoothing.settle(&self.attribute_cpu(), &self.cargo, now),
        }
    }

    /// Sum what each cargo invocation is using with what everything
    /// under it is using.
    ///
    /// A cargo process spends next to nothing on its own account: the
    /// work is in the `rustc`, `sccache`, build-script and linker
    /// processes it starts, and a row reporting only the cargo's own
    /// share would read as idle right through a build saturating the
    /// machine. Nesting is settled the way [`attribute_compilers`] settles
    /// it -- a process counts against the nearest cargo above it -- so a
    /// managed command's share lands on that command rather than on its
    /// manager, and [`Self::group`] adds the descendants back in for the
    /// lead.
    ///
    /// [`attribute_compilers`]: Self::attribute_compilers
    fn attribute_cpu(&self) -> HashMap<Pid, f32> {
        let mut tallies: HashMap<Pid, f32> = HashMap::new();
        for (&pid, &cpu) in &self.cpu {
            let owner = self
                .cargo
                .contains(&pid)
                .then_some(pid)
                .or_else(|| self.owning_cargo(pid));
            if let Some(owner) = owner {
                *tallies.entry(owner).or_default() += cpu;
            }
        }
        tallies
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

    /// Walk `pid` up its parent chain to the nearest ancestor the cell
    /// actually draws, which is what the `parent` column names.
    ///
    /// Two things are drawn: the cargo invocations, which are the rows
    /// of the table, and the chain block standing over it. The first
    /// ancestor that is one of them is the answer, and it is the only
    /// answer worth writing down -- a pid the screen shows nowhere is a
    /// number the eye cannot pair with anything.
    ///
    /// Which is why the immediate parent is never it. A captured run's
    /// parent is the pty the shim opened, whose parent is the shim,
    /// whose parent is the shell; none of the three is drawn, and the
    /// shell is the first thing above them that is. For an invocation
    /// another cargo started, the same walk stops one step sooner, at
    /// that cargo's own row.
    ///
    /// Bounded by [`PARENT_WALK_LIMIT`], like every other walk here, so
    /// a reparented chain that loops cannot spin.
    fn drawn_parent(&self, pid: Pid, ancestry: &[Ancestor]) -> Option<u32> {
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            let parent = *self.parents.get(&current)?;
            if self.cargo.contains(&parent)
                || ancestry
                    .iter()
                    .any(|ancestor| ancestor.pid == parent.as_u32())
            {
                return Some(parent.as_u32());
            }
            current = parent;
        }
        None
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

    /// Every process a scan reads the costly fields for: the cargo
    /// invocations, and everything standing above one of them.
    ///
    /// A superset of what any cell ends up listing. Which ancestors are
    /// the invocation's own plumbing is settled on argv, which is what
    /// this pass is about to read, so the filtering waits for
    /// [`Self::ancestry`] and the extra reads are a handful of
    /// processes.
    fn detailed(&self) -> Vec<Pid> {
        let mut pids = self.cargo.clone();
        for &pid in &self.cargo {
            for ancestor in self.ancestor_pids(pid) {
                if !pids.contains(&ancestor) {
                    pids.push(ancestor);
                }
            }
        }
        pids
    }

    /// Every process standing above `pid`, outermost first.
    ///
    /// The walk stops short of the init process the whole tree roots
    /// at: everything on the machine descends from it, so a row naming
    /// it tells one command from no other. [`PARENT_WALK_LIMIT`] bounds
    /// it the way it bounds the compiler walk, and a pid already on the
    /// chain ends it outright, so a reparented cycle cannot spin here.
    fn ancestor_pids(&self, pid: Pid) -> Vec<Pid> {
        let mut chain = Vec::new();
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            let Some(&parent) = self.parents.get(&current) else {
                break;
            };
            if parent.as_u32() <= ROOT_PROCESS_PID || chain.contains(&parent) {
                break;
            }
            chain.push(parent);
            current = parent;
        }
        chain.reverse();
        chain
    }

    /// What stands above `pid`, as the command's own cell lists it:
    /// outermost first, each entry a pid and what that process is.
    ///
    /// One kind of process is dropped outright: the wrappers belonging
    /// to the invocation itself. A captured run reaches cargo through a
    /// shim and a pty, both running this same cargo command line, and
    /// listing them would answer "what started this" with the machinery
    /// this tool installed to watch it.
    ///
    /// The shells and login processes that merely passed the command
    /// through are marked rather than dropped -- see
    /// [`Ancestor::passes_through`].
    ///
    /// A cargo ancestor that is *not* plumbing cannot reach here:
    /// [`Self::groups`] leads a group with an invocation that has no
    /// cargo above it.
    fn ancestry(&self, system: &System, home: Option<&Path>, pid: Pid) -> Vec<Ancestor> {
        self.ancestor_pids(pid)
            .into_iter()
            .filter_map(|pid| system.process(pid))
            .filter(|process| !names_cargo(process.cmd()))
            .map(|process| Ancestor {
                pid:            process.pid().as_u32(),
                command:        describe(process, home),
                passes_through: is_transparent(process.name()),
            })
            .collect()
    }

    /// What the capture behind `pid` reports, if one is behind it.
    ///
    /// The walk goes upward because the pid a capture is filed under is
    /// the shim's, and the shim is an ancestor of the cargo it started
    /// rather than the cargo itself -- two levels up when the run went
    /// through a pty, one when it did not. The same bound the compiler
    /// walk uses stops a reparented cycle here.
    fn captured_run(&self, capture: &Capture, pid: Pid) -> Option<RunState> {
        let mut walking = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            if let Some(state) = capture.read(walking.as_u32()) {
                return Some(state);
            }
            walking = *self.parents.get(&walking)?;
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
        attributed: &Attributed,
        home: Option<&Path>,
        capture: &Capture,
    ) -> Vec<CargoGroup> {
        let children = self.cargo_children();
        let mut dated: Vec<(u64, CargoGroup)> = self
            .cargo
            .iter()
            .filter(|&&pid| self.owning_cargo(pid).is_none_or(|owner| owner == pid))
            .filter_map(|&pid| {
                let start = system.process(pid)?.start_time();
                Some((
                    start,
                    self.group(system, attributed, home, &children, capture, pid)?,
                ))
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
        &self,
        system: &System,
        attributed: &Attributed,
        home: Option<&Path>,
        children: &HashMap<Pid, Vec<Pid>>,
        capture: &Capture,
        root: Pid,
    ) -> Option<CargoGroup> {
        let managed = Self::descendants(children, root);
        let whole_group = std::iter::once(root).chain(managed.clone());
        let mut lead = row(
            system.process(root)?,
            root,
            attributed.compilers.get(&root).cloned(),
            managed.len(),
            home,
            aggregate_cpu(&attributed.cpu, whole_group.clone()),
        )?;
        // The lead reports the whole group's compilers: what a developer
        // wants off the summary row is how much work the command they
        // typed is doing, and for a manager none of that work is running
        // under the manager's own pid. Its CPU share is the same story
        // told in cores rather than in processes.
        lead.compiler = aggregate_compilers(&attributed.compilers, whole_group);
        lead.state = self.captured_run(capture, root);
        let ancestry = self.ancestry(system, home, root);
        lead.parent = self.drawn_parent(root, &ancestry);

        let mut dated: Vec<(u64, CargoProcess)> = managed
            .into_iter()
            .filter_map(|pid| {
                let process = system.process(pid)?;
                let under = Self::descendants(children, pid).len();
                let mut managed_row = row(
                    process,
                    pid,
                    attributed.compilers.get(&pid).cloned(),
                    under,
                    home,
                    aggregate_cpu(&attributed.cpu, std::iter::once(pid)),
                )?;
                // The same read the lead gets. An invocation the lead
                // is driving is captured in its own right where it came
                // through the shim, and where it went round the shim --
                // a cargo the enclosing run started, which the shim
                // declines to capture twice -- the enclosing capture is
                // still its own: the lock it prints about is the one
                // this row is waiting on, mirrored into the log of the
                // run it is inside.
                managed_row.state = self.captured_run(capture, pid);
                managed_row.parent = self.drawn_parent(pid, &ancestry);
                managed_row.nested = !children
                    .get(&root)
                    .is_some_and(|started_by_the_lead| started_by_the_lead.contains(&pid));
                Some((process.start_time(), managed_row))
            })
            .collect();
        dated.sort_by(|left, right| right.0.cmp(&left.0).then(right.1.pid.cmp(&left.1.pid)));
        Some(CargoGroup {
            lead,
            rest: dated.into_iter().map(|(_, process)| process).collect(),
            ancestry,
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

/// One CPU share across a whole group: what every member and everything
/// under it add up to.
fn aggregate_cpu(shares: &HashMap<Pid, f32>, members: impl Iterator<Item = Pid>) -> f32 {
    members.filter_map(|pid| shares.get(&pid)).sum()
}

/// Format one cargo process into its table row.
fn row(
    process: &Process,
    pid: Pid,
    compiler: Option<Compiler>,
    managed: usize,
    home: Option<&Path>,
    cpu: f32,
) -> Option<CargoProcess> {
    Some(CargoProcess {
        path: process.cwd().map_or_else(
            || UNRESOLVED_PATH.to_string(),
            |cwd| home_relative(cwd, home),
        ),
        pid: pid.as_u32(),
        parent: None,
        start: start_label(process.start_time()),
        started: process.start_time(),
        duration: duration_label(process.run_time()),
        cpu: cpu_label(cpu),
        compiler,
        state: None,
        managed,
        nested: false,
        command: command_text(process.cmd(), home)?,
    })
}

/// A CPU share as the whole-number percent the table carries.
///
/// Rounded rather than truncated, and never below nought: a quarter of a
/// second of sampling has no meaningful resolution under one percent,
/// and a column of decimals costs width the command line wants.
fn cpu_label(cpu: f32) -> String {
    let percent = cpu.max(0.0);
    format!("{percent:.0}%")
}

/// Whether a process only passed a command through rather than being
/// what launched it, per [`TRANSPARENT_PROCESS_NAMES`].
fn is_transparent(name: &OsStr) -> bool {
    name.to_str()
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
fn describe(process: &Process, home: Option<&Path>) -> String {
    let line: Vec<String> = process
        .cmd()
        .iter()
        .map(|word| home_relative(Path::new(word), home))
        .collect();
    if !line.is_empty() {
        return line.join(" ");
    }
    process.exe().map_or_else(
        || process.name().to_string_lossy().into_owned(),
        |exe| home_relative(exe, home),
    )
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
    use std::fs;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::DEFAULT_HIDDEN_WHEN_IDLE;
    use crate::constants::PID_SEPARATOR;
    use crate::constants::RUN_LOG_PREFIX;
    use crate::constants::RUN_LOG_SUFFIX;
    use crate::constants::SIBLING_SUBCOMMAND_NAME;
    use crate::progress::RunLiveness;

    /// A capture directory holding one live run's log per entry.
    fn capture_root(runs: &[(u32, &str)]) -> TempDir {
        let root = tempdir().expect("temp dir must be created");
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::create_dir_all(&markers).expect("marker dir must be created");
        for (pid, output) in runs {
            let name =
                format!("{RUN_LOG_PREFIX}20260824-084300{PID_SEPARATOR}{pid}{RUN_LOG_SUFFIX}");
            fs::write(root.path().join(name), output).expect("run log must be written");
            fs::write(markers.join(pid.to_string()), "").expect("live marker must be written");
        }
        root
    }

    /// A census that knows nothing but who each process's parent is,
    /// which is all the capture walk reads.
    fn census_of(parents: &[(u32, u32)]) -> Census {
        Census {
            parents:   parents
                .iter()
                .map(|&(child, parent)| (Pid::from_u32(child), Pid::from_u32(parent)))
                .collect(),
            cargo:     Vec::new(),
            compilers: Vec::new(),
            cpu:       HashMap::new(),
        }
    }

    /// A cell names what launched the command, so the walk has to
    /// reach past the shell to whatever started that.
    #[test]
    fn the_chain_above_a_command_reads_outermost_first() {
        // cargo (64432) under a shell (12445) under a login (12444)
        // under the editor that opened it (6218), which launchd owns.
        let census = census_of(&[(64432, 12445), (12445, 12444), (12444, 6218), (6218, 1)]);

        assert_eq!(
            census.ancestor_pids(Pid::from_u32(64432)),
            vec![
                Pid::from_u32(6218),
                Pid::from_u32(12444),
                Pid::from_u32(12445),
            ],
        );
    }

    /// Every command on the machine descends from the init process, so
    /// a row naming it tells one command from no other.
    #[test]
    fn the_walk_stops_short_of_the_process_the_tree_roots_at() {
        let census = census_of(&[(64432, 12445), (12445, 1)]);

        assert_eq!(
            census.ancestor_pids(Pid::from_u32(64432)),
            vec![Pid::from_u32(12445)],
        );
    }

    /// A reparented chain that comes back round on itself must end the
    /// walk rather than spin it.
    #[test]
    fn a_chain_that_loops_ends_where_it_repeats() {
        let census = census_of(&[(64432, 900), (900, 901), (901, 900)]);

        assert_eq!(
            census.ancestor_pids(Pid::from_u32(64432)),
            vec![Pid::from_u32(901), Pid::from_u32(900)],
        );
    }

    /// A shell or login process passed a command through rather than
    /// starting it, and is marked so the cell can decide whether to
    /// draw it.
    #[test]
    fn a_shell_and_a_login_are_marked_as_passing_through() {
        for name in ["zsh", "bash", "sh", "login"] {
            assert!(is_transparent(OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn what_started_a_command_is_never_marked() {
        for name in ["zed", "iTerm2", "node", "cargo-mend"] {
            assert!(!is_transparent(OsStr::new(name)), "{name}");
        }
    }

    /// A nested cargo waits on the build-directory lock in its own
    /// right, and nothing above it can say which invocation is the one
    /// waiting -- so the row has to carry it.
    #[test]
    fn an_invocation_under_the_lead_reports_its_own_wait() {
        // `cargo doc` (76847) under a shim (76846) the lead (64432)
        // started, which is how a manager's nested cargo is captured.
        let root = capture_root(&[(
            76846,
            "    Blocking waiting for file lock on build directory",
        )]);
        let census = census_of(&[(76847, 76846), (76846, 64432)]);
        let capture = Capture::take_from(root.path(), |_| RunLiveness::Running);

        assert_eq!(
            census.captured_run(&capture, Pid::from_u32(76847)),
            Some(RunState::Blocked),
        );
    }

    /// A cargo the enclosing run started has no capture of its own --
    /// the shim declines to open a second one inside a run it is
    /// already capturing. The enclosing one is still its own reading:
    /// the wait it prints about is mirrored into that log because it is
    /// the process doing the waiting.
    #[test]
    fn a_nested_invocation_reads_the_run_it_is_inside() {
        let root = capture_root(&[(
            64431,
            "    Blocking waiting for file lock on build directory",
        )]);
        let census = census_of(&[(76847, 64432), (64432, 64431)]);
        let capture = Capture::take_from(root.path(), |_| RunLiveness::Running);

        assert_eq!(
            census.captured_run(&capture, Pid::from_u32(76847)),
            Some(RunState::Blocked),
        );
        assert_eq!(
            census.captured_run(&capture, Pid::from_u32(64432)),
            Some(RunState::Blocked),
            "the lead still reads its own shim",
        );
    }

    /// The list as it reaches [`CommandText::is_hidden_when_idle`] once
    /// the config has turned it into owned strings.
    fn hidden_when_idle() -> Vec<String> {
        DEFAULT_HIDDEN_WHEN_IDLE
            .iter()
            .map(|subcommand| (*subcommand).to_string())
            .collect()
    }

    #[test]
    fn the_subcommand_is_the_first_argument_past_a_toolchain_selector() {
        assert_eq!(
            CommandText::of(CARGO_DISPLAY_NAME, &["+nightly", "build"]).subcommand(),
            Some("build")
        );
        assert_eq!(
            CommandText::of(CARGO_DISPLAY_NAME, &["build"]).subcommand(),
            Some("build")
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
        let text = command_text(&argv, None).expect("argv names a cargo binary");
        assert_eq!(
            text.line(ManifestPath::Hidden),
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
        let text = command_text(&argv, None).expect("argv names a cargo binary");
        assert_eq!(text.line(ManifestPath::Hidden), "test --no-run");
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
        let text = command_text(&argv, None).expect("argv names a cargo binary");
        assert_eq!(
            text.line(ManifestPath::Hidden),
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
        let text = command_text(&argv, None).expect("argv names a cargo binary");
        assert_eq!(
            text.line(ManifestPath::Shown),
            "build --bin hana --message-format=json"
        );
        assert_eq!(text.line(ManifestPath::Hidden), "build --bin hana");
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

    /// The scale is `top`'s, so a build across several cores reads past
    /// 100% rather than being folded back into a share of the machine.
    #[test]
    fn a_cpu_share_reads_as_a_whole_number_of_percent() {
        assert_eq!(cpu_label(0.0), "0%");
        assert_eq!(cpu_label(12.4), "12%");
        assert_eq!(cpu_label(12.6), "13%");
        assert_eq!(cpu_label(783.2), "783%");
    }

    /// A share the platform could only report as a rounding artefact
    /// still has to read as idle rather than as a negative percent.
    #[test]
    fn a_share_below_nought_reads_as_nought() {
        assert_eq!(cpu_label(-0.4), "0%");
    }

    /// What the lead row carries: its own share and every one under it,
    /// with the pids that used nothing simply absent from the tally.
    #[test]
    fn a_group_adds_up_the_shares_of_everything_under_it() {
        let shares = HashMap::from([(Pid::from(1), 90.4), (Pid::from(2), 300.2)]);
        let members = [1, 2, 3].into_iter().map(Pid::from);

        assert_eq!(cpu_label(aggregate_cpu(&shares, members)), "391%");
    }

    /// A cargo that owns nothing and is doing nothing itself has no
    /// entry to find, which is idle rather than missing.
    #[test]
    fn a_group_with_no_shares_at_all_reads_as_idle() {
        let shares = HashMap::new();

        assert_eq!(
            cpu_label(aggregate_cpu(&shares, std::iter::once(Pid::from(1)))),
            "0%"
        );
    }

    /// A clock the settling tests step forward by hand.
    fn start() -> Instant { Instant::now() }

    /// One scan's worth of the poll interval.
    fn poll() -> Duration { Duration::from_millis(PROCESS_POLL_MILLIS) }

    /// One invocation's reading once `sampled` has been folded in at
    /// `now`.
    fn settle_one(smoothing: &mut CpuSmoothing, sampled: f32, now: Instant) -> f32 {
        let pid = Pid::from(1);
        smoothing
            .settle(&HashMap::from([(pid, sampled)]), &[pid], now)
            .get(&pid)
            .copied()
            .unwrap_or_default()
    }

    /// One invocation settled at `sampled` for `over`, reporting where
    /// the table's reading stood at the end of it.
    fn settle_over(
        smoothing: &mut CpuSmoothing,
        sampled: f32,
        from: Instant,
        over: Duration,
    ) -> f32 {
        let mut elapsed = Duration::ZERO;
        let mut reading = 0.0;
        while elapsed < over {
            elapsed += poll();
            reading = settle_one(smoothing, sampled, from + elapsed);
        }
        reading
    }

    /// A command that starts busy is reported busy rather than drawn
    /// climbing to what its first sample already said.
    #[test]
    fn the_first_sample_of_an_invocation_is_taken_whole() {
        let opening = settle_one(&mut CpuSmoothing::default(), 400.0, start());

        assert!(
            (opening - 400.0).abs() < f32::EPSILON,
            "opened at {opening} rather than at its own sample"
        );
    }

    /// A burst lands as a step toward itself, not as the whole of it:
    /// one scan of a command that works in bursts is mostly artefact.
    #[test]
    fn a_sample_that_jumps_is_taken_a_step_at_a_time() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        settle_one(&mut smoothing, 0.0, now);

        // Past the report interval so what comes back is this scan's
        // settled reading rather than the one being held.
        let stepped = settle_one(
            &mut smoothing,
            100.0,
            now + Duration::from_millis(CPU_REPORT_MILLIS),
        );

        assert!(stepped > 0.0, "the burst moved the reading");
        assert!(stepped < 100.0, "but not the whole way to it: {stepped}");
    }

    /// Held long enough, a steady share is what the column settles on --
    /// the smoothing is a delay, not a ceiling.
    #[test]
    fn a_share_held_steady_is_arrived_at() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        settle_one(&mut smoothing, 0.0, now);

        // Four windows of the climb, by which point a reading settled
        // this way stands within two percent of what it is climbing to.
        let over = Duration::from_secs_f32(CPU_SMOOTHING_SECONDS * 4.0);

        assert_eq!(
            cpu_label(settle_over(&mut smoothing, 100.0, now, over)),
            "98%"
        );
    }

    /// The reading behind the column moves on every scan; the column
    /// itself is only allowed to say something new once a second, so a
    /// smooth figure is not redrawn faster than it can be read.
    #[test]
    fn the_table_holds_a_reading_for_the_whole_report_interval() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        settle_one(&mut smoothing, 0.0, now);
        let held = settle_one(&mut smoothing, 100.0, now + poll());

        assert!(
            held.abs() < f32::EPSILON,
            "the opening reading was still being held, not {held}"
        );

        let refreshed = settle_one(
            &mut smoothing,
            100.0,
            now + Duration::from_millis(CPU_REPORT_MILLIS),
        );

        assert!(refreshed > 0.0, "the second brought the climb through");
    }

    /// A command that starts partway through somebody else's second is
    /// reported straight away rather than drawn idle until it ends.
    #[test]
    fn an_invocation_that_arrives_mid_second_reports_at_once() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        let (running, arriving) = (Pid::from(1), Pid::from(2));
        smoothing.settle(&HashMap::from([(running, 10.0)]), &[running], now);

        let reported = smoothing.settle(
            &HashMap::from([(running, 10.0), (arriving, 400.0)]),
            &[running, arriving],
            now + poll(),
        );

        assert_eq!(reported.get(&arriving).copied(), Some(400.0));
    }

    /// An invocation the scan no longer carries takes its history with
    /// it, so a pid handed out again opens fresh.
    #[test]
    fn an_invocation_that_ends_is_let_go_of() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        settle_one(&mut smoothing, 400.0, now);
        smoothing.settle(&HashMap::new(), &[], now + poll());

        assert!(smoothing.settled.is_empty());
        assert!(smoothing.reported.is_empty());
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
            mend.line(ManifestPath::Shown).contains("--json"),
            "and the long line still carries every one of them",
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
}
