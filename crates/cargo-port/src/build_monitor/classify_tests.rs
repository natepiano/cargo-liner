//! Classification behavior that is decided by argv, cached evidence, and
//! collected candidates alone.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::time::Instant;

use super::activity::CompilationTarget;
use super::activity::CompileActivity;
use super::activity::CompiledCrateIdentity;
use super::activity::CompilerAttribution;
use super::activity::CompilerKind;
use super::activity::CompilerSessionCandidates;
use super::activity::ResolvedCompilerAttribution;
use super::build_classifier::BuildClassifier;
use super::classify::ArgumentPath;
use super::classify::BuildClassification;
use super::classify::CargoSubcommandRecognition;
use super::classify::observed_process_path_arguments;
use super::execution::CompileClassificationCancellation;
use super::execution::CompileClassificationDemand;
use super::execution::CompileClassificationExecution;
use super::execution::CompileMonitorGeneration;
use super::scope::BuildScopeKey;
use super::session::BuildProfileAttribution;
use super::session::BuildProfileLabel;
use super::session::BuildSession;
use super::session::BuildSessionId;
use super::session::CargoCommandSelector;
use super::session::CargoSubcommand;
use super::session::LiveOwnedRoot;
use super::session::OwnedRootEvidence;
use super::session::OwnedRootLifecycle;
use super::session::ScopeAttribution;
use super::session::SessionScope;
use super::session::SessionTargetDirectory;
use super::session::TargetDirectoryEvidence;
use super::termination::OwnedTerminationSupport;
use crate::process_observation::BuildCandidateRole;
use crate::process_observation::ProcessObserver;
use crate::process_observation::identity::ProcessIdentity;
use crate::process_observation::identity::ProcessIncarnation;
use crate::process_observation::snapshot_builder::ObservedCandidateRole;
use crate::process_observation::snapshot_builder::ObservedProcess;
use crate::process_observation::snapshot_builder::snapshot_of;
use crate::project::AbsolutePath;
use crate::project::CargoWorkspaceIndex;
use crate::project::FileStamp;
use crate::project::ManifestFingerprint;
use crate::project::ProjectListRevision;
use crate::project::WorkspaceMetadata;
use crate::project::WorkspaceMetadataStore;
use crate::tui::OwnedRunId;

fn build_session_id() -> BuildSessionId {
    BuildSessionId::for_test(ProcessIncarnation::for_test(
        ProcessIdentity::for_test(4_242, 1),
        "cargo build",
    ))
}

/// The version the one unattributed activity's dependency manifest named.
fn dependency_package_version(
    build_classification: &BuildClassification,
) -> Result<String, Box<dyn std::error::Error>> {
    let [activity] = build_classification.unattributed_compile_activities() else {
        return Err("the dependency compiler is unattributed".into());
    };
    let CompiledCrateIdentity::DependencyPackage(manifest_package_identity) =
        activity.compiled_crate_identity()
    else {
        return Err("the cached manifest identifies the dependency exactly".into());
    };
    Ok(manifest_package_identity.version().to_owned())
}

fn argv(arguments: &[&str]) -> Vec<OsString> {
    arguments
        .iter()
        .map(|argument| OsString::from(*argument))
        .collect()
}

#[test]
fn build_subcommands_are_recognized_as_building() {
    for subcommand in [
        "bench", "build", "check", "clippy", "doc", "fix", "install", "nextest", "package",
        "publish", "run", "rustc", "rustdoc", "test",
    ] {
        assert_eq!(
            CargoSubcommandRecognition::from_subcommand(subcommand),
            CargoSubcommandRecognition::Build,
            "{subcommand} should be recognized as a build subcommand"
        );
    }
}

#[test]
fn non_build_subcommands_are_recognized_as_non_building() {
    for subcommand in [
        "add",
        "clean",
        "config",
        "fetch",
        "generate-lockfile",
        "help",
        "init",
        "locate-project",
        "login",
        "logout",
        "metadata",
        "new",
        "owner",
        "pkgid",
        "read-manifest",
        "remove",
        "search",
        "tree",
        "uninstall",
        "update",
        "vendor",
        "verify-project",
        "version",
        "yank",
    ] {
        assert_eq!(
            CargoSubcommandRecognition::from_subcommand(subcommand),
            CargoSubcommandRecognition::NonBuild,
            "{subcommand} should be recognized as a non-build subcommand"
        );
    }
}

#[test]
fn builtin_aliases_normalize_to_their_subcommands() {
    for (alias, expected) in [
        ("b", CargoSubcommandRecognition::Build),
        ("c", CargoSubcommandRecognition::Build),
        ("d", CargoSubcommandRecognition::Build),
        ("r", CargoSubcommandRecognition::Build),
        ("t", CargoSubcommandRecognition::Build),
        ("rm", CargoSubcommandRecognition::NonBuild),
        ("ver", CargoSubcommandRecognition::NonBuild),
    ] {
        assert_eq!(
            CargoSubcommandRecognition::from_subcommand(alias),
            expected,
            "built-in alias {alias} should normalize before recognition"
        );
    }
}

#[test]
fn configured_aliases_and_third_party_plugins_stay_unrecognized() {
    // Neither name is a built-in subcommand, and deciding them would mean
    // reading a per-checkout `.cargo/config.toml`, which classification never
    // does. A compiler descendant promotes them instead.
    for subcommand in ["lint", "machete", "expand", "watch", "make"] {
        assert_eq!(
            CargoSubcommandRecognition::from_subcommand(subcommand),
            CargoSubcommandRecognition::Unrecognized,
            "{subcommand} is not a built-in subcommand"
        );
    }
}

#[test]
fn manifest_path_argument_is_read_in_both_spellings() {
    let separate = observed_process_path_arguments(
        BuildCandidateRole::Cargo,
        &argv(&["cargo", "build", "--manifest-path", "/checkout/Cargo.toml"]),
    );
    let joined = observed_process_path_arguments(
        BuildCandidateRole::Cargo,
        &argv(&["cargo", "build", "--manifest-path=/checkout/Cargo.toml"]),
    );
    assert_eq!(
        separate.manifest_directory,
        ArgumentPath::Named(PathBuf::from("/checkout"))
    );
    assert_eq!(
        joined.manifest_directory,
        ArgumentPath::Named(PathBuf::from("/checkout"))
    );
}

