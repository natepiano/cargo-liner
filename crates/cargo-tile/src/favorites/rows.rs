use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use chrono::DateTime;
use chrono::FixedOffset;
use chrono::Local;
use chrono::SecondsFormat;
use chrono::Timelike;
use toml::Table;
use toml::Value;
use tui_pane::BandDirection;
use tui_pane::BandFraying;
use tui_pane::BandSettings;
use tui_pane::PixelFill;
use tui_pane::PixelResolve;
use tui_pane::PixelSettings;
use tui_pane::TextDrift;
use tui_pane::TextFill;
use tui_pane::TextSettings;
use uuid::Uuid;

use crate::attract::AttractMode;
use crate::constants::FAVORITE_BLOCK_COLUMNS_KEY;
use crate::constants::FAVORITE_DIRECTION_KEY;
use crate::constants::FAVORITE_DRIFT_KEY;
use crate::constants::FAVORITE_FILL_KEY;
use crate::constants::FAVORITE_FRAYING_KEY;
use crate::constants::FAVORITE_ID_KEY;
use crate::constants::FAVORITE_MISSING_VALUE;
use crate::constants::FAVORITE_MODE_KEY;
use crate::constants::FAVORITE_RESOLVE_KEY;
use crate::constants::FAVORITE_SAVED_KEY;
use crate::constants::FAVORITE_SPEED_KEY;
use crate::constants::FAVORITE_SPREAD_KEY;
use crate::constants::FAVORITE_TAIL_SPEED_KEY;
use crate::constants::FAVORITE_WAVE_PERCENT_KEY;
use crate::constants::FAVORITE_WIDTH_KEY;
use crate::constants::FAVORITES_ARRAY_KEY;

/// Stable identity for one favorite row.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct FavoriteId(Uuid);

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
            saved: timestamp_at_file_precision(saved),
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

/// The file key and spelling that prevented a favorite row from being recognized.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnrecognizedFavoriteValue {
    /// TOML key whose value was missing, malformed, or unknown.
    pub(crate) key:      String,
    /// Value spelling found in the file, or `<missing>` when absent.
    pub(crate) spelling: String,
}

impl UnrecognizedFavoriteValue {
    fn new(key: &str, spelling: impl Into<String>) -> Self {
        Self {
            key:      key.to_string(),
            spelling: spelling.into(),
        }
    }
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

    pub(super) fn serialize(&self) -> Result<String, toml::ser::Error> {
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
            .map(|(raw_table_index, table)| match recognize_favorite(table) {
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
            })
            .collect();
        self.recognitions.sort_by(compare_recognitions);
    }

    pub(super) fn push(&mut self, candidate: &Favorite) -> FavoriteSaveOutcome {
        let existing_index = self.tables.iter().position(|table| {
            matches!(
                recognize_favorite(table),
                Ok(favorite) if favorite.settings == candidate.settings
            )
        });
        let outcome = if let Some(index) = existing_index {
            self.tables[index].insert(
                FAVORITE_SAVED_KEY.to_string(),
                Value::String(timestamp_spelling(candidate.saved)),
            );
            FavoriteSaveOutcome::Refreshed
        } else {
            self.tables.push(table_from_favorite(candidate));
            FavoriteSaveOutcome::Added
        };
        self.refresh_recognitions();
        outcome
    }

