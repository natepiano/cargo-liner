//! Repository reads whose supported forms need no Git child. Unsupported semantics
//! retain a named subprocess route; an unsuccessful read is never a missing fact.

use std::collections::BTreeMap;
use std::env::VarError;
use std::io::ErrorKind;
use std::path::Path;

use gix::Repository;
use gix::config::File;
use gix::config::Source;
use gix::diff::index::Change;
use gix::object::Kind;
use gix::progress::Discard;
use gix::remote::Direction;
use gix::status::Item;
use gix::status::UntrackedFiles;
use gix::status::index_worktree::iter::Summary;
use gix::status::tree_index::TrackRenames;

use super::command;
use super::constants::GIT_BRANCH_CONFIGURATION_SECTION;
use super::constants::GIT_BRANCH_MERGE_KEY;
use super::constants::GIT_BRANCH_REMOTE_KEY;
use super::constants::GIT_COMMIT_PEEL_SUFFIX;
use super::constants::GIT_CONFIG_COUNT_ENVIRONMENT;
use super::constants::GIT_CONFIG_FALSE_VALUE;
use super::constants::GIT_CONFIG_KEY_PREFIX;
use super::constants::GIT_CONFIG_PARAMETERS_ENVIRONMENT;
use super::constants::GIT_CONFIG_VALUE_PREFIX;
use super::constants::GIT_CONFIG_ZERO_VALUE;
use super::constants::GIT_FILE_TYPE_MASK;
use super::constants::GIT_FILESYSTEM_MONITOR_KEY;
use super::constants::GIT_GC_AUTO_KEY;
use super::constants::GIT_HEAD_REVISION;
use super::constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use super::constants::GIT_LOCAL_REMOTE_VALUE;
use super::constants::GIT_MAINTENANCE_AUTO_KEY;
use super::constants::GIT_OBJECT_FORMAT_KEY;
use super::constants::GIT_REF_STORAGE_KEY;
use super::constants::GIT_REFTABLE_STORAGE_VALUE;
use super::constants::GIT_REPOSITORY_ENVIRONMENT;
use super::constants::GIT_SHA256_OBJECT_FORMAT_VALUE;
use super::constants::GIT_SPARSE_CHECKOUT_KEY;
use super::constants::GIT_SPLIT_INDEX_KEY;
use super::error::GitError;
use super::object::BranchUpstream;
use super::object::CommitObjectResolution;
use super::refs::HeadAttachment;
use super::refs::ReferenceLookup;
use super::refs::RetentionRefMutation;
use crate::ids::GitObjectId;

/// Cases outside the in-process reader's compatibility contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GitReadRequirement {
    /// Git's command boundary clears these selectors before reading another checkout.
    RepositoryEnvironment,
    /// Command-line configuration is interpreted by Git, including conditional includes.
    ConfigurationEnvironment,
    /// This build reads SHA-1 objects; Git owns repositories using SHA-256.
    Sha256Objects,
    /// Git owns the reftable reference database.
    ReftableReferences,
    /// Git owns its configured filesystem monitor protocol.
    FileSystemMonitor,
    /// Sparse checkout and sparse-index expansion retain Git's status semantics.
    SparseCheckout,
    /// A split index requires Git's shared-index extension handling.
    SplitIndex,
    /// Git computes nested submodule dirtiness and ignore rules.
    SubmoduleStatus,
    /// Git combines the conflict stages into porcelain's seven conflict states.
    UnmergedIndex,
    /// Git treats intent-to-add entries specially in its tree/index comparison.
    IntentToAdd,
    /// Locked, prunable or inaccessible registrations need Git's diagnostic fields.
    ExtendedWorktreeRegistration,
    /// Symbolic refs outside HEAD retain Git's dereferencing and broken-ref behavior.
    SymbolicReference,
    /// Annotated tags require Git's commit-peeling semantics.
    AnnotatedTag,
    /// Revision syntax and abbreviated object names retain the existing batch protocol.
    RevisionExpression,
    /// Git chooses the first merge ref when a branch configures several upstreams.
    MultipleUpstreamBranches,
    /// Git resolves HEAD when it is configured as a local branch upstream.
    LocalUpstreamHead,
}

/// A successful in-process read or the precise semantics delegated to Git.
#[derive(Debug)]
pub(super) enum RepositoryRead<T> {
    InProcess(T),
    GitRequired(GitReadRequirement),
}

impl GitReadRequirement {
    /// Every unsupported form executes the existing Git read rather than supplying a fact.
    pub(super) fn execute<T>(
        self,
        subprocess: impl FnOnce() -> Result<T, GitError>,
    ) -> Result<T, GitError> {
        match self {
            Self::RepositoryEnvironment
            | Self::ConfigurationEnvironment
            | Self::Sha256Objects
            | Self::ReftableReferences
            | Self::FileSystemMonitor
            | Self::SparseCheckout
            | Self::SplitIndex
            | Self::SubmoduleStatus
            | Self::UnmergedIndex
            | Self::IntentToAdd
            | Self::ExtendedWorktreeRegistration
            | Self::SymbolicReference
            | Self::AnnotatedTag
            | Self::RevisionExpression
            | Self::MultipleUpstreamBranches
            | Self::LocalUpstreamHead => subprocess(),
        }
    }
}

impl<T> RepositoryRead<T> {
    pub(super) fn or_git(
        self,
        subprocess: impl FnOnce() -> Result<T, GitError>,
    ) -> Result<T, GitError> {
        match self {
            Self::InProcess(value) => Ok(value),
            Self::GitRequired(requirement) => requirement.execute(subprocess),
        }
    }
}

/// Open configuration during preparation, but observe each ref only when maintenance commits.
pub(super) struct RetentionRefReader {
    repository: RepositoryRead<Repository>,
}

impl RetentionRefReader {
    pub(super) fn new(root: &Path) -> Result<Self, GitError> {
        Ok(Self {
            repository: open(root)?,
        })
    }

    /// Bypass the mtime cache: packed refs may have been replaced without changing their timestamp.
    pub(super) fn required_writes(
        &self,
        writes: &[RetentionRefMutation],
    ) -> Result<RepositoryRead<Vec<RetentionRefMutation>>, GitError> {
        let repository = match &self.repository {
            RepositoryRead::InProcess(repository) => repository,
            RepositoryRead::GitRequired(requirement) => {
                return Ok(RepositoryRead::GitRequired(*requirement));
            },
        };
        let packed = repository
            .refs
            .open_packed_buffer()
            .map_err(|error| failure("read current packed retention refs", error))?;
        let mut required = Vec::new();
        for write in writes {
            let reference = repository
                .refs
                .try_find_packed(write.reference(), packed.as_ref())
                .map_err(|error| failure("read retention reference", error))?;
            let current = match reference {
                Some(reference) => match reference.target.try_id() {
                    Some(id) => ReferenceLookup::Present(
                        id.to_string().parse().map_err(GitError::InvalidObjectId)?,
                    ),
                    None => {
                        return Ok(RepositoryRead::GitRequired(
                            GitReadRequirement::SymbolicReference,
                        ));
                    },
                },
                None => ReferenceLookup::Missing,
            };
            if !write.is_satisfied_by(&current) {
                required.push(write.clone());
            }
        }
        Ok(RepositoryRead::InProcess(required))
    }
}