#[test]
fn absolute_manifest_argument_without_a_flag_is_read() {
    let observed = observed_process_path_arguments(
        BuildCandidateRole::Cargo,
        &argv(&["cargo", "package", "/checkout/member/Cargo.toml"]),
    );
    assert_eq!(
        observed.manifest_directory,
        ArgumentPath::Named(PathBuf::from("/checkout/member"))
    );
}

#[test]
fn target_directory_argument_is_read() {
    let observed = observed_process_path_arguments(
        BuildCandidateRole::Cargo,
        &argv(&["cargo", "build", "--target-dir", "/shared/target"]),
    );
    assert_eq!(
        observed.target_directory_argument,
        ArgumentPath::Named(PathBuf::from("/shared/target"))
    );
}

#[test]
fn compiler_output_directory_and_primary_input_are_read() {
    let observed = observed_process_path_arguments(
        BuildCandidateRole::Compiler,
        &argv(&[
            "rustc",
            "--crate-name",
            "example",
            "/checkout/src/lib.rs",
            "--out-dir",
            "/checkout/target/debug/deps",
        ]),
    );
    assert_eq!(
        observed.primary_input,
        ArgumentPath::Named(PathBuf::from("/checkout/src/lib.rs"))
    );
    assert_eq!(
        observed.output_directory,
        ArgumentPath::Named(PathBuf::from("/checkout/target/debug/deps"))
    );
}

#[test]
fn a_cargo_root_never_claims_a_primary_input() {
    // `cargo run src/main.rs` is not a compile of that file; only a compiler
    // process names a primary input.
    let observed = observed_process_path_arguments(
        BuildCandidateRole::Cargo,
        &argv(&["cargo", "build", "/checkout/src/lib.rs"]),
    );
    assert_eq!(observed.primary_input, ArgumentPath::NotNamed);
}

#[test]
fn no_candidate_is_unattributed() {
    let candidates = CompilerSessionCandidates::default();
    assert_eq!(
        candidates.attribution(ResolvedCompilerAttribution::ValidatedParentChain),
        CompilerAttribution::Unattributed
    );
}

#[test]
fn a_degraded_cycle_never_produces_a_unique_match() {
    // One candidate the uniqueness test could not see makes every "unique"
    // match in the cycle possibly false-unique, so even a single surviving
    // candidate stays ambiguous and cannot authorize termination.
    let mut candidates = CompilerSessionCandidates::default();
    assert_eq!(
        candidates.clone().degraded_attribution(),
        CompilerAttribution::Unattributed
    );
    candidates.include(build_session_id());
    assert!(matches!(
        candidates.degraded_attribution(),
        CompilerAttribution::Ambiguous { .. }
    ));
}

#[test]
fn repeated_candidates_do_not_manufacture_ambiguity() {
    let mut candidates = CompilerSessionCandidates::default();
    candidates.include(build_session_id());
    candidates.include(build_session_id());
    assert_eq!(candidates.len(), 1);
}

#[test]
fn profile_labels_map_to_their_build_directories() {
    assert_eq!(BuildProfileLabel::from_label("dev"), BuildProfileLabel::Dev);
    assert_eq!(
        BuildProfileLabel::from_label("release"),
        BuildProfileLabel::Release
    );
    assert_eq!(
        BuildProfileLabel::from_label("bench-fast"),
        BuildProfileLabel::Custom("bench-fast".to_owned())
    );
    assert_eq!(BuildProfileLabel::Dev.build_directory_name(), "debug");
    assert_eq!(BuildProfileLabel::Release.build_directory_name(), "release");
    assert_eq!(
        BuildProfileLabel::Custom("bench-fast".to_owned()).build_directory_name(),
        "bench-fast"
    );
}

#[test]
fn an_unobservable_target_directory_never_matches_output() {
    assert_eq!(
        SessionTargetDirectory::Unobservable.evidence(),
        TargetDirectoryEvidence::Unobservable
    );
}

// --- whole-cycle classification -------------------------------------------

/// One indexed workspace on disk, plus the classifier that reads it.
pub(super) struct ClassificationFixture {
    temp_dir:                         tempfile::TempDir,
    pub(super) checkout_root:         std::path::PathBuf,
    workspace_metadata_store:         WorkspaceMetadataStore,
    pub(super) cargo_workspace_index: CargoWorkspaceIndex,
    pub(super) build_classifier:      BuildClassifier,
}

