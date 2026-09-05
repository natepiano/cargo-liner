//! Putting the capture shim in front of each toolchain's cargo, and
//! taking it back out.
//!
//! The grid reads a run's progress out of a log, and something has to
//! write that log. Cargo cannot be asked to -- a process's output
//! belongs to the terminal it was started from, and the runs in the grid
//! were started from other terminals. So the shim takes the place of
//! each toolchain's `cargo`, with the real binary kept beside it under
//! [`REAL_CARGO_NAME`], and mirrors what it runs.
//!
//! Standing in front of every cargo invocation on the machine is not
//! something to do quietly. The grid does it when it opens, through
//! [`at_startup`], and says so on screen every time it changes
//! anything; `config.toml` can turn that off, and `cargo tile install`
//! does the same by name. Two properties make it safe to live with: the
//! real binary is only ever moved, never written over or removed, and a
//! `cargo` without [`SHIM_MARKER`] in it is treated as the real one no
//! matter what is beside it -- which is what makes installing twice
//! harmless and repairs the hook after `rustup update` puts a fresh
//! cargo back.
//!
//! Taking the shim out is never automatic. Runs started from other
//! terminals need it whether or not a grid is open.

use std::env;
use std::fs;
use std::io;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;

use crate::constants::CARGO_NAME;
use crate::constants::REAL_CARGO_NAME;
use crate::constants::RUSTUP_DIRNAME;
use crate::constants::RUSTUP_HOME_ENV;
use crate::constants::SHIM_MARKER;
use crate::constants::SHIM_MARKER_SEARCH_BYTES;
use crate::constants::SHIM_MODE;
use crate::constants::SHIM_STAGING_NAME;
use crate::constants::TOOLCHAIN_BIN_DIR;
use crate::constants::TOOLCHAINS_DIR;

/// The shim script, compiled in so the binary carries everything it
/// installs and a copy on disk can never drift from it.
const SHIM_SOURCE: &str = include_str!("cargo-capture-shim.sh");

/// One toolchain's cargo, and whatever stands in front of it.
pub(crate) struct Hook {
    /// The toolchain's name, which is what a report names it by.
    name:    String,
    /// The `cargo` the toolchain resolves, which the shim takes the
    /// place of.
    cargo:   PathBuf,
    /// Where the real cargo is kept while the shim holds its name.
    real:    PathBuf,
    /// Where a shim is written before it is renamed over `cargo`.
    staging: PathBuf,
}

/// What the grid found, and did, standing the shim up as it opened.
///
/// Every list holds toolchain names, sorted, so the notice built from
/// it reads the same twice running. A toolchain whose shim was already
/// current is in none of them: it is the ordinary case, and there is
/// nothing to say about it.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct Startup {
    /// Toolchains the shim was put in front of just now.
    pub(crate) installed: Vec<String>,
    /// Toolchains whose shim was out of date -- written by an earlier
    /// cargo-tile -- and now carries this binary's copy.
    pub(crate) refreshed: Vec<String>,
    /// Toolchains whose shim has no real cargo beside it. Left alone:
    /// installing over that would write a shim in front of nothing, and
    /// the only repair is `rustup` putting a cargo back.
    pub(crate) orphaned:  Vec<String>,
    /// Toolchains where installing failed, and the error's text. A
    /// read-only toolchain directory is the usual reason.
    pub(crate) failed:    Vec<(String, String)>,
}

impl Startup {
    /// Whether anything happened that the user should hear about.
    pub(crate) const fn is_quiet(&self) -> bool {
        self.installed.is_empty()
            && self.refreshed.is_empty()
            && self.orphaned.is_empty()
            && self.failed.is_empty()
    }
}

/// Stand the shim up in front of every toolchain that lacks one, and
/// bring every installed shim up to date with this binary's copy.
///
/// The one state left alone is [`HookState::Orphaned`], which is
/// reported rather than repaired. A toolchain that fails does not stop
/// the others: each is its own file system operation, and one refusing
/// says nothing about the next.
pub(crate) fn at_startup() -> io::Result<Startup> { Ok(stand_up(&Hook::all()?)) }

/// [`at_startup`] over a known set of hooks, which is what the tests
/// hand in.
fn stand_up(hooks: &[Hook]) -> Startup {
    let mut startup = Startup::default();
    for hook in hooks {
        match hook.ensure() {
            Ok(Some(Change::Installed)) => startup.installed.push(hook.name.clone()),
            Ok(Some(Change::Refreshed)) => startup.refreshed.push(hook.name.clone()),
            Ok(Some(Change::Orphaned)) => startup.orphaned.push(hook.name.clone()),
            Ok(Some(Change::Removed | Change::AlreadyAbsent) | None) => {},
            Err(error) => startup.failed.push((hook.name.clone(), error.to_string())),
        }
    }
    startup
}

