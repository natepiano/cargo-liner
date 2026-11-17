//! Slot ownership comes from the live lock, independently of cached lease contents.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::fs::TryLockError;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use crate::config::CensusConfig;
use crate::config::Config;
use crate::constants::BUILD_FOLDERS_CONFIG;
use crate::constants::ENV_VARIABLE_CLOSE;
use crate::constants::ENV_VARIABLE_OPEN;
use crate::constants::ENV_VARIABLE_PREFIX;
use crate::constants::ENV_VARIABLE_UNDERSCORE;
use crate::constants::HOME_ALIAS;
use crate::constants::HOST_DOMAIN_SEPARATOR;
use crate::constants::SLOT_CARGO_ROOT_KEY;
use crate::constants::SLOT_FREE_LABEL;
use crate::constants::SLOT_LABEL_SEPARATOR;
use crate::constants::SLOT_LEASE_SUFFIX;
use crate::constants::SLOT_LOCK_SUFFIX;
use crate::constants::SLOT_ROOT_KEY;
use crate::constants::SLOT_UNREADABLE_LABEL;

/// Current slot ownership, including observations that cannot name a holder.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum SlotObservation {
    /// None of the directories belongs to a configured slot root.
    #[default]
    Outside,
    /// No process holds the slot lock.
    Free { slot: String },
    /// A process holds the lock and the current lease names its work.
    Held {
        slot:    String,
        holder:  String,
        purpose: String,
    },
    /// The lock or lease cannot establish the current holder.
    Unreadable { slot: String },
}

impl SlotObservation {
    /// Replace display text without changing the directory used for grouping.
    pub(crate) fn display_path(&self, original: &str) -> String {
        match self {
            Self::Outside => original.to_owned(),
            Self::Free { slot } => format!("{slot}{SLOT_LABEL_SEPARATOR}{SLOT_FREE_LABEL}"),
            Self::Held {
                slot,
                holder,
                purpose,
            } => format!("{slot}{SLOT_LABEL_SEPARATOR}{holder}{SLOT_LABEL_SEPARATOR}{purpose}"),
            Self::Unreadable { slot } => {
                format!("{slot}{SLOT_LABEL_SEPARATOR}{SLOT_UNREADABLE_LABEL}")
            },
        }
    }
}

/// File identity detects replacements and writes, including equal-sized writes.
#[derive(Debug, Default, Eq, PartialEq)]
enum FileRevision {
    #[default]
    Unavailable,
    Present {
        device:   u64,
        inode:    u64,
        length:   u64,
        modified: (i64, i64),
        changed:  (i64, i64),
    },
}