    pub(super) fn remove_recognized(&mut self, favorite_id: FavoriteId) {
        let row = self.tables.iter().position(|table| {
            matches!(recognize_favorite(table), Ok(Favorite { id, .. }) if id == favorite_id)
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

enum FavoriteValueRecognition<T> {
    Recognized(T),
    Unrecognized(UnrecognizedFavoriteValue),
}

impl<T> FavoriteValueRecognition<T> {
    fn and_then<U>(
        self,
        recognize: impl FnOnce(T) -> FavoriteValueRecognition<U>,
    ) -> FavoriteValueRecognition<U> {
        match self {
            Self::Recognized(value) => recognize(value),
            Self::Unrecognized(value) => FavoriteValueRecognition::Unrecognized(value),
        }
    }

    fn into_result(self) -> Result<T, UnrecognizedFavoriteValue> {
        match self {
            Self::Recognized(value) => Ok(value),
            Self::Unrecognized(value) => Err(value),
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

fn recognize_favorite(table: &Table) -> Result<Favorite, UnrecognizedFavoriteValue> {
    let id = recognize_favorite_id(table).into_result()?;
    let saved = recognize_saved(table).into_result()?;
    let attract_mode = recognize_attract_mode(table).into_result()?;
    let settings = match attract_mode {
        AttractMode::MovingBand => AttractSettings::MovingBand(recognize_band_settings(table)?),
        AttractMode::MovingText => AttractSettings::MovingText(recognize_text_settings(table)?),
        AttractMode::Pixelate => AttractSettings::Pixelate(recognize_pixel_settings(table)?),
    };
    Ok(Favorite {
        id,
        saved,
        settings,
    })
}

fn recognize_band_settings(table: &Table) -> Result<BandSettings, UnrecognizedFavoriteValue> {
    Ok(BandSettings {
        direction:  recognize_band_direction(table).into_result()?,
        width:      recognize_u32(table, FAVORITE_WIDTH_KEY).into_result()?,
        speed:      recognize_u32(table, FAVORITE_SPEED_KEY).into_result()?,
        tail_speed: recognize_u32(table, FAVORITE_TAIL_SPEED_KEY).into_result()?,
        fraying:    recognize_band_fraying(table).into_result()?,
    })
}

fn recognize_text_settings(table: &Table) -> Result<TextSettings, UnrecognizedFavoriteValue> {
    Ok(TextSettings {
        direction: recognize_band_direction(table).into_result()?,
        speed:     recognize_u32(table, FAVORITE_SPEED_KEY).into_result()?,
        spread:    recognize_u32(table, FAVORITE_SPREAD_KEY).into_result()?,
        drift:     recognize_text_drift(table).into_result()?,
        fill:      recognize_text_fill(table).into_result()?,
    })
}

fn recognize_pixel_settings(table: &Table) -> Result<PixelSettings, UnrecognizedFavoriteValue> {
    Ok(PixelSettings {
        direction:     recognize_band_direction(table).into_result()?,
        speed:         recognize_u32(table, FAVORITE_SPEED_KEY).into_result()?,
        wave_percent:  recognize_u32(table, FAVORITE_WAVE_PERCENT_KEY).into_result()?,
        block_columns: recognize_u32(table, FAVORITE_BLOCK_COLUMNS_KEY).into_result()?,
        resolve:       recognize_pixel_resolve(table).into_result()?,
        fill:          recognize_pixel_fill(table).into_result()?,
    })
}

fn recognize_favorite_id(table: &Table) -> FavoriteValueRecognition<FavoriteId> {
    recognize_string(table, FAVORITE_ID_KEY).and_then(|spelling| {
        Uuid::parse_str(spelling).map_or_else(
            |_| unrecognized(FAVORITE_ID_KEY, spelling),
            |uuid| FavoriteValueRecognition::Recognized(FavoriteId(uuid)),
        )
    })
}

fn recognize_saved(table: &Table) -> FavoriteValueRecognition<DateTime<FixedOffset>> {
    recognize_string(table, FAVORITE_SAVED_KEY).and_then(|spelling| {
        DateTime::parse_from_rfc3339(spelling).map_or_else(
            |_| unrecognized(FAVORITE_SAVED_KEY, spelling),
            FavoriteValueRecognition::Recognized,
        )
    })
}

fn recognize_string<'a>(table: &'a Table, key: &str) -> FavoriteValueRecognition<&'a str> {
    match table.get(key) {
        Some(Value::String(value)) => FavoriteValueRecognition::Recognized(value),
        Some(value) => unrecognized(key, value.to_string()),
        None => unrecognized(key, FAVORITE_MISSING_VALUE),
    }
}

fn recognize_u32(table: &Table, key: &str) -> FavoriteValueRecognition<u32> {
    match table.get(key) {
        Some(Value::Integer(value)) => u32::try_from(*value).map_or_else(
            |_| unrecognized(key, value.to_string()),
            FavoriteValueRecognition::Recognized,
        ),
        Some(value) => unrecognized(key, value.to_string()),
        None => unrecognized(key, FAVORITE_MISSING_VALUE),
    }
}

fn unrecognized<T>(key: &str, spelling: impl Into<String>) -> FavoriteValueRecognition<T> {
    FavoriteValueRecognition::Unrecognized(UnrecognizedFavoriteValue::new(key, spelling))
}

const fn attract_mode_name(attract_mode: AttractMode) -> &'static str {
    match attract_mode {
        AttractMode::MovingBand => "moving_band",
        AttractMode::MovingText => "moving_text",
        AttractMode::Pixelate => "pixelate",
    }
}

fn recognize_attract_mode(table: &Table) -> FavoriteValueRecognition<AttractMode> {
    recognize_string(table, FAVORITE_MODE_KEY).and_then(|spelling| match spelling {
        "moving_band" => FavoriteValueRecognition::Recognized(AttractMode::MovingBand),
        "moving_text" => FavoriteValueRecognition::Recognized(AttractMode::MovingText),
        "pixelate" => FavoriteValueRecognition::Recognized(AttractMode::Pixelate),
        _ => unrecognized(FAVORITE_MODE_KEY, spelling),
    })
}

const fn band_direction_name(direction: BandDirection) -> &'static str {
    match direction {
        BandDirection::Left => "left",
        BandDirection::Right => "right",
        BandDirection::Up => "up",
        BandDirection::Down => "down",
    }
}

fn recognize_band_direction(table: &Table) -> FavoriteValueRecognition<BandDirection> {
    recognize_string(table, FAVORITE_DIRECTION_KEY).and_then(|spelling| match spelling {
        "left" => FavoriteValueRecognition::Recognized(BandDirection::Left),
        "right" => FavoriteValueRecognition::Recognized(BandDirection::Right),
        "up" => FavoriteValueRecognition::Recognized(BandDirection::Up),
        "down" => FavoriteValueRecognition::Recognized(BandDirection::Down),
        _ => unrecognized(FAVORITE_DIRECTION_KEY, spelling),
    })
}

const fn band_fraying_name(fraying: BandFraying) -> &'static str {
    match fraying {
        BandFraying::Trailing => "trailing",
        BandFraying::Both => "both",
        BandFraying::Leading => "leading",
        BandFraying::Neither => "neither",
    }
}

fn recognize_band_fraying(table: &Table) -> FavoriteValueRecognition<BandFraying> {
    recognize_string(table, FAVORITE_FRAYING_KEY).and_then(|spelling| match spelling {
        "trailing" => FavoriteValueRecognition::Recognized(BandFraying::Trailing),
        "both" => FavoriteValueRecognition::Recognized(BandFraying::Both),
        "leading" => FavoriteValueRecognition::Recognized(BandFraying::Leading),
        "neither" => FavoriteValueRecognition::Recognized(BandFraying::Neither),
        _ => unrecognized(FAVORITE_FRAYING_KEY, spelling),
    })
}

const fn text_drift_name(drift: TextDrift) -> &'static str {
    match drift {
        TextDrift::Together => "together",
        TextDrift::Apart => "apart",
    }
}

fn recognize_text_drift(table: &Table) -> FavoriteValueRecognition<TextDrift> {
    recognize_string(table, FAVORITE_DRIFT_KEY).and_then(|spelling| match spelling {
        "together" => FavoriteValueRecognition::Recognized(TextDrift::Together),
        "apart" => FavoriteValueRecognition::Recognized(TextDrift::Apart),
        _ => unrecognized(FAVORITE_DRIFT_KEY, spelling),
    })
}

const fn text_fill_name(fill: TextFill) -> &'static str {
    match fill {
        TextFill::Bars => "bars",
        TextFill::Glyphs => "glyphs",
    }
}

