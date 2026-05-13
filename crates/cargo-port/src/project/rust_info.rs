use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Deref;
use std::ops::DerefMut;
use std::path::Component;
use std::path::Path;

use cargo_metadata::TargetKind;

use super::cargo::ExampleGroup;
use super::cargo::ProjectType;
use super::cargo_metadata_store::PackageRecord;
use super::cargo_metadata_store::PublishPolicy;
use super::info::ProjectInfo;
use super::vendored_package::VendoredPackage;
use crate::lint::LintRuns;

/// Rust-specific project data shared by both `Workspace` and `Package`.
/// Derefs to `ProjectInfo` for uniform metadata access.
#[derive(Clone, Default)]
pub(crate) struct RustInfo {
    pub(crate) info:             ProjectInfo,
    pub(crate) cargo:            Cargo,
    pub(crate) vendored:         Vec<VendoredPackage>,
    pub(crate) lint_runs:        LintRuns,
    pub(crate) crates_version:   Option<String>,
    pub(crate) crates_downloads: Option<u64>,
}

impl RustInfo {
    pub(crate) fn vendored(&self) -> &[VendoredPackage] { &self.vendored }

    pub(crate) const fn vendored_mut(&mut self) -> &mut Vec<VendoredPackage> { &mut self.vendored }

    pub(crate) fn crates_version(&self) -> Option<&str> { self.crates_version.as_deref() }

    pub(crate) const fn crates_downloads(&self) -> Option<u64> { self.crates_downloads }

    pub(crate) fn set_crates_io(&mut self, version: String, downloads: u64) {
        self.crates_version = Some(version);
        self.crates_downloads = Some(downloads);
    }
}

impl Deref for RustInfo {
    type Target = ProjectInfo;

    fn deref(&self) -> &ProjectInfo { &self.info }
}

impl DerefMut for RustInfo {
    fn deref_mut(&mut self) -> &mut ProjectInfo { &mut self.info }
}

/// Shared Cargo fields populated from the `cargo metadata`
/// [`WorkspaceMetadata`](super::cargo_metadata_store::WorkspaceMetadata).
///
/// Step 3b full retirement: these fields are no longer hand-parsed out
/// of `Cargo.toml`. `types` / `examples` / `benches` / `test_count` stay
/// empty until the metadata lands and gets stamped in via
/// `Cargo::apply_package_record`; `publishable` defaults to `true` so
/// the crates.io scheduler continues firing for named packages
/// pre-metadata (matches pre-retirement behavior; the metadata later
/// flips it to `false` when `publish = false`).
#[derive(Clone, Debug)]
pub(crate) struct Cargo {
    pub(crate) types:       Vec<ProjectType>,
    pub(crate) examples:    Vec<ExampleGroup>,
    pub(crate) benches:     Vec<String>,
    pub(crate) test_count:  usize,
    pub(crate) publishable: bool,
}

impl Default for Cargo {
    fn default() -> Self {
        Self {
            types:       Vec::new(),
            examples:    Vec::new(),
            benches:     Vec::new(),
            test_count:  0,
            publishable: true,
        }
    }
}

impl Cargo {
    pub(crate) fn types(&self) -> &[ProjectType] { &self.types }

    pub(crate) fn examples(&self) -> &[ExampleGroup] { &self.examples }

    pub(crate) fn benches(&self) -> &[String] { &self.benches }

    pub(crate) const fn test_count(&self) -> usize { self.test_count }

    pub(crate) fn example_count(&self) -> usize {
        self.examples.iter().map(|g| g.names.len()).sum()
    }

    #[allow(
        dead_code,
        reason = "no production callers; kept for future reuse against \
                  PackageRecord.targets."
    )]
    pub(crate) fn is_binary(&self) -> bool {
        self.types.iter().any(|t| matches!(t, ProjectType::Binary))
    }

    pub(crate) const fn publishable(&self) -> bool { self.publishable }

    /// Derive a `Cargo` from the authoritative [`PackageRecord`] returned
    /// by `cargo metadata`. Called on every metadata arrival (and on
    /// vendored / workspace-member fan-out) to replace the defaults that
    /// `from_cargo_toml` leaves in place.
    ///
    /// `ProjectType::Workspace` is not derived here — that's a property
    /// of the `[workspace]` table presence, owned by the parse path.
    pub(crate) fn from_package_record(record: &PackageRecord) -> Self {
        let manifest_dir = record.manifest_path.as_path().parent();
        let mut has_lib = false;
        let mut has_bin = false;
        let mut has_proc_macro = false;
        let mut example_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut benches: Vec<String> = Vec::new();
        let mut test_count: usize = 0;

        for target in &record.targets {
            for kind in &target.kinds {
                match kind {
                    TargetKind::Bin => has_bin = true,
                    TargetKind::Lib
                    | TargetKind::RLib
                    | TargetKind::DyLib
                    | TargetKind::CDyLib
                    | TargetKind::StaticLib => has_lib = true,
                    TargetKind::ProcMacro => has_proc_macro = true,
                    TargetKind::Example => {
                        let category = example_category(manifest_dir, target.src_path.as_path());
                        example_groups
                            .entry(category)
                            .or_default()
                            .push(target.name.clone());
                    },
                    TargetKind::Bench => benches.push(target.name.clone()),
                    TargetKind::Test => test_count += 1,
                    _ => {},
                }
            }
        }

        let mut types = Vec::new();
        if has_proc_macro {
            types.push(ProjectType::ProcMacro);
        } else if has_lib {
            types.push(ProjectType::Library);
        }
        if has_bin {
            types.push(ProjectType::Binary);
        }

        let mut examples: Vec<ExampleGroup> = example_groups
            .into_iter()
            .map(|(category, mut names)| {
                names.sort();
                ExampleGroup { category, names }
            })
            .collect();
        // Root-level category first, then alphabetical — matches the
        // ordering `collect_examples` produced before the retirement.
        examples.sort_by(|a, b| {
            let a_root = a.category.is_empty();
            let b_root = b.category.is_empty();
            match (a_root, b_root) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => a.category.cmp(&b.category),
            }
        });
        benches.sort();

        let publishable = !matches!(record.publish, PublishPolicy::Never);

        Self {
            types,
            examples,
            benches,
            test_count,
            publishable,
        }
    }
}

/// Extract the subdirectory category from an example's `src_path`
/// relative to its package's manifest dir. `examples/foo.rs` → root
/// (empty category); `examples/2d/foo.rs` → `2d`. Matches the grouping
/// `TargetsData::from_package_record` uses for the Targets pane.
fn example_category(manifest_dir: Option<&Path>, src_path: &Path) -> String {
    let Some(dir) = manifest_dir else {
        return String::new();
    };
    let Ok(rel) = src_path.strip_prefix(dir) else {
        return String::new();
    };
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(seg) => Some(seg.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if parts.len() >= 3 {
        parts[1].clone()
    } else {
        String::new()
    }
}