impl ClassificationFixture {
    pub(super) fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let mut fixture = Self {
            checkout_root: temp_dir.path().join("checkout"),
            workspace_metadata_store: WorkspaceMetadataStore::new(),
            cargo_workspace_index: CargoWorkspaceIndex::from_metadata_store(
                &WorkspaceMetadataStore::new(),
                ProjectListRevision::default(),
            ),
            build_classifier: BuildClassifier::default(),
            temp_dir,
        };
        fixture.index_checkout("checkout")?;
        Ok(fixture)
    }

    /// Create one checkout directory with its own `target/` and index it as a
    /// workspace, so a test can state two live checkouts side by side.
    fn index_checkout(
        &mut self,
        name: &str,
    ) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
        let checkout_root = self.temp_dir.path().join(name);
        std::fs::create_dir_all(checkout_root.join("target"))?;
        std::fs::create_dir_all(checkout_root.join("src"))?;
        self.workspace_metadata_store.upsert(WorkspaceMetadata {
            declared_checkout_root:   AbsolutePath::from(checkout_root.as_path()),
            cargo_workspace_root:     AbsolutePath::from(checkout_root.as_path()),
            target_directory:         AbsolutePath::from(checkout_root.join("target").as_path()),
            packages:                 std::collections::HashMap::new(),
            fingerprint:              ManifestFingerprint {
                manifest:       FileStamp {
                    content_hash: [0_u8; 32],
                },
                lockfile:       None,
                rust_toolchain: None,
                configs:        BTreeMap::new(),
            },
            out_of_tree_target_bytes: None,
        });
        self.cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
            &self.workspace_metadata_store,
            ProjectListRevision::default(),
        );
        Ok(checkout_root)
    }

    pub(super) fn classify(
        &mut self,
        observed_processes: &[ObservedProcess],
    ) -> BuildClassification {
        self.classify_owned(observed_processes, &OwnedRootEvidence::NoLiveRoot)
    }

    /// Classify one cycle against `owned_root_evidence`, so a staged root can be
    /// attributed to the owned run the way a live owned build is.
    pub(super) fn classify_owned(
        &mut self,
        observed_processes: &[ObservedProcess],
        owned_root_evidence: &OwnedRootEvidence,
    ) -> BuildClassification {
        self.build_classifier.classify_cycle(
            &snapshot_of(observed_processes),
            &self.cargo_workspace_index,
            owned_root_evidence,
            Instant::now(),
        )
    }

    /// Write one dependency checkout outside the indexed workspace, so its
    /// package identity can only come from reading its own manifest.
    fn dependency_checkout(
        &self,
        name: &str,
        version: &str,
    ) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
        let dependency_root = self.temp_dir.path().join(format!("{name}-{version}"));
        std::fs::create_dir_all(dependency_root.join("src"))?;
        std::fs::write(
            dependency_root.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\n"),
        )?;
        std::fs::write(dependency_root.join("src/lib.rs"), "")?;
        Ok(dependency_root)
    }

    pub(super) fn cargo_root(&self, arguments: &[&str]) -> ObservedProcess {
        self.cargo_root_with_pid(101, arguments)
    }

    /// A Cargo root under a caller-chosen pid, so a test can state several live
    /// roots in one checkout and get one session per root.
    pub(super) fn cargo_root_with_pid(&self, pid: u32, arguments: &[&str]) -> ObservedProcess {
        ObservedProcess::new(pid, 1, &arguments.join(" "), "/usr/bin/cargo", arguments)
            .with_cwd(&self.checkout_root)
            .with_candidate_role(ObservedCandidateRole::Cargo)
    }

    /// One `rustc` under `cargo_root`'s validated parentage, so classification
    /// attributes it to that root's session and no other.
    pub(super) fn compiler_under(&self, pid: u32, cargo_root: &ObservedProcess) -> ObservedProcess {
        let compiled_crate_name = format!("crate{pid}");
        ObservedProcess::new(
            pid,
            1,
            &format!("rustc {compiled_crate_name}"),
            "/usr/bin/rustc",
            &["rustc", "--crate-name", &compiled_crate_name, "src/lib.rs"],
        )
        .with_cwd(&self.checkout_root)
        .with_validated_parent(cargo_root.identity())
        .with_candidate_role(ObservedCandidateRole::Compiler)
    }
}

#[test]
fn a_building_cargo_root_in_an_indexed_checkout_opens_one_resolved_session()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);

    let build_classification = fixture.classify(&[cargo_root]);

    let [build_session] = build_classification.build_sessions() else {
        return Err("one build session should be classified".into());
    };
    assert!(matches!(
        build_session.session_scope(),
        SessionScope::Resolved {
            method: ScopeAttribution::WorkingDirectoryManifest,
            ..
        }
    ));
    assert_eq!(
        build_session.build_profile().label(),
        &BuildProfileLabel::Dev
    );
    Ok(())
}

#[test]
fn a_session_records_the_operative_command_and_the_root_it_was_observed_on()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&[
        "cargo",
        "build",
        "--workspace",
        "-p",
        "demo",
        "--bin",
        "demo",
        "--tests",
        "--benches",
        "--all-targets",
    ]);

    let build_classification = fixture.classify(&[cargo_root]);

    let [build_session] = build_classification.build_sessions() else {
        return Err("one build session should be classified".into());
    };
    let operative_cargo_command = build_session.operative_cargo_command();
    assert_eq!(
        operative_cargo_command.subcommand(),
        &CargoSubcommand::Named("build".to_string())
    );
    assert_eq!(
        operative_cargo_command.selectors().to_vec(),
        vec![
            CargoCommandSelector::AllPackages,
            CargoCommandSelector::Package("demo".to_string()),
            CargoCommandSelector::Binary("demo".to_string()),
            CargoCommandSelector::AllTests,
            CargoCommandSelector::AllBenchmarks,
            CargoCommandSelector::AllTargets,
        ]
    );
    assert_eq!(build_session.root_observation().root_pid(), 101);
    assert_eq!(
        build_session.root_identity(),
        build_session.root_observation().root_identity()
    );
    Ok(())
}

/// Classification rebuilds every session from scratch each cycle, so the root
/// observation must carry the ledger's first sighting rather than the current
/// cycle's instant. Re-stamping it would make a ten-minute build report the age
/// of one refresh interval forever.
#[test]
fn a_session_keeps_its_first_observed_instant_across_cycles()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);

    let first_cycle = fixture.classify(std::slice::from_ref(&cargo_root));
    let [first_session] = first_cycle.build_sessions() else {
        return Err("one build session should be classified".into());
    };
    let first_observed_at = first_session.root_observation().first_observed_at();

    let second_cycle = fixture.classify(std::slice::from_ref(&cargo_root));
    let [second_session] = second_cycle.build_sessions() else {
        return Err("the same build session should be classified again".into());
    };

    assert_eq!(
        second_session.root_observation().first_observed_at(),
        first_observed_at
    );
    Ok(())
}

#[test]
fn a_non_building_subcommand_opens_no_session_until_a_compiler_child_promotes_it()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "fmt"]);
    let compiler = ObservedProcess::new(
        202,
        2,
        "rustc lib",
        "/usr/bin/rustc",
        &["rustc", "--crate-name", "demo", "src/lib.rs"],
    )
    .with_cwd(&fixture.checkout_root)
    .with_validated_parent(cargo_root.identity())
    .with_candidate_role(ObservedCandidateRole::Compiler);

    assert!(
        fixture
            .classify(std::slice::from_ref(&cargo_root))
            .build_sessions()
            .is_empty(),
        "a formatting root compiles nothing"
    );

    let promoted = fixture.classify(&[cargo_root, compiler]);
    assert_eq!(promoted.build_sessions().len(), 1);
    assert_eq!(promoted.compile_activities().len(), 1);
    Ok(())
}

