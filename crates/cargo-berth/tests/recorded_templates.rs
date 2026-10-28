#![allow(
    clippy::expect_used,
    reason = "fixture setup should stop on an unexpected command or filesystem failure"
)]

//! nextest setup entry point and the cold single-target setup regression.

use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;

#[path = "support/git_configuration.rs"]
mod git_configuration;
#[path = "support/loose_refs.rs"]
mod loose_refs;

#[path = "support/edges_templates.rs"]
mod edges_templates;
#[path = "support/linked_checkout.rs"]
mod linked_checkout;
#[path = "support/successor_templates.rs"]
mod successor_templates;
pub use edges_templates::COVER_LANDING;
pub use edges_templates::CROSS_TARGET_AFTER_PAIR;
pub use edges_templates::CROSS_TARGET_PAIR;
pub use edges_templates::DIFFERENT_SUCCESSOR;
pub use edges_templates::EQUIVALENT_SUCCESSOR;
pub use edges_templates::HISTORICAL_PROOF;
pub use edges_templates::INITIAL_INTEGRATION_PAIR;
pub use edges_templates::LEFT_RIGHT_TREES;
pub use edges_templates::LEGACY_NEGATIVE_PROTECTED_GAP;
pub use edges_templates::LEGACY_NEGATIVE_UNRELATED_GAP;
pub use edges_templates::MERGE_EXTENT_MAIN;
pub use edges_templates::MERGE_EXTENT_PAIR;
pub use edges_templates::ONE_PREDECESSOR;
pub use edges_templates::RECORDED_PROJECTION;
pub use edges_templates::RewrittenSuccessorState;
pub use edges_templates::SuccessorGap;
pub use edges_templates::TWO_PREDECESSORS;
pub use edges_templates::append_edge_fixture_event;
pub use edges_templates::build_rewritten_successor;
pub use edges_templates::commit_successor_fixture_file;
pub use edges_templates::legacy_negative_state;
pub use edges_templates::legacy_negative_template;
pub use edges_templates::retain_release;
pub use edges_templates::successor_history_state;
#[path = "support/covered_branch_template.rs"]
mod covered_branch_template;
#[path = "support/ordered_target_pair_template.rs"]
mod ordered_target_pair_template;
#[path = "support/pending_rewrite_templates.rs"]
mod pending_rewrite_templates;
pub use pending_rewrite_templates::PENDING_ORDERED_REWRITE;
pub use pending_rewrite_templates::TWO_PENDING_ORDERED_REWRITES;
#[path = "support/gate_templates.rs"]
mod gate_templates;
pub use gate_templates::CrossTargetLandingWitnesses;
pub use gate_templates::ENROLLED_PAIR;
pub use gate_templates::EnrolledPair;
pub use gate_templates::REVALIDATED_CROSS_TARGET_LANDING;
pub use gate_templates::UNENROLLED_PAIR;
pub use gate_templates::build_scratch_repository;
pub use gate_templates::enroll_pair;
pub use gate_templates::read_cross_target_landing;
pub use gate_templates::read_enrolled_pair;
pub use gate_templates::set_gate_mode;
#[path = "support/reanchoring_templates.rs"]
mod reanchoring_templates;
pub use reanchoring_templates::SplitRefChange;
#[path = "support/split_rebase.rs"]
mod split_rebase;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: cargo_berth_test_support::GitDriver = cargo_berth_test_support::GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      cargo_berth_test_support::OptionalLocks::Taken,
    cleared_environment: &[],
};
use cargo_berth_test_support::git_command;

