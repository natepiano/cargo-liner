//! Per-user themes directory path for cargo-port.
//!
//! The framework owns theme types, registry assembly, the directory
//! watch, the resolver, and the OS appearance poller. The app owns
//! the on-disk location: `dirs::config_dir() / "cargo-port" / "themes"`.

mod constants;
mod paths;

pub(crate) use paths::themes_dir;