#[test]
fn promotion_and_first_seen_survive_a_cycle_whose_working_directory_is_unreadable()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "fmt"]);
    let compiler = ObservedProcess::new(
        202,
        2,
        "rustc lib",
        "/usr/bin/rustc",
        &["rustc", "--crate-name", "demo", "src/lib.rs"],
    )
    .with_cwd(&fixture.checkout_root)
    .with_validated_parent(cargo_root.identity())
    .with_candidate_role(ObservedCandidateRole::Compiler);

    let promoted = fixture.classify(&[cargo_root.clone(), compiler]);
    let first_seen = promoted.build_sessions()[0].first_seen();

    let blinded = fixture.classify(&[cargo_root.clone().with_unreadable_cwd()]);
    assert!(
        blinded
            .observed_incarnations()
            .contains(cargo_root.incarnation()),
        "an unreadable working directory still observes the incarnation"
    );
    assert_eq!(
        blinded.build_sessions().len(),
        1,
        "an unreadable working directory degrades the evidence that depends on it, not the row"
    );

    let recovered = fixture.classify(&[cargo_root]);
    assert_eq!(
        recovered.build_sessions().len(),
        1,
        "sticky promotion survives the blinded cycle"
    );
    assert_eq!(
        recovered.build_sessions()[0].first_seen(),
        first_seen,
        "first-seen order survives the blinded cycle"
    );
    Ok(())
}

#[test]
fn classifying_the_same_snapshot_twice_produces_the_same_result()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build", "--release"]);
    let process_observation_snapshot = snapshot_of(&[cargo_root]);
    let cycle_instant = Instant::now();

    let first = fixture.build_classifier.classify_cycle(
        &process_observation_snapshot,
        &fixture.cargo_workspace_index,
        &OwnedRootEvidence::NoLiveRoot,
        cycle_instant,
    );
    let second = fixture.build_classifier.classify_cycle(
        &process_observation_snapshot,
        &fixture.cargo_workspace_index,
        &OwnedRootEvidence::NoLiveRoot,
        cycle_instant,
    );

    assert_eq!(first.build_sessions(), second.build_sessions());
    assert_eq!(first.compile_activities(), second.compile_activities());
    assert_eq!(
        first.build_sessions()[0].build_profile().label(),
        &BuildProfileLabel::Release
    );
    assert!(
        first.build_classification_cycle() < second.build_classification_cycle(),
        "each cycle advances the counter even when the observation is unchanged"
    );
    assert_eq!(first.cycle_instant(), cycle_instant);
    assert_eq!(second.cycle_instant(), cycle_instant);
    Ok(())
}

#[test]
fn a_verified_owned_root_is_attributed_to_the_owned_run() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let owned_root_evidence = OwnedRootEvidence::Root(LiveOwnedRoot::new(
        OwnedRunId::for_test(NonZeroU64::MIN),
        cargo_root.identity().clone(),
        std::fs::canonicalize(&fixture.checkout_root)?,
        OwnedRootLifecycle::Live,
    ));

    let build_classification = fixture.build_classifier.classify_cycle(
        &snapshot_of(&[cargo_root]),
        &fixture.cargo_workspace_index,
        &owned_root_evidence,
        Instant::now(),
    );

    assert!(matches!(
        build_classification.build_sessions()[0].session_scope(),
        SessionScope::Resolved {
            method: ScopeAttribution::OwnedRoot,
            ..
        }
    ));
    Ok(())
}

#[test]
fn compiler_descendants_are_classified_by_the_executable_they_run()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "doc"]);
    let rustdoc = ObservedProcess::new(
        303,
        3,
        "rustdoc lib",
        "/usr/bin/rustdoc",
        &["rustdoc", "--crate-name", "demo", "src/lib.rs"],
    )
    .with_cwd(&fixture.checkout_root)
    .with_validated_parent(cargo_root.identity())
    .with_candidate_role(ObservedCandidateRole::Compiler);
    let build_script = ObservedProcess::new(
        404,
        4,
        "build script",
        "/tmp/target/debug/build/demo-1/build-script-build",
        &["build-script-build"],
    )
    .with_cwd(&fixture.checkout_root)
    .with_validated_parent(cargo_root.identity())
    .with_candidate_role(ObservedCandidateRole::Compiler);
    let linker = ObservedProcess::new(
        505,
        5,
        "linker",
        "/usr/bin/cc",
        &["cc", "demo.rcgu.o", "-o", "demo"],
    )
    .with_cwd(&fixture.checkout_root)
    .with_validated_parent(cargo_root.identity())
    .with_candidate_role(ObservedCandidateRole::Compiler);

    let build_classification = fixture.classify(&[cargo_root, rustdoc, build_script, linker]);

    let compiler_kinds: Vec<CompilerKind> = build_classification
        .compile_activities()
        .iter()
        .map(CompileActivity::compiler_kind)
        .collect();
    assert!(compiler_kinds.contains(&CompilerKind::Rustdoc));
    assert!(compiler_kinds.contains(&CompilerKind::BuildScript));
    assert!(compiler_kinds.contains(&CompilerKind::Linker));
    Ok(())
}

#[test]
fn a_clippy_driver_compiler_is_not_reported_as_a_generic_wrapper()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "clippy"]);
    let clippy_driver = ObservedProcess::new(
        606,
        6,
        "clippy",
        "/usr/bin/clippy-driver",
        &["clippy-driver", "--crate-name", "demo", "src/lib.rs"],
    )
    .with_cwd(&fixture.checkout_root)
    .with_validated_parent(cargo_root.identity())
    .with_candidate_role(ObservedCandidateRole::Wrapper);

    let build_classification = fixture.classify(&[cargo_root, clippy_driver]);

    assert_eq!(
        build_classification.compile_activities()[0].compiler_kind(),
        CompilerKind::ClippyDriver
    );
    Ok(())
}

#[test]
fn an_unrecognized_subcommand_is_not_a_building_root() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let plugin_root = fixture.cargo_root(&["cargo", "port"]);

    assert!(fixture.classify(&[plugin_root]).build_sessions().is_empty());
    Ok(())
}

#[test]
fn a_compiler_with_no_reachable_root_is_reported_as_unattributed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let orphan_compiler = ObservedProcess::new(
        707,
        7,
        "rustc dep",
        "/usr/bin/rustc",
        &[
            "rustc",
            "--crate-name",
            "serde",
            "--out-dir",
            "/elsewhere/target/debug/deps",
            "/registry/serde-1.0.0/src/lib.rs",
        ],
    )
    .with_cwd(&fixture.checkout_root)
    .with_unobservable_parentage()
    .with_candidate_role(ObservedCandidateRole::Compiler);

    let build_classification = fixture.classify(&[orphan_compiler]);

    let [unattributed] = build_classification.unattributed_compile_activities() else {
        return Err("an unreachable compiler is unattributed".into());
    };
    assert_eq!(unattributed.compiler_kind(), CompilerKind::Rustc);
    assert_eq!(
        unattributed.compiler_attribution(),
        &CompilerAttribution::Unattributed
    );
    assert!(matches!(
        unattributed.compiled_crate_identity(),
        CompiledCrateIdentity::CrateNameOnly(compiled_crate_name)
            if compiled_crate_name.as_str() == "serde"
    ));
    assert_eq!(
        fixture
            .build_classifier
            .dependency_manifest_snapshot()
            .len(),
        build_classification
            .dependency_manifest_lookup_requests()
            .len(),
        "an unindexed source root asks for its manifest once and is cached under that request"
    );
    assert_eq!(
        fixture.build_classifier.first_seen_ledger().len(),
        1,
        "the ledger holds the one observed incarnation"
    );
    Ok(())
}

