//! Idempotent installation of every git hook managed by this crate.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;

use super::REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT;
use super::permit::PENDING_BYPASS_FILE_PREFIX;
use super::permit::PENDING_BYPASS_FILE_SUFFIX;
use crate::git;
use crate::git::GitError;

const EXECUTABLE_PERMISSIONS: u32 = 0o755;
const POST_COMMIT_HOOK_NAME: &str = "post-commit";
const POST_COMMIT_MARKER: &str = "# cargo-berth managed hook: post-commit";
const REFERENCE_TRANSACTION_HOOK_NAME: &str = "reference-transaction";
const REFERENCE_TRANSACTION_MARKER: &str = "# cargo-berth managed hook: reference-transaction";
/// Names a specific `cargo-berth` for a managed hook, ahead of the installed one.
///
/// The generated shell carries this name as its own literal; tests reach for it
/// through this constant, which `executable_resolution_reads_the_override_it_documents`
/// holds to the same spelling.
#[cfg(test)]
pub(super) const EXECUTABLE_ENVIRONMENT: &str = "CARGO_BERTH_EXECUTABLE";
/// Shell that names the `cargo-berth` a managed hook runs.
///
/// A hook outlives the build that installed it and is read by every checkout
/// sharing the repository, so the path is resolved when the hook runs rather
/// than written into the script. Baking one in pins the gate to a single
/// machine, and to a build directory that `cargo clean` or a removed worktree
/// silently empties; every managed hook fails open, so the gate would then be
/// absent rather than noisy. `CARGO_BERTH_EXECUTABLE` names a specific binary
/// for anyone exercising a build that is not the installed one.
const EXECUTABLE_RESOLUTION: &str = r#"cargo_berth_executable="${CARGO_BERTH_EXECUTABLE:-}"
if [ -z "$cargo_berth_executable" ]; then
    cargo_berth_executable=$(command -v cargo-berth 2>/dev/null)
fi
if [ -z "$cargo_berth_executable" ]; then
    cargo_berth_executable="${CARGO_HOME:-$HOME/.cargo}/bin/cargo-berth"
fi"#;
/// One hook name paired with the complete script body owned by `cargo-berth`.
#[derive(Clone, Copy)]
struct ManagedHook {
    name:     &'static str,
    marker:   &'static str,
    dispatch: ManagedHookDispatch,
}

#[derive(Clone, Copy)]
enum ManagedHookDispatch {
    PostCommit,
    ReferenceTransaction,
}

/// The reference-transaction hook definition registered below.
const REFERENCE_TRANSACTION_HOOK: ManagedHook = ManagedHook {
    name:     REFERENCE_TRANSACTION_HOOK_NAME,
    marker:   REFERENCE_TRANSACTION_MARKER,
    dispatch: ManagedHookDispatch::ReferenceTransaction,
};
/// The post-commit drift-warning hook definition registered below.
const POST_COMMIT_HOOK: ManagedHook = ManagedHook {
    name:     POST_COMMIT_HOOK_NAME,
    marker:   POST_COMMIT_MARKER,
    dispatch: ManagedHookDispatch::PostCommit,
};
/// The complete managed hook registry extended by later hook-owning phases.
const MANAGED_HOOKS: &[ManagedHook] = &[REFERENCE_TRANSACTION_HOOK, POST_COMMIT_HOOK];

/// The activation outcome for one managed hook name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedHookInstallation {
    name:       &'static str,
    activation: ManagedHookActivationOutcome,
}

impl ManagedHookInstallation {
    /// Return the managed git hook name represented by this installation result.
    pub(crate) const fn name(&self) -> &'static str { self.name }

    /// Return whether the managed hook is active and how initialization reached that state.
    pub(crate) const fn activation(&self) -> &ManagedHookActivationOutcome { &self.activation }
}

/// Whether one managed hook will run after initialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ManagedHookActivationOutcome {
    /// The managed hook is installed and executable.
    Active {
        /// Whether this call created or retained the managed hook.
        installation: ActiveManagedHookInstallation,
    },
    /// The managed hook is not in force.
    Inactive {
        /// Why initialization could not activate this hook.
        reason: ManagedHookInactivity,
    },
}

/// How an active managed hook reached its current state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveManagedHookInstallation {
    /// The hook path did not exist and now contains the managed script.
    Installed,
    /// The existing managed script was already current or was refreshed in place.
    Current,
}

