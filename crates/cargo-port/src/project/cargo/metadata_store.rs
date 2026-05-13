//! Typed cache of `cargo metadata` output, keyed by workspace root.
//!
//! Holds one [`WorkspaceMetadata`] per detected workspace. Defines
//! the structure and read-side access for the `cargo_metadata`
//! integration.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use cargo_metadata::PackageId;
use cargo_metadata::TargetKind;
use cargo_metadata::semver::Version;
use sha2::Digest as _;

use crate::project::AbsolutePath;

/// Process-wide cache of `cargo metadata` results, keyed by workspace root.
///
/// Populated by [`BackgroundMsg::CargoMetadata`](crate::scan::BackgroundMsg)
/// arrivals. Callers that want read-only access should go through
/// `App::resolve_metadata` / `App::resolve_target_dir` rather than touching
/// this type directly.
#[derive(Debug, Default)]
pub(crate) struct WorkspaceMetadataStore {
    pub(crate) by_root:              HashMap<AbsolutePath, WorkspaceMetadata>,
    /// Per-workspace monotonic counter. Every dispatch bumps the counter
    /// and stamps the spawned work with the new value; arrivals only
    /// commit if their stamp still matches the current counter. This
    /// coalesces rapid edits to a single accepted result.
    pub(crate) dispatch_generations: HashMap<AbsolutePath, u64>,
}

impl WorkspaceMetadataStore {
    pub(crate) fn new() -> Self { Self::default() }

    /// Look up the metadata for the workspace whose root is `workspace_root`.
    pub(crate) fn get(&self, workspace_root: &AbsolutePath) -> Option<&WorkspaceMetadata> {
        self.by_root.get(workspace_root)
    }

    /// Walk `path`'s ancestors and return the first one that has metadata.
    /// Enables callers to resolve `target_directory` from any path inside a
    /// known workspace without having to find the workspace root themselves.
    pub(crate) fn containing_workspace_root(&self, path: &AbsolutePath) -> Option<&AbsolutePath> {
        let mut cursor: Option<&Path> = Some(path.as_path());
        while let Some(p) = cursor {
            if let Some((root, _)) = self.by_root.iter().find(|(root, _)| root.as_path() == p) {
                return Some(root);
            }
            cursor = p.parent();
        }
        None
    }

    /// Resolve the owning workspace's `target_directory` for any `path`
    /// inside a known workspace. Returns `None` when no metadata covers
    /// `path` yet; callers should fall back to `<project_root>/target`.
    /// This is the lock-free core of [`crate::tui::App::resolve_target_dir`].
    pub(crate) fn resolved_target_dir(&self, path: &AbsolutePath) -> Option<&AbsolutePath> {
        let root = self.containing_workspace_root(path)?;
        self.by_root.get(root).map(|snap| &snap.target_directory)
    }

    /// Look up the [`PackageRecord`] whose manifest sits at
    /// `<path>/Cargo.toml` — i.e. the package whose root directory is
    /// `path`. Works for standalone packages (where `path` is the
    /// workspace root) and for workspace members (where `path` is a
    /// member dir under the workspace root). Returns `None` when no
    /// metadata covers `path` or when no package in that metadata
    /// matches — the latter happens transiently when a manifest has
    /// been edited and the follow-up `cargo metadata` hasn't landed
    /// yet, so callers should treat `None` as "Loading…".
    pub(crate) fn package_for_path(&self, path: &AbsolutePath) -> Option<&PackageRecord> {
        let root = self.containing_workspace_root(path)?;
        let snap = self.by_root.get(root)?;
        let expected_manifest = path.as_path().join("Cargo.toml");
        snap.packages
            .values()
            .find(|pkg| pkg.manifest_path.as_path() == expected_manifest)
    }

    /// Insert or replace the metadata for `workspace_root`.
    pub(crate) fn upsert(&mut self, workspace_metadata: WorkspaceMetadata) {
        self.by_root.insert(
            workspace_metadata.workspace_root.clone(),
            workspace_metadata,
        );
    }

