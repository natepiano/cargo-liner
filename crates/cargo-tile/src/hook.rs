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
use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Write as _;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Error;
use std::io::ErrorKind;
use std::io::IsTerminal;
use std::io::PipeReader;
use std::io::Read;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
#[cfg(not(test))]
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Output;
use std::process::Stdio;
use std::sync::Arc;
#[cfg(test)]
use std::sync::LazyLock;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;

use chrono::DateTime;
use chrono::Utc;
use sysinfo::ProcessesToUpdate;
use tempfile::TempDir;

use crate::birth_stamp;
use crate::birth_stamp::KernelObservation;
use crate::birth_stamp::Observation;
use crate::constants::ACCOUNT_EXECUTABLE_MODE;
use crate::constants::ACCOUNT_EXECUTABLE_PREFIX;
use crate::constants::ACCOUNT_GROUPS_GROWTH_FACTOR;
use crate::constants::ACCOUNT_GROUPS_INITIAL_CAPACITY;
use crate::constants::ACCOUNT_GROUPS_MAX_CAPACITY;
use crate::constants::ACCOUNT_HOOK_REPORT_FLAG;
use crate::constants::BINARY_NAME;
use crate::constants::CAPTURE_RUNNER_NAME;
use crate::constants::CAPTURE_SHIM_ARGUMENT;
use crate::constants::CAPTURE_SHIM_EXECUTABLE;
use crate::constants::CAPTURE_SHIM_PASSTHROUGH_ARGUMENT;
use crate::constants::CAPTURE_SHIM_VERSION;
use crate::constants::CAPTURE_STDERR_PUMP_BUFFER_BYTES;
use crate::constants::CARGO_NAME;
use crate::constants::COMMAND_VALUED_OPTIONS;
use crate::constants::REAL_CARGO_NAME;
use crate::constants::RUSTUP_DIRNAME;
use crate::constants::RUSTUP_HOME_ENV;
use crate::constants::SHIM_LOCK_NAME;
use crate::constants::SHIM_LOCK_RECOVERY;
use crate::constants::SHIM_LOCK_RETRY_ATTEMPTS;
use crate::constants::SHIM_LOCK_RETRY_DELAY;
use crate::constants::SHIM_MARKER;
use crate::constants::SHIM_MARKER_SEARCH_BYTES;
use crate::constants::SHIM_MODE;
use crate::constants::SHIM_STAGING_NAME;
use crate::constants::SHIM_VERSION_PREFIX;
use crate::constants::TOOLCHAIN_BIN_DIR;
use crate::constants::TOOLCHAINS_DIR;
#[cfg(test)]
use crate::shim_registration::native_fixture;

/// The shim script, compiled in so the binary carries everything it
/// installs and a copy on disk can never drift from it.
#[cfg(test)]
const SHIM_SOURCE: &str = include_str!("cargo-capture-shim.sh");
/// The shim script, compiled in so the binary carries everything it
/// installs and a copy on disk can never drift from it.
#[cfg(test)]
pub(crate) fn shim_source() -> &'static str {
    static SOURCE: LazyLock<String> = LazyLock::new(|| {
        let executable = fixture_runner();
        shim_source_for(&executable)
    });
    SOURCE.as_str()
}

/// Keep one named native runner for all capture fixtures in this test process.
#[cfg(test)]
pub(crate) fn fixture_runner() -> PathBuf { tests::fixture_runner() }

/// Render a UTF-8 fixture entry; production publishes raw shell bytes.
#[cfg(test)]
pub(crate) fn shim_source_for(executable: &Path) -> String { tests::utf8_shell_source(executable) }

/// The executable entry preserves every Unix path byte through shell parsing.
fn capture_shell_source(executable: &Path) -> Vec<u8> {
    let source = include_str!("cargo-capture-shim.sh")
        .replace("@CAPTURE_SHIM_ARGUMENT@", CAPTURE_SHIM_ARGUMENT)
        .replace(
            "@CAPTURE_SHIM_PASSTHROUGH_ARGUMENT@",
            CAPTURE_SHIM_PASSTHROUGH_ARGUMENT,
        );
    let quoted = capture_shell_word(executable.as_os_str());
    let mut bytes = Vec::new();
    for (index, part) in source.split(CAPTURE_SHIM_EXECUTABLE).enumerate() {
        if index > 0 {
            bytes.extend_from_slice(&quoted);
        }
        bytes.extend_from_slice(part.as_bytes());
    }
    bytes
}

/// Enter capture before CLI parsing or worker threads start.
/// Ordinary cargo-tile calls do not inspect or alter capture state.
#[cfg(not(test))]
pub(crate) fn capture_shim_entry() {
    let mut arguments = env::args_os().skip(1);
    let mode = match arguments.next().as_deref() {
        Some(argument) if argument == CAPTURE_SHIM_ARGUMENT => CaptureEntry::Captured,
        Some(argument) if argument == CAPTURE_SHIM_PASSTHROUGH_ARGUMENT => {
            CaptureEntry::Passthrough
        },
        _ => return,
    };
    let (Some(shim), Some(parent)) = (arguments.next(), arguments.next()) else {
        std::process::exit(127);
    };
    let result = NativeCargoCall::new(Path::new(&shim), PathBuf::from(parent), arguments.collect())
        .and_then(|call| match mode {
            CaptureEntry::Captured => match env::var_os("CARGO_TILE_PTY_START_SOCKET") {
                Some(socket) => call.exec_with_acknowledgement(Path::new(&socket)),
                None => call.run(),
            },
            CaptureEntry::Passthrough => Err(call.command().exec()),
        });
    std::process::exit(result.unwrap_or(127));
}

/// The shell records stream availability before Rust initializes standard descriptors.
#[cfg(not(test))]
enum CaptureEntry {
    Captured,
    Passthrough,
}

/// Quote one Unix word without decoding its path or argument bytes.
fn capture_shell_word(word: &OsStr) -> Vec<u8> {
    let mut quoted = vec![b'\''];
    for byte in word.as_bytes() {
        if *byte == b'\'' {
            quoted.extend_from_slice(b"'\\''");
        } else {
            quoted.push(*byte);
        }
    }
    quoted.push(b'\'');
    quoted
}

/// One resolved toolchain call, before it acquires any capture artifacts.
struct NativeCargoCall {
    real:      PathBuf,
    shim:      PathBuf,
    parent:    PathBuf,
    arguments: Vec<OsString>,
}

impl NativeCargoCall {
    /// Resolve symlinks without starting readlink, dirname, or an identity utility.
    fn new(shim: &Path, parent: PathBuf, arguments: Vec<OsString>) -> io::Result<Self> {
        let shim = fs::canonicalize(shim)?;
        let directory = shim
            .parent()
            .ok_or_else(|| io::Error::other("shim has no directory"))?;
        Ok(Self {
            real: directory.join(REAL_CARGO_NAME),
            shim,
            parent,
            arguments,
        })
    }

    /// Every path, including capture failure, retains cargo's arguments and status.
    #[cfg(not(test))]
    fn run(self) -> io::Result<i32> {
        let cargo = self.command();
        self.run_with_publication(cargo, CapturePublication::prepare, || Ok(()))
    }

    /// EOF on this close-on-exec connection confirms that exec accepted cargo.
    #[cfg(not(test))]
    fn exec_with_acknowledgement(self, socket: &Path) -> io::Result<i32> {
        let mut acknowledgement = UnixStream::connect(socket)?;
        acknowledgement.write_all(b"exec")?;
        let error = self
            .command()
            .env_remove("CARGO_TILE_PTY_START_SOCKET")
            .exec();
        let _ = acknowledgement.write_all(b"failed");
        Err(error)
    }

    fn command(&self) -> Command {
        let mut cargo = Command::new(&self.real);
        cargo.args(&self.arguments).env("CARGO", &self.shim);
        cargo
    }

    /// The publication boundary supplies either owned artifacts or unchanged cargo.
    fn run_with_publication(
        self,
        mut cargo: Command,
        publish: impl FnOnce(&Path, &[OsString]) -> io::Result<CapturePublication>,
        after_readiness: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<i32> {
        if !capture_requested(&self.arguments) || capture_is_nested() {
            return Err(cargo.exec());
        }
        let Ok(terminal) = capture_terminal() else {
            return Err(cargo.exec());
        };
        let Ok(signals) = DeferredCaptureSignals::new() else {
            return Err(cargo.exec());
        };
        let Ok(publication) = publish(&self.parent, &self.arguments) else {
            if let CaptureInterruption::Signaled(status) = signals.before_cargo() {
                return Ok(status);
            }
            return Err(cargo.exec());
        };
        if let CaptureInterruption::Signaled(status) = signals.before_cargo() {
            return Ok(status);
        }
        let completion = match CaptureStartup::start(
            &cargo,
            terminal,
            &self.arguments,
            &publication.log.0,
            &signals,
            after_readiness,
        ) {
            CaptureStartup::Interrupted(status) => {
                publication.finish();
                return Ok(status);
            },
            CaptureStartup::CargoStarted(running) => CargoCompletion::Finished(running.wait()?),
            CaptureStartup::HelperStarted(helper) => helper.wait()?,
            CaptureStartup::PlainCargo => CargoCompletion::NotStarted,
        };
        let status = match completion {
            CargoCompletion::Finished(status) => status,
            CargoCompletion::NotStarted => {
                publication.finish();
                return Err(cargo.exec());
            },
        };
        publication.finish();
        Ok(signals.exit_status(status))
    }
}

/// Query commands have no compilation output to capture.
fn capture_requested(arguments: &[OsString]) -> bool {
    let mut words = arguments.iter();
    while let Some(word) = words.next() {
        if COMMAND_VALUED_OPTIONS
            .iter()
            .any(|(option, scope)| scope.accepted_before_subcommand() && word == OsStr::new(option))
        {
            words.next();
            continue;
        }
        let bytes = word.as_bytes();
        if matches!(bytes, b"-V" | b"--version" | b"-vV" | b"--list") {
            return false;
        }
        if bytes.starts_with(b"-") || bytes.starts_with(b"+") {
            continue;
        }
        return !matches!(
            bytes,
            b"metadata"
                | b"pkgid"
                | b"locate-project"
                | b"read-manifest"
                | b"config"
                | b"tile"
                | b"port"
                | b"handler"
                | b"berth"
        );
    }
    false
}

/// Resolve an inherited enclosing pid through kernel parent links, without ps.
fn capture_is_nested() -> bool {
    let Ok(enclosing) = env::var("CARGOTILE_NESTED")
        .unwrap_or_default()
        .parse::<u32>()
    else {
        return false;
    };
    let Some(parent) = rustix::process::getppid() else {
        return false;
    };
    let mut parent = sysinfo::Pid::from_u32(parent.as_raw_nonzero().get().cast_unsigned());
    let mut system = sysinfo::System::new();
    for _ in 0..32 {
        if parent.as_u32() == enclosing {
            return true;
        }
        if parent.as_u32() <= 1 {
            break;
        }
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[parent]),
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        let Some(next) = system.process(parent).and_then(sysinfo::Process::parent) else {
            break;
        };
        parent = next;
    }
    false
}

/// Capture may borrow streams only when the caller supplied them.
#[derive(Clone, Copy)]
enum CaptureTerminal {
    Pty,
    Piped,
}

fn capture_terminal() -> io::Result<CaptureTerminal> {
    let script_available =
        env::split_paths(&env::var_os("PATH").unwrap_or_default()).any(|directory| {
            fs::metadata(directory.join("script"))
                .is_ok_and(|file| file.is_file() && file.permissions().mode() & 0o111 != 0)
        });
    if script_available
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal()
    {
        return Ok(CaptureTerminal::Pty);
    }
    let descriptor_open = |descriptor| fs::metadata(format!("/dev/fd/{descriptor}")).is_ok();
    if !descriptor_open(1)
        || !descriptor_open(2)
        || (3..=9)
            .filter(|descriptor| !descriptor_open(*descriptor))
            .count()
            < 2
    {
        Err(io::Error::other(
            "caller streams or spare descriptors unavailable",
        ))
    } else {
        Ok(CaptureTerminal::Piped)
    }
}

/// Only successful exclusive creation grants cleanup ownership of an artifact.
struct CapturePublication {
    registration: OwnedCaptureFile,
    log:          OwnedCaptureFile,
}

/// Convert a native observation to registration fields, retaining boot without birth.
pub(crate) fn capture_identity_fields(pid: u32, identity: KernelObservation) -> (String, String) {
    match identity.into_capture_observation(pid) {
        Observation::Present(stamp) => {
            let (boot, birth) = stamp.registration_fields();
            (boot.to_owned(), birth.to_string())
        },
        Observation::Ended | Observation::Unknown => {
            (birth_stamp::capture_boot().to_owned(), String::new())
        },
    }
}

/// Exclusive creation or publication supplies the only cleanup authority.
struct OwnedCaptureFile(PathBuf);

impl Drop for OwnedCaptureFile {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}

impl CapturePublication {
    /// Remove the registration before its log after cargo and its output have ended.
    fn finish(self) {
        drop(self.registration);
        drop(self.log);
    }

    /// Identity is sampled once and reused for setup and registration publication.
    fn prepare(parent: &Path, arguments: &[OsString]) -> io::Result<Self> {
        Self::prepare_at_publication(parent, arguments, |_, _| Ok(()))
    }

    /// Tests can supply a filesystem change at the exact staging/publication boundary.
    fn prepare_at_publication(
        parent: &Path,
        arguments: &[OsString],
        boundary: impl FnOnce(&Path, &Path) -> io::Result<()>,
    ) -> io::Result<Self> {
        let pid = std::process::id();
        let (boot, birth) = capture_identity_fields(pid, birth_stamp::observe(pid));
        let directories = CaptureDirectories::prepare(parent, &boot)?;
        directories.remove_dead_publications()?;
        let generation = capture_generation_at(chrono::Utc::now());
        Self::publish(
            &directories,
            &generation,
            &boot,
            &birth,
            arguments,
            boundary,
        )
    }

    fn publish(
        directories: &CaptureDirectories,
        generation: &str,
        boot: &str,
        birth: &str,
        arguments: &[OsString],
        boundary: impl FnOnce(&Path, &Path) -> io::Result<()>,
    ) -> io::Result<Self> {
        let pid = std::process::id();
        let name = format!("run-{generation}-{pid}.log");
        let registration = directories.pids.join(format!("{pid}.{generation}"));
        let log = directories.root.join(&name);
        let staging = registration.with_extension(format!("{generation}.tmp"));
        let bytes = capture_registration(generation, boot, birth, &name, arguments)?;
        let mut staged = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&staging)?;
        let result = (|| {
            staged.write_all(&bytes)?;
            staged.set_permissions(fs::Permissions::from_mode(0o644))?;
            boundary(&staging, &registration)?;
            fs::hard_link(&staging, &registration)
        })();
        let _ = fs::remove_file(&staging);
        result?;
        // Registration precedes the log, so a reader sweep cannot remove an orphan log.
        let registration = OwnedCaptureFile(registration);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&log)?;
        let publication = Self {
            registration,
            log: OwnedCaptureFile(log),
        };
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
        Ok(publication)
    }
}

/// Supplied wire fields exercise publication failures without a product fault flag.
#[cfg(test)]
pub(crate) struct CaptureTestPublication<'a> {
    pub(crate) boot:            &'a str,
    pub(crate) birth:           &'a str,
    pub(crate) generation:      &'a str,
    pub(crate) environment:     &'a [(&'a str, &'a OsStr)],
    pub(crate) after_readiness: fn() -> io::Result<()>,
}