fn main() {
    if env::args().any(|argument| argument == "--probe-stalled-template") {
        let (_sender, receiver) = mpsc::channel();
        require_template_progress(&receiver, &["stalled-fixture"], Duration::from_millis(25));
        return;
    }
    let templates = recorded_populations();
    let directory = template_cache_directory();
    if env::args().any(|argument| argument == "--list") {
        fs::create_dir_all(&directory).expect("builder directory should exist");
        let link = directory.join("builder");
        let replacement = directory.join(format!("builder-{}", std::process::id()));
        symlink(
            env::current_exe().expect("builder path should resolve"),
            &replacement,
        )
        .expect("replacement builder link should write");
        fs::rename(replacement, link).expect("builder link should replace atomically");
        if let Some(run_id) = env::var_os("NEXTEST_RUN_ID") {
            fs::write(directory.join("builder-run-id"), run_id.as_encoded_bytes())
                .expect("the run that listed this builder should record");
        }
        if !env::args().any(|argument| argument == "--ignored") {
            println!("cold_single_target_setup_builds_every_recorded_template: test");
            println!("stalled_template_build_fails_with_a_clear_deadline: test");
        }
        return;
    }
    if env::args().any(|argument| argument == "stalled_template_build_fails_with_a_clear_deadline")
    {
        assert_stalled_template_deadline();
        return;
    }
    if env::args()
        .any(|argument| argument == "cold_single_target_setup_builds_every_recorded_template")
    {
        assert_single_target_setup(&templates);
        return;
    }
    let builds = directory.join("before-run").join(
        templates[0]
            .repository
            .before_run_directory()
            .file_name()
            .expect("template build should have a key"),
    );
    std::thread::scope(|scope| {
        let (completed, progress) = mpsc::channel();
        for template in templates {
            let builds = &builds;
            let completed = completed.clone();
            scope.spawn(move || {
                template.prepare_before_run(builds);
                completed
                    .send(template.repository.name)
                    .expect("setup progress should publish");
            });
        }
        drop(completed);
        // Cold SETUP took 39.527 s under tracing. Allow over three times that
        // for one completion, so machine load does not look like a stalled child.
        let names = templates.map(|template| template.repository.name);
        require_template_progress(&progress, &names, Duration::from_secs(120));
    });
    if env::var_os("CARGO_BERTH_SINGLE_TARGET_SETUP_PROBE").is_none() {
        prepare_single_target_setup_probe(&directory);
    }
    if let Some(environment) = env::var_os("NEXTEST_ENV") {
        writeln!(
            OpenOptions::new()
                .append(true)
                .open(environment)
                .expect("nextest environment should open"),
            "CARGO_BERTH_PREBUILT_TEMPLATES={}",
            builds.display()
        )
        .expect("template directory should publish");
    }
}

const fn recorded_populations() -> [LedgerTemplate; 34] {
    [
        edges_templates::RECORDED_PROJECTION,
        edges_templates::MERGE_EXTENT_MAIN,
        edges_templates::MERGE_EXTENT_PAIR,
        edges_templates::INITIAL_INTEGRATION_PAIR,
        edges_templates::EQUIVALENT_SUCCESSOR,
        edges_templates::DIFFERENT_SUCCESSOR,
        edges_templates::ONE_PREDECESSOR,
        edges_templates::TWO_PREDECESSORS,
        successor_templates::ONE_SUCCESSOR,
        successor_templates::TWO_SUCCESSORS,
        successor_templates::FOUR_SUCCESSORS,
        edges_templates::HISTORICAL_PROOF,
        edges_templates::COVER_LANDING,
        edges_templates::CROSS_TARGET_PAIR,
        edges_templates::CROSS_TARGET_AFTER_PAIR,
        edges_templates::LEGACY_NEGATIVE_UNRELATED_GAP,
        edges_templates::LEGACY_NEGATIVE_PROTECTED_GAP,
        edges_templates::LEFT_RIGHT_TREES,
        covered_branch_template::COVERED_BRANCH_HOOK,
        ordered_target_pair_template::SEPARATE_TARGET_PAIR,
        ordered_target_pair_template::MERGED_TARGET_PAIR,
        PENDING_ORDERED_REWRITE,
        TWO_PENDING_ORDERED_REWRITES,
        gate_templates::ENROLLED_PAIR,
        gate_templates::UNENROLLED_PAIR,
        gate_templates::REVALIDATED_CROSS_TARGET_LANDING,
        reanchoring_templates::TWO_REBASE_HISTORY,
        reanchoring_templates::SPLIT_ACTIVE_PHASE,
        reanchoring_templates::SPLIT_OUTSTANDING_PHASE,
        reanchoring_templates::DRAINED_SPLIT_RETENTION_CHECKPOINT,
        reanchoring_templates::DRAINED_SPLIT_SIBLING_CHECKPOINT,
        reanchoring_templates::STACKED_UPPER_CHECKPOINT,
        reanchoring_templates::SPLIT_LOWER_CHECKPOINT_ACTIVE_RETENTION,
        reanchoring_templates::SPLIT_LOWER_CHECKPOINT_UPDATED_SIBLING,
    ]
}

