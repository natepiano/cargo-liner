//! Read configuration changes for the terminal loop.

#[cfg(target_os = "macos")]
mod macos;
mod word_highlight_updates;

#[cfg(test)]
pub(crate) use word_highlight_updates::ConfigFileReadFixture;
pub(crate) use word_highlight_updates::WordHighlightUpdate;
pub(crate) use word_highlight_updates::WordHighlightUpdates;
