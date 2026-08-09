//! What build classification adds to one refresh cycle.
//!
//! The observer-only baseline is reported by
//! `crate::process_observation::benchmarks`; the number that matters here is
//! the increment classification adds on top of it, because that increment is
//! what a cycle pays once the compile monitor is enabled. Snapshot assembly is
//! measured separately and excluded from the classification samples, while
//! `classify_cycle`'s canonicalization and target-directory resolution are
//! charged to classification, which is where those costs are actually paid.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hint::black_box;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use tempfile::TempDir;

use super::build_classifier::BuildClassifier;
use super::classify::BuildClassification;
use super::session::OwnedRootEvidence;
use crate::process_observation::snapshot::ProcessObservationSnapshot;
use crate::process_observation::snapshot_builder;
use crate::process_observation::snapshot_builder::ObservedCandidateRole;
use crate::process_observation::snapshot_builder::ObservedProcess;
use crate::project::AbsolutePath;
use crate::project::CargoWorkspaceIndex;
use crate::project::FileStamp;
use crate::project::ManifestFingerprint;
use crate::project::ProjectListRevision;
use crate::project::WorkspaceMetadata;
use crate::project::WorkspaceMetadataStore;

/// Cargo roots per hundred fixture processes.
const CARGO_ROOTS_PER_HUNDRED: usize = 2;
/// Compiler processes per hundred fixture processes.
const COMPILERS_PER_HUNDRED: usize = 20;
/// The event-loop refresh budget `crate::process_observation::benchmarks`
/// selects an execution backend against.
///
/// Classification is measured against the same budget so the two costs are
/// comparable: an increment above it cannot be paid inline in the event loop
/// on top of an observer refresh that already spends its own share.
const EVENT_LOOP_PROCESS_REFRESH_BUDGET: Duration = Duration::from_millis(15);
const FIXTURE_PROCESS_CREATION_TOKEN_OFFSET: u64 = 10_000;
const INDEXED_CHECKOUT_COUNT: usize = 4;
const LARGE_FIXTURE_PROCESS_COUNT: usize = 5_000;
/// The population the enabled monitor actually runs under: the
/// `CARGO_ROOTS_PER_HUNDRED + COMPILERS_PER_HUNDRED + WRAPPERS_PER_HUNDRED`
/// build candidates one hundred processes produce, a few dozen in total. The
/// thousand- and five-thousand-process rows are both far above the refresh
/// budget, so a change in per-candidate cost moves neither of them; this row is
/// the one that can still cross the budget when it regresses.
const MONITOR_SCALE_FIXTURE_PROCESS_COUNT: usize = 100;
const PROCESS_COUNTS: [usize; 3] = [
    MONITOR_SCALE_FIXTURE_PROCESS_COUNT,
    SMALL_FIXTURE_PROCESS_COUNT,
    LARGE_FIXTURE_PROCESS_COUNT,
];
const RECORDED_FIXTURE_BACKED_CLASSIFICATION_TIMINGS: [FixtureBackedClassificationTimingSamples;
    3] = [
    FixtureBackedClassificationTimingSamples {
        process_count:         MONITOR_SCALE_FIXTURE_PROCESS_COUNT,
        snapshot_assemblies:   [
            Duration::from_nanos(91_791),
            Duration::from_nanos(87_667),
            Duration::from_nanos(84_584),
            Duration::from_nanos(85_916),
            Duration::from_nanos(84_375),
        ],
        classification_cycles: [
            Duration::from_nanos(776_083),
            Duration::from_nanos(755_792),
            Duration::from_nanos(742_875),
            Duration::from_nanos(764_542),
            Duration::from_nanos(869_625),
        ],
    },
    FixtureBackedClassificationTimingSamples {
        process_count:         SMALL_FIXTURE_PROCESS_COUNT,
        snapshot_assemblies:   [
            Duration::from_nanos(1_047_667),
            Duration::from_nanos(1_010_791),
            Duration::from_nanos(1_041_125),
            Duration::from_nanos(1_118_792),
            Duration::from_nanos(1_049_709),
        ],
        classification_cycles: [
            Duration::from_nanos(7_876_416),
            Duration::from_nanos(8_479_500),
            Duration::from_nanos(7_800_541),
            Duration::from_nanos(8_199_333),
            Duration::from_micros(8107),
        ],
    },
    FixtureBackedClassificationTimingSamples {
        process_count:         LARGE_FIXTURE_PROCESS_COUNT,
        snapshot_assemblies:   [
            Duration::from_nanos(5_612_625),
            Duration::from_nanos(5_795_042),
            Duration::from_nanos(5_961_125),
            Duration::from_nanos(5_794_083),
            Duration::from_nanos(5_458_125),
        ],
        classification_cycles: [
            Duration::from_nanos(49_287_750),
            Duration::from_nanos(46_792_875),
            Duration::from_nanos(46_903_416),
            Duration::from_nanos(45_833_500),
            Duration::from_nanos(46_539_709),
        ],
    },
];
const RECORDED_TIMING_DATE: &str = "2026-08-04";
const REPEATED_SAMPLE_COUNT: usize = 5;
const SMALL_FIXTURE_PROCESS_COUNT: usize = 1_000;
/// Compiler-wrapper processes per hundred fixture processes.
const WRAPPERS_PER_HUNDRED: usize = 4;

