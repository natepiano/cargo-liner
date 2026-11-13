//! Live supervisors select unit directories without listing historical sessions.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use super::DIRECTOR_FILE;
use super::WorkerReference;
use crate::census::activity;
use crate::census::activity::RunDirector;
use crate::census::classify::ProcessEntry;
use crate::census::hierarchy::FileRevision;
use crate::census::hierarchy::StaffRoster;
use crate::constants::DETACHED_RUNS_DIRECTORY;
use crate::constants::SESSION_ROOTS;

/// The configured roots used to validate worker-owned session paths.
#[derive(Debug)]
pub(in crate::census) struct SessionRoots {
    pub(super) directories: Vec<PathBuf>,
    directors:              BTreeMap<PathBuf, (FileRevision, String)>,
    owners:                 BTreeMap<PathBuf, (FileRevision, UnitDirectorSource)>,
    #[cfg(test)]
    director_reads:         usize,
}

/// `UnitDirectorSource` retains the unit's own director proof without a worker run.
#[derive(Debug)]
pub(super) enum UnitDirectorSource {
    Staff(WorkerReference),
    Session(String),
    Unrecorded,
}

#[derive(Deserialize)]
struct UnitOwnerRecord {
    owner_staff_id: Option<String>,
    main_agent:     Option<UnitMainAgent>,
}

#[derive(Deserialize)]
struct UnitMainAgent {
    session_id: Option<String>,
}

impl From<UnitOwnerRecord> for UnitDirectorSource {
    fn from(record: UnitOwnerRecord) -> Self {
        if let Some(staff_id) = record.owner_staff_id.filter(|id| !id.trim().is_empty()) {
            Self::Staff(WorkerReference::Staff { staff_id })
        } else if let Some(id) = record
            .main_agent
            .and_then(|agent| agent.session_id)
            .filter(|id| !id.trim().is_empty())
        {
            Self::Session(id)
        } else {
            Self::Unrecorded
        }
    }
}

