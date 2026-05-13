mod cargo;
mod git;
mod info;
mod member_group;
mod non_rust;
mod paths;
mod project_entry;
mod project_fields;
mod root_item;
mod vendored_package;

// ── Cargo parsing ────────────────────────────────────────────────────
// ── Rust info ────────────────────────────────────────────────────────
pub(crate) use cargo::Cargo;
pub(crate) use cargo::CargoParseResult;
pub(crate) use cargo::ExampleGroup;
// ── Cargo metadata cache ─────────────────────────────────────────────
pub(crate) use cargo::FileStamp;
pub(crate) use cargo::ManifestFingerprint;
// ── Project types ────────────────────────────────────────────────────
pub(crate) use cargo::Package;
pub(crate) use cargo::PackageRecord;
pub(crate) use cargo::ProjectType;
pub(crate) use cargo::PublishPolicy;
pub(crate) use cargo::RustInfo;
pub(crate) use cargo::RustProject;
pub(crate) use cargo::TargetRecord;
pub(crate) use cargo::Workspace;
pub(crate) use cargo::WorkspaceMetadata;
pub(crate) use cargo::WorkspaceMetadataStore;
pub(crate) use cargo::from_cargo_toml;
pub(crate) use cargo::from_git_dir;
// ── Git types and functions ──────────────────────────────────────────
pub(crate) use git::CheckoutInfo;
pub(crate) use git::GitOrigin;
pub(crate) use git::GitRepoPresence;
pub(crate) use git::GitStatus;
pub(crate) use git::LocalGitState;
#[cfg(test)]
pub(crate) use git::RemoteInfo;
pub(crate) use git::RemoteKind;
pub(crate) use git::RepoInfo;
// ── Submodule types ─────────────────────────────────────────────────
pub(crate) use git::Submodule;
#[cfg(test)]
pub(crate) use git::WorkflowPresence;
pub(crate) use git::WorktreeGroup;
pub(crate) use git::WorktreeStatus;
pub(crate) use git::get_first_commit;
pub(crate) use git::get_submodules;
pub(crate) use git::git_repo_root;
pub(crate) use git::resolve_common_git_dir;
pub(crate) use git::resolve_git_dir;
pub(crate) use git::worktree_ahead_behind_primary;
// ── Info types ───────────────────────────────────────────────────────
pub(crate) use info::GitHubInfo;
pub(crate) use info::LangEntry;
pub(crate) use info::LanguageStats;
pub(crate) use info::ProjectCiData;
pub(crate) use info::ProjectCiInfo;
pub(crate) use info::ProjectInfo;
pub(crate) use info::Visibility;
pub(crate) use info::WorktreeHealth;
pub(crate) use member_group::MemberGroup;
pub(crate) use non_rust::NonRustProject;
// ── Path types ───────────────────────────────────────────────────────
pub(crate) use paths::AbsolutePath;
pub(crate) use paths::DisplayPath;
pub(crate) use paths::home_relative_path;
pub(crate) use project_entry::ProjectEntry;
pub(crate) use project_entry::entry_contains;
pub(crate) use project_fields::ProjectFields;
pub(crate) use root_item::RootItem;
pub(crate) use vendored_package::VendoredPackage;