/// Whether a recorded sample set leaves room to classify inside the event
/// loop, or requires the dedicated worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClassificationCyclePlacement {
    EventLoopAffordable,
    RequiresDedicatedWorker,
}

#[derive(Clone, Debug)]
struct FixtureBackedClassificationTimingSamples {
    process_count:         usize,
    /// Cost of assembling the snapshot classification reads, excluded from the
    /// classification samples so the increment is not double-counted.
    snapshot_assemblies:   [Duration; REPEATED_SAMPLE_COUNT],
    /// Cost of one `classify_cycle` over that snapshot, canonicalization and
    /// target-directory resolution included.
    classification_cycles: [Duration; REPEATED_SAMPLE_COUNT],
}

impl FixtureBackedClassificationTimingSamples {
    /// The assembly cost the classification samples exclude.
    fn slowest_snapshot_assembly(&self) -> Duration {
        self.snapshot_assemblies
            .iter()
            .copied()
            .max()
            .unwrap_or(Duration::ZERO)
    }

    /// The classification increment one cycle pays in the worst recorded case.
    fn slowest_classification_cycle(&self) -> Duration {
        self.classification_cycles
            .iter()
            .copied()
            .max()
            .unwrap_or(Duration::ZERO)
    }

    fn classification_cycle_placement(&self) -> ClassificationCyclePlacement {
        if self
            .classification_cycles
            .iter()
            .all(|elapsed| *elapsed <= EVENT_LOOP_PROCESS_REFRESH_BUDGET)
        {
            ClassificationCyclePlacement::EventLoopAffordable
        } else {
            ClassificationCyclePlacement::RequiresDedicatedWorker
        }
    }
}

/// One indexed set of checkouts on disk plus the processes observed against
/// them, so canonicalization and target-directory resolution do the work a live
/// cycle does.
struct DeterministicClassificationFixture {
    /// Holds the checkouts classification canonicalizes against; every
    /// fixture path is derived from it.
    temp_dir:              TempDir,
    cargo_workspace_index: CargoWorkspaceIndex,
    observed_processes:    Vec<ObservedProcess>,
}

