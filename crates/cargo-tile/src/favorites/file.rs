use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::io;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::thread;

use tui_pane::KeySequence;

use super::UnrecognizedFavoriteRemovalLocator;
use super::rows::AttractSettings;
use super::rows::Favorite;
use super::rows::FavoriteId;
use super::rows::FavoriteRows;
use super::rows::FavoriteSaveOutcome;
use super::rows::UnrecognizedFavoriteRemoval;
use crate::config;
use crate::constants::FAVORITES_FILENAME;
use crate::constants::FAVORITES_LOCK_RETRY_ATTEMPTS;
use crate::constants::FAVORITES_LOCK_RETRY_DELAY;
use crate::constants::FAVORITES_LOCK_SUFFIX;
use crate::constants::FAVORITES_TEMP_SUFFIX;

/// A keymap lookup whose variants say whether the action can currently be invoked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ResolvedBinding {
    /// The action has a primary binding.
    Bound {
        /// TOML name of the action the key invokes.
        action_name: &'static str,
        /// Primary key sequence for the action.
        sequence:    KeySequence,
    },
    /// The action is deliberately unbound.
    Unbound {
        /// TOML name of the action the reader needs to bind.
        action_name: &'static str,
    },
}

impl ResolvedBinding {
    /// Resolve the primary binding for one named keymap action.
    pub(crate) fn for_action(action_name: &'static str, binding: Option<KeySequence>) -> Self {
        binding.map_or(Self::Unbound { action_name }, |sequence| Self::Bound {
            action_name,
            sequence,
        })
    }

    /// Compact label for the resolved key, or an empty label when it is unbound.
    pub(crate) fn display_short(&self) -> String {
        match self {
            Self::Bound { sequence, .. } => sequence.display_short(),
            Self::Unbound { .. } => String::new(),
        }
    }

    fn retry_phrase(&self, instruction: &str) -> String {
        match self {
            Self::Bound { sequence, .. } => {
                format!("press {} to {instruction}", sequence.display_short())
            },
            Self::Unbound { action_name } => format!("bind the {action_name} action first"),
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
    /// An unrecognized-row locator no longer matched exactly one raw table.
    UnrecognizedFavoriteChanged,
    /// The directory, temporary file, or atomic rename could not be written.
    WriteFailed {
        /// Favorites path the mutation was intended to update.
        path:  PathBuf,
        /// Write failure text.
        error: String,
    },
}

/// Identity of the recognized or unrecognized favorite row to remove.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FavoriteRemovalTarget {
    /// A recognized row named by its stable identifier.
    Recognized(FavoriteId),
    /// An unrecognized row named by its load-time raw-table locator.
    Unrecognized(UnrecognizedFavoriteRemovalLocator),
}

impl From<UnrecognizedFavoriteRemovalLocator> for FavoriteRemovalTarget {
    fn from(removal_locator: UnrecognizedFavoriteRemovalLocator) -> Self {
        Self::Unrecognized(removal_locator)
    }
}

/// Favorites-file mutation being reported to the reader.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FavoritesMutation {
    /// Saving the current attract parameters.
    Save,
    /// Deleting a saved favorite.
    Delete,
}

impl FavoritesMutation {
    const fn label(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::Delete => "deletion",
        }
    }
}

/// Usable instruction for retrying a refused favorites mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FavoritesRetryInstruction {
    /// Retry through one action that is available on the current surface.
    Press(ResolvedBinding),
    /// Reopen the favorites overlay before invoking its local retry action.
    ReopenThenPress {
        /// Binding that reopens the favorites overlay.
        open:  ResolvedBinding,
        /// Binding that retries the mutation inside the overlay.
        retry: ResolvedBinding,
    },
}

impl FavoritesRetryInstruction {
    fn sentence(&self, mutation: FavoritesMutation) -> String {
        match self {
            Self::Press(binding) => binding.retry_phrase("try again"),
            Self::ReopenThenPress { open, retry } => format!(
                "{}, then {}",
                open.retry_phrase("reopen favorites"),
                retry.retry_phrase(&format!("retry the {}", mutation.label())),
            ),
        }
    }
}

