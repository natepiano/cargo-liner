//! Per-user themes directory path for cargo-port.
//!
//! The framework owns theme types, registry assembly, the directory
//! watch, the resolver, and the OS appearance poller. The app owns
//! the on-disk location: `dirs::config_dir() / "cargo-port" / "themes"`.

mod constants;
mod paths;

#[cfg(test)]
pub(crate) use paths::ThemesDirOverrideGuard;
#[cfg(test)]
pub(crate) use paths::set_themes_dir_override_for_test;
pub(crate) use paths::themes_dir;
