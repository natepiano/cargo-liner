//! Lossless persistence for attract-screen parameter favorites.

/// Favorites-file access, state, and mutation.
mod file;
/// Favorite row models, recognition, sorting, and serialization.
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
pub(crate) use rows::AttractSettings;
pub(crate) use rows::Favorite;
pub(crate) use rows::FavoriteId;
pub(crate) use rows::FavoriteRowRecognition;
pub(crate) use rows::FavoriteRows;
pub(crate) use rows::UnrecognizedFavoriteRemovalLocator;
pub(crate) use rows::UnrecognizedFavoriteValue;
#[cfg(test)]
pub(crate) use rows::parse_rows_for_overlay_test;