/// Explain a refused mutation, including a retry that works on the current surface.
#[must_use]
pub(crate) fn favorite_refusal_message(
    mutation: FavoritesMutation,
    retry: &FavoritesRetryInstruction,
    error: &FavoritesMutationError,
) -> String {
    match error {
        FavoritesMutationError::LockUnavailable { .. } => format!(
            "Favorites refused the {} because they are in use; {}. {error}",
            mutation.label(),
            retry.sentence(mutation)
        ),
        FavoritesMutationError::LocationUnavailable
        | FavoritesMutationError::Unparseable { .. }
        | FavoritesMutationError::Unreadable { .. }
        | FavoritesMutationError::UnrecognizedFavoriteChanged
        | FavoritesMutationError::WriteFailed { .. } => {
            format!("Favorites refused the {}: {error}", mutation.label())
        },
    }
}

impl Display for FavoritesMutationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
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
            Self::UnrecognizedFavoriteChanged => write!(
                formatter,
                "the unrecognized favorite changed after it was loaded"
            ),
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

/// Save one parameter set and report whether it added or refreshed a row.
///
/// # Errors
///
/// Returns the read-only file state or the lock, directory, serialization, or write failure.
pub(crate) fn push(
    settings: AttractSettings,
) -> Result<FavoriteSaveOutcome, FavoritesMutationError> {
    let favorite = Favorite::now(settings);
    push_to_location(FavoritesLocation::from(config::favorites_path()), &favorite)
}

/// Remove `target` after re-reading and re-verifying it under the file lock.
///
/// # Errors
///
/// Returns a stale unrecognized-row locator, read-only file state, or the lock, directory,
/// serialization, or write failure.
pub(crate) fn remove(target: FavoriteRemovalTarget) -> Result<(), FavoritesMutationError> {
    remove_from_location(FavoritesLocation::from(config::favorites_path()), target)
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
    favorite: &Favorite,
) -> Result<FavoriteSaveOutcome, FavoritesMutationError> {
    edit_at_location(location, |rows| Ok(rows.push(favorite)))
}

fn remove_from_location(
    location: FavoritesLocation,
    target: FavoriteRemovalTarget,
) -> Result<(), FavoritesMutationError> {
    edit_at_location(location, |rows| match target {
        FavoriteRemovalTarget::Recognized(favorite_id) => {
            rows.remove_recognized(favorite_id);
            Ok(())
        },
        FavoriteRemovalTarget::Unrecognized(removal_locator) => {
            match rows.remove_unrecognized(&removal_locator) {
                UnrecognizedFavoriteRemoval::Removed => Ok(()),
                UnrecognizedFavoriteRemoval::LocatorStale => {
                    Err(FavoritesMutationError::UnrecognizedFavoriteChanged)
                },
            }
        },
    })
}

fn edit_at_location<T>(
    location: FavoritesLocation,
    edit: impl FnOnce(&mut FavoriteRows) -> Result<T, FavoritesMutationError>,
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
    let result = edit(&mut rows)?;
    atomic_replace(&path, &rows)?;
    Ok(result)
}

