//! Keep the imported `reader_scenario_cases.py` and `fixture_events.py` bytecode outside the source
//! tree.

use std::process::Command;

/// An inherited request that stops Python from writing bytecode. With it set, each
/// scenario process compiles `reader_scenario_cases.py` again, 45-180 ms under load.
const DONT_WRITE_BYTECODE_ENVIRONMENT: &str = "PYTHONDONTWRITEBYTECODE";

/// Give `reader_scenario.py` and `installed.rs` a persistent Python bytecode directory,
/// and let Python write to it.
pub(super) fn cache_python_modules(command: &mut Command) {
    command
        .env(
            "PYTHONPYCACHEPREFIX",
            dirs::cache_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("cargo-tile/test-python-cache"),
        )
        .env_remove(DONT_WRITE_BYTECODE_ENVIRONMENT);
}