#[cfg(test)]
impl CaptureTestPublication<'_> {
    /// Run the production call and filesystem publication with controlled boundary work.
    fn run(
        self,
        shim: &Path,
        parent: PathBuf,
        arguments: Vec<OsString>,
        before: impl FnOnce(&Path, &Path) -> io::Result<()>,
        after: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<i32> {
        self.run_for_account(
            shim,
            parent,
            arguments,
            rustix::process::getuid().as_raw(),
            before,
            after,
        )
    }

    /// Supply an account identity to test refusal of directories owned by another uid.
    pub(crate) fn run_for_account(
        self,
        shim: &Path,
        parent: PathBuf,
        arguments: Vec<OsString>,
        uid: u32,
        before: impl FnOnce(&Path, &Path) -> io::Result<()>,
        after: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<i32> {
        let call = NativeCargoCall::new(shim, parent, arguments)?;
        let mut cargo = call.command();
        for (key, value) in self.environment {
            cargo.env(key, value);
        }
        call.run_with_publication(
            cargo,
            |parent, arguments| {
                let directories = CaptureDirectories::prepare_for_account(parent, self.boot, uid)?;
                directories.remove_dead_publications()?;
                let publication = CapturePublication::publish(
                    &directories,
                    self.generation,
                    self.boot,
                    self.birth,
                    arguments,
                    before,
                )?;
                after()?;
                Ok(publication)
            },
            self.after_readiness,
        )
    }
}

/// A supplied calendar reading cannot make two calls share a publication name.
pub(crate) fn capture_generation_at(now: DateTime<Utc>) -> String {
    format!("{}-{}", now.format("%Y%m%d-%H%M%S"), uuid::Uuid::now_v7())
}

/// Checked account directories and their once-per-boot setup receipt.
struct CaptureDirectories {
    root: PathBuf,
    pids: PathBuf,
}

impl CaptureDirectories {
    fn prepare(parent: &Path, boot: &str) -> io::Result<Self> {
        Self::prepare_for_account(parent, boot, rustix::process::getuid().as_raw())
    }

    fn prepare_for_account(parent: &Path, boot: &str, uid: u32) -> io::Result<Self> {
        capture_directory(parent, CaptureDirectoryOwner::Shared)?;
        let root = parent.join(uid.to_string());
        let state = root.join("state");
        let pids = state.join("pids");
        let mut receipt = boot.to_owned();
        let parent_metadata = fs::symlink_metadata(parent)?;
        let _ = write!(
            receipt,
            ":{}:{}",
            parent_metadata.dev(),
            parent_metadata.ino()
        );
        for path in [&root, &state, &pids] {
            capture_directory(path, CaptureDirectoryOwner::Account(uid))?;
            let metadata = fs::symlink_metadata(path)?;
            let _ = write!(receipt, ":{}:{}", metadata.dev(), metadata.ino());
        }
        let cache = parent.join(format!(".setup-v4-{uid}"));
        let current = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&cache)
            .and_then(|file| {
                let metadata = file.metadata()?;
                if metadata.uid() != uid || !metadata.is_file() {
                    return Err(io::Error::other("foreign setup receipt"));
                }
                let mut value = String::new();
                file.take(4096).read_to_string(&mut value)?;
                Ok(value)
            });
        if current.as_deref().ok() != Some(receipt.as_str()) {
            if fs::symlink_metadata(parent)?.uid() == uid {
                fs::set_permissions(parent, fs::Permissions::from_mode(0o1777))?;
            }
            for path in [&root, &state, &pids] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
            }
            let mut cache = OpenOptions::new()
                .write(true)
                .create(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .mode(0o600)
                .open(cache)?;
            let metadata = cache.metadata()?;
            if metadata.uid() != uid || !metadata.is_file() {
                return Err(io::Error::other("foreign setup receipt"));
            }
            cache.set_len(0)?;
            cache.write_all(receipt.as_bytes())?;
        }
        Ok(Self { root, pids })
    }

    /// A killed writer cannot clean itself up; its next account call removes exact names.
    fn remove_dead_publications(&self) -> io::Result<()> {
        for entry in fs::read_dir(&self.pids)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some((pid, generation)) = name.trim_end_matches(".tmp").split_once('.') else {
                continue;
            };
            let Ok(pid) = pid.parse::<i32>() else {
                continue;
            };
            let Some(subject) = rustix::process::Pid::from_raw(pid) else {
                continue;
            };
            if pid.cast_unsigned() != std::process::id()
                && rustix::process::test_kill_process(subject) != Err(rustix::io::Errno::SRCH)
            {
                continue;
            }
            let _ = fs::remove_file(self.root.join(format!("run-{generation}-{pid}.log")));
            let _ = fs::remove_file(self.pids.join(format!("{pid}.{generation}")));
            let _ = fs::remove_file(self.pids.join(format!("{pid}.{generation}.tmp")));
        }
        Ok(())
    }
}

/// Refuse symlinks and foreign account ownership before any permission change.
#[derive(Clone, Copy)]
enum CaptureDirectoryOwner {
    Shared,
    Account(u32),
}

fn capture_directory(path: &Path, owner: CaptureDirectoryOwner) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => {},
        Err(error) if error.kind() == ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {},
            Err(error) => return Err(error),
        },
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || matches!(owner, CaptureDirectoryOwner::Account(uid) if metadata.uid() != uid)
    {
        return Err(io::Error::other(
            "capture directory has an invalid owner or type",
        ));
    }
    Ok(())
}

/// Keep the v3 framing, raw argv bytes, physical cwd and absolute-home rule.
fn capture_registration(
    generation: &str,
    boot: &str,
    birth: &str,
    log: &str,
    arguments: &[OsString],
) -> io::Result<Vec<u8>> {
    let directory = env::current_dir()?;
    let home = env::var_os("HOME")
        .filter(|home| Path::new(home).is_absolute())
        .unwrap_or_default();
    let count = arguments.len().to_string();
    let mut bytes = Vec::new();
    for field in [
        OsStr::new("cargo-tile-v3"),
        OsStr::new(generation),
        OsStr::new(boot),
        OsStr::new(birth),
        OsStr::new(log),
        directory.as_os_str(),
        &home,
        OsStr::new(&count),
    ]
    .into_iter()
    .chain(arguments.iter().map(std::ffi::OsString::as_os_str))
    {
        bytes.extend_from_slice(field.as_bytes());
        bytes.push(0);
    }
    Ok(bytes)
}

/// A completed signal sample either permits cargo or supplies the deferred exit status.
enum CaptureInterruption {
    Uninterrupted,
    Signaled(i32),
}

/// A shim-only signal waits for cargo and for the last mirrored byte.
struct DeferredCaptureSignals {
    received: Arc<AtomicUsize>,
}

impl DeferredCaptureSignals {
    fn new() -> io::Result<Self> {
        let received = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for signal in [
            signal_hook::consts::SIGHUP,
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGQUIT,
        ] {
            signal_hook::flag::register_usize(
                signal,
                std::sync::Arc::clone(&received),
                signal.cast_unsigned() as usize,
            )?;
        }
        Ok(Self { received })
    }
    fn before_cargo(&self) -> CaptureInterruption {
        let signal = self.received.load(Ordering::Relaxed);
        if signal == 0 {
            CaptureInterruption::Uninterrupted
        } else {
            CaptureInterruption::Signaled(128 + i32::try_from(signal).unwrap_or(0))
        }
    }
    fn exit_status(&self, status: ExitStatus) -> i32 {
        let signal = self.received.load(Ordering::Relaxed);
        if signal != 0 {
            return 128 + i32::try_from(signal).unwrap_or(0);
        }
        status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
    }
}

/// Startup failure permits passthrough only while cargo has not started.
enum CaptureStartup {
    PlainCargo,
    Interrupted(i32),
    CargoStarted(RunningCapture),
    HelperStarted(PtyCaptureHelper),
}

impl CaptureStartup {
    fn start(
        cargo: &Command,
        terminal: CaptureTerminal,
        arguments: &[OsString],
        log: &Path,
        signals: &DeferredCaptureSignals,
        after_readiness: impl FnOnce() -> io::Result<()>,
    ) -> Self {
        let captured = capture_child_command(cargo);
        match terminal {
            CaptureTerminal::Piped => {
                capture_piped(captured, arguments, log, signals, after_readiness)
                    .unwrap_or(Self::PlainCargo)
            },
            CaptureTerminal::Pty => {
                capture_pty(&captured, log).map_or(Self::PlainCargo, Self::HelperStarted)
            },
        }
    }
}

/// The original command retains its arguments, environment and inherited streams for fallback.
fn capture_child_command(original: &Command) -> Command {
    let mut cargo = Command::new(original.get_program());
    cargo.args(original.get_args());
    copy_command_context(original, &mut cargo);
    cargo.env("CARGOTILE_NESTED", std::process::id().to_string());
    cargo
}

/// Preserve explicit environment values, removals, and the working directory.
fn copy_command_context(original: &Command, child: &mut Command) {
    for (key, value) in original.get_envs() {
        if let Some(value) = value {
            child.env(key, value);
        } else {
            child.env_remove(key);
        }
    }
    if let Some(directory) = original.get_current_dir() {
        child.current_dir(directory);
    }
}

/// Once cargo starts, wait errors cannot cause a second invocation.
struct RunningCapture {
    cargo:  Child,
    mirror: NativeStderrPump,
}

impl RunningCapture {
    fn wait(mut self) -> io::Result<ExitStatus> {
        let status = self.cargo.wait();
        let mirrored = self.mirror.wait();
        let status = status?;
        mirrored?;
        Ok(status)
    }
}

/// Machine-readable stdout is untouched; only JSON cargo's quiet flag is removed.
fn capture_piped(
    mut cargo: Command,
    arguments: &[OsString],
    log: &Path,
    signals: &DeferredCaptureSignals,
    after_readiness: impl FnOnce() -> io::Result<()>,
) -> io::Result<CaptureStartup> {
    let json = arguments
        .iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg.as_bytes().starts_with(b"--message-format=json"))
        || arguments
            .windows(2)
            .take_while(|pair| pair[0] != "--")
            .any(|pair| pair[0] == "--message-format" && pair[1].as_bytes().starts_with(b"json"));
    if json {
        let mut filtered = Command::new(cargo.get_program());
        copy_command_context(&cargo, &mut filtered);
        let mut passthrough = false;
        for argument in arguments {
            if argument == "--" {
                passthrough = true;
            }
            if !passthrough && (argument == "--quiet" || argument == "-q") {
                continue;
            }
            filtered.arg(argument);
        }
        cargo = filtered;
    }
    cargo.env(
        "CARGO_TERM_PROGRESS_WHEN",
        env::var_os("CARGO_TERM_PROGRESS_WHEN")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "always".into()),
    );
    cargo.env(
        "CARGO_TERM_PROGRESS_WIDTH",
        env::var_os("CARGO_TERM_PROGRESS_WIDTH")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "100".into()),
    );
    let (reader, writer) = std::io::pipe()?;
    let mirror = NativeStderrPump::start(reader, log)?;
    if let Err(error) = after_readiness() {
        drop(writer);
        let _ = mirror.wait();
        return Err(error);
    }
    if let CaptureInterruption::Signaled(status) = signals.before_cargo() {
        drop(writer);
        let _ = mirror.wait();
        return Ok(CaptureStartup::Interrupted(status));
    }
    let result = cargo.stderr(Stdio::from(writer)).spawn();
    drop(cargo);
    match result {
        Ok(cargo) => Ok(CaptureStartup::CargoStarted(RunningCapture {
            cargo,
            mirror,
        })),
        Err(error) => {
            let _ = mirror.wait();
            Err(error)
        },
    }
}

/// An open log and caller stream belong to a started pump before cargo can write.
struct NativeStderrPump {
    worker: JoinHandle<()>,
}

impl NativeStderrPump {
    fn start(reader: PipeReader, log: &Path) -> io::Result<Self> {
        let log = OpenOptions::new().append(true).open(log)?;
        let stderr = File::from(io::stderr().as_fd().try_clone_to_owned()?);
        let worker = thread::Builder::new()
            .name("cargo-stderr".into())
            .spawn(move || Self::drain(reader, log, stderr))?;
        Ok(Self { worker })
    }

    /// A failed log file or caller stderr cannot close cargo's pipe.
    /// The other output still receives bytes.
    fn drain(mut reader: PipeReader, mut log: File, mut stderr: File) {
        let mut bytes = [0; CAPTURE_STDERR_PUMP_BUFFER_BYTES];
        loop {
            match reader.read(&mut bytes) {
                Ok(0) => break,
                Ok(length) => {
                    let _ = log.write_all(&bytes[..length]);
                    let _ = stderr.write_all(&bytes[..length]);
                },
                Err(error) if error.kind() == ErrorKind::Interrupted => {},
                Err(_) => break,
            }
        }
    }

    fn wait(self) -> io::Result<()> {
        self.worker
            .join()
            .map_err(|_| io::Error::other("stderr pump failed"))
    }
}

/// A helper's exit is separate from an acknowledged cargo execution.
enum CargoCompletion {
    NotStarted,
    Finished(ExitStatus),
}

/// The script child cannot establish that cargo started without its exec acknowledgement.
struct PtyCaptureHelper {
    child:    Child,
    listener: UnixListener,
    socket:   OwnedCaptureFile,
}

impl PtyCaptureHelper {
    fn wait(mut self) -> io::Result<CargoCompletion> {
        let status = self.child.wait()?;
        let completion = match self.listener.accept() {
            Ok((mut stream, _)) => {
                let mut acknowledgement = Vec::new();
                stream.read_to_end(&mut acknowledgement)?;
                if acknowledgement == b"exec" {
                    CargoCompletion::Finished(status)
                } else {
                    CargoCompletion::NotStarted
                }
            },
            Err(error) if error.kind() == ErrorKind::WouldBlock => CargoCompletion::NotStarted,
            Err(error) => return Err(error),
        };
        drop(self.socket);
        Ok(completion)
    }
}

/// script remains the only helper process; the native child execs cargo in its own pid.
fn capture_pty(cargo: &Command, log: &Path) -> io::Result<PtyCaptureHelper> {
    let (_, random) = uuid::Uuid::now_v7().as_u64_pair();
    let socket_path = env::temp_dir().join(format!("tile-start-{random:016x}"));
    let listener = UnixListener::bind(&socket_path)?;
    let socket = OwnedCaptureFile(socket_path);
    fs::set_permissions(&socket.0, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    #[cfg(not(test))]
    let runner = env::current_exe()?;
    #[cfg(test)]
    let runner = native_fixture::executable()?.to_owned();
    let mut acknowledged = pty_cargo_command(cargo, log, &runner);
    acknowledged.env("CARGO_TILE_PTY_START_SOCKET", &socket.0);
    #[cfg(target_os = "linux")]
    let implementation = CaptureScript::UtilLinux;
    #[cfg(target_os = "macos")]
    let implementation = CaptureScript::Bsd;
    let child = implementation.command(&acknowledged, log).spawn()?;
    Ok(PtyCaptureHelper {
        child,
        listener,
        socket,
    })
}

/// Reuse the native transport; its socket entry execs cargo without a second publication.
fn pty_cargo_command(cargo: &Command, log: &Path, runner: &Path) -> Command {
    let mut acknowledged = Command::new(runner);
    acknowledged
        .arg(CAPTURE_SHIM_ARGUMENT)
        .arg(Path::new(cargo.get_program()).with_file_name(CARGO_NAME))
        .arg(log.parent().unwrap_or_else(|| Path::new(".")))
        .args(cargo.get_args());
    copy_command_context(cargo, &mut acknowledged);
    acknowledged
}

/// Each platform's command preserves cargo's exit status and flushes its log.
enum CaptureScript {
    #[cfg(any(target_os = "linux", test))]
    UtilLinux,
    #[cfg(any(target_os = "macos", test))]
    Bsd,
}

impl CaptureScript {
    fn command(self, cargo: &Command, log: &Path) -> Command {
        let mut script = Command::new("script");
        match self {
            #[cfg(any(target_os = "linux", test))]
            Self::UtilLinux => {
                let mut command = b"exec ".to_vec();
                for (index, word) in std::iter::once(cargo.get_program())
                    .chain(cargo.get_args())
                    .enumerate()
                {
                    if index > 0 {
                        command.push(b' ');
                    }
                    command.extend_from_slice(&capture_shell_word(word));
                }
                script
                    .args(["-q", "-e", "-f", "-c"])
                    .arg(OsStr::from_bytes(&command))
                    .arg(log);
            },
            #[cfg(any(target_os = "macos", test))]
            Self::Bsd => {
                script
                    .args(["-q", "-t", "0"])
                    .arg(log)
                    .arg(cargo.get_program())
                    .args(cargo.get_args());
            },
        }
        copy_command_context(cargo, &mut script);
        script
    }
}

/// The account database identity whose default rustup home is inspected.
#[derive(Debug)]
pub(crate) struct HookAccount {
    /// The system database's display name.
    pub(crate) name: String,
    /// User identity under which the account child runs.
    pub(crate) uid:  u32,
    /// Primary group under which the account child runs.
    pub(crate) gid:  u32,
    /// The home recorded in the account database, independent of HOME.
    pub(crate) home: PathBuf,
}

/// The operation performed by each account child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookOperation {
    /// Install or refresh capture shims.
    Install,
    /// Restore the saved cargo binaries.
    Uninstall,
    /// Inspect hook state without changing toolchains or capture storage.
    Status,
}

impl HookOperation {
    /// The CLI subcommand passed to the account child.
    pub(crate) const fn subcommand(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Uninstall => "uninstall",
            Self::Status => "status",
        }
    }

    /// Whether every requested removal or inspection completed.
    /// Installation failures remain diagnostics for runner job-start hooks.
    pub(crate) fn completion(self, reports: &[AccountHookReport]) -> io::Result<()> {
        let incomplete = reports
            .iter()
            .filter(|report| matches!(report.outcome, AccountHookOutcome::Incomplete(_)))
            .count();
        if self != Self::Install && incomplete > 0 {
            return Err(io::Error::other(format!(
                "{} incomplete for {incomplete} account(s)",
                self.subcommand()
            )));
        }
        Ok(())
    }
}

/// Whether handling an account's toolchains completed.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AccountHookOutcome {
    /// Discovery completed and found no cargo binaries.
    NoToolchains,
    /// Every discovered toolchain was handled for the requested operation.
    Completed,
    /// Discovery, credentials, child execution, or a toolchain operation failed.
    Incomplete(String),
}

/// The operation's result for one toolchain, including retained inspection errors.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ToolchainHookOutcome {
    /// Installation's effect on the shim.
    Install(HookOperationOutcome),
    /// Removal's effect on the shim.
    Uninstall(HookOperationOutcome),
    /// The observed hook state.
    Status(HookState),
    /// Inspection failed before a hook state could be established.
    Unreadable(String),
    /// The requested operation failed with this reason.
    Failed(String),
}

impl ToolchainHookOutcome {
    /// Encode the child protocol without allowing names or errors to create extra rows.
    fn protocol(&self) -> String {
        match self {
            Self::Install(outcome) | Self::Uninstall(outcome) => match outcome {
                HookOperationOutcome::Installed => "installed",
                HookOperationOutcome::Refreshed => "refreshed",
                HookOperationOutcome::AlreadyCurrent => "already installed",
                HookOperationOutcome::Removed => "removed",
                HookOperationOutcome::AlreadyAbsent => "absent",
                HookOperationOutcome::Orphaned => "orphaned",
                HookOperationOutcome::DowngradeRefused {
                    installed,
                    supported,
                } => {
                    return format!("downgrade refused\t{installed}\t{supported}");
                },
            }
            .to_owned(),
            Self::Status(state) => match state {
                HookState::Installed => "installed",
                HookState::Absent => "absent",
                HookState::Repairable => "repairable",
                HookState::Orphaned => "orphaned",
            }
            .to_owned(),
            Self::Unreadable(reason) => format!("unreadable\t{}", protocol_field(reason)),
            Self::Failed(reason) => format!("error\t{}", protocol_field(reason)),
        }
    }