fn read_rows(path: &Path) -> FavoritesReadOutcome {
    match fs::read_to_string(path) {
        Ok(text) => FavoriteRows::parse(&text).map_or_else(
            FavoritesReadOutcome::Unparseable,
            FavoritesReadOutcome::Loaded,
        ),
        Err(error) if error.kind() == ErrorKind::NotFound => FavoritesReadOutcome::Missing,
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

#[cfg(test)]
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::HashSet;
    use std::fs;

    use chrono::DateTime;
    use chrono::FixedOffset;
    use tempfile::TempDir;
    use tui_pane::BandDirection;
    use tui_pane::BandFraying;
    use tui_pane::BandSettings;
    use tui_pane::KeyBind;
    use uuid::Uuid;

    use super::super::rows::FavoriteRowRecognition;
    use super::*;
    use crate::constants::FAVORITE_DIRECTION_KEY;
    use crate::constants::FAVORITE_MODE_KEY;

    const FIRST_ID: &str = "01a03f5e-9c14-7b41-8a02-1de4c7c9b330";
    const FIRST_SAVED: &str = "2026-08-26T09:02:44.870-07:00";
    const SECOND_ID: &str = "01a03f60-2e8b-77c2-858f-476ee413d81c";
    const SECOND_SAVED: &str = "2026-08-26T14:31:05.412-07:00";

    fn mutation_errors() -> [FavoritesMutationError; 6] {
        let path = PathBuf::from("/tmp/favorites.toml");
        [
            FavoritesMutationError::LocationUnavailable,
            FavoritesMutationError::Unparseable {
                path:  path.clone(),
                error: "bad TOML".to_string(),
            },
            FavoritesMutationError::Unreadable {
                path:  path.clone(),
                error: "permission denied".to_string(),
            },
            FavoritesMutationError::LockUnavailable {
                path:  path.clone(),
                error: "favorites are in use".to_string(),
            },
            FavoritesMutationError::UnrecognizedFavoriteChanged,
            FavoritesMutationError::WriteFailed {
                path,
                error: "disk is read-only".to_string(),
            },
        ]
    }

    #[test]
    fn every_refusal_names_a_distinct_cause_for_both_mutations() {
        for mutation in [FavoritesMutation::Save, FavoritesMutation::Delete] {
            let action_name = match mutation {
                FavoritesMutation::Save => "save_favorite",
                FavoritesMutation::Delete => "delete",
            };
            let retry = FavoritesRetryInstruction::Press(ResolvedBinding::for_action(
                action_name,
                Some(KeySequence::from(KeyBind::from('x'))),
            ));
            let messages = mutation_errors()
                .iter()
                .map(|error| favorite_refusal_message(mutation, &retry, error))
                .collect::<Vec<_>>();
            let distinct = messages.iter().map(String::as_str).collect::<HashSet<_>>();

            assert_eq!(distinct.len(), messages.len());
            assert!(messages.iter().all(|message| match mutation {
                FavoritesMutation::Save => message.contains("save") || message.contains("in use"),
                FavoritesMutation::Delete => !message.contains("save"),
            }));
        }
    }

    #[test]
    fn lock_refusal_names_the_retry_that_works_on_each_surface() {
        let error = FavoritesMutationError::LockUnavailable {
            path:  PathBuf::from("/tmp/favorites.lock"),
            error: "held".to_string(),
        };
        let save = favorite_refusal_message(
            FavoritesMutation::Save,
            &FavoritesRetryInstruction::Press(ResolvedBinding::for_action(
                "save_favorite",
                Some(KeySequence::from(KeyBind::ctrl('s'))),
            )),
            &error,
        );
        let open_delete = favorite_refusal_message(
            FavoritesMutation::Delete,
            &FavoritesRetryInstruction::Press(ResolvedBinding::for_action(
                "delete",
                Some(KeySequence::from(KeyBind::from('x'))),
            )),
            &error,
        );
        let closed_delete = favorite_refusal_message(
            FavoritesMutation::Delete,
            &FavoritesRetryInstruction::ReopenThenPress {
                open:  ResolvedBinding::for_action(
                    "open_favorites",
                    Some(KeySequence::from(KeyBind::ctrl('o'))),
                ),
                retry: ResolvedBinding::for_action(
                    "delete",
                    Some(KeySequence::from(KeyBind::from('x'))),
                ),
            },
            &error,
        );
        let unbound_delete = favorite_refusal_message(
            FavoritesMutation::Delete,
            &FavoritesRetryInstruction::Press(ResolvedBinding::for_action("delete", None)),
            &error,
        );

        assert!(save.contains("press ⌃s to try again"));
        assert!(open_delete.contains("press x to try again"));
        assert!(closed_delete.contains("press ⌃o to reopen favorites"));
        assert!(closed_delete.contains("press x to retry the deletion"));
        assert!(unbound_delete.contains("bind the delete action first"));
        assert!(!unbound_delete.contains("bind the try again action"));
    }

    fn favorites_path(directory: &TempDir) -> PathBuf { directory.path().join(FAVORITES_FILENAME) }

    fn location(path: &Path) -> FavoritesLocation { FavoritesLocation::Path(path.to_path_buf()) }

    /// Rewrite the file under its lock without changing a row, exercising the
    /// read-modify-write path on its own.
    fn save_to_location(location: FavoritesLocation) -> Result<(), FavoritesMutationError> {
        edit_at_location(location, |_| Ok(()))
    }

    fn write_favorites(path: &Path, text: &str) {
        fs::write(path, text).expect("favorite fixture should be written");
    }

    fn favorite_id(spelling: &str) -> FavoriteId {
        FavoriteId::from_uuid_for_test(Uuid::parse_str(spelling).expect("favorite id should parse"))
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

    fn unrecognized_locators(
        state: &FavoritesFileState,
    ) -> Vec<UnrecognizedFavoriteRemovalLocator> {
        loaded_rows(state)
            .iter()
            .filter_map(|recognition| match recognition {
                FavoriteRowRecognition::Recognized(_) => None,
                FavoriteRowRecognition::Unrecognized {
                    removal_locator, ..
                } => Some(removal_locator.clone()),
            })
            .collect()
    }

    fn band_settings() -> AttractSettings {
        AttractSettings::MovingBand(BandSettings {
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
                FavoriteRowRecognition::Unrecognized { diagnostic, .. } => Some(diagnostic),
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

        remove_from_location(
            location(&path),
            FavoriteRemovalTarget::Recognized(favorite_id(SECOND_ID)),
        )
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
    fn unrecognized_deletion_survives_a_row_inserted_ahead_of_the_target() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        let target = format!(
            r#"[[favorite]]
id = "{FIRST_ID}"
saved = "{FIRST_SAVED}"
mode = "future_mode"
future_parameter = 88
"#
        );
        write_favorites(&path, &target);
        let state = load_from(location(&path));
        let locator = unrecognized_locators(&state)
            .into_iter()
            .next()
            .expect("unrecognized target should have a locator");
        let inserted_ahead = format!(
            r#"[[favorite]]
id = "{SECOND_ID}"
saved = "{SECOND_SAVED}"
mode = "future_mode"
future_parameter = 41

{target}"#
        );
        write_favorites(&path, &inserted_ahead);

        remove_from_location(
            location(&path),
            FavoriteRemovalTarget::Unrecognized(locator),
        )
        .expect("shifted unrecognized favorite should be removed");

        let remaining = fs::read_to_string(&path).expect("favorites should remain readable");
        assert!(remaining.contains(SECOND_ID));
        assert!(remaining.contains("future_parameter = 41"));
        assert!(!remaining.contains(FIRST_ID));
        assert!(!remaining.contains("future_parameter = 88"));
    }

    #[test]
    fn duplicate_id_row_deletion_preserves_the_recognized_twin() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        let duplicate_rows = format!(
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
        write_favorites(&path, &duplicate_rows);
        let state = load_from(location(&path));
        let locator = unrecognized_locators(&state)
            .into_iter()
            .next()
            .expect("duplicate-id row should have a locator");

        remove_from_location(
            location(&path),
            FavoriteRemovalTarget::Unrecognized(locator),
        )
        .expect("duplicate-id row should be removed");

        let after = load_from(location(&path));
        let recognized = loaded_rows(&after).recognized().collect::<Vec<_>>();
        assert_eq!(recognized.len(), 1);
        assert_eq!(recognized[0].id, favorite_id(FIRST_ID));
        let text = fs::read_to_string(path).expect("favorites should remain readable");
        assert_eq!(text.matches(FIRST_ID).count(), 1);
        assert!(text.contains(FIRST_SAVED));
        assert!(!text.contains(SECOND_SAVED));
    }

    #[test]
    fn byte_identical_unrecognized_rows_are_deleted_one_at_a_time() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        let target = format!(
            r#"[[favorite]]
id = "{FIRST_ID}"
saved = "{FIRST_SAVED}"
mode = "future_mode"
future_parameter = 17
"#
        );
        let text = format!("{target}\n{target}");
        write_favorites(&path, &text);
        let state = load_from(location(&path));
        let locators = unrecognized_locators(&state);
        assert_eq!(locators.len(), 2);

        remove_from_location(
            location(&path),
            FavoriteRemovalTarget::Unrecognized(locators[0].clone()),
        )
        .expect("first unrecognized favorite should be removed");
        let after_first = load_from(location(&path));
        assert_eq!(loaded_rows(&after_first).iter().count(), 1);

        remove_from_location(
            location(&path),
            FavoriteRemovalTarget::Unrecognized(locators[1].clone()),
        )
        .expect("second unrecognized favorite should be removed after its index changes");
        let after_second = load_from(location(&path));
        assert_eq!(loaded_rows(&after_second).iter().count(), 0);
    }

    #[test]
    fn unrecognized_nan_value_is_deletable_after_an_unchanged_reload() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        let target = format!(
            r#"[[favorite]]
id = "{FIRST_ID}"
saved = "{FIRST_SAVED}"
mode = "moving_band"
direction = "right"
width = 12
speed = nan
tail_speed = 96
fraying = "both"
"#
        );
        write_favorites(&path, &target);
        let state = load_from(location(&path));
        let locator = unrecognized_locators(&state)
            .into_iter()
            .next()
            .expect("NaN row should have an unrecognized locator");

        remove_from_location(
            location(&path),
            FavoriteRemovalTarget::Unrecognized(locator),
        )
        .expect("unchanged NaN row should be removed after reloading");

        let after = load_from(location(&path));
        assert_eq!(loaded_rows(&after).iter().count(), 0);
    }

    #[test]
    fn unrecognized_deletion_refuses_after_the_target_body_changes() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        let target = format!(
            r#"[[favorite]]
id = "{FIRST_ID}"
saved = "{FIRST_SAVED}"
mode = "moving_band"
direction = "right"
width = 12
speed = -0.0
tail_speed = 96
fraying = "both"
"#
        );
        write_favorites(&path, &target);
        let state = load_from(location(&path));
        let locator = unrecognized_locators(&state)
            .into_iter()
            .next()
            .expect("negative-zero row should have an unrecognized locator");
        let edited = target.replace("speed = -0.0", "speed = 0.0");
        write_favorites(&path, &edited);

        assert_eq!(
            remove_from_location(
                location(&path),
                FavoriteRemovalTarget::Unrecognized(locator),
            ),
            Err(FavoritesMutationError::UnrecognizedFavoriteChanged)
        );
        assert_eq!(
            fs::read_to_string(path).expect("refused favorites should remain readable"),
            edited
        );
    }

    #[test]
    fn unrecognized_deletion_refuses_an_ambiguous_stale_locator() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        let target = format!(
            r#"[[favorite]]
id = "{FIRST_ID}"
saved = "{FIRST_SAVED}"
mode = "future_mode"
future_parameter = 88
"#
        );
        write_favorites(&path, &target);
        let state = load_from(location(&path));
        let locator = unrecognized_locators(&state)
            .into_iter()
            .next()
            .expect("unrecognized target should have a locator");
        let moved_and_duplicated = format!(
            r#"[[favorite]]
id = "{SECOND_ID}"
saved = "{SECOND_SAVED}"
mode = "future_mode"
future_parameter = 41

{target}
{target}"#
        );
        write_favorites(&path, &moved_and_duplicated);

        assert_eq!(
            remove_from_location(
                location(&path),
                FavoriteRemovalTarget::Unrecognized(locator),
            ),
            Err(FavoritesMutationError::UnrecognizedFavoriteChanged)
        );
        assert_eq!(
            fs::read_to_string(path).expect("refused favorites should remain readable"),
            moved_and_duplicated
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
            remove_from_location(
                location(&path),
                FavoriteRemovalTarget::Recognized(favorite_id(FIRST_ID)),
            ),
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
            remove_from_location(
                location(&path),
                FavoriteRemovalTarget::Recognized(favorite_id(FIRST_ID)),
            ),
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
            push_to_location(FavoritesLocation::Unavailable, &favorite),
            Err(FavoritesMutationError::LocationUnavailable)
        );
        assert_eq!(
            remove_from_location(
                FavoritesLocation::Unavailable,
                FavoriteRemovalTarget::Recognized(favorite_id(FIRST_ID)),
            ),
            Err(FavoritesMutationError::LocationUnavailable)
        );
    }

    #[test]
    fn first_save_adds_and_identical_second_save_refreshes_one_row() {
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

        let first_outcome =
            push_to_location(location(&path), &first).expect("first favorite should be written");
        let second_outcome =
            push_to_location(location(&path), &second).expect("identical favorite should update");

        assert_eq!(first_outcome, FavoriteSaveOutcome::Added);
        assert_eq!(second_outcome, FavoriteSaveOutcome::Refreshed);
        let state = load_from(location(&path));
        let favorites: Vec<_> = loaded_rows(&state).recognized().collect();
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].id, first.id);
        assert_eq!(favorites[0].saved, saved(SECOND_SAVED));
        assert_ne!(favorites[0].saved, first.saved);
        assert_eq!(favorites[0].settings, first.settings);
        let text = fs::read_to_string(path).expect("updated favorites should be readable");
        assert!(text.contains(FIRST_ID));
        assert!(!text.contains(SECOND_ID));
    }

    #[test]
    fn live_favorite_is_identical_after_file_round_trip() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = favorites_path(&directory);
        let favorite = Favorite::now(band_settings());

        let outcome =
            push_to_location(location(&path), &favorite).expect("live favorite should be written");
        let state = load_from(location(&path));
        let loaded: Vec<_> = loaded_rows(&state).recognized().collect();

        assert_eq!(outcome, FavoriteSaveOutcome::Added);
        assert_eq!(loaded, [&favorite]);
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
            &Favorite {
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