impl FileRevision {
    fn read(path: &Path) -> Self {
        fs::metadata(path).map_or(Self::Unavailable, |metadata| Self::Present {
            device:   metadata.dev(),
            inode:    metadata.ino(),
            length:   metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed:  (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

/// This revision has no readable, parsed file contents.
struct FileRevisionFailure;

/// `RevisionCachedFile` retains parsed contents or failure until the file revision changes.
struct RevisionCachedFile<T> {
    revision: FileRevision,
    contents: Result<T, FileRevisionFailure>,
}

impl<T> Default for RevisionCachedFile<T> {
    fn default() -> Self {
        Self {
            revision: FileRevision::Unavailable,
            contents: Err(FileRevisionFailure),
        }
    }
}

impl<T> RevisionCachedFile<T> {
    fn read(&mut self, path: &Path, parse: impl FnOnce(&str) -> Result<T, ()>) -> Result<&T, ()> {
        let revision = FileRevision::read(path);
        if revision != self.revision {
            self.contents = fs::read_to_string(path)
                .map_err(|_| ())
                .and_then(|text| parse(&text))
                .map_err(|()| FileRevisionFailure);
            self.revision = revision;
        }
        self.contents.as_ref().map_err(|_| ())
    }
}

/// Root values stay in the configuration file, including host overrides.
#[derive(Default)]
struct SlotRoots {
    values: HashMap<String, String>,
}

impl From<&str> for SlotRoots {
    fn from(text: &str) -> Self {
        let values = text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.starts_with('#') {
                    return None;
                }
                let (key, value) = line.split_once('=')?;
                Some((key.trim().to_owned(), value.trim().to_owned()))
            })
            .collect();
        Self { values }
    }
}

impl SlotRoots {
    fn root(&self, key: &str, machine: &str, home: &Path) -> Result<PathBuf, ()> {
        let value = self
            .values
            .get(&format!("{key}.{machine}"))
            .or_else(|| self.values.get(key))
            .filter(|value| !value.is_empty())
            .ok_or(())?;
        resolve_root(&expand_home(&expand_variables(value), home)).map_err(|_| ())
    }
}

/// Missing checkout configuration disables slot matching for the complete scan.
enum ConfiguredSlotRoots {
    Disabled,
    Enabled {
        checkouts: PathBuf,
        cargo:     Result<PathBuf, ()>,
    },
}

/// Staff names load only when the first held slot needs them in this scan.
#[derive(Default)]
enum ScanStaffNames {
    #[default]
    Unread,
    Unreadable,
    Loaded(HashMap<String, String>),
}

#[derive(Deserialize)]
struct LeaseRecord {
    unit:    String,
    purpose: String,
}

#[derive(Default, Deserialize)]
struct Staff {
    rows: Vec<StaffRow>,
}

#[derive(Deserialize)]
struct StaffRow {
    id:   String,
    name: String,
}

/// `SlotLeaseReader` retains parsed files until their filesystem revisions change.
#[derive(Default)]
pub(super) struct SlotLeaseReader {
    roots:    RevisionCachedFile<SlotRoots>,
    settings: RevisionCachedFile<Config>,
    staff:    HashMap<PathBuf, RevisionCachedFile<Staff>>,
    leases:   HashMap<PathBuf, RevisionCachedFile<LeaseRecord>>,
}

/// One scan shares root resolution, staff names and each slot's live observation.
pub(super) struct SlotLeaseScan<'a> {
    reader:       &'a mut SlotLeaseReader,
    roots:        ConfiguredSlotRoots,
    home:         PathBuf,
    config:       PathBuf,
    staff:        ScanStaffNames,
    observations: HashMap<String, SlotObservation>,
}

impl SlotLeaseReader {
    /// Resolve roots once before processing the scan's invocation rows.
    pub(in crate::census) fn begin_scan(
        &mut self,
        home: &Path,
        machine: &str,
        config: &Path,
    ) -> SlotLeaseScan<'_> {
        let machine = machine
            .split(HOST_DOMAIN_SEPARATOR)
            .next()
            .unwrap_or(machine);
        let roots = self.roots.read(&home.join(BUILD_FOLDERS_CONFIG), |text| {
            Ok(SlotRoots::from(text))
        });
        let roots = match roots {
            Ok(roots) => match roots.root(SLOT_ROOT_KEY, machine, home) {
                Ok(checkouts) => ConfiguredSlotRoots::Enabled {
                    checkouts,
                    cargo: roots.root(SLOT_CARGO_ROOT_KEY, machine, home),
                },
                Err(()) => ConfiguredSlotRoots::Disabled,
            },
            Err(()) => ConfiguredSlotRoots::Disabled,
        };
        SlotLeaseScan {
            reader: self,
            roots,
            home: home.to_owned(),
            config: config.to_owned(),
            staff: ScanStaffNames::Unread,
            observations: HashMap::new(),
        }
    }

    #[cfg(test)]
    fn observe_in<'a>(
        &mut self,
        paths: impl IntoIterator<Item = &'a Path>,
        home: &Path,
        machine: &str,
        config: &Path,
    ) -> SlotObservation {
        self.begin_scan(home, machine, config).observe(paths)
    }
}

