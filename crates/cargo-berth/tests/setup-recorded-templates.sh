#!/usr/bin/env bash
set -euo pipefail

# A target-selected run does not list recorded_templates. Let its tests build
# their fixtures, even when an earlier full run left a builder symlink behind.
template_directory="${CARGO_TARGET_DIR:-target}/tmp/cargo-berth-templates"
builder="$template_directory/builder"
listed_run="$template_directory/builder-run-id"
if [[ -n "${NEXTEST_RUN_ID:-}" && -x "$builder" && -f "$listed_run" &&
      "$(cat "$listed_run")" == "$NEXTEST_RUN_ID" ]]; then
    exec "$builder"
fi
