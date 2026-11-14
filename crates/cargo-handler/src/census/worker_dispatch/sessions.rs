//! Live supervisors select unit directories without listing historical sessions.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use super::DIRECTOR_FILE;
use super::WorkerReference;
use crate::census::Observed;
use crate::census::PassAssignment;
use crate::census::PassRole;
use crate::census::SourceRead;
use crate::census::UnknownReason;
use crate::census::activity;
use crate::census::activity::RunDirector;
use crate::census::classify::ProcessEntry;
use crate::census::hierarchy::FileRevision;
use crate::census::hierarchy::StaffRoster;
use crate::census::hierarchy::StaffRunPointer;
use crate::constants::DETACHED_RUNS_DIRECTORY;
use crate::constants::PROGRESS_HISTORY_STATE_FILE;
use crate::constants::SESSION_ROOTS;

/// An open pass supplies an activity start without defining a session status.
#[derive(Clone, Debug)]
pub(in crate::census) struct WorkerPass {
    pub(in crate::census) worker: String,
    pub(in crate::census) state:  PassState,
}

/// Only an open pass has a start and an assignment.
#[derive(Clone, Debug)]
pub(in crate::census) enum PassState {
    Open {
        started:    Observed<u64>,
        assignment: Observed<PassAssignment>,
    },
    Closed,
}

impl PassState {
    fn read(pass: &Value) -> Self {
        if pass["status"] != "active" {
            return Self::Closed;
        }
        let started = pass["started_at"].as_f64().map_or_else(
            || Observed::unknown("open pass has no start"),
            activity::epoch_seconds,
        );
        let role = PassRole::deserialize(&pass["kind"]).ok();
        let phase = pass["phase"]["instance_id"]
            .as_str()
            .filter(|phase| !phase.is_empty());
        let assignment = match (role, phase) {
            (Some(role), Some(phase)) => Observed::Known(PassAssignment {
                role,
                phase: phase.to_owned(),
            }),
            (None, _) => Observed::Unknown(UnknownReason(format!(
                "open pass kind {} is not impl, test, fix or review",
                pass["kind"]
            ))),
            (Some(_), None) => Observed::unknown("open pass has no phase instance id"),
        };
        Self::Open {
            started,
            assignment,
        }
    }
}

/// Read only the staff-selected run's pass map.
pub(in crate::census) fn current_passes(directory: &Path) -> SourceRead<Vec<WorkerPass>> {
    let path = directory.join(PROGRESS_HISTORY_STATE_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return SourceRead::Unreadable(UnknownReason(format!("{}: {error}", path.display())));
        },
    };
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return SourceRead::Unreadable(UnknownReason(format!("{}: {error}", path.display())));
        },
    };
    let passes: Vec<&Value> = match value.get("pass") {
        None => Vec::new(),
        Some(single @ Value::Object(fields)) if fields.contains_key("instance_id") => vec![single],
        Some(Value::Object(slots)) => slots.values().collect(),
        Some(_) => {
            return SourceRead::Unreadable(UnknownReason(format!(
                "{}: pass is not an object",
                path.display()
            )));
        },
    };
    let mut result = Vec::new();
    for pass in passes {
        let Some(worker) = pass["worker_id"]
            .as_str()
            .filter(|worker| !worker.is_empty())
        else {
            continue;
        };
        result.push(WorkerPass {
            worker: worker.to_owned(),
            state:  PassState::read(pass),
        });
    }
    SourceRead::Loaded(result)
}

/// `RunSelection` names whether a unit directory is its staff row's current run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::census) enum RunSelection {
    Current,
    Historical,
}

