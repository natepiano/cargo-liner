#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

ABORT_PATTERN='std[.]process::abort[[:space:]]*[(]|process::abort[[:space:]]*[(]|(^|[^.:[:alnum:]_])abort[[:space:]]*[(]'

inventory_file="$(mktemp -t cargo-port-abort-inventory.XXXXXX)"
actual_counts_file="$(mktemp -t cargo-port-abort-counts.XXXXXX)"
expected_counts_file="$(mktemp -t cargo-port-abort-expected.XXXXXX)"
failures_file="$(mktemp -t cargo-port-abort-failures.XXXXXX)"
trap 'rm -f "${inventory_file}" "${actual_counts_file}" "${expected_counts_file}" "${failures_file}"' EXIT

: >"${expected_counts_file}"

while IFS= read -r source_file; do
    while IFS= read -r match; do
        [[ -n "${match}" ]] || continue
        printf '%s:%s\n' "${source_file}" "${match}" >>"${inventory_file}"
    done < <(grep -nE "${ABORT_PATTERN}" "${source_file}" || true)
done < <(
    find . \
        \( -path './.git' -o -path './target' \) -prune \
        -o -type f -name '*.rs' -print \
        | sed 's#^\./##' \
        | LC_ALL=C sort
)

if [[ -s "${inventory_file}" ]]; then
    awk -F ':' '{ counts[$1] += 1 } END { for (path in counts) print path, counts[path] }' \
        "${inventory_file}" \
        | LC_ALL=C sort >"${actual_counts_file}"
else
    : >"${actual_counts_file}"
fi

count_for_path() {
    local path="$1"
    awk -v path="${path}" '$1 == path { print $2 }' "${actual_counts_file}"
}

expected_for_path() {
    local path="$1"
    awk -v path="${path}" '$1 == path { print $2 }' "${expected_counts_file}"
}

record_failure() {
    printf '%s\n' "$1" >>"${failures_file}"
}

for helper_path in src/test_support.rs src/tui/test_support.rs; do
    helper_count="$(count_for_path "${helper_path}")"
    helper_count="${helper_count:-0}"
    if [[ "${helper_count}" -ne 0 ]]; then
        record_failure "${helper_path}: expected 0 abort call lines, found ${helper_count}"
    fi
done

while read -r expected_path expected_count; do
    actual_count="$(count_for_path "${expected_path}")"
    actual_count="${actual_count:-0}"
    if [[ "${actual_count}" -ne "${expected_count}" ]]; then
        record_failure "${expected_path}: expected ${expected_count} abort call lines, found ${actual_count}"
    fi
done <"${expected_counts_file}"

while read -r actual_path actual_count; do
    expected_count="$(expected_for_path "${actual_path}")"
    if [[ -z "${expected_count}" ]]; then
        record_failure "${actual_path}: unexpected abort call lines found (${actual_count})"
    fi
done <"${actual_counts_file}"

if [[ -s "${failures_file}" ]]; then
    echo "Abort inventory check failed."
    echo
    echo "Failures:"
    sed 's/^/  /' "${failures_file}"
    echo
    echo "Current inventory:"
    if [[ -s "${inventory_file}" ]]; then
        sed 's/^/  /' "${inventory_file}"
    else
        echo "  <none>"
    fi
    exit 1
fi

total_matches="$(wc -l <"${inventory_file}" | tr -d '[:space:]')"
allowed_files="$(wc -l <"${expected_counts_file}" | tr -d '[:space:]')"
printf 'Abort inventory OK: %s matched abort call lines across %s allowlisted files.\n' \
    "${total_matches}" \
    "${allowed_files}"
