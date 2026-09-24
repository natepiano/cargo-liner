//! [`InitialRows`]: the `tiles.initial_rows` setting.

use serde::Deserialize;
use serde::Serialize;

use super::constants::DEFAULT_INITIAL_ROWS;
use super::constants::MAX_INITIAL_ROWS;
use crate::MIN_INITIAL_ROWS;
use crate::app_settings;
use crate::app_settings::SettingStep;

/// `tiles.initial_rows`: rows the grid grows to in a single column
/// before it starts arranging itself into a square.
///
/// Serialized as the bare integer, so an app keeps declaring its own
/// `[tiles]` table with this key in the position it chooses, and a bad
/// value reports the same parse error a plain integer would.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct InitialRows(usize);

impl Default for InitialRows {
    fn default() -> Self { Self(DEFAULT_INITIAL_ROWS) }
}

impl InitialRows {
    /// Rows the single column grows to, never below
    /// [`MIN_INITIAL_ROWS`].
    ///
    /// Clamped on read rather than at load so a hand-edited zero in
    /// `config.toml` is corrected rather than rejected: the file keeps
    /// what was typed, the grid stays laid out.
    #[must_use]
    pub fn get(self) -> usize { self.0.max(MIN_INITIAL_ROWS) }

    /// Move one step through [`MIN_INITIAL_ROWS`] to
    /// [`MAX_INITIAL_ROWS`], wrapping at both ends.
    pub(crate) fn step(&mut self, step: SettingStep) {
        let choices: Vec<String> = (MIN_INITIAL_ROWS..=MAX_INITIAL_ROWS)
            .map(|rows| rows.to_string())
            .collect();
        self.0 = app_settings::stepped(&choices, &self.get().to_string(), step)
            .parse()
            .unwrap_or(MIN_INITIAL_ROWS);
    }
}

#[cfg(test)]
mod tests {
    use super::InitialRows;
    use crate::app_settings::SettingStep;

    #[test]
    fn a_zero_reads_as_the_floor() {
        assert_eq!(InitialRows(0).get(), 1);
    }

    #[test]
    fn stepping_wraps_at_both_ends() {
        let mut rows = InitialRows(8);
        rows.step(SettingStep::Next);
        assert_eq!(rows, InitialRows(1));
        rows.step(SettingStep::Prev);
        assert_eq!(rows, InitialRows(8));
    }

    #[test]
    fn a_value_past_the_ceiling_steps_to_the_first_entry() {
        let mut rows = InitialRows(100);
        rows.step(SettingStep::Next);
        assert_eq!(rows, InitialRows(1));
    }
}
