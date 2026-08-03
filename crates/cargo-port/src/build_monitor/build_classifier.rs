//! The stateful side of build classification: the caches that make the pure
//! [`super::classify::classify`] call possible, and the filesystem work that
//! fills them.
//!
//! Everything mutable lives here. `classify` receives shared references to the
//! two tables below and has no path to the classifier itself, so a cycle cannot
//! read a value this cycle wrote.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;
use std::time::SystemTime;

use super::activity::ManifestPackageIdentity;
use super::classify::ArgumentPath;
use super::classify::BuildClassification;
use super::classify::BuildClassificationCycle;
use super::classify::BuildClassificationInput;
use super::classify::CanonicalProcessPathSet;
use super::classify::CanonicalProcessPaths;
use super::classify::ClassificationTables;
use super::classify::CycleStamp;
use super::classify::IndexedWorkspaceTargetDirectories;
use super::classify::ObservedCycle;
use super::classify::SessionBuildDirectory;
use super::classify::classify;
use super::classify::observed_process_path_arguments;
use super::constants::MANIFEST_FILE_NAME;
use super::constants::MAX_BUILD_DIRECTORY_ENTRIES;
use super::constants::MAX_DEPENDENCY_MANIFEST_ENTRIES;
use super::constants::MAX_FIRST_SEEN_ENTRIES;
use super::constants::MAX_MANIFEST_SEARCH_DEPTH;
use super::session::BuildSession;
use super::session::BuildSessionId;
use super::session::OwnedRootEvidence;
use crate::process_observation::identity::ProcessIncarnation;
use crate::process_observation::snapshot::ProcessFieldObservation;
use crate::process_observation::snapshot::ProcessObservationSnapshot;
use crate::project::AbsolutePath;
use crate::project::CanonicalPathResolution;
use crate::project::CargoWorkspaceIndex;

/// What one cached manifest read established about a dependency source root.
#[derive(Clone, Debug, Eq, PartialEq)]
enum DependencyManifestIdentity {
    /// A manifest was found and named this package.
    Package(ManifestPackageIdentity),
    /// The search reached its depth limit without finding a readable manifest.
    NoManifest,
}

/// When a manifest was last read, so a rewritten or removed manifest is
/// reparsed instead of served from cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManifestStamp {
    /// The manifest existed with this modification time and length.
    Present {
        /// Last modification time reported by the filesystem.
        modified: SystemTime,
        /// Length in bytes.
        length:   u64,
    },
    /// No manifest existed when the entry was recorded.
    Missing,
}

impl ManifestStamp {
    fn read(manifest_path: &Path) -> Self {
        fs::metadata(manifest_path).map_or(Self::Missing, |metadata| {
            metadata
                .modified()
                .map_or(Self::Missing, |modified| Self::Present {
                    modified,
                    length: metadata.len(),
                })
        })
    }
}

/// One cached dependency identity plus the evidence that keeps it valid.
#[derive(Clone, Debug, Eq, PartialEq)]
struct DependencyManifestEntry {
    manifest_path:                  PathBuf,
    manifest_stamp:                 ManifestStamp,
    dependency_manifest_identity:   DependencyManifestIdentity,
    last_used_classification_cycle: BuildClassificationCycle,
}

/// Whether a dependency source root has a cached package identity, and whether
/// it has been looked up at all.
///
/// "Not yet looked up" and "looked up and found nothing" are different facts:
/// the first must produce a lookup request, the second must not, or every cycle
/// would re-read a manifest that does not exist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DependencyManifestLookup<'a> {
    /// A manifest read identified this dependency package.
    Resolved(&'a ManifestPackageIdentity),
    /// No read has been attempted for this source root.
    NotYetLookedUp,
    /// A read was attempted and found no manifest naming a package.
    Absent,
}

/// Dependency package identities read from manifests outside the workspace
/// index, keyed by canonical source root.
#[derive(Debug, Default)]
pub(super) struct DependencyManifestSnapshot {
    entries: HashMap<AbsolutePath, DependencyManifestEntry>,
}

