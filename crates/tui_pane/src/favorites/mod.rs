//! Lossless persistence for attract-screen parameter favorites.
//!
//! Each app keeps its favorites in `favorites.toml` beside its
//! `config.toml`, under the directory its [`AppIdentity`](crate::AppIdentity)
//! names. [`load_favorites`] reads the file without replacing anything it
//! cannot recognize, [`push_favorite`] saves the running
//! [`AttractSettings`](crate::AttractSettings) as a row (or refreshes the
//! row already holding them), and [`remove_favorite`] deletes one row under
//! the file's lock, re-verifying it first. Rows the running version cannot
//! read are kept and reported as [`FavoriteRowRecognition::Unrecognized`],
//! so a newer version's values survive a save from an older one.

/// Keys, spellings and lock timing for `favorites.toml`.
mod constants;
/// Favorites-file access, state, and mutation.
mod file;
/// TOML recognition and serialization of favorite field values.
mod recognition;
/// Favorite row models, sorting, and raw-table bookkeeping.
mod rows;

pub(crate) use file::FavoriteRemovalTarget;
pub use file::FavoritesFileState;
pub(crate) use file::FavoritesMutation;
pub(crate) use file::FavoritesMutationError;
pub(crate) use file::FavoritesRetryInstruction;
pub(crate) use file::ResolvedBinding;
pub(crate) use file::favorite_refusal_message;
pub(crate) use file::load_favorites;
pub(crate) use file::push_favorite;
pub(crate) use file::remove_favorite;
pub(crate) use recognition::UnrecognizedFavoriteValue;
pub use rows::Favorite;
pub(crate) use rows::FavoriteId;
pub(crate) use rows::FavoriteRowRecognition;
pub use rows::FavoriteRows;
pub(crate) use rows::FavoriteSaveOutcome;
pub(crate) use rows::UnrecognizedFavoriteRemovalLocator;
pub use rows::parse_favorite_rows_for_test;