    /// Failed inspection and unrepaired mutation outcomes leave handling incomplete.
    pub(crate) const fn is_incomplete(&self) -> bool {
        matches!(
            self,
            Self::Unreadable(_)
                | Self::Failed(_)
                | Self::Install(
                    HookOperationOutcome::Orphaned | HookOperationOutcome::DowngradeRefused { .. }
                )
                | Self::Uninstall(
                    HookOperationOutcome::Orphaned | HookOperationOutcome::DowngradeRefused { .. }
                )
        )
    }
}

/// One named toolchain's result, retained independently of its account summary.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ToolchainHookReport {
    /// The toolchain name from the child's report.
    pub(crate) toolchain: String,
    /// The child's reported operation outcome or inspection failure.
    pub(crate) outcome:   ToolchainHookOutcome,
}

impl ToolchainHookReport {
    /// Decode one line using the operation requested from this child.
    fn parse(operation: HookOperation, line: &str) -> Result<Self, String> {
        let (toolchain, result) = line
            .split_once('\t')
            .filter(|(toolchain, _)| !toolchain.is_empty())
            .ok_or_else(|| format!("invalid hook report: {line}"))?;
        let outcome = match (operation, result) {
            (HookOperation::Install, "installed") => {
                ToolchainHookOutcome::Install(HookOperationOutcome::Installed)
            },
            (HookOperation::Install, "refreshed") => {
                ToolchainHookOutcome::Install(HookOperationOutcome::Refreshed)
            },
            (HookOperation::Install, "already installed") => {
                ToolchainHookOutcome::Install(HookOperationOutcome::AlreadyCurrent)
            },
            (HookOperation::Install, "orphaned") => {
                ToolchainHookOutcome::Install(HookOperationOutcome::Orphaned)
            },
            (HookOperation::Uninstall, "removed") => {
                ToolchainHookOutcome::Uninstall(HookOperationOutcome::Removed)
            },
            (HookOperation::Uninstall, "absent") => {
                ToolchainHookOutcome::Uninstall(HookOperationOutcome::AlreadyAbsent)
            },
            (HookOperation::Uninstall, "orphaned") => {
                ToolchainHookOutcome::Uninstall(HookOperationOutcome::Orphaned)
            },
            (HookOperation::Status, "installed") => {
                ToolchainHookOutcome::Status(HookState::Installed)
            },
            (HookOperation::Status, "absent") => ToolchainHookOutcome::Status(HookState::Absent),
            (HookOperation::Status, "repairable") => {
                ToolchainHookOutcome::Status(HookState::Repairable)
            },
            (HookOperation::Status, "orphaned") => {
                ToolchainHookOutcome::Status(HookState::Orphaned)
            },
            (_, result) => match result.split_once('\t') {
                Some(("downgrade refused", versions)) if operation == HookOperation::Install => {
                    versions
                        .split_once('\t')
                        .and_then(|(installed, supported)| {
                            let installed = installed.parse::<u64>().ok()?;
                            let supported = supported.parse::<u64>().ok()?;
                            (installed > supported).then_some(
                                HookOperationOutcome::DowngradeRefused {
                                    installed,
                                    supported,
                                },
                            )
                        })
                        .map_or_else(
                            || ToolchainHookOutcome::Failed(format!("invalid hook report: {line}")),
                            ToolchainHookOutcome::Install,
                        )
                },
                Some(("unreadable", reason)) => ToolchainHookOutcome::Unreadable(reason.to_owned()),
                Some(("error", reason)) => ToolchainHookOutcome::Failed(reason.to_owned()),
                _ => ToolchainHookOutcome::Failed(format!("invalid hook report: {line}")),
            },
        };
        Ok(Self {
            toolchain: toolchain.to_owned(),
            outcome,
        })
    }

    /// One line in the protocol shared by all account children.
    pub(crate) fn protocol(&self) -> String {
        format!(
            "{}\t{}",
            protocol_field(&self.toolchain),
            self.outcome.protocol()
        )
    }
}

impl Display for ToolchainHookReport {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: ", self.toolchain)?;
        match &self.outcome {
            ToolchainHookOutcome::Install(HookOperationOutcome::DowngradeRefused {
                installed,
                supported,
            })
            | ToolchainHookOutcome::Uninstall(HookOperationOutcome::DowngradeRefused {
                installed,
                supported,
            }) => write!(
                formatter,
                "downgrade refused -- newer shim v{installed} kept; this reader supports v{supported}; upgrade and restart the reader"
            ),
            ToolchainHookOutcome::Install(HookOperationOutcome::Orphaned)
            | ToolchainHookOutcome::Uninstall(HookOperationOutcome::Orphaned)
            | ToolchainHookOutcome::Status(HookState::Orphaned) => formatter
                .write_str("orphaned -- the shim is installed but the real cargo is missing"),
            ToolchainHookOutcome::Unreadable(reason) => write!(formatter, "unreadable: {reason}"),
            ToolchainHookOutcome::Failed(reason) => write!(formatter, "error: {reason}"),
            outcome => formatter.write_str(&outcome.protocol()),
        }
    }
}

/// One account's toolchain reports and completion state.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct AccountHookReport {
    /// The account name used for this report.
    pub(crate) account:    String,
    /// The operation requested from the account child.
    pub(crate) operation:  HookOperation,
    /// Completion across this account's toolchains and child process.
    pub(crate) outcome:    AccountHookOutcome,
    /// Every named toolchain result, in the order reported by the child.
    pub(crate) toolchains: Vec<ToolchainHookReport>,
}

impl AccountHookReport {
    /// Record a failure that prevented the child from reporting any toolchains.
    fn incomplete(account: &HookAccount, operation: HookOperation, reason: String) -> Self {
        Self {
            account: account.name.clone(),
            operation,
            outcome: AccountHookOutcome::Incomplete(reason),
            toolchains: Vec::new(),
        }
    }

    /// Retain every valid toolchain report despite malformed output or a failed exit.
    fn from_output(account: &HookAccount, operation: HookOperation, output: &Output) -> Self {
        let mut toolchains = Vec::new();
        let mut failures = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            match ToolchainHookReport::parse(operation, line) {
                Ok(report) => {
                    if report.outcome.is_incomplete() {
                        failures.push(report.to_string());
                    }
                    toolchains.push(report);
                },
                Err(error) => failures.push(error),
            }
        }
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr.trim();
            failures.push(if reason.is_empty() {
                format!("account child exited with {}", output.status)
            } else {
                reason.to_owned()
            });
        }
        let outcome = if !failures.is_empty() {
            AccountHookOutcome::Incomplete(failures.join("; "))
        } else if toolchains.is_empty() {
            AccountHookOutcome::NoToolchains
        } else {
            AccountHookOutcome::Completed
        };
        Self {
            account: account.name.clone(),
            operation,
            outcome,
            toolchains,
        }
    }
}

impl Display for AccountHookReport {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {}: ",
            self.account,
            self.operation.subcommand()
        )?;
        match &self.outcome {
            AccountHookOutcome::NoToolchains => formatter.write_str("no toolchains")?,
            AccountHookOutcome::Completed => formatter.write_str("completed")?,
            AccountHookOutcome::Incomplete(reason) => write!(formatter, "incomplete: {reason}")?,
        }
        for report in &self.toolchains {
            write!(formatter, "\n{}: {report}", self.account)?;
        }
        Ok(())
    }
}

/// Keep child-protocol fields on a single tab-separated line.
fn protocol_field(value: &str) -> String { value.replace(['\t', '\r', '\n'], " ") }

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
    pub(crate) installed:  Vec<String>,
    /// Toolchains whose shim was out of date -- written by an earlier
    /// cargo-tile -- and now carries this binary's copy.
    pub(crate) refreshed:  Vec<String>,
    /// Newer installed shims kept intact because this reader is older.
    pub(crate) kept_newer: Vec<NewerShim>,
    /// Toolchains whose shim has no real cargo beside it. Left alone:
    /// installing over that would write a shim in front of nothing, and
    /// the only repair is `rustup` putting a cargo back.
    pub(crate) orphaned:   Vec<String>,
    /// Toolchains where installing failed, and the error's text. A
    /// read-only toolchain directory is the usual reason.
    pub(crate) failed:     Vec<(String, String)>,
}

impl Startup {
    /// Whether anything happened that the user should hear about.
    pub(crate) const fn is_quiet(&self) -> bool {
        self.installed.is_empty()
            && self.refreshed.is_empty()
            && self.kept_newer.is_empty()
            && self.orphaned.is_empty()
            && self.failed.is_empty()
    }
}

/// One installed shim that startup cannot replace with this older reader's copy.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct NewerShim {
    /// The toolchain whose shim remains installed.
    pub(crate) toolchain: String,
    /// The version declared by the installed shim.
    pub(crate) installed: u64,
    /// The newest framing version this reader supports.
    pub(crate) supported: u64,
}

#[cfg(test)]
enum StartupSource {
    Discover,
    Injected(Startup),
}

#[cfg(test)]
thread_local! {
    static STARTUP_SOURCE: std::cell::RefCell<StartupSource> =
        const { std::cell::RefCell::new(StartupSource::Discover) };
}

/// Supply one startup result on this thread, restoring discovery even after a panic.
#[cfg(test)]
pub(crate) fn with_startup_for_test(startup: Startup, run: impl FnOnce()) {
    struct RestoreStartup(StartupSource);

    impl Drop for RestoreStartup {
        fn drop(&mut self) {
            STARTUP_SOURCE.set(std::mem::replace(&mut self.0, StartupSource::Discover));
        }
    }

    let _restore = RestoreStartup(STARTUP_SOURCE.replace(StartupSource::Injected(startup)));
    run();
}

/// Stand the shim up in front of every toolchain that lacks one, and
/// bring every installed shim up to date with this binary's copy.
///
/// Orphaned and newer shims are reported and kept unchanged.
/// A toolchain that fails does not stop
/// the others: each is its own file system operation, and one refusing
/// says nothing about the next.
pub(crate) fn at_startup() -> io::Result<Startup> {
    #[cfg(test)]
    if let StartupSource::Injected(startup) = STARTUP_SOURCE.replace(StartupSource::Discover) {
        return Ok(startup);
    }

    let mut hooks = Vec::new();
    let mut failed = Vec::new();
    for discovery in Hook::all()? {
        match discovery {
            HookDiscovery::Discovered(hook) => hooks.push(hook),
            HookDiscovery::AbsentCargo => {},
            HookDiscovery::InspectionFailed { path, error } => {
                failed.push((path.display().to_string(), error.to_string()));
            },
        }
    }
    let mut startup = stand_up(&hooks);
    startup.failed.extend(failed);
    Ok(startup)
}

/// [`at_startup`] over a known set of hooks, which is what the tests
/// hand in.
fn stand_up(hooks: &[Hook]) -> Startup {
    let mut startup = Startup::default();
    for hook in hooks {
        match hook.ensure() {
            Ok(HookOperationOutcome::Installed) => startup.installed.push(hook.name.clone()),
            Ok(HookOperationOutcome::Refreshed) => startup.refreshed.push(hook.name.clone()),
            Ok(HookOperationOutcome::Orphaned) => startup.orphaned.push(hook.name.clone()),
            Ok(HookOperationOutcome::DowngradeRefused {
                installed,
                supported,
            }) => {
                startup.kept_newer.push(NewerShim {
                    toolchain: hook.name.clone(),
                    installed,
                    supported,
                });
            },
            Ok(
                HookOperationOutcome::Removed
                | HookOperationOutcome::AlreadyAbsent
                | HookOperationOutcome::AlreadyCurrent,
            ) => {},
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
    /// Installation moved the real binary aside but did not write the
    /// shim. Restoring the saved binary repairs the missing `cargo`.
    Repairable,
    /// The shim holds `cargo` but the real binary beside it is gone, so
    /// every invocation fails. Only reachable by deleting the real
    /// binary by hand.
    Orphaned,
}

/// What installing or removing did to one toolchain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookOperationOutcome {
    /// The shim now stands where a real cargo did.
    Installed,
    /// The installed shim differed from this binary's copy and was
    /// brought up to date.
    Refreshed,
    /// The installed shim matches this binary's copy and was untouched.
    AlreadyCurrent,
    /// The installed shim is newer than this reader's copy and remains unchanged.
    DowngradeRefused {
        /// The installed shim's declared framing version.
        installed: u64,
        /// The newest framing version this reader supports.
        supported: u64,
    },
    /// The real cargo has its name back.
    Removed,
    /// Nothing to do: no shim was installed.
    AlreadyAbsent,
    /// Nothing done: the shim is there with no real cargo beside it,
    /// which is not a state installing can repair.
    Orphaned,
}

/// Discovery distinguishes a missing cargo from a path that could not be inspected.
enum HookDiscovery {
    /// A toolchain has a cargo or a saved original to operate on.
    Discovered(Hook),
    /// Neither cargo nor the saved original exists in this directory.
    AbsentCargo,
    /// The path could not be inspected, with the original filesystem error.
    InspectionFailed {
        /// Toolchain directory, or its parent when reading an entry failed.
        path:  PathBuf,
        /// Filesystem error retained for the account report.
        error: Error,
    },
}

impl Hook {
    /// Yield each completed operation before starting the next toolchain.
    pub(crate) fn reports(
        operation: HookOperation,
    ) -> io::Result<impl Iterator<Item = ToolchainHookReport>> {
        Ok(Self::all()?
            .into_iter()
            .filter_map(move |discovery| match discovery {
                HookDiscovery::Discovered(hook) => Some(ToolchainHookReport {
                    toolchain: hook.name().to_owned(),
                    outcome:   hook.perform(operation),
                }),
                HookDiscovery::AbsentCargo => None,
                HookDiscovery::InspectionFailed { path, error } => Some(ToolchainHookReport {
                    toolchain: path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                    outcome:   ToolchainHookOutcome::Unreadable(format!(
                        "{}: {error}",
                        path.display()
                    )),
                }),
            }))
    }

    /// Every toolchain rustup has installed with a cargo under either
    /// its own name or the saved real binary's name.
    ///
    /// Sorted by name so a report reads the same twice running.
    fn all() -> io::Result<Vec<HookDiscovery>> { Self::in_rustup_home(&rustup_home()?) }

    /// Retain directory-entry and cargo inspection failures beside discovered toolchains.
    fn in_rustup_home(home: &Path) -> io::Result<Vec<HookDiscovery>> {
        let directory = home.join(TOOLCHAINS_DIR);
        let mut entries = Vec::new();
        let mut failures = Vec::new();
        // A rustup home with no toolchains directory -- root's, say -- has
        // no toolchains; it has not failed discovery.
        let listing = match fs::read_dir(&directory) {
            Ok(listing) => listing,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("{}: {error}", directory.display()),
                ));
            },
        };
        for entry in listing {
            match entry {
                Ok(entry) => entries.push(entry.path()),
                Err(error) => failures.push(HookDiscovery::InspectionFailed {
                    path: directory.clone(),
                    error,
                }),
            }
        }
        entries.sort();
        let mut discoveries: Vec<_> = entries
            .iter()
            .map(|entry| Self::at(entry))
            .filter(|discovery| !matches!(discovery, HookDiscovery::AbsentCargo))
            .collect();
        discoveries.extend(failures);
        Ok(discoveries)
    }

    /// Inspect both cargo paths without folding filesystem failures into absence.
    fn at(toolchain: &Path) -> HookDiscovery {
        let binaries = toolchain.join(TOOLCHAIN_BIN_DIR);
        let cargo = binaries.join(CARGO_NAME);
        let real = binaries.join(REAL_CARGO_NAME);
        match cargo.try_exists().and_then(|cargo_exists| {
            real.try_exists()
                .map(|real_exists| cargo_exists || real_exists)
        }) {
            Ok(false) => HookDiscovery::AbsentCargo,
            Ok(true) => HookDiscovery::Discovered(Self {
                name: toolchain
                    .file_name()
                    .unwrap_or(toolchain.as_os_str())
                    .to_string_lossy()
                    .into_owned(),
                cargo,
                real,
                staging: binaries.join(SHIM_STAGING_NAME),
            }),
            Err(error) => HookDiscovery::InspectionFailed {
                path: toolchain.to_owned(),
                error,
            },
        }
    }

    /// The toolchain this hook belongs to.
    pub(crate) fn name(&self) -> &str { &self.name }

    /// What is standing in front of cargo, retaining metadata and read failures.
    fn state(&self) -> io::Result<HookState> {
        let cargo_exists = self.cargo.try_exists().map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", self.cargo.display()))
        })?;
        let real_exists = self.real.try_exists().map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", self.real.display()))
        })?;
        if !cargo_exists && real_exists {
            return Ok(HookState::Repairable);
        }
        match inspect_shim(&self.cargo)? {
            CargoContents::Original => Ok(HookState::Absent),
            CargoContents::Shim if real_exists => Ok(HookState::Installed),
            CargoContents::Shim => Ok(HookState::Orphaned),
        }
    }

    /// Inspect status directly; mutations inspect state under their toolchain lock.
    fn perform(&self, operation: HookOperation) -> ToolchainHookOutcome {
        match operation {
            HookOperation::Install => self.ensure().map_or_else(
                |error| ToolchainHookOutcome::Failed(error.to_string()),
                ToolchainHookOutcome::Install,
            ),
            HookOperation::Uninstall => self.remove().map_or_else(
                |error| ToolchainHookOutcome::Failed(error.to_string()),
                ToolchainHookOutcome::Uninstall,
            ),
            HookOperation::Status => self.state().map_or_else(
                |error| ToolchainHookOutcome::Unreadable(error.to_string()),
                ToolchainHookOutcome::Status,
            ),
        }
    }

    /// Put the current shim in front of this toolchain's cargo, leaving
    /// an identical installed copy untouched.
    ///
    /// Writing the shim is the last step, so a failure part way through
    /// can leave `cargo` missing. Retrying writes the shim directly and
    /// leaves the saved real binary in place. The per-toolchain lock
    /// covers state inspection and publication, including repair.
    fn install(&self) -> io::Result<HookOperationOutcome> {
        #[cfg(not(test))]
        let source = env::current_exe()?;
        #[cfg(test)]
        let source = native_fixture::executable()?.to_owned();
        {
            let _inspection_lock =
                HookInstallationLock::acquire(self.cargo.with_file_name(SHIM_LOCK_NAME))?;
            match self.state()? {
                HookState::Orphaned => return Ok(HookOperationOutcome::Orphaned),
                HookState::Installed => {
                    let contents = fs::read(&self.cargo)?;
                    if let ShimVersion::Versioned(installed) = shim_version(&contents)?
                        && installed > CAPTURE_SHIM_VERSION
                    {
                        return Ok(HookOperationOutcome::DowngradeRefused {
                            installed,
                            supported: CAPTURE_SHIM_VERSION,
                        });
                    }
                    if contents == self.installed_bytes() && self.runner_is_current(&source)? {
                        return Ok(HookOperationOutcome::AlreadyCurrent);
                    }
                },
                HookState::Repairable if self.runner_is_current(&source)? => {
                    self.write_shim()?;
                    return Ok(HookOperationOutcome::Installed);
                },
                HookState::Absent | HookState::Repairable => {},
            }
            CaptureRunnerDestination::inspect(self.runner_path())?.for_install()?;
        }
        let prepared = PreparedCaptureRunner::new(&self.cargo, &source)?;
        let _installation_lock =
            HookInstallationLock::acquire(self.cargo.with_file_name(SHIM_LOCK_NAME))?;
        match self.state()? {
            HookState::Installed => {
                let contents = fs::read(&self.cargo)?;
                if contents == self.installed_bytes() {
                    self.write_runner(&source, &prepared)?;
                    return Ok(HookOperationOutcome::AlreadyCurrent);
                }
                if let ShimVersion::Versioned(installed) = shim_version(&contents)?
                    && installed > CAPTURE_SHIM_VERSION
                {
                    return Ok(HookOperationOutcome::DowngradeRefused {
                        installed,
                        supported: CAPTURE_SHIM_VERSION,
                    });
                }
                self.write_runner(&source, &prepared)?;
                self.write_shim()?;
                return Ok(HookOperationOutcome::Refreshed);
            },
            HookState::Orphaned => return Ok(HookOperationOutcome::Orphaned),
            HookState::Repairable => {
                self.write_runner(&source, &prepared)?;
                self.write_shim()?;
                return Ok(HookOperationOutcome::Installed);
            },
            HookState::Absent => {},
        }
        // Keep an unmarked first install or the fresh cargo that
        // `rustup update` put back over the shim.
        self.write_runner(&source, &prepared)?;
        fs::rename(&self.cargo, &self.real)?;
        self.write_shim()?;
        Ok(HookOperationOutcome::Installed)
    }

    /// Keep startup and the install subcommand on the same repair and
    /// content-comparison path as [`install`](Self::install).
    ///
    /// [`HookOperationOutcome::AlreadyCurrent`] requires reading the shim, but no
    /// shim writes: repeated launches and CI job hooks leave it untouched.
    fn ensure(&self) -> io::Result<HookOperationOutcome> { self.install() }

    /// Each installed shell names an executable that outlives an account staging guard.
    fn runner_path(&self) -> PathBuf { self.cargo.with_file_name(CAPTURE_RUNNER_NAME) }

    fn installed_bytes(&self) -> Vec<u8> { capture_shell_source(&self.runner_path()) }

    #[cfg(test)]
    fn installed_source(&self) -> String { shim_source_for(&self.runner_path()) }

    /// A current runner has independent storage, exact permissions, and equal bytes.
    fn runner_is_current(&self, source: &Path) -> io::Result<bool> {
        let source_metadata = fs::metadata(source)?;
        let destination = self.runner_path();
        let Ok(destination_metadata) = fs::metadata(&destination) else {
            return Ok(false);
        };
        Ok(
            destination_metadata.permissions().mode() & 0o7777 == SHIM_MODE
                && (destination_metadata.dev(), destination_metadata.ino())
                    != (source_metadata.dev(), source_metadata.ino())
                && capture_runner_matches(source, &destination)?,
        )
    }

    /// Publish the prepared inode while the toolchain lock owns the destination.
    fn write_runner(&self, source: &Path, prepared: &PreparedCaptureRunner) -> io::Result<()> {
        if self.runner_is_current(source)? {
            return Ok(());
        }
        CaptureRunnerDestination::inspect(self.runner_path())?
            .for_install()?
            .publish(&prepared.path())
    }

    /// Publish a complete executable entry through a new inode.
    /// Rename preserves any process that still reads the previous entry.
    fn write_shim(&self) -> io::Result<()> {
        let mut staging = OpenOptions::new()
            .write(true)
            .mode(SHIM_MODE)
            .create(true)
            .truncate(true)
            .open(&self.staging)?;
        staging.write_all(&self.installed_bytes())?;
        staging.set_permissions(fs::Permissions::from_mode(SHIM_MODE))?;
        fs::rename(&self.staging, &self.cargo)
    }

    /// Give the real cargo its name back.
    fn remove(&self) -> io::Result<HookOperationOutcome> {
        let _installation_lock =
            HookInstallationLock::acquire(self.cargo.with_file_name(SHIM_LOCK_NAME))?;
        match self.state()? {
            HookState::Absent => {
                CaptureRunnerDestination::inspect(self.runner_path())?.remove_owned()?;
                Ok(HookOperationOutcome::AlreadyAbsent)
            },
            HookState::Orphaned => Ok(HookOperationOutcome::Orphaned),
            HookState::Installed | HookState::Repairable => {
                fs::rename(&self.real, &self.cargo)?;
                CaptureRunnerDestination::inspect(self.runner_path())?.remove_owned()?;
                Ok(HookOperationOutcome::Removed)
            },
        }
    }
}