impl UnitDirectorSource {
    pub(super) fn read(directory: &Path) -> Self {
        fs::read(directory.join("progress_history_state.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<UnitOwnerRecord>(&bytes).ok())
            .map_or(Self::Unrecorded, Self::from)
    }

    pub(super) fn session<'a>(&'a self, staff: &'a StaffRoster) -> RunDirector<'a> {
        match self {
            Self::Staff(reference) => reference
                .session(staff)
                .map_or(RunDirector::Unrecorded, RunDirector::Session),
            Self::Session(id) => RunDirector::Session(id),
            Self::Unrecorded => RunDirector::Unrecorded,
        }
    }
}

/// A live unit directory joined to its director's census row.
pub(in crate::census) struct ManagedSession {
    pub(in crate::census) directory:    PathBuf,
    pub(in crate::census) director_pid: u32,
}

impl SessionRoots {
    pub(in crate::census) fn new(directories: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            directories:                 directories.into_iter().collect(),
            directors:                   BTreeMap::new(),
            owners:                      BTreeMap::new(),
            #[cfg(test)]
            director_reads:              0,
        }
    }

    pub(in crate::census) fn production() -> Self { Self::new(SESSION_ROOTS.map(PathBuf::from)) }

    /// Live supervisors prove their unit directories before staff resolution.
    pub(in crate::census) fn live_directories(
        &self,
        processes: &[ProcessEntry],
    ) -> HashSet<PathBuf> {
        processes
            .iter()
            .filter_map(activity::supervisor_record)
            .filter_map(|record| {
                let runs = record.parent()?;
                if runs.file_name()? != DETACHED_RUNS_DIRECTORY {
                    return None;
                }
                let session = runs.parent()?;
                self.directories
                    .iter()
                    .any(|root| session.parent() == Some(root.as_path()))
                    .then(|| session.to_owned())
            })
            .collect()
    }

    /// `UnitDirectorSource` reads the unit record only when its file revision changes.
    fn recorded_owner<'a>(
        &'a mut self,
        directory: &Path,
        staff: &'a StaffRoster,
    ) -> RunDirector<'a> {
        let path = directory.join("progress_history_state.json");
        let revision = fs::metadata(&path).map_or(FileRevision::Missing, |metadata| {
            FileRevision::of(&metadata)
        });
        if self
            .owners
            .get(directory)
            .is_none_or(|(old, _)| *old != revision)
        {
            let owner = UnitDirectorSource::read(directory);
            self.owners.insert(directory.to_owned(), (revision, owner));
        }
        self.owners[directory].1.session(staff)
    }

    /// Only a live supervisor can request discovery of a unit's director.
    pub(in crate::census) fn managed_sessions(
        &mut self,
        sessions: &HashSet<PathBuf>,
        directors: &BTreeMap<&str, u32>,
        staff: &StaffRoster,
        mut staff_director: impl FnMut(&Path) -> Option<u32>,
    ) -> Vec<ManagedSession> {
        self.directors.retain(|path, _| sessions.contains(path));
        self.owners.retain(|path, _| sessions.contains(path));
        sessions
            .iter()
            .cloned()
            .filter_map(|session_path| {
                let path = session_path.join(DIRECTOR_FILE);
                let revision = fs::metadata(&path).map_or(FileRevision::Missing, |metadata| {
                    FileRevision::of(&metadata)
                });
                if self
                    .directors
                    .get(&session_path)
                    .is_none_or(|(old, _)| *old != revision)
                {
                    #[cfg(test)]
                    {
                        self.director_reads += 1;
                    }
                    self.directors.insert(
                        session_path.clone(),
                        (revision, fs::read_to_string(path).unwrap_or_default()),
                    );
                }
                let owner = match self.recorded_owner(&session_path, staff) {
                    RunDirector::Session(id) => directors.get(id).copied(),
                    RunDirector::Unrecorded => None,
                };
                let director_pid =
                    owner
                        .or_else(|| staff_director(&session_path))
                        .or_else(|| {
                            directors
                                .get(self.directors[&session_path].1.trim())
                                .copied()
                        })?;
                Some(ManagedSession {
                    directory: session_path,
                    director_pid,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::census::account::ConfigHome;

    #[test]
    fn only_live_supervisors_select_sessions_and_unchanged_directors_are_not_read_again()
    -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        let active = root.path().join("active");
        let idle = root.path().join("idle");
        fs::create_dir_all(active.join("detached_runs"))?;
        fs::create_dir_all(&idle)?;
        fs::write(active.join(DIRECTOR_FILE), "live-director")?;
        fs::write(idle.join(DIRECTOR_FILE), "live-director")?;
        let mut roots = SessionRoots::new([root.path().to_owned()]);
        let process = ProcessEntry {
            pid:         100,
            parent:      Some(1),
            name:        "python3".into(),
            arguments:   vec![
                "python3".into(),
                "detached_runs.py".into(),
                "supervise".into(),
                active
                    .join("detached_runs/gate.json")
                    .to_string_lossy()
                    .into_owned(),
            ],
            started:     100,
            directory:   None,
            claude_pid:  None,
            config_home: ConfigHome::Unread,
        };
        let directors = BTreeMap::from([("live-director", 10)]);
        let processes = [process];
        let selected = roots.managed_sessions(
            &roots.live_directories(&processes),
            &directors,
            &StaffRoster::default(),
            |_| None,
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].directory, active);
        assert_eq!(selected[0].director_pid, 10);
        assert_eq!(roots.director_reads, 1);
        assert_eq!(
            roots
                .managed_sessions(
                    &roots.live_directories(&processes),
                    &directors,
                    &StaffRoster::default(),
                    |_| None
                )
                .len(),
            1
        );
        assert_eq!(
            roots.director_reads, 1,
            "an unchanged director file is read once"
        );
        fs::write(active.join(DIRECTOR_FILE), "other-director")?;
        assert!(
            roots
                .managed_sessions(
                    &roots.live_directories(&processes),
                    &directors,
                    &StaffRoster::default(),
                    |_| None
                )
                .is_empty()
        );
        assert_eq!(roots.director_reads, 2);
        assert!(
            roots
                .managed_sessions(&HashSet::new(), &directors, &StaffRoster::default(), |_| {
                    None
                })
                .is_empty()
        );
        assert!(
            roots.directors.is_empty(),
            "finished sessions release their cached director"
        );
        Ok(())
    }
}
