//! Fresh metadata for processes selected by the cargo census.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;

use sysinfo::Pid;
use sysinfo::System;

use super::DetailPresence;
use super::LinuxDiscoveryStat;
use super::ProcessDiscovery;
use super::ProcessField;
use super::ProcessObservation;
use super::ProcessObservations;
use super::command_text;
use super::is_transparent;
use crate::birth_stamp::LifetimeEvidence;
use crate::birth_stamp::ProcessLifetime;
use crate::census::invocation_cpu_accounting::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::constants::CARGO_TARGET_DIR_ENV;

/// Precise birth and a fresh image observation bracket every metadata read.
/// No metadata survives a scan, including failed rereads or an exec of the same image.
#[derive(Debug, Eq, PartialEq)]
struct ProcessImage {
    pid:        Pid,
    birth:      ProcessBirth,
    name:       OsString,
    executable: ExecutableLink,
}

/// One birth owner retains precise local evidence even without a boot identity.
#[derive(Debug, Eq, PartialEq)]
enum ProcessBirth {
    /// A boot id and precise ticks establish the process lifetime.
    Proven(ProcessLifetime),
    /// These ticks bracket this fresh read but prove no cross-scan identity.
    BootUnavailable { start_ticks: u64 },
}

/// Exe denial does not discard independently readable process fields.
#[derive(Debug, Eq, PartialEq)]
enum ExecutableLink {
    /// Keep the readable path even if the backing inode cannot be inspected.
    Read {
        path:  PathBuf,
        inode: ProcessField<(u64, u64)>,
    },
    /// Main's argv/kernel-name fallback applies when the link is unreadable.
    Unreadable,
}

impl ExecutableLink {
    fn read(directory: &Path) -> Self {
        let link = directory.join("exe");
        let Ok(mut path) = fs::read_link(&link) else {
            return Self::Unreadable;
        };
        let inode = fs::metadata(&link)
            .ok()
            .map(|metadata| (metadata.dev(), metadata.ino()))
            .into();
        if let Some(name) = path.file_name()
            && let Some(name) = name.as_bytes().strip_suffix(b" (deleted)")
        {
            let name = OsString::from_vec(name.to_vec());
            path.set_file_name(name);
        }
        Self::Read { path, inode }
    }

    fn observed(&self) -> ProcessField<&Path> {
        match self {
            Self::Read { path, .. } => ProcessField::Observed(path.as_path()),
            Self::Unreadable => ProcessField::Unavailable,
        }
    }
}

/// Only the constructor that checks both image observations can publish this record.
struct CurrentProcessDetail {
    image:       ProcessImage,
    parent:      ProcessField<Pid>,
    memory:      u64,
    argv:        Vec<OsString>,
    directory:   ProcessField<PathBuf>,
    environment: Vec<OsString>,
    uid:         ProcessField<u32>,
    started:     u64,
    elapsed:     u64,
}

/// One owner for one scan's metadata; all census consumers borrow it.
/// Deliberately no retained process table or metadata cache between scans.
pub(super) struct CurrentProcessDetails(Vec<CurrentProcessDetail>);

impl CurrentProcessDetails {
    pub(super) fn read(pids: &[Pid], discovery: &ProcessDiscovery) -> Self {
        Self::read_at(Path::new("/proc"), pids, discovery)
    }

    fn read_at(root: &Path, pids: &[Pid], discovery: &ProcessDiscovery) -> Self {
        Self::read_with_lifetime(root, pids, discovery, ProcessLifetime::linux)
    }

    fn read_with_lifetime(
        root: &Path,
        pids: &[Pid],
        discovery: &ProcessDiscovery,
        lifetime: impl Fn(u64) -> LifetimeEvidence,
    ) -> Self {
        let mut reader = DetailReader::default();
        let boot = System::boot_time();
        let uptime = System::uptime();
        Self(
            pids.iter()
                .filter_map(|&pid| {
                    let directory = root.join(pid.to_string());
                    let (image, stat) = reader
                        .image_with_lifetime(&directory, pid, &lifetime)
                        .ok()?;
                    if let Some(baseline) = discovery.baselines.get(&pid)
                        && let LifetimeEvidence::Available(discovered) = &baseline.lifetime
                        && let ProcessBirth::Proven(current) = &image.birth
                        && discovered != current
                    {
                        return None;
                    }
                    let argv = reader.strings(&directory.join("cmdline"));
                    let environment = reader.target_environment(&directory.join("environ"));
                    let uid = reader.uid(&directory.join("status"));
                    let cwd = fs::read_link(directory.join("cwd")).ok().into();
                    let (after, _) = reader
                        .image_with_lifetime(&directory, pid, &lifetime)
                        .ok()?;
                    if image != after {
                        return None;
                    }
                    let started = stat
                        .start_ticks
                        .checked_div(rustix::param::clock_ticks_per_second())?;
                    Some(CurrentProcessDetail {
                        image,
                        parent: stat.parent,
                        memory: stat
                            .resident_pages
                            .saturating_mul(rustix::param::page_size() as u64),
                        argv,
                        directory: cwd,
                        environment,
                        uid,
                        started: boot.saturating_add(started),
                        elapsed: uptime.saturating_sub(started),
                    })
                })
                .collect(),
        )
    }