#[test]
fn a_profile_missing_from_argv_is_named_by_the_build_directory_its_compiler_writes_under()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let release_output_directory = fixture.checkout_root.join("target/release/deps");
    std::fs::create_dir_all(&release_output_directory)?;
    // A `.cargo/config.toml` profile or a Cargo alias never reaches argv, so
    // the output path is the only evidence of the profile.
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let compiler = ObservedProcess::new(
        808,
        8,
        "rustc release",
        "/usr/bin/rustc",
        &[
            "rustc",
            "--crate-name",
            "demo",
            "src/lib.rs",
            "--out-dir",
            &release_output_directory.to_string_lossy(),
        ],
    )
    .with_cwd(&fixture.checkout_root)
    .with_validated_parent(cargo_root.identity())
    .with_candidate_role(ObservedCandidateRole::Compiler);

    let build_classification = fixture.classify(&[cargo_root, compiler]);

    let [build_session] = build_classification.build_sessions() else {
        return Err("one build session should be classified".into());
    };
    assert_eq!(
        build_session.build_profile().label(),
        &BuildProfileLabel::Release
    );
    assert_eq!(
        build_session.build_profile().attribution(),
        BuildProfileAttribution::OutputDirectory
    );
    assert_eq!(
        build_session.cargo_subcommand_recognition(),
        CargoSubcommandRecognition::Build
    );
    assert!(matches!(
        build_session.session_target_directory().evidence(),
        TargetDirectoryEvidence::Determined(_)
    ));

    let [compile_activity] = build_classification.compile_activities() else {
        return Err("the compiler child should be one attributed activity".into());
    };
    assert_eq!(
        compile_activity.compiler_attribution(),
        &CompilerAttribution::Confirmed(build_session.build_session_id().clone())
    );
    assert_eq!(
        compile_activity.compilation_target(),
        &CompilationTarget::Host
    );
    assert!(matches!(
        compile_activity.compiled_crate_identity(),
        CompiledCrateIdentity::CrateNameOnly(compiled_crate_name)
            if compiled_crate_name.as_str() == "demo"
    ));
    Ok(())
}

#[test]
fn a_dependency_source_root_is_identified_from_its_own_manifest_on_the_next_cycle()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let dependency_root = fixture.dependency_checkout("serde", "1.0.0")?;
    let compiler = ObservedProcess::new(
        909,
        9,
        "rustc serde",
        "/usr/bin/rustc",
        &[
            "rustc",
            "--crate-name",
            "serde",
            &dependency_root.join("src/lib.rs").to_string_lossy(),
        ],
    )
    .with_cwd(&fixture.checkout_root)
    .with_unobservable_parentage()
    .with_candidate_role(ObservedCandidateRole::Compiler);

    let first_cycle = fixture.classify(std::slice::from_ref(&compiler));
    let [first_activity] = first_cycle.unattributed_compile_activities() else {
        return Err("the dependency compiler is unattributed".into());
    };
    assert!(matches!(
        first_activity.compiled_crate_identity(),
        CompiledCrateIdentity::CrateNameOnly(_)
    ));
    assert_eq!(
        first_cycle.dependency_manifest_lookup_requests().len(),
        1,
        "a source root absent from the snapshot asks for its manifest once"
    );

    let second_cycle = fixture.classify(&[compiler]);
    let [second_activity] = second_cycle.unattributed_compile_activities() else {
        return Err("the dependency compiler is still unattributed".into());
    };
    let CompiledCrateIdentity::DependencyPackage(manifest_package_identity) =
        second_activity.compiled_crate_identity()
    else {
        return Err("the cached manifest identifies the dependency exactly".into());
    };
    assert_eq!(manifest_package_identity.name(), "serde");
    assert_eq!(manifest_package_identity.version(), "1.0.0");
    assert!(
        second_cycle
            .dependency_manifest_lookup_requests()
            .is_empty(),
        "the second cycle reads no manifest it already cached"
    );
    Ok(())
}

#[test]
fn a_nested_cargo_joins_the_outer_session_only_while_it_stays_in_the_same_checkout()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let sibling_checkout_root = fixture.index_checkout("sibling")?;
    let outer_root = fixture.cargo_root(&["cargo", "build"]);
    let nested_root = ObservedProcess::new(
        111,
        11,
        "cargo build nested",
        "/usr/bin/cargo",
        &["cargo", "build"],
    )
    .with_cwd(&fixture.checkout_root)
    .with_validated_parent(outer_root.identity())
    .with_candidate_role(ObservedCandidateRole::Cargo);
    let divergent_root = ObservedProcess::new(
        222,
        22,
        "cargo build divergent",
        "/usr/bin/cargo",
        &["cargo", "build"],
    )
    .with_cwd(&sibling_checkout_root)
    .with_validated_parent(outer_root.identity())
    .with_candidate_role(ObservedCandidateRole::Cargo);

    assert_eq!(
        fixture
            .classify(&[outer_root.clone(), nested_root])
            .build_sessions()
            .len(),
        1,
        "a nested Cargo whose scope matches its parent's is the same build"
    );
    assert_eq!(
        fixture
            .classify(&[outer_root, divergent_root])
            .build_sessions()
            .len(),
        2,
        "a nested Cargo that entered another checkout builds something else"
    );
    Ok(())
}

#[test]
fn two_sibling_cargo_roots_in_different_checkouts_stay_two_sessions()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let sibling_checkout_root = fixture.index_checkout("sibling")?;
    let first_root = fixture.cargo_root(&["cargo", "build"]);
    let second_root = ObservedProcess::new(
        333,
        33,
        "cargo build sibling",
        "/usr/bin/cargo",
        &["cargo", "build"],
    )
    .with_cwd(&sibling_checkout_root)
    .with_candidate_role(ObservedCandidateRole::Cargo);

    let build_classification = fixture.classify(&[first_root, second_root]);

    let [first_session, second_session] = build_classification.build_sessions() else {
        return Err("neither sibling root is nested under the other".into());
    };
    assert!(
        !first_session
            .session_scope()
            .shares_resolved_root(second_session.session_scope()),
        "each sibling session builds its own checkout"
    );
    Ok(())
}

