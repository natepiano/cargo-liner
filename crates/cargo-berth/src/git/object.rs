//! Resolving revision expressions to commit objects, and reading whether those
//! objects are still present.
//!
//! Every cluster in this module starts from one of these two questions, so the
//! resolution types live here rather than inside any one of them.

use std::collections::BTreeMap;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
#[cfg(test)]
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::mpsc::Receiver;

use super::child;
use super::child::ChildDeadline;
use super::child::ChildProcess;
use super::command;
use super::constants::GIT_AMBIGUOUS_OBJECT_SUFFIX;
use super::constants::GIT_BATCH_CHECK_OBJECT_FORMAT_ARG;
use super::constants::GIT_CAT_FILE_COMMAND;
use super::constants::GIT_COMMIT_OBJECT_TYPE;
use super::constants::GIT_COMMIT_PEEL_SUFFIX;
use super::constants::GIT_MISSING_OBJECT_SUFFIX;
#[cfg(test)]
use super::constants::GIT_REV_PARSE_COMMAND;
#[cfg(test)]
use super::error;
use super::error::GitError;
use super::protocol::LineExchange;
use super::read;
use super::read::RepositoryRead;
use crate::ids::GitObjectId;
use crate::ledger::FullRefName;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CommitObjectResolution {
    Resolved(GitObjectId),
    Missing,
    Ambiguous,
    WrongType { object_type: String },
}

#[derive(Clone, Copy)]
pub(super) enum CommitAvailability {
    Available,
    ObjectUnknown,
}

#[cfg(test)]
pub(super) fn object_id(repository_root: &Path, revision: &str) -> Result<GitObjectId, GitError> {
    let output = error::completed_git_command(
        command::git_output(repository_root, [GIT_REV_PARSE_COMMAND, revision]).into(),
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let object_id = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    GitObjectId::from_str(object_id.trim()).map_err(GitError::InvalidObjectId)
}

/// One operation's object classifier. The child is owned until the operation ends;
/// workers serialize complete request/response pairs through the same pipe.
pub(super) struct CommitObjectReader {
    process: Mutex<CommitObjectProcess>,
}

struct CommitObjectProcess {
    child:       ChildProcess,
    pipes:       LineExchange,
    diagnostics: Receiver<io::Result<Vec<u8>>>,
}

impl CommitObjectReader {
    pub(super) fn open(repository_root: &Path) -> Result<Self, GitError> {
        let mut child = command::spawn_with_piped_output(
            command::git_command(repository_root)?
                .args([GIT_CAT_FILE_COMMAND, GIT_BATCH_CHECK_OBJECT_FORMAT_ARG])
                .stdin(Stdio::piped()),
        )?;
        // These streams were piped above. Convert the standard library's optional
        // handles at this boundary, before publishing the running reader.
        let streams = {
            let pipes = child.child_mut();
            (pipes.stdin.take(), pipes.stdout.take(), pipes.stderr.take())
        };
        let (Some(input), Some(output), Some(diagnostics)) = streams else {
            return Err(io::Error::other("git object reader omitted a piped stream").into());
        };
        Ok(Self {
            process: Mutex::new(CommitObjectProcess {
                child,
                pipes: LineExchange::new(input, output)?,
                diagnostics: child::drain(Some(diagnostics))?,
            }),
        })
    }

    pub(super) fn resolve(
        &self,
        expressions: &[String],
    ) -> Result<Vec<CommitObjectResolution>, GitError> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| io::Error::other("git object reader lock was poisoned"))?;
        expressions
            .iter()
            .map(|expression| process.resolve(expression))
            .collect()
    }
}

impl CommitObjectProcess {
    fn resolve(&mut self, expression: &str) -> Result<CommitObjectResolution, GitError> {
        // A newline is the batch protocol boundary, never part of an expression.
        if expression.contains(['\n', '\r']) || expression.contains("@{") {
            return Err(GitError::InvalidBatchObjectLine {
                line: expression.to_owned(),
            });
        }
        let line = match self
            .child
            .exchange(&self.pipes, expression, child::GIT_COMMAND_DEADLINE)
        {
            Ok(line) => line,
            // Git may close stdin before the protocol worker writes its request.
            // Preserve its repository diagnostic just as stdout EOF does.
            Err(error) if error.kind() == ErrorKind::BrokenPipe => return self.ended_reader(),
            Err(error) => return Err(error.into()),
        };
        if line.is_empty() {
            return self.ended_reader();
        }
        commit_object_resolution(line.trim_end_matches('\n'))
    }