impl SlotLeaseScan<'_> {
    /// Probe each distinct slot once; reuse its observation for the remaining rows.
    pub(super) fn observe<'a>(
        &mut self,
        paths: impl IntoIterator<Item = &'a Path>,
    ) -> SlotObservation {
        let ConfiguredSlotRoots::Enabled { checkouts, cargo } = &self.roots else {
            return SlotObservation::Outside;
        };
        let candidates: Vec<_> = paths
            .into_iter()
            .flat_map(|path| {
                std::iter::once(checkouts)
                    .chain(cargo.as_ref().ok())
                    .filter_map(move |root| {
                        let relative = path.strip_prefix(root).ok()?;
                        let Component::Normal(slot) = relative.components().next()? else {
                            return None;
                        };
                        if relative
                            .components()
                            .any(|part| matches!(part, Component::ParentDir))
                        {
                            return None;
                        }
                        Some(slot.to_string_lossy().into_owned())
                    })
            })
            .collect();
        if candidates.is_empty() {
            return SlotObservation::Outside;
        }
        let checkouts = checkouts.clone();
        for slot in candidates {
            let observed = if let Some(observed) = self.observations.get(&slot) {
                observed.clone()
            } else {
                let observed = self.read_slot(&slot, &checkouts);
                self.observations.insert(slot, observed.clone());
                observed
            };
            if observed != SlotObservation::Outside {
                return observed;
            }
        }
        SlotObservation::Outside
    }

    fn read_slot(&mut self, name: &str, checkouts: &Path) -> SlotObservation {
        let slot = name.to_owned();
        let lock_path = checkouts.join(format!("{slot}{SLOT_LOCK_SUFFIX}"));
        let lock = match File::open(&lock_path) {
            Ok(lock) => lock,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return if checkouts.join(&slot).is_dir()
                    || checkouts
                        .join(format!("{slot}{SLOT_LEASE_SUFFIX}"))
                        .is_file()
                {
                    SlotObservation::Free { slot }
                } else {
                    SlotObservation::Outside
                };
            },
            Err(_) => return SlotObservation::Unreadable { slot },
        };
        match lock.try_lock() {
            Ok(()) => return SlotObservation::Free { slot },
            Err(TryLockError::WouldBlock) => {},
            Err(TryLockError::Error(_)) => return SlotObservation::Unreadable { slot },
        }
        if !lock
            .metadata()
            .ok()
            .zip(fs::metadata(&lock_path).ok())
            .is_some_and(|(opened, named)| {
                opened.dev() == named.dev() && opened.ino() == named.ino()
            })
        {
            return SlotObservation::Unreadable { slot };
        }
        let lease_path = checkouts.join(format!("{slot}{SLOT_LEASE_SUFFIX}"));
        let Ok(record) = self
            .reader
            .leases
            .entry(lease_path.clone())
            .or_default()
            .read(&lease_path, |text| {
                serde_json::from_str(text).map_err(|_| ())
            })
        else {
            return SlotObservation::Unreadable { slot };
        };
        let unit = record.unit.clone();
        let purpose = record.purpose.clone();
        if matches!(self.staff, ScanStaffNames::Unread) {
            let staff_path = self
                .reader
                .settings
                .read(&self.config, |text| toml::from_str(text).map_err(|_| ()))
                .map_or_else(
                    |()| CensusConfig::default().staff_file,
                    |config| config.census.staff_file.clone(),
                );
            let staff_path = expand_home(&staff_path, &self.home);
            self.staff = self
                .reader
                .staff
                .entry(staff_path.clone())
                .or_default()
                .read(&staff_path, |text| {
                    serde_json::from_str(text).map_err(|_| ())
                })
                .map_or(ScanStaffNames::Unreadable, |staff| {
                    ScanStaffNames::Loaded(
                        staff
                            .rows
                            .iter()
                            .map(|row| (row.id.clone(), row.name.clone()))
                            .collect(),
                    )
                });
        }
        let holder = match &self.staff {
            ScanStaffNames::Loaded(names) => names.get(&unit).cloned().unwrap_or(unit),
            ScanStaffNames::Unread | ScanStaffNames::Unreadable => unit,
        };
        SlotObservation::Held {
            slot,
            holder,
            purpose,
        }
    }
}

/// Resolve existing ancestors before adding components that do not exist yet.
fn resolve_root(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_relative() {
        return resolve_root(&std::env::current_dir()?.join(path));
    }
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let parent = path.parent().ok_or(error)?;
            let mut resolved = resolve_root(parent)?;
            match path.components().next_back() {
                Some(Component::ParentDir) => {
                    resolved.pop();
                },
                Some(Component::Normal(name)) => resolved.push(name),
                _ => {},
            }
            Ok(resolved)
        },
        Err(error) => Err(error),
    }
}

