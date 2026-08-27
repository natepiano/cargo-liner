//! Lossless persistence for attract-screen parameter favorites.

use std::cmp::Ordering;
use std::error::Error;
use std::fmt;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::thread;

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
use crate::config;
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
use crate::constants::FAVORITES_FILENAME;
use crate::constants::FAVORITES_LOCK_RETRY_ATTEMPTS;
use crate::constants::FAVORITES_LOCK_RETRY_DELAY;
use crate::constants::FAVORITES_LOCK_SUFFIX;
use crate::constants::FAVORITES_TEMP_SUFFIX;

/// Stable identity for one favorite row.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct FavoriteId(Uuid);

impl fmt::Display for FavoriteId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(formatter) }
}

/// Parameters saved for one attract-screen mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FavoriteSettings {
    /// Moving-band parameters.
    MovingBand(BandSettings),
    /// Moving-text parameters.
    MovingText(TextSettings),
    /// Pixelate parameters.
    Pixelate(PixelSettings),
}

impl FavoriteSettings {
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
    pub(crate) settings: FavoriteSettings,
}

impl Favorite {
    fn now(settings: FavoriteSettings) -> Self {
        let saved = Local::now().fixed_offset();
        Self {
            id: FavoriteId(Uuid::now_v7()),
            saved: timestamp_at_file_precision(saved),
            settings,
        }
    }
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

/// Typed recognition result for one raw favorite table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FavoriteRowRecognition {
    /// The table contains one complete, recognized favorite.
    Recognized(Favorite),
    /// The table is retained but omitted from display and loading.
    Unrecognized(UnrecognizedFavoriteValue),
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
                FavoriteRowRecognition::Unrecognized(_) => None,
            })
    }

    fn parse(text: &str) -> Result<Self, String> {
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

    fn serialize(&self) -> Result<String, toml::ser::Error> {
        let mut table = self.additional_fields.clone();
        table.insert(
            FAVORITES_ARRAY_KEY.to_string(),
            Value::Array(self.tables.iter().cloned().map(Value::Table).collect()),
        );
        toml::to_string_pretty(&table)
    }

    fn refresh_recognitions(&mut self) {
        self.recognitions = self.tables.iter().map(recognize_favorite).collect();
        self.recognitions.sort_by(compare_recognitions);
    }

    fn push(&mut self, candidate: Favorite) -> Favorite {
        let existing = self.tables.iter().enumerate().find_map(|(index, table)| {
            match recognize_favorite(table) {
                FavoriteRowRecognition::Recognized(favorite)
                    if favorite.settings == candidate.settings =>
                {
                    Some((index, favorite))
                },
                FavoriteRowRecognition::Recognized(_) | FavoriteRowRecognition::Unrecognized(_) => {
                    None
                },
            }
        });
        let saved = candidate.saved;
        let favorite = if let Some((index, existing)) = existing {
            self.tables[index].insert(
                FAVORITE_SAVED_KEY.to_string(),
                Value::String(timestamp_spelling(saved)),
            );
            Favorite {
                id: existing.id,
                saved,
                settings: candidate.settings,
            }
        } else {
            self.tables.push(table_from_favorite(&candidate));
            candidate
        };
        self.refresh_recognitions();
        favorite
    }

    fn remove(&mut self, favorite_id: FavoriteId) {
        let row = self.tables.iter().position(|table| {
            matches!(
                recognize_favorite(table),
                FavoriteRowRecognition::Recognized(Favorite { id, .. }) if id == favorite_id
            )
        });
        if let Some(index) = row {
            self.tables.remove(index);
            self.refresh_recognitions();
        }
    }
}

/// Result of resolving and reading the favorites file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FavoritesFileState {
    /// The operating system did not provide a configuration directory.
    LocationUnavailable,
    /// No favorites file exists yet.
    Missing {
        /// Path where the first favorite will create the file.
        path: PathBuf,
    },
    /// The favorites file was read and parsed.
    Loaded {
        /// Path the rows came from.
        path: PathBuf,
        /// Lossless raw rows and their typed interpretations.
        rows: FavoriteRows,
    },
    /// The file was read but its TOML or favorite-table structure did not parse.
    Unparseable {
        /// Path containing the invalid content.
        path:  PathBuf,
        /// Parse failure text.
        error: String,
    },
    /// The file exists but could not be read.
    Unreadable {
        /// Path that could not be read.
        path:  PathBuf,
        /// File-system failure text.
        error: String,
    },
}