fn recognize_text_fill(table: &Table) -> FavoriteValueRecognition<TextFill> {
    recognize_string(table, FAVORITE_FILL_KEY).and_then(|spelling| match spelling {
        "bars" => FavoriteValueRecognition::Recognized(TextFill::Bars),
        "glyphs" => FavoriteValueRecognition::Recognized(TextFill::Glyphs),
        _ => unrecognized(FAVORITE_FILL_KEY, spelling),
    })
}

const fn pixel_resolve_name(resolve: PixelResolve) -> &'static str {
    match resolve {
        PixelResolve::Blend => "blend",
        PixelResolve::Step => "step",
        PixelResolve::Scatter => "scatter",
    }
}

fn recognize_pixel_resolve(table: &Table) -> FavoriteValueRecognition<PixelResolve> {
    recognize_string(table, FAVORITE_RESOLVE_KEY).and_then(|spelling| match spelling {
        "blend" => FavoriteValueRecognition::Recognized(PixelResolve::Blend),
        "step" => FavoriteValueRecognition::Recognized(PixelResolve::Step),
        "scatter" => FavoriteValueRecognition::Recognized(PixelResolve::Scatter),
        _ => unrecognized(FAVORITE_RESOLVE_KEY, spelling),
    })
}

const fn pixel_fill_name(fill: PixelFill) -> &'static str {
    match fill {
        PixelFill::Solid => "solid",
        PixelFill::Shades => "shades",
    }
}