/// Expand shell variable names without invoking a shell or changing unknown names.
fn expand_variables(value: &str) -> String {
    let mut result = String::new();
    let mut characters = value.char_indices().peekable();
    while let Some((start, character)) = characters.next() {
        if character != ENV_VARIABLE_PREFIX {
            result.push(character);
            continue;
        }
        let braced = characters
            .peek()
            .is_some_and(|(_, next)| *next == ENV_VARIABLE_OPEN);
        if braced {
            characters.next();
        }
        let mut name = String::new();
        let mut end = start + character.len_utf8();
        while let Some(&(index, next)) = characters.peek() {
            if braced && next == ENV_VARIABLE_CLOSE {
                characters.next();
                end = index + next.len_utf8();
                break;
            }
            if !next.is_ascii_alphanumeric() && next != ENV_VARIABLE_UNDERSCORE {
                break;
            }
            name.push(next);
            characters.next();
            end = index + next.len_utf8();
        }
        match std::env::var(&name) {
            Ok(expanded) => result.push_str(&expanded),
            Err(_) => result.push_str(&value[start..end]),
        }
    }
    result
}

fn expand_home(value: &str, home: &Path) -> PathBuf {
    let path = Path::new(value);
    path.strip_prefix(HOME_ALIAS)
        .map_or_else(|_| path.to_owned(), |suffix| home.join(suffix))
}