/// Why one managed hook is not in force after initialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ManagedHookInactivity {
    /// An unrelated hook already owns this name and remains byte-identical.
    PreservedUnmanaged,
    /// The hook could not be inspected, written, or made executable.
    InstallationFailed {
        /// The filesystem or git diagnostic returned while installing the hook.
        diagnostic: String,
    },
}

/// Install or refresh each registered script without overwriting unrelated hooks.
pub(crate) fn install_managed_hooks(
    common_git_directory: &Path,
    policy_worktree: &Path,
    trunk_reference: &str,
) -> Vec<ManagedHookInstallation> {
    let setup = git::hooks_directory(policy_worktree)
        .map_err(HookInstallationError::from)
        .and_then(|hooks_directory| {
            fs::create_dir_all(&hooks_directory)?;
            Ok(hooks_directory)
        });
    let hooks_directory = match setup {
        Ok(hooks_directory) => hooks_directory,
        Err(error) => return failed_managed_hook_installations(&error),
    };
    MANAGED_HOOKS
        .iter()
        .map(|hook| {
            let activation = match install_managed_hook(
                &hooks_directory,
                hook,
                common_git_directory,
                policy_worktree,
                trunk_reference,
            ) {
                Ok(activation) => activation,
                Err(error) => ManagedHookActivationOutcome::Inactive {
                    reason: ManagedHookInactivity::InstallationFailed {
                        diagnostic: error.to_string(),
                    },
                },
            };
            ManagedHookInstallation {
                name: hook.name,
                activation,
            }
        })
        .collect()
}

fn failed_managed_hook_installations(
    error: &HookInstallationError,
) -> Vec<ManagedHookInstallation> {
    let diagnostic = error.to_string();
    MANAGED_HOOKS
        .iter()
        .map(|hook| ManagedHookInstallation {
            name:       hook.name,
            activation: ManagedHookActivationOutcome::Inactive {
                reason: ManagedHookInactivity::InstallationFailed {
                    diagnostic: diagnostic.clone(),
                },
            },
        })
        .collect()
}

fn install_managed_hook(
    hooks_directory: &Path,
    hook: &ManagedHook,
    common_git_directory: &Path,
    policy_worktree: &Path,
    trunk_reference: &str,
) -> Result<ManagedHookActivationOutcome, HookInstallationError> {
    let hook_path = hooks_directory.join(hook.name);
    let existing = match fs::read(&hook_path) {
        Ok(existing) => Some(existing),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(HookInstallationError::Io(error)),
    };
    if existing.as_ref().is_some_and(|contents| {
        !contents
            .windows(hook.marker.len())
            .any(|window| window == hook.marker.as_bytes())
    }) {
        return Ok(ManagedHookActivationOutcome::Inactive {
            reason: ManagedHookInactivity::PreservedUnmanaged,
        });
    }
    let was_present = existing.is_some();
    let script = hook.script(common_git_directory, policy_worktree, trunk_reference);
    if existing.as_deref() == Some(script.as_bytes()) {
        let mut permissions = fs::metadata(&hook_path)?.permissions();
        permissions.set_mode(EXECUTABLE_PERMISSIONS);
        fs::set_permissions(&hook_path, permissions)?;
    } else {
        PendingManagedHookReplacement::create(hooks_directory, hook.name)?
            .activate(&hook_path, script.as_bytes())?;
    }
    let _ = fs::File::open(hooks_directory).and_then(|directory| directory.sync_all());
    let installation = if was_present {
        ActiveManagedHookInstallation::Current
    } else {
        ActiveManagedHookInstallation::Installed
    };
    Ok(ManagedHookActivationOutcome::Active { installation })
}

/// A fully written replacement that is not visible at the managed hook path yet.
struct PendingManagedHookReplacement {
    path: PathBuf,
    file: File,
}

