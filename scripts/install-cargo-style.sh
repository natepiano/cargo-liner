#!/usr/bin/env bash

# Install cargo-style with a staged build and a recoverable binary backup.

set -u

repository_path=${1-}
if [[ -z $repository_path || ! -f $repository_path/Cargo.toml ]]; then
    printf 'cargo-style install failed: pass the cargo-liner repository path\n' >&2
    exit 1
fi

binary_directory=${CARGO_HOME:-$HOME/.cargo}/bin
binary_path=$binary_directory/cargo-style

case :$PATH: in
    *:$binary_directory:*) ;;
    *)
        printf 'cargo-style install failed: %s is not on PATH\n' "$binary_directory" >&2
        exit 1
        ;;
esac
mkdir -p -- "$binary_directory" || {
    printf 'cargo-style install failed: cannot create %s\n' "$binary_directory" >&2
    exit 1
}

staging_directory=$(mktemp -d "${TMPDIR:-/tmp}/cargo-style-install.XXXXXX") || {
    printf 'cargo-style install failed: cannot create the staging folder\n' >&2
    exit 1
}

previous_binary=Absent
publication=NotStarted
rollback_installation() {
    if [[ $publication == Started ]]; then
        if [[ $previous_binary == BackedUp ]]; then
            if ! cp -p -- "$staging_directory/previous-cargo-style" "$binary_path"; then
                printf 'cargo-style rollback failed: cannot restore the previous binary\n' >&2
                printf 'Staging folder: %s\nBackup file: %s\n' \
                    "$staging_directory" "$staging_directory/previous-cargo-style" >&2
                return
            fi
        elif ! rm -f -- "$binary_path"; then
            printf 'cargo-style rollback failed: cannot remove the incomplete binary\n' >&2
        fi
    fi
    rm -rf -- "$staging_directory"
}
trap rollback_installation EXIT
trap 'exit 1' HUP INT TERM

if [[ -f $binary_path ]]; then
    cp -p -- "$binary_path" "$staging_directory/previous-cargo-style" || {
        printf 'cargo-style install failed: cannot back up the previous binary\n' >&2
        exit 1
    }
    previous_binary=BackedUp
fi

# The rustup shim consumes +stable before Cargo reads its arguments.
# A direct Cargo installation has no shim and rejects +stable as an unknown command.
# Select +stable only when rustup is available.
# Use conditional expansion for Bash 3.2, which rejects empty arrays with set -u.
if command -v rustup >/dev/null 2>&1; then
    toolchain_argument=+stable
else
    toolchain_argument=
fi

if ! CARGO_TARGET_DIR=$staging_directory/target cargo ${toolchain_argument:+"$toolchain_argument"} build \
    --manifest-path "$repository_path/Cargo.toml" \
    -p cargo-style \
    --release \
    --locked; then
    printf 'cargo-style install failed during the build\n' >&2
    exit 1
fi

# BSD install on macOS uses -S for a safe copy without an argument.
# GNU install uses -S for a suffix and consumes the next argument.
# Select -S only on macOS.
if [[ $(uname -s) == Darwin ]]; then
    safe_copy_argument=-S
else
    safe_copy_argument=
fi

publication=Started
if ! install ${safe_copy_argument:+"$safe_copy_argument"} -m 755 \
    "$staging_directory/target/release/cargo-style" "$binary_path"; then
    printf 'cargo-style install failed during binary publication\n' >&2
    exit 1
fi

publication=Complete
printf 'Installed cargo-style from %s\n' "$repository_path"