impl DeterministicClassificationFixture {
    fn new(process_count: usize) -> Self {
        let temp_dir = tempfile::tempdir().expect("fixture temp directory is creatable");
        let mut workspace_metadata_store = WorkspaceMetadataStore::new();
        for index in 0..INDEXED_CHECKOUT_COUNT {
            let checkout_root = checkout_root_under(temp_dir.path(), index);
            std::fs::create_dir_all(checkout_root.join("target/debug/deps"))
                .expect("fixture checkout directories are creatable");
            std::fs::create_dir_all(checkout_root.join("src"))
                .expect("fixture source directories are creatable");
            workspace_metadata_store.upsert(workspace_metadata(&checkout_root));
        }
        let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
            &workspace_metadata_store,
            ProjectListRevision::default(),
        );
        let mut fixture = Self {
            cargo_workspace_index,
            observed_processes: Vec::with_capacity(process_count),
            temp_dir,
        };
        fixture.observed_processes = fixture.observe_processes(process_count);
        fixture
    }

    /// A representative build population: a handful of Cargo roots, the
    /// compilers and wrappers they drive, and a majority of processes that have
    /// nothing to do with any build.
    fn observe_processes(&self, process_count: usize) -> Vec<ObservedProcess> {
        let hundreds = process_count / 100;
        let cargo_root_count = hundreds * CARGO_ROOTS_PER_HUNDRED;
        let compiler_count = hundreds * COMPILERS_PER_HUNDRED;
        let wrapper_count = hundreds * WRAPPERS_PER_HUNDRED;
        let mut observed_processes = Vec::with_capacity(process_count);

        for index in 0..cargo_root_count {
            observed_processes.push(self.cargo_root(index));
        }
        for index in 0..compiler_count {
            let cargo_root_index = index % cargo_root_count.max(1);
            let cargo_root = &observed_processes[cargo_root_index];
            observed_processes.push(self.compiler(
                cargo_root_count + index,
                cargo_root_index,
                cargo_root,
            ));
        }
        for index in 0..wrapper_count {
            let cargo_root_index = index % cargo_root_count.max(1);
            let cargo_root = &observed_processes[cargo_root_index];
            observed_processes.push(self.wrapper(
                cargo_root_count + compiler_count + index,
                cargo_root_index,
                cargo_root,
            ));
        }
        for index in observed_processes.len()..process_count {
            observed_processes.push(unrelated_process(index));
        }
        observed_processes
    }

    fn checkout_root(&self, index: usize) -> PathBuf {
        checkout_root_under(self.temp_dir.path(), index)
    }

    fn cargo_root(&self, index: usize) -> ObservedProcess {
        let package = format!("fixture-package-{index}");
        let arguments = ["cargo", "build", "--package", package.as_str()];
        observed(index, &arguments.join(" "), "/usr/bin/cargo", &arguments)
            .with_cwd(&self.checkout_root(index))
            .with_candidate_role(ObservedCandidateRole::Cargo)
    }

    /// One compiler child of the Cargo root at `cargo_root_index`. Its working
    /// directory and `--out-dir` come from that root's checkout, not from its
    /// own global process index, or attribution would be timed entirely on its
    /// output-directory and member-root miss paths.
    fn compiler(
        &self,
        index: usize,
        cargo_root_index: usize,
        cargo_root: &ObservedProcess,
    ) -> ObservedProcess {
        let crate_name = format!("fixture_crate_{index}");
        let out_dir = self
            .checkout_root(cargo_root_index)
            .join("target/debug/deps")
            .to_string_lossy()
            .into_owned();
        let arguments = [
            "rustc",
            "--crate-name",
            crate_name.as_str(),
            "--edition=2024",
            "--out-dir",
            out_dir.as_str(),
        ];
        observed(index, &arguments.join(" "), "/usr/bin/rustc", &arguments)
            .with_cwd(&self.checkout_root(cargo_root_index))
            .with_validated_parent(cargo_root.identity())
            .with_candidate_role(ObservedCandidateRole::Compiler)
    }

    /// One compiler-wrapper child, in the same checkout as the Cargo root it
    /// was launched by.
    fn wrapper(
        &self,
        index: usize,
        cargo_root_index: usize,
        cargo_root: &ObservedProcess,
    ) -> ObservedProcess {
        let arguments = ["sccache", "rustc", "--crate-name", "fixture_wrapped"];
        observed(index, &arguments.join(" "), "/usr/bin/sccache", &arguments)
            .with_cwd(&self.checkout_root(cargo_root_index))
            .with_validated_parent(cargo_root.identity())
            .with_candidate_role(ObservedCandidateRole::Wrapper)
    }

    /// Assemble the snapshot classification reads, timed separately so its cost
    /// stays out of the classification increment.
    fn measure_snapshot_assembly(&self) -> (Duration, ProcessObservationSnapshot) {
        let started = Instant::now();
        let process_observation_snapshot =
            snapshot_builder::snapshot_of_at(Instant::now(), &self.observed_processes);
        let elapsed = started.elapsed();
        (elapsed, process_observation_snapshot)
    }

    fn measure_classification_cycle(
        &self,
        build_classifier: &mut BuildClassifier,
        process_observation_snapshot: &ProcessObservationSnapshot,
    ) -> Duration {
        let started = Instant::now();
        let build_classification = build_classifier.classify_cycle(
            process_observation_snapshot,
            &self.cargo_workspace_index,
            &OwnedRootEvidence::NoLiveRoot,
            Instant::now(),
        );
        let elapsed = started.elapsed();
        black_box(build_classification);
        elapsed
    }

    fn classify_once(&self, build_classifier: &mut BuildClassifier) -> BuildClassification {
        let (_, process_observation_snapshot) = self.measure_snapshot_assembly();
        build_classifier.classify_cycle(
            &process_observation_snapshot,
            &self.cargo_workspace_index,
            &OwnedRootEvidence::NoLiveRoot,
            Instant::now(),
        )
    }
}