    fn ended_reader(&mut self) -> Result<CommitObjectResolution, GitError> {
        let status = self.child.wait(child::PIPE_EOF_DEADLINE)?;
        let stderr = ChildDeadline::new(
            self.child.command(),
            "object reader diagnostics EOF",
            child::PIPE_EOF_DEADLINE,
        )
        .receive(&self.diagnostics)?;
        Err(GitError::CommandFailed {
            command: GIT_CAT_FILE_COMMAND,
            stderr:  if stderr.is_empty() {
                format!("object reader ended with {status}")
            } else {
                String::from_utf8_lossy(&stderr).trim().to_owned()
            },
        })
    }
}

pub(super) fn commit_object_resolution(line: &str) -> Result<CommitObjectResolution, GitError> {
    if line.ends_with(GIT_MISSING_OBJECT_SUFFIX) {
        return Ok(CommitObjectResolution::Missing);
    }
    if line.ends_with(GIT_AMBIGUOUS_OBJECT_SUFFIX) {
        return Ok(CommitObjectResolution::Ambiguous);
    }
    let Some((object_id, object_type)) = line.split_once(' ') else {
        return Err(GitError::InvalidBatchObjectLine {
            line: line.to_owned(),
        });
    };
    if object_type != GIT_COMMIT_OBJECT_TYPE {
        return Ok(CommitObjectResolution::WrongType {
            object_type: object_type.to_owned(),
        });
    }
    object_id
        .parse()
        .map(CommitObjectResolution::Resolved)
        .map_err(GitError::InvalidObjectId)
}

/// A branch's configured tracking reference, or its explicit lack of an upstream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BranchUpstream {
    Configured(FullRefName),
    Unconfigured,
}

impl BranchUpstream {
    pub(super) fn from_reference(reference: &str) -> Result<Self, GitError> {
        if reference.is_empty() {
            Ok(Self::Unconfigured)
        } else {
            reference
                .parse()
                .map(Self::Configured)
                .map_err(|_| GitError::InvalidReferenceName {
                    reference: reference.to_owned(),
                })
        }
    }
}

type UpstreamReferenceRead = Result<BTreeMap<String, BranchUpstream>, Arc<GitError>>;

/// Shared object queries for one operation, including its post-append repairs.
/// Clones retain the same lazy child; no prepared action can open another one.
#[derive(Clone)]
pub(crate) struct OperationObjectReader {
    repository_root: PathBuf,
    reader:          Arc<OnceLock<Result<CommitObjectReader, Arc<GitError>>>>,
    upstreams:       Arc<OnceLock<UpstreamReferenceRead>>,
}

impl OperationObjectReader {
    pub(super) fn new(repository_root: &Path) -> Self {
        Self {
            repository_root: repository_root.to_path_buf(),
            reader:          Arc::new(OnceLock::new()),
            upstreams:       Arc::new(OnceLock::new()),
        }
    }

    /// Resolve includes, remote fetch mappings and packed references once per operation.
    /// Optional upstream selectors must never enter the persistent batch protocol:
    /// Git treats an unconfigured selector as fatal, rather than a missing object.
    pub(super) fn upstream_references(
        &self,
    ) -> Result<&BTreeMap<String, BranchUpstream>, GitError> {
        self.upstreams
            .get_or_init(|| self.read_upstream_references().map_err(Arc::new))
            .as_ref()
            .map_err(|failure| GitError::SnapshotReadFailed(Arc::clone(failure)))
    }

    fn read_upstream_references(&self) -> Result<BTreeMap<String, BranchUpstream>, GitError> {
        read::upstreams(&self.repository_root)?.or_git(|| {
            let output = command::git_output(
                &self.repository_root,
                [
                    "for-each-ref",
                    "--format=%(refname)%00%(upstream)",
                    "refs/heads/",
                ],
            )?;
            if !output.status.success() {
                return Err(GitError::CommandFailed {
                    command: "for-each-ref",
                    stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                });
            }
            let text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
            text.lines()
                .map(|line| {
                    line.split_once('\0')
                        .ok_or_else(|| GitError::InvalidBatchObjectLine {
                            line: line.to_owned(),
                        })
                        .and_then(|(branch, upstream)| {
                            BranchUpstream::from_reference(upstream)
                                .map(|upstream| (branch.to_owned(), upstream))
                        })
                })
                .collect()
        })
    }

