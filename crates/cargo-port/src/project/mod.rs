mod cargo;
mod cargo_metadata_store;
mod git;
mod info;
mod language;
mod member_group;
mod non_rust;
mod package;
mod paths;
mod project_entry;
mod project_fields;
mod root_item;
mod rust_info;
mod rust_project;
mod submodule;
mod vendored_package;
mod workspace;
mod worktree_group;

// ── Cargo parsing ────────────────────────────────────────────────────
pub(crate) use cargo::CargoParseResult;
pub(crate) use cargo::ExampleGroup;
pub(crate) use cargo::ProjectType;
pub(crate) use cargo::from_cargo_toml;
pub(crate) use cargo::from_git_dir;
// ── Cargo metadata cache ─────────────────────────────────────────────
pub(crate) use cargo_metadata_store::FileStamp;
pub(crate) use cargo_metadata_store::ManifestFingerprint;
pub(crate) use cargo_metadata_store::PackageRecord;
pub(crate) use cargo_metadata_store::PublishPolicy;
pub(crate) use cargo_metadata_store::TargetRecord;
pub(crate) use cargo_metadata_store::WorkspaceMetadata;
pub(crate) use cargo_metadata_store::WorkspaceMetadataHandle;
pub(crate) use cargo_metadata_store::WorkspaceMetadataStore;
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
#[cfg(test)]
pub(crate) use git::WorkflowPresence;
pub(crate) use git::WorktreeStatus;
pub(crate) use git::get_first_commit;
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
// ── Language helpers ────────────────────────────────────────────────
pub(crate) use language::language_icon;
// ── Project types ────────────────────────────────────────────────────
pub(crate) use member_group::MemberGroup;
pub(crate) use non_rust::NonRustProject;
pub(crate) use package::Package;
// ── Path types ───────────────────────────────────────────────────────
pub(crate) use paths::AbsolutePath;
pub(crate) use paths::DisplayPath;
pub(crate) use paths::home_relative_path;
pub(crate) use project_entry::ProjectEntry;
pub(crate) use project_entry::entry_contains;
pub(crate) use project_fields::ProjectFields;
pub(crate) use root_item::RootItem;
// ── Rust info ────────────────────────────────────────────────────────
pub(crate) use rust_info::Cargo;
pub(crate) use rust_info::RustInfo;
pub(crate) use rust_project::RustProject;
// ── Submodule types ─────────────────────────────────────────────────
pub(crate) use submodule::Submodule;
pub(crate) use submodule::get_submodules;
pub(crate) use vendored_package::VendoredPackage;
pub(crate) use workspace::Workspace;
pub(crate) use worktree_group::WorktreeGroup;
