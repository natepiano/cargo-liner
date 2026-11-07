//! Keep the imported `reader_scenario_cases.py` and `fixture_events.py` bytecode outside the source
//! tree.

use std::process::Command;

/// Give `reader_scenario.py` and `installed.rs` a persistent Python bytecode directory.
pub(super) fn cache_python_modules(command: &mut Command) {
    command.env(
        "PYTHONPYCACHEPREFIX",
        dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("cargo-tile/test-python-cache"),
    );
}