/// Failure from a locked favorites mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FavoritesMutationError {
    /// The operating system did not provide a configuration directory.
    LocationUnavailable,
    /// Existing favorites could not be parsed, so the file was not changed.
    Unparseable {
        /// Path containing the invalid content.
        path:  PathBuf,
        /// Parse failure text.
        error: String,
    },
    /// Existing favorites could not be read, so the file was not changed.
    Unreadable {
        /// Path that could not be read.
        path:  PathBuf,
        /// File-system failure text.
        error: String,
    },
    /// The sibling lock could not be acquired.
    LockUnavailable {
        /// Lock-file path.
        path:  PathBuf,
        /// Lock failure text.
        error: String,
    },
    /// The directory, temporary file, or atomic rename could not be written.
    WriteFailed {
        /// Favorites path the mutation was intended to update.
        path:  PathBuf,
        /// Write failure text.
        error: String,
    },
}

impl fmt::Display for FavoritesMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocationUnavailable => write!(
                formatter,
                "no OS config directory: cannot write {FAVORITES_FILENAME}"
            ),
            Self::Unparseable { path, error } => {
                write!(
                    formatter,
                    "{}: cannot update unparseable favorites: {error}",
                    path.display()
                )
            },
            Self::Unreadable { path, error } => {
                write!(
                    formatter,
                    "{}: cannot read favorites: {error}",
                    path.display()
                )
            },
            Self::LockUnavailable { path, error } => {
                write!(
                    formatter,
                    "{}: cannot acquire favorites lock: {error}",
                    path.display()
                )
            },
            Self::WriteFailed { path, error } => {
                write!(
                    formatter,
                    "{}: cannot write favorites: {error}",
                    path.display()
                )
            },
        }
    }
}

impl Error for FavoritesMutationError {}

/// Read the configured favorites file without replacing malformed or unreadable content.
#[must_use]
pub(crate) fn load() -> FavoritesFileState {
    load_from(FavoritesLocation::from(config::favorites_path()))
}

/// Save one parameter set, updating an identical row's timestamp instead of duplicating it.
///
/// # Errors
///
/// Returns the read-only file state or the lock, directory, serialization, or write failure.
pub(crate) fn push(settings: FavoriteSettings) -> Result<Favorite, FavoritesMutationError> {
    push_to_location(
        FavoritesLocation::from(config::favorites_path()),
        Favorite::now(settings),
    )
}

