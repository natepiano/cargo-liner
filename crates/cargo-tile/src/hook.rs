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
//! something to do quietly, so nothing here runs unless it is asked for
//! by name. Two properties make that safe to live with: the real binary
//! is only ever moved, never written over or removed, and a `cargo`
//! without [`SHIM_MARKER`] in it is treated as the real one no matter
//! what is beside it -- which is what makes installing twice harmless
//! and repairs the hook after `rustup update` puts a fresh cargo back.

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
use crate::constants::TOOLCHAIN_BIN_DIR;
use crate::constants::TOOLCHAINS_DIR;

/// The shim script, compiled in so the binary carries everything it
/// installs and a copy on disk can never drift from it.
const SHIM_SOURCE: &str = include_str!("cargo-capture-shim.sh");

/// One toolchain's cargo, and whatever stands in front of it.
pub(crate) struct Hook {
    /// The toolchain's name, which is what a report names it by.
    name:  String,
    /// The `cargo` the toolchain resolves, which the shim takes the
    /// place of.
    cargo: PathBuf,
    /// Where the real cargo is kept while the shim holds its name.
    real:  PathBuf,
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
            write_shim(&self.cargo)?;
            return Ok(Change::Refreshed);
        }
        // Whatever holds the name without the marker in it is the real
        // binary -- a first install, or the fresh cargo `rustup update`
        // just put back over the shim. Either way it is the one to keep.
        fs::rename(&self.cargo, &self.real)?;
        write_shim(&self.cargo)?;
        Ok(Change::Installed)
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

/// Write the shim to `path` and make it executable.
fn write_shim(path: &Path) -> io::Result<()> {
    fs::write(path, SHIM_SOURCE)?;
    fs::set_permissions(path, fs::Permissions::from_mode(SHIM_MODE))
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
        assert!(SHIM_SOURCE.contains("script -q -e -c"));
    }

    /// The grid is reachable as `cargo tile`, which puts it in front of
    /// the shim like anything else. Capturing it would run a terminal UI
    /// under `script`, copying every redraw into a log for as long as
    /// the grid stayed open.
    #[test]
    fn the_shim_passes_the_grids_own_subcommand_through() {
        assert!(SHIM_SOURCE.contains(&format!("\n    {SUBCOMMAND_NAME})\n")));
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

    #[test]
    fn a_toolchain_with_no_cargo_in_it_is_not_a_hook() {
        let home = tempdir().unwrap();
        let toolchain = home.path().join(TOOLCHAINS_DIR).join("stable-test");
        fs::create_dir_all(toolchain.join(TOOLCHAIN_BIN_DIR)).unwrap();

        assert!(Hook::at(&toolchain).is_none());
    }
}
