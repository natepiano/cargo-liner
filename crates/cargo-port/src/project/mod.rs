mod cargo;
mod git;
mod member_group;
mod paths;
mod types;

// ── Path types ───────────────────────────────────────────────────────
// ── Cargo parsing ────────────────────────────────────────────────────
pub(crate) use cargo::CargoParseResult;
pub(crate) use cargo::ExampleGroup;
pub(crate) use cargo::ProjectType;
pub(crate) use cargo::from_cargo_toml;
pub(crate) use cargo::from_git_dir;
// ── Git types and functions ──────────────────────────────────────────
pub(crate) use git::GitInfo;
pub(crate) use git::GitOrigin;
pub(crate) use git::GitPathState;
pub(crate) use git::GitRepoPresence;
#[cfg(test)]
pub(crate) use git::WorkflowPresence;
pub(crate) use git::detect_first_commit;
pub(crate) use git::detect_git_path_state;
pub(crate) use git::detect_git_path_states_batch;
pub(crate) use git::git_repo_root;
pub(crate) use git::resolve_common_git_dir;
pub(crate) use git::resolve_git_dir;
pub(crate) use paths::AbsolutePath;
pub(crate) use paths::DisplayPath;
pub(crate) use paths::home_relative_path;
// ── Core project types ───────────────────────────────────────────────
pub(crate) use types::Cargo;
pub(crate) use types::CargoKind;
pub(crate) use types::MemberGroup;
pub(crate) use types::NonRustProject;
pub(crate) use types::Package;
pub(crate) use types::ProjectInfo;
pub(crate) use types::RootItem;
pub(crate) use types::RustProject;
pub(crate) use types::Visibility;
pub(crate) use types::Workspace;
pub(crate) use types::WorktreeGroup;
pub(crate) use types::WorktreeHealth;