impl DependencyManifestSnapshot {
    /// Look one dependency source root up without reading the filesystem.
    pub(super) fn lookup(
        &self,
        canonical_source_root: &AbsolutePath,
    ) -> DependencyManifestLookup<'_> {
        self.entries.get(canonical_source_root).map_or(
            DependencyManifestLookup::NotYetLookedUp,
            |entry| match &entry.dependency_manifest_identity {
                DependencyManifestIdentity::Package(manifest_package_identity) => {
                    DependencyManifestLookup::Resolved(manifest_package_identity)
                },
                DependencyManifestIdentity::NoManifest => DependencyManifestLookup::Absent,
            },
        )
    }

    /// Read one dependency source root's nearest manifest and cache the result.
    fn record(
        &mut self,
        canonical_source_root: AbsolutePath,
        build_classification_cycle: BuildClassificationCycle,
    ) {
        let (manifest_path, dependency_manifest_identity) =
            read_nearest_manifest(canonical_source_root.as_path());
        let manifest_stamp = ManifestStamp::read(&manifest_path);
        self.entries.insert(
            canonical_source_root,
            DependencyManifestEntry {
                manifest_path,
                manifest_stamp,
                dependency_manifest_identity,
                last_used_classification_cycle: build_classification_cycle,
            },
        );
    }

    /// Drop entries whose manifest changed or disappeared since the read, and
    /// mark the rest as used this cycle so eviction keeps what is live.
    fn revalidate(
        &mut self,
        canonical_source_roots_in_use: &[AbsolutePath],
        build_classification_cycle: BuildClassificationCycle,
    ) {
        let mut stale = Vec::new();
        for canonical_source_root in canonical_source_roots_in_use {
            let Some(entry) = self.entries.get_mut(canonical_source_root) else {
                continue;
            };
            if ManifestStamp::read(&entry.manifest_path) == entry.manifest_stamp {
                entry.last_used_classification_cycle = build_classification_cycle;
            } else {
                stale.push(canonical_source_root.clone());
            }
        }
        for canonical_source_root in stale {
            self.entries.remove(&canonical_source_root);
        }
    }

    /// Drop the least recently used entries once the table exceeds its cap. A
    /// dropped entry is re-read on demand, so eviction costs one manifest read,
    /// never a wrong identity.
    fn evict_least_recently_used(&mut self) {
        if self.entries.len() <= MAX_DEPENDENCY_MANIFEST_ENTRIES {
            return;
        }
        let mut recency: Vec<(AbsolutePath, BuildClassificationCycle)> = self
            .entries
            .iter()
            .map(|(canonical_source_root, entry)| {
                (
                    canonical_source_root.clone(),
                    entry.last_used_classification_cycle,
                )
            })
            .collect();
        recency.sort_by_key(|(_, last_used)| *last_used);
        let excess = self
            .entries
            .len()
            .saturating_sub(MAX_DEPENDENCY_MANIFEST_ENTRIES);
        for (canonical_source_root, _) in recency.into_iter().take(excess) {
            self.entries.remove(&canonical_source_root);
        }
    }

    /// How many source roots are cached.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize { self.entries.len() }
}

/// Whether a session's own compilers have been observed writing under a build
/// directory inside its target directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ObservedBuildDirectoryLookup<'a> {
    /// A compiler of this session wrote under this build directory.
    Observed(&'a str),
    /// No compiler of this session has been observed writing under one.
    NotObserved,
}

/// One session's observed build directory plus the recency that bounds it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct BuildDirectoryEntry {
    build_directory_name:           String,
    last_used_classification_cycle: BuildClassificationCycle,
}

/// The build directory each session's own compilers were observed writing
/// under, held for the session incarnation's lifetime.
///
/// The pure classifier rebuilds its live-compiler evidence every cycle, so a
/// profile that never reaches argv — one configured in `.cargo/config.toml` or
/// carried by a Cargo alias — would report `release` on cycles with a live
/// compiler child and revert to `dev` on cycles with none. This ledger makes it
/// sticky the same way [`FirstSeenLedger`] makes promotion sticky.
#[derive(Debug, Default)]
pub(super) struct BuildDirectoryLedger {
    entries: HashMap<BuildSessionId, BuildDirectoryEntry>,
}