impl PendingManagedHookReplacement {
    fn create(hooks_directory: &Path, hook_name: &str) -> std::io::Result<Self> {
        const MAXIMUM_CREATE_ATTEMPTS: u16 = 1_024;

        for attempt in 0..MAXIMUM_CREATE_ATTEMPTS {
            let path = hooks_directory.join(format!(
                ".cargo-berth-{hook_name}-{}-{attempt}.tmp",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok(Self { path, file }),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {},
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            ErrorKind::AlreadyExists,
            format!("could not allocate a temporary replacement for hook {hook_name}"),
        ))
    }

    fn activate(mut self, hook_path: &Path, script: &[u8]) -> std::io::Result<()> {
        self.file.write_all(script)?;
        let mut permissions = self.file.metadata()?.permissions();
        permissions.set_mode(EXECUTABLE_PERMISSIONS);
        self.file.set_permissions(permissions)?;
        self.file.sync_all()?;
        fs::rename(&self.path, hook_path)
    }
}

impl Drop for PendingManagedHookReplacement {
    fn drop(&mut self) { let _ = fs::remove_file(&self.path); }
}

impl ManagedHook {
    fn script(
        &self,
        common_git_directory: &Path,
        policy_worktree: &Path,
        trunk_reference: &str,
    ) -> String {
        let pending_marker_prefix = shell_single_quoted(
            &common_git_directory
                .join(PENDING_BYPASS_FILE_PREFIX)
                .to_string_lossy(),
        );
        let pending_marker_suffix = shell_single_quoted(PENDING_BYPASS_FILE_SUFFIX);
        let policy_worktree = shell_single_quoted(&policy_worktree.to_string_lossy());
        let trunk_reference = shell_single_quoted(trunk_reference);
        match self.dispatch {
            ManagedHookDispatch::PostCommit => format!(
                "#!/bin/sh\n{POST_COMMIT_MARKER}\nif [ \"${{CARGO_BERTH_BYPASS:-}}\" = \"1\" ]; then\n    exit 0\nfi\n{EXECUTABLE_RESOLUTION}\nif [ ! -x \"$cargo_berth_executable\" ]; then\n    printf '%s\\n' 'cargo-berth could not check this commit drift because its executable is unavailable. Run `cargo-berth drift --full` by hand; this commit remains in place.' >&2\n    exit 0\nfi\nCARGO_BERTH_POST_COMMIT=1 \"$cargo_berth_executable\" drift --full\nstatus=$?\nif [ \"$status\" -eq 126 ] || [ \"$status\" -eq 127 ]; then\n    printf '%s\\n' 'cargo-berth could not run the post-commit drift check. Run `cargo-berth drift --full` by hand; this commit remains in place.' >&2\nfi\nexit 0\n"
            ),
            ManagedHookDispatch::ReferenceTransaction => reference_transaction_script(
                &pending_marker_prefix,
                &pending_marker_suffix,
                &policy_worktree,
                &trunk_reference,
            ),
        }
    }
}

const REFERENCE_TRANSACTION_SCRIPT_TEMPLATE: &str = r#"#!/bin/sh
__REFERENCE_TRANSACTION_MARKER__
__ISSUING_DIRECTORY_ENVIRONMENT__=$PWD
export __ISSUING_DIRECTORY_ENVIRONMENT__
if [ -d __POLICY_WORKTREE__ ]; then
    cd __POLICY_WORKTREE__
fi
cargo_berth_trunk_reference=__TRUNK_REFERENCE__
case "${1:-}" in
    preparing|aborted) exit 0 ;;
    prepared|committed) ;;
    *) exit 0 ;;
esac

__EXECUTABLE_RESOLUTION__

transaction_input=''
transaction_buffered=0
buffered_transaction_input=''
if buffered_transaction_input=$(umask 077; mktemp "${TMPDIR:-/tmp}/cargo-berth-reference-transaction.XXXXXX") 2>/dev/null; then
    transaction_input=$buffered_transaction_input
    trap 'rm -f "$transaction_input"' EXIT HUP INT TERM
    if ! cat > "$transaction_input"; then
        printf '%s\n' 'cargo-berth could not preserve the complete ref transaction; refusing to decide from partial input. Retry the git command after correcting temporary-file access.' >&2
        exit 1
    fi
    transaction_buffered=1
fi