    pub(super) fn resolve(
        &self,
        expressions: &[String],
    ) -> Result<Vec<CommitObjectResolution>, GitError> {
        let mut resolutions = Vec::new();
        for expression in expressions {
            match read::commit(&self.repository_root, expression)? {
                RepositoryRead::InProcess(resolution) => resolutions.push(resolution),
                RepositoryRead::GitRequired(requirement) => {
                    return requirement.execute(|| self.resolve_with_git(expressions));
                },
            }
        }
        Ok(resolutions)
    }

    fn resolve_with_git(
        &self,
        expressions: &[String],
    ) -> Result<Vec<CommitObjectResolution>, GitError> {
        self.reader
            .get_or_init(|| CommitObjectReader::open(&self.repository_root).map_err(Arc::new))
            .as_ref()
            .map_err(|failure| GitError::SnapshotReadFailed(Arc::clone(failure)))?
            .resolve(expressions)
    }

    /// Resolve one commit expression through this operation's existing child.
    pub(crate) fn commit(&self, expression: &str) -> Result<GitObjectId, GitError> {
        let resolutions = self.resolve(&[expression.to_owned()])?;
        match resolutions.into_iter().next() {
            Some(CommitObjectResolution::Resolved(object_id)) => Ok(object_id),
            Some(CommitObjectResolution::Missing) => Err(GitError::MissingCommitExpression {
                expression: expression.to_owned(),
            }),
            Some(CommitObjectResolution::Ambiguous) => Err(GitError::AmbiguousCommitExpression {
                expression: expression.to_owned(),
            }),
            Some(CommitObjectResolution::WrongType { object_type }) => {
                Err(GitError::WrongCommitExpressionType {
                    expression: expression.to_owned(),
                    object_type,
                })
            },
            None => Err(GitError::InvalidBatchObjectCount {
                expected: 1,
                actual:   0,
            }),
        }
    }

    pub(super) fn commit_availability(
        &self,
        object_ids: &[GitObjectId],
    ) -> Result<Vec<CommitAvailability>, GitError> {
        let expressions = object_ids
            .iter()
            .map(|object_id| format!("{object_id}{GIT_COMMIT_PEEL_SUFFIX}"))
            .collect::<Vec<_>>();
        let resolutions = self.resolve(&expressions)?;
        Ok(resolutions
            .into_iter()
            .map(|resolution| match resolution {
                CommitObjectResolution::Resolved(_) => CommitAvailability::Available,
                CommitObjectResolution::Missing
                | CommitObjectResolution::Ambiguous
                | CommitObjectResolution::WrongType { .. } => CommitAvailability::ObjectUnknown,
            })
            .collect())
    }