impl BuildDirectoryLedger {
    /// The build directory this session was last observed writing under.
    pub(super) fn lookup(
        &self,
        build_session_id: &BuildSessionId,
    ) -> ObservedBuildDirectoryLookup<'_> {
        self.entries
            .get(build_session_id)
            .map_or(ObservedBuildDirectoryLookup::NotObserved, |entry| {
                ObservedBuildDirectoryLookup::Observed(entry.build_directory_name.as_str())
            })
    }

    fn record(
        &mut self,
        session_build_directories: &[SessionBuildDirectory],
        build_classification_cycle: BuildClassificationCycle,
    ) {
        for session_build_directory in session_build_directories {
            self.entries.insert(
                session_build_directory.build_session_id().clone(),
                BuildDirectoryEntry {
                    build_directory_name:           session_build_directory
                        .build_directory_name()
                        .to_owned(),
                    last_used_classification_cycle: build_classification_cycle,
                },
            );
        }
    }

    /// Mark every still-classified session as used this cycle, then drop the
    /// least recently used entries once the table exceeds its cap. A dropped
    /// entry costs one cycle of argv-only profile, never a wrong profile: the
    /// key is exec-sensitive, so no later session can collide with it.
    fn retain_live(
        &mut self,
        build_sessions: &[BuildSession],
        build_classification_cycle: BuildClassificationCycle,
    ) {
        for build_session in build_sessions {
            if let Some(entry) = self.entries.get_mut(build_session.build_session_id()) {
                entry.last_used_classification_cycle = build_classification_cycle;
            }
        }
        if self.entries.len() <= MAX_BUILD_DIRECTORY_ENTRIES {
            return;
        }
        let mut recency: Vec<(BuildSessionId, BuildClassificationCycle)> = self
            .entries
            .iter()
            .map(|(build_session_id, entry)| {
                (
                    build_session_id.clone(),
                    entry.last_used_classification_cycle,
                )
            })
            .collect();
        recency.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        let excess = self
            .entries
            .len()
            .saturating_sub(MAX_BUILD_DIRECTORY_ENTRIES);
        for (build_session_id, _) in recency.into_iter().take(excess) {
            self.entries.remove(&build_session_id);
        }
    }
}

/// Whether a Cargo root has been proven to compile.
///
/// Promotion is sticky for the incarnation's lifetime: a root whose only
/// compiler descendant has already exited is still a build session, or a
/// finishing build would vanish from the pane mid-link.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RootPromotion {
    /// A live compiler descendant attached to this root at some cycle.
    Promoted,
    /// No compiler descendant has attached to this root.
    NotPromoted,
}

/// One incarnation's ordering and promotion history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FirstSeenEntry {
    first_seen:     BuildClassificationCycle,
    root_promotion: RootPromotion,
}

/// Whether the ledger already holds a first-seen cycle for an incarnation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FirstSeenLookup {
    /// The incarnation was first observed in this cycle.
    Recorded(BuildClassificationCycle),
    /// The incarnation has not been observed before.
    NotRecorded,
}

/// When each observed incarnation was first seen, and which Cargo roots have
/// been promoted to build sessions.
#[derive(Debug, Default)]
pub(super) struct FirstSeenLedger {
    entries: HashMap<ProcessIncarnation, FirstSeenEntry>,
}

impl FirstSeenLedger {
    /// The cycle this incarnation was first observed in, when known.
    pub(super) fn lookup(&self, incarnation: &ProcessIncarnation) -> FirstSeenLookup {
        self.entries
            .get(incarnation)
            .map_or(FirstSeenLookup::NotRecorded, |entry| {
                FirstSeenLookup::Recorded(entry.first_seen)
            })
    }

    /// Whether a compiler descendant has ever attached to this Cargo root.
    pub(super) fn is_promoted(&self, incarnation: &ProcessIncarnation) -> bool {
        self.entries
            .get(incarnation)
            .is_some_and(|entry| matches!(entry.root_promotion, RootPromotion::Promoted))
    }

    fn record_observed(
        &mut self,
        observed_incarnations: &[ProcessIncarnation],
        build_classification_cycle: BuildClassificationCycle,
    ) {
        for incarnation in observed_incarnations {
            self.entries
                .entry(incarnation.clone())
                .or_insert(FirstSeenEntry {
                    first_seen:     build_classification_cycle,
                    root_promotion: RootPromotion::NotPromoted,
                });
        }
    }