fn recognize_pixel_fill(table: &Table) -> FavoriteValueRecognition<PixelFill> {
    recognize_string(table, FAVORITE_FILL_KEY).and_then(|spelling| match spelling {
        "solid" => FavoriteValueRecognition::Recognized(PixelFill::Solid),
        "shades" => FavoriteValueRecognition::Recognized(PixelFill::Shades),
        _ => unrecognized(FAVORITE_FILL_KEY, spelling),
    })
}

fn table_from_favorite(favorite: &Favorite) -> Table {
    let mut table = Table::new();
    insert_string(&mut table, FAVORITE_ID_KEY, favorite.id.to_string());
    insert_string(
        &mut table,
        FAVORITE_SAVED_KEY,
        timestamp_spelling(favorite.saved),
    );
    insert_string(
        &mut table,
        FAVORITE_MODE_KEY,
        attract_mode_name(favorite.settings.mode()),
    );
    match favorite.settings {
        AttractSettings::MovingBand(settings) => {
            insert_string(
                &mut table,
                FAVORITE_DIRECTION_KEY,
                band_direction_name(settings.direction),
            );
            insert_u32(&mut table, FAVORITE_WIDTH_KEY, settings.width);
            insert_u32(&mut table, FAVORITE_SPEED_KEY, settings.speed);
            insert_u32(&mut table, FAVORITE_TAIL_SPEED_KEY, settings.tail_speed);
            insert_string(
                &mut table,
                FAVORITE_FRAYING_KEY,
                band_fraying_name(settings.fraying),
            );
        },
        AttractSettings::MovingText(settings) => {
            insert_string(
                &mut table,
                FAVORITE_DIRECTION_KEY,
                band_direction_name(settings.direction),
            );
            insert_u32(&mut table, FAVORITE_SPEED_KEY, settings.speed);
            insert_u32(&mut table, FAVORITE_SPREAD_KEY, settings.spread);
            insert_string(
                &mut table,
                FAVORITE_DRIFT_KEY,
                text_drift_name(settings.drift),
            );
            insert_string(&mut table, FAVORITE_FILL_KEY, text_fill_name(settings.fill));
        },
        AttractSettings::Pixelate(settings) => {
            insert_string(
                &mut table,
                FAVORITE_DIRECTION_KEY,
                band_direction_name(settings.direction),
            );
            insert_u32(&mut table, FAVORITE_SPEED_KEY, settings.speed);
            insert_u32(&mut table, FAVORITE_WAVE_PERCENT_KEY, settings.wave_percent);
            insert_u32(
                &mut table,
                FAVORITE_BLOCK_COLUMNS_KEY,
                settings.block_columns,
            );
            insert_string(
                &mut table,
                FAVORITE_RESOLVE_KEY,
                pixel_resolve_name(settings.resolve),
            );
            insert_string(
                &mut table,
                FAVORITE_FILL_KEY,
                pixel_fill_name(settings.fill),
            );
        },
    }
    table
}

