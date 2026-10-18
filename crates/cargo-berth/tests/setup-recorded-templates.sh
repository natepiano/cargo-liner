#!/usr/bin/env bash
set -euo pipefail

# Full runs publish this link while listing recorded_templates. A selected
# target may never list that binary, so build and locate it when necessary.
template_directory="${CARGO_BERTH_TEMPLATE_CACHE:-${CARGO_TARGET_DIR:-target}/tmp/cargo-berth-templates}"
builder="$template_directory/builder"
listed_run="$template_directory/builder-run-id"
if [[ -z "${NEXTEST_RUN_ID:-}" || ! -x "$builder" || ! -f "$listed_run" ||
      "$(cat "$listed_run")" != "$NEXTEST_RUN_ID" ]]; then
    builder=$(cargo build -p cargo-berth --test recorded_templates --message-format=json |
        python3 -c 'import json, sys
for line in sys.stdin:
    value = json.loads(line)
    if value.get("reason") == "compiler-artifact" and value.get("target", {}).get("name") == "recorded_templates" and value.get("executable"):
        print(value["executable"])')
    if [[ -z "$builder" || ! -x "$builder" ]]; then
        echo 'cargo-berth template builder was not produced' >&2
        exit 1
    fi
fi
exec "$builder"
