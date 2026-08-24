//! Idempotent installation of every git hook managed by this crate.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::permit::PENDING_BYPASS_FILE_PREFIX;
use super::permit::PENDING_BYPASS_FILE_SUFFIX;
use crate::git;

const EXECUTABLE_PERMISSIONS: u32 = 0o755;
const REFERENCE_TRANSACTION_HOOK_NAME: &str = "reference-transaction";
const REFERENCE_TRANSACTION_MARKER: &str = "# cargo-berth managed hook: reference-transaction";
/// One hook name paired with the complete script body owned by `cargo-berth`.
#[derive(Clone, Copy)]
struct ManagedHook {
    name:     &'static str,
    marker:   &'static str,
    dispatch: ManagedHookDispatch,
}

#[derive(Clone, Copy)]
enum ManagedHookDispatch {
    ReferenceTransaction,
}

/// The reference-transaction hook definition registered below.
const REFERENCE_TRANSACTION_HOOK: ManagedHook = ManagedHook {
    name:     REFERENCE_TRANSACTION_HOOK_NAME,
    marker:   REFERENCE_TRANSACTION_MARKER,
    dispatch: ManagedHookDispatch::ReferenceTransaction,
};
/// The complete managed hook registry extended by later hook-owning phases.
const MANAGED_HOOKS: &[ManagedHook] = &[REFERENCE_TRANSACTION_HOOK];

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
    let setup = std::env::current_exe()
        .map_err(HookInstallationError::from)
        .and_then(|executable| {
            let hooks_directory = git::hooks_directory(policy_worktree)?;
            fs::create_dir_all(&hooks_directory)?;
            Ok((executable, hooks_directory))
        });
    let (executable, hooks_directory) = match setup {
        Ok(setup) => setup,
        Err(error) => return failed_managed_hook_installations(&error),
    };
    MANAGED_HOOKS
        .iter()
        .map(|hook| {
            let activation = match install_managed_hook(
                &hooks_directory,
                hook,
                &executable,
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
    executable: &Path,
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
    let script = hook.script(
        executable,
        common_git_directory,
        policy_worktree,
        trunk_reference,
    );
    if existing.as_deref() != Some(script.as_bytes()) {
        let mut hook_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&hook_path)?;
        hook_file.write_all(script.as_bytes())?;
        hook_file.sync_all()?;
    }
    let mut permissions = fs::metadata(&hook_path)?.permissions();
    permissions.set_mode(EXECUTABLE_PERMISSIONS);
    fs::set_permissions(&hook_path, permissions)?;
    fs::File::open(hooks_directory)?.sync_all()?;
    let installation = if was_present {
        ActiveManagedHookInstallation::Current
    } else {
        ActiveManagedHookInstallation::Installed
    };
    Ok(ManagedHookActivationOutcome::Active { installation })
}

impl ManagedHook {
    fn script(
        &self,
        executable: &Path,
        common_git_directory: &Path,
        policy_worktree: &Path,
        trunk_reference: &str,
    ) -> String {
        let executable = shell_single_quoted(&executable.to_string_lossy());
        let pending_marker_prefix = shell_single_quoted(
            &common_git_directory
                .join(PENDING_BYPASS_FILE_PREFIX)
                .to_string_lossy(),
        );
        let pending_marker_suffix = shell_single_quoted(PENDING_BYPASS_FILE_SUFFIX);
        let policy_worktree = shell_single_quoted(&policy_worktree.to_string_lossy());
        let trunk_reference = shell_single_quoted(trunk_reference);
        match self.dispatch {
            ManagedHookDispatch::ReferenceTransaction => format!(
                "#!/bin/sh\n{REFERENCE_TRANSACTION_MARKER}\nif [ -d {policy_worktree} ]; then\n    cd {policy_worktree}\nfi\nif [ \"${{CARGO_BERTH_BYPASS:-}}\" = \"1\" ]; then\n    if [ -x {executable} ]; then\n        {executable} __reference-transaction \"$@\" {trunk_reference}\n        status=$?\n        if [ \"$status\" -eq 0 ]; then\n            exit 0\n        fi\n        printf '%s\\n' 'cargo-berth could not record this bypass; permitting this ref transaction and leaving a marker to report it later. Rerun cargo berth init after restoring cargo-berth. CARGO_BERTH_BYPASS=1 remains the explicit override.' >&2\n    else\n        printf '%s\\n' 'cargo-berth trunk gate executable is unavailable; permitting this ref transaction. Rerun cargo berth init after restoring cargo-berth. CARGO_BERTH_BYPASS=1 remains the explicit override.' >&2\n    fi\n    if [ \"$1\" = \"prepared\" ]; then\n        if occurred_at=$(date -u '+%Y-%m-%dT%H:%M:%S.000Z' 2>/dev/null); then\n            case \"$occurred_at\" in\n                [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9][0-9]Z) marker_contents='{{\"cause\":{{\"kind\":\"environment_override\"}},\"occurrence_time\":{{\"status\":\"known\",\"at\":\"'\"$occurred_at\"'\"}}}}' ;;\n                *) marker_contents='{{\"cause\":{{\"kind\":\"environment_override\"}},\"occurrence_time\":{{\"status\":\"unavailable\"}}}}' ;;\n            esac\n        else\n            marker_contents='{{\"cause\":{{\"kind\":\"environment_override\"}},\"occurrence_time\":{{\"status\":\"unavailable\"}}}}'\n        fi\n        marker_base={pending_marker_prefix}\"$$\"\n        marker=\"$marker_base\"{pending_marker_suffix}\n        sequence=0\n        while [ -e \"$marker\" ]; do\n            sequence=$((sequence + 1))\n            marker=\"$marker_base-$sequence\"{pending_marker_suffix}\n        done\n        (umask 077; set -C; printf '%s\\n' \"$marker_contents\" > \"$marker\") 2>/dev/null || :\n    fi\n    exit 0\nfi\nif [ ! -x {executable} ]; then\n    printf '%s\\n' 'cargo-berth trunk gate executable is unavailable; permitting this ref transaction. Rerun cargo berth init after restoring cargo-berth. CARGO_BERTH_BYPASS=1 remains the explicit override.' >&2\n    exit 0\nfi\n{executable} __reference-transaction \"$@\" {trunk_reference}\nstatus=$?\nif [ \"$status\" -eq 126 ] || [ \"$status\" -eq 127 ]; then\n    printf '%s\\n' 'cargo-berth trunk gate executable could not run; permitting this ref transaction. Rerun cargo berth init after restoring cargo-berth. CARGO_BERTH_BYPASS=1 remains the explicit override.' >&2\n    exit 0\nfi\nexit \"$status\"\n"
            ),
        }
    }
}

/// Render the reference-transaction script for marker compatibility tests.
#[cfg(test)]
pub(super) fn reference_transaction_hook_script_for_test(
    executable: &Path,
    common_git_directory: &Path,
    policy_worktree: &Path,
    trunk_reference: &str,
) -> String {
    REFERENCE_TRANSACTION_HOOK.script(
        executable,
        common_git_directory,
        policy_worktree,
        trunk_reference,
    )
}

fn shell_single_quoted(value: &str) -> String { format!("'{}'", value.replace('\'', "'\"'\"'")) }

/// A managed hook could not be inspected, written, or made executable.
#[derive(Debug)]
pub(crate) enum HookInstallationError {
    /// Filesystem access failed.
    Io(std::io::Error),
    /// Git could not resolve its effective hook directory.
    Git(crate::git::GitError),
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

impl From<crate::git::GitError> for HookInstallationError {
    fn from(error: crate::git::GitError) -> Self { Self::Git(error) }
}
