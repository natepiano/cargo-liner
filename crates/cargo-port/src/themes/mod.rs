//! cargo-port's themes: the palettes it ships and where user files live.
//!
//! The framework owns theme types, registry assembly, the directory
//! watch, the resolver, and the OS appearance poller — machinery, no
//! colors. The app owns the palettes ([`builtins()`]) and the on-disk
//! location: `dirs::config_dir() / "cargo-port" / "themes"`.

mod builtins;
mod constants;
mod paths;

pub(crate) use builtins::builtins;
#[cfg(test)]
pub(crate) use paths::ThemesDirOverrideGuard;
#[cfg(test)]
pub(crate) use paths::set_themes_dir_override_for_test;
pub(crate) use paths::themes_dir;