/// Load strict configuration from every Git source, including its installation prefix.
fn open(root: &Path) -> Result<RepositoryRead<Repository>, GitError> {
    if GIT_REPOSITORY_ENVIRONMENT
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        return Ok(RepositoryRead::GitRequired(
            GitReadRequirement::RepositoryEnvironment,
        ));
    }
    if let RepositoryRead::GitRequired(reason) = configuration_overlay() {
        return Ok(RepositoryRead::GitRequired(reason));
    }
    let mut permissions = gix::open::Permissions::default();
    permissions.config.git_binary = true;
    // gix discovers installation configuration with its own piped Git command.
    // Share the spawn boundary: on platforms without atomic close-on-exec pipes,
    // a concurrent persistent child must not inherit that probe's pipe writer.
    match command::with_spawn_lock(|| {
        gix::open_opts(
            root,
            gix::open::Options::default()
                .permissions(permissions)
                .strict_config(true),
        )
    })? {
        Ok(repository) => match repository_format(repository.config_snapshot().plumbing()) {
            RepositoryRead::InProcess(()) => Ok(RepositoryRead::InProcess(repository)),
            RepositoryRead::GitRequired(reason) => Ok(RepositoryRead::GitRequired(reason)),
        },
        Err(error) => {
            // SHA-256 is rejected during open, before a repository snapshot exists.
            let (path, _) =
                gix::discover::upwards(root).map_err(|_| failure("open repository", &error))?;
            let (git_dir, _) = path.into_repository_and_work_tree_directories();
            let common_dir = match std::fs::read_to_string(git_dir.join("commondir")) {
                Ok(relative) => git_dir.join(relative.trim()),
                Err(error) if error.kind() == ErrorKind::NotFound => git_dir,
                Err(error) => return Err(failure("read common directory", error)),
            };
            let config =
                gix::config::File::from_path_no_includes(common_dir.join("config"), Source::Local)
                    .map_err(|error| failure("read repository format", error))?;
            match repository_format(&config) {
                RepositoryRead::GitRequired(reason) => Ok(RepositoryRead::GitRequired(reason)),
                RepositoryRead::InProcess(()) => Err(failure("open repository", error)),
            }
        },
    }
}

/// Maintenance overlays cannot affect these reads; other command configuration stays with Git.
fn configuration_overlay() -> RepositoryRead<()> {
    let unsupported = RepositoryRead::GitRequired(GitReadRequirement::ConfigurationEnvironment);
    if std::env::var_os(GIT_CONFIG_PARAMETERS_ENVIRONMENT).is_some() {
        return unsupported;
    }
    let count = match std::env::var(GIT_CONFIG_COUNT_ENVIRONMENT) {
        Ok(count) => match count.parse::<usize>() {
            Ok(count) => count,
            Err(_) => return unsupported,
        },
        Err(VarError::NotPresent) => return RepositoryRead::InProcess(()),
        Err(VarError::NotUnicode(_)) => return unsupported,
    };
    for index in 0..count {
        let Ok(key) = std::env::var(format!("{GIT_CONFIG_KEY_PREFIX}{index}")) else {
            return unsupported;
        };
        if !(key.eq_ignore_ascii_case(GIT_MAINTENANCE_AUTO_KEY)
            || key.eq_ignore_ascii_case(GIT_GC_AUTO_KEY))
            || std::env::var_os(format!("{GIT_CONFIG_VALUE_PREFIX}{index}")).is_none()
        {
            return unsupported;
        }
    }
    RepositoryRead::InProcess(())
}

/// Recognize storage formats before a missing loose ref can be mistaken for absence.
fn repository_format(config: &File) -> RepositoryRead<()> {
    if config
        .string(GIT_OBJECT_FORMAT_KEY)
        .is_some_and(|format| format.as_slice() == GIT_SHA256_OBJECT_FORMAT_VALUE.as_bytes())
    {
        return RepositoryRead::GitRequired(GitReadRequirement::Sha256Objects);
    }
    if config
        .string(GIT_REF_STORAGE_KEY)
        .is_some_and(|format| format.as_slice() == GIT_REFTABLE_STORAGE_VALUE.as_bytes())
    {
        return RepositoryRead::GitRequired(GitReadRequirement::ReftableReferences);
    }
    RepositoryRead::InProcess(())
}

/// Convert backend failures at the boundary, retaining the failed operation.
fn failure(activity: &'static str, error: impl std::fmt::Display) -> GitError {
    GitError::RepositoryReadFailed {
        activity,
        diagnostic: error.to_string(),
    }
}

/// Identify status settings whose Git behavior requires the subprocess route.
fn status_configuration(config: &File) -> RepositoryRead<()> {
    for (key, reason) in [
        (
            GIT_FILESYSTEM_MONITOR_KEY,
            GitReadRequirement::FileSystemMonitor,
        ),
        (GIT_SPARSE_CHECKOUT_KEY, GitReadRequirement::SparseCheckout),
        (GIT_SPLIT_INDEX_KEY, GitReadRequirement::SplitIndex),
    ] {
        if config.string(key).is_some_and(|value| {
            value.as_slice() != GIT_CONFIG_FALSE_VALUE.as_bytes()
                && value.as_slice() != GIT_CONFIG_ZERO_VALUE.as_bytes()
        }) {
            return RepositoryRead::GitRequired(reason);
        }
    }
    RepositoryRead::InProcess(())
}

/// Produce the same no-renames porcelain columns consumed by drift's partitioner.
pub(super) fn status(root: &Path) -> Result<RepositoryRead<Vec<u8>>, GitError> {
    let repository = match open(root)? {
        RepositoryRead::InProcess(repository) => repository,
        RepositoryRead::GitRequired(reason) => return Ok(RepositoryRead::GitRequired(reason)),
    };
    if let RepositoryRead::GitRequired(reason) = status_configuration(&repository.config_snapshot())
    {
        return Ok(RepositoryRead::GitRequired(reason));
    }
    let index = repository
        .index_or_empty()
        .map_err(|error| failure("read index", error))?;
    if index.entries().iter().any(|entry| entry.stage_raw() != 0) {
        return Ok(RepositoryRead::GitRequired(
            GitReadRequirement::UnmergedIndex,
        ));
    }
    if index
        .entries()
        .iter()
        .any(|entry| entry.mode == gix::index::entry::Mode::COMMIT)
    {
        return Ok(RepositoryRead::GitRequired(
            GitReadRequirement::SubmoduleStatus,
        ));
    }
    let dirwalk = repository
        .dirwalk_options()
        .map_err(|error| failure("configure untracked files", error))?;
    let changes = repository
        .status(Discard)
        .map_err(|error| failure("configure status", error))?
        .index_worktree_options_mut(|options| options.dirwalk_options = Some(dirwalk))
        .untracked_files(UntrackedFiles::Files)
        .index_worktree_rewrites(None)
        .tree_index_track_renames(TrackRenames::Disabled)
        .into_iter(std::iter::empty::<gix::bstr::BString>())
        .map_err(|error| failure("read status", error))?;
    let mut paths: BTreeMap<Vec<u8>, [u8; 2]> = BTreeMap::new();
    let mut untracked = std::collections::BTreeSet::new();
    for change in changes {
        let change = change.map_err(|error| failure("read status entry", error))?;
        let path = change.location().to_vec();
        let columns = paths.entry(path.clone()).or_insert(*b"  ");
        match change {
            Item::TreeIndex(change) => {
                columns[0] = match change {
                    Change::Addition { .. } => b'A',
                    Change::Deletion { .. } => b'D',
                    Change::Modification {
                        previous_entry_mode,
                        entry_mode,
                        ..
                    } => {
                        if previous_entry_mode.bits() & GIT_FILE_TYPE_MASK
                            == entry_mode.bits() & GIT_FILE_TYPE_MASK
                        {
                            b'M'
                        } else {
                            b'T'
                        }
                    },
                    Change::Rewrite { .. } => {
                        return Err(failure("read status", "rename tracking remained enabled"));
                    },
                };
            },
            Item::IndexWorktree(change) => match change.summary() {
                Some(Summary::Added) => {
                    untracked.insert(path);
                },
                Some(Summary::Removed) => columns[1] = b'D',
                Some(Summary::TypeChange) => columns[1] = b'T',
                Some(Summary::Modified) => columns[1] = b'M',
                Some(Summary::Conflict) => {
                    return Ok(RepositoryRead::GitRequired(
                        GitReadRequirement::UnmergedIndex,
                    ));
                },
                Some(Summary::IntentToAdd) => {
                    return Ok(RepositoryRead::GitRequired(GitReadRequirement::IntentToAdd));
                },
                Some(Summary::Renamed | Summary::Copied) => {
                    return Err(failure("read status", "rename tracking remained enabled"));
                },
                None => {},
            },
        }
    }
    let mut bytes = porcelain_status(paths);
    bytes.extend(porcelain_status(
        untracked.into_iter().map(|path| (path, *b"??")).collect(),
    ));
    Ok(RepositoryRead::InProcess(bytes))
}