fn insert_string(table: &mut Table, key: &str, value: impl Into<String>) {
    table.insert(key.to_string(), Value::String(value.into()));
}

fn insert_u32(table: &mut Table, key: &str, value: u32) {
    table.insert(key.to_string(), Value::Integer(i64::from(value)));
}

fn timestamp_spelling(timestamp: DateTime<FixedOffset>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Millis, false)
}

fn timestamp_at_file_precision(timestamp: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    let nanoseconds = timestamp.nanosecond() / 1_000_000 * 1_000_000;
    timestamp.with_nanosecond(nanoseconds).unwrap_or(timestamp)
}

#[cfg(test)]
pub(crate) fn parse_rows_for_overlay_test(text: &str) -> Result<FavoriteRows, String> {
    FavoriteRows::parse(text)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    const FIRST_ID: &str = "01a03f5e-9c14-7b41-8a02-1de4c7c9b330";
    const FIRST_SAVED: &str = "2026-08-26T09:02:44.870-07:00";
    const SECOND_SAVED: &str = "2026-08-26T14:31:05.412-07:00";

    fn recognized_value<T>(recognition: FavoriteValueRecognition<T>) -> T {
        match recognition {
            FavoriteValueRecognition::Recognized(value) => value,
            FavoriteValueRecognition::Unrecognized(value) => {
                panic!("expected recognized value, got {value:?}")
            },
        }
    }

    fn string_table(key: &str, spelling: &str) -> Table {
        let mut table = Table::new();
        insert_string(&mut table, key, spelling);
        table
    }

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

    #[test]
    fn every_enum_variant_round_trips_through_its_file_spelling() {
        for attract_mode in [
            AttractMode::MovingBand,
            AttractMode::MovingText,
            AttractMode::Pixelate,
        ] {
            let table = string_table(FAVORITE_MODE_KEY, attract_mode_name(attract_mode));
            assert_eq!(
                recognized_value(recognize_attract_mode(&table)),
                attract_mode
            );
        }
        for direction in [
            BandDirection::Left,
            BandDirection::Right,
            BandDirection::Up,
            BandDirection::Down,
        ] {
            let table = string_table(FAVORITE_DIRECTION_KEY, band_direction_name(direction));
            assert_eq!(
                recognized_value(recognize_band_direction(&table)),
                direction
            );
        }
        for fraying in [
            BandFraying::Trailing,
            BandFraying::Both,
            BandFraying::Leading,
            BandFraying::Neither,
        ] {
            let table = string_table(FAVORITE_FRAYING_KEY, band_fraying_name(fraying));
            assert_eq!(recognized_value(recognize_band_fraying(&table)), fraying);
        }
        for drift in [TextDrift::Together, TextDrift::Apart] {
            let table = string_table(FAVORITE_DRIFT_KEY, text_drift_name(drift));
            assert_eq!(recognized_value(recognize_text_drift(&table)), drift);
        }
        for fill in [TextFill::Bars, TextFill::Glyphs] {
            let table = string_table(FAVORITE_FILL_KEY, text_fill_name(fill));
            assert_eq!(recognized_value(recognize_text_fill(&table)), fill);
        }
        for resolve in [
            PixelResolve::Blend,
            PixelResolve::Step,
            PixelResolve::Scatter,
        ] {
            let table = string_table(FAVORITE_RESOLVE_KEY, pixel_resolve_name(resolve));
            assert_eq!(recognized_value(recognize_pixel_resolve(&table)), resolve);
        }
        for fill in [PixelFill::Solid, PixelFill::Shades] {
            let table = string_table(FAVORITE_FILL_KEY, pixel_fill_name(fill));
            assert_eq!(recognized_value(recognize_pixel_fill(&table)), fill);
        }
    }
}
