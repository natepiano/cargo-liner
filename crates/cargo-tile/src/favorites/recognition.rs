//! TOML codec for favorite rows: file spellings recognized into typed values, and
//! typed values written back out as tables.

use chrono::DateTime;
use chrono::FixedOffset;
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

use super::rows::AttractSettings;
use super::rows::Favorite;
use super::rows::FavoriteId;
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

/// The file key and spelling that prevented a favorite row from being recognized.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnrecognizedFavoriteValue {
    /// TOML key whose value was missing, malformed, or unknown.
    pub(crate) key:      String,
    /// Value spelling found in the file, or `<missing>` when absent.
    pub(crate) spelling: String,
}

impl UnrecognizedFavoriteValue {
    pub(super) fn new(key: &str, spelling: impl Into<String>) -> Self {
        Self {
            key:      key.to_string(),
            spelling: spelling.into(),
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

pub(super) fn recognize_favorite(table: &Table) -> Result<Favorite, UnrecognizedFavoriteValue> {
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

pub(super) fn table_from_favorite(favorite: &Favorite) -> Table {
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

pub(super) fn timestamp_spelling(timestamp: DateTime<FixedOffset>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Millis, false)
}

pub(super) fn timestamp_at_file_precision(
    timestamp: DateTime<FixedOffset>,
) -> DateTime<FixedOffset> {
    let nanoseconds = timestamp.nanosecond() / 1_000_000 * 1_000_000;
    timestamp.with_nanosecond(nanoseconds).unwrap_or(timestamp)
}

#[cfg(test)]
#[expect(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use super::*;

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