/// Serialize tracked changes before untracked paths in Git's NUL-delimited format.
fn porcelain_status(paths: BTreeMap<Vec<u8>, [u8; 2]>) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut paths: Vec<_> = paths.into_iter().collect();
    paths.sort_by_key(|(_, columns)| *columns == *b"??");
    for (path, columns) in paths {
        if columns == *b"  " {
            continue;
        }
        bytes.extend_from_slice(&columns);
        bytes.push(b' ');
        bytes.extend_from_slice(&path);
        bytes.push(0);
    }
    bytes
}

/// Preserve main-first ordering, branch attachment, detached and unborn heads.
pub(super) fn worktrees(root: &Path) -> Result<RepositoryRead<Vec<u8>>, GitError> {
    let repository = match open(root)? {
        RepositoryRead::InProcess(repository) => repository,
        RepositoryRead::GitRequired(reason) => return Ok(RepositoryRead::GitRequired(reason)),
    };
    for proxy in repository
        .worktrees()
        .map_err(|error| failure("read worktree registry", error))?
    {
        if proxy.is_locked() || proxy.is_prunable() {
            return Ok(RepositoryRead::GitRequired(
                GitReadRequirement::ExtendedWorktreeRegistration,
            ));
        }
    }
    let mut bytes = Vec::new();
    let worktrees = repository
        .worktrees_including_main()
        .map_err(|error| failure("read worktree registry", error))?;
    let Ok(mut worktrees) = worktrees.collect::<Result<Vec<_>, _>>() else {
        return Ok(RepositoryRead::GitRequired(
            GitReadRequirement::ExtendedWorktreeRegistration,
        ));
    };
    // Git lists the main checkout first, then linked checkouts by their path.
    if worktrees.len() > 1 {
        worktrees[1..].sort_by(|left, right| left.workdir().cmp(&right.workdir()));
    }
    for (index, worktree) in worktrees.into_iter().enumerate() {
        let root = worktree.workdir().unwrap_or_else(|| worktree.git_dir());
        // Git resolves the main checkout's path; linked paths retain their registration spelling.
        let root = if index == 0 {
            std::fs::canonicalize(root).map_err(|error| failure("resolve main worktree", error))?
        } else {
            root.to_path_buf()
        };
        bytes.extend_from_slice(b"worktree ");
        bytes.extend_from_slice(root.as_os_str().as_encoded_bytes());
        bytes.push(0);
        if worktree.is_bare() {
            bytes.extend_from_slice(b"bare\0\0");
            continue;
        }
        let head = worktree
            .head()
            .map_err(|error| failure("read worktree HEAD", error))?;
        if matches!(&head.kind, gix::head::Kind::Symbolic(reference) if reference.target.try_id().is_none())
        {
            return Ok(RepositoryRead::GitRequired(
                GitReadRequirement::SymbolicReference,
            ));
        }
        bytes.extend_from_slice(b"HEAD ");
        let id = head.id().map_or_else(
            || gix::hash::ObjectId::null(worktree.object_hash()).to_string(),
            |id| id.to_string(),
        );
        bytes.extend_from_slice(id.as_bytes());
        bytes.push(0);
        match head.referent_name() {
            Some(name) => {
                bytes.extend_from_slice(b"branch ");
                bytes.extend_from_slice(name.as_bstr());
                bytes.push(0);
            },
            None => bytes.extend_from_slice(b"detached\0"),
        }
        bytes.push(0);
    }
    Ok(RepositoryRead::InProcess(bytes))
}

pub(super) fn attachment(root: &Path) -> Result<RepositoryRead<HeadAttachment>, GitError> {
    let repository = match open(root)? {
        RepositoryRead::InProcess(repository) => repository,
        RepositoryRead::GitRequired(reason) => return Ok(RepositoryRead::GitRequired(reason)),
    };
    let head = repository
        .head()
        .map_err(|error| failure("read HEAD attachment", error))?;
    if matches!(&head.kind, gix::head::Kind::Symbolic(reference) if reference.target.try_id().is_none())
    {
        return Ok(RepositoryRead::GitRequired(
            GitReadRequirement::SymbolicReference,
        ));
    }
    let attachment = match head.referent_name() {
        Some(name) => {
            let reference = name.as_bstr().to_string();
            HeadAttachment::Branch {
                full_ref: reference
                    .parse()
                    .map_err(|_| GitError::InvalidReferenceName { reference })?,
            }
        },
        None => HeadAttachment::Detached,
    };
    Ok(RepositoryRead::InProcess(attachment))
}

pub(super) fn reference(
    root: &Path,
    name: &str,
) -> Result<RepositoryRead<ReferenceLookup>, GitError> {
    let repository = match open(root)? {
        RepositoryRead::InProcess(repository) => repository,
        RepositoryRead::GitRequired(reason) => return Ok(RepositoryRead::GitRequired(reason)),
    };
    let reference = repository
        .try_find_reference(name)
        .map_err(|error| failure("read reference", error))?;
    match reference {
        Some(reference) => match reference.try_id() {
            Some(id) => Ok(RepositoryRead::InProcess(ReferenceLookup::Present(
                id.to_string().parse().map_err(GitError::InvalidObjectId)?,
            ))),
            None => Ok(RepositoryRead::GitRequired(
                GitReadRequirement::SymbolicReference,
            )),
        },
        None => Ok(RepositoryRead::InProcess(ReferenceLookup::Missing)),
    }
}