    /// Stamp the cached out-of-tree target size onto an existing metadata
    /// entry. No-op when `workspace_root` has no metadata (it may have
    /// been replaced between dispatch and arrival) or when the entry's
    /// current `target_directory` no longer matches `target_dir` (a follow-
    /// up `cargo metadata` redirected the target before the walk landed).
    pub(crate) fn set_out_of_tree_target_bytes(
        &mut self,
        workspace_root: &AbsolutePath,
        target_dir: &AbsolutePath,
        bytes: u64,
    ) -> bool {
        let Some(snap) = self.by_root.get_mut(workspace_root) else {
            return false;
        };
        if snap.target_directory != *target_dir {
            return false;
        }
        snap.out_of_tree_target_bytes = Some(bytes);
        true
    }

    /// Bump the dispatch generation for `workspace_root` and return the new
    /// value. Callers should stamp the spawned work with this value and use
    /// [`Self::is_current_generation`] at merge time.
    pub(crate) fn next_generation(&mut self, workspace_root: &AbsolutePath) -> u64 {
        let slot = self
            .dispatch_generations
            .entry(workspace_root.clone())
            .or_default();
        *slot = slot.saturating_add(1);
        *slot
    }

    /// `true` when the captured `generation` still matches the latest
    /// dispatch for `workspace_root`. Arrivals that fail this check are
    /// stale and should be discarded.
    pub(crate) fn is_current_generation(
        &self,
        workspace_root: &AbsolutePath,
        generation: u64,
    ) -> bool {
        self.dispatch_generations.get(workspace_root).copied() == Some(generation)
    }
}

/// A single workspace's resolved `cargo metadata` output.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceMetadata {
    pub workspace_root:           AbsolutePath,
    pub target_directory:         AbsolutePath,
    pub packages:                 HashMap<PackageId, PackageRecord>,
    pub fingerprint:              ManifestFingerprint,
    /// Byte size of `target_directory` when it lives *outside*
    /// `workspace_root` (a sharer — e.g. redirected by
    /// `CARGO_TARGET_DIR` or an ancestor `.cargo/config.toml`).
    /// `None` for in-tree targets (the scan walker's
    /// `in_project_target` covers those) and until the
    /// out-of-tree walk has reported back. Populated by
    /// [`crate::scan::BackgroundMsg::OutOfTreeTargetSize`] arrivals
    /// via `WorkspaceMetadataStore::set_out_of_tree_target_bytes`.
    pub out_of_tree_target_bytes: Option<u64>,
}

/// Normalized form of a single package's metadata. Field structure mirrors
/// `cargo_metadata::Package` but keep only the bits the UI and query paths
/// actually need.
#[derive(Clone, Debug)]
pub(crate) struct PackageRecord {
    pub name:          String,
    pub version:       Version,
    pub edition:       String,
    pub description:   Option<String>,
    pub license:       Option<String>,
    pub homepage:      Option<String>,
    pub repository:    Option<String>,
    pub manifest_path: AbsolutePath,
    pub targets:       Vec<TargetRecord>,
    pub publish:       PublishPolicy,
}

/// Publish policy, normalized from `Cargo.toml`'s `publish` field. Cargo
/// encodes three states: any-registry (omitted), never (`publish = false`),
/// and allowlisted (`publish = ["crates-io", ...]`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PublishPolicy {
    Any,
    Never,
    Registries(Vec<String>),
}

impl PublishPolicy {
    /// Construct from `cargo_metadata::Package::publish` — `None` means any
    /// registry, `Some(empty)` is the canonical `publish = false`, and a
    /// non-empty list is an allowlist.
    pub(crate) fn from_cargo_publish(raw: Option<&[String]>) -> Self {
        match raw {
            None => Self::Any,
            Some([]) => Self::Never,
            Some(list) => Self::Registries(list.to_vec()),
        }
    }
}

/// A single build target (bin, lib, example, test, bench, proc-macro, …).
#[derive(Clone, Debug)]
pub(crate) struct TargetRecord {
    pub name:     String,
    pub kinds:    Vec<TargetKind>,
    pub src_path: AbsolutePath,
}