    fn record_promotions(
        &mut self,
        promoted_root_incarnations: &[ProcessIncarnation],
        build_classification_cycle: BuildClassificationCycle,
    ) {
        for incarnation in promoted_root_incarnations {
            let entry = self
                .entries
                .entry(incarnation.clone())
                .or_insert(FirstSeenEntry {
                    first_seen:     build_classification_cycle,
                    root_promotion: RootPromotion::NotPromoted,
                });
            entry.root_promotion = RootPromotion::Promoted;
        }
    }

    /// Forget incarnations the snapshot no longer contains, then cap what is
    /// left. A long build execs thousands of compilers, so pruning against the
    /// snapshot alone is not a bound.
    fn prune(&mut self, observed_incarnations: &[ProcessIncarnation]) {
        self.entries
            .retain(|incarnation, _| observed_incarnations.contains(incarnation));
        if self.entries.len() <= MAX_FIRST_SEEN_ENTRIES {
            return;
        }
        let mut order: Vec<(ProcessIncarnation, BuildClassificationCycle)> = self
            .entries
            .iter()
            .map(|(incarnation, entry)| (incarnation.clone(), entry.first_seen))
            .collect();
        order.sort_by_key(|(_, first_seen)| *first_seen);
        let excess = self.entries.len().saturating_sub(MAX_FIRST_SEEN_ENTRIES);
        for (incarnation, _) in order.into_iter().take(excess) {
            self.entries.remove(&incarnation);
        }
    }

    /// How many incarnations the ledger remembers.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize { self.entries.len() }
}

/// Owns the classification caches and drives one cycle at a time.
#[derive(Debug, Default)]
pub(crate) struct BuildClassifier {
    dependency_manifest_snapshot: DependencyManifestSnapshot,
    first_seen_ledger:            FirstSeenLedger,
    build_directory_ledger:       BuildDirectoryLedger,
    build_classification_cycle:   BuildClassificationCycle,
}

impl BuildClassifier {
    /// Prepare one cycle's input, run the pure classification, then apply what
    /// it asked for. Every filesystem read happens on one side or the other of
    /// the [`classify`] call, never inside it.
    pub(crate) fn classify_cycle(
        &mut self,
        process_observation_snapshot: &ProcessObservationSnapshot,
        cargo_workspace_index: &CargoWorkspaceIndex,
        owned_root_evidence: &OwnedRootEvidence,
        cycle_instant: Instant,
    ) -> BuildClassification {
        self.build_classification_cycle.advance();
        let canonical_process_paths = resolve_canonical_process_paths(process_observation_snapshot);
        let indexed_workspace_target_directories =
            IndexedWorkspaceTargetDirectories::resolve(cargo_workspace_index);

        let build_classification = classify(BuildClassificationInput::new(
            ObservedCycle::new(
                process_observation_snapshot,
                cargo_workspace_index,
                &indexed_workspace_target_directories,
                &canonical_process_paths,
                owned_root_evidence,
            ),
            ClassificationTables::new(
                &self.dependency_manifest_snapshot,
                &self.first_seen_ledger,
                &self.build_directory_ledger,
            ),
            CycleStamp::new(self.build_classification_cycle, cycle_instant),
        ));

        self.build_directory_ledger.record(
            build_classification.observed_session_build_directories(),
            self.build_classification_cycle,
        );
        self.build_directory_ledger.retain_live(
            build_classification.build_sessions(),
            self.build_classification_cycle,
        );

        self.first_seen_ledger.record_observed(
            build_classification.observed_incarnations(),
            self.build_classification_cycle,
        );
        self.first_seen_ledger.record_promotions(
            build_classification.promoted_root_incarnations(),
            self.build_classification_cycle,
        );
        self.first_seen_ledger
            .prune(build_classification.observed_incarnations());

        self.dependency_manifest_snapshot.revalidate(
            build_classification.dependency_source_roots_in_use(),
            self.build_classification_cycle,
        );
        for request in build_classification.dependency_manifest_lookup_requests() {
            self.dependency_manifest_snapshot.record(
                request.canonical_source_root().clone(),
                self.build_classification_cycle,
            );
        }
        self.dependency_manifest_snapshot
            .evict_least_recently_used();

        build_classification
    }

    /// The cached dependency identities, for tests that assert a second cycle
    /// issues no further lookup requests.
    #[cfg(test)]
    pub(super) const fn dependency_manifest_snapshot(&self) -> &DependencyManifestSnapshot {
        &self.dependency_manifest_snapshot
    }