/// What is standing in front of a toolchain's cargo right now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookState {
    /// The shim holds `cargo` and the real binary is beside it. Runs on
    /// this toolchain are captured.
    Installed,
    /// A real cargo holds its own name. Nothing is captured.
    Absent,
    /// The shim holds `cargo` but the real binary beside it is gone, so
    /// every invocation fails. Only reachable by deleting the real
    /// binary by hand.
    Orphaned,
}

/// What installing or removing did to one toolchain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Change {
    /// The shim now stands where a real cargo did.
    Installed,
    /// The shim was already there and was rewritten from the copy in
    /// this binary.
    Refreshed,
    /// The real cargo has its name back.
    Removed,
    /// Nothing to do: no shim was installed.
    AlreadyAbsent,
    /// Nothing done: the shim is there with no real cargo beside it,
    /// which is not a state installing can repair.
    Orphaned,
}

impl Hook {
    /// Every toolchain rustup has installed that has a cargo in it.
    ///
    /// Sorted by name so a report reads the same twice running.
    pub(crate) fn all() -> io::Result<Vec<Self>> {
        let mut hooks: Vec<Self> = fs::read_dir(rustup_home()?.join(TOOLCHAINS_DIR))?
            .flatten()
            .filter_map(|entry| Self::at(&entry.path()))
            .collect();
        hooks.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(hooks)
    }

    /// The hook for one toolchain directory, or `None` where there is no
    /// cargo to stand in front of.
    fn at(toolchain: &Path) -> Option<Self> {
        let binaries = toolchain.join(TOOLCHAIN_BIN_DIR);
        let cargo = binaries.join(CARGO_NAME);
        if !cargo.exists() {
            return None;
        }
        Some(Self {
            name: toolchain.file_name()?.to_str()?.to_owned(),
            cargo,
            real: binaries.join(REAL_CARGO_NAME),
            staging: binaries.join(SHIM_STAGING_NAME),
        })
    }

    /// The toolchain this hook belongs to.
    pub(crate) fn name(&self) -> &str { &self.name }

    /// What is standing in front of this toolchain's cargo.
    pub(crate) fn state(&self) -> HookState {
        if !is_shim(&self.cargo) {
            return HookState::Absent;
        }
        if self.real.exists() {
            HookState::Installed
        } else {
            HookState::Orphaned
        }
    }

    /// Put the shim in front of this toolchain's cargo.
    ///
    /// Writing the shim is the last step, so a failure part way through
    /// leaves the real cargo reachable under one name or the other
    /// rather than leaving the toolchain with no cargo at all.
    pub(crate) fn install(&self) -> io::Result<Change> {
        if self.state() == HookState::Installed {
            self.write_shim()?;
            return Ok(Change::Refreshed);
        }
        // Whatever holds the name without the marker in it is the real
        // binary -- a first install, or the fresh cargo `rustup update`
        // just put back over the shim. Either way it is the one to keep.
        fs::rename(&self.cargo, &self.real)?;
        self.write_shim()?;
        Ok(Change::Installed)
    }

    /// [`install`](Self::install) as the grid runs it on every launch:
    /// a missing shim goes in, a stale one is brought up to date, a
    /// current one is left untouched, and an orphaned one is reported.
    ///
    /// `None` is the ordinary case -- the shim is there and current --
    /// and the one that must cost nothing, because it is what every
    /// launch after the first finds.
    fn ensure(&self) -> io::Result<Option<Change>> {
        match self.state() {
            HookState::Absent => self.install().map(Some),
            HookState::Orphaned => Ok(Some(Change::Orphaned)),
            HookState::Installed => {
                if fs::read_to_string(&self.cargo)? == SHIM_SOURCE {
                    return Ok(None);
                }
                self.write_shim()?;
                Ok(Some(Change::Refreshed))
            },
        }
    }

    /// Write the shim as `cargo`, executable, without ever writing the
    /// file already there in place.
    ///
    /// A shim that is mid-run is still being read by its `sh`, which
    /// waits on the real cargo rather than `exec`ing it. Writing over it
    /// would hand that `sh` a half-written script. So the shim goes in
    /// beside it and is renamed across: the running `sh` keeps the inode
    /// it opened, and the name changes hands in one step.
    fn write_shim(&self) -> io::Result<()> {
        fs::write(&self.staging, SHIM_SOURCE)?;
        fs::set_permissions(&self.staging, fs::Permissions::from_mode(SHIM_MODE))?;
        fs::rename(&self.staging, &self.cargo)
    }

