//! Typed cache of `cargo metadata` output, keyed by workspace root.
//!
//! Holds one [`WorkspaceSnapshot`] per detected workspace. Phase 1 of the
//! `cargo_metadata` integration — this module defines the shape and
//! read-side access; producers and consumers land in later steps.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use cargo_metadata::PackageId;
use cargo_metadata::TargetKind;
use cargo_metadata::semver::Version;
use sha2::Digest as _;

use super::AbsolutePath;

/// Process-wide cache of cargo-metadata snapshots, keyed by workspace root.
///
/// Populated by [`BackgroundMsg::CargoMetadata`](crate::scan::BackgroundMsg)
/// arrivals. Callers that want read-only access should go through
/// `App::resolve_metadata` / `App::resolve_target_dir` rather than touching
/// this type directly.
#[derive(Debug, Default)]
pub(crate) struct WorkspaceMetadataStore {
    pub(crate) by_root:              HashMap<AbsolutePath, WorkspaceSnapshot>,
    /// Per-workspace monotonic counter. Every dispatch bumps the counter
    /// and stamps the spawned work with the new value; arrivals only
    /// commit if their stamp still matches the current counter. This
    /// coalesces rapid edits to a single accepted result.
    pub(crate) dispatch_generations: HashMap<AbsolutePath, u64>,
}

impl WorkspaceMetadataStore {
    pub(crate) fn new() -> Self { Self::default() }

    /// Look up the snapshot for the workspace whose root is `workspace_root`.
    pub(crate) fn get(&self, workspace_root: &AbsolutePath) -> Option<&WorkspaceSnapshot> {
        self.by_root.get(workspace_root)
    }

    /// Walk `path`'s ancestors and return the first one that has a snapshot.
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

    /// Insert or replace the snapshot for `workspace_root`.
    pub(crate) fn upsert(&mut self, snapshot: WorkspaceSnapshot) {
        self.by_root
            .insert(snapshot.workspace_root.clone(), snapshot);
    }

    /// Drop the snapshot for `workspace_root`, if any.
    pub(crate) fn remove(&mut self, workspace_root: &AbsolutePath) {
        self.by_root.remove(workspace_root);
        self.dispatch_generations.remove(workspace_root);
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
pub(crate) struct WorkspaceSnapshot {
    pub workspace_root:    AbsolutePath,
    pub target_directory:  AbsolutePath,
    pub packages:          HashMap<PackageId, PackageRecord>,
    pub workspace_members: Vec<PackageId>,
    pub fetched_at:        SystemTime,
    pub fingerprint:       ManifestFingerprint,
}

/// Normalized form of a single package's metadata. Field shapes mirror
/// `cargo_metadata::Package` but keep only the bits the UI and query paths
/// actually need.
#[derive(Clone, Debug)]
pub(crate) struct PackageRecord {
    pub id:            PackageId,
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
            Some(list) if list.is_empty() => Self::Never,
            Some(list) => Self::Registries(list.to_vec()),
        }
    }
}

/// A single build target (bin, lib, example, test, bench, proc-macro, …).
#[derive(Clone, Debug)]
pub(crate) struct TargetRecord {
    pub name:              String,
    pub kinds:             Vec<TargetKind>,
    pub src_path:          AbsolutePath,
    pub edition:           String,
    pub required_features: Vec<String>,
}

/// Cheap, cross-conversation-stable reference to a package inside a
/// [`WorkspaceSnapshot`]. `RustInfo` and similar project-state carry this
/// handle so the snapshot body is not duplicated across every member.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceMetadataHandle {
    pub workspace_root: AbsolutePath,
    pub package_id:     PackageId,
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
    /// itself a diff and invalidates the snapshot.
    pub configs:        std::collections::BTreeMap<PathBuf, Option<FileStamp>>,
}

/// Fingerprint for a single file. Equality uses `content_hash` only;
/// `(mtime, len)` is a pre-check that skips the hash when the file clearly
/// changed. Inode is deliberately omitted — editor atomic-save workflows
/// rotate inodes on every save even when bytes are identical.
#[derive(Clone, Debug, Eq)]
pub(crate) struct FileStamp {
    pub mtime:        SystemTime,
    pub len:          u64,
    pub content_hash: [u8; 32],
}

impl PartialEq for FileStamp {
    fn eq(&self, other: &Self) -> bool { self.content_hash == other.content_hash }
}

impl FileStamp {
    /// Read `path` and compute its stamp. SHA-256 of the bytes is the
    /// authoritative identity; `(mtime, len)` rides along for callers that
    /// want a cheap change heuristic but is not part of equality.
    pub(crate) fn from_path(path: &Path) -> io::Result<Self> {
        let meta = fs::metadata(path)?;
        let bytes = fs::read(path)?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        let mut content_hash = [0_u8; 32];
        content_hash.copy_from_slice(&digest);
        Ok(Self {
            mtime: meta.modified()?,
            len: meta.len(),
            content_hash,
        })
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
    /// invalidates the snapshot even though no tracked-file edit occurred.
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
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
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
    use std::time::Duration;

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
        // Same bytes → equal regardless of (mtime, len) drift.
        let now = SystemTime::now();
        let later = now + Duration::from_secs(10);
        let a = FileStamp {
            mtime:        now,
            len:          4,
            content_hash: [7; 32],
        };
        let b = FileStamp {
            mtime:        later,
            len:          999,
            content_hash: [7; 32],
        };
        assert_eq!(a, b, "different (mtime, len), same hash → equal");

        let c = FileStamp {
            mtime:        now,
            len:          4,
            content_hash: [8; 32],
        };
        assert_ne!(a, c, "same (mtime, len), different hash → unequal");
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
            fp.configs.values().any(|stamp| stamp.is_none()),
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

    #[test]
    fn remove_clears_generation_too() {
        let mut store = WorkspaceMetadataStore::new();
        let root = AbsolutePath::from(PathBuf::from("/ws"));
        let _ = store.next_generation(&root);
        store.remove(&root);
        assert!(
            !store.dispatch_generations.contains_key(&root),
            "generation counter is dropped with the snapshot"
        );
    }
}
