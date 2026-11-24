//! Named values of the `sites` command.

// command
pub(crate) const SITES_SUBCOMMAND: &str = "sites";

// ledger
/// The ledger schema that the pinned Python ledger writes; `sites` also reads schema 47.
pub(super) const SITES_LEDGER_SCHEMA_VERSION: i64 = 51;

// protocol
pub(super) const SITES_REPLY_SCHEMA: u32 = 1;

// site identity
/// Lowercase hexadecimal digits, indexed by a 4-bit value.
pub(super) const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

// display text
/// Python keeps the first 200 characters of a site's stripped display text.
pub(super) const DISPLAY_LIMIT: usize = 200;

// exclusion interpreter
/// The variable that names the first Python interpreter that `scripts/lib/py` tries.
pub(super) const CLAUDE_PYTHON: &str = "CLAUDE_PYTHON";
/// The `python3` paths that `scripts/lib/py` tries after the per-user profile, in order.
pub(super) const PACKAGE_PYTHONS: [&str; 2] =
    ["/opt/homebrew/bin/python3", "/usr/local/bin/python3"];
/// The folder that holds each user's profile; `scripts/lib/py` tries `<folder>/$USER/bin/python3`.
pub(super) const PER_USER_PROFILES: &str = "/etc/profiles/per-user";
pub(super) const PROFILE_PYTHON: &str = "bin/python3";
/// The command name that `scripts/lib/py` looks up on `PATH`.
pub(super) const PYTHON_COMMAND: &str = "python3";
/// Exits 0 only on Python 3.12 or newer, the floor that `scripts/lib/py` sets.
pub(super) const PYTHON_VERSION_CHECK: &str =
    "import sys; sys.exit(0 if sys.version_info >= (3, 12) else 1)";
pub(super) const SYSTEM_PYTHON: &str = "/run/current-system/sw/bin/python3";
