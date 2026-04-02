#!/usr/bin/env bash
set -euo pipefail

repo_root="${BEVY_ROOT:-$HOME/rust/bevy}"
entry="${ENTRY_NAME:-settings.local.json}"
row_path="${ROW_PATH:-~/rust/bevy}"
perf_log="${PERF_LOG_PATH:-${TMPDIR:-/tmp}/cargo-port-tui-perf.log}"
timeout_secs="${TIMEOUT_SECS:-30}"
action="${1:-toggle}"

now_ms() {
  perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1000'
}

usage() {
  cat <<'EOF'
Usage: scripts/time_bevy_exclude_toggle.sh [add|remove|toggle]

Environment overrides:
  BEVY_ROOT       Repo root to mutate (default: ~/rust/bevy)
  ENTRY_NAME      Exclude entry to toggle (default: settings.local.json)
  ROW_PATH        Row path to watch in the perf log (default: ~/rust/bevy)
  PERF_LOG_PATH   Perf log path (default: /tmp/cargo-port-tui-perf.log)
  TIMEOUT_SECS    Wait timeout (default: 30)
EOF
}

if [[ "$action" == "--help" || "$action" == "-h" ]]; then
  usage
  exit 0
fi

case "$action" in
  add|remove|toggle) ;;
  *)
    usage >&2
    exit 2
    ;;
esac

git_dir="$(git -C "$repo_root" rev-parse --absolute-git-dir)"
exclude_file="$git_dir/info/exclude"
mkdir -p "$(dirname "$exclude_file")"
touch "$exclude_file"

has_entry() {
  grep -qxF "$entry" "$exclude_file"
}

if [[ "$action" == "toggle" ]]; then
  if has_entry; then
    action="remove"
  else
    action="add"
  fi
fi

expected_state="clean"
if [[ "$action" == "remove" ]]; then
  expected_state="untracked"
fi

start_ms="$(now_ms)"
tmp_file="$(mktemp)"
cleanup() {
  rm -f "$tmp_file"
}
trap cleanup EXIT

if [[ "$action" == "add" ]]; then
  cp "$exclude_file" "$tmp_file"
  if ! has_entry; then
    printf '%s\n' "$entry" >> "$tmp_file"
  fi
else
  grep -vxF "$entry" "$exclude_file" > "$tmp_file" || true
fi
mv "$tmp_file" "$exclude_file"

needle="app_git_path_state_applied path=$row_path state=$expected_state"
deadline=$((start_ms + timeout_secs * 1000))

echo "exclude_file=$exclude_file"
echo "action=$action expected_state=$expected_state start_ms=$start_ms"
echo "waiting for: $needle"

matched_line=""
while (( "$(now_ms)" < deadline )); do
  if [[ -f "$perf_log" ]]; then
    matched_line="$(
      python3 - "$perf_log" "$start_ms" "$needle" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
start = int(sys.argv[2])
needle = sys.argv[3]
matched = ""
for line in path.read_text().splitlines():
    parts = line.split(" ", 1)
    if len(parts) != 2:
        continue
    try:
        ts = int(parts[0])
    except ValueError:
        continue
    if ts >= start and needle in line:
        matched = line
if matched:
    print(matched)
PY
    )"
    if [[ -n "$matched_line" ]]; then
      break
    fi
  fi
  sleep 0.1
done

if [[ -z "$matched_line" ]]; then
  echo "timed out waiting for row update after ${timeout_secs}s" >&2
  if [[ -f "$perf_log" ]]; then
    echo
    echo "recent perf events:"
    python3 - "$perf_log" "$start_ms" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
start = int(sys.argv[2])
needles = ("watcher_", "git_path_states_", "app_git_path_state_applied", "poll_background", "tokio_")
for line in path.read_text().splitlines():
    parts = line.split(" ", 1)
    if len(parts) != 2:
        continue
    try:
        ts = int(parts[0])
    except ValueError:
        continue
    if ts >= start and any(needle in line for needle in needles):
        print(line)
PY
  fi
  exit 1
fi

end_ms="$(awk '{ print $1 }' <<<"$matched_line")"
elapsed_ms=$((end_ms - start_ms))

echo "matched_ms=$end_ms elapsed_ms=$elapsed_ms"
echo "$matched_line"
echo
echo "perf events since toggle:"
python3 - "$perf_log" "$start_ms" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
start = int(sys.argv[2])
needles = ("watcher_", "git_path_states_", "app_git_path_state_applied", "poll_background", "tokio_")
for line in path.read_text().splitlines():
    parts = line.split(" ", 1)
    if len(parts) != 2:
        continue
    try:
        ts = int(parts[0])
    except ValueError:
        continue
    if ts >= start and any(needle in line for needle in needles):
        print(line)
PY
