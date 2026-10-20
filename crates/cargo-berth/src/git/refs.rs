//! Git references: reading them, moving them, and the reservation retention refs.
//!
//! Every read of a reference — `HEAD`, a local branch, an arbitrary full ref — and every
//! write to one lives here, including the private `refs/` namespace that retains each
//! reservation's protected commit.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Write;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Stdio;

use super::command;
use super::command::GitHookExecutionPolicy;
use super::constants::GIT_FOR_EACH_REF_COMMAND;
use super::constants::GIT_FULL_REF_FORMAT_ARG;
#[cfg(test)]
use super::constants::GIT_HEAD_REVISION;
use super::constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use super::constants::GIT_MAX_COUNT_ONE_ARG;
use super::constants::GIT_POINTS_AT_ARG_PREFIX;
use super::constants::GIT_REFLOG_COMMAND;
use super::constants::GIT_REFLOG_SHOW_ARG;
use super::constants::GIT_REFLOG_SUBJECT_FORMAT_ARG;
use super::constants::GIT_REV_PARSE_COMMAND;
use super::constants::GIT_STDIN_ARG;
use super::constants::GIT_UPDATE_REF_COMMAND;
use super::constants::RESERVATION_RETENTION_REF_PREFIX;
use super::error;
use super::error::GitError;
use super::error::GitLockRefusal;
#[cfg(test)]
use super::object;
use super::object::CommitAvailability;
use super::object::OperationObjectReader;
use super::reachability;
use super::reachability::Reachability;
use super::reachability::ResolvedBatchCommitCandidates;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ledger::FullRefName;

const GIT_MISSING_REFERENCE_EXIT_CODE: i32 = 2;
const GIT_SHOW_REF_COMMAND: &str = "show-ref";
const GIT_SHOW_REF_EXISTS_ARG: &str = "--exists";

/// The target's upstream commit, if its ref and object are available locally.
pub(crate) enum LocalTargetUpstream {
    Available(GitObjectId),
    Unavailable,
}

pub(crate) fn local_target_upstream(
    objects: &OperationObjectReader,
    branch: &str,
) -> Result<LocalTargetUpstream, GitError> {
    let branch = format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{branch}");
    let upstreams = objects.upstream_references()?;
    let Some(reference) = upstreams
        .get(&branch)
        .filter(|reference| !reference.is_empty())
    else {
        return Ok(LocalTargetUpstream::Unavailable);
    };
    match objects.commit(reference) {
        Ok(commit) => Ok(LocalTargetUpstream::Available(commit)),
        Err(
            GitError::MissingCommitExpression { .. } | GitError::WrongCommitExpressionType { .. },
        ) => Ok(LocalTargetUpstream::Unavailable),
        Err(error) => Err(error),
    }
}

/// Read fixture HEAD after a test mutates the repository.
#[cfg(test)]
pub(super) fn head_object_id(repository_root: &Path) -> Result<GitObjectId, GitError> {
    object::object_id(repository_root, GIT_HEAD_REVISION)
}

/// Read the full object id currently named by a local branch.
pub(crate) fn branch_object_id(
    objects: &OperationObjectReader,
    branch: &str,
) -> Result<GitObjectId, GitError> {
    objects.commit(&format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{branch}"))
}

/// Whether Git currently reports `HEAD` as attached to a branch or detached.
#[derive(Clone)]
pub(crate) enum HeadAttachment {
    /// `HEAD` names this full branch reference.
    Branch { full_ref: FullRefName },
    /// `HEAD` names a commit directly.
    Detached,
}

/// Whether a full git reference currently resolves to an object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReferenceLookup {
    /// The reference resolves to this object id.
    Present(GitObjectId),
    /// Git reports no object under this reference name.
    Missing,
}