    /// The first-seen ledger, for tests that assert ordering stability.
    #[cfg(test)]
    pub(super) const fn first_seen_ledger(&self) -> &FirstSeenLedger { &self.first_seen_ledger }
}

/// Canonicalize every candidate's observed paths before the pure call, so
/// classification compares canonical paths on both sides without touching the
/// filesystem itself.
fn resolve_canonical_process_paths(
    process_observation_snapshot: &ProcessObservationSnapshot,
) -> CanonicalProcessPaths {
    let mut canonical_process_paths = CanonicalProcessPaths::default();
    for (incarnation, build_candidate_role) in process_observation_snapshot
        .build_candidate_incarnations()
        .iter()
    {
        let Some(record) = process_observation_snapshot
            .strongly_identified_processes()
            .get(incarnation.identity())
        else {
            continue;
        };
        let ProcessFieldObservation::Observed(argv) = record.argv() else {
            continue;
        };
        let observed = observed_process_path_arguments(build_candidate_role, argv);
        let working_directory = match record.cwd() {
            ProcessFieldObservation::Observed(cwd) => canonicalize(cwd),
            ProcessFieldObservation::Unavailable(_) | ProcessFieldObservation::Invalidated(_) => {
                CanonicalPathResolution::Unresolved
            },
        };
        canonical_process_paths.insert(
            incarnation.clone(),
            CanonicalProcessPathSet::new(
                working_directory,
                canonicalize_argument(&observed.manifest_directory, record.cwd()),
                canonicalize_argument(&observed.target_directory_argument, record.cwd()),
                canonicalize_argument(&observed.output_directory, record.cwd()),
                canonicalize_argument(&observed.primary_input, record.cwd()),
            ),
        );
    }
    canonical_process_paths
}

/// Resolve one observed path argument, joining a relative argument onto the
/// process's own working directory rather than this process's.
fn canonicalize_argument(
    argument_path: &ArgumentPath,
    cwd: &ProcessFieldObservation<PathBuf>,
) -> CanonicalPathResolution<AbsolutePath> {
    let Some(argument) = argument_path.named_path() else {
        return CanonicalPathResolution::Unresolved;
    };
    if argument.is_absolute() {
        return canonicalize(argument);
    }
    match cwd {
        ProcessFieldObservation::Observed(cwd) => canonicalize(&cwd.join(argument)),
        ProcessFieldObservation::Unavailable(_) | ProcessFieldObservation::Invalidated(_) => {
            CanonicalPathResolution::Unresolved
        },
    }
}

fn canonicalize(path: &Path) -> CanonicalPathResolution<AbsolutePath> {
    fs::canonicalize(path).map_or(CanonicalPathResolution::Unresolved, |canonical| {
        CanonicalPathResolution::Resolved(AbsolutePath::from(canonical))
    })
}

/// Walk up from a dependency source root to its nearest manifest and read the
/// package name and version out of it.
fn read_nearest_manifest(canonical_source_root: &Path) -> (PathBuf, DependencyManifestIdentity) {
    let mut directory = canonical_source_root;
    for _ in 0..MAX_MANIFEST_SEARCH_DEPTH {
        let manifest_path = directory.join(MANIFEST_FILE_NAME);
        if let Ok(contents) = fs::read_to_string(&manifest_path) {
            let dependency_manifest_identity = parse_manifest_package(&contents);
            return (manifest_path, dependency_manifest_identity);
        }
        let Some(parent) = directory.parent() else {
            break;
        };
        directory = parent;
    }
    (
        canonical_source_root.join(MANIFEST_FILE_NAME),
        DependencyManifestIdentity::NoManifest,
    )
}

/// Read `[package] name` and `version` without deserializing the whole
/// manifest: a dependency manifest may use table syntax this crate has no
/// model for, and only these two fields identify the package.
fn parse_manifest_package(contents: &str) -> DependencyManifestIdentity {
    let Ok(document) = contents.parse::<toml::Table>() else {
        return DependencyManifestIdentity::NoManifest;
    };
    let Some(package) = document.get("package").and_then(toml::Value::as_table) else {
        return DependencyManifestIdentity::NoManifest;
    };
    let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
        return DependencyManifestIdentity::NoManifest;
    };
    let version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    DependencyManifestIdentity::Package(ManifestPackageIdentity::new(
        name.to_owned(),
        version.to_owned(),
    ))
}