    pub(super) fn overlay<'scan>(&'scan self, observations: &mut ProcessObservations<'scan>) {
        for detail in &self.0 {
            observations.overlay_record(ProcessObservation {
                pid:         detail.image.pid,
                parent:      detail.parent,
                name:        ProcessField::Observed(&detail.image.name),
                argv:        (!detail.argv.is_empty())
                    .then_some(detail.argv.as_slice())
                    .into(),
                directory:   match &detail.directory {
                    ProcessField::Observed(directory) => {
                        ProcessField::Observed(directory.as_path())
                    },
                    ProcessField::Unavailable => ProcessField::Unavailable,
                },
                executable:  detail.image.executable.observed(),
                uid:         detail.uid,
                environment: &detail.environment,
                lifetime:    Cow::Borrowed(&LifetimeEvidence::Unavailable),
                cpu:         Measurement::Unavailable(MeasurementAbsence::Unproven),
                memory:      detail.memory,
                accumulated: 0,
                native_cpu:  std::cell::Cell::new(Measurement::Unavailable(
                    MeasurementAbsence::ReadFailed,
                )),
                started:     detail.started,
                elapsed:     detail.elapsed,
                details:     DetailPresence::Observed,
            });
        }
    }
}

/// Reusable capacity contains only scratch bytes; every read clears the previous input.
#[derive(Default)]
struct DetailReader {
    bytes: Vec<u8>,
}

impl DetailReader {
    fn read(&mut self, path: &Path) -> std::io::Result<&[u8]> {
        self.bytes.clear();
        fs::File::open(path)?.read_to_end(&mut self.bytes)?;
        Ok(&self.bytes)
    }

    fn image_with_lifetime(
        &mut self,
        directory: &Path,
        pid: Pid,
        lifetime: &impl Fn(u64) -> LifetimeEvidence,
    ) -> Result<(ProcessImage, LinuxDiscoveryStat), ()> {
        let stat = LinuxDiscoveryStat::parse(self.read(&directory.join("stat")).map_err(|_| ())?)?;
        if stat.pid != pid {
            return Err(());
        }
        let birth = match lifetime(stat.start_ticks) {
            LifetimeEvidence::Available(lifetime) => ProcessBirth::Proven(lifetime),
            LifetimeEvidence::Unavailable => ProcessBirth::BootUnavailable {
                start_ticks: stat.start_ticks,
            },
        };
        let executable = ExecutableLink::read(directory);
        Ok((
            ProcessImage {
                pid,
                birth,
                name: stat.name.clone(),
                executable,
            },
            stat,
        ))
    }

    fn strings(&mut self, path: &Path) -> Vec<OsString> {
        self.read(path).map_or_else(
            |_| Vec::new(),
            |bytes| strings(bytes).map(OsStr::to_os_string).collect(),
        )
    }

    /// `CARGO_TARGET_DIR` is the sole environment field used by census attribution.
    /// Read fresh bytes, but do not allocate strings for unrelated environment entries.
    fn target_environment(&mut self, path: &Path) -> Vec<OsString> {
        self.read(path).map_or_else(
            |_| Vec::new(),
            |bytes| {
                strings(bytes)
                    .filter(|entry| {
                        entry
                            .as_bytes()
                            .strip_prefix(CARGO_TARGET_DIR_ENV.as_bytes())
                            .is_some_and(|suffix| suffix.starts_with(b"="))
                    })
                    .map(OsStr::to_os_string)
                    .collect()
            },
        )
    }

    fn uid(&mut self, path: &Path) -> ProcessField<u32> {
        self.read(path)
            .ok()
            .and_then(|bytes| {
                let line = bytes
                    .split(|byte| *byte == b'\n')
                    .find_map(|line| line.strip_prefix(b"Uid:"))?;
                let value = line
                    .split(u8::is_ascii_whitespace)
                    .find(|field| !field.is_empty())?;
                std::str::from_utf8(value).ok()?.parse().ok()
            })
            .into()
    }
}

fn strings(bytes: &[u8]) -> impl Iterator<Item = &OsStr> {
    let mut remaining = bytes;
    std::iter::from_fn(move || {
        loop {
            if remaining.is_empty() {
                return None;
            }
            let end = remaining
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(remaining.len());
            let entry = remaining[..end].trim_ascii();
            remaining = remaining.get(end + 1..).unwrap_or_default();
            if !entry.is_empty() {
                return Some(OsStr::from_bytes(entry));
            }
        }
    })
}

/// Only a recognized launcher's argv can establish cargo membership.
/// Arbitrary names need no argv probe; this reads no cwd, exe, environment or uid.
pub(super) fn candidate_arguments(directory: &Path, name: &OsStr) -> Vec<OsString> {
    if !is_transparent(name) {
        return Vec::new();
    }
    let mut reader = DetailReader::default();
    let argv = reader.strings(&directory.join("cmdline"));
    if command_text::cargo_split(&argv)
        .is_ok_and(|arguments| arguments.start == 2 && arguments.follows_cargo_executable())
    {
        argv
    } else {
        Vec::new()
    }
}
