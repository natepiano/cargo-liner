//! Record setup histories with their linked checkouts and typed witnesses.

use std::fs;
use std::path::Path;

use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;

const EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const STATE: &str = ".git/tested-operation-state.json";

/// Name a history that nextest prepares before the tested operation.
pub(crate) const fn template(name: &'static str, build: fn(&Path)) -> LedgerTemplate {
    LedgerTemplate {
        repository:   RepositoryTemplate {
            target_directory: env!("CARGO_TARGET_TMPDIR"),
            executable: EXECUTABLE,
            name,
            build,
        },
        ledger:       LedgerFixture::Recorded,
        build_timing: TemplateBuildTiming::BeforeRun,
    }
}

/// Save the repository, its linked checkouts, and the operation's witnesses.
pub(crate) fn save(root: &Path, source: &Path, state: &impl serde::Serialize) {
    let relocations = cargo_berth_test_support::copy_recorded_repository(EXECUTABLE, root, source);
    relocate_tree(root, &relocations);
    cargo_berth_test_support::write_recorded_state(
        root,
        STATE,
        serde_json::to_value(state).expect("history state should encode"),
        &relocations,
    );
}

pub(crate) use cargo_berth_test_support::relocate_tree;

/// Read the witnesses after template relocation renews their paths.
pub(crate) fn read<State: serde::de::DeserializeOwned>(root: &Path) -> State {
    serde_json::from_slice(&fs::read(root.join(STATE)).expect("history state should read"))
        .expect("history state should decode")
}
