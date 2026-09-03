use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use chrono::DateTime;
use chrono::FixedOffset;
use chrono::Local;
use toml::Table;
use toml::Value;
use toml::ser::Error;
use tui_pane::BandSettings;
use tui_pane::PixelSettings;
use tui_pane::TextSettings;
use uuid::Uuid;

use super::recognition;
use super::recognition::UnrecognizedFavoriteValue;
use crate::attract::AttractMode;
use crate::constants::FAVORITE_ID_KEY;
use crate::constants::FAVORITE_SAVED_KEY;
use crate::constants::FAVORITES_ARRAY_KEY;

/// Stable identity for one favorite row.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct FavoriteId(pub(super) Uuid);

impl FavoriteId {
    #[cfg(test)]
    pub(super) const fn from_uuid_for_test(uuid: Uuid) -> Self { Self(uuid) }
}

impl Display for FavoriteId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { self.0.fmt(formatter) }
}

/// Parameters that one attract-screen mode runs with, whether loaded or newly drawn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AttractSettings {
    /// Moving-band parameters.
    MovingBand(BandSettings),
    /// Moving-text parameters.
    MovingText(TextSettings),
    /// Pixelate parameters.
    Pixelate(PixelSettings),
}

impl AttractSettings {
    /// Attract mode that owns these parameters.
    #[must_use]
    pub(crate) const fn mode(&self) -> AttractMode {
        match self {
            Self::MovingBand(_) => AttractMode::MovingBand,
            Self::MovingText(_) => AttractMode::MovingText,
            Self::Pixelate(_) => AttractMode::Pixelate,
        }
    }
}

/// Typed values derived from one recognized favorite table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Favorite {
    /// Stable row identity used by selection and deletion.
    pub(crate) id:       FavoriteId,
    /// Local RFC 3339 time of the most recent save.
    pub(crate) saved:    DateTime<FixedOffset>,
    /// Mode-specific animation parameters.
    pub(crate) settings: AttractSettings,
}

impl Favorite {
    pub(super) fn now(settings: AttractSettings) -> Self {
        let saved = Local::now().fixed_offset();
        Self {
            id: FavoriteId(Uuid::now_v7()),
            saved: recognition::timestamp_at_file_precision(saved),
            settings,
        }
    }
}

/// Successful effect of saving one attract parameter set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FavoriteSaveOutcome {
    /// A new favorite row was appended.
    Added,
    /// An existing row with identical settings received the new timestamp.
    Refreshed,
}

/// Opaque identity for removing one unrecognized raw favorite table.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UnrecognizedFavoriteRemovalLocator {
    raw_table_index: usize,
    fingerprint:     String,
}

impl UnrecognizedFavoriteRemovalLocator {
    fn new(raw_table_index: usize, table: &Table) -> Self {
        Self {
            raw_table_index,
            fingerprint: table.to_string(),
        }
    }

    fn locate(&self, tables: &[Table]) -> UnrecognizedFavoriteTableLocation {
        if tables
            .get(self.raw_table_index)
            .is_some_and(|table| self.matches(table))
        {
            return UnrecognizedFavoriteTableLocation::ExactlyOne(self.raw_table_index);
        }

        let mut matching_indices = tables
            .iter()
            .enumerate()
            .filter_map(|(index, table)| self.matches(table).then_some(index));
        let Some(candidate_index) = matching_indices.next() else {
            return UnrecognizedFavoriteTableLocation::NotExactlyOne;
        };
        if matching_indices.next().is_some() {
            UnrecognizedFavoriteTableLocation::NotExactlyOne
        } else {
            UnrecognizedFavoriteTableLocation::ExactlyOne(candidate_index)
        }
    }

    fn matches(&self, table: &Table) -> bool { table.to_string() == self.fingerprint }
}

enum UnrecognizedFavoriteTableLocation {
    ExactlyOne(usize),
    NotExactlyOne,
}

/// Outcome of checking and removing an unrecognized favorite locator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UnrecognizedFavoriteRemoval {
    /// The locator still named one table, which was removed.
    Removed,
    /// The locator matched zero or multiple tables, so none was removed.
    LocatorStale,
}