/// Remove the row with `favorite_id` after re-reading it under the file lock.
///
/// # Errors
///
/// Returns the read-only file state or the lock, directory, serialization, or write failure.
pub(crate) fn remove(favorite_id: FavoriteId) -> Result<(), FavoritesMutationError> {
    remove_from_location(
        FavoritesLocation::from(config::favorites_path()),
        favorite_id,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FavoritesLocation {
    Unavailable,
    Path(PathBuf),
}

impl From<Option<PathBuf>> for FavoritesLocation {
    fn from(path: Option<PathBuf>) -> Self { path.map_or(Self::Unavailable, Self::Path) }
}

enum FavoritesReadOutcome {
    Missing,
    Loaded(FavoriteRows),
    Unparseable(String),
    Unreadable(String),
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

struct FavoritesLock {
    file: File,
}

impl Drop for FavoritesLock {
    fn drop(&mut self) { drop(self.file.unlock()); }
}

fn load_from(location: FavoritesLocation) -> FavoritesFileState {
    let FavoritesLocation::Path(path) = location else {
        return FavoritesFileState::LocationUnavailable;
    };
    match read_rows(&path) {
        FavoritesReadOutcome::Missing => FavoritesFileState::Missing { path },
        FavoritesReadOutcome::Loaded(rows) => FavoritesFileState::Loaded { path, rows },
        FavoritesReadOutcome::Unparseable(error) => FavoritesFileState::Unparseable { path, error },
        FavoritesReadOutcome::Unreadable(error) => FavoritesFileState::Unreadable { path, error },
    }
}

fn push_to_location(
    location: FavoritesLocation,
    favorite: Favorite,
) -> Result<Favorite, FavoritesMutationError> {
    edit_at_location(location, |rows| rows.push(favorite))
}

fn remove_from_location(
    location: FavoritesLocation,
    favorite_id: FavoriteId,
) -> Result<(), FavoritesMutationError> {
    edit_at_location(location, |rows| rows.remove(favorite_id))
}

fn edit_at_location<T>(
    location: FavoritesLocation,
    edit: impl FnOnce(&mut FavoriteRows) -> T,
) -> Result<T, FavoritesMutationError> {
    let FavoritesLocation::Path(path) = location else {
        return Err(FavoritesMutationError::LocationUnavailable);
    };
    let parent = path
        .parent()
        .ok_or_else(|| FavoritesMutationError::WriteFailed {
            path:  path.clone(),
            error: "favorites path has no parent directory".to_string(),
        })?;
    fs::create_dir_all(parent).map_err(|error| FavoritesMutationError::WriteFailed {
        path:  path.clone(),
        error: error.to_string(),
    })?;
    let lock_path = sibling_path(&path, FAVORITES_LOCK_SUFFIX);
    let _favorites_lock = acquire_lock(&lock_path)?;
    let mut rows = match read_rows(&path) {
        FavoritesReadOutcome::Missing => FavoriteRows::default(),
        FavoritesReadOutcome::Loaded(rows) => rows,
        FavoritesReadOutcome::Unparseable(error) => {
            return Err(FavoritesMutationError::Unparseable { path, error });
        },
        FavoritesReadOutcome::Unreadable(error) => {
            return Err(FavoritesMutationError::Unreadable { path, error });
        },
    };
    let result = edit(&mut rows);
    atomic_replace(&path, &rows)?;
    Ok(result)
}

fn read_rows(path: &Path) -> FavoritesReadOutcome {
    match fs::read_to_string(path) {
        Ok(text) => FavoriteRows::parse(&text).map_or_else(
            FavoritesReadOutcome::Unparseable,
            FavoritesReadOutcome::Loaded,
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => FavoritesReadOutcome::Missing,
        Err(error) => FavoritesReadOutcome::Unreadable(error.to_string()),
    }
}

fn acquire_lock(path: &Path) -> Result<FavoritesLock, FavoritesMutationError> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| lock_error(path, error))?;
    let mut retries_remaining = FAVORITES_LOCK_RETRY_ATTEMPTS;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(FavoritesLock { file }),
            Err(TryLockError::WouldBlock) if retries_remaining > 0 => {
                retries_remaining -= 1;
                thread::sleep(FAVORITES_LOCK_RETRY_DELAY);
            },
            Err(TryLockError::WouldBlock) => {
                return Err(lock_error(path, "lock remained held"));
            },
            Err(TryLockError::Error(error)) => return Err(lock_error(path, error)),
        }
    }
}

fn lock_error(path: &Path, error: impl fmt::Display) -> FavoritesMutationError {
    FavoritesMutationError::LockUnavailable {
        path:  path.to_path_buf(),
        error: error.to_string(),
    }
}

fn atomic_replace(path: &Path, rows: &FavoriteRows) -> Result<(), FavoritesMutationError> {
    let text = rows
        .serialize()
        .map_err(|error| FavoritesMutationError::WriteFailed {
            path:  path.to_path_buf(),
            error: error.to_string(),
        })?;
    let temporary_path = sibling_path(path, FAVORITES_TEMP_SUFFIX);
    if let Err(error) = write_temporary_file(&temporary_path, text.as_bytes()) {
        drop(fs::remove_file(&temporary_path));
        return Err(FavoritesMutationError::WriteFailed {
            path:  path.to_path_buf(),
            error: error.to_string(),
        });
    }
    if let Err(error) = fs::rename(&temporary_path, path) {
        drop(fs::remove_file(&temporary_path));
        return Err(FavoritesMutationError::WriteFailed {
            path:  path.to_path_buf(),
            error: error.to_string(),
        });
    }
    Ok(())
}

fn write_temporary_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut sibling = path.as_os_str().to_os_string();
    sibling.push(suffix);
    PathBuf::from(sibling)
}

