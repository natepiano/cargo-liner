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

pub use file::FavoriteRemovalTarget;
pub use file::FavoritesFileState;
pub use file::FavoritesMutation;
pub use file::FavoritesMutationError;
pub use file::FavoritesRetryInstruction;
pub use file::ResolvedBinding;
pub use file::favorite_refusal_message;
pub use file::load_favorites;
pub use file::push_favorite;
pub use file::remove_favorite;
pub use recognition::UnrecognizedFavoriteValue;
pub use rows::Favorite;
pub use rows::FavoriteId;
pub use rows::FavoriteRowRecognition;
pub use rows::FavoriteRows;
pub use rows::FavoriteSaveOutcome;
pub use rows::UnrecognizedFavoriteRemovalLocator;
pub use rows::parse_favorite_rows_for_test;