/// Compare runner bytes without allocating both executable images.
fn capture_runner_matches(source: &Path, destination: &Path) -> io::Result<bool> {
    let mut source = fs::File::open(source)?;
    let mut destination = fs::File::open(destination)?;
    if source.metadata()?.len() != destination.metadata()?.len() {
        return Ok(false);
    }
    let mut source_bytes = vec![0; 65_536];
    let mut destination_bytes = vec![0; 65_536];
    loop {
        let count = source.read(&mut source_bytes)?;
        destination.read_exact(&mut destination_bytes[..count])?;
        if source_bytes[..count] != destination_bytes[..count] {
            return Ok(false);
        }
        if count == 0 {
            return Ok(true);
        }
    }
}

/// Complete executable bytes prepared without holding a toolchain publication lock.
struct PreparedCaptureRunner {
    directory: TempDir,
}

impl PreparedCaptureRunner {
    fn new(cargo: &Path, source: &Path) -> io::Result<Self> {
        let parent = cargo
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| io::Error::other("toolchain has no preparation directory"))?;
        let directory = tempfile::Builder::new()
            .prefix(".cargo-tile-runner-")
            .tempdir_in(parent)?;
        let executable = directory.path().join(CAPTURE_RUNNER_NAME);
        fs::copy(source, &executable)?;
        fs::set_permissions(executable, fs::Permissions::from_mode(SHIM_MODE))?;
        Ok(Self { directory })
    }

    fn path(&self) -> PathBuf { self.directory.path().join(CAPTURE_RUNNER_NAME) }
}

/// Find the ownership marker without reading executable debug sections into memory.
fn capture_runner_has_marker(file: &mut File) -> io::Result<bool> {
    let mut buffer = [0; 8192];
    let mut window = Vec::with_capacity(buffer.len() + SHIM_MARKER.len());
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Ok(false);
        }
        window.extend_from_slice(&buffer[..count]);
        // Invalid UTF-8 bytes cannot form the ASCII ownership marker.
        if String::from_utf8_lossy(&window).contains(SHIM_MARKER) {
            return Ok(true);
        }
        let retained = window.len().saturating_sub(SHIM_MARKER.len() - 1);
        window.drain(..retained);
    }
}

/// Ownership established before publication or removal of the native runner.
enum CaptureRunnerDestination {
    Absent(PathBuf),
    Owned(OwnedCaptureRunner),
    Foreign,
}

impl CaptureRunnerDestination {
    fn inspect(path: PathBuf) -> io::Result<Self> {
        match fs::File::open(&path) {
            Ok(mut file) => {
                if capture_runner_has_marker(&mut file)? {
                    Ok(Self::Owned(OwnedCaptureRunner { path }))
                } else {
                    Ok(Self::Foreign)
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::Absent(path)),
            Err(error) => Err(error),
        }
    }

    fn remove_owned(self) -> io::Result<()> {
        match self {
            Self::Owned(runner) => runner.remove(),
            Self::Absent(_) | Self::Foreign => Ok(()),
        }
    }

    fn for_install(self) -> io::Result<WritableCaptureRunner> {
        match self {
            Self::Absent(path) => Ok(WritableCaptureRunner { path }),
            Self::Owned(runner) => Ok(WritableCaptureRunner { path: runner.path }),
            Self::Foreign => Err(io::Error::new(
                ErrorKind::AlreadyExists,
                "native runner name belongs to another file",
            )),
        }
    }
}

/// Only a checked owned runner has a removal operation.
struct OwnedCaptureRunner {
    path: PathBuf,
}

impl OwnedCaptureRunner {
    fn remove(self) -> io::Result<()> { fs::remove_file(self.path) }
}

/// An absent or owned destination can accept a runner on its own executable inode.
struct WritableCaptureRunner {
    path: PathBuf,
}

impl WritableCaptureRunner {
    fn publish(self, source: &Path) -> io::Result<()> {
        let staging = self.path.with_extension("staging");
        match fs::remove_file(&staging) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
        match fs::hard_link(source, &staging) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::CrossesDevices => {
                let mut source = fs::File::open(source)?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(SHIM_MODE)
                    .open(&staging)?;
                let owned = OwnedCaptureFile(staging);
                io::copy(&mut source, &mut file)?;
                file.set_permissions(fs::Permissions::from_mode(SHIM_MODE))?;
                return fs::rename(&owned.0, &self.path);
            },
            Err(error) => return Err(error),
        }
        let owned = OwnedCaptureFile(staging);
        fs::rename(&owned.0, &self.path)
    }
}

/// Exclusive access to one toolchain's install, repair, and removal operations.
/// The lock file's presence owns access until this guard is dropped.
struct HookInstallationLock {
    /// Only the guard that created this path may remove it.
    path: PathBuf,
}

impl HookInstallationLock {
    /// Wait briefly for another installer, then fail without touching
    /// cargo if exclusive creation remains unavailable.
    fn acquire(path: PathBuf) -> io::Result<Self> {
        let started = std::time::Instant::now();
        let wait_budget = SHIM_LOCK_RETRY_DELAY
            .saturating_mul(u32::try_from(SHIM_LOCK_RETRY_ATTEMPTS).map_err(io::Error::other)?);
        let mut retries_remaining = SHIM_LOCK_RETRY_ATTEMPTS;
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self { path }),
                Err(error)
                    if error.kind() == ErrorKind::AlreadyExists
                        && retries_remaining > 0
                        && started.elapsed() < wait_budget
                        && fs::symlink_metadata(&path)
                            .map_or(true, |metadata| metadata.is_file()) =>
                {
                    retries_remaining -= 1;
                    thread::sleep(
                        SHIM_LOCK_RETRY_DELAY.min(wait_budget.saturating_sub(started.elapsed())),
                    );
                },
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("{}: {error}; {SHIM_LOCK_RECOVERY}", path.display()),
                    ));
                },
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("{}: {error}", path.display()),
                    ));
                },
            }
        }
    }
}

impl Drop for HookInstallationLock {
    fn drop(&mut self) { drop(fs::remove_file(&self.path)); }
}

/// An executable copy accessible to account children until reporting ends.
pub(crate) struct StagedExecutable {
    /// The fresh directory is removed with all its contents when the guard drops.
    directory:  PathBuf,
    /// The executable path passed to each account child.
    executable: PathBuf,
}

impl StagedExecutable {
    /// Keep children independent of permissions on the administrator's home.
    pub(crate) fn path(&self) -> &Path { &self.executable }
}

impl Drop for StagedExecutable {
    fn drop(&mut self) { drop(fs::remove_dir_all(&self.directory)); }
}

/// Stage beneath a trusted parent so other accounts cannot replace the executable.
/// The admin command supplies the root-owned sticky system temporary directory.
pub(crate) fn stage_executable(parent: &Path, source: &Path) -> io::Result<StagedExecutable> {
    let directory = tempfile::Builder::new()
        .prefix(ACCOUNT_EXECUTABLE_PREFIX)
        .permissions(fs::Permissions::from_mode(ACCOUNT_EXECUTABLE_MODE))
        .tempdir_in(parent)?;
    fs::set_permissions(
        directory.path(),
        fs::Permissions::from_mode(ACCOUNT_EXECUTABLE_MODE),
    )?;
    let executable = directory.path().join(BINARY_NAME);
    fs::copy(source, &executable)?;
    fs::set_permissions(
        &executable,
        fs::Permissions::from_mode(ACCOUNT_EXECUTABLE_MODE),
    )?;
    Ok(StagedExecutable {
        directory: directory.keep(),
        executable,
    })
}

/// Read the system account database before the CLI starts any worker threads.
/// `getpwent` includes Directory Services accounts on macOS and NSS on Linux.
#[allow(
    unsafe_code,
    reason = "system account enumeration and home directories require libc getpwent; called only by the single-threaded admin command"
)]
pub(crate) fn system_accounts() -> io::Result<Vec<HookAccount>> {
    let mut accounts = Vec::new();
    // SAFETY: the admin CLI calls this before starting threads or other account
    // lookups. Every passwd string is copied before the next getpwent call, and
    // endpwent runs on both success and error before returning owned values.
    unsafe {
        libc::setpwent();
        let result = loop {
            #[cfg(target_os = "linux")]
            let errno = libc::__errno_location();
            #[cfg(target_os = "macos")]
            let errno = libc::__error();
            *errno = 0;
            let entry = libc::getpwent();
            if entry.is_null() {
                break if *errno == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                };
            }
            let entry = &*entry;
            if entry.pw_name.is_null() || entry.pw_dir.is_null() {
                break Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "account database returned an incomplete account",
                ));
            }
            accounts.push(HookAccount {
                name: CStr::from_ptr(entry.pw_name).to_string_lossy().into_owned(),
                uid:  entry.pw_uid,
                gid:  entry.pw_gid,
                home: PathBuf::from(OsStr::from_bytes(CStr::from_ptr(entry.pw_dir).to_bytes())),
            });
        };
        libc::endpwent();
        result?;
    }
    accounts.sort_by(|left, right| left.name.cmp(&right.name).then(left.uid.cmp(&right.uid)));
    Ok(accounts)
}

/// Resolve the account database memberships, including its primary group, before spawning.
#[allow(
    unsafe_code,
    reason = "libc getgrouplist is required to resolve complete account memberships on Linux and macOS"
)]
pub(crate) fn account_groups(name: &str, primary_gid: u32) -> io::Result<Vec<u32>> {
    let name =
        CString::new(name).map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    #[cfg(target_os = "macos")]
    let primary_gid = libc::c_int::try_from(primary_gid)
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    #[cfg(target_os = "linux")]
    let primary_gid: libc::gid_t = primary_gid;
    let groups = account_groups_with(primary_gid, |groups, count| {
        // SAFETY: name is NUL-terminated and lives through every call. The group
        // element type matches this platform's libc declaration, and count is
        // the allocated slice length on entry to each call.
        unsafe { libc::getgrouplist(name.as_ptr(), primary_gid, groups.as_mut_ptr(), count) }
    })?;
    #[cfg(target_os = "macos")]
    {
        groups
            .into_iter()
            .map(|group| {
                u32::try_from(group).map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
            })
            .collect()
    }
    #[cfg(target_os = "linux")]
    {
        Ok(groups)
    }
}

/// Grow an undersized group buffer within a ceiling, retaining the platform's element type.
fn account_groups_with<Group: Copy>(
    primary_gid: Group,
    mut resolve: impl FnMut(&mut [Group], &mut libc::c_int) -> libc::c_int,
) -> io::Result<Vec<Group>> {
    let mut groups = vec![primary_gid; ACCOUNT_GROUPS_INITIAL_CAPACITY];
    let count = loop {
        let mut count = libc::c_int::try_from(groups.len()).map_err(io::Error::other)?;
        if resolve(&mut groups, &mut count) != -1 {
            break count;
        }
        let needed = usize::try_from(count)
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
        if groups.len() == ACCOUNT_GROUPS_MAX_CAPACITY || needed > ACCOUNT_GROUPS_MAX_CAPACITY {
            return Err(io::Error::other(format!(
                "getgrouplist could not resolve groups within the ceiling of {ACCOUNT_GROUPS_MAX_CAPACITY} entries"
            )));
        }
        // glibc reports the required size; Darwin can leave the supplied count unchanged.
        let needed = if needed > groups.len() {
            needed
        } else {
            groups
                .len()
                .saturating_mul(ACCOUNT_GROUPS_GROWTH_FACTOR)
                .min(ACCOUNT_GROUPS_MAX_CAPACITY)
        };
        groups.resize(needed, primary_gid);
    };
    let count =
        usize::try_from(count).map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    if count > groups.len() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "getgrouplist returned {count} groups for a buffer of {} entries",
                groups.len()
            ),
        ));
    }
    groups.truncate(count);
    Ok(groups)
}

/// Handle explicit account records through the same staged executable for every operation.
/// Accounts without a default rustup home are omitted.
pub(crate) fn run_account_hooks(
    accounts: &[HookAccount],
    executable: &Path,
    operation: HookOperation,
) -> Vec<AccountHookReport> {
    run_account_hooks_with(accounts, executable, operation, |command, account| {
        if rustix::process::geteuid().is_root() {
            account_credentials(command, account)
        } else {
            command.uid(account.uid).gid(account.gid);
            Ok(())
        }
    })
}