/// Resolve a full reference while preserving Git failures separately from a missing reference.
pub(crate) fn reference_lookup(
    repository_root: &Path,
    reference: &str,
) -> Result<ReferenceLookup, GitError> {
    reference
        .parse::<FullRefName>()
        .map_err(|_| GitError::InvalidReferenceName {
            reference: reference.to_owned(),
        })?;
    let existence_output = command::git_output(
        repository_root,
        [GIT_SHOW_REF_COMMAND, GIT_SHOW_REF_EXISTS_ARG, reference],
    )?;
    if !existence_output.status.success() {
        if existence_output.status.code() == Some(GIT_MISSING_REFERENCE_EXIT_CODE) {
            return Ok(ReferenceLookup::Missing);
        }
        return Err(GitError::CommandFailed {
            command: GIT_SHOW_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&existence_output.stderr)
                .trim()
                .to_owned(),
        });
    }

    let output = command::git_output(repository_root, [GIT_REV_PARSE_COMMAND, reference])?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let object_id = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    object_id
        .trim()
        .parse()
        .map(ReferenceLookup::Present)
        .map_err(GitError::InvalidObjectId)
}

/// Atomically move one local branch from the expected old object to a proposed object.
pub(crate) fn update_local_branch(
    repository_root: &Path,
    branch: &str,
    proposed: &GitObjectId,
    expected_previous: &GitObjectId,
) -> Result<(), GitError> {
    match reachability::reachability(repository_root, expected_previous, proposed)? {
        Reachability::Ancestor => {},
        Reachability::NotAncestor => {
            return Err(GitError::NonFastForwardBranchUpdate {
                previous: expected_previous.clone(),
                proposed: proposed.clone(),
            });
        },
        Reachability::ObjectUnknown => {
            return Err(GitError::BranchUpdateObjectUnavailable {
                previous: expected_previous.clone(),
                proposed: proposed.clone(),
            });
        },
    }
    let reference = format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{branch}");
    let proposed = proposed.to_string();
    let expected_previous = expected_previous.to_string();
    let output = command::git_output(
        repository_root,
        [
            GIT_UPDATE_REF_COMMAND,
            &reference,
            &proposed,
            &expected_previous,
        ],
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(error::ref_write_failure(repository_root, &output.stderr))
    }
}

/// Whether one rename target was proven for a deleted local branch's object tip.
pub(crate) enum LocalBranchRenameTargetResolution {
    /// No local branch at the object proves a rename from the deleted branch.
    NotProven,
    /// Exactly one local branch at the object proves the rename.
    Unique(FullRefName),
    /// Several local branches prove the rename, so no single target can be chosen.
    Ambiguous,
}

/// Whether a local branch's newest reflog entry proves it replaced a deleted branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalBranchRenameProof {
    /// The newest reflog entry records the candidate's rename from the deleted branch.
    Recorded,
    /// The candidate has no matching newest reflog entry.
    NotRecorded,
}