#[test]
fn a_cache_daemon_serving_two_checkouts_through_one_target_directory_is_ambiguous()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let sibling_checkout_root = fixture.index_checkout("sibling")?;
    let shared_target_directory = fixture.temp_dir.path().join("shared-target");
    let shared_output_directory = shared_target_directory.join("debug/deps");
    std::fs::create_dir_all(&shared_output_directory)?;
    let shared_target_argument = shared_target_directory.to_string_lossy().into_owned();
    let first_root =
        fixture.cargo_root(&["cargo", "build", "--target-dir", &shared_target_argument]);
    let second_root = ObservedProcess::new(
        444,
        44,
        "cargo build sibling",
        "/usr/bin/cargo",
        &["cargo", "build", "--target-dir", &shared_target_argument],
    )
    .with_cwd(&sibling_checkout_root)
    .with_candidate_role(ObservedCandidateRole::Cargo);
    // One `sccache` server outlives every build it serves, so its parentage
    // reaches neither root and its output directory is all the evidence there
    // is.
    let cache_daemon = ObservedProcess::new(
        555,
        55,
        "sccache server",
        "/usr/local/bin/sccache",
        &[
            "sccache",
            "--crate-name",
            "demo",
            "--out-dir",
            &shared_output_directory.to_string_lossy(),
        ],
    )
    .with_cwd(fixture.temp_dir.path())
    .with_unobservable_parentage()
    .with_candidate_role(ObservedCandidateRole::Wrapper);

    let build_classification = fixture.classify(&[first_root, second_root, cache_daemon]);

    assert_eq!(
        build_classification.build_sessions().len(),
        2,
        "both checkouts are building"
    );
    let [cache_activity] = build_classification.unattributed_compile_activities() else {
        return Err("the cache daemon belongs to no single session".into());
    };
    let CompilerAttribution::Ambiguous { candidates } = cache_activity.compiler_attribution()
    else {
        return Err("a shared target directory cannot select a unique owner".into());
    };
    assert_eq!(
        candidates.len(),
        2,
        "both live sessions write under the shared target directory"
    );
    Ok(())
}

#[test]
fn a_same_pid_exec_replaces_the_session_and_activity_identities()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let compiler_arguments = ["rustc", "--crate-name", "demo", "src/lib.rs"];
    let before_root = fixture.cargo_root(&["cargo", "build"]);
    let before_compiler =
        ObservedProcess::new(666, 66, "rustc demo", "/usr/bin/rustc", &compiler_arguments)
            .with_cwd(&fixture.checkout_root)
            .with_validated_parent(before_root.identity())
            .with_candidate_role(ObservedCandidateRole::Compiler);

    let before = fixture.classify(&[before_root, before_compiler]);
    let [before_session] = before.build_sessions() else {
        return Err("one build session should be classified".into());
    };
    let [before_activity] = before.compile_activities() else {
        return Err("one compile activity should be classified".into());
    };
    let retained_build_session_id = before_session.build_session_id().clone();
    let retained_compile_activity_id = before_activity.compile_activity_id().clone();

    // The same PID and creation token, re-executed: only the exec fingerprint
    // changes, and it is what the identities are keyed by.
    let after_root =
        ObservedProcess::new(101, 1, "cargo test", "/usr/bin/cargo", &["cargo", "test"])
            .with_cwd(&fixture.checkout_root)
            .with_candidate_role(ObservedCandidateRole::Cargo);
    let after_compiler = ObservedProcess::new(
        666,
        66,
        "rustc demo tests",
        "/usr/bin/rustc",
        &compiler_arguments,
    )
    .with_cwd(&fixture.checkout_root)
    .with_validated_parent(after_root.identity())
    .with_candidate_role(ObservedCandidateRole::Compiler);

    let after = fixture.classify(&[after_root, after_compiler]);
    let [after_session] = after.build_sessions() else {
        return Err("the re-executed root opens one build session".into());
    };
    let [after_activity] = after.compile_activities() else {
        return Err("the re-executed compiler is one compile activity".into());
    };
    assert_ne!(
        after_session.build_session_id(),
        &retained_build_session_id,
        "a retained session identity stops matching after the exec"
    );
    assert_ne!(
        after_activity.compile_activity_id(),
        &retained_compile_activity_id,
        "a retained activity identity stops matching after the exec"
    );
    Ok(())
}