/// Inputs whose bytes determine a cargo-metadata invocation's result.
/// Equality on the whole fingerprint decides whether a pending metadata
/// spawn is still relevant or needs to be discarded.
///
/// See `docs/cargo_metadata.md` → `ManifestFingerprint` for the full
/// rationale (content-hash authoritative, `(mtime, len)` as a pre-check,
/// inode deliberately omitted).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManifestFingerprint {
    pub manifest:       FileStamp,
    pub lockfile:       Option<FileStamp>,
    pub rust_toolchain: Option<FileStamp>,
    /// Every ancestor `.cargo/config[.toml]` candidate path, recorded as
    /// present (`Some`) or absent (`None`). A `None → Some` transition is
    /// itself a diff and invalidates the cached metadata.
    pub configs:        BTreeMap<PathBuf, Option<FileStamp>>,
}

/// Fingerprint for a single file. Equality is the SHA-256 of the file's
/// bytes. Inode is deliberately omitted — editor atomic-save workflows
/// rotate inodes on every save even when bytes are identical.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileStamp {
    pub content_hash: [u8; 32],
}

impl FileStamp {
    /// Read `path` and compute its stamp. SHA-256 of the bytes is the
    /// authoritative identity.
    pub(crate) fn from_path(path: &Path) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        let mut content_hash = [0_u8; 32];
        content_hash.copy_from_slice(&digest);
        Ok(Self { content_hash })
    }
}

impl ManifestFingerprint {
    /// Capture every input that can affect `cargo metadata`'s output for a
    /// workspace: the manifest, the lockfile, an optional `rust-toolchain`
    /// file, and every ancestor `.cargo/config[.toml]` candidate up to
    /// `CARGO_HOME`.
    ///
    /// Missing files are represented as `None` in [`Self::configs`]; a
    /// `None → Some` transition on any config slot is a real change and
    /// invalidates the cached metadata even though no tracked-file edit occurred.
    pub(crate) fn capture(workspace_root: &Path) -> io::Result<Self> {
        let manifest = FileStamp::from_path(&workspace_root.join("Cargo.toml"))?;
        let lockfile = optional_stamp(&workspace_root.join("Cargo.lock"))?;
        let rust_toolchain = toolchain_stamp(workspace_root)?;
        let configs = capture_config_chain(workspace_root)?;
        Ok(Self {
            manifest,
            lockfile,
            rust_toolchain,
            configs,
        })
    }
}