/// Typed recognition result for one raw favorite table.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FavoriteRowRecognition {
    /// The table contains one complete, recognized favorite.
    Recognized(Favorite),
    /// The table is retained and diagnosed in the overlay, but excluded from loading.
    Unrecognized {
        /// The first field value that could not be recognized.
        diagnostic:      UnrecognizedFavoriteValue,
        /// Identity used to re-verify the raw table during deletion.
        removal_locator: UnrecognizedFavoriteRemovalLocator,
    },
}

/// Raw favorite tables together with their ordered typed interpretations.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FavoriteRows {
    tables:            Vec<Table>,
    recognitions:      Vec<FavoriteRowRecognition>,
    additional_fields: Table,
}

impl FavoriteRows {
    /// All row recognition results, with recognized rows grouped by mode and newest first.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &FavoriteRowRecognition> {
        self.recognitions.iter()
    }

    /// Recognized favorites, grouped by mode and newest first within each mode.
    pub(crate) fn recognized(&self) -> impl Iterator<Item = &Favorite> {
        self.recognitions
            .iter()
            .filter_map(|recognition| match recognition {
                FavoriteRowRecognition::Recognized(favorite) => Some(favorite),
                FavoriteRowRecognition::Unrecognized { .. } => None,
            })
    }

    pub(super) fn parse(text: &str) -> Result<Self, String> {
        let mut additional_fields =
            toml::from_str::<Table>(text).map_err(|error| error.to_string())?;
        let tables = match additional_fields.remove(FAVORITES_ARRAY_KEY) {
            None => Vec::new(),
            Some(Value::Array(values)) => values
                .into_iter()
                .enumerate()
                .map(|(index, value)| match value {
                    Value::Table(table) => Ok(table),
                    _ => Err(format!("{FAVORITES_ARRAY_KEY}[{index}] must be a table")),
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(format!("{FAVORITES_ARRAY_KEY} must be an array of tables")),
        };
        let mut rows = Self {
            tables,
            recognitions: Vec::new(),
            additional_fields,
        };
        rows.refresh_recognitions();
        Ok(rows)
    }

    pub(super) fn serialize(&self) -> Result<String, Error> {
        let mut table = self.additional_fields.clone();
        table.insert(
            FAVORITES_ARRAY_KEY.to_string(),
            Value::Array(self.tables.iter().cloned().map(Value::Table).collect()),
        );
        toml::to_string_pretty(&table)
    }

    fn refresh_recognitions(&mut self) {
        let mut recognized_ids = HashSet::new();
        self.recognitions = self
            .tables
            .iter()
            .enumerate()
            .map(
                |(raw_table_index, table)| match recognition::recognize_favorite(table) {
                    Ok(favorite) if !recognized_ids.insert(favorite.id) => unrecognized_row(
                        raw_table_index,
                        table,
                        UnrecognizedFavoriteValue::new(
                            FAVORITE_ID_KEY,
                            format!("{} (duplicate)", favorite.id),
                        ),
                    ),
                    Ok(favorite) => FavoriteRowRecognition::Recognized(favorite),
                    Err(diagnostic) => unrecognized_row(raw_table_index, table, diagnostic),
                },
            )
            .collect();
        self.recognitions.sort_by(compare_recognitions);
    }

    pub(super) fn push(&mut self, candidate: &Favorite) -> FavoriteSaveOutcome {
        let existing_index = self.tables.iter().position(|table| {
            matches!(
                recognition::recognize_favorite(table),
                Ok(favorite) if favorite.settings == candidate.settings
            )
        });
        let outcome = if let Some(index) = existing_index {
            self.tables[index].insert(
                FAVORITE_SAVED_KEY.to_string(),
                Value::String(recognition::timestamp_spelling(candidate.saved)),
            );
            FavoriteSaveOutcome::Refreshed
        } else {
            self.tables
                .push(recognition::table_from_favorite(candidate));
            FavoriteSaveOutcome::Added
        };
        self.refresh_recognitions();
        outcome
    }

    pub(super) fn remove_recognized(&mut self, favorite_id: FavoriteId) {
        let row = self.tables.iter().position(|table| {
            matches!(recognition::recognize_favorite(table), Ok(Favorite { id, .. }) if id == favorite_id)
        });
        if let Some(index) = row {
            self.tables.remove(index);
            self.refresh_recognitions();
        }
    }

    pub(super) fn remove_unrecognized(
        &mut self,
        removal_locator: &UnrecognizedFavoriteRemovalLocator,
    ) -> UnrecognizedFavoriteRemoval {
        match removal_locator.locate(&self.tables) {
            UnrecognizedFavoriteTableLocation::ExactlyOne(index) => {
                self.tables.remove(index);
                self.refresh_recognitions();
                UnrecognizedFavoriteRemoval::Removed
            },
            UnrecognizedFavoriteTableLocation::NotExactlyOne => {
                UnrecognizedFavoriteRemoval::LocatorStale
            },
        }
    }
}

fn compare_recognitions(left: &FavoriteRowRecognition, right: &FavoriteRowRecognition) -> Ordering {
    match (left, right) {
        (FavoriteRowRecognition::Recognized(left), FavoriteRowRecognition::Recognized(right)) => {
            attract_mode_order(left.settings.mode())
                .cmp(&attract_mode_order(right.settings.mode()))
                .then_with(|| right.saved.cmp(&left.saved))
                .then_with(|| right.id.cmp(&left.id))
        },
        (FavoriteRowRecognition::Recognized(_), FavoriteRowRecognition::Unrecognized { .. }) => {
            Ordering::Less
        },
        (FavoriteRowRecognition::Unrecognized { .. }, FavoriteRowRecognition::Recognized(_)) => {
            Ordering::Greater
        },
        (
            FavoriteRowRecognition::Unrecognized { .. },
            FavoriteRowRecognition::Unrecognized { .. },
        ) => Ordering::Equal,
    }
}

fn unrecognized_row(
    raw_table_index: usize,
    table: &Table,
    diagnostic: UnrecognizedFavoriteValue,
) -> FavoriteRowRecognition {
    FavoriteRowRecognition::Unrecognized {
        diagnostic,
        removal_locator: UnrecognizedFavoriteRemovalLocator::new(raw_table_index, table),
    }
}

const fn attract_mode_order(attract_mode: AttractMode) -> u8 {
    match attract_mode {
        AttractMode::MovingBand => 0,
        AttractMode::MovingText => 1,
        AttractMode::Pixelate => 2,
    }
}

#[cfg(test)]
pub(crate) fn parse_rows_for_overlay_test(text: &str) -> Result<FavoriteRows, String> {
    FavoriteRows::parse(text)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    const FIRST_ID: &str = "01a03f5e-9c14-7b41-8a02-1de4c7c9b330";
    const FIRST_SAVED: &str = "2026-08-26T09:02:44.870-07:00";
    const SECOND_SAVED: &str = "2026-08-26T14:31:05.412-07:00";

    #[test]
    fn duplicate_id_is_recognized_only_once() {
        let text = format!(
            r#"[[favorite]]
id = "{FIRST_ID}"
saved = "{FIRST_SAVED}"
mode = "moving_band"
direction = "right"
width = 12
speed = 40
tail_speed = 96
fraying = "both"

[[favorite]]
id = "{FIRST_ID}"
saved = "{SECOND_SAVED}"
mode = "pixelate"
direction = "left"
speed = 24
wave_percent = 145
block_columns = 6
resolve = "scatter"
fill = "solid"
"#
        );

        let rows = FavoriteRows::parse(&text).expect("duplicate fixture should parse");
        let recognitions = rows.iter().collect::<Vec<_>>();

        assert_eq!(rows.recognized().count(), 1);
        assert_eq!(recognitions.len(), 2);
        assert!(matches!(
            recognitions[1],
            FavoriteRowRecognition::Unrecognized {
                diagnostic: UnrecognizedFavoriteValue { key, spelling },
                ..
            }
                if key == FAVORITE_ID_KEY && spelling == &format!("{FIRST_ID} (duplicate)")
        ));
    }
}