#[test]
fn compilers_sharing_one_target_directory_are_separated_by_their_primary_input()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let sibling_checkout_root = fixture.index_checkout("sibling")?;
    let shared_target_directory = fixture.temp_dir.path().join("shared-target");
    let shared_output_directory = shared_target_directory.join("debug/deps");
    std::fs::create_dir_all(&shared_output_directory)?;
    std::fs::write(fixture.checkout_root.join("src/lib.rs"), "")?;
    std::fs::write(sibling_checkout_root.join("src/lib.rs"), "")?;
    let dependency_root = fixture.dependency_checkout("serde", "1.0.0")?;
    let shared_target_argument = shared_target_directory.to_string_lossy().into_owned();
    let shared_output_argument = shared_output_directory.to_string_lossy().into_owned();
    let first_root =
        fixture.cargo_root(&["cargo", "build", "--target-dir", &shared_target_argument]);
    let second_root = ObservedProcess::new(
        777,
        77,
        "cargo build sibling",
        "/usr/bin/cargo",
        &["cargo", "build", "--target-dir", &shared_target_argument],
    )
    .with_cwd(&sibling_checkout_root)
    .with_candidate_role(ObservedCandidateRole::Cargo);
    let compiler_for = |pid: u32, fingerprint_source: &str, primary_input: &std::path::Path| {
        ObservedProcess::new(
            pid,
            u64::from(pid),
            fingerprint_source,
            "/usr/bin/rustc",
            &[
                "rustc",
                "--crate-name",
                "demo",
                &primary_input.to_string_lossy(),
                "--out-dir",
                &shared_output_argument,
            ],
        )
        .with_cwd(fixture.temp_dir.path())
        .with_unobservable_parentage()
        .with_candidate_role(ObservedCandidateRole::Compiler)
    };
    let first_compiler = compiler_for(
        888,
        "rustc first",
        &fixture.checkout_root.join("src/lib.rs"),
    );
    let second_compiler = compiler_for(
        999,
        "rustc second",
        &sibling_checkout_root.join("src/lib.rs"),
    );
    let dependency_compiler = compiler_for(
        1_010,
        "rustc dependency",
        &dependency_root.join("src/lib.rs"),
    );

    let build_classification = fixture.classify(&[
        first_root,
        second_root,
        first_compiler,
        second_compiler,
        dependency_compiler,
    ]);

    let [first_session, second_session] = build_classification.build_sessions() else {
        return Err("both checkouts are building".into());
    };
    let attributions: Vec<&CompilerAttribution> = build_classification
        .compile_activities()
        .iter()
        .map(CompileActivity::compiler_attribution)
        .collect();
    assert!(
        attributions.contains(&&CompilerAttribution::UniqueOutputMatch(
            first_session.build_session_id().clone()
        )),
        "the compiler whose source is in the first checkout belongs to it"
    );
    assert!(
        attributions.contains(&&CompilerAttribution::UniqueOutputMatch(
            second_session.build_session_id().clone()
        )),
        "the compiler whose source is in the second checkout belongs to it"
    );
    let [unattributed] = build_classification.unattributed_compile_activities() else {
        return Err("the dependency compiler sits under neither checkout".into());
    };
    assert!(
        matches!(
            unattributed.compiler_attribution(),
            CompilerAttribution::Ambiguous { candidates } if candidates.len() == 2
        ),
        "a compiler under neither checkout stays claimable by both"
    );
    Ok(())
}

#[test]
fn a_rewritten_dependency_manifest_is_read_again_and_a_missing_one_is_looked_up_once()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let dependency_root = fixture.dependency_checkout("serde", "1.0.0")?;
    let compiler = ObservedProcess::new(
        1_111,
        111,
        "rustc serde",
        "/usr/bin/rustc",
        &[
            "rustc",
            "--crate-name",
            "serde",
            &dependency_root.join("src/lib.rs").to_string_lossy(),
        ],
    )
    .with_cwd(&fixture.checkout_root)
    .with_unobservable_parentage()
    .with_candidate_role(ObservedCandidateRole::Compiler);

    fixture.classify(std::slice::from_ref(&compiler));
    assert_eq!(
        dependency_package_version(&fixture.classify(std::slice::from_ref(&compiler)))?,
        "1.0.0"
    );

    // A version whose text length also changes, so the stamp differs whatever
    // the filesystem's modification-time resolution is.
    std::fs::write(
        dependency_root.join("Cargo.toml"),
        "[package]\nname = \"serde\"\nversion = \"1.0.10\"\n",
    )?;

    let stale_cycle = fixture.classify(std::slice::from_ref(&compiler));
    assert_eq!(
        dependency_package_version(&stale_cycle)?,
        "1.0.0",
        "the rewrite is noticed by the revalidation that follows this cycle"
    );
    let invalidated_cycle = fixture.classify(std::slice::from_ref(&compiler));
    assert_eq!(
        invalidated_cycle
            .dependency_manifest_lookup_requests()
            .len(),
        1,
        "the invalidated entry is asked for again"
    );
    assert_eq!(
        dependency_package_version(&fixture.classify(&[compiler]))?,
        "1.0.10",
        "the re-read manifest names the new version"
    );
    Ok(())
}

#[test]
fn a_source_root_with_no_manifest_above_it_is_looked_up_once_and_then_known_absent()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    // Deeper than the manifest search reaches, so the search can never leave
    // the temporary directory and find an unrelated manifest.
    let unmanifested_root = fixture.temp_dir.path().join("a/b/c/d/e/f/g/h");
    std::fs::create_dir_all(&unmanifested_root)?;
    std::fs::write(unmanifested_root.join("lib.rs"), "")?;
    let compiler = ObservedProcess::new(
        1_212,
        121,
        "rustc unmanifested",
        "/usr/bin/rustc",
        &[
            "rustc",
            "--crate-name",
            "mystery",
            &unmanifested_root.join("lib.rs").to_string_lossy(),
        ],
    )
    .with_cwd(&fixture.checkout_root)
    .with_unobservable_parentage()
    .with_candidate_role(ObservedCandidateRole::Compiler);

    let not_yet_looked_up = fixture.classify(std::slice::from_ref(&compiler));
    assert_eq!(
        not_yet_looked_up
            .dependency_manifest_lookup_requests()
            .len(),
        1,
        "a source root nothing is known about is asked for once"
    );

    let looked_up_and_absent = fixture.classify(&[compiler]);
    assert!(
        looked_up_and_absent
            .dependency_manifest_lookup_requests()
            .is_empty(),
        "a source root already found to have no manifest is never re-read"
    );
    assert_eq!(
        fixture
            .build_classifier
            .dependency_manifest_snapshot()
            .len(),
        1,
        "the absent result is cached, which is what distinguishes it from not yet looked up"
    );
    let [activity] = looked_up_and_absent.unattributed_compile_activities() else {
        return Err("the compiler reaches no session".into());
    };
    assert!(matches!(
        activity.compiled_crate_identity(),
        CompiledCrateIdentity::CrateNameOnly(compiled_crate_name)
            if compiled_crate_name.as_str() == "mystery"
    ));
    Ok(())
}

