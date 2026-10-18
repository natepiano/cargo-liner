#![allow(
    clippy::expect_used,
    reason = "fixture setup should stop on an unexpected command or filesystem failure"
)]

//! nextest setup entry point; this target deliberately exposes no tests.

use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::Path;

#[path = "support/git_configuration.rs"]
mod git_configuration;
#[path = "support/loose_refs.rs"]
mod loose_refs;

#[path = "support/edges_templates.rs"]
mod edges_templates;
pub use edges_templates::CROSS_TARGET_AFTER_PAIR;
pub use edges_templates::CROSS_TARGET_PAIR;
pub use edges_templates::LEFT_RIGHT_TREES;
pub use edges_templates::LEGACY_NEGATIVE_PROTECTED_GAP;
pub use edges_templates::LEGACY_NEGATIVE_UNRELATED_GAP;
pub use edges_templates::RECORDED_PROJECTION;
pub use edges_templates::RewrittenSuccessorState;
pub use edges_templates::SuccessorGap;
pub use edges_templates::append_edge_fixture_event;
pub use edges_templates::build_rewritten_successor;
pub use edges_templates::commit_successor_fixture_file;
pub use edges_templates::legacy_negative_state;
pub use edges_templates::legacy_negative_template;
pub use edges_templates::retain_release;
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
    let templates = [
        edges_templates::RECORDED_PROJECTION,
        edges_templates::CROSS_TARGET_PAIR,
        edges_templates::CROSS_TARGET_AFTER_PAIR,
        edges_templates::LEGACY_NEGATIVE_UNRELATED_GAP,
        edges_templates::LEGACY_NEGATIVE_PROTECTED_GAP,
        edges_templates::LEFT_RIGHT_TREES,
        gate_templates::ENROLLED_PAIR,
        gate_templates::UNENROLLED_PAIR,
        gate_templates::REVALIDATED_CROSS_TARGET_LANDING,
        reanchoring_templates::TWO_REBASE_HISTORY,
        reanchoring_templates::DRAINED_SPLIT_RETENTION_CHECKPOINT,
        reanchoring_templates::DRAINED_SPLIT_SIBLING_CHECKPOINT,
        reanchoring_templates::STACKED_UPPER_CHECKPOINT,
        reanchoring_templates::SPLIT_LOWER_CHECKPOINT_ACTIVE_RETENTION,
        reanchoring_templates::SPLIT_LOWER_CHECKPOINT_UPDATED_SIBLING,
    ];
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cargo-berth-templates");
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
        return;
    }
    let builds = templates[0].repository.before_run_directory();
    std::thread::scope(|scope| {
        for template in templates {
            let builds = &builds;
            scope.spawn(move || template.prepare_before_run(builds));
        }
    });
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