/// Read only exact full refs and HEAD here. Other expressions keep the shared Git batch.
pub(super) fn commit(
    root: &Path,
    expression: &str,
) -> Result<RepositoryRead<CommitObjectResolution>, GitError> {
    let reference_name = expression
        .strip_suffix(GIT_COMMIT_PEEL_SUFFIX)
        .unwrap_or(expression);
    if reference_name != GIT_HEAD_REVISION && !reference_name.starts_with("refs/") {
        return Ok(RepositoryRead::GitRequired(
            GitReadRequirement::RevisionExpression,
        ));
    }
    if reference_name
        .bytes()
        .any(|byte| matches!(byte, b'^' | b'~' | b'@' | b':' | b'*' | b'?'))
    {
        return Ok(RepositoryRead::GitRequired(
            GitReadRequirement::RevisionExpression,
        ));
    }
    let repository = match open(root)? {
        RepositoryRead::InProcess(repository) => repository,
        RepositoryRead::GitRequired(reason) => return Ok(RepositoryRead::GitRequired(reason)),
    };
    let id = if reference_name == GIT_HEAD_REVISION {
        let head = repository
            .head()
            .map_err(|error| failure("read HEAD", error))?;
        if matches!(&head.kind, gix::head::Kind::Symbolic(reference) if reference.target.try_id().is_none())
        {
            return Ok(RepositoryRead::GitRequired(
                GitReadRequirement::SymbolicReference,
            ));
        }
        head.id().map(gix::Id::detach)
    } else {
        match repository
            .try_find_reference(reference_name)
            .map_err(|error| failure("read commit reference", error))?
        {
            Some(reference) => match reference.try_id() {
                Some(id) => Some(id.detach()),
                None => {
                    return Ok(RepositoryRead::GitRequired(
                        GitReadRequirement::SymbolicReference,
                    ));
                },
            },
            None => None,
        }
    };
    let Some(id) = id else {
        return Ok(RepositoryRead::InProcess(CommitObjectResolution::Missing));
    };
    let Some(object) = repository
        .try_find_object(id)
        .map_err(|error| failure("read referenced object", error))?
    else {
        return Ok(RepositoryRead::InProcess(CommitObjectResolution::Missing));
    };
    let resolution = match object.kind {
        Kind::Commit => CommitObjectResolution::Resolved(
            id.to_string().parse().map_err(GitError::InvalidObjectId)?,
        ),
        Kind::Tag => {
            return Ok(RepositoryRead::GitRequired(
                GitReadRequirement::AnnotatedTag,
            ));
        },
        _ if reference_name != expression => {
            return Ok(RepositoryRead::GitRequired(
                GitReadRequirement::RevisionExpression,
            ));
        },
        kind => CommitObjectResolution::WrongType {
            object_type: kind.to_string(),
        },
    };
    Ok(RepositoryRead::InProcess(resolution))
}

/// Resolve configured tracking refs without treating an unconfigured upstream as a failed read.
pub(super) fn upstreams(
    root: &Path,
) -> Result<RepositoryRead<BTreeMap<String, BranchUpstream>>, GitError> {
    let repository = match open(root)? {
        RepositoryRead::InProcess(repository) => repository,
        RepositoryRead::GitRequired(reason) => return Ok(RepositoryRead::GitRequired(reason)),
    };
    let config = repository.config_snapshot();
    let platform = repository
        .references()
        .map_err(|error| failure("read branches", error))?;
    let branches = platform
        .prefixed(GIT_LOCAL_BRANCH_REF_PREFIX)
        .map_err(|error| failure("list branches", error))?;
    let mut upstreams = BTreeMap::new();
    for branch in branches {
        let branch = branch.map_err(|error| failure("read branch", error))?;
        let short = branch.name().shorten();
        if config
            .plumbing()
            .strings_by(
                GIT_BRANCH_CONFIGURATION_SECTION,
                Some(short),
                GIT_BRANCH_MERGE_KEY,
            )
            .is_some_and(|values| values.len() > 1)
        {
            return Ok(RepositoryRead::GitRequired(
                GitReadRequirement::MultipleUpstreamBranches,
            ));
        }
        let upstream = if config
            .plumbing()
            .string_by(
                GIT_BRANCH_CONFIGURATION_SECTION,
                Some(short),
                GIT_BRANCH_REMOTE_KEY,
            )
            .as_deref()
            .is_some_and(|value| value == GIT_LOCAL_REMOTE_VALUE.as_bytes())
        {
            match config.plumbing().string_by(
                GIT_BRANCH_CONFIGURATION_SECTION,
                Some(short),
                GIT_BRANCH_MERGE_KEY,
            ) {
                Some(name) if name == GIT_HEAD_REVISION.as_bytes() => {
                    return Ok(RepositoryRead::GitRequired(
                        GitReadRequirement::LocalUpstreamHead,
                    ));
                },
                Some(name) => BranchUpstream::from_reference(&name.to_string())?,
                None => BranchUpstream::Unconfigured,
            }
        } else {
            match branch.remote_tracking_ref_name(Direction::Fetch) {
                Some(Ok(name)) => BranchUpstream::from_reference(&name.as_bstr().to_string())?,
                Some(Err(error)) => return Err(failure("read tracking reference", error)),
                None => BranchUpstream::Unconfigured,
            }
        };
        upstreams.insert(branch.name().as_bstr().to_string(), upstream);
    }
    Ok(RepositoryRead::InProcess(upstreams))
}