if [ "$transaction_buffered" -eq 1 ]; then
    LC_ALL=C grep -q '[^	 -~]' "$transaction_input"
    transaction_byte_scan_status=$?
    if [ "$transaction_byte_scan_status" -eq 1 ]; then
        LC_ALL=C awk -v phase="$1" -v trunk="$cargo_berth_trunk_reference" '
        function valid_transaction_bytes(value, byte_index, byte) {
            for (byte_index = 1; byte_index <= length(value); byte_index += 1) {
                byte = substr(value, byte_index, 1)
                if (byte != tab && byte !~ /^[ -~]$/) {
                    return 0
                }
            }
            return 1
        }
        function valid_full_ref(value, suffix, count, components, component_index, component) {
            if (substr(value, 1, 5) != "refs/" || length(value) == 5 || value ~ /\.\./ || index(value, "@{") != 0 || substr(value, length(value), 1) == ".") {
                return 0
            }
            if (value ~ /[^!-~]/ || value ~ /[~^:?*\[\\]/) {
                return 0
            }
            suffix = substr(value, 6)
            count = split(suffix, components, "/")
            for (component_index = 1; component_index <= count; component_index += 1) {
                component = components[component_index]
                if (component == "" || substr(component, 1, 1) == "." || component ~ /\.lock$/) {
                    return 0
                }
            }
            return 1
        }
        function valid_object(value, length_) {
            if (substr(value, 1, 4) == "ref:") {
                return valid_full_ref(substr(value, 5))
            }
            length_ = length(value)
            return (length_ == 40 || length_ == 64) && value !~ /[^0-9a-f]/
        }
        BEGIN {
            decision = 1
            tab = sprintf("%c", 9)
        }
        {
            if (!valid_transaction_bytes($0) || NF != 3) {
                malformed = 1
                next
            }
            if (substr($3, 1, 11) == "refs/heads/") {
                if (!valid_object($1) || !valid_object($2) || !valid_full_ref($3)) {
                    malformed = 1
                }
                if (phase == "committed") {
                    decision = 0
                }
            }
            if (phase == "prepared" && $3 == trunk) {
                decision = 0
            }
        }
        END {
            if (malformed) {
                exit 2
            }
            exit decision
        }
        ' "$transaction_input"
        dispatch_status=$?
    else
        dispatch_status=2
    fi
    case "$dispatch_status" in
        0|2) ;;
        1)
            if [ "$1" = "prepared" ] && ! git show-ref --verify --quiet "$cargo_berth_trunk_reference" >/dev/null 2>&1; then
                :
            else
                exit 0
            fi
            ;;
        *) ;;
    esac
fi

bypassed_merge_id="${CARGO_BERTH_BYPASSED_MERGE_ID:-git-process-${PPID:-$$}}"
case "$bypassed_merge_id" in
    ''|*[!A-Za-z0-9_-]*) bypassed_merge_id="git-process-${PPID:-$$}" ;;
esac
if [ "${CARGO_BERTH_BYPASS:-}" = "1" ]; then
    if [ -x "$cargo_berth_executable" ]; then
        if [ "$transaction_buffered" -eq 1 ]; then
            CARGO_BERTH_BYPASSED_MERGE_ID="$bypassed_merge_id" "$cargo_berth_executable" __reference-transaction "$@" "$cargo_berth_trunk_reference" < "$transaction_input"
        else
            CARGO_BERTH_BYPASSED_MERGE_ID="$bypassed_merge_id" "$cargo_berth_executable" __reference-transaction "$@" "$cargo_berth_trunk_reference"
        fi
        status=$?
        if [ "$status" -eq 0 ]; then
            exit 0
        fi
        printf '%s\n' 'cargo-berth could not record this bypass; permitting this ref transaction and leaving a marker to report it later. Rerun cargo berth init after restoring cargo-berth. CARGO_BERTH_BYPASS=1 remains the explicit override.' >&2
    else
        printf '%s\n' 'cargo-berth trunk gate executable is unavailable; permitting this ref transaction. Rerun cargo berth init after restoring cargo-berth. CARGO_BERTH_BYPASS=1 remains the explicit override.' >&2
    fi
    if [ "$1" = "prepared" ]; then
        if occurred_at=$(date -u '+%Y-%m-%dT%H:%M:%S.000Z' 2>/dev/null); then
            case "$occurred_at" in
                [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9][0-9]Z) marker_contents='{"cause":{"kind":"environment_override","bypassed_merge":"'"$bypassed_merge_id"'"},"occurrence_time":{"status":"known","at":"'"$occurred_at"'"}}' ;;
                *) marker_contents='{"cause":{"kind":"environment_override","bypassed_merge":"'"$bypassed_merge_id"'"},"occurrence_time":{"status":"unavailable"}}' ;;
            esac
        else
            marker_contents='{"cause":{"kind":"environment_override","bypassed_merge":"'"$bypassed_merge_id"'"},"occurrence_time":{"status":"unavailable"}}'
        fi
        marker_base=__PENDING_MARKER_PREFIX__"$$"
        marker="$marker_base"__PENDING_MARKER_SUFFIX__
        sequence=0
        while [ -e "$marker" ]; do
            sequence=$((sequence + 1))
            marker="$marker_base-$sequence"__PENDING_MARKER_SUFFIX__
        done
        (umask 077; set -C; printf '%s\n' "$marker_contents" > "$marker") 2>/dev/null || :
    fi
    exit 0
fi
if [ ! -x "$cargo_berth_executable" ]; then
    printf '%s\n' 'cargo-berth trunk gate executable is unavailable; permitting this ref transaction. Rerun cargo berth init after restoring cargo-berth. CARGO_BERTH_BYPASS=1 remains the explicit override.' >&2
    exit 0
