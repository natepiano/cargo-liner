//! Live supervisors select unit directories without listing historical sessions.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use super::DIRECTOR_FILE;
use crate::census::activity;
use crate::census::classify::ProcessEntry;
use crate::census::hierarchy::FileRevision;
use crate::constants::SESSION_ROOTS;

/// The configured roots used to validate worker-owned session paths.
#[derive(Debug)]
pub(in crate::census) struct SessionRoots {
    pub(super) directories: Vec<PathBuf>,
    directors:              BTreeMap<PathBuf, (FileRevision, String)>,
    #[cfg(test)]
    director_reads:         usize,
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
            #[cfg(test)]
            director_reads:              0,
        }
    }

    pub(in crate::census) fn production() -> Self { Self::new(SESSION_ROOTS.map(PathBuf::from)) }

    /// Only a live supervisor can request discovery of a unit's director and roster.
    pub(in crate::census) fn managed_sessions(
        &mut self,
        processes: &[ProcessEntry],
        directors: &BTreeMap<&str, u32>,
        mut staff_director: impl FnMut(&Path) -> Option<u32>,
    ) -> Vec<ManagedSession> {
        let sessions: HashSet<_> = processes
            .iter()
            .filter_map(activity::supervisor_record)
            .filter_map(|record| {
                let runs = record.parent()?;
                if runs.file_name()? != "mesh_runs" {
                    return None;
                }
                let session = runs.parent()?;
                self.directories
                    .iter()
                    .any(|root| session.parent() == Some(root.as_path()))
                    .then(|| session.to_owned())
            })
            .collect();
        self.directors.retain(|path, _| sessions.contains(path));
        sessions
            .into_iter()
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
                let director_pid = staff_director(&session_path).or_else(|| {
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
        fs::create_dir_all(active.join("mesh_runs"))?;
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
                "mesh_runs.py".into(),
                "supervise".into(),
                active
                    .join("mesh_runs/gate.json")
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
        let selected = roots.managed_sessions(&processes, &directors, |_| None);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].directory, active);
        assert_eq!(selected[0].director_pid, 10);
        assert_eq!(roots.director_reads, 1);
        assert_eq!(
            roots
                .managed_sessions(&processes, &directors, |_| None)
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
                .managed_sessions(&processes, &directors, |_| None)
                .is_empty()
        );
        assert_eq!(roots.director_reads, 2);
        assert!(roots.managed_sessions(&[], &directors, |_| None).is_empty());
        assert!(
            roots.directors.is_empty(),
            "finished sessions release their cached director"
        );
        Ok(())
    }
}