fn compare_recognitions(left: &FavoriteRowRecognition, right: &FavoriteRowRecognition) -> Ordering {
    match (left, right) {
        (FavoriteRowRecognition::Recognized(left), FavoriteRowRecognition::Recognized(right)) => {
            attract_mode_order(left.settings.mode())
                .cmp(&attract_mode_order(right.settings.mode()))
                .then_with(|| right.saved.cmp(&left.saved))
                .then_with(|| right.id.cmp(&left.id))
        },
        (FavoriteRowRecognition::Recognized(_), FavoriteRowRecognition::Unrecognized(_)) => {
            Ordering::Less
        },
        (FavoriteRowRecognition::Unrecognized(_), FavoriteRowRecognition::Recognized(_)) => {
            Ordering::Greater
        },
        (FavoriteRowRecognition::Unrecognized(_), FavoriteRowRecognition::Unrecognized(_)) => {
            Ordering::Equal
        },
    }
}

const fn attract_mode_order(attract_mode: AttractMode) -> u8 {
    match attract_mode {
        AttractMode::MovingBand => 0,
        AttractMode::MovingText => 1,
        AttractMode::Pixelate => 2,
    }
}

fn recognize_favorite(table: &Table) -> FavoriteRowRecognition {
    let favorite = (|| {
        let id = recognize_favorite_id(table).into_result()?;
        let saved = recognize_saved(table).into_result()?;
        let attract_mode = recognize_attract_mode(table).into_result()?;
        let settings = match attract_mode {
            AttractMode::MovingBand => {
                FavoriteSettings::MovingBand(recognize_band_settings(table)?)
            },
            AttractMode::MovingText => {
                FavoriteSettings::MovingText(recognize_text_settings(table)?)
            },
            AttractMode::Pixelate => FavoriteSettings::Pixelate(recognize_pixel_settings(table)?),
        };
        Ok(Favorite {
            id,
            saved,
            settings,
        })
    })();
    match favorite {
        Ok(favorite) => FavoriteRowRecognition::Recognized(favorite),
        Err(value) => FavoriteRowRecognition::Unrecognized(value),
    }
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
        FavoriteSettings::MovingBand(settings) => {
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
        FavoriteSettings::MovingText(settings) => {
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
        FavoriteSettings::Pixelate(settings) => {
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
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    const FIRST_ID: &str = "01a03f5e-9c14-7b41-8a02-1de4c7c9b330";
    const FIRST_SAVED: &str = "2026-08-26T09:02:44.870-07:00";
    const SECOND_ID: &str = "01a03f60-2e8b-77c2-858f-476ee413d81c";
    const SECOND_SAVED: &str = "2026-08-26T14:31:05.412-07:00";

    fn favorites_path(directory: &TempDir) -> PathBuf { directory.path().join(FAVORITES_FILENAME) }

    fn location(path: &Path) -> FavoritesLocation { FavoritesLocation::Path(path.to_path_buf()) }

    /// Rewrite the file under its lock without changing a row, exercising the
    /// read-modify-write path on its own.
    fn save_to_location(location: FavoritesLocation) -> Result<(), FavoritesMutationError> {
        edit_at_location(location, |_| ())
    }

    fn write_favorites(path: &Path, text: &str) {
        fs::write(path, text).expect("favorite fixture should be written");
    }

    fn favorite_id(spelling: &str) -> FavoriteId {
        FavoriteId(Uuid::parse_str(spelling).expect("favorite id should parse"))
    }

    fn saved(spelling: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(spelling).expect("favorite timestamp should parse")
    }

    fn loaded_rows(state: &FavoritesFileState) -> &FavoriteRows {
        match state {
            FavoritesFileState::Loaded { rows, .. } => rows,
            _ => panic!("expected loaded favorites, got {state:?}"),
        }
    }

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

    fn band_settings() -> FavoriteSettings {
        FavoriteSettings::MovingBand(BandSettings {
            direction:  BandDirection::Right,
            width:      12,
            speed:      40,
            tail_speed: 96,
            fraying:    BandFraying::Both,
        })
    }

    fn valid_favorites() -> &'static str {
        r#"[[favorite]]
id = "01a03f60-2e8b-77c2-858f-476ee413d81c"
saved = "2026-08-26T14:31:05.412-07:00"
mode = "pixelate"
direction = "left"
speed = 24
wave_percent = 145
block_columns = 6
resolve = "scatter"
fill = "solid"

[[favorite]]
id = "01a03f5e-9c14-7b41-8a02-1de4c7c9b330"
saved = "2026-08-26T09:02:44.870-07:00"
mode = "moving_band"
direction = "right"
width = 12
speed = 40
tail_speed = 96
fraying = "both"
"#
    }

    #[test]
    fn list_survives_save_and_load_unchanged() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        write_favorites(&path, valid_favorites());

        let before = load_from(location(&path));
        save_to_location(location(&path)).expect("recognized favorites should save");
        let after = load_from(location(&path));

        assert_eq!(after, before);
        assert!(sibling_path(&path, FAVORITES_LOCK_SUFFIX).exists());
        assert!(!sibling_path(&path, FAVORITES_TEMP_SUFFIX).exists());
    }

    #[test]
    fn unknown_rows_and_keys_survive_save_and_delete() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        let text = format!(
            r#"[[favorite]]
id = "{FIRST_ID}"
saved = "{FIRST_SAVED}"
mode = "future_mode"
future_parameter = 88

[[favorite]]
id = "01a03f5f-9c14-7b41-8a02-1de4c7c9b331"
saved = "2026-08-26T10:02:44.870-07:00"
mode = "moving_band"
direction = "sideways"
width = 9
speed = 30
tail_speed = 72
fraying = "both"

[[favorite]]
id = "01a03f60-9c14-7b41-8a02-1de4c7c9b332"
saved = "2026-08-26T11:02:44.870-07:00"
mode = "moving_band"
direction = "left"
width = 10
speed = 32
tail_speed = 72
fraying = "leading"
future_glow = "amber"

[[favorite]]
id = "{SECOND_ID}"
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
        write_favorites(&path, &text);

        let state = load_from(location(&path));
        let rows = loaded_rows(&state);
        assert_eq!(rows.recognized().count(), 2);
        let unrecognized: Vec<_> = rows
            .iter()
            .filter_map(|recognition| match recognition {
                FavoriteRowRecognition::Recognized(_) => None,
                FavoriteRowRecognition::Unrecognized(value) => Some(value),
            })
            .collect();
        assert!(
            unrecognized
                .iter()
                .any(|value| { value.key == FAVORITE_MODE_KEY && value.spelling == "future_mode" })
        );
        assert!(
            unrecognized.iter().any(|value| {
                value.key == FAVORITE_DIRECTION_KEY && value.spelling == "sideways"
            })
        );

        save_to_location(location(&path)).expect("lossless favorites should save");
        let after_save = fs::read_to_string(&path).expect("saved favorites should be readable");
        assert!(after_save.contains("future_mode"));
        assert!(after_save.contains("future_parameter = 88"));
        assert!(after_save.contains("direction = \"sideways\""));
        assert!(after_save.contains("future_glow = \"amber\""));

        remove_from_location(location(&path), favorite_id(SECOND_ID))
            .expect("recognized favorite should be removed");
        let after_delete = fs::read_to_string(&path).expect("favorites should remain readable");
        assert!(after_delete.contains("future_mode"));
        assert!(after_delete.contains("future_parameter = 88"));
        assert!(after_delete.contains("direction = \"sideways\""));
        assert!(after_delete.contains("future_glow = \"amber\""));
        assert!(!after_delete.contains(SECOND_ID));
        assert_eq!(
            loaded_rows(&load_from(location(&path)))
                .recognized()
                .count(),
            1
        );
    }

    #[test]
    fn unparseable_file_is_read_only_and_carries_its_path() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        let truncated = "[[favorite]\nid = \"cut off\"\n";
        write_favorites(&path, truncated);

        let state = load_from(location(&path));
        assert!(matches!(
            state,
            FavoritesFileState::Unparseable { path: ref failed_path, .. } if failed_path == &path
        ));
        assert!(matches!(
            save_to_location(location(&path)),
            Err(FavoritesMutationError::Unparseable { path: ref failed_path, .. })
                if failed_path == &path
        ));
        assert!(matches!(
            remove_from_location(location(&path), favorite_id(FIRST_ID)),
            Err(FavoritesMutationError::Unparseable { path: ref failed_path, .. })
                if failed_path == &path
        ));
        assert_eq!(
            fs::read_to_string(&path).expect("invalid favorites should remain readable"),
            truncated
        );
        assert!(sibling_path(&path, FAVORITES_LOCK_SUFFIX).exists());
    }

    #[test]
    fn missing_file_loads_as_an_empty_state() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);

        assert_eq!(
            load_from(location(&path)),
            FavoritesFileState::Missing { path }
        );
    }

    #[test]
    fn unreadable_file_is_read_only_and_carries_its_path() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        fs::create_dir(&path).expect("directory fixture should be created");

        assert!(matches!(
            load_from(location(&path)),
            FavoritesFileState::Unreadable { path: ref failed_path, .. } if failed_path == &path
        ));
        assert!(matches!(
            save_to_location(location(&path)),
            Err(FavoritesMutationError::Unreadable { path: ref failed_path, .. })
                if failed_path == &path
        ));
        assert!(matches!(
            remove_from_location(location(&path), favorite_id(FIRST_ID)),
            Err(FavoritesMutationError::Unreadable { path: ref failed_path, .. })
                if failed_path == &path
        ));
        assert!(path.is_dir());
    }

    #[test]
    fn unavailable_location_refuses_save_push_and_delete() {
        let favorite = Favorite {
            id:       favorite_id(FIRST_ID),
            saved:    saved(FIRST_SAVED),
            settings: band_settings(),
        };

        assert_eq!(
            load_from(FavoritesLocation::Unavailable),
            FavoritesFileState::LocationUnavailable
        );
        assert_eq!(
            save_to_location(FavoritesLocation::Unavailable),
            Err(FavoritesMutationError::LocationUnavailable)
        );
        assert_eq!(
            push_to_location(FavoritesLocation::Unavailable, favorite),
            Err(FavoritesMutationError::LocationUnavailable)
        );
        assert_eq!(
            remove_from_location(FavoritesLocation::Unavailable, favorite_id(FIRST_ID)),
            Err(FavoritesMutationError::LocationUnavailable)
        );
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

    #[test]
    fn identical_settings_update_saved_time_without_changing_id() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        let first = Favorite {
            id:       favorite_id(FIRST_ID),
            saved:    saved(FIRST_SAVED),
            settings: band_settings(),
        };
        let second = Favorite {
            id:       favorite_id(SECOND_ID),
            saved:    saved(SECOND_SAVED),
            settings: band_settings(),
        };

        let inserted = push_to_location(location(&path), first.clone())
            .expect("first favorite should be written");
        let updated =
            push_to_location(location(&path), second).expect("identical favorite should update");

        assert_eq!(inserted, first);
        assert_eq!(updated.id, first.id);
        assert_eq!(updated.saved, saved(SECOND_SAVED));
        let state = load_from(location(&path));
        let favorites: Vec<_> = loaded_rows(&state).recognized().collect();
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0], &updated);
        let text = fs::read_to_string(path).expect("updated favorites should be readable");
        assert!(text.contains(FIRST_ID));
        assert!(!text.contains(SECOND_ID));
    }

    #[test]
    fn live_favorite_is_identical_after_file_round_trip() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        let favorite = Favorite::now(band_settings());

        let inserted =
            push_to_location(location(&path), favorite).expect("live favorite should be written");
        let state = load_from(location(&path));
        let loaded: Vec<_> = loaded_rows(&state).recognized().collect();

        assert_eq!(loaded, [&inserted]);
    }

    #[test]
    fn second_lock_fails_until_first_guard_drops() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = sibling_path(&favorites_path(&directory), FAVORITES_LOCK_SUFFIX);
        let first_lock = acquire_lock(&path).expect("first lock should be acquired");

        assert!(matches!(
            acquire_lock(&path),
            Err(FavoritesMutationError::LockUnavailable { .. })
        ));

        drop(first_lock);
        let second_lock =
            acquire_lock(&path).expect("second lock should be acquired after release");
        drop(second_lock);
        assert!(path.exists());
    }

    #[test]
    fn save_while_another_handle_holds_lock_preserves_file() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        push_to_location(
            location(&path),
            Favorite {
                id:       favorite_id(FIRST_ID),
                saved:    saved(FIRST_SAVED),
                settings: band_settings(),
            },
        )
        .expect("existing favorite should be written");
        let existing_contents =
            fs::read_to_string(&path).expect("existing favorites should be readable");
        let lock_path = sibling_path(&path, FAVORITES_LOCK_SUFFIX);
        let competing_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .expect("competing lock file should open");
        competing_file
            .try_lock()
            .expect("competing file should acquire the lock");

        assert!(matches!(
            save_to_location(location(&path)),
            Err(FavoritesMutationError::LockUnavailable { .. })
        ));
        assert_eq!(
            fs::read_to_string(path).expect("favorites should remain readable"),
            existing_contents
        );
    }
}
