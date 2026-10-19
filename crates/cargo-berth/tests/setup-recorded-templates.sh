#!/usr/bin/env bash
set -euo pipefail

# Full runs publish this link while listing recorded_templates. A selected
# target may never list that binary, so build and locate it when necessary.
template_directory="${CARGO_BERTH_TEMPLATE_CACHE:-${CARGO_TARGET_DIR:-target}/tmp/cargo-berth-templates}"
listed_builder="$template_directory/builder"
listed_run="$template_directory/builder-run-id"
if [[ -n "${CARGO_BERTH_TEMPLATE_BUILDER:-}" ]]; then
    builder="$CARGO_BERTH_TEMPLATE_BUILDER"
    source='explicit builder'
elif [[ -n "${NEXTEST_RUN_ID:-}" && -x "$listed_builder" && -f "$listed_run" &&
        "$(< "$listed_run")" == "$NEXTEST_RUN_ID" ]]; then
    builder="$listed_builder"
    source='listed link'
else
    source='cargo fallback'
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
echo "cargo-berth template builder source: $source" >&2
exec "$builder"