/// Find whether exactly one local branch at `tip` has proof it replaced the deleted branch.
pub(crate) fn local_branch_rename_target_resolution(
    repository_root: &Path,
    tip: &GitObjectId,
    deleted_reference: &FullRefName,
) -> Result<LocalBranchRenameTargetResolution, GitError> {
    let arguments = vec![
        GIT_FOR_EACH_REF_COMMAND.to_owned(),
        GIT_FULL_REF_FORMAT_ARG.to_owned(),
        format!("{GIT_POINTS_AT_ARG_PREFIX}{tip}"),
        GIT_LOCAL_BRANCH_REF_PREFIX.to_owned(),
    ];
    let output = command::git_output_dynamic(repository_root, &arguments)?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_FOR_EACH_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let references = String::from_utf8(output.stdout)
        .map_err(GitError::InvalidOutput)?
        .lines()
        .map(|reference| {
            reference
                .parse::<FullRefName>()
                .map_err(|_| GitError::InvalidReferenceName {
                    reference: reference.to_owned(),
                })
        })
        .filter(|reference| {
            reference
                .as_ref()
                .map_or(true, |reference| reference != deleted_reference)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut proven_replacements = LocalBranchRenameTargetResolution::NotProven;
    for reference in references {
        match local_branch_rename_proof(repository_root, deleted_reference, &reference)? {
            LocalBranchRenameProof::Recorded => match proven_replacements {
                LocalBranchRenameTargetResolution::NotProven => {
                    proven_replacements = LocalBranchRenameTargetResolution::Unique(reference);
                },
                LocalBranchRenameTargetResolution::Unique(_)
                | LocalBranchRenameTargetResolution::Ambiguous => {
                    return Ok(LocalBranchRenameTargetResolution::Ambiguous);
                },
            },
            LocalBranchRenameProof::NotRecorded => {},
        }
    }
    Ok(proven_replacements)
}

/// Read whether `candidate_reference` records a rename from `deleted_reference`.
fn local_branch_rename_proof(
    repository_root: &Path,
    deleted_reference: &FullRefName,
    candidate_reference: &FullRefName,
) -> Result<LocalBranchRenameProof, GitError> {
    let candidate_reference = candidate_reference.to_string();
    let output = command::git_output(
        repository_root,
        [
            GIT_REFLOG_COMMAND,
            GIT_REFLOG_SHOW_ARG,
            GIT_MAX_COUNT_ONE_ARG,
            GIT_REFLOG_SUBJECT_FORMAT_ARG,
            &candidate_reference,
        ],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REFLOG_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let subject = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let expected_subject = format!("Branch: renamed {deleted_reference} to {candidate_reference}");
    Ok(match subject.lines().next() {
        Some(subject) if subject == expected_subject => LocalBranchRenameProof::Recorded,
        Some(_) | None => LocalBranchRenameProof::NotRecorded,
    })
}

/// One reservation ref that must retain its protected commit when that commit is readable.
pub(crate) struct ReservationRetentionRefRepair {
    reservation_id: ReservationId,
    protected_tip:  GitObjectId,
}

impl ReservationRetentionRefRepair {
    pub(crate) const fn new(reservation_id: ReservationId, protected_tip: GitObjectId) -> Self {
        Self {
            reservation_id,
            protected_tip,
        }
    }
}

/// The full private git ref that retains one reservation's protected tip.
struct ReservationRetentionRef(String);

impl ReservationRetentionRef {
    fn for_reservation(reservation_id: ReservationId) -> Self {
        Self(format!(
            "{RESERVATION_RETENTION_REF_PREFIX}{reservation_id}"
        ))
    }
}

impl Display for ReservationRetentionRef {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { formatter.write_str(&self.0) }
}

/// The ref maintenance following one durable reconciliation append.
#[derive(Clone, Debug)]
pub(crate) enum RetentionMaintenance {
    Completed,
    Deferred(Box<GitLockRefusal>),
}

impl RetentionMaintenance {
    /// Report once at a boundary without an output envelope, after releasing the lock.
    pub(crate) fn report(&self) {
        if let Self::Deferred(lock) = self {
            eprintln!("{}", lock.maintenance_notice());
        }
    }
}

/// Return the full private ref used to retain one reservation's protected tip.
pub(crate) fn reservation_retention_ref_name(reservation_id: ReservationId) -> String {
    ReservationRetentionRef::for_reservation(reservation_id).to_string()
}

/// Create or update a reservation's retention ref.
pub(crate) fn write_reservation_retention_ref(
    repository_root: &Path,
    reservation_id: ReservationId,
    protected_tip: &GitObjectId,
) -> Result<(), GitError> {
    let retention_ref = reservation_retention_ref_name(reservation_id);
    let protected_tip = protected_tip.to_string();
    let arguments = [
        GIT_UPDATE_REF_COMMAND.to_owned(),
        retention_ref,
        protected_tip,
    ];
    let output = command::git_output_dynamic_with_hook_execution_policy(
        repository_root,
        &arguments,
        GitHookExecutionPolicy::SuppressedForRetentionRef,
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(error::ref_write_failure(repository_root, &output.stderr))
    }
}

/// A ref transaction whose object availability was established before ledger acquisition.
pub(crate) struct PreparedRetentionRefTransaction {
    writer: RetentionRefWriter,
}

/// No ref locks or changes exist until the started transaction receives its writes and commit.
enum RetentionRefWriter {
    NoWrites,
    Started {
        child:           Child,
        input:           String,
        repository_root: PathBuf,
    },
}

impl PreparedRetentionRefTransaction {
    /// Reuse the target batch's availability, resolving only newly selected settlement tips.
    pub(crate) fn prepare_reconciliation(
        repository_root: &Path,
        objects: &OperationObjectReader,
        repairs: &[ReservationRetentionRefRepair],
        deletions: &[ReservationId],
        resolved_candidates: &ResolvedBatchCommitCandidates,
    ) -> Result<Self, GitError> {
        let mut input = String::new();
        let mut unresolved = Vec::new();
        for repair in repairs {
            if resolved_candidates.contains(&repair.protected_tip) {
                let _ = writeln!(
                    input,
                    "update {} {}",
                    reservation_retention_ref_name(repair.reservation_id),
                    repair.protected_tip
                );
            } else {
                unresolved.push(ReservationRetentionRefRepair::new(
                    repair.reservation_id,
                    repair.protected_tip.clone(),
                ));
            }
        }
        if unresolved.len() == repairs.len() {
            return Self::prepare(repository_root, objects, repairs, deletions);
        }
        input.push_str(&Self::input(objects, &unresolved, deletions)?);
        Self::start(repository_root, input)
    }

    /// Resolve repair tips through the operation's existing reader, before the append.
    pub(crate) fn prepare(
        repository_root: &Path,
        objects: &OperationObjectReader,
        repairs: &[ReservationRetentionRefRepair],
        deletions: &[ReservationId],
    ) -> Result<Self, GitError> {
        Self::start(repository_root, Self::input(objects, repairs, deletions)?)
    }

    fn input(
        objects: &OperationObjectReader,
        repairs: &[ReservationRetentionRefRepair],
        deletions: &[ReservationId],
    ) -> Result<String, GitError> {
        let protected_tips = repairs
            .iter()
            .map(|repair| repair.protected_tip.clone())
            .collect::<Vec<_>>();
        let availability = if protected_tips.is_empty() {
            Vec::new()
        } else {
            objects.commit_availability(&protected_tips)?
        };
        let input = repairs.iter().zip(availability).fold(
            String::new(),
            |mut input, (repair, availability)| {
                if matches!(availability, CommitAvailability::Available) {
                    let _ = writeln!(
                        input,
                        "update {} {}",
                        reservation_retention_ref_name(repair.reservation_id),
                        repair.protected_tip
                    );
                }
                input
            },
        );
        let input = deletions.iter().fold(input, |mut input, reservation_id| {
            let _ = writeln!(
                input,
                "delete {}",
                reservation_retention_ref_name(*reservation_id)
            );
            input
        });
        Ok(input)
    }

    /// A committed-hook audit narrows the plan's writes without starting another writer.
    pub(crate) fn for_committed_hook(
        mut self,
        repository_root: &Path,
        objects: &OperationObjectReader,
        repairs: &[ReservationRetentionRefRepair],
    ) -> Result<Self, GitError> {
        let mut input = Self::input(objects, repairs, &[])?;
        if input.is_empty() {
            return Ok(Self::empty());
        }
        if let RetentionRefWriter::Started { input: pending, .. } = &mut self.writer {
            input.push_str("commit\n");
            *pending = input;
            Ok(self)
        } else {
            Self::start(repository_root, input)
        }
    }

    pub(crate) const fn empty() -> Self {
        Self {
            writer: RetentionRefWriter::NoWrites,
        }
    }

    fn start(repository_root: &Path, mut input: String) -> Result<Self, GitError> {
        if input.is_empty() {
            return Ok(Self::empty());
        }
        input.push_str("commit\n");
        let child = command::spawn_with_piped_output(
            command::git_command_with_hook_execution_policy(
                repository_root,
                GitHookExecutionPolicy::DeferrableRetentionMaintenance,
            )?
            .args([GIT_UPDATE_REF_COMMAND, GIT_STDIN_ARG])
            .stdin(Stdio::piped()),
        )?;
        let mut transaction = Self {
            writer: RetentionRefWriter::Started {
                child,
                input,
                repository_root: repository_root.to_path_buf(),
            },
        };
        if let RetentionRefWriter::Started { child, .. } = &mut transaction.writer {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| std::io::Error::other("retention input was not piped"))?;
            stdin.write_all(b"start\n")?;
            stdin.flush()?;
            let stdout = child
                .stdout
                .as_mut()
                .ok_or_else(|| std::io::Error::other("retention output was not piped"))?;
            let mut response = String::new();
            BufReader::new(stdout).read_line(&mut response)?;
            if response != "start: ok\n" {
                return Err(GitError::CommandFailed {
                    command: GIT_UPDATE_REF_COMMAND,
                    stderr:  format!(
                        "retention transaction did not acknowledge start: {response:?}"
                    ),
                });
            }
        }
        Ok(transaction)
    }

    /// Defer only a lock refusal after a reconciliation append has already committed.
    pub(crate) fn commit_maintenance(self) -> Result<RetentionMaintenance, GitError> {
        match self.commit() {
            Ok(()) => Ok(RetentionMaintenance::Completed),
            Err(GitError::LockRefused(lock)) => Ok(RetentionMaintenance::Deferred(lock)),
            Err(error) => Err(error),
        }
    }

    /// Apply only the ordered ref write using the process initialized before acquisition.
    pub(crate) fn commit(mut self) -> Result<(), GitError> {
        let RetentionRefWriter::Started {
            child,
            input,
            repository_root,
        } = std::mem::replace(&mut self.writer, RetentionRefWriter::NoWrites)
        else {
            return Ok(());
        };
        let output = command::child_output_with_input(child, input.as_bytes())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(error::ref_write_failure(&repository_root, &output.stderr))
        }
    }
}

impl Drop for PreparedRetentionRefTransaction {
    fn drop(&mut self) {
        if let RetentionRefWriter::Started { child, .. } = &mut self.writer {
            // Invalidation discards a started transaction with no queued ref commands.
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Retire private references without resolving any object.
pub(crate) fn delete_reservation_retention_refs(
    repository_root: &Path,
    deletions: &[ReservationId],
) -> Result<(), GitError> {
    let input = deletions.iter().fold(String::new(), |mut input, id| {
        let _ = writeln!(input, "delete {}", reservation_retention_ref_name(*id));
        input
    });
    if input.is_empty() {
        return Ok(());
    }
    apply_transaction(repository_root, &input)
}

/// Apply one transaction containing only retention-ref writes and deletions.
fn apply_transaction(repository_root: &Path, input: &str) -> Result<(), GitError> {
    let arguments = [GIT_UPDATE_REF_COMMAND.to_owned(), GIT_STDIN_ARG.to_owned()];
    let output = command::git_output_dynamic_with_hook_execution_policy_and_input(
        repository_root,
        &arguments,
        GitHookExecutionPolicy::SuppressedForRetentionRef,
        input.as_bytes(),
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(error::ref_write_failure(repository_root, &output.stderr))
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;

    use super::PreparedRetentionRefTransaction;
    use super::ReservationRetentionRefRepair;
    use super::reservation_retention_ref_name;
    use crate::git::OperationObjectReader;
    use crate::git::fixture::FixtureResult;
    use crate::git::fixture::PatchEquivalenceFixture;
    use crate::git::fixture::UNAVAILABLE_OBJECT_ID;
    use crate::ids::ReservationId;

    #[test]
    fn retention_writer_starts_without_locking_refs_or_publishing_before_commit() -> FixtureResult {
        const CHILD: &str = "CARGO_BERTH_TEST_PREPARED_RETENTION_WRITER";
        const TEST: &str = "git::refs::tests::retention_writer_starts_without_locking_refs_or_publishing_before_commit";
        if std::env::var_os(CHILD).is_none() {
            let trace = tempfile::tempdir()?;
            let output = std::process::Command::new(std::env::current_exe()?)
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, "1")
                .env("GIT_TRACE2_EVENT", trace.path().join("git.json"))
                .output()?;
            assert!(output.status.success(), "{output:?}");
            return Ok(());
        }
        let fixture = PatchEquivalenceFixture::new()?;
        let id = ReservationId::new();
        let objects = OperationObjectReader::new(fixture.root());
        let prepare = || {
            PreparedRetentionRefTransaction::prepare(
                fixture.root(),
                &objects,
                &[ReservationRetentionRefRepair::new(
                    id,
                    fixture.phase_start_head.clone(),
                )],
                &[],
            )
        };
        let transaction = prepare()?;
        let trace = fs::read_to_string(std::env::var_os("GIT_TRACE2_EVENT").ok_or("trace path")?)?;
        assert!(
            trace.lines().any(|line| {
                let event: serde_json::Value = serde_json::from_str(line).expect("Git trace event");
                event["event"] == "start"
                    && event["argv"]
                        .as_array()
                        .is_some_and(|args| args.iter().any(|arg| arg == "update-ref"))
            }),
            "the retention command must have started during preparation: {trace}"
        );
        let reference = reservation_retention_ref_name(id);
        let path = fixture.root().join(".git").join(&reference);
        assert!(!path.exists(), "preparation must not publish the ref");
        // Another writer can change the same ref: preparation holds no ref lock.
        super::write_reservation_retention_ref(fixture.root(), id, &fixture.phase_start_head)?;
        drop(transaction);
        assert_eq!(
            fs::read_to_string(&path)?.trim(),
            fixture.phase_start_head.to_string()
        );
        fixture.git(&["update-ref", "-d", &reference])?;
        drop(prepare()?);
        assert!(
            !path.exists(),
            "discarding preparation must not publish the ref"
        );
        prepare()?.commit()?;
        assert_eq!(
            fs::read_to_string(path)?.trim(),
            fixture.phase_start_head.to_string()
        );
        Ok(())
    }

    #[test]
    fn prepared_retention_writes_outlive_the_object_reader_and_omit_missing_tips() -> FixtureResult
    {
        let fixture = PatchEquivalenceFixture::new()?;
        let available = ReservationId::new();
        let missing = ReservationId::new();
        let retired = ReservationId::new();
        super::write_reservation_retention_ref(fixture.root(), retired, &fixture.phase_start_head)?;
        let objects = OperationObjectReader::new(fixture.root());
        let transaction = PreparedRetentionRefTransaction::prepare(
            fixture.root(),
            &objects,
            &[
                ReservationRetentionRefRepair::new(available, fixture.phase_start_head.clone()),
                ReservationRetentionRefRepair::new(missing, UNAVAILABLE_OBJECT_ID.parse()?),
            ],
            &[retired],
        )?;
        drop(objects);
        transaction.commit()?;
        let path = |id| {
            fixture
                .root()
                .join(".git")
                .join(reservation_retention_ref_name(id))
        };
        assert_eq!(
            fs::read_to_string(path(available))?.trim(),
            &fixture.phase_start_head.to_string()
        );
        assert!(!path(missing).exists());
        assert!(!path(retired).exists());
        Ok(())
    }
}