fn optional_stamp(path: &Path) -> io::Result<Option<FileStamp>> {
    match FileStamp::from_path(path) {
        Ok(stamp) => Ok(Some(stamp)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Find `rust-toolchain.toml` or `rust-toolchain` in `workspace_root` or any
/// ancestor up to (and including) the filesystem root. Cargo uses the first
/// match it finds walking upward, and toolchain selection can shift
/// `target_directory` (e.g. via `[build] target = ...`), so the file is
/// part of the fingerprint.
fn toolchain_stamp(workspace_root: &Path) -> io::Result<Option<FileStamp>> {
    for ancestor in workspace_root.ancestors() {
        for name in ["rust-toolchain.toml", "rust-toolchain"] {
            let candidate = ancestor.join(name);
            if let Some(stamp) = optional_stamp(&candidate)? {
                return Ok(Some(stamp));
            }
        }
    }
    Ok(None)
}

/// Walk from `workspace_root` up to `CARGO_HOME`, recording a `FileStamp`
/// for every `.cargo/config` and `.cargo/config.toml` candidate path —
/// present files as `Some`, absent as `None`. Both legal filenames are
/// probed at each level; the `BTreeMap` keeps the keyset stable across
/// runs for equality comparisons.
fn capture_config_chain(workspace_root: &Path) -> io::Result<BTreeMap<PathBuf, Option<FileStamp>>> {
    let cargo_home = resolve_cargo_home();
    let mut configs = BTreeMap::new();
    let mut visited: Option<PathBuf> = None;

    for ancestor in workspace_root.ancestors() {
        for name in [".cargo/config.toml", ".cargo/config"] {
            let candidate = ancestor.join(name);
            configs.insert(candidate.clone(), optional_stamp(&candidate)?);
        }
        visited = Some(ancestor.to_path_buf());
        if cargo_home.as_ref().is_some_and(|home| ancestor == home) {
            return Ok(configs);
        }
    }

    // Ancestor walk never reached `CARGO_HOME` (e.g. it's outside the
    // project's ancestor chain). Probe it explicitly so a user-wide
    // `~/.cargo/config.toml` still participates in the fingerprint.
    if let Some(home) = cargo_home
        && visited.as_deref() != Some(home.as_path())
    {
        for name in ["config.toml", "config"] {
            let candidate = home.join(name);
            configs.insert(candidate.clone(), optional_stamp(&candidate)?);
        }
    }
    Ok(configs)
}

/// Resolve `CARGO_HOME`: explicit env var wins; otherwise fall back to
/// `$HOME/.cargo`. Returns `None` only when neither is available — rare
/// enough that we tolerate an incomplete config chain rather than failing
/// the whole capture.
fn resolve_cargo_home() -> Option<PathBuf> {
    if let Ok(home) = env::var("CARGO_HOME")
        && !home.is_empty()
    {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir().map(|home| home.join(".cargo"))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn write_file(path: &Path, body: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn equality_is_content_hash_only() {
        let a = FileStamp {
            content_hash: [7; 32],
        };
        let b = FileStamp {
            content_hash: [7; 32],
        };
        assert_eq!(a, b, "same hash → equal");

        let c = FileStamp {
            content_hash: [8; 32],
        };
        assert_ne!(a, c, "different hash → unequal");
    }

    #[test]
    fn same_bytes_produce_same_content_hash() {
        let tmp = TempDir::new().unwrap();
        let first_path = tmp.path().join("a.toml");
        let second_path = tmp.path().join("b.toml");
        write_file(&first_path, b"payload");
        write_file(&second_path, b"payload");

        let first = FileStamp::from_path(&first_path).unwrap();
        let second = FileStamp::from_path(&second_path).unwrap();
        assert_eq!(
            first.content_hash, second.content_hash,
            "identical bytes hash identically"
        );
    }

    #[test]
    fn identical_bytes_written_via_rename_stay_equal() {
        // Simulates an editor's atomic-save: write to a temp then rename.
        // Inode changes, but bytes are identical — fingerprint must NOT
        // invalidate.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("file.toml");
        write_file(&path, b"payload");
        let first = FileStamp::from_path(&path).unwrap();

        let tmp_path = tmp.path().join("file.toml.new");
        write_file(&tmp_path, b"payload");
        fs::rename(&tmp_path, &path).unwrap();

        let second = FileStamp::from_path(&path).unwrap();
        assert_eq!(
            first, second,
            "same bytes → same stamp, even after rename-save"
        );
    }

    #[test]
    fn config_chain_records_absent_files_as_none() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("ws");
        write_file(&workspace.join("Cargo.toml"), b"[package]\nname=\"x\"\n");

        let fp = ManifestFingerprint::capture(&workspace).unwrap();
        assert!(
            fp.configs.values().any(Option::is_none),
            "absent configs are recorded as None, not omitted"
        );
    }

    #[test]
    fn config_chain_none_to_some_invalidates() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("ws");
        write_file(&workspace.join("Cargo.toml"), b"[package]\nname=\"x\"\n");

        let before = ManifestFingerprint::capture(&workspace).unwrap();

        // Creating a .cargo/config.toml somewhere in the ancestor chain
        // must register as a change — the new Some file stamp replaces a
        // previous None slot.
        write_file(&workspace.join(".cargo/config.toml"), b"[build]\n");
        let after = ManifestFingerprint::capture(&workspace).unwrap();

        assert_ne!(
            before, after,
            "None → Some on a tracked config slot invalidates"
        );
    }

    #[test]
    fn next_generation_is_strictly_monotonic() {
        let mut store = WorkspaceMetadataStore::new();
        let root = AbsolutePath::from(PathBuf::from("/ws"));
        let g1 = store.next_generation(&root);
        let g2 = store.next_generation(&root);
        let g3 = store.next_generation(&root);
        assert!(g1 < g2 && g2 < g3);
    }

    #[test]
    fn is_current_generation_rejects_stale_stamps() {
        let mut store = WorkspaceMetadataStore::new();
        let root = AbsolutePath::from(PathBuf::from("/ws"));
        let gen_a = store.next_generation(&root);
        assert!(store.is_current_generation(&root, gen_a));
        let _gen_b = store.next_generation(&root);
        assert!(
            !store.is_current_generation(&root, gen_a),
            "older generation no longer current after a new dispatch"
        );
    }

    fn fake_metadata(
        workspace_root: AbsolutePath,
        target_directory: AbsolutePath,
    ) -> WorkspaceMetadata {
        WorkspaceMetadata {
            workspace_root,
            target_directory,
            packages: std::collections::HashMap::new(),
            fingerprint: ManifestFingerprint {
                manifest:       FileStamp {
                    content_hash: [0_u8; 32],
                },
                lockfile:       None,
                rust_toolchain: None,
                configs:        BTreeMap::new(),
            },
            out_of_tree_target_bytes: None,
        }
    }

    #[test]
    fn resolved_target_dir_is_none_without_metadata() {
        let store = WorkspaceMetadataStore::new();
        let path = AbsolutePath::from(PathBuf::from("/ws/src/lib.rs"));
        assert!(
            store.resolved_target_dir(&path).is_none(),
            "no metadata → None; callers fall back to <project>/target"
        );
    }

    #[test]
    fn resolved_target_dir_returns_target_for_workspace_root() {
        let mut store = WorkspaceMetadataStore::new();
        let root = AbsolutePath::from(PathBuf::from("/ws"));
        let target = AbsolutePath::from(PathBuf::from("/tmp/out-of-tree-target"));
        store.upsert(fake_metadata(root.clone(), target.clone()));

        assert_eq!(
            store.resolved_target_dir(&root).cloned(),
            Some(target),
            "exact-match workspace root resolves its own target_directory"
        );
    }

    #[test]
    fn resolved_target_dir_walks_ancestors_from_member_or_worktree_paths() {
        let mut store = WorkspaceMetadataStore::new();
        let root = AbsolutePath::from(PathBuf::from("/ws"));
        let target = AbsolutePath::from(PathBuf::from("/tmp/out-of-tree-target"));
        store.upsert(fake_metadata(root, target.clone()));

        let member = AbsolutePath::from(PathBuf::from("/ws/crates/core/src/lib.rs"));
        assert_eq!(
            store.resolved_target_dir(&member).cloned(),
            Some(target),
            "member paths resolve via ancestor walk up to the workspace root"
        );
    }

    fn fake_package_record(name: &str, manifest_path: AbsolutePath) -> (PackageId, PackageRecord) {
        let id = PackageId {
            repr: format!("{name}-test-id"),
        };
        let record = PackageRecord {
            name: name.into(),
            version: Version::new(0, 1, 0),
            edition: "2021".into(),
            description: None,
            license: Some("MIT".into()),
            homepage: None,
            repository: Some(format!("https://example.test/{name}")),
            manifest_path,
            targets: Vec::new(),
            publish: PublishPolicy::Any,
        };
        (id, record)
    }

    #[test]
    fn package_for_path_is_none_without_metadata() {
        let store = WorkspaceMetadataStore::new();
        let path = AbsolutePath::from(PathBuf::from("/ws"));
        assert!(store.package_for_path(&path).is_none());
    }

    #[test]
    fn package_for_path_matches_standalone_package_at_its_root() {
        let mut store = WorkspaceMetadataStore::new();
        let root = AbsolutePath::from(PathBuf::from("/ws"));
        let target = AbsolutePath::from(PathBuf::from("/ws/target"));
        let mut snap = fake_metadata(root.clone(), target);
        let (pkg_id, pkg) =
            fake_package_record("demo", AbsolutePath::from(PathBuf::from("/ws/Cargo.toml")));
        snap.packages.insert(pkg_id, pkg);
        store.upsert(snap);

        let found = store.package_for_path(&root).expect("package found");
        assert_eq!(found.name, "demo");
        assert_eq!(found.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn package_for_path_matches_workspace_member() {
        let mut store = WorkspaceMetadataStore::new();
        let root = AbsolutePath::from(PathBuf::from("/ws"));
        let target = AbsolutePath::from(PathBuf::from("/ws/target"));
        let mut snap = fake_metadata(root, target);
        let member_root = AbsolutePath::from(PathBuf::from("/ws/crates/core"));
        let (pkg_id, pkg) = fake_package_record(
            "core",
            AbsolutePath::from(PathBuf::from("/ws/crates/core/Cargo.toml")),
        );
        snap.packages.insert(pkg_id, pkg);
        store.upsert(snap);

        let found = store
            .package_for_path(&member_root)
            .expect("member resolves via its own manifest_path");
        assert_eq!(found.name, "core");
    }

    #[test]
    fn package_for_path_returns_none_when_metadata_has_no_matching_package() {
        // Transient case: metadata covers this workspace but the
        // specific package-dir path doesn't match any manifest (e.g. a
        // Cargo.toml was just added and the follow-up dispatch hasn't
        // landed yet). Callers should treat None as "Loading…".
        let mut store = WorkspaceMetadataStore::new();
        let root = AbsolutePath::from(PathBuf::from("/ws"));
        let target = AbsolutePath::from(PathBuf::from("/ws/target"));
        store.upsert(fake_metadata(root, target));
        let phantom_member = AbsolutePath::from(PathBuf::from("/ws/crates/never"));
        assert!(store.package_for_path(&phantom_member).is_none());
    }

    #[test]
    fn set_out_of_tree_target_bytes_stamps_matching_metadata() {
        let mut store = WorkspaceMetadataStore::new();
        let root = AbsolutePath::from(PathBuf::from("/ws"));
        let target = AbsolutePath::from(PathBuf::from("/elsewhere/target"));
        store.upsert(fake_metadata(root.clone(), target.clone()));

        let applied = store.set_out_of_tree_target_bytes(&root, &target, 42_000);
        assert!(applied, "matching target_directory accepts the stamp");
        assert_eq!(
            store.get(&root).and_then(|s| s.out_of_tree_target_bytes),
            Some(42_000)
        );
    }

    #[test]
    fn set_out_of_tree_target_bytes_declines_stale_target_dir() {
        // A follow-up metadata arrival re-pointed target_directory between
        // the walk's dispatch and its arrival. The old walk must NOT stamp
        // the new metadata.
        let mut store = WorkspaceMetadataStore::new();
        let root = AbsolutePath::from(PathBuf::from("/ws"));
        let stale_target = AbsolutePath::from(PathBuf::from("/old/target"));
        let current_target = AbsolutePath::from(PathBuf::from("/new/target"));
        store.upsert(fake_metadata(root.clone(), current_target));

        let applied = store.set_out_of_tree_target_bytes(&root, &stale_target, 999);
        assert!(
            !applied,
            "stale target_dir is discarded; a fresh walk is already in flight"
        );
        assert!(
            store
                .get(&root)
                .and_then(|s| s.out_of_tree_target_bytes)
                .is_none()
        );
    }

    #[test]
    fn set_out_of_tree_target_bytes_noop_without_metadata() {
        let mut store = WorkspaceMetadataStore::new();
        let root = AbsolutePath::from(PathBuf::from("/ws"));
        let target = AbsolutePath::from(PathBuf::from("/elsewhere/target"));
        let applied = store.set_out_of_tree_target_bytes(&root, &target, 1);
        assert!(!applied, "no metadata → nothing to stamp");
    }
}