/// The configured roots used to validate worker-owned session paths.
#[derive(Debug)]
pub(in crate::census) struct SessionRoots {
    pub(super) directories: Vec<PathBuf>,
    directors:              BTreeMap<PathBuf, (FileRevision, String)>,
    owners:                 BTreeMap<PathBuf, (FileRevision, UnitDirectorSource)>,
    passes:                 BTreeMap<PathBuf, (FileRevision, SourceRead<Vec<WorkerPass>>)>,
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
        fs::read(directory.join(PROGRESS_HISTORY_STATE_FILE))
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
    /// Retain unowned legacy activity while rejecting a checked unit's historical run.
    pub(in crate::census) fn selection(
        &mut self,
        directory: &Path,
        staff: &StaffRoster,
    ) -> RunSelection {
        let pointer = match self.recorded_owner(directory, staff) {
            RunDirector::Session(session) => staff.unit_run(session),
            RunDirector::Unrecorded => staff.unit_run(
                self.directors
                    .get(directory)
                    .map_or("", |(_, session)| session.trim()),
            ),
        };
        match pointer {
            Observed::Known(StaffRunPointer::Current(run)) if run.path() == directory => {
                RunSelection::Current
            },
            Observed::Known(StaffRunPointer::Current(_) | StaffRunPointer::NoRun) => {
                RunSelection::Historical
            },
            Observed::Unknown(_) => RunSelection::Current,
        }
    }

    /// Reuse an unchanged pass source without reading its document again.
    pub(in crate::census) fn passes(&mut self, directory: &Path) -> SourceRead<Vec<WorkerPass>> {
        let revision = fs::metadata(directory.join(PROGRESS_HISTORY_STATE_FILE))
            .map_or(FileRevision::Missing, |metadata| {
                FileRevision::of(&metadata)
            });
        if let Some((old, passes)) = self.passes.get(directory)
            && old == &revision
        {
            return passes.clone();
        }
        let passes = current_passes(directory);
        self.passes
            .insert(directory.to_owned(), (revision, passes.clone()));
        passes
    }

    pub(in crate::census) fn new(directories: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            directories:                 directories.into_iter().collect(),
            directors:                   BTreeMap::new(),
            owners:                      BTreeMap::new(),
            passes:                      BTreeMap::new(),
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
        let path = directory.join(PROGRESS_HISTORY_STATE_FILE);
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
        let current: BTreeMap<_, _> = staff
            .current_runs()
            .filter_map(|(session, run)| {
                directors
                    .get(session)
                    .map(|pid| (run.path().to_owned(), *pid))
            })
            .collect();
        let sessions: HashSet<_> = sessions
            .iter()
            .cloned()
            .chain(current.keys().cloned())
            .collect();
        self.directors.retain(|path, _| sessions.contains(path));
        self.owners.retain(|path, _| sessions.contains(path));
        self.passes.retain(|path, _| sessions.contains(path));
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
                let director_pid = current
                    .get(&session_path)
                    .copied()
                    .or(owner)
                    .or_else(|| staff_director(&session_path))
                    .or_else(|| {
                        directors
                            .get(self.directors[&session_path].1.trim())
                            .copied()
                    })?;
                if let Some((session, _)) = directors.iter().find(|(_, pid)| **pid == director_pid)
                {
                    match staff.unit_run(session) {
                        Observed::Known(StaffRunPointer::Current(run))
                            if run.path() != session_path =>
                        {
                            return None;
                        },
                        Observed::Known(StaffRunPointer::NoRun) => return None,
                        Observed::Known(StaffRunPointer::Current(_)) | Observed::Unknown(_) => {},
                    }
                }
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
    use std::sync::Arc;

    use super::*;
    use crate::census::HierarchySource;
    use crate::census::account::ConfigHome;

    fn staff_for_current_run(root: &Path, current: &Path) -> std::io::Result<Arc<StaffRoster>> {
        let path = root.join("staff.json");
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "schema":2,"rows":[
                    {"id":"boss","kind":"showrunner","name":"boss","parent":null,"current_session":"boss"},
                    {"id":"unit","kind":"unit","name":"unit","parent":"boss","current_session":"live-director","run":current}
                ]
            }))?,
        )?;
        Ok(crate::census::hierarchy::StaffFileReader::default()
            .roster(&HierarchySource::File(path.to_string_lossy().into_owned())))
    }

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
        let staff = staff_for_current_run(root.path(), &idle)?;
        for candidates in [HashSet::from([active, idle.clone()]), HashSet::new()] {
            let current = roots.managed_sessions(&candidates, &directors, &staff, |_| None);
            assert_eq!(
                current
                    .iter()
                    .map(|item| &item.directory)
                    .collect::<Vec<_>>(),
                [&idle],
                "the staff pointer selects the current run even without a supervisor"
            );
        }
        Ok(())
    }
}