/// Inject only credential preparation so resolver failures can be verified without root.
pub(crate) fn run_account_hooks_with(
    accounts: &[HookAccount],
    executable: &Path,
    operation: HookOperation,
    mut credentials: impl FnMut(&mut Command, &HookAccount) -> io::Result<()>,
) -> Vec<AccountHookReport> {
    let mut reports = Vec::new();
    for account in accounts {
        let home = account.home.join(RUSTUP_DIRNAME);
        if fs::metadata(&home).is_err_and(|error| error.kind() == ErrorKind::NotFound) {
            continue;
        }
        reports.push(run_account_hook(
            account,
            executable,
            operation,
            &mut credentials,
        ));
    }
    reports
}

/// Drop privileges before any toolchain discovery, lock, or operation.
fn run_account_hook(
    account: &HookAccount,
    executable: &Path,
    operation: HookOperation,
    credentials: &mut impl FnMut(&mut Command, &HookAccount) -> io::Result<()>,
) -> AccountHookReport {
    let mut command = Command::new(executable);
    if let Err(error) = credentials(&mut command, account) {
        return AccountHookReport::incomplete(
            account,
            operation,
            format!("could not resolve {}'s groups: {error}", account.name),
        );
    }
    command
        .env("HOME", &account.home)
        .env(RUSTUP_HOME_ENV, account.home.join(RUSTUP_DIRNAME))
        .arg(operation.subcommand())
        .arg(format!("--{ACCOUNT_HOOK_REPORT_FLAG}"))
        .output()
        .map_or_else(
            |error| {
                AccountHookReport::incomplete(
                    account,
                    operation,
                    format!(
                        "could not start the account child as {}: {error}",
                        account.name
                    ),
                )
            },
            |output| AccountHookReport::from_output(account, operation, &output),
        )
}

/// Parent-resolved groups and the uid used for Darwin's dynamic membership lookup.
#[cfg(any(target_os = "macos", test))]
pub(crate) struct DarwinGroupMembership {
    /// Complete resolved membership, with the primary gid first.
    pub(crate) groups: Vec<u32>,
    /// Prefix retained in the kernel credential; other groups resolve through uid.
    pub(crate) count:  libc::c_uint,
    /// Directory-services identity passed to the kernel initgroups entry point.
    pub(crate) uid:    libc::c_int,
}

#[cfg(any(target_os = "macos", test))]
impl DarwinGroupMembership {
    /// Prepare every allocation and conversion before the account child forks.
    pub(crate) fn new(
        uid: u32,
        primary_gid: u32,
        mut groups: Vec<u32>,
        limit: libc::c_long,
    ) -> io::Result<Self> {
        let limit = usize::try_from(limit)
            .ok()
            .filter(|limit| *limit > 0)
            .ok_or_else(|| io::Error::other("Darwin credential group limit is unavailable"))?;
        let primary = groups
            .iter()
            .position(|group| *group == primary_gid)
            .ok_or_else(|| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    "resolved groups omit the primary gid",
                )
            })?;
        groups.swap(0, primary);
        let count = libc::c_uint::try_from(groups.len().min(limit))
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
        let uid = libc::c_int::try_from(uid)
            .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
        Ok(Self { groups, count, uid })
    }
}

/// Apply root's resolved credentials together, before exec, without std clearing groups.
/// `CommandExt::groups` is unstable, so the callback also owns the uid/gid transition.
#[allow(
    unsafe_code,
    reason = "stable Command lacks explicit groups; the child must apply kernel group credentials before setgid and setuid using only libc credential syscalls"
)]
fn account_credentials(command: &mut Command, account: &HookAccount) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        // Apple's Libinfo lookup.subproj/libinfo.c initgroups calls this raw
        // libsyscall entry after resolving groups in userspace. XNU's
        // bsd/kern/syscalls.master and kern_prot.c initgroups/setgroups1 pass
        // gmuid to the external group resolver; setgroups opts out instead.
        // Source: github.com/apple-oss-distributions/Libinfo/blob/
        // 39b70c515baee5b609e7e91693edbd934b6845a1/lookup.subproj/libinfo.c
        fn __initgroups(
            count: libc::c_uint,
            groups: *const libc::gid_t,
            membership_uid: libc::c_int,
        ) -> libc::c_int;
    }
    let groups = account_groups(&account.name, account.gid)?;
    #[cfg(target_os = "macos")]
    let membership = {
        // SAFETY: sysconf takes the platform selector and has no pointer arguments.
        let limit = unsafe { libc::sysconf(libc::_SC_NGROUPS_MAX) };
        DarwinGroupMembership::new(account.uid, account.gid, groups, limit)?
    };
    #[cfg(target_os = "linux")]
    let count = groups.len();
    let uid = account.uid;
    let gid = account.gid;
    // SAFETY: the parent resolves and allocates groups before fork. The callback
    // uses only credential syscalls and last_os_error, with no allocation or
    // locks; groups owns at least count gid_t entries for its entire lifetime.
    // Darwin registers the account uid for groups beyond the credential prefix;
    // the first group is the primary gid, so setgid does not displace a group
    // and disable dynamic membership. No directory lookup runs in the child.
    // No std uid/gid setters precede it, and any failed syscall prevents exec.
    unsafe {
        command.pre_exec(move || {
            #[cfg(target_os = "macos")]
            let result = __initgroups(membership.count, membership.groups.as_ptr(), membership.uid);
            #[cfg(target_os = "linux")]
            let result = libc::setgroups(count, groups.as_ptr());
            if result == -1 || libc::setgid(gid) == -1 || libc::setuid(uid) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
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

/// What a successful read establishes about the cargo file.
enum CargoContents {
    /// The opening contains the capture shim marker.
    Shim,
    /// The opening belongs to an unmarked cargo binary.
    Original,
}

/// Whether the installed shim declares its framing version.
enum ShimVersion {
    /// Historical shims have no version line and may be upgraded.
    Unversioned,
    /// The header explicitly identifies the shim's framing version.
    Versioned(u64),
}

/// Inspect only the shim header; malformed declarations must not authorize replacement.
fn shim_version(contents: &[u8]) -> io::Result<ShimVersion> {
    let opening = &contents[..contents.len().min(SHIM_MARKER_SEARCH_BYTES)];
    let mut version = ShimVersion::Unversioned;
    for line in String::from_utf8_lossy(opening).split_inclusive('\n') {
        let Some(terminated) = line.strip_suffix('\n') else {
            // Even a partial prefix may begin a declaration cut off by the opening.
            if line.starts_with(SHIM_VERSION_PREFIX) || SHIM_VERSION_PREFIX.starts_with(line) {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "incomplete shim version line",
                ));
            }
            continue;
        };
        let line = terminated.strip_suffix('\r').unwrap_or(terminated);
        let Some(number) = line.strip_prefix(SHIM_VERSION_PREFIX) else {
            continue;
        };
        if matches!(version, ShimVersion::Versioned(_)) {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "duplicate shim version line",
            ));
        }
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "invalid shim version line",
            ));
        }
        version = ShimVersion::Versioned(number.parse().map_err(|error| {
            io::Error::new(
                ErrorKind::InvalidData,
                format!("invalid shim version: {error}"),
            )
        })?);
    }
    Ok(version)
}