#[test]
fn a_cohort_discovered_in_one_cycle_keeps_its_order_across_later_cycles()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let second_checkout_root = fixture.index_checkout("second")?;
    let third_checkout_root = fixture.index_checkout("third")?;
    let cohort = [
        fixture.cargo_root(&["cargo", "build"]),
        ObservedProcess::new(
            1_313,
            131,
            "cargo build second",
            "/usr/bin/cargo",
            &["cargo", "build"],
        )
        .with_cwd(&second_checkout_root)
        .with_candidate_role(ObservedCandidateRole::Cargo),
        ObservedProcess::new(
            1_414,
            141,
            "cargo build third",
            "/usr/bin/cargo",
            &["cargo", "build"],
        )
        .with_cwd(&third_checkout_root)
        .with_candidate_role(ObservedCandidateRole::Cargo),
    ];

    let first_cycle = fixture.classify(&cohort);
    let first_seen_cycles: Vec<_> = first_cycle
        .build_sessions()
        .iter()
        .map(BuildSession::first_seen)
        .collect();
    assert_eq!(
        first_seen_cycles.len(),
        3,
        "all three roots are discovered in the same cycle"
    );
    assert!(
        first_seen_cycles.windows(2).all(|pair| pair[0] == pair[1]),
        "nothing in the ledger separates a cohort discovered together"
    );
    let order: Vec<BuildSessionId> = first_cycle
        .build_sessions()
        .iter()
        .map(|build_session| build_session.build_session_id().clone())
        .collect();

    for _ in 0..2 {
        let later_cycle = fixture.classify(&cohort);
        let later_order: Vec<BuildSessionId> = later_cycle
            .build_sessions()
            .iter()
            .map(|build_session| build_session.build_session_id().clone())
            .collect();
        assert_eq!(
            later_order, order,
            "a cohort with one first-seen cycle still orders the same way every cycle"
        );
    }
    Ok(())
}

#[test]
fn a_profile_that_never_reaches_argv_keeps_one_label_when_its_compiler_exits()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let release_output_directory = fixture.checkout_root.join("target/release/deps");
    std::fs::create_dir_all(&release_output_directory)?;
    // A `.cargo/config.toml` profile or a Cargo alias never reaches argv, so
    // the only cycles that can see the profile are the ones with a live
    // compiler in them.
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    let compiler = ObservedProcess::new(
        1_515,
        151,
        "rustc release",
        "/usr/bin/rustc",
        &[
            "rustc",
            "--crate-name",
            "demo",
            "src/lib.rs",
            "--out-dir",
            &release_output_directory.to_string_lossy(),
        ],
    )
    .with_cwd(&fixture.checkout_root)
    .with_validated_parent(cargo_root.identity())
    .with_candidate_role(ObservedCandidateRole::Compiler);

    let with_compiler = fixture.classify(&[cargo_root.clone(), compiler]);
    let between_compilers = fixture.classify(&[cargo_root]);

    for (build_classification, cycle) in [
        (&with_compiler, "the cycle with a live compiler"),
        (&between_compilers, "the cycle between compilers"),
    ] {
        let [build_session] = build_classification.build_sessions() else {
            return Err("one build session should be classified".into());
        };
        assert_eq!(
            build_session.build_profile().label(),
            &BuildProfileLabel::Release,
            "{cycle} names the same profile"
        );
        assert_eq!(
            build_session.build_profile().attribution(),
            BuildProfileAttribution::OutputDirectory,
            "{cycle} names the profile the same way"
        );
    }
    Ok(())
}

#[test]
fn every_linker_cargo_runs_is_reported_as_a_linker() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let cargo_root = fixture.cargo_root(&["cargo", "build"]);
    for (index, executable) in ["ld64.lld", "ld.lld", "rust-lld", "lld-link"]
        .into_iter()
        .enumerate()
    {
        let pid = 1_600 + u32::try_from(index)?;
        let linker = ObservedProcess::new(
            pid,
            u64::from(pid),
            executable,
            &format!("/usr/bin/{executable}"),
            &[executable, "demo.demo.rcgu.o", "-o", "demo"],
        )
        .with_cwd(&fixture.checkout_root)
        .with_validated_parent(cargo_root.identity())
        .with_candidate_role(ObservedCandidateRole::Compiler);

        let build_classification = fixture.classify(&[cargo_root.clone(), linker]);

        let [compile_activity] = build_classification.compile_activities() else {
            return Err("the linker is the root's one compile activity".into());
        };
        assert_eq!(
            compile_activity.compiler_kind(),
            CompilerKind::Linker,
            "{executable} is the link step, not a compiler"
        );
    }
    Ok(())
}

#[test]
fn a_process_whose_arguments_are_unreadable_is_observed_without_a_session()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = ClassificationFixture::new()?;
    let blinded_root = fixture
        .cargo_root(&["cargo", "build"])
        .with_unreadable_argv();

    let build_classification = fixture.classify(std::slice::from_ref(&blinded_root));

    assert!(build_classification.build_sessions().is_empty());
    assert!(
        build_classification
            .observed_incarnations()
            .contains(blinded_root.incarnation())
    );
    Ok(())
}

// --- demand-driven classification -----------------------------------------

fn requested_demand(
    compile_monitor_generation: CompileMonitorGeneration,
    cancellation: CompileClassificationCancellation,
) -> CompileClassificationDemand {
    CompileClassificationDemand::Requested {
        compile_monitor_generation,
        build_scope_key: BuildScopeKey::for_test(AbsolutePath::from(std::path::Path::new("/"))),
        cargo_workspace_index: std::sync::Arc::new(CargoWorkspaceIndex::from_metadata_store(
            &WorkspaceMetadataStore::new(),
            ProjectListRevision::default(),
        )),
        owned_root_evidence: OwnedRootEvidence::NoLiveRoot,
        owned_termination_support: OwnedTerminationSupport::Unavailable,
        cancellation,
    }
}

#[test]
fn a_cycle_that_owes_the_monitor_nothing_runs_no_classification() {
    let mut build_classifier = BuildClassifier::default();
    let process_observer = ProcessObserver::default();

    assert!(matches!(
        build_classifier.classify_demand(
            &process_observer,
            &snapshot_of(&[]),
            CompileClassificationDemand::NotRequested,
            Instant::now(),
        ),
        CompileClassificationExecution::NotRequested
    ));
}

/// Cancellation is read after the observation the cycle already paid for, so a
/// cancelled cycle names the generation it belonged to and does no compile
/// parsing or classification at all.
#[test]
fn a_cancelled_demand_skips_classification_and_names_its_generation() {
    let mut build_classifier = BuildClassifier::default();
    let process_observer = ProcessObserver::default();
    let mut compile_monitor_generation = CompileMonitorGeneration::default();
    compile_monitor_generation.advance();
    let cancellation =
        CompileClassificationCancellation::for_generation(compile_monitor_generation);
    let _ = cancellation.cancel(compile_monitor_generation);

    assert!(matches!(
        build_classifier.classify_demand(
            &process_observer,
            &snapshot_of(&[]),
            requested_demand(compile_monitor_generation, cancellation),
            Instant::now(),
        ),
        CompileClassificationExecution::Cancelled(returned_generation)
            if returned_generation == compile_monitor_generation
    ));
}