thread_local! {
    /// `SLOT_LEASES` retains each census thread's `SlotLeaseReader`.
    pub(super) static SLOT_LEASES: RefCell<SlotLeaseReader> = RefCell::default();
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::os::unix::fs::symlink;

    use super::*;

    const SLOT: &str = "hana-slot2";
    const MACHINE: &str = "test-host";
    const QUALIFIED_MACHINE: &str = "test-host.example";
    const HOLDER: &str = "fps";
    const STAFF_ID: &str = "staff-id";
    const RENAMED_HOLDER: &str = "graphics";
    const PURPOSE: &str = "measure startup";
    const CHANGED_PURPOSE: &str = "measure repaint";
    const ORIGINAL: &str = "original directory";
    const CHECKOUT_FOLDER: &str = "slots";
    const OUTPUT_FOLDER: &str = "cargo";
    const ROOT_ALIAS: &str = "slot-link";
    const CONFIG_FILE: &str = "config.toml";
    const STAFF_FILE: &str = "staff.json";
    const INVALID_LEASE: &str = "invalid JSON";
    const INVALID_STAFF: &str = "invalid staff JSON";
    const CONFIG_DIRECTORY: &str = ".claude/config";
    const UNASSIGNED_OUTPUT_FOLDER: &str = "scratch";
    const OTHER_DIRECTORY: &str = "other";
    const CHECKOUT_SUFFIX: &str = "crates/app";
    const OUTPUT_SUFFIX: &str = "debug";
    const REPLACEMENT: &str = "replacement";
    const HOME_ENVIRONMENT: &str = "HOME";
    const HOME_VARIABLE: &str = "${HOME}";
    const PARENT_COMPONENT: &str = "..";
    const EMPTY_SECTION: &str = "[roots]\n";

    enum ExpectedStaffState {
        Loaded,
        Unreadable,
    }

    fn label(holder: &str, purpose: &str) -> String {
        format!("{SLOT}{SLOT_LABEL_SEPARATOR}{holder}{SLOT_LABEL_SEPARATOR}{purpose}")
    }

    fn write_lease(path: &Path, purpose: &str) -> io::Result<()> {
        fs::write(
            path,
            serde_json::json!({"unit": STAFF_ID, "purpose": purpose}).to_string(),
        )
    }

    fn write_staff(path: &Path, name: &str) -> io::Result<()> {
        fs::write(
            path,
            serde_json::json!({"rows": [{"id": STAFF_ID, "name": name}]}).to_string(),
        )
    }

    fn assert_root_revisions(
        reader: &mut SlotLeaseReader,
        home: &Path,
        config: &Path,
        checkout: &Path,
        output: &Path,
        roots: String,
    ) -> io::Result<()> {
        let folders = home.join(BUILD_FOLDERS_CONFIG);
        let outside = home.join(OTHER_DIRECTORY);
        let unassigned_output = home.join(OUTPUT_FOLDER).join(UNASSIGNED_OUTPUT_FOLDER);
        fs::create_dir_all(&unassigned_output)?;
        for (paths, machine, expected) in [
            (vec![checkout], MACHINE, label(HOLDER, PURPOSE)),
            (vec![checkout], QUALIFIED_MACHINE, label(HOLDER, PURPOSE)),
            (
                vec![outside.as_path(), output],
                MACHINE,
                label(HOLDER, PURPOSE),
            ),
            (vec![outside.as_path()], MACHINE, ORIGINAL.to_owned()),
            (
                vec![outside.as_path(), unassigned_output.as_path()],
                MACHINE,
                ORIGINAL.to_owned(),
            ),
            (
                vec![unassigned_output.as_path(), output],
                MACHINE,
                label(HOLDER, PURPOSE),
            ),
        ] {
            let observed = reader.observe_in(paths, home, machine, config);
            assert_eq!(observed.display_path(ORIGINAL), expected);
        }
        let account_home = std::env::var(HOME_ENVIRONMENT)
            .map(PathBuf::from)
            .map_err(io::Error::other)?;
        let ascent = account_home
            .components()
            .filter(|part| matches!(part, Component::Normal(_)))
            .map(|_| PARENT_COMPONENT)
            .collect::<Vec<_>>()
            .join(std::path::MAIN_SEPARATOR_STR);
        let relative_home = home
            .strip_prefix(std::path::MAIN_SEPARATOR_STR)
            .map_err(io::Error::other)?;
        let environment_root = format!(
            "{HOME_VARIABLE}/{ascent}/{}/{ROOT_ALIAS}",
            relative_home.display()
        );
        for (configuration, paths, expected) in [
            (
                format!("{SLOT_ROOT_KEY}.{MACHINE}={environment_root}\n"),
                vec![checkout],
                label(HOLDER, PURPOSE),
            ),
            (
                format!(
                    "{SLOT_ROOT_KEY}.{MACHINE}=~/{ROOT_ALIAS}\n{SLOT_CARGO_ROOT_KEY}=~/{OUTPUT_FOLDER}\n"
                ),
                vec![checkout],
                label(HOLDER, PURPOSE),
            ),
            (
                format!("{SLOT_CARGO_ROOT_KEY}=~/{OUTPUT_FOLDER}\n"),
                vec![output],
                ORIGINAL.to_owned(),
            ),
            (roots, vec![checkout, output], label(HOLDER, PURPOSE)),
        ] {
            fs::write(&folders, configuration)?;
            let observed = reader.observe_in(paths, home, MACHINE, config);
            assert_eq!(observed.display_path(ORIGINAL), expected);
        }
        Ok(())
    }

    fn assert_lease_revisions(
        reader: &mut SlotLeaseReader,
        home: &Path,
        config: &Path,
        checkout: &Path,
        staff: &Path,
        lease: &Path,
        slots: &Path,
    ) -> io::Result<()> {
        let replacement = slots.join(REPLACEMENT);
        write_lease(&replacement, CHANGED_PURPOSE)?;
        fs::rename(&replacement, lease)?;
        assert_eq!(
            reader
                .observe_in([checkout], home, MACHINE, config)
                .display_path(ORIGINAL),
            label(HOLDER, CHANGED_PURPOSE)
        );
        write_staff(&replacement, RENAMED_HOLDER)?;
        fs::rename(&replacement, staff)?;
        assert_eq!(
            reader
                .observe_in([checkout], home, MACHINE, config)
                .display_path(ORIGINAL),
            label(RENAMED_HOLDER, CHANGED_PURPOSE)
        );
        for (contents, expected) in [
            (
                serde_json::json!({"rows": []}).to_string(),
                ExpectedStaffState::Loaded,
            ),
            (INVALID_STAFF.to_owned(), ExpectedStaffState::Unreadable),
        ] {
            fs::write(staff, contents)?;
            let mut scan = reader.begin_scan(home, MACHINE, config);
            assert_eq!(
                scan.observe([checkout]).display_path(ORIGINAL),
                label(STAFF_ID, CHANGED_PURPOSE)
            );
            match expected {
                ExpectedStaffState::Loaded => {
                    assert!(matches!(scan.staff, ScanStaffNames::Loaded(_)));
                },
                ExpectedStaffState::Unreadable => {
                    assert!(matches!(scan.staff, ScanStaffNames::Unreadable));
                },
            }
        }
        fs::write(lease, INVALID_LEASE)?;
        assert_eq!(
            reader
                .observe_in([checkout], home, MACHINE, config)
                .display_path(ORIGINAL),
            format!("{SLOT}{SLOT_LABEL_SEPARATOR}{SLOT_UNREADABLE_LABEL}")
        );
        write_lease(lease, CHANGED_PURPOSE)?;
        Ok(())
    }

    #[test]
    fn slot_rows_follow_live_locks_leases_staff_and_configured_roots() -> io::Result<()> {
        let temporary = tempfile::tempdir()?;
        let home = temporary.path().canonicalize()?;
        let slots = home.join(CHECKOUT_FOLDER);
        let cargo = home.join(OUTPUT_FOLDER);
        fs::create_dir_all(home.join(CONFIG_DIRECTORY))?;
        fs::create_dir_all(&slots)?;
        fs::create_dir_all(&cargo)?;
        symlink(&slots, home.join(ROOT_ALIAS))?;
        let folders = home.join(BUILD_FOLDERS_CONFIG);
        let roots = format!(
            "{SLOT_ROOT_KEY}=~/wrong\n{SLOT_ROOT_KEY}.{MACHINE}=~/{CHECKOUT_FOLDER}\n{SLOT_CARGO_ROOT_KEY}=~/{OUTPUT_FOLDER}\n"
        );
        fs::write(&folders, &roots)?;
        let config = home.join(CONFIG_FILE);
        fs::write(&config, format!("[census]\nstaff_file='~/{STAFF_FILE}'\n"))?;
        let staff = home.join(STAFF_FILE);
        write_staff(&staff, HOLDER)?;
        let lease = slots.join(format!("{SLOT}{SLOT_LEASE_SUFFIX}"));
        write_lease(&lease, PURPOSE)?;
        let guard = File::create(slots.join(format!("{SLOT}{SLOT_LOCK_SUFFIX}")))?;
        guard.lock()?;
        let mut reader = SlotLeaseReader::default();
        let checkout = slots.join(SLOT).join(CHECKOUT_SUFFIX);
        let output = cargo.join(SLOT).join(OUTPUT_SUFFIX);
        assert_root_revisions(&mut reader, &home, &config, &checkout, &output, roots)?;
        // Each scan keeps one lock observation even if the holder releases it during the scan.
        {
            let mut scan = reader.begin_scan(&home, MACHINE, &config);
            assert_eq!(
                scan.observe([checkout.as_path()]).display_path(ORIGINAL),
                label(HOLDER, PURPOSE)
            );
            guard.unlock()?;
            assert_eq!(
                scan.observe([output.as_path()]).display_path(ORIGINAL),
                label(HOLDER, PURPOSE)
            );
        }
        assert_eq!(
            reader
                .observe_in([checkout.as_path()], &home, MACHINE, &config)
                .display_path(ORIGINAL),
            format!("{SLOT}{SLOT_LABEL_SEPARATOR}{SLOT_FREE_LABEL}")
        );
        guard.lock()?;
        assert_lease_revisions(
            &mut reader,
            &home,
            &config,
            &checkout,
            &staff,
            &lease,
            &slots,
        )?;
        drop(guard);
        assert_eq!(
            reader
                .observe_in([checkout.as_path()], &home, MACHINE, &config)
                .display_path(ORIGINAL),
            format!("{SLOT}{SLOT_LABEL_SEPARATOR}{SLOT_FREE_LABEL}")
        );
        fs::write(&folders, EMPTY_SECTION)?;
        assert_eq!(
            reader
                .observe_in(
                    [checkout.as_path(), output.as_path()],
                    &home,
                    MACHINE,
                    &config
                )
                .display_path(ORIGINAL),
            ORIGINAL
        );
        Ok(())
    }
}