    pub(crate) fn commit_is_available(&self, object_id: &GitObjectId) -> Result<bool, GitError> {
        Ok(matches!(
            self.commit_availability(std::slice::from_ref(object_id))?
                .as_slice(),
            [CommitAvailability::Available]
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::CommitObjectReader;
    use super::CommitObjectResolution;
    use super::GitError;
    use super::commit_object_resolution;
    use crate::git::fixture::FixtureResult;
    use crate::git::fixture::PatchEquivalenceFixture;
    use crate::git::fixture::UNAVAILABLE_OBJECT_ID;
    use crate::ids::GitObjectId;

    #[test]
    fn concurrent_history_reads_finish_while_the_object_reader_remains_open() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let repository =
            std::sync::Arc::new(crate::git::RepositorySnapshot::discover(fixture.root())?);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(5));
        let (completed, progress) = std::sync::mpsc::channel();
        for _ in 0..4 {
            let repository = std::sync::Arc::clone(&repository);
            let barrier = std::sync::Arc::clone(&barrier);
            let completed = completed.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let output = super::command::git_output(
                    repository.repository_root(),
                    ["rev-list", "--parents", "HEAD"],
                );
                let success =
                    output.is_ok_and(|output| output.status.success() && !output.stdout.is_empty());
                let _ = completed.send(success);
            });
        }
        barrier.wait();
        let head = repository.head_object_id()?.clone();
        for _ in 0..4 {
            assert!(
                progress.recv_timeout(std::time::Duration::from_secs(3))?,
                "history read should succeed"
            );
        }
        assert_eq!(repository.object_reader().commit("HEAD")?, head);
        Ok(())
    }

    #[test]
    fn operation_object_reader_shares_packed_branch_and_head_queries() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["pack-refs", "--all"])?;
        let snapshot = crate::git::RepositorySnapshot::discover(fixture.root())?;
        let head = snapshot.head_object_id()?.clone();
        let objects = snapshot.object_reader();
        assert!(
            objects.reader.get().is_none(),
            "the exact HEAD read needs no Git child"
        );
        assert!(objects.commit_is_available(&head)?);
        let child_id = || -> Result<u32, Box<dyn std::error::Error>> {
            let reader = objects
                .reader
                .get()
                .ok_or("object child was not started")?
                .as_ref()
                .map_err(ToString::to_string)?;
            Ok(reader
                .process
                .lock()
                .map_err(|error| error.to_string())?
                .child
                .child_mut()
                .id())
        };
        assert!(
            objects.reader.get().is_none(),
            "exact object availability needs no child"
        );
        assert_eq!(objects.commit("HEAD~0")?, head);
        let first_child = child_id()?;
        assert_eq!(crate::git::branch_object_id(&objects, "main")?, head);
        assert!(matches!(
            crate::git::branch_object_id(&objects, "absent-branch"),
            Err(GitError::MissingCommitExpression { .. })
        ));
        assert_eq!(snapshot.head_object_id()?, &head);
        assert_eq!(child_id()?, first_child);
        Ok(())
    }

    #[test]
    fn missing_upstream_preserves_later_included_and_packed_upstream_queries() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&[
            "update-ref",
            "refs/remotes/origin/main",
            &fixture.phase_start_head.to_string(),
        ])?;
        fixture.git(&["branch", "no-upstream"])?;
        fixture.git(&["pack-refs", "--all"])?;
        let configuration = fixture.root().join(".git/upstream.conf");
        std::fs::write(
            &configuration,
            "[remote \"origin\"]\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n",
        )?;
        fixture.git(&["config", "include.path", "upstream.conf"])?;
        let repository = crate::git::RepositorySnapshot::discover(fixture.root())?;
        let head = repository.head_object_id()?.clone();
        let objects = repository.object_reader();
        let child_id = || -> Result<u32, Box<dyn std::error::Error>> {
            let reader = objects
                .reader
                .get()
                .ok_or("object child was not started")?
                .as_ref()
                .map_err(ToString::to_string)?;
            Ok(reader
                .process
                .lock()
                .map_err(|error| error.to_string())?
                .child
                .child_mut()
                .id())
        };
        assert!(
            objects.reader.get().is_none(),
            "exact HEAD needs no Git child"
        );
        assert!(objects.commit_is_available(&head)?);
        assert!(
            objects.reader.get().is_none(),
            "exact object availability needs no child"
        );
        assert_eq!(objects.commit("HEAD~0")?, head);
        let child = child_id()?;
        assert!(matches!(
            crate::git::local_target_upstream(&objects, "no-upstream")?,
            crate::git::LocalTargetUpstream::Unavailable
        ));
        assert!(matches!(
            crate::git::local_target_upstream(&objects, "main")?,
            crate::git::LocalTargetUpstream::Available(upstream) if upstream == head
        ));
        fixture.git(&["update-ref", "-d", "refs/remotes/origin/main"])?;
        assert!(matches!(
            crate::git::local_target_upstream(&objects, "main")?,
            crate::git::LocalTargetUpstream::Unavailable
        ));
        assert_eq!(child_id()?, child);
        Ok(())
    }

    #[test]
    fn operation_object_reader_reuses_one_child_for_packed_refs_and_missing_objects()
    -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["pack-refs", "--all"])?;
        let reader = CommitObjectReader::open(fixture.root())?;
        let child = reader
            .process
            .lock()
            .map_err(|error| error.to_string())?
            .child
            .child_mut()
            .id();
        assert_eq!(
            reader.resolve(&["refs/heads/main".to_owned()])?,
            [CommitObjectResolution::Resolved(
                fixture.phase_start_head.clone()
            ),]
        );
        assert_eq!(
            reader.resolve(&[
                "HEAD".to_owned(),
                "HEAD^{tree}".to_owned(),
                UNAVAILABLE_OBJECT_ID.to_owned()
            ])?,
            [
                CommitObjectResolution::Resolved(fixture.phase_start_head),
                CommitObjectResolution::WrongType {
                    object_type: "tree".to_owned(),
                },
                CommitObjectResolution::Missing,
            ]
        );
        assert_eq!(
            reader
                .process
                .lock()
                .map_err(|error| error.to_string())?
                .child
                .child_mut()
                .id(),
            child
        );
        Ok(())
    }

    #[test]
    fn missing_ref_object_and_ancestor_queries_preserve_the_shared_reader() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let reader = CommitObjectReader::open(fixture.root())?;
        for expression in [
            "refs/heads/absent",
            "HEAD~99999",
            "HEAD^{tree}^{commit}",
            UNAVAILABLE_OBJECT_ID,
        ] {
            assert_eq!(
                reader.resolve(&[expression.to_owned()])?,
                [CommitObjectResolution::Missing]
            );
            assert_eq!(
                reader.resolve(&["HEAD".to_owned()])?,
                [CommitObjectResolution::Resolved(
                    fixture.phase_start_head.clone()
                )]
            );
        }
        assert!(
            reader
                .resolve(&["main@{upstream}^{commit}".to_owned()])
                .is_err()
        );
        assert_eq!(
            reader.resolve(&["HEAD".to_owned()])?,
            [CommitObjectResolution::Resolved(fixture.phase_start_head)]
        );
        Ok(())
    }

    #[test]
    fn upstream_read_failure_is_not_a_missing_upstream() -> FixtureResult {
        let directory = tempfile::tempdir()?;
        let objects = super::OperationObjectReader::new(directory.path());
        assert!(crate::git::local_target_upstream(&objects, "main").is_err());
        Ok(())
    }

    #[test]
    fn operation_object_reader_survives_snapshot_until_retention_finishes() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let snapshot = crate::git::RepositorySnapshot::discover(fixture.root())?;
        let head = snapshot.head_object_id()?.clone();
        let objects = snapshot.object_reader();
        assert!(
            objects.reader.get().is_none(),
            "the exact HEAD read needs no Git child"
        );
        assert!(objects.commit_is_available(&head)?);
        let child_id = || -> Result<u32, Box<dyn std::error::Error>> {
            let reader = objects
                .reader
                .get()
                .ok_or("object child was not started")?
                .as_ref()
                .map_err(ToString::to_string)?;
            Ok(reader
                .process
                .lock()
                .map_err(|error| error.to_string())?
                .child
                .child_mut()
                .id())
        };
        assert!(
            objects.reader.get().is_none(),
            "exact object availability needs no child"
        );
        assert_eq!(objects.commit("HEAD~0")?, head);
        let first_child = child_id()?;
        drop(snapshot);
        assert!(objects.commit_is_available(&head)?);
        assert!(!objects.commit_is_available(&UNAVAILABLE_OBJECT_ID.parse()?)?);
        assert_eq!(child_id()?, first_child);
        Ok(())
    }

    #[test]
    fn operation_object_reader_keeps_a_failed_repository_distinct_from_a_missing_object()
    -> FixtureResult {
        let directory = tempfile::tempdir()?;
        std::fs::write(
            directory.path().join(".git"),
            "invalid repository locator\n",
        )?;
        let reader = CommitObjectReader::open(directory.path())?;
        let error = reader
            .resolve(&["HEAD".to_owned()])
            .err()
            .ok_or("expected the repository failure")?;
        assert!(
            matches!(error, GitError::CommandFailed { command: "cat-file", stderr } if stderr.contains("invalid gitfile format"))
        );
        Ok(())
    }

    #[test]
    fn batch_object_records_preserve_every_resolution_failure() -> FixtureResult {
        let object_id = UNAVAILABLE_OBJECT_ID.parse::<GitObjectId>()?;
        assert_eq!(
            commit_object_resolution(&format!("{object_id} commit"))?,
            CommitObjectResolution::Resolved(object_id.clone())
        );
        assert_eq!(
            commit_object_resolution("missing-expression missing")?,
            CommitObjectResolution::Missing
        );
        assert_eq!(
            commit_object_resolution("ambiguous-expression ambiguous")?,
            CommitObjectResolution::Ambiguous
        );
        assert_eq!(
            commit_object_resolution(&format!("{object_id} tree"))?,
            CommitObjectResolution::WrongType {
                object_type: "tree".to_owned(),
            }
        );
        Ok(())
    }
}