/// Bound stalled setup rather than leave nextest waiting on a stuck fixture child.
fn require_template_progress(progress: &mpsc::Receiver<&str>, names: &[&str], deadline: Duration) {
    let mut unfinished = names.to_vec();
    while !unfinished.is_empty() {
        match progress.recv_timeout(deadline) {
            Ok(name) => {
                unfinished.retain(|pending| *pending != name);
                eprintln!("cargo-berth template complete: {name}");
            },
            Err(error) => {
                eprintln!(
                    "cargo-berth template SETUP failed: no template completed within {deadline:?}; unfinished templates: {}: {error}",
                    unfinished.join(", ")
                );
                std::process::exit(1);
            },
        }
    }
}

fn assert_stalled_template_deadline() {
    let output = Command::new(env::current_exe().expect("builder should resolve"))
        .arg("--probe-stalled-template")
        .output()
        .expect("stalled setup probe should run");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("template SETUP failed: no template completed within 25ms; unfinished templates: stalled-fixture")
    );
}

fn template_cache_directory() -> PathBuf {
    env::var_os("CARGO_BERTH_TEMPLATE_CACHE").map_or_else(
        || Path::new(env!("CARGO_TARGET_TMPDIR")).join("cargo-berth-templates"),
        PathBuf::from,
    )
}

/// Exercise the actual wrapper with no listing or cache, under nextest's setup
/// clock, reusing this executable. The test checks the real template artifacts.
fn prepare_single_target_setup_probe(directory: &Path) {
    let probe = directory.join("single-target-setup");
    if probe.exists() {
        fs::remove_dir_all(&probe).expect("previous setup probe should remove");
    }
    fs::create_dir(&probe).expect("empty setup probe should exist");
    assert!(
        fs::read_dir(&probe)
            .expect("empty setup probe should read")
            .next()
            .is_none()
    );
    let environment = directory.join("single-target-environment");
    fs::write(&environment, "").expect("probe environment should start empty");
    let output = Command::new("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/setup-recorded-templates.sh"))
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .env("CARGO_BERTH_TEMPLATE_CACHE", &probe)
        .env(
            "CARGO_BERTH_TEMPLATE_BUILDER",
            env::current_exe().expect("running builder should resolve"),
        )
        .env("CARGO_BERTH_SINGLE_TARGET_SETUP_PROBE", "1")
        .env("NEXTEST_ENV", &environment)
        .output()
        .expect("cold single-target setup should run");
    assert!(
        output.status.success(),
        "cold single-target setup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .any(|line| line == "cargo-berth template builder source: explicit builder"),
        "cold setup should reuse the running builder: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let publication = fs::read_to_string(environment).expect("probe publication should read");
    let builds = publication
        .trim()
        .strip_prefix("CARGO_BERTH_PREBUILT_TEMPLATES=")
        .expect("cold single-target setup should publish its builds");
    fs::write(directory.join("single-target-builds"), builds)
        .expect("actual cold setup artifacts should be recorded");
}

fn assert_single_target_setup(templates: &[LedgerTemplate]) {
    let directory = template_cache_directory();
    let probe = directory.join("single-target-setup");
    assert!(
        !probe.join("builder-run-id").exists(),
        "the regression must take the path without a listed builder"
    );
    let publication = fs::read_to_string(directory.join("single-target-builds"))
        .expect("cold setup should have published its actual builds");
    let builds = Path::new(publication.trim());
    assert!(builds.starts_with(&probe));
    for template in templates {
        let root = builds.join(template.repository.name);
        assert!(
            root.with_extension("complete").is_file(),
            "single-target setup did not complete {}",
            template.repository.name
        );
        assert!(
            root.join(".git/HEAD").is_file(),
            "single-target setup did not build repository {}",
            template.repository.name
        );
        let journal = root.join(".git/cargo-berth/journal.ndjson");
        match template.ledger {
            LedgerFixture::Recorded => assert!(
                journal.is_file(),
                "single-target setup did not build recorded ledger {}",
                template.repository.name
            ),
            LedgerFixture::Empty => {
                let records = match fs::read(&journal) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    records => records.expect("single-target setup journal should be readable"),
                };
                assert!(
                    records.is_empty(),
                    "single-target setup added journal records to empty fixture {}",
                    template.repository.name
                );
            },
        }
    }
}