/// One indexed checkout directory, named by its index so every fixture path is
/// reproducible from the temporary root alone.
fn checkout_root_under(temporary_root: &Path, index: usize) -> PathBuf {
    temporary_root.join(format!("checkout-{}", index % INDEXED_CHECKOUT_COUNT))
}

fn observed(
    index: usize,
    fingerprint_source: &str,
    executable: &str,
    argv: &[&str],
) -> ObservedProcess {
    let pid = u32::try_from(index + 1).expect("fixture process count fits in u32");
    ObservedProcess::new(
        pid,
        u64::from(pid) + FIXTURE_PROCESS_CREATION_TOKEN_OFFSET,
        fingerprint_source,
        executable,
        argv,
    )
}

fn unrelated_process(index: usize) -> ObservedProcess {
    let name = format!("unrelated-{index}");
    observed(index, &name, &format!("/usr/bin/{name}"), &[name.as_str()])
}

fn workspace_metadata(checkout_root: &Path) -> WorkspaceMetadata {
    WorkspaceMetadata {
        declared_checkout_root:   AbsolutePath::from(checkout_root),
        cargo_workspace_root:     AbsolutePath::from(checkout_root),
        target_directory:         AbsolutePath::from(checkout_root.join("target").as_path()),
        packages:                 HashMap::new(),
        fingerprint:              ManifestFingerprint {
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

fn measure_fixture(process_count: usize) -> FixtureBackedClassificationTimingSamples {
    let fixture = DeterministicClassificationFixture::new(process_count);
    let mut build_classifier = BuildClassifier::default();

    black_box(fixture.classify_once(&mut build_classifier));

    let mut snapshot_assemblies = [Duration::ZERO; REPEATED_SAMPLE_COUNT];
    let mut classification_cycles = [Duration::ZERO; REPEATED_SAMPLE_COUNT];
    for sample in 0..REPEATED_SAMPLE_COUNT {
        let (assembly_elapsed, process_observation_snapshot) = fixture.measure_snapshot_assembly();
        snapshot_assemblies[sample] = assembly_elapsed;
        classification_cycles[sample] = fixture
            .measure_classification_cycle(&mut build_classifier, &process_observation_snapshot);
    }
    FixtureBackedClassificationTimingSamples {
        process_count,
        snapshot_assemblies,
        classification_cycles,
    }
}

#[test]
fn report_fixture_backed_build_classification_timings() {
    let current_samples: Vec<_> = PROCESS_COUNTS.into_iter().map(measure_fixture).collect();
    eprintln!(
        "BuildClassifier::classify_cycle timing over deterministic on-disk checkouts, reported as \
         the increment on top of the observer-only baseline in process_observation::benchmarks; \
         snapshot assembly is measured separately and excluded, canonicalization and \
         target-directory resolution are charged to classification. \
         budget={EVENT_LOOP_PROCESS_REFRESH_BUDGET:?}, recorded_on={RECORDED_TIMING_DATE}, \
         recorded_samples={RECORDED_FIXTURE_BACKED_CLASSIFICATION_TIMINGS:#?}, \
         current_samples={current_samples:#?}"
    );
    for samples in &current_samples {
        eprintln!(
            "process_count={}, slowest_snapshot_assembly={:?}, \
             slowest_classification_increment={:?}, classification_cycle_placement={:?}",
            samples.process_count,
            samples.slowest_snapshot_assembly(),
            samples.slowest_classification_cycle(),
            samples.classification_cycle_placement(),
        );
    }
}

/// Every Cargo root in the population must open a session and its compiler
/// children must be attributed, or the timings measure attribution misses
/// instead of the representative build population.
#[test]
fn the_fixture_population_produces_classified_build_sessions() {
    let fixture = DeterministicClassificationFixture::new(SMALL_FIXTURE_PROCESS_COUNT);
    let mut build_classifier = BuildClassifier::default();
    let expected_session_count = SMALL_FIXTURE_PROCESS_COUNT / 100 * CARGO_ROOTS_PER_HUNDRED;

    let build_classification = fixture.classify_once(&mut build_classifier);

    assert_eq!(
        build_classification.build_sessions().len(),
        expected_session_count,
        "every fixture Cargo root must open its own build session"
    );
    assert!(
        !build_classification.compile_activities().is_empty(),
        "the fixture compilers must be attributed to their own root's session"
    );
}

/// The load the monitor actually runs under must classify inside the same
/// event-loop refresh budget the backend selection is made against. This is the
/// only row whose measurement a per-candidate regression — an added
/// `fs::canonicalize` per build candidate, say — can push over that budget.
#[test]
fn the_monitor_scale_population_classifies_inside_the_refresh_budget() {
    let samples = measure_fixture(MONITOR_SCALE_FIXTURE_PROCESS_COUNT);

    assert_eq!(
        samples.classification_cycle_placement(),
        ClassificationCyclePlacement::EventLoopAffordable,
        "slowest classification increment over {} build candidates was {:?}",
        MONITOR_SCALE_FIXTURE_PROCESS_COUNT / 100
            * (CARGO_ROOTS_PER_HUNDRED + COMPILERS_PER_HUNDRED + WRAPPERS_PER_HUNDRED),
        samples.slowest_classification_cycle(),
    );
}

#[test]
fn a_recorded_sample_under_the_refresh_budget_is_event_loop_affordable() {
    let samples = FixtureBackedClassificationTimingSamples {
        process_count:         SMALL_FIXTURE_PROCESS_COUNT,
        snapshot_assemblies:   [Duration::ZERO; REPEATED_SAMPLE_COUNT],
        classification_cycles: [EVENT_LOOP_PROCESS_REFRESH_BUDGET; REPEATED_SAMPLE_COUNT],
    };

    assert_eq!(
        samples.classification_cycle_placement(),
        ClassificationCyclePlacement::EventLoopAffordable
    );
}

#[test]
fn a_recorded_sample_above_the_refresh_budget_requires_the_dedicated_worker() {
    let samples = FixtureBackedClassificationTimingSamples {
        process_count:         SMALL_FIXTURE_PROCESS_COUNT,
        snapshot_assemblies:   [Duration::ZERO; REPEATED_SAMPLE_COUNT],
        classification_cycles: [
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            EVENT_LOOP_PROCESS_REFRESH_BUDGET + Duration::from_nanos(1),
        ],
    };

    assert_eq!(
        samples.classification_cycle_placement(),
        ClassificationCyclePlacement::RequiresDedicatedWorker
    );
}