    /// Give the real cargo its name back.
    pub(crate) fn remove(&self) -> io::Result<Change> {
        match self.state() {
            HookState::Absent => Ok(Change::AlreadyAbsent),
            HookState::Orphaned => Err(io::Error::other(format!(
                "{}: the shim is installed but the real cargo is missing from {}",
                self.name,
                self.real.display()
            ))),
            HookState::Installed => {
                fs::rename(&self.real, &self.cargo)?;
                Ok(Change::Removed)
            },
        }
    }
}

/// Where rustup keeps its toolchains.
fn rustup_home() -> io::Result<PathBuf> {
    if let Some(home) = env::var_os(RUSTUP_HOME_ENV) {
        return Ok(PathBuf::from(home));
    }
    dirs::home_dir()
        .map(|home| home.join(RUSTUP_DIRNAME))
        .ok_or_else(|| io::Error::other("no home directory to find rustup under"))
}

/// Whether the file at `path` is a shim this installer wrote.
///
/// Reads only the opening of the file: the marker is in the shim's first
/// comment, and the alternative is a thirty-megabyte binary.
fn is_shim(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut opening = vec![0; SHIM_MARKER_SEARCH_BYTES];
    let Ok(read) = file.read(&mut opening) else {
        return false;
    };
    opening.truncate(read);
    String::from_utf8_lossy(&opening).contains(SHIM_MARKER)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::constants::LOCK_WAIT_MARKER;
    use crate::constants::SIBLING_SUBCOMMAND_NAME;
    use crate::constants::SUBCOMMAND_NAME;

    /// A rustup home holding one toolchain whose cargo is `contents`.
    fn toolchain(contents: &str) -> (TempDir, Hook) {
        let home = tempdir().unwrap();
        let binaries = home
            .path()
            .join(TOOLCHAINS_DIR)
            .join("stable-test")
            .join(TOOLCHAIN_BIN_DIR);
        fs::create_dir_all(&binaries).unwrap();
        fs::write(binaries.join(CARGO_NAME), contents).unwrap();
        let hook = Hook::at(&home.path().join(TOOLCHAINS_DIR).join("stable-test")).unwrap();
        (home, hook)
    }

    #[test]
    fn the_shim_carries_the_marker_the_installer_looks_for() {
        assert!(SHIM_SOURCE.contains(SHIM_MARKER));
    }

    /// The shim runs in front of every cargo invocation on two operating
    /// systems, where the only shell that can be counted on is `sh`.
    #[test]
    fn the_shim_is_posix_sh_and_calls_both_script_implementations() {
        assert!(SHIM_SOURCE.starts_with("#!/bin/sh\n"));
        assert!(SHIM_SOURCE.contains("util-linux"));
        // util-linux exits with its own status without `-e`, which would
        // report every failed build as a success.
        assert!(SHIM_SOURCE.contains("script -q -e -f -c"));
        // Neither implementation flushes the log per write on its own,
        // and the BSD one holds output for thirty seconds at a time --
        // longer than many runs last, and long enough that the single
        // line a blocked run prints reaches the grid after the wait it
        // announced is over.
        assert!(SHIM_SOURCE.contains("script -q -t 0"));
    }

    /// A log is read only while the run writing it is alive, so the run
    /// takes it away as it goes. Unconditionally: what a finished log
    /// happens to hold is no longer a question the shim asks, which is
    /// what freed it from spelling out the reader's markers to answer.
    #[test]
    fn the_shim_retires_its_log_when_its_run_ends() {
        assert!(SHIM_SOURCE.contains(r#"rm -f "$log""#));
        assert!(
            !SHIM_SOURCE.contains(LOCK_WAIT_MARKER),
            "and no longer carries a copy of a marker the reader owns"
        );
    }

    /// Whether the shim's exemption arm names this subcommand. The arm
    /// lists several, so what matters is that this one is among them
    /// rather than what the whole line reads.
    fn shim_passes_through(subcommand: &str) -> bool {
        SHIM_SOURCE.lines().any(|line| {
            line.strip_prefix("    ")
                .and_then(|arm| arm.strip_suffix(')'))
                .is_some_and(|arm| arm.split('|').any(|name| name.trim() == subcommand))
        })
    }

    /// The grid is reachable as `cargo tile`, which puts it in front of
    /// the shim like anything else. Capturing it would run a terminal UI
    /// under `script`, copying every redraw into a log for as long as
    /// the grid stayed open.
    #[test]
    fn the_shim_passes_the_grids_own_subcommand_through() {
        assert!(shim_passes_through(SUBCOMMAND_NAME));
    }

    #[test]
    fn the_shim_passes_the_sibling_terminal_ui_through() {
        assert!(shim_passes_through(SIBLING_SUBCOMMAND_NAME));
    }

    #[test]
    fn a_real_cargo_reads_as_no_shim_installed() {
        let (_home, hook) = toolchain("\u{7f}ELF not a script at all");

        assert_eq!(hook.state(), HookState::Absent);
    }

    #[test]
    fn installing_moves_the_real_cargo_aside_and_keeps_every_byte_of_it() {
        let real = "\u{7f}ELF the one and only real cargo";
        let (_home, hook) = toolchain(real);

        assert_eq!(hook.install().unwrap(), Change::Installed);
        assert_eq!(hook.state(), HookState::Installed);
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), real);
        assert!(
            fs::read_to_string(&hook.cargo)
                .unwrap()
                .contains(SHIM_MARKER)
        );
    }

    #[test]
    fn the_installed_shim_is_executable() {
        let (_home, hook) = toolchain("real");
        hook.install().unwrap();

        let mode = fs::metadata(&hook.cargo).unwrap().permissions().mode();

        assert_eq!(mode & 0o777, SHIM_MODE);
    }

    /// Installing twice must not push the shim itself into the place the
    /// real cargo is kept, which would lose the real binary.
    #[test]
    fn installing_twice_rewrites_the_shim_and_leaves_the_real_cargo_alone() {
        let real = "\u{7f}ELF the one and only real cargo";
        let (_home, hook) = toolchain(real);
        hook.install().unwrap();

        assert_eq!(hook.install().unwrap(), Change::Refreshed);
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), real);
    }

    /// What `rustup update` leaves behind: a fresh real cargo back on the
    /// name, with the previous real binary still beside it.
    #[test]
    fn a_toolchain_update_that_overwrote_the_shim_is_repaired_by_installing() {
        let (_home, hook) = toolchain("\u{7f}ELF old cargo");
        hook.install().unwrap();
        fs::write(&hook.cargo, "\u{7f}ELF cargo as rustup just reinstalled it").unwrap();

        assert_eq!(hook.state(), HookState::Absent);
        assert_eq!(hook.install().unwrap(), Change::Installed);
        assert_eq!(
            fs::read_to_string(&hook.real).unwrap(),
            "\u{7f}ELF cargo as rustup just reinstalled it"
        );
    }

    #[test]
    fn removing_gives_the_real_cargo_its_name_back() {
        let real = "\u{7f}ELF the one and only real cargo";
        let (_home, hook) = toolchain(real);
        hook.install().unwrap();

        assert_eq!(hook.remove().unwrap(), Change::Removed);
        assert_eq!(hook.state(), HookState::Absent);
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), real);
        assert!(!hook.real.exists());
    }

    #[test]
    fn removing_what_was_never_installed_changes_nothing() {
        let (_home, hook) = toolchain("\u{7f}ELF real cargo");

        assert_eq!(hook.remove().unwrap(), Change::AlreadyAbsent);
        assert_eq!(
            fs::read_to_string(&hook.cargo).unwrap(),
            "\u{7f}ELF real cargo"
        );
    }

    #[test]
    fn a_shim_with_no_real_cargo_beside_it_reports_orphaned_and_refuses_removal() {
        let (_home, hook) = toolchain("\u{7f}ELF real cargo");
        hook.install().unwrap();
        fs::remove_file(&hook.real).unwrap();

        assert_eq!(hook.state(), HookState::Orphaned);
        assert!(hook.remove().is_err());
    }

    /// The shim is renamed into place, never written there, so a `sh`
    /// that is part way through reading it keeps the file it opened.
    #[test]
    fn writing_the_shim_leaves_nothing_staged_beside_it() {
        let (_home, hook) = toolchain("\u{7f}ELF real cargo");
        hook.install().unwrap();
        hook.install().unwrap();

        assert!(!hook.staging.exists());
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), SHIM_SOURCE);
    }

    /// A rustup home holding one toolchain per entry, each with a cargo
    /// holding `contents`, in the order [`Hook::all`] would list them.
    fn toolchains(entries: &[(&str, &str)]) -> (TempDir, Vec<Hook>) {
        let home = tempdir().unwrap();
        for (name, contents) in entries {
            let binaries = home
                .path()
                .join(TOOLCHAINS_DIR)
                .join(name)
                .join(TOOLCHAIN_BIN_DIR);
            fs::create_dir_all(&binaries).unwrap();
            fs::write(binaries.join(CARGO_NAME), contents).unwrap();
        }
        let mut hooks: Vec<Hook> = entries
            .iter()
            .map(|(name, _)| Hook::at(&home.path().join(TOOLCHAINS_DIR).join(name)).unwrap())
            .collect();
        hooks.sort_by(|left, right| left.name.cmp(&right.name));
        (home, hooks)
    }

    #[test]
    fn startup_installs_where_nothing_is_and_says_which_toolchains() {
        let (_home, hooks) = toolchains(&[
            ("stable-test", "\u{7f}ELF stable"),
            ("nightly-test", "\u{7f}ELF nightly"),
        ]);

        let startup = stand_up(&hooks);

        assert_eq!(
            startup,
            Startup {
                installed: vec!["nightly-test".to_owned(), "stable-test".to_owned()],
                ..Startup::default()
            }
        );
        assert!(
            hooks
                .iter()
                .all(|hook| hook.state() == HookState::Installed)
        );
    }

    /// Every launch after the first finds this, and it must change
    /// nothing and say nothing.
    #[test]
    fn startup_over_a_current_shim_is_quiet() {
        let (_home, hooks) = toolchains(&[("stable-test", "\u{7f}ELF stable")]);
        stand_up(&hooks);
        let written = fs::metadata(&hooks[0].cargo).unwrap().modified().unwrap();

        let startup = stand_up(&hooks);

        assert!(startup.is_quiet());
        assert_eq!(
            fs::metadata(&hooks[0].cargo).unwrap().modified().unwrap(),
            written
        );
    }

    /// A shim an earlier cargo-tile wrote still carries the marker, so
    /// it is installed rather than absent -- and out of date.
    #[test]
    fn startup_brings_a_stale_shim_up_to_this_binarys_copy() {
        let (_home, hooks) = toolchains(&[("stable-test", "\u{7f}ELF stable")]);
        stand_up(&hooks);
        fs::write(
            &hooks[0].cargo,
            format!("#!/bin/sh\n# {SHIM_MARKER} from an earlier version\n"),
        )
        .unwrap();

        let startup = stand_up(&hooks);

        assert_eq!(startup.refreshed, vec!["stable-test".to_owned()]);
        assert_eq!(fs::read_to_string(&hooks[0].cargo).unwrap(), SHIM_SOURCE);
        assert_eq!(
            fs::read_to_string(&hooks[0].real).unwrap(),
            "\u{7f}ELF stable"
        );
    }

    #[test]
    fn startup_reports_an_orphaned_shim_and_leaves_it_alone() {
        let (_home, hooks) = toolchains(&[("stable-test", "\u{7f}ELF stable")]);
        stand_up(&hooks);
        fs::remove_file(&hooks[0].real).unwrap();

        let startup = stand_up(&hooks);

        assert_eq!(startup.orphaned, vec!["stable-test".to_owned()]);
        assert_eq!(hooks[0].state(), HookState::Orphaned);
        assert!(!hooks[0].real.exists());
    }

    /// One toolchain refusing must not stop the shim going in front of
    /// the others.
    #[test]
    fn startup_carries_on_past_a_toolchain_that_cannot_be_written() {
        let (_home, hooks) = toolchains(&[
            ("nightly-test", "\u{7f}ELF nightly"),
            ("stable-test", "\u{7f}ELF stable"),
        ]);
        let locked = hooks[0].cargo.parent().unwrap().to_path_buf();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

        let startup = stand_up(&hooks);

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(startup.installed, vec!["stable-test".to_owned()]);
        assert_eq!(startup.failed.len(), 1);
        assert_eq!(startup.failed[0].0, "nightly-test");
        assert_eq!(hooks[0].state(), HookState::Absent);
        assert_eq!(
            fs::read_to_string(&hooks[0].cargo).unwrap(),
            "\u{7f}ELF nightly"
        );
    }

    #[test]
    fn a_toolchain_with_no_cargo_in_it_is_not_a_hook() {
        let home = tempdir().unwrap();
        let toolchain = home.path().join(TOOLCHAINS_DIR).join("stable-test");
        fs::create_dir_all(toolchain.join(TOOLCHAIN_BIN_DIR)).unwrap();

        assert!(Hook::at(&toolchain).is_none());
    }
}
