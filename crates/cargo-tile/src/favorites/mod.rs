//! Lossless persistence for attract-screen parameter favorites.

/// Favorites-file access, state, and mutation.
mod file;
/// TOML recognition and serialization of favorite field values.
mod recognition;
/// Favorite row models, sorting, and raw-table bookkeeping.
mod rows;

pub(crate) use file::FavoriteRemovalTarget;
pub(crate) use file::FavoritesFileState;
pub(crate) use file::FavoritesMutation;
pub(crate) use file::FavoritesMutationError;
pub(crate) use file::FavoritesRetryInstruction;
pub(crate) use file::ResolvedBinding;
pub(crate) use file::favorite_refusal_message;
pub(crate) use file::load;
pub(crate) use file::push;
pub(crate) use file::remove;
pub(crate) use recognition::UnrecognizedFavoriteValue;
pub(crate) use rows::AttractSettings;
pub(crate) use rows::Favorite;
pub(crate) use rows::FavoriteId;
pub(crate) use rows::FavoriteRowRecognition;
pub(crate) use rows::FavoriteRows;
pub(crate) use rows::FavoriteSaveOutcome;
pub(crate) use rows::UnrecognizedFavoriteRemovalLocator;
#[cfg(test)]
pub(crate) use rows::parse_rows_for_overlay_test;
