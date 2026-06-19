#!/usr/bin/env bash
set -euo pipefail

readonly DEFAULT_REPEAT_COUNT=10

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

repeat_count="${PHASE6_REPEAT_COUNT:-${DEFAULT_REPEAT_COUNT}}"
if [[ ! "${repeat_count}" =~ ^[1-9][0-9]*$ ]]; then
    echo "PHASE6_REPEAT_COUNT must be a positive integer; got '${repeat_count}'." >&2
    exit 2
fi

filters=(
    "tui::app::tests::framework_keymap::output_cancel_bindings_clear_output_and_handle_focus"
    "tui::app::tests::background::quiet_scan_result_does_not_start_startup_workers_or_wait_for_lint_history"
    "tui::app::tests::background::quiet_rescan_uses_noop_scan_without_real_startup_effects"
    "tui::app::tests::background::quiet_completed_scan_applies_noop_rescan_when_enabling_non_rust_without_cached_projects"
    "tui::app::tests::framework_keymap::helper_returned_keymap_fixture_reloads_from_app_path"
    "tui::test_support::tests::test_http_client_skips_host_github_auth"
    "tui::test_support::tests::make_app_uses_quiet_startup_by_default"
    "tui::test_support::tests::quiet_startup_persists_through_reload_and_reset_paths"
    "tui::test_support::tests::lint_runtime_opt_in_enables_only_lint_runtime_startup"
    "tui::test_support::tests::local_startup_fixture_owns_theme_dir_and_suppresses_unowned_effects"
    "tui::background::tests::disabled_watcher_handle_ignores_registration_messages"
)

for filter in "${filters[@]}"; do
    matches="$(
        cargo nextest list \
            --workspace \
            --all-features \
            --tests \
            --message-format oneline \
            -- "${filter}" \
            --exact
    )"
    if [[ -z "${matches}" ]]; then
        echo "No nextest test matched exact filter: ${filter}" >&2
        exit 1
    fi
    printf 'Matched exact nextest filter: %s\n' "${filter}"
done

for run_index in $(seq 1 "${repeat_count}"); do
    printf 'Phase 6 focused nextest run %s/%s\n' "${run_index}" "${repeat_count}"
    # These tests exercise startup fixture overrides and background-handle
    # suppression. Run the focused regression gate serially so nextest reports
    # real fixture leaks instead of parallel process contention between the
    # same startup paths.
    cargo nextest run \
        --workspace \
        --all-features \
        --tests \
        --fail-fast \
        --test-threads 1 \
        -- "${filters[@]}" \
        --exact
done