/// Match direct local branch tips; symbolic refs and tag objects keep Git's peeling rules.
pub(super) fn branches_at(
    root: &Path,
    tip: &GitObjectId,
) -> Result<RepositoryRead<Vec<u8>>, GitError> {
    let repository = match open(root)? {
        RepositoryRead::InProcess(repository) => repository,
        RepositoryRead::GitRequired(reason) => return Ok(RepositoryRead::GitRequired(reason)),
    };
    let platform = repository
        .references()
        .map_err(|error| failure("read branches", error))?;
    let branches = platform
        .prefixed(GIT_LOCAL_BRANCH_REF_PREFIX)
        .map_err(|error| failure("list branches", error))?;
    let mut bytes = Vec::new();
    for branch in branches {
        let branch = branch.map_err(|error| failure("read branch", error))?;
        let Some(id) = branch.try_id() else {
            return Ok(RepositoryRead::GitRequired(
                GitReadRequirement::SymbolicReference,
            ));
        };
        let object = id
            .object()
            .map_err(|error| failure("read branch object", error))?;
        if object.kind == Kind::Tag {
            return Ok(RepositoryRead::GitRequired(
                GitReadRequirement::AnnotatedTag,
            ));
        }
        if id == tip.to_string() {
            bytes.extend_from_slice(branch.name().as_bstr());
            bytes.push(b'\n');
        }
    }
    Ok(RepositoryRead::InProcess(bytes))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::Duration;
    use std::time::Instant;

    use cargo_berth_test_support::TestProcessGroup;

    use super::GitReadRequirement;
    use super::RepositoryRead;
    use super::status;
    use super::worktrees;
    use crate::git::command;
    use crate::git::fixture::FixtureResult;
    use crate::git::fixture::PRIMARY_PATH;
    use crate::git::fixture::PatchEquivalenceFixture;
    use crate::git::object::BranchUpstream;
    use crate::git::object::CommitObjectResolution;
    use crate::git::object::OperationObjectReader;
    use crate::git::refs::ReferenceLookup;
    use crate::git::snapshot::RepositorySnapshot;

    enum EnvironmentTestRole {
        Child,
        ParentFinished,
    }

    fn in_process<T>(read: RepositoryRead<T>) -> Result<T, Box<dyn std::error::Error>> {
        match read {
            RepositoryRead::InProcess(value) => Ok(value),
            RepositoryRead::GitRequired(reason) => {
                Err(format!("unexpected Git route: {reason:?}").into())
            },
        }
    }

    fn git_status(fixture: &PatchEquivalenceFixture) -> FixtureResult<Vec<u8>> {
        let output = command::git_output(
            fixture.root(),
            [
                "status",
                "--porcelain",
                "-z",
                "--no-renames",
                "--untracked-files=all",
            ],
        )?;
        assert!(output.status.success(), "{output:?}");
        Ok(output.stdout)
    }

    fn assert_status_route(
        fixture: &PatchEquivalenceFixture,
        reason: GitReadRequirement,
    ) -> FixtureResult {
        assert!(
            matches!(status(fixture.root())?, RepositoryRead::GitRequired(actual) if actual == reason)
        );
        assert_eq!(
            RepositorySnapshot::discover(fixture.root())?
                .working_tree_status()?
                .porcelain,
            git_status(fixture)?
        );
        Ok(())
    }

    #[test]
    fn in_process_status_preserves_staged_unstaged_untracked_deleted_and_ignored_paths()
    -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "staged\n")?;
        fixture.git(&["add", PRIMARY_PATH])?;
        fixture.write(PRIMARY_PATH, "unstaged\n")?;
        fixture.remove("src/secondary.rs")?;
        fixture.write("new file\nwith newline", "untracked\n")?;
        fixture.write(".gitignore", "ignored.txt\n")?;
        fixture.write("ignored.txt", "ignored\n")?;
        assert_eq!(in_process(status(fixture.root())?)?, git_status(&fixture)?);
        Ok(())
    }

    #[test]
    fn in_process_status_reads_an_unborn_index_and_a_staged_deletion() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["rm", "src/secondary.rs"])?;
        assert_eq!(in_process(status(fixture.root())?)?, git_status(&fixture)?);
        fixture.git(&["checkout", "--orphan", "unborn"])?;
        assert_eq!(in_process(status(fixture.root())?)?, git_status(&fixture)?);
        Ok(())
    }

    #[test]
    fn untracked_files_are_reported_even_when_repository_status_hides_them() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["config", "status.showUntrackedFiles", "no"])?;
        fixture.write("untracked.txt", "untracked\n")?;
        let expected = git_status(&fixture)?;
        assert_eq!(expected, b"?? untracked.txt\0");
        assert_eq!(in_process(status(fixture.root())?)?, expected);
        Ok(())
    }

    #[test]
    fn removing_a_file_from_the_index_keeps_both_deleted_and_untracked_records() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["rm", "--cached", PRIMARY_PATH])?;
        let expected = git_status(&fixture)?;
        assert_eq!(expected, b"D  src/primary.rs\0?? src/primary.rs\0");
        assert_eq!(in_process(status(fixture.root())?)?, expected);
        Ok(())
    }

    #[test]
    fn chained_symbolic_head_reads_keep_git_commit_attachment_and_worktree_semantics()
    -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["symbolic-ref", "refs/heads/alias", "refs/heads/main"])?;
        fixture.git(&["symbolic-ref", "HEAD", "refs/heads/alias"])?;
        assert!(matches!(
            super::commit(fixture.root(), "HEAD")?,
            RepositoryRead::GitRequired(GitReadRequirement::SymbolicReference)
        ));
        assert!(matches!(
            super::attachment(fixture.root())?,
            RepositoryRead::GitRequired(GitReadRequirement::SymbolicReference)
        ));
        assert!(matches!(
            worktrees(fixture.root())?,
            RepositoryRead::GitRequired(GitReadRequirement::SymbolicReference)
        ));
        let snapshot = RepositorySnapshot::discover(fixture.root())?;
        let expected = command::git_output(fixture.root(), ["rev-parse", "HEAD"])?;
        assert!(expected.status.success(), "{expected:?}");
        assert_eq!(
            snapshot.head_object_id()?.to_string(),
            String::from_utf8(expected.stdout)?.trim()
        );
        let expected = command::git_output(fixture.root(), ["symbolic-ref", "--quiet", "HEAD"])?;
        assert!(expected.status.success(), "{expected:?}");
        let crate::git::refs::HeadAttachment::Branch { full_ref } = snapshot.head_attachment()?
        else {
            return Err("expected attached HEAD".into());
        };
        assert_eq!(
            full_ref.to_string(),
            String::from_utf8(expected.stdout)?.trim()
        );
        let expected =
            command::git_output(fixture.root(), ["worktree", "list", "--porcelain", "-z"])?;
        assert!(expected.status.success(), "{expected:?}");
        assert_eq!(snapshot.worktree_list_porcelain()?, expected.stdout);
        Ok(())
    }

    #[test]
    fn installation_configuration_excludes_the_same_paths_as_git() -> FixtureResult {
        const TEST: &str = "installation_configuration_excludes_the_same_paths_as_git";
        if std::env::var("CARGO_BERTH_READ_ENVIRONMENT_TEST").as_deref() == Ok(TEST) {
            let fixture = PatchEquivalenceFixture::new()?;
            fixture.write("installation.ignored", "ignored by installation only\n")?;
            fixture.write("visible.txt", "visible\n")?;
            let expected = git_status(&fixture)?;
            assert_eq!(expected, b"?? visible.txt\0");
            assert_eq!(in_process(status(fixture.root())?)?, expected);
            return Ok(());
        }
        // Select a private system configuration at the executable boundary, for both Git's
        // installation probe and its actual status command. No repository config sets the rule.
        let installation = tempfile::tempdir()?;
        let root = installation.path();
        std::fs::write(root.join("excludes"), "*.ignored\n")?;
        let config = root.join("gitconfig");
        let mut configure = cargo_berth_test_support::fixture_git_command();
        let output = configure
            .args(["config", "--file"])
            .arg(&config)
            .arg("core.excludesFile")
            .arg(root.join("excludes"))
            .output()?;
        assert!(output.status.success(), "{output:?}");
        let git = std::env::split_paths(&std::env::var_os("PATH").ok_or("PATH missing")?)
            .map(|path| path.join("git"))
            .find(|path| path.is_file())
            .ok_or("git missing")?;
        let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
        let executable = root.join("git");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nexport GIT_CONFIG_SYSTEM={}\nexec {} \"$@\"\n",
                quote(&config),
                quote(&git)
            ),
        )?;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
        let path = std::env::join_paths(std::iter::once(root.to_path_buf()).chain(
            std::env::split_paths(&std::env::var_os("PATH").ok_or("PATH missing")?),
        ))?;
        let output = std::process::Command::new(std::env::current_exe()?)
            .arg(TEST)
            .arg("--nocapture")
            .env("CARGO_BERTH_READ_ENVIRONMENT_TEST", TEST)
            .env("PATH", path)
            .env_remove("GIT_CONFIG_NOSYSTEM")
            .output()?;
        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("running 1 test"),
            "{output:?}"
        );
        Ok(())
    }

    #[test]
    fn installation_probe_seals_its_pipes_before_another_git_child_starts() -> FixtureResult {
        const TEST: &str = "installation_probe_seals_its_pipes_before_another_git_child_starts";
        if let Some(root) = std::env::var_os("CARGO_BERTH_PROBE_SERIALIZATION_ROOT") {
            return check_installation_probe_serialization(Path::new(&root));
        }
        let fixture = PatchEquivalenceFixture::new()?;
        let shim = tempfile::tempdir()?;
        let git = std::env::split_paths(&std::env::var_os("PATH").ok_or("PATH missing")?)
            .map(|path| path.join("git"))
            .find(|path| path.is_file())
            .ok_or("git missing")?;
        let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
        let executable = shim.path().join("git");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nif [ \"$1\" = config ]; then\n  : > {}\n  while [ ! -e {} ]; do sleep 0.005; done\nfi\nexec {} \"$@\"\n",
                quote(&shim.path().join("ready")),
                quote(&shim.path().join("continue")),
                quote(&git),
            ),
        )?;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
        let path = std::env::join_paths(std::iter::once(shim.path().to_path_buf()).chain(
            std::env::split_paths(&std::env::var_os("PATH").ok_or("PATH missing")?),
        ))?;
        let mut process = TestProcessGroup::spawn(
            std::process::Command::new(std::env::current_exe()?)
                .args([TEST, "--nocapture"])
                .env("CARGO_BERTH_PROBE_SERIALIZATION_ROOT", fixture.root())
                .env("CARGO_BERTH_PROBE_SERIALIZATION_SHIM", shim.path())
                .env("PATH", path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )?;
        let deadline = Instant::now() + Duration::from_millis(750);
        while process.child_mut().try_wait()?.is_none() {
            assert!(
                Instant::now() < deadline,
                "installation probe test did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let output = process.wait_with_output()?;
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("running 1 test"));
        Ok(())
    }

    fn check_installation_probe_serialization(root: &Path) -> FixtureResult {
        let shim = std::path::PathBuf::from(
            std::env::var_os("CARGO_BERTH_PROBE_SERIALIZATION_SHIM").ok_or("shim missing")?,
        );
        std::thread::scope(|scope| -> FixtureResult {
            let observation = scope.spawn(|| super::worktrees(root));
            let deadline = Instant::now() + Duration::from_millis(250);
            while !shim.join("ready").exists() {
                if Instant::now() >= deadline {
                    std::fs::write(shim.join("continue"), "")?;
                    return Err("installation probe did not start".into());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            let (started, attempting) = mpsc::channel();
            let (finished, completed) = mpsc::channel();
            let other = scope.spawn(move || {
                started.send(()).map_err(std::io::Error::other)?;
                let output = command::git_output(root, ["--version"]);
                finished.send(()).map_err(std::io::Error::other)?;
                output
            });
            let attempted = attempting.recv_timeout(Duration::from_millis(100));
            let premature = completed.recv_timeout(Duration::from_millis(25));
            // Always continue before asserting, so a failing proof also joins both workers.
            std::fs::write(shim.join("continue"), "")?;
            let observed = observation.join().map_err(|_| "observation panicked")?;
            let output = other.join().map_err(|_| "Git worker panicked")??;
            attempted?;
            assert!(
                premature.is_err(),
                "another child started during the installation probe"
            );
            assert!(output.status.success(), "{output:?}");
            assert_ne!(in_process(observed?)?, [] as [u8; 0]);
            Ok(())
        })
    }

    #[test]
    fn sha256_repository_status_uses_git_and_keeps_untracked_paths() -> FixtureResult {
        let repository = tempfile::tempdir()?;
        let root = repository.path();
        let output = command::git_output(root, ["init", "--object-format=sha256"])?;
        assert!(output.status.success(), "{output:?}");
        std::fs::write(root.join("untracked.txt"), "untracked\n")?;
        assert!(matches!(
            status(root)?,
            RepositoryRead::GitRequired(GitReadRequirement::Sha256Objects)
        ));
        assert_eq!(
            RepositorySnapshot::discover(root)?
                .working_tree_status()?
                .porcelain,
            b"?? untracked.txt\0"
        );
        Ok(())
    }

    #[test]
    fn reftable_reference_lookup_uses_git_and_preserves_present_and_missing_refs() -> FixtureResult
    {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["refs", "migrate", "--ref-format=reftable"])?;
        assert!(matches!(
            super::reference(fixture.root(), "refs/heads/main")?,
            RepositoryRead::GitRequired(GitReadRequirement::ReftableReferences)
        ));
        assert_eq!(
            crate::git::reference_lookup(fixture.root(), "refs/heads/main")?,
            ReferenceLookup::Present(fixture.phase_start_head.clone())
        );
        assert_eq!(
            crate::git::reference_lookup(fixture.root(), "refs/heads/missing")?,
            ReferenceLookup::Missing
        );
        Ok(())
    }

    #[test]
    fn filesystem_monitor_status_uses_git_and_keeps_dirty_paths() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["config", "core.fsmonitor", "true"])?;
        fixture.write(PRIMARY_PATH, "dirty\n")?;
        assert_status_route(&fixture, GitReadRequirement::FileSystemMonitor)
    }

    #[test]
    fn sparse_checkout_status_uses_git_and_keeps_visible_changes() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["sparse-checkout", "set", "--no-cone", "src/"])?;
        fixture.write(PRIMARY_PATH, "dirty\n")?;
        assert_status_route(&fixture, GitReadRequirement::SparseCheckout)
    }

    #[test]
    fn split_index_status_uses_git_and_keeps_staged_changes() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["config", "core.splitIndex", "true"])?;
        fixture.git(&["update-index", "--split-index"])?;
        fixture.write(PRIMARY_PATH, "dirty\n")?;
        fixture.git(&["add", PRIMARY_PATH])?;
        assert_status_route(&fixture, GitReadRequirement::SplitIndex)
    }

    #[test]
    fn submodule_status_uses_git_and_reports_the_gitlink() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let cacheinfo = format!("160000,{},submodule", fixture.phase_start_head);
        fixture.git(&["update-index", "--add", "--cacheinfo", &cacheinfo])?;
        assert_status_route(&fixture, GitReadRequirement::SubmoduleStatus)
    }

    #[test]
    fn conflicted_index_status_uses_git_and_preserves_conflict_columns() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["checkout", "-b", "other"])?;
        fixture.write(PRIMARY_PATH, "other\n")?;
        fixture.commit("other")?;
        fixture.git(&["checkout", "main"])?;
        fixture.write(PRIMARY_PATH, "main\n")?;
        fixture.commit("main")?;
        let output = command::git_output(fixture.root(), ["merge", "--no-ff", "other"])?;
        assert!(!output.status.success());
        assert!(git_status(&fixture)?.starts_with(b"UU "), "{output:?}");
        assert_status_route(&fixture, GitReadRequirement::UnmergedIndex)
    }

    #[test]
    fn intent_to_add_status_uses_git_and_keeps_the_unstaged_addition() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write("intent.txt", "intent\n")?;
        fixture.git(&["add", "--intent-to-add", "intent.txt"])?;
        assert_status_route(&fixture, GitReadRequirement::IntentToAdd)
    }

    #[test]
    fn in_process_worktrees_preserve_linked_detached_and_unborn_heads() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let parent = tempfile::tempdir()?;
        let linked = parent.path().join("linked path");
        fixture.git(&[
            "worktree",
            "add",
            "--detach",
            linked.to_str().ok_or("path encoding")?,
        ])?;
        let expected =
            command::git_output(fixture.root(), ["worktree", "list", "--porcelain", "-z"])?;
        assert!(expected.status.success());
        assert_eq!(in_process(worktrees(fixture.root())?)?, expected.stdout);
        fixture.git(&["checkout", "--orphan", "unborn"])?;
        let expected =
            command::git_output(fixture.root(), ["worktree", "list", "--porcelain", "-z"])?;
        assert_eq!(in_process(worktrees(fixture.root())?)?, expected.stdout);
        Ok(())
    }

    #[test]
    fn locked_worktree_registrations_keep_git_lock_reasons() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let parent = tempfile::tempdir()?;
        let linked = parent.path().join("locked path");
        let path = linked.to_str().ok_or("path encoding")?;
        fixture.git(&["worktree", "add", "--detach", path])?;
        fixture.git(&["worktree", "lock", "--reason", "external disk", path])?;
        assert_worktree_registration_route(&fixture)
    }

    #[test]
    fn prunable_worktree_registrations_keep_git_missing_checkout_diagnostics() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let parent = tempfile::tempdir()?;
        let linked = parent.path().join("missing path");
        fixture.git(&[
            "worktree",
            "add",
            "--detach",
            linked.to_str().ok_or("path encoding")?,
        ])?;
        std::fs::remove_dir_all(&linked)?;
        assert_worktree_registration_route(&fixture)
    }

    fn assert_worktree_registration_route(fixture: &PatchEquivalenceFixture) -> FixtureResult {
        assert!(matches!(
            worktrees(fixture.root())?,
            RepositoryRead::GitRequired(GitReadRequirement::ExtendedWorktreeRegistration)
        ));
        let expected =
            command::git_output(fixture.root(), ["worktree", "list", "--porcelain", "-z"])?;
        assert!(expected.status.success(), "{expected:?}");
        assert_eq!(
            RepositorySnapshot::discover(fixture.root())?.worktree_list_porcelain()?,
            expected.stdout
        );
        Ok(())
    }

    #[test]
    fn in_process_reference_lookup_distinguishes_packed_missing_and_failed_reads() -> FixtureResult
    {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["pack-refs", "--all"])?;
        assert_eq!(
            in_process(super::reference(fixture.root(), "refs/heads/main")?)?,
            ReferenceLookup::Present(fixture.phase_start_head.clone())
        );
        assert_eq!(
            in_process(super::reference(fixture.root(), "refs/heads/missing")?)?,
            ReferenceLookup::Missing
        );
        std::fs::write(
            fixture.root().join(".git/refs/heads/corrupt"),
            "not an object\n",
        )?;
        assert!(super::reference(fixture.root(), "refs/heads/corrupt").is_err());
        Ok(())
    }

    #[test]
    fn symbolic_reference_lookup_uses_git_without_peeling_annotated_objects() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["symbolic-ref", "refs/heads/alias", "refs/heads/main"])?;
        assert!(matches!(
            super::reference(fixture.root(), "refs/heads/alias")?,
            RepositoryRead::GitRequired(GitReadRequirement::SymbolicReference)
        ));
        assert_eq!(
            crate::git::reference_lookup(fixture.root(), "refs/heads/alias")?,
            ReferenceLookup::Present(fixture.phase_start_head)
        );
        Ok(())
    }

    #[test]
    fn annotated_tag_commit_reads_delegate_peeling_to_the_git_batch() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["tag", "--annotate", "annotated", "--message", "tag object"])?;
        assert!(matches!(
            super::commit(fixture.root(), "refs/tags/annotated^{commit}")?,
            RepositoryRead::GitRequired(GitReadRequirement::AnnotatedTag)
        ));
        let objects = OperationObjectReader::new(fixture.root());
        assert_eq!(
            objects.commit("refs/tags/annotated^{commit}")?,
            fixture.phase_start_head
        );
        assert!(
            matches!(objects.commit("refs/tags/annotated"), Err(crate::git::error::GitError::WrongCommitExpressionType { object_type, .. }) if object_type == "tag")
        );
        Ok(())
    }

    #[test]
    fn revision_expression_reads_keep_git_ancestry_and_missing_object_semantics() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["commit", "--allow-empty", "-m", "next"])?;
        assert!(matches!(
            super::commit(fixture.root(), "HEAD~1")?,
            RepositoryRead::GitRequired(GitReadRequirement::RevisionExpression)
        ));
        assert_eq!(
            OperationObjectReader::new(fixture.root()).commit("HEAD~1")?,
            fixture.phase_start_head
        );
        assert_eq!(
            in_process(super::commit(fixture.root(), "refs/heads/missing")?)?,
            CommitObjectResolution::Missing
        );
        Ok(())
    }

    #[test]
    fn staged_executable_changes_are_modifications_and_symlink_changes_are_type_changes()
    -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.set_executable(PRIMARY_PATH)?;
        fixture.git(&["add", PRIMARY_PATH])?;
        assert_eq!(in_process(status(fixture.root())?)?, git_status(&fixture)?);
        fixture.remove("src/secondary.rs")?;
        std::os::unix::fs::symlink("primary.rs", fixture.root().join("src/secondary.rs"))?;
        fixture.git(&["add", "src/secondary.rs"])?;
        assert_eq!(in_process(status(fixture.root())?)?, git_status(&fixture)?);
        Ok(())
    }

    #[test]
    fn worktree_listing_resolves_main_checkout_aliases_like_git() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let parent = tempfile::tempdir()?;
        let alias = parent.path().join("main alias");
        std::os::unix::fs::symlink(fixture.root(), &alias)?;
        let linked = parent.path().join("linked");
        fixture.git(&[
            "worktree",
            "add",
            "--detach",
            linked.to_str().ok_or("path encoding")?,
        ])?;
        for checkout in [&alias, &linked] {
            let expected =
                command::git_output(checkout, ["worktree", "list", "--porcelain", "-z"])?;
            assert!(expected.status.success());
            assert_eq!(in_process(worktrees(checkout)?)?, expected.stdout);
        }
        Ok(())
    }

    #[test]
    fn worktree_listing_orders_paths_independently_of_registration_names() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let parent = tempfile::tempdir()?;
        for path in [parent.path().join("a/z"), parent.path().join("z/a")] {
            std::fs::create_dir_all(path.parent().ok_or("parent path")?)?;
            fixture.git(&[
                "worktree",
                "add",
                "--detach",
                path.to_str().ok_or("path encoding")?,
            ])?;
        }
        let expected =
            command::git_output(fixture.root(), ["worktree", "list", "--porcelain", "-z"])?;
        assert!(expected.status.success());
        assert_eq!(in_process(worktrees(fixture.root())?)?, expected.stdout);
        Ok(())
    }

    #[test]
    fn tracking_reference_reads_preserve_remote_local_and_unconfigured_upstreams() -> FixtureResult
    {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["branch", "local"])?;
        fixture.git(&["branch", "unconfigured"])?;
        fixture.git(&["config", "branch.local.remote", "."])?;
        fixture.git(&["config", "branch.local.merge", "refs/heads/main"])?;
        fixture.git(&["config", "remote.origin.url", "."])?;
        fixture.git(&[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ])?;
        fixture.git(&["config", "branch.main.remote", "origin"])?;
        fixture.git(&["config", "branch.main.merge", "refs/heads/trunk"])?;
        let upstreams = in_process(super::upstreams(fixture.root())?)?;
        assert_eq!(
            upstreams["refs/heads/main"],
            BranchUpstream::Configured("refs/remotes/origin/trunk".parse()?)
        );
        assert_eq!(
            upstreams["refs/heads/local"],
            BranchUpstream::Configured("refs/heads/main".parse()?)
        );
        assert_eq!(
            upstreams["refs/heads/unconfigured"],
            BranchUpstream::Unconfigured
        );
        fixture.git(&["config", "--add", "branch.main.merge", "refs/heads/trunk"])?;
        assert!(matches!(
            super::upstreams(fixture.root())?,
            RepositoryRead::GitRequired(GitReadRequirement::MultipleUpstreamBranches)
        ));
        let objects = OperationObjectReader::new(fixture.root());
        assert_eq!(objects.upstream_references()?, &upstreams);
        assert!(matches!(
            crate::git::local_target_upstream(&objects, "unconfigured")?,
            crate::git::LocalTargetUpstream::Unavailable
        ));
        assert!(matches!(
            crate::git::local_target_upstream(&objects, "local")?,
            crate::git::LocalTargetUpstream::Available(commit) if commit == fixture.phase_start_head
        ));
        Ok(())
    }

    #[test]
    fn local_head_upstream_uses_git_resolution_instead_of_an_invalid_ref() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["config", "branch.main.remote", "."])?;
        fixture.git(&["config", "branch.main.merge", "HEAD"])?;
        let expected = command::git_output(
            fixture.root(),
            ["for-each-ref", "--format=%(upstream)", "refs/heads/main"],
        )?;
        assert!(expected.status.success(), "{expected:?}");
        let expected = String::from_utf8(expected.stdout)?;
        let objects = OperationObjectReader::new(fixture.root());
        assert_eq!(
            objects.upstream_references()?["refs/heads/main"],
            BranchUpstream::from_reference(expected.trim())?
        );
        assert!(matches!(
            super::upstreams(fixture.root())?,
            RepositoryRead::GitRequired(GitReadRequirement::LocalUpstreamHead)
        ));
        assert!(
            matches!(crate::git::local_target_upstream(&objects, "main")?, crate::git::LocalTargetUpstream::Available(commit) if commit == fixture.phase_start_head)
        );
        Ok(())
    }

    #[test]
    fn multiple_merge_refs_use_git_first_upstream_instead_of_the_last_config_value() -> FixtureResult
    {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["config", "branch.main.remote", "."])?;
        fixture.git(&["config", "--add", "branch.main.merge", "refs/heads/first"])?;
        fixture.git(&["config", "--add", "branch.main.merge", "refs/heads/second"])?;
        assert!(matches!(
            super::upstreams(fixture.root())?,
            RepositoryRead::GitRequired(GitReadRequirement::MultipleUpstreamBranches)
        ));
        assert_eq!(
            OperationObjectReader::new(fixture.root()).upstream_references()?["refs/heads/main"],
            BranchUpstream::Configured("refs/heads/first".parse()?)
        );
        Ok(())
    }

    #[test]
    fn direct_branch_tip_listing_and_detached_attachment_match_git() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["branch", "same"])?;
        let expected = command::git_output(
            fixture.root(),
            [
                "for-each-ref",
                &format!("--points-at={}", fixture.phase_start_head),
                "--format=%(refname)",
                "refs/heads/",
            ],
        )?;
        assert!(expected.status.success());
        assert_eq!(
            in_process(super::branches_at(
                fixture.root(),
                &fixture.phase_start_head
            )?)?,
            expected.stdout
        );
        assert!(matches!(
            in_process(super::attachment(fixture.root())?)?,
            crate::git::refs::HeadAttachment::Branch { .. }
        ));
        fixture.git(&["checkout", "--detach"])?;
        assert!(matches!(
            in_process(super::attachment(fixture.root())?)?,
            crate::git::refs::HeadAttachment::Detached
        ));
        Ok(())
    }

    fn environment_child(
        name: &str,
        environment: &[(&str, &str)],
    ) -> FixtureResult<EnvironmentTestRole> {
        if std::env::var("CARGO_BERTH_READ_ENVIRONMENT_TEST").as_deref() == Ok(name) {
            return Ok(EnvironmentTestRole::Child);
        }
        let output = std::process::Command::new(std::env::current_exe()?)
            .arg(name)
            .arg("--nocapture")
            .env("CARGO_BERTH_READ_ENVIRONMENT_TEST", name)
            .envs(environment.iter().copied())
            .output()?;
        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("running 1 test"),
            "{output:?}"
        );
        Ok(EnvironmentTestRole::ParentFinished)
    }

    #[test]
    fn inherited_repository_selectors_use_git_without_redirecting_the_requested_checkout()
    -> FixtureResult {
        if let Some(root) = std::env::var_os("CARGO_BERTH_READ_REQUESTED_ROOT") {
            let root = std::path::Path::new(&root);
            assert!(matches!(
                status(root)?,
                RepositoryRead::GitRequired(GitReadRequirement::RepositoryEnvironment)
            ));
            let snapshot = RepositorySnapshot::discover(root)?;
            let actual = &snapshot.working_tree_status()?.porcelain;
            let expected = command::git_output(
                root,
                [
                    "status",
                    "--porcelain",
                    "-z",
                    "--no-renames",
                    "--untracked-files=all",
                ],
            )?;
            assert!(expected.status.success(), "{expected:?}");
            assert_eq!(actual, &expected.stdout);
            assert_eq!(actual.as_slice(), b" M src/primary.rs\0");
            return Ok(());
        }
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "requested checkout\n")?;
        environment_child(
            "inherited_repository_selectors_use_git_without_redirecting_the_requested_checkout",
            &[
                ("GIT_DIR", "/missing/inherited/repository"),
                (
                    "CARGO_BERTH_READ_REQUESTED_ROOT",
                    fixture.root().to_str().ok_or("fixture path encoding")?,
                ),
            ],
        )?;
        Ok(())
    }

    #[test]
    fn maintenance_configuration_keeps_status_and_worktree_reads_in_process() -> FixtureResult {
        if matches!(
            environment_child(
                "maintenance_configuration_keeps_status_and_worktree_reads_in_process",
                &[
                    ("GIT_CONFIG_COUNT", "2"),
                    ("GIT_CONFIG_KEY_0", "maintenance.auto"),
                    ("GIT_CONFIG_VALUE_0", "false"),
                    ("GIT_CONFIG_KEY_1", "gc.auto"),
                    ("GIT_CONFIG_VALUE_1", "0"),
                ],
            )?,
            EnvironmentTestRole::ParentFinished
        ) {
            return Ok(());
        }
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write(PRIMARY_PATH, "dirty\n")?;
        assert_eq!(in_process(status(fixture.root())?)?, git_status(&fixture)?);
        let expected =
            command::git_output(fixture.root(), ["worktree", "list", "--porcelain", "-z"])?;
        assert!(expected.status.success(), "{expected:?}");
        assert_eq!(in_process(worktrees(fixture.root())?)?, expected.stdout);
        Ok(())
    }

    #[test]
    fn empty_configuration_overlay_keeps_reference_reads_in_process() -> FixtureResult {
        if matches!(
            environment_child(
                "empty_configuration_overlay_keeps_reference_reads_in_process",
                &[("GIT_CONFIG_COUNT", "0")],
            )?,
            EnvironmentTestRole::ParentFinished
        ) {
            return Ok(());
        }
        let fixture = PatchEquivalenceFixture::new()?;
        assert_eq!(
            in_process(super::reference(fixture.root(), "refs/heads/main")?)?,
            ReferenceLookup::Present(fixture.phase_start_head.clone())
        );
        assert_eq!(
            in_process(super::reference(fixture.root(), "refs/heads/missing")?)?,
            ReferenceLookup::Missing
        );
        Ok(())
    }

    #[test]
    fn inherited_configuration_uses_git_and_preserves_its_status_rules() -> FixtureResult {
        if matches!(
            environment_child(
                "inherited_configuration_uses_git_and_preserves_its_status_rules",
                &[
                    ("GIT_CONFIG_COUNT", "1"),
                    ("GIT_CONFIG_KEY_0", "core.filemode"),
                    ("GIT_CONFIG_VALUE_0", "false"),
                ],
            )?,
            EnvironmentTestRole::ParentFinished
        ) {
            return Ok(());
        }
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.set_executable(PRIMARY_PATH)?;
        assert_status_route(&fixture, GitReadRequirement::ConfigurationEnvironment)
    }
}