/// Read the bounded opening completely, retaining open and read errors.
fn inspect_shim(path: &Path) -> io::Result<CargoContents> {
    let mut file = fs::File::open(path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?
        .take(SHIM_MARKER_SEARCH_BYTES as u64);
    let mut opening = Vec::new();
    file.read_to_end(&mut opening)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    Ok(if String::from_utf8_lossy(&opening).contains(SHIM_MARKER) {
        CargoContents::Shim
    } else {
        CargoContents::Original
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    /// Retain a named runner symlink for the test process.
    pub(super) fn fixture_runner() -> PathBuf {
        static DIRECTORY: LazyLock<TempDir> = std::sync::LazyLock::new(|| {
            let directory = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(
                native_fixture::executable().unwrap(),
                directory.path().join(crate::constants::CAPTURE_RUNNER_NAME),
            )
            .unwrap();
            directory
        });
        DIRECTORY.path().join(crate::constants::CAPTURE_RUNNER_NAME)
    }
    pub(super) fn utf8_shell_source(executable: &Path) -> String {
        String::from_utf8(super::capture_shell_source(executable)).unwrap()
    }
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Barrier;
    use std::sync::LazyLock;
    use std::time::SystemTime;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::constants::HANDLER_SUBCOMMAND_NAME;
    use crate::constants::HOOK_TEST_DIRECTORY_COLLISION_CARGO;
    use crate::constants::HOOK_TEST_REAL_CARGO;
    use crate::constants::HOOK_TEST_VERSION_ARGUMENT;
    use crate::constants::LOCK_WAIT_MARKER;
    use crate::constants::PORT_SUBCOMMAND_NAME;
    use crate::constants::SUBCOMMAND_NAME;

    #[test]
    fn embedded_shim_declares_the_supported_framing_version() {
        assert!(matches!(
            shim_version(SHIM_SOURCE.as_bytes()).unwrap(),
            ShimVersion::Versioned(version) if version == CAPTURE_SHIM_VERSION
        ));
    }

    #[test]
    fn invalid_or_repeated_version_lines_preserve_installed_bytes() {
        let (_home, hook) = toolchain(CARGO_NAME);
        hook.install().unwrap();
        for version in [
            "",
            "newer",
            "+4",
            "18446744073709551616",
            "4\n# cargo-tile-shim-version: 3",
        ] {
            let contents = format!("#!/bin/sh\n# {SHIM_MARKER}\n{SHIM_VERSION_PREFIX}{version}\n");
            fs::write(&hook.cargo, &contents).unwrap();
            let written = mark_old_mtime(&hook.cargo);

            assert_eq!(hook.ensure().unwrap_err().kind(), ErrorKind::InvalidData);
            assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), contents);
            assert_eq!(
                fs::metadata(&hook.cargo).unwrap().modified().unwrap(),
                written
            );
            assert_eq!(fs::read_to_string(&hook.real).unwrap(), CARGO_NAME);
            assert!(!hook.staging.exists());
        }
    }

    #[test]
    fn version_lines_crossing_the_opening_preserve_installed_files() {
        let (_home, hook) = toolchain(CARGO_NAME);
        fs::rename(&hook.cargo, &hook.real).unwrap();
        let header = format!("#!/bin/sh\n# {SHIM_MARKER}\n#");
        let future_version = format!("{CAPTURE_SHIM_VERSION}0");
        for ending in ["\n", "\r\n"] {
            let declaration = format!("{SHIM_VERSION_PREFIX}{future_version}{ending}");
            for visible in 1..=declaration.len() {
                let padding = " ".repeat(SHIM_MARKER_SEARCH_BYTES - header.len() - 1 - visible);
                let contents = format!("{header}{padding}\n{declaration}");
                fs::write(&hook.cargo, &contents).unwrap();
                for path in [&hook.cargo, &hook.real] {
                    fs::set_permissions(path, fs::Permissions::from_mode(0o751)).unwrap();
                    mark_old_mtime(path);
                }
                let preserved = [&hook.cargo, &hook.real].map(|path| {
                    let metadata = fs::metadata(path).unwrap();
                    (
                        fs::read(path).unwrap(),
                        metadata.permissions().mode(),
                        metadata.modified().unwrap(),
                    )
                });

                for install in [Hook::install, Hook::ensure] {
                    if visible == declaration.len() {
                        assert_eq!(
                            install(&hook).unwrap(),
                            HookOperationOutcome::DowngradeRefused {
                                installed: future_version.parse().unwrap(),
                                supported: CAPTURE_SHIM_VERSION,
                            }
                        );
                    } else {
                        assert_eq!(install(&hook).unwrap_err().kind(), ErrorKind::InvalidData);
                    }
                    for (path, (bytes, mode, modified)) in
                        [&hook.cargo, &hook.real].into_iter().zip(&preserved)
                    {
                        let metadata = fs::metadata(path).unwrap();
                        assert_eq!(&fs::read(path).unwrap(), bytes);
                        assert_eq!(metadata.permissions().mode(), *mode);
                        assert_eq!(metadata.modified().unwrap(), *modified);
                    }
                    assert!(!hook.staging.exists());
                    assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());
                }
            }
        }
    }

    #[test]
    fn version_lines_without_a_final_newline_are_invalid() {
        for ending in ["", "\r"] {
            let contents = format!("{SHIM_VERSION_PREFIX}{CAPTURE_SHIM_VERSION}{ending}");
            assert!(matches!(
                shim_version(contents.as_bytes()),
                Err(error) if error.kind() == ErrorKind::InvalidData
            ));
        }
    }

    #[test]
    fn incomplete_script_lines_do_not_invalidate_complete_version_declarations() {
        let contents = format!(
            "{SHIM_VERSION_PREFIX}{CAPTURE_SHIM_VERSION}\n{}",
            "exec cargo-real ".repeat(SHIM_MARKER_SEARCH_BYTES)
        );
        assert!(matches!(
            shim_version(contents.as_bytes()).unwrap(),
            ShimVersion::Versioned(version) if version == CAPTURE_SHIM_VERSION
        ));
        assert!(matches!(
            shim_version(b"#!/bin/sh\nexec cargo-real").unwrap(),
            ShimVersion::Unversioned
        ));
    }

    #[test]
    fn malformed_downgrade_reports_retain_the_toolchain_as_incomplete() {
        for result in ["4", "4\t4", "3\t4", "future\t3", "4\t3\textra"] {
            let report = ToolchainHookReport::parse(
                HookOperation::Install,
                &format!("nightly\tdowngrade refused\t{result}"),
            )
            .unwrap();
            assert_eq!(report.toolchain, "nightly");
            assert!(matches!(report.outcome, ToolchainHookOutcome::Failed(_)));
            assert!(report.outcome.is_incomplete());
        }
    }

    #[test]
    fn darwin_membership_keeps_resolved_groups_beyond_the_credential_prefix() {
        let groups = vec![7, 11, 19];
        for limit in 1..=groups.len() + 1 {
            let membership = DarwinGroupMembership::new(
                501,
                19,
                groups.clone(),
                libc::c_long::try_from(limit).unwrap(),
            )
            .unwrap();
            assert_eq!(membership.uid, 501);
            assert_eq!(
                membership.count,
                u32::try_from(limit.min(groups.len())).unwrap()
            );
            assert_eq!(membership.groups, [19, 11, 7]);
        }
    }

    #[test]
    fn darwin_membership_requires_a_positive_group_limit_and_primary_gid() {
        for limit in [libc::c_long::MIN, -1, 0] {
            assert!(DarwinGroupMembership::new(501, 19, vec![19], limit).is_err());
        }
        assert!(DarwinGroupMembership::new(501, 19, vec![7, 11], 16).is_err());
        assert!(DarwinGroupMembership::new(u32::MAX, 19, vec![19], 16).is_err());
    }

    #[test]
    fn credential_failure_is_incomplete_and_later_accounts_still_run() {
        let fixture = tempdir().unwrap();
        let executable = fixture.path().join(BINARY_NAME);
        fs::write(&executable, "#!/bin/sh\nprintf '%s\\n' started > \"$HOME/child-started\"\ncase $1 in\ninstall|status) printf 'stable\\tinstalled\\n';;\nuninstall) printf 'stable\\tremoved\\n';;\nesac\n").unwrap();
        fs::set_permissions(
            &executable,
            fs::Permissions::from_mode(ACCOUNT_EXECUTABLE_MODE),
        )
        .unwrap();
        let accounts = ["blocked", "later"].map(|name| {
            let home = fixture.path().join(name);
            fs::create_dir_all(home.join(RUSTUP_DIRNAME)).unwrap();
            HookAccount {
                name: name.to_owned(),
                uid: rustix::process::getuid().as_raw(),
                gid: rustix::process::getgid().as_raw(),
                home,
            }
        });
        for operation in [
            HookOperation::Install,
            HookOperation::Uninstall,
            HookOperation::Status,
        ] {
            let mut attempted = Vec::new();
            let reports =
                run_account_hooks_with(&accounts, &executable, operation, |_, account| {
                    attempted.push(account.name.clone());
                    if account.name == "blocked" {
                        Err(io::Error::other("membership resolver unavailable"))
                    } else {
                        Ok(())
                    }
                });
            assert_eq!(attempted, ["blocked", "later"]);
            assert_eq!(reports.len(), accounts.len());
            assert_eq!(
                reports[0].outcome,
                AccountHookOutcome::Incomplete(
                    "could not resolve blocked's groups: membership resolver unavailable"
                        .to_owned()
                )
            );
            assert_eq!(reports[0].toolchains, [] as [ToolchainHookReport; 0]);
            assert!(!accounts[0].home.join("child-started").exists());
            assert_eq!(reports[1].outcome, AccountHookOutcome::Completed);
            assert_eq!(reports[1].toolchains.len(), 1);
            assert_eq!(
                fs::read(accounts[1].home.join("child-started")).unwrap(),
                b"started\n"
            );
            let rendered = reports[0].to_string();
            assert!(rendered.contains("blocked: "), "{rendered}");
            assert!(rendered.contains("incomplete: could not resolve blocked's groups: membership resolver unavailable"), "{rendered}");
            assert_eq!(
                operation.completion(&reports).is_ok(),
                operation == HookOperation::Install
            );
        }
    }

    #[test]
    fn discovery_retains_an_uninspectable_toolchain_beside_a_working_one() {
        let (home, _) = toolchain(CARGO_NAME);
        let unreadable = home.path().join(TOOLCHAINS_DIR).join("unreadable");
        std::os::unix::fs::symlink(&unreadable, &unreadable).unwrap();
        let discoveries = Hook::in_rustup_home(home.path()).unwrap();
        assert_eq!(discoveries.len(), 2);
        assert!(
            matches!(&discoveries[0], HookDiscovery::Discovered(hook) if hook.name() == "stable-test")
        );
        assert!(
            matches!(&discoveries[1], HookDiscovery::InspectionFailed { path, error } if path == &unreadable && error.raw_os_error() == Some(libc::ELOOP))
        );
    }

    #[test]
    fn an_unreadable_cargo_retains_its_error_without_changing_the_toolchain() {
        let (_home, hook) = toolchain(CARGO_NAME);
        fs::remove_file(&hook.cargo).unwrap();
        fs::create_dir(&hook.cargo).unwrap();
        let error = hook.state().unwrap_err().to_string();
        assert_eq!(
            hook.perform(HookOperation::Status),
            ToolchainHookOutcome::Unreadable(error)
        );
        assert!(hook.cargo.is_dir());
        assert!(!hook.real.exists());
        assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());
    }

    #[test]
    fn mutations_acquire_the_lock_before_classifying_an_orphaned_shim() {
        let (_home, hook) = toolchain(SHIM_SOURCE);
        let lock_path = hook.cargo.with_file_name(SHIM_LOCK_NAME);
        let _installation_lock = HookInstallationLock::acquire(lock_path.clone()).unwrap();

        assert_eq!(
            hook.perform(HookOperation::Status),
            ToolchainHookOutcome::Status(HookState::Orphaned)
        );
        thread::scope(|scope| {
            let operations = [HookOperation::Install, HookOperation::Uninstall].map(|operation| {
                let hook = &hook;
                let lock_path = &lock_path;
                scope.spawn(move || {
                    let outcome = hook.perform(operation);
                    assert!(
                        matches!(&outcome, ToolchainHookOutcome::Failed(reason) if reason.contains(&lock_path.display().to_string())),
                        "{outcome:?}"
                    );
                })
            });
            for operation in operations {
                operation.join().unwrap();
            }
        });
        assert_eq!(
            fs::read_to_string(&hook.cargo).unwrap(),
            hook.installed_source()
        );
        assert!(!hook.real.exists());
        assert!(lock_path.exists());
    }

    #[test]
    fn mutations_acquire_the_lock_before_inspecting_unreadable_cargo() {
        let (_home, hook) = toolchain(CARGO_NAME);
        fs::remove_file(&hook.cargo).unwrap();
        fs::create_dir(&hook.cargo).unwrap();
        let lock_path = hook.cargo.with_file_name(SHIM_LOCK_NAME);
        let _installation_lock = HookInstallationLock::acquire(lock_path.clone()).unwrap();

        assert!(matches!(
            hook.perform(HookOperation::Status),
            ToolchainHookOutcome::Unreadable(_)
        ));
        thread::scope(|scope| {
            let operations = [HookOperation::Install, HookOperation::Uninstall].map(|operation| {
                let hook = &hook;
                let lock_path = &lock_path;
                scope.spawn(move || {
                    let outcome = hook.perform(operation);
                    assert!(
                        matches!(&outcome, ToolchainHookOutcome::Failed(reason) if reason.contains(&lock_path.display().to_string())),
                        "{outcome:?}"
                    );
                })
            });
            for operation in operations {
                operation.join().unwrap();
            }
        });
        assert!(hook.cargo.is_dir());
        assert!(!hook.real.exists());
        assert!(lock_path.exists());
    }

    #[test]
    fn account_groups_resizes_and_returns_every_reported_group() {
        let expected: Vec<u32> = (0..=ACCOUNT_GROUPS_INITIAL_CAPACITY)
            .map(|group| u32::try_from(group).unwrap())
            .collect();
        let mut calls = 0;
        let groups = account_groups_with(expected[0], |groups, count| {
            calls += 1;
            assert_eq!(usize::try_from(*count).unwrap(), groups.len());
            if calls == 1 {
                assert_eq!(groups.len(), ACCOUNT_GROUPS_INITIAL_CAPACITY);
                *count = libc::c_int::try_from(expected.len()).unwrap();
                return -1;
            }
            assert_eq!(calls, 2);
            assert_eq!(groups.len(), expected.len());
            groups.copy_from_slice(&expected);
            *count = libc::c_int::try_from(expected.len()).unwrap();
            0
        })
        .unwrap();

        assert_eq!(calls, 2);
        assert_eq!(groups, expected);
    }

    #[test]
    fn account_groups_reports_failure_after_resizing() {
        let needed = ACCOUNT_GROUPS_MAX_CAPACITY;
        let mut calls = 0;
        let error = account_groups_with(0_u32, |groups, count| {
            calls += 1;
            assert_eq!(usize::try_from(*count).unwrap(), groups.len());
            if calls == 1 {
                assert_eq!(groups.len(), ACCOUNT_GROUPS_INITIAL_CAPACITY);
            } else {
                assert_eq!(calls, 2);
                assert_eq!(groups.len(), needed);
            }
            *count = libc::c_int::try_from(needed).unwrap();
            -1
        })
        .unwrap_err();

        assert_eq!(calls, 2);
        assert_eq!(error.kind(), ErrorKind::Other);
        assert_eq!(
            error.to_string(),
            format!("getgrouplist could not resolve groups within the ceiling of {needed} entries")
        );
    }

    #[test]
    fn account_groups_rejects_negative_retry_counts_without_calling_again() {
        let initial_count = libc::c_int::try_from(ACCOUNT_GROUPS_INITIAL_CAPACITY).unwrap();
        let mut calls = 0;
        let error = account_groups_with(0_u32, |groups, count| {
            calls += 1;
            assert_eq!(calls, 1);
            assert_eq!(groups.len(), ACCOUNT_GROUPS_INITIAL_CAPACITY);
            assert_eq!(*count, initial_count);
            *count = -1;
            -1
        })
        .unwrap_err();

        assert_eq!(calls, 1);
        assert_eq!(error.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn account_groups_grows_when_failed_counts_supply_no_larger_size() {
        let initial_count = libc::c_int::try_from(ACCOUNT_GROUPS_INITIAL_CAPACITY).unwrap();
        let expected: Vec<u32> = (0..=ACCOUNT_GROUPS_INITIAL_CAPACITY)
            .map(|group| u32::try_from(group).unwrap())
            .collect();
        for reported_count in [0, initial_count - 1, initial_count] {
            let mut calls = 0;
            let groups = account_groups_with(expected[0], |groups, count| {
                calls += 1;
                assert_eq!(usize::try_from(*count).unwrap(), groups.len());
                if calls == 1 {
                    assert_eq!(groups.len(), ACCOUNT_GROUPS_INITIAL_CAPACITY);
                    *count = reported_count;
                    return -1;
                }
                assert_eq!(calls, 2);
                assert_eq!(
                    groups.len(),
                    ACCOUNT_GROUPS_INITIAL_CAPACITY * ACCOUNT_GROUPS_GROWTH_FACTOR
                );
                groups[..expected.len()].copy_from_slice(&expected);
                *count = libc::c_int::try_from(expected.len()).unwrap();
                0
            })
            .unwrap();

            assert_eq!(calls, 2);
            assert_eq!(groups, expected, "reported count {reported_count}");
        }
    }

    #[test]
    fn account_groups_retries_darwin_overflow_until_every_group_fits() {
        let first_growth = ACCOUNT_GROUPS_INITIAL_CAPACITY * ACCOUNT_GROUPS_GROWTH_FACTOR;
        let expected: Vec<libc::c_int> = (0..=first_growth)
            .map(|group| libc::c_int::try_from(group).unwrap())
            .collect();
        let mut capacities = Vec::new();
        let groups = account_groups_with(expected[0], |groups, count| {
            assert_eq!(usize::try_from(*count).unwrap(), groups.len());
            capacities.push(groups.len());
            if groups.len() < expected.len() {
                return -1;
            }
            groups[..expected.len()].copy_from_slice(&expected);
            *count = libc::c_int::try_from(expected.len()).unwrap();
            0
        })
        .unwrap();

        assert_eq!(
            capacities,
            [
                ACCOUNT_GROUPS_INITIAL_CAPACITY,
                first_growth,
                first_growth * ACCOUNT_GROUPS_GROWTH_FACTOR,
            ]
        );
        assert_eq!(groups, expected);
    }

    #[test]
    fn account_groups_stops_persistent_overflow_at_the_ceiling() {
        let mut previous_capacity = 0;
        let error = account_groups_with(0_u32, |groups, count| {
            assert_eq!(usize::try_from(*count).unwrap(), groups.len());
            assert!(groups.len() > previous_capacity);
            assert!(groups.len() <= ACCOUNT_GROUPS_MAX_CAPACITY);
            if previous_capacity != 0 {
                assert_eq!(
                    groups.len(),
                    previous_capacity * ACCOUNT_GROUPS_GROWTH_FACTOR
                );
            }
            previous_capacity = groups.len();
            -1
        })
        .unwrap_err();

        assert_eq!(previous_capacity, ACCOUNT_GROUPS_MAX_CAPACITY);
        assert_eq!(error.kind(), ErrorKind::Other);
        assert_eq!(
            error.to_string(),
            format!(
                "getgrouplist could not resolve groups within the ceiling of {ACCOUNT_GROUPS_MAX_CAPACITY} entries"
            )
        );
    }

    #[test]
    fn account_groups_rejects_required_size_above_the_ceiling_without_retrying() {
        let mut calls = 0;
        let error = account_groups_with(0_u32, |groups, count| {
            calls += 1;
            assert_eq!(calls, 1);
            assert_eq!(groups.len(), ACCOUNT_GROUPS_INITIAL_CAPACITY);
            *count = libc::c_int::try_from(ACCOUNT_GROUPS_MAX_CAPACITY + 1).unwrap();
            -1
        })
        .unwrap_err();

        assert_eq!(calls, 1);
        assert_eq!(error.kind(), ErrorKind::Other);
        assert_eq!(
            error.to_string(),
            format!(
                "getgrouplist could not resolve groups within the ceiling of {ACCOUNT_GROUPS_MAX_CAPACITY} entries"
            )
        );
    }

    /// Enumerate the host database in an isolated process, without installing
    /// anything or sharing libc's enumeration cursor with another test.
    #[test]
    fn system_account_database_includes_the_running_account() {
        if env::var_os("CARGO_TILE_TEST_ACCOUNT_DATABASE").is_some() {
            let accounts = system_accounts().unwrap();
            let uid = rustix::process::getuid().as_raw();
            assert!(accounts.iter().any(|account| account.uid == uid));
            assert!(accounts.iter().all(|account| !account.name.is_empty()));
            return;
        }
        let output = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "hook::tests::system_account_database_includes_the_running_account",
                "--nocapture",
            ])
            .env("CARGO_TILE_TEST_ACCOUNT_DATABASE", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Require a discovered hook while preserving the failure detail in test output.
    fn discovered_hook(toolchain: &Path) -> Hook {
        match Hook::at(toolchain) {
            HookDiscovery::Discovered(hook) => hook,
            HookDiscovery::AbsentCargo => panic!("{} has no cargo", toolchain.display()),
            HookDiscovery::InspectionFailed { path, error } => {
                panic!("{}: {error}", path.display())
            },
        }
    }

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
        let hook = discovered_hook(&home.path().join(TOOLCHAINS_DIR).join("stable-test"));
        if contents == SHIM_SOURCE {
            fs::write(&hook.cargo, hook.installed_source()).unwrap();
        }
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
        assert!(shim_source().starts_with("#!/bin/sh\n"));
        let cargo = || {
            let mut command = Command::new("cargo-real");
            command.args(["build", "a b", ""]);
            command
        };
        let linux = CaptureScript::UtilLinux.command(&cargo(), Path::new("run.log"));
        assert_eq!(
            linux.get_args().collect::<Vec<_>>(),
            [
                "-q",
                "-e",
                "-f",
                "-c",
                "exec 'cargo-real' 'build' 'a b' ''",
                "run.log"
            ]
        );
        let bsd = CaptureScript::Bsd.command(&cargo(), Path::new("run.log"));
        assert_eq!(
            bsd.get_args().collect::<Vec<_>>(),
            ["-q", "-t", "0", "run.log", "cargo-real", "build", "a b", ""]
        );
    }

    /// A log is read only while the run writing it is alive, so the run
    /// takes it away as it goes. Unconditionally: what a finished log
    /// happens to hold is no longer a question the shim asks, which is
    /// what freed it from spelling out the reader's markers to answer.
    #[test]
    fn the_shim_retires_its_log_when_its_run_ends() {
        let directory = tempfile::tempdir().unwrap();
        let publication =
            CapturePublication::prepare(&directory.path().join("capture"), &["build".into()])
                .unwrap();
        let registration = publication.registration.0.clone();
        let log = publication.log.0.clone();
        assert!(registration.exists());
        assert!(log.exists());
        publication.finish();
        assert!(!registration.exists());
        assert!(!log.exists());
        assert!(!shim_source().contains(LOCK_WAIT_MARKER));
    }

    /// A directory can win the name after setup checks it; POSIX ln
    /// then succeeds inside it, which must still cause uncaptured cargo.
    #[test]
    fn a_directory_arriving_at_publication_preserves_original_cargo() {
        if let Some(shim) = env::var_os("HOOK_TEST_NATIVE_SHIM") {
            let parent = PathBuf::from(env::var_os("HOOK_TEST_NATIVE_PARENT").unwrap());
            let observations = PathBuf::from(env::var_os("HOOK_TEST_OBSERVATIONS").unwrap());
            let output = fs::OpenOptions::new()
                .write(true)
                .open("/dev/fd/9")
                .unwrap();
            rustix::stdio::dup2_stdout(&output).unwrap();
            let status = CaptureTestPublication {
                boot:            "test-boot",
                birth:           "1",
                generation:      "collision",
                environment:     &[],
                after_readiness: || Ok(()),
            }
            .run(
                Path::new(&shim),
                parent,
                ["check", "--quiet", "--message-format=json", "--", "a b", ""]
                    .map(Into::into)
                    .to_vec(),
                |_, registration| {
                    fs::create_dir(registration)?;
                    fs::write(registration.join("keep"), b"preserved\n")?;
                    fs::write(
                        observations.join("registration-path"),
                        registration.as_os_str().as_bytes(),
                    )
                },
                || Ok(()),
            )
            .unwrap();
            std::process::exit(status);
        }
        let (home, hook) = toolchain(HOOK_TEST_DIRECTORY_COLLISION_CARGO);
        fs::set_permissions(&hook.cargo, fs::Permissions::from_mode(SHIM_MODE)).unwrap();
        hook.install().unwrap();
        let observations = home.path().join("observations");
        let parent = home.path().join("capture");
        let root = parent.join(rustix::process::getuid().as_raw().to_string());
        fs::write(
            &hook.cargo,
            shim_source().replace(
                "capture_parent=/tmp/cargo-tile",
                &format!("capture_parent='{}'", parent.display()),
            ),
        )
        .unwrap();
        fs::create_dir(&observations).unwrap();
        let arguments = ["check", "--quiet", "--message-format=json", "--", "a b", ""];
        let output = Command::new("sh")
            .args([
                "-c",
                "umask 0066; exec \"$@\" 9>&1 >/dev/null",
                "publication-test",
            ])
            .arg(env::current_exe().unwrap())
            .args([
                "--exact",
                "hook::tests::a_directory_arriving_at_publication_preserves_original_cargo",
                "--nocapture",
            ])
            .env("HOOK_TEST_NATIVE_SHIM", &hook.cargo)
            .env("HOOK_TEST_NATIVE_PARENT", &parent)
            .env("POSIXLY_CORRECT", "1")
            .env("HOOK_TEST_OBSERVATIONS", &observations)
            .env_remove("CARGOTILE_NESTED")
            .env_remove("CARGO_TERM_PROGRESS_WHEN")
            .env_remove("CARGO_TERM_PROGRESS_WIDTH")
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(37));
        assert_eq!(output.stdout, b"cargo-stdout\n");
        assert_eq!(output.stderr, b"cargo-stderr\n");
        let expected: Vec<u8> = arguments
            .iter()
            .flat_map(|word| word.bytes().chain(std::iter::once(0)))
            .collect();
        assert_eq!(fs::read(observations.join("arguments")).unwrap(), expected);
        let umask = fs::read_to_string(observations.join("umask")).unwrap();
        assert_eq!(u32::from_str_radix(umask.trim(), 8).unwrap(), 0o066);
        assert_eq!(
            fs::read(observations.join("environment")).unwrap(),
            b"unset\0unset\0unset\0"
        );
        let registration = fs::read_to_string(observations.join("registration-path")).unwrap();
        let registration = PathBuf::from(registration);
        assert_eq!(fs::read(registration.join("keep")).unwrap(), b"preserved\n");
        assert_eq!(fs::read_dir(&registration).unwrap().count(), 1);
        assert_eq!(
            fs::read_dir(registration.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
        assert_eq!(fs::read_dir(root.join("state")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }

    /// Whether the shim's exemption arm names this subcommand. The arm
    /// lists several, so what matters is that this one is among them
    /// rather than what the whole line reads.
    fn shim_passes_through(subcommand: &str) -> bool { !capture_requested(&[subcommand.into()]) }

    /// The grid is reachable as `cargo tile`, which puts it in front of
    /// the shim like anything else. Capturing it would run a terminal UI
    /// under `script`, copying every redraw into a log for as long as
    /// the grid stayed open.
    #[test]
    fn the_shim_passes_the_grids_own_subcommand_through() {
        assert!(shim_passes_through(SUBCOMMAND_NAME));
    }

    #[test]
    fn the_shim_passes_the_sibling_terminal_uis_through() {
        assert!(shim_passes_through(PORT_SUBCOMMAND_NAME));
        assert!(shim_passes_through(HANDLER_SUBCOMMAND_NAME));
    }

    #[test]
    fn a_real_cargo_reads_as_no_shim_installed() {
        let (_home, hook) = toolchain("\u{7f}ELF not a script at all");

        assert_eq!(hook.state().unwrap(), HookState::Absent);
    }

    #[test]
    fn installing_moves_the_real_cargo_aside_and_keeps_every_byte_of_it() {
        let real = "\u{7f}ELF the one and only real cargo";
        let (_home, hook) = toolchain(real);

        assert_eq!(hook.install().unwrap(), HookOperationOutcome::Installed);
        assert_eq!(hook.state().unwrap(), HookState::Installed);
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), real);
        assert!(
            fs::read_to_string(&hook.cargo)
                .unwrap()
                .contains(SHIM_MARKER)
        );
    }

    /// A held lock must stop the install before it moves cargo or
    /// creates a staging file, and a failed contender must leave it held.
    #[test]
    fn installing_with_a_held_lock_leaves_the_toolchain_untouched() {
        let (_home, hook) = toolchain(CARGO_NAME);
        let lock_path = hook.cargo.with_file_name(SHIM_LOCK_NAME);
        let installation_lock = HookInstallationLock::acquire(lock_path.clone()).unwrap();

        assert_eq!(hook.install().unwrap_err().kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), CARGO_NAME);
        assert!(!hook.real.exists());
        assert!(!hook.staging.exists());
        assert!(lock_path.exists());

        drop(installation_lock);
        assert_eq!(hook.install().unwrap(), HookOperationOutcome::Installed);
        assert!(!lock_path.exists());
    }

    /// A stranded lock must name its path and the operator's recovery
    /// steps once the bounded wait finishes, without deleting the lock.
    #[test]
    fn an_exhausted_installation_lock_reports_how_to_recover() {
        let (_home, hook) = toolchain(CARGO_NAME);
        let lock_path = hook.cargo.with_file_name(SHIM_LOCK_NAME);
        fs::write(&lock_path, CARGO_NAME).unwrap();

        let error = hook.install().unwrap_err();
        let message = error.to_string();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(message.contains(&lock_path.display().to_string()));
        assert!(message.contains(SHIM_LOCK_RECOVERY));
        assert!(message.lines().eq([message.as_str()]));
        assert_eq!(fs::read_to_string(&lock_path).unwrap(), CARGO_NAME);
    }

    /// Independent installers starting together must agree which one
    /// saves cargo; the second must inspect the first one's finished shim.
    #[test]
    fn concurrent_installs_preserve_the_original_real_cargo() {
        let (_home, hook) = toolchain(CARGO_NAME);
        let toolchain = hook.cargo.parent().unwrap().parent().unwrap();
        let competing_hook = discovered_hook(toolchain);
        let installers = [&hook, &competing_hook];
        let start = Barrier::new(installers.len());

        let changes = thread::scope(|scope| {
            let start = &start;
            let installs = installers.map(|hook| {
                scope.spawn(move || {
                    start.wait();
                    hook.install()
                })
            });
            installs.map(|install| install.join().unwrap().unwrap())
        });

        assert!(matches!(
            changes,
            [
                HookOperationOutcome::Installed,
                HookOperationOutcome::AlreadyCurrent
            ] | [
                HookOperationOutcome::AlreadyCurrent,
                HookOperationOutcome::Installed
            ]
        ));
        assert_eq!(hook.state().unwrap(), HookState::Installed);
        assert_eq!(
            fs::read_to_string(&hook.cargo).unwrap(),
            hook.installed_source()
        );
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), CARGO_NAME);
        assert!(matches!(
            inspect_shim(&hook.real).unwrap(),
            CargoContents::Original
        ));
        assert!(!hook.staging.exists());
        assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());
    }

    #[test]
    fn the_installed_shim_is_executable() {
        let (_home, hook) = toolchain("real");
        hook.install().unwrap();

        let mode = fs::metadata(&hook.cargo).unwrap().permissions().mode();

        assert_eq!(mode & 0o777, SHIM_MODE);
    }

    /// Set the mtime far from the current time so even a filesystem with
    /// coarse timestamps exposes an unnecessary shim rewrite.
    fn mark_old_mtime(path: &Path) -> SystemTime {
        fs::File::open(path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();
        fs::metadata(path).unwrap().modified().unwrap()
    }

    /// Installing twice must preserve both the shim's mtime and the
    /// saved real cargo, even though the shim's timestamp is old.
    #[test]
    fn installing_twice_leaves_the_current_shim_and_real_cargo_alone() {
        let real = "\u{7f}ELF the one and only real cargo";
        let (_home, hook) = toolchain(real);
        hook.install().unwrap();
        let written = mark_old_mtime(&hook.cargo);

        assert_eq!(
            hook.install().unwrap(),
            HookOperationOutcome::AlreadyCurrent
        );
        assert_eq!(
            fs::metadata(&hook.cargo).unwrap().modified().unwrap(),
            written
        );
        assert_eq!(
            fs::read_to_string(&hook.cargo).unwrap(),
            hook.installed_source()
        );
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), real);
    }

    /// Size and mtime can remain unchanged while the installed bytes
    /// differ, so neither can establish that the shim is current.
    #[test]
    fn ensuring_refreshes_changed_contents_with_the_same_size_and_mtime() {
        let (_home, hook) = toolchain(CARGO_NAME);
        hook.install().unwrap();
        let written = mark_old_mtime(&hook.cargo);
        let length = fs::metadata(&hook.cargo).unwrap().len();
        let mut stale = hook.installed_bytes();
        stale.rotate_right(1);
        fs::write(&hook.cargo, &stale).unwrap();
        assert_eq!(mark_old_mtime(&hook.cargo), written);
        assert_eq!(fs::metadata(&hook.cargo).unwrap().len(), length);

        assert_eq!(hook.state().unwrap(), HookState::Installed);
        assert_eq!(hook.ensure().unwrap(), HookOperationOutcome::Refreshed);
        assert_eq!(
            fs::read_to_string(&hook.cargo).unwrap(),
            hook.installed_source()
        );
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), CARGO_NAME);
    }

    /// Rediscovery must retain a toolchain whose interrupted install
    /// left the real binary beside the missing `cargo`.
    fn interrupted_toolchain() -> (TempDir, Hook) {
        let (home, hook) = toolchain(HOOK_TEST_REAL_CARGO);
        fs::set_permissions(&hook.cargo, fs::Permissions::from_mode(SHIM_MODE)).unwrap();
        fs::rename(&hook.cargo, &hook.real).unwrap();
        let toolchain = hook.cargo.parent().unwrap().parent().unwrap();
        let hook = discovered_hook(toolchain);
        (home, hook)
    }

    /// A repaired install must execute the saved cargo through the shim
    /// and retain the saved binary's contents.
    fn assert_working_install(hook: &Hook) {
        assert_eq!(hook.state().unwrap(), HookState::Installed);
        assert_eq!(
            fs::read_to_string(&hook.cargo).unwrap(),
            hook.installed_source()
        );
        assert_eq!(
            fs::read_to_string(&hook.real).unwrap(),
            HOOK_TEST_REAL_CARGO
        );
        let output = Command::new(&hook.cargo)
            .arg(HOOK_TEST_VERSION_ARGUMENT)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            output.stdout,
            format!("{HOOK_TEST_VERSION_ARGUMENT}\n").as_bytes()
        );
        assert_eq!(output.stderr, [] as [u8; 0]);
    }

    /// Installation repairs the missing command without using the
    /// saved cargo's marker-bearing contents to classify it as a shim.
    #[test]
    fn installing_repairs_a_toolchain_with_only_the_saved_cargo() {
        let (_home, hook) = interrupted_toolchain();

        assert_eq!(hook.state().unwrap(), HookState::Repairable);
        assert_eq!(hook.install().unwrap(), HookOperationOutcome::Installed);
        assert_working_install(&hook);
        let written = mark_old_mtime(&hook.cargo);
        assert_eq!(
            hook.install().unwrap(),
            HookOperationOutcome::AlreadyCurrent
        );
        assert_eq!(
            fs::metadata(&hook.cargo).unwrap().modified().unwrap(),
            written
        );
    }

    /// The entry point shared by startup and the CLI repairs the same
    /// interrupted state and makes a repeated attempt a no-op.
    #[test]
    fn ensuring_repairs_a_toolchain_with_only_the_saved_cargo() {
        let (_home, hook) = interrupted_toolchain();

        assert_eq!(hook.state().unwrap(), HookState::Repairable);
        assert_eq!(hook.ensure().unwrap(), HookOperationOutcome::Installed);
        assert_working_install(&hook);
        let written = mark_old_mtime(&hook.cargo);
        assert_eq!(hook.ensure().unwrap(), HookOperationOutcome::AlreadyCurrent);
        assert_eq!(
            fs::metadata(&hook.cargo).unwrap().modified().unwrap(),
            written
        );
    }

    /// A failed staging write leaves a discoverable repair state, so a
    /// later attempt can complete after the write obstruction is gone.
    #[test]
    fn a_failed_shim_write_can_be_rediscovered_and_repaired() {
        let (_home, hook) = toolchain(CARGO_NAME);
        fs::create_dir(&hook.staging).unwrap();

        assert!(hook.install().is_err());
        assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());
        assert!(!hook.cargo.exists());
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), CARGO_NAME);
        let toolchain = hook.cargo.parent().unwrap().parent().unwrap();
        let hook = discovered_hook(toolchain);
        assert_eq!(hook.state().unwrap(), HookState::Repairable);

        fs::remove_dir(&hook.staging).unwrap();
        assert_eq!(hook.ensure().unwrap(), HookOperationOutcome::Installed);
        assert_eq!(hook.state().unwrap(), HookState::Installed);
        assert_eq!(
            fs::read_to_string(&hook.cargo).unwrap(),
            hook.installed_source()
        );
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), CARGO_NAME);
    }

    /// A repair failure must release its lock while leaving the saved
    /// cargo at its original path for the next attempt.
    #[test]
    fn a_failed_repair_releases_the_lock_and_preserves_saved_cargo() {
        let (_home, hook) = interrupted_toolchain();
        fs::create_dir(&hook.staging).unwrap();

        assert!(hook.install().is_err());
        assert!(!hook.cargo.exists());
        assert_eq!(
            fs::read_to_string(&hook.real).unwrap(),
            HOOK_TEST_REAL_CARGO
        );
        assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());

        fs::remove_dir(&hook.staging).unwrap();
        assert_eq!(hook.ensure().unwrap(), HookOperationOutcome::Installed);
        assert_working_install(&hook);
    }

    /// A remover must leave both installed and repairable toolchains
    /// untouched while another operation owns their installation lock.
    #[test]
    fn removing_with_a_held_lock_leaves_the_toolchain_untouched() {
        thread::scope(|scope| {
            let cases = [HookState::Installed, HookState::Repairable].map(|state| {
                scope.spawn(move || {
                    let (_home, hook) = interrupted_toolchain();
                    if state == HookState::Installed {
                        hook.install().unwrap();
                    }
                    let lock_path = hook.cargo.with_file_name(SHIM_LOCK_NAME);
                    let installation_lock =
                        HookInstallationLock::acquire(lock_path.clone()).unwrap();

                    assert_eq!(hook.remove().unwrap_err().kind(), ErrorKind::AlreadyExists);
                    assert_eq!(hook.state().unwrap(), state);
                    assert_eq!(
                        fs::read_to_string(&hook.real).unwrap(),
                        HOOK_TEST_REAL_CARGO
                    );
                    if state == HookState::Installed {
                        assert_eq!(
                            fs::read_to_string(&hook.cargo).unwrap(),
                            hook.installed_source()
                        );
                    } else {
                        assert!(!hook.cargo.exists());
                    }
                    assert!(!hook.staging.exists());
                    assert!(lock_path.exists());

                    drop(installation_lock);
                    assert_eq!(hook.remove().unwrap(), HookOperationOutcome::Removed);
                    assert_eq!(
                        fs::read_to_string(&hook.cargo).unwrap(),
                        HOOK_TEST_REAL_CARGO
                    );
                    assert!(!hook.real.exists());
                    assert!(!lock_path.exists());
                })
            });
            for case in cases {
                case.join().unwrap();
            }
        });
    }

    /// Starting from the interrupted-install state must preserve an
    /// executable cargo whichever competing operation acquires the lock first.
    #[test]
    fn concurrent_install_and_remove_preserve_a_working_cargo() {
        // The restored cargo must classify as Absent, so this fixture
        // omits the marker used by the saved-cargo repair tests.
        let real = HOOK_TEST_REAL_CARGO.replace(SHIM_MARKER, CARGO_NAME);
        let (_home, hook) = toolchain(&real);
        fs::set_permissions(&hook.cargo, fs::Permissions::from_mode(SHIM_MODE)).unwrap();
        fs::rename(&hook.cargo, &hook.real).unwrap();
        let operations = [Hook::install, Hook::remove];
        let start = Barrier::new(operations.len());

        let changes = thread::scope(|scope| {
            let start = &start;
            let hook = &hook;
            let attempts = operations.map(|operation| {
                scope.spawn(move || {
                    start.wait();
                    operation(hook)
                })
            });
            attempts.map(|attempt| attempt.join().unwrap().unwrap())
        });

        assert_eq!(
            changes,
            [
                HookOperationOutcome::Installed,
                HookOperationOutcome::Removed
            ]
        );
        assert!(matches!(
            hook.state().unwrap(),
            HookState::Installed | HookState::Absent
        ));
        if hook.real.exists() {
            assert_eq!(
                fs::read_to_string(&hook.cargo).unwrap(),
                hook.installed_source()
            );
            assert_eq!(fs::read_to_string(&hook.real).unwrap(), real);
        } else {
            assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), real);
        }
        let output = Command::new(&hook.cargo)
            .arg(HOOK_TEST_VERSION_ARGUMENT)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            output.stdout,
            format!("{HOOK_TEST_VERSION_ARGUMENT}\n").as_bytes()
        );
        assert_eq!(output.stderr, [] as [u8; 0]);
        assert!(!hook.staging.exists());
        assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());
    }

    /// Uninstalling must also restore a cargo left under its saved name
    /// by a failed install, without installing a shim first.
    #[test]
    fn removing_an_interrupted_install_restores_the_real_cargo() {
        let (_home, hook) = toolchain(CARGO_NAME);
        fs::rename(&hook.cargo, &hook.real).unwrap();

        assert_eq!(hook.remove().unwrap(), HookOperationOutcome::Removed);
        assert_eq!(hook.state().unwrap(), HookState::Absent);
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), CARGO_NAME);
        assert!(!hook.real.exists());
        assert_eq!(hook.remove().unwrap(), HookOperationOutcome::AlreadyAbsent);
    }

    /// What `rustup update` leaves behind: a fresh real cargo back on the
    /// name, with the previous real binary still beside it.
    #[test]
    fn a_toolchain_update_that_overwrote_the_shim_is_repaired_by_installing() {
        let (_home, hook) = toolchain("\u{7f}ELF old cargo");
        hook.install().unwrap();
        fs::write(&hook.cargo, "\u{7f}ELF cargo as rustup just reinstalled it").unwrap();

        assert_eq!(hook.state().unwrap(), HookState::Absent);
        assert_eq!(hook.install().unwrap(), HookOperationOutcome::Installed);
        assert_eq!(
            fs::read_to_string(&hook.real).unwrap(),
            "\u{7f}ELF cargo as rustup just reinstalled it"
        );
    }

    #[test]
    fn removing_gives_the_real_cargo_its_name_back() {
        for interrupted in [false, true] {
            let real = "\u{7f}ELF the one and only real cargo";
            let (_home, hook) = toolchain(real);
            hook.install().unwrap();

            if interrupted {
                fs::rename(&hook.real, &hook.cargo).unwrap();
                assert_eq!(hook.remove().unwrap(), HookOperationOutcome::AlreadyAbsent);
            } else {
                assert_eq!(hook.remove().unwrap(), HookOperationOutcome::Removed);
            }
            assert!(!hook.runner_path().exists());
            assert_eq!(hook.state().unwrap(), HookState::Absent);
            assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), real);
            assert!(!hook.real.exists());
        }
    }

    #[test]
    fn removing_what_was_never_installed_changes_nothing() {
        let (_home, hook) = toolchain("\u{7f}ELF real cargo");

        assert_eq!(hook.remove().unwrap(), HookOperationOutcome::AlreadyAbsent);
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

        assert_eq!(hook.state().unwrap(), HookState::Orphaned);
        assert_eq!(hook.install().unwrap(), HookOperationOutcome::Orphaned);
        assert_eq!(
            fs::read_to_string(&hook.cargo).unwrap(),
            hook.installed_source()
        );
        assert!(!hook.real.exists());
        assert_eq!(hook.remove().unwrap(), HookOperationOutcome::Orphaned);
    }

    /// The shim is renamed into place, never written there, so a `sh`
    /// that is part way through reading it keeps the file it opened.
    #[test]
    fn writing_the_shim_leaves_nothing_staged_beside_it() {
        let (_home, hook) = toolchain("\u{7f}ELF real cargo");
        hook.install().unwrap();
        hook.install().unwrap();

        assert!(!hook.staging.exists());
        assert_eq!(
            fs::read_to_string(&hook.cargo).unwrap(),
            hook.installed_source()
        );
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
            .map(|(name, _)| discovered_hook(&home.path().join(TOOLCHAINS_DIR).join(name)))
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
                .all(|hook| hook.state().unwrap() == HookState::Installed)
        );
    }

    /// Preserve the filesystem assertions from the three startup reader scenarios.
    #[test]
    fn startup_preserves_newer_and_truncated_shims_with_separate_failures() {
        let newer = CAPTURE_SHIM_VERSION + 1;
        let header = format!("#!/bin/sh\n# {SHIM_MARKER}\n{SHIM_VERSION_PREFIX}{newer}");
        for (contents, failures) in [
            (format!("{header}\n"), Vec::new()),
            (format!("{header}\n"), vec![("z-broken", CARGO_NAME)]),
            (header, Vec::new()),
        ] {
            let mut entries = vec![("a-newer", CARGO_NAME)];
            entries.extend(failures);
            let (_home, hooks) = toolchains(&entries);
            hooks[0].install().unwrap();
            fs::write(&hooks[0].cargo, &contents).unwrap();
            let paths = [&hooks[0].cargo, &hooks[0].real];
            for path in paths {
                fs::set_permissions(path, fs::Permissions::from_mode(0o751)).unwrap();
                mark_old_mtime(path);
            }
            let snapshot = || {
                paths.map(|path| {
                    let metadata = fs::metadata(path).unwrap();
                    (
                        fs::read(path).unwrap(),
                        metadata.ino(),
                        metadata.mode(),
                        metadata.modified().unwrap(),
                    )
                })
            };
            let before = snapshot();
            for hook in &hooks[1..] {
                fs::create_dir(hook.cargo.with_file_name(SHIM_LOCK_NAME)).unwrap();
            }

            let startup = stand_up(&hooks);

            if contents.ends_with('\n') {
                assert_eq!(
                    startup.kept_newer,
                    vec![NewerShim {
                        toolchain: "a-newer".into(),
                        installed: newer,
                        supported: CAPTURE_SHIM_VERSION,
                    }]
                );
                assert_eq!(startup.failed.len(), hooks.len() - 1);
                for (name, _) in &startup.failed {
                    assert_eq!(name, "z-broken");
                }
            } else {
                assert_eq!(startup.kept_newer, [] as [NewerShim; 0]);
                assert_eq!(
                    startup.failed,
                    vec![("a-newer".into(), "incomplete shim version line".into())]
                );
            }
            assert_eq!(startup.installed, [] as [String; 0]);
            assert_eq!(startup.refreshed, [] as [String; 0]);
            assert_eq!(snapshot(), before);
            assert!(!hooks[0].cargo.with_file_name(SHIM_LOCK_NAME).exists());
            assert!(!hooks[0].staging.exists());
            for hook in &hooks[1..] {
                assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), CARGO_NAME);
                assert!(hook.cargo.with_file_name(SHIM_LOCK_NAME).is_dir());
                assert!(!hook.real.exists());
                assert!(!hook.staging.exists());
            }
        }
    }

    /// Startup must include a repaired toolchain in its existing
    /// installed notice so the restored capture is visible.
    #[test]
    fn startup_reports_a_repaired_toolchain_as_installed() {
        let (_home, hook) = interrupted_toolchain();
        let name = hook.name.clone();
        let hooks = [hook];

        let startup = stand_up(&hooks);

        assert_eq!(
            startup,
            Startup {
                installed: vec![name],
                ..Startup::default()
            }
        );
        assert_working_install(&hooks[0]);
    }

    /// Every launch after the first finds this, and it must change
    /// nothing and say nothing.
    #[test]
    fn startup_over_a_current_shim_is_quiet() {
        let (_home, hooks) = toolchains(&[("stable-test", "\u{7f}ELF stable")]);
        stand_up(&hooks);
        let written = mark_old_mtime(&hooks[0].cargo);

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
        assert_eq!(
            fs::read_to_string(&hooks[0].cargo).unwrap(),
            hooks[0].installed_source()
        );
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
        assert_eq!(hooks[0].state().unwrap(), HookState::Orphaned);
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
        assert_eq!(hooks[0].state().unwrap(), HookState::Absent);
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

        assert!(matches!(Hook::at(&toolchain), HookDiscovery::AbsentCargo));
    }
    #[test]
    fn an_installed_runner_has_an_inode_separate_from_its_installer() {
        for stale_staging in [false, true] {
            let (_home, hook) = toolchain(CARGO_NAME);
            if stale_staging {
                fs::write(
                    hook.runner_path().with_extension("staging"),
                    b"interrupted runner",
                )
                .unwrap();
            }
            hook.install().unwrap();
            assert!(!hook.runner_path().with_extension("staging").exists());
            let installer = fs::metadata(native_fixture::executable().unwrap()).unwrap();
            let runner = fs::metadata(hook.cargo.with_file_name("cargo-tile-capture")).unwrap();
            assert_ne!(
                (runner.dev(), runner.ino()),
                (installer.dev(), installer.ino()),
                "temporary toolchain paths must not alias the installer's executable vnode"
            );
            let output = Command::new(hook.cargo.with_file_name("cargo-tile-capture"))
                .arg("--version")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(output.stdout.starts_with(b"cargo-tile "));
            assert!(output.stderr.is_empty(), "{output:?}");
        }
    }

    #[test]
    fn installing_repairs_a_runner_without_execute_permissions() {
        let (_home, hook) = toolchain(CARGO_NAME);
        hook.install().unwrap();
        let previous = fs::metadata(hook.cargo.with_file_name("cargo-tile-capture")).unwrap();
        fs::set_permissions(
            hook.cargo.with_file_name("cargo-tile-capture"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert_eq!(
            hook.install().unwrap(),
            HookOperationOutcome::AlreadyCurrent
        );
        let output = Command::new(hook.cargo.with_file_name("cargo-tile-capture"))
            .arg("--version")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.starts_with(b"cargo-tile "));
        assert!(output.stderr.is_empty(), "{output:?}");
        let repaired = fs::metadata(hook.cargo.with_file_name("cargo-tile-capture")).unwrap();
        assert_ne!(
            (previous.dev(), previous.ino()),
            (repaired.dev(), repaired.ino())
        );
        assert_eq!(repaired.mode() & 0o777, SHIM_MODE);
    }

    #[test]
    fn installing_repairs_runner_modes_that_differ_from_the_installed_mode() {
        assert_install_repairs_runner_mode(0o645);
    }

    #[test]
    fn installing_repairs_a_runner_missing_group_and_other_execution() {
        assert_install_repairs_runner_mode(0o744);
    }

    fn assert_install_repairs_runner_mode(mode: u32) {
        let (_home, hook) = toolchain(CARGO_NAME);
        hook.install().unwrap();
        let previous = fs::metadata(hook.cargo.with_file_name("cargo-tile-capture")).unwrap();
        fs::set_permissions(
            hook.cargo.with_file_name("cargo-tile-capture"),
            fs::Permissions::from_mode(mode),
        )
        .unwrap();
        assert_eq!(
            hook.install().unwrap(),
            HookOperationOutcome::AlreadyCurrent
        );
        let repaired = fs::metadata(hook.cargo.with_file_name("cargo-tile-capture")).unwrap();
        assert_eq!(repaired.mode() & 0o7777, SHIM_MODE, "repair {mode:o}");
        assert_ne!(
            (previous.dev(), previous.ino()),
            (repaired.dev(), repaired.ino())
        );
        let output = Command::new(hook.cargo.with_file_name("cargo-tile-capture"))
            .arg("--version")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{mode:o}: {output:?}");
        assert_eq!(
            output.stdout,
            format!("cargo-tile {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
        );
        assert_eq!(output.stderr, b"");
    }

    #[test]
    fn a_foreign_runner_refuses_install_before_cargo_moves() {
        let (_home, hook) = toolchain("original cargo");
        fs::write(
            hook.cargo.with_file_name("cargo-tile-capture"),
            b"foreign runner",
        )
        .unwrap();
        assert_eq!(hook.install().unwrap_err().kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&hook.cargo).unwrap(), b"original cargo");
        assert!(!hook.real.exists());
        assert_eq!(
            fs::read(hook.cargo.with_file_name("cargo-tile-capture")).unwrap(),
            b"foreign runner"
        );
    }

    #[test]
    fn uninstall_keeps_a_foreign_runner_beside_a_version_three_shim() {
        let (_home, hook) = toolchain(CARGO_NAME);
        fs::rename(&hook.cargo, &hook.real).unwrap();
        fs::write(
            &hook.cargo,
            "#!/bin/sh\n# cargo-tile-capture-shim\n# cargo-tile-shim-version: 3\n",
        )
        .unwrap();
        fs::write(
            hook.cargo.with_file_name("cargo-tile-capture"),
            b"foreign runner",
        )
        .unwrap();
        assert_eq!(hook.remove().unwrap(), HookOperationOutcome::Removed);
        assert_eq!(fs::read(&hook.cargo).unwrap(), CARGO_NAME.as_bytes());
        assert_eq!(
            fs::read(hook.cargo.with_file_name("cargo-tile-capture")).unwrap(),
            b"foreign runner"
        );
    }

    #[test]
    fn installing_version_four_updates_a_version_three_shim() {
        let (_home, hook) = toolchain(CARGO_NAME);
        hook.install().unwrap();
        fs::write(
            &hook.cargo,
            "#!/bin/sh\n# cargo-tile-capture-shim\n# cargo-tile-shim-version: 3\n",
        )
        .unwrap();
        assert_eq!(hook.ensure().unwrap(), HookOperationOutcome::Refreshed);
        assert_eq!(
            fs::read_to_string(&hook.cargo).unwrap(),
            hook.installed_source()
        );
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), CARGO_NAME);
        assert!(matches!(
            shim_version(&fs::read(&hook.cargo).unwrap()).unwrap(),
            ShimVersion::Versioned(4)
        ));
    }

    /// Execute both entry paths and check their protocol arguments.
    #[test]
    fn the_capture_script_uses_the_native_entry_protocol_words() {
        use crate::constants::CAPTURE_ROOT;
        use crate::constants::CAPTURE_RUNNER_NAME;
        use crate::constants::CAPTURE_SHIM_ARGUMENT;
        use crate::constants::CAPTURE_SHIM_PASSTHROUGH_ARGUMENT;

        let (_home, hook) = toolchain("#!/bin/sh\nprintf original >&2\nexit 37\n");
        fs::set_permissions(&hook.cargo, fs::Permissions::from_mode(SHIM_MODE)).unwrap();
        hook.install().unwrap();
        let runner = hook.cargo.with_file_name(CAPTURE_RUNNER_NAME);
        fs::write(&runner, "#!/bin/sh\nprintf '%s\\000' \"$@\" >&2\nexit 43\n").unwrap();
        fs::set_permissions(&runner, fs::Permissions::from_mode(SHIM_MODE)).unwrap();
        for (command, flag) in [
            ("exec \"$@\"", CAPTURE_SHIM_ARGUMENT),
            ("exec \"$@\" 1>&-", CAPTURE_SHIM_PASSTHROUGH_ARGUMENT),
        ] {
            let output = Command::new("/bin/sh")
                .args(["-c", command, "entry-protocol"])
                .arg(&hook.cargo)
                .args(["build", "a b", ""])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(43), "{output:?}");
            let expected: Vec<u8> = [
                OsStr::new(flag),
                hook.cargo.as_os_str(),
                OsStr::new(CAPTURE_ROOT),
                OsStr::new("build"),
                OsStr::new("a b"),
                OsStr::new(""),
            ]
            .into_iter()
            .flat_map(|word| word.as_bytes().iter().copied().chain([0]))
            .collect();
            assert_eq!(output.stderr, expected);
            assert_eq!(output.stdout, b"");
        }
    }

    #[test]
    fn a_replaced_shared_parent_invalidates_the_setup_receipt() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("capture");
        CaptureDirectories::prepare(&parent, "boot-a").unwrap();
        let prior = directory.path().join("prior");
        fs::rename(&parent, &prior).unwrap();
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        for entry in fs::read_dir(&prior).unwrap() {
            let entry = entry.unwrap();
            fs::rename(entry.path(), parent.join(entry.file_name())).unwrap();
        }
        CaptureDirectories::prepare(&parent, "boot-a").unwrap();
        assert_eq!(
            fs::metadata(&parent).unwrap().permissions().mode() & 0o7777,
            0o1777
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_non_utf8_install_path_executes_the_installed_runner() {
        let home = tempdir().unwrap();
        let toolchain = home.path().join(OsStr::from_bytes(b"toolchain-\xff-'"));
        let binaries = toolchain.join("bin");
        fs::create_dir_all(&binaries).unwrap();
        fs::write(
            binaries.join("cargo"),
            b"#!/bin/sh\nprintf 'cargo version\n'\nexit 37\n",
        )
        .unwrap();
        fs::set_permissions(binaries.join("cargo"), fs::Permissions::from_mode(0o755)).unwrap();
        let hook = discovered_hook(&toolchain);
        hook.install().unwrap();
        let output = Command::new(&hook.cargo).arg("--version").output().unwrap();
        assert_eq!(output.status.code(), Some(37), "{output:?}");
        assert_eq!(output.stdout, b"cargo version\n");
    }

    #[test]
    fn a_non_utf8_runner_path_is_preserved_in_the_installed_entry() {
        let path = Path::new(OsStr::from_bytes(b"/toolchain-\xff-'/cargo-tile-capture"));
        let source = capture_shell_source(path);
        let entry = source
            .split(|byte| *byte == b'\n')
            .find(|line| line.starts_with(b"exec "))
            .unwrap();
        let mut script = b"set -- ".to_vec();
        script.extend_from_slice(&entry[b"exec ".len()..]);
        script.extend_from_slice(b"\nprintf '%s' \"$1\"\n");
        let output = Command::new("sh")
            .arg("-c")
            .arg(OsStr::from_bytes(&script))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, path.as_os_str().as_bytes());
    }

    #[test]
    fn the_linux_pty_command_passes_non_utf8_arguments_to_cargo() {
        let home = tempdir().unwrap();
        let executable = home.path().join(OsStr::from_bytes(b"cargo-'"));
        fs::write(&executable, b"#!/bin/sh\nprintf '%s' \"$1\"\nprintf '%s\\000%s\\000%s' \"$PWD\" \"${HOME-unset}\" \"$CAPTURE_CONTEXT_VALUE\" > \"$CAPTURE_CONTEXT_FILE\"\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let argument = OsStr::from_bytes(b"argument-\xff-' with space");
        let mut cargo = Command::new(&executable);
        cargo
            .arg(argument)
            .env_remove("HOME")
            .env("CAPTURE_CONTEXT_VALUE", "retained value")
            .env("CAPTURE_CONTEXT_FILE", home.path().join("context"))
            .current_dir(home.path());
        let script = CaptureScript::UtilLinux.command(&cargo, &home.path().join("capture.log"));
        let command = script.get_args().nth(4).unwrap();
        let mut shell = Command::new("sh");
        copy_command_context(&script, &mut shell);
        let output = shell.arg("-c").arg(command).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, argument.as_bytes());
        let mut expected = home
            .path()
            .canonicalize()
            .unwrap()
            .as_os_str()
            .as_bytes()
            .to_vec();
        expected.extend_from_slice(b"\0unset\0retained value");
        assert_eq!(fs::read(home.path().join("context")).unwrap(), expected);
    }

    #[test]
    fn native_publication_preserves_a_directory_that_wins_the_registration_name() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("capture");
        let mut collision = PathBuf::new();
        let result = CapturePublication::prepare_at_publication(
            &parent,
            &["build".into()],
            |staging, registration| {
                assert_eq!(fs::metadata(staging)?.permissions().mode() & 0o777, 0o644);
                fs::create_dir(registration)?;
                fs::write(registration.join("keep"), b"preserved")?;
                collision = registration.to_owned();
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(collision.join("keep")).unwrap(), b"preserved");
        assert_eq!(fs::read_dir(collision).unwrap().count(), 1);
    }

    #[test]
    fn native_publication_keeps_complete_readable_staging_after_a_boundary_sweep() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("capture");
        let mut staged_bytes = Vec::new();
        let publication = CapturePublication::prepare_at_publication(
            &parent,
            &["build".into(), "a b".into(), "".into()],
            |staging, registration| {
                assert_eq!(fs::metadata(staging)?.permissions().mode() & 0o7777, 0o644);
                staged_bytes = fs::read(staging)?;
                assert!(!registration.exists());
                // A sweep at this boundary sees no published log to preserve.
                let root = registration
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap();
                for entry in fs::read_dir(root)? {
                    let entry = entry?;
                    if entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "log")
                    {
                        fs::remove_file(entry.path())?;
                    }
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(fs::read(&publication.registration.0).unwrap(), staged_bytes);
        assert_eq!(
            fs::metadata(&publication.log.0)
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o644
        );
    }

    #[test]
    fn native_setup_reuses_its_boot_receipt_and_rechecks_replaced_directories() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("capture");
        let first = CaptureDirectories::prepare(&parent, "boot-a").unwrap();
        let receipt = parent.join(format!(".setup-v4-{}", rustix::process::getuid().as_raw()));
        let past = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1);
        fs::File::options()
            .write(true)
            .open(&receipt)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(past))
            .unwrap();
        let written = fs::metadata(&receipt).unwrap().modified().unwrap();
        CaptureDirectories::prepare(&parent, "boot-a").unwrap();
        assert_eq!(fs::metadata(&receipt).unwrap().modified().unwrap(), written);
        let old = fs::read(&receipt).unwrap();
        CaptureDirectories::prepare(&parent, "boot-b").unwrap();
        assert_ne!(fs::read(&receipt).unwrap(), old);
        fs::remove_dir(&first.pids).unwrap();
        std::os::unix::fs::symlink(directory.path(), &first.pids).unwrap();
        assert!(CaptureDirectories::prepare(&parent, "boot-b").is_err());
    }

    #[test]
    fn the_installed_shim_skips_every_valued_global_option() {
        let captured_queries: Vec<_> = [
            ("--color", "auto"),
            ("-Z", "unstable-options"),
            ("-C", "workspace"),
        ]
        .into_iter()
        .filter(|(option, value)| {
            capture_requested(&[(*option).into(), (*value).into(), "metadata".into()])
        })
        .collect();
        assert!(
            captured_queries.is_empty(),
            "query commands requested capture after: {captured_queries:?}"
        );
        for (option, scope) in COMMAND_VALUED_OPTIONS {
            if scope.accepted_before_subcommand() {
                assert!(!capture_requested(&[
                    (*option).into(),
                    "build".into(),
                    "metadata".into()
                ]));
                assert!(capture_requested(&[
                    (*option).into(),
                    "metadata".into(),
                    "build".into()
                ]));
            }
        }
    }
}