fi
if [ "$transaction_buffered" -eq 1 ]; then
    "$cargo_berth_executable" __reference-transaction "$@" "$cargo_berth_trunk_reference" < "$transaction_input"
else
    "$cargo_berth_executable" __reference-transaction "$@" "$cargo_berth_trunk_reference"
fi
status=$?
if [ "$status" -eq 126 ] || [ "$status" -eq 127 ]; then
    printf '%s\n' 'cargo-berth trunk gate executable could not run; permitting this ref transaction. Rerun cargo berth init after restoring cargo-berth. CARGO_BERTH_BYPASS=1 remains the explicit override.' >&2
    exit 0
fi
exit "$status"
"#;

fn reference_transaction_script(
    pending_marker_prefix: &str,
    pending_marker_suffix: &str,
    policy_worktree: &str,
    trunk_reference: &str,
) -> String {
    render_reference_transaction_template(&[
        (
            "__REFERENCE_TRANSACTION_MARKER__",
            REFERENCE_TRANSACTION_MARKER,
        ),
        (
            "__ISSUING_DIRECTORY_ENVIRONMENT__",
            REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT,
        ),
        ("__POLICY_WORKTREE__", policy_worktree),
        ("__TRUNK_REFERENCE__", trunk_reference),
        ("__EXECUTABLE_RESOLUTION__", EXECUTABLE_RESOLUTION),
        ("__PENDING_MARKER_PREFIX__", pending_marker_prefix),
        ("__PENDING_MARKER_SUFFIX__", pending_marker_suffix),
    ])
}

fn render_reference_transaction_template(substitutions: &[(&str, &str)]) -> String {
    let mut rendered = String::with_capacity(REFERENCE_TRANSACTION_SCRIPT_TEMPLATE.len());
    let mut remaining = REFERENCE_TRANSACTION_SCRIPT_TEMPLATE;
    while let Some((offset, placeholder, replacement)) = substitutions
        .iter()
        .filter_map(|(placeholder, replacement)| {
            remaining
                .find(*placeholder)
                .map(|offset| (offset, *placeholder, *replacement))
        })
        .min_by_key(|(offset, _, _)| *offset)
    {
        rendered.push_str(&remaining[..offset]);
        rendered.push_str(replacement);
        remaining = &remaining[offset + placeholder.len()..];
    }
    rendered.push_str(remaining);
    rendered
}

/// Render the reference-transaction script for the marker-writer agreement tests.
#[cfg(test)]
pub(super) fn reference_transaction_hook_script_for_test(
    common_git_directory: &Path,
    policy_worktree: &Path,
    trunk_reference: &str,
) -> String {
    REFERENCE_TRANSACTION_HOOK.script(common_git_directory, policy_worktree, trunk_reference)
}

fn shell_single_quoted(value: &str) -> String { format!("'{}'", value.replace('\'', "'\"'\"'")) }

/// A managed hook could not be inspected, written, or made executable.
#[derive(Debug)]
enum HookInstallationError {
    /// Filesystem access failed.
    Io(std::io::Error),
    /// Git could not resolve its effective hook directory.
    Git(GitError),
}

impl Display for HookInstallationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "managed hook installation failed: {error}"),
            Self::Git(error) => write!(formatter, "managed hook path resolution failed: {error}"),
        }
    }
}

impl std::error::Error for HookInstallationError {}

impl From<std::io::Error> for HookInstallationError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<GitError> for HookInstallationError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}

#[cfg(test)]
mod tests {
    use super::EXECUTABLE_ENVIRONMENT;
    use super::EXECUTABLE_RESOLUTION;
    use super::reference_transaction_script;

    #[test]
    fn executable_resolution_reads_the_override_it_documents() {
        assert!(
            EXECUTABLE_RESOLUTION.contains(EXECUTABLE_ENVIRONMENT),
            "resolution shell must read {EXECUTABLE_ENVIRONMENT}"
        );
    }

    #[test]
    fn template_rendering_does_not_rescan_inserted_placeholder_text() {
        let values = [
            "'/bin/__TRUNK_REFERENCE__'",
            "'/git/__PENDING_MARKER_SUFFIX__'",
            "'.json.__POLICY_WORKTREE__'",
            "'refs/heads/__PENDING_MARKER_PREFIX__'",
        ];

        let script = reference_transaction_script(values[0], values[1], values[2], values[3]);

        for value in values {
            assert!(script.contains(value), "rendering changed {value}");
        }
    }
}
