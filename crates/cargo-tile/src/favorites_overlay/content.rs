//! Display-ready favorites content and row lifecycle state.

use std::mem;
use std::path::PathBuf;
use std::time::Instant;

use chrono::Datelike;
use chrono::Local;

use crate::attract::AttractMode;
use crate::favorites::AttractSettings;
use crate::favorites::Favorite;
use crate::favorites::FavoriteId;
use crate::favorites::FavoriteRowRecognition;
use crate::favorites::FavoriteRows;
use crate::favorites::FavoritesFileState;
use crate::favorites::UnrecognizedFavoriteRemovalLocator;
use crate::favorites::UnrecognizedFavoriteValue;

/// The content carried by an open favorites modal.
#[derive(Clone, Debug)]
pub(crate) enum FavoritesOverlayContent {
    /// At least one recognized favorite, with any unrecognized rows retained below it.
    Rows(FavoriteRowsView),
    /// No favorites file has been created yet, or its loaded row list is empty.
    NoneSaved,
    /// The file has rows, but this build recognizes none of them.
    OnlyUnrecognized(UnrecognizedFavoritesView),
    /// The operating system supplied no configuration directory.
    LocationUnavailable,
    /// The file exists but its TOML or row structure is invalid.
    Unparseable {
        /// Path holding the invalid content.
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

impl From<FavoritesFileState> for FavoritesOverlayContent {
    fn from(state: FavoritesFileState) -> Self {
        match state {
            FavoritesFileState::LocationUnavailable => Self::LocationUnavailable,
            FavoritesFileState::Missing { .. } => Self::NoneSaved,
            FavoritesFileState::Loaded { rows, .. } => {
                let view = FavoriteRowsView::from(&rows);
                if view.saved_count() > 0 {
                    Self::Rows(view)
                } else if view.unrecognized.is_empty() {
                    Self::NoneSaved
                } else {
                    Self::OnlyUnrecognized(UnrecognizedFavoritesView {
                        rows: view.unrecognized,
                    })
                }
            },
            FavoritesFileState::Unparseable { path, error } => Self::Unparseable { path, error },
            FavoritesFileState::Unreadable { path, error } => Self::Unreadable { path, error },
        }
    }
}

impl FavoritesOverlayContent {
    pub(super) fn saved_count(&self) -> usize {
        match self {
            Self::Rows(rows) => rows.saved_count(),
            Self::NoneSaved
            | Self::OnlyUnrecognized(_)
            | Self::LocationUnavailable
            | Self::Unparseable { .. }
            | Self::Unreadable { .. } => 0,
        }
    }

    pub(super) fn navigable_row_count(&self) -> usize {
        match self {
            Self::Rows(rows) => rows.navigable_row_count(),
            Self::OnlyUnrecognized(rows) => rows.rows.len(),
            Self::NoneSaved
            | Self::LocationUnavailable
            | Self::Unparseable { .. }
            | Self::Unreadable { .. } => 0,
        }
    }

    /// Collapse to the emptier content state a completed removal may have reached.
    pub(super) fn normalize_after_removal(&mut self) {
        let current = mem::replace(self, Self::NoneSaved);
        *self = match current {
            Self::Rows(rows) if rows.saved_count() == 0 => {
                if rows.unrecognized.is_empty() {
                    Self::NoneSaved
                } else {
                    Self::OnlyUnrecognized(UnrecognizedFavoritesView {
                        rows: rows.unrecognized,
                    })
                }
            },
            Self::OnlyUnrecognized(rows) if rows.rows.is_empty() => Self::NoneSaved,
            other => other,
        };
    }
}

/// Cached, display-ready recognized favorites and diagnostics.
#[derive(Clone, Debug)]
pub(crate) struct FavoriteRowsView {
    pub(super) sections:     Vec<FavoriteModeSection>,
    pub(super) unrecognized: Vec<UnrecognizedFavoriteView>,
}

impl From<&FavoriteRows> for FavoriteRowsView {
    fn from(rows: &FavoriteRows) -> Self {
        let mut sections: Vec<FavoriteModeSection> = Vec::new();
        let mut unrecognized = Vec::new();
        for recognition in rows.iter() {
            match recognition {
                FavoriteRowRecognition::Recognized(favorite) => {
                    let mode = favorite.settings.mode();
                    if let Some(section) = sections.iter_mut().find(|section| section.mode == mode)
                    {
                        section.rows.push(FavoriteRowView::from(favorite));
                    } else {
                        sections.push(FavoriteModeSection {
                            mode,
                            rows: vec![FavoriteRowView::from(favorite)],
                        });
                    }
                },
                FavoriteRowRecognition::Unrecognized {
                    diagnostic,
                    removal_locator,
                } => {
                    unrecognized.push(UnrecognizedFavoriteView::new(
                        diagnostic,
                        removal_locator.clone(),
                    ));
                },
            }
        }
        Self {
            sections,
            unrecognized,
        }
    }
}

impl FavoriteRowsView {
    pub(super) fn saved_count(&self) -> usize {
        self.sections.iter().map(|section| section.rows.len()).sum()
    }

    pub(super) fn navigable_row_count(&self) -> usize {
        self.saved_count().saturating_add(self.unrecognized.len())
    }

    pub(super) fn row(&self, favorite_id: FavoriteId) -> FavoriteRowLookup<'_> {
        self.sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.id == favorite_id)
            .map_or(FavoriteRowLookup::Missing, FavoriteRowLookup::Found)
    }

    pub(super) fn row_mut(&mut self, favorite_id: FavoriteId) -> FavoriteRowLookupMut<'_> {
        self.sections
            .iter_mut()
            .flat_map(|section| &mut section.rows)
            .find(|row| row.id == favorite_id)
            .map_or(FavoriteRowLookupMut::Missing, FavoriteRowLookupMut::Found)
    }

    pub(super) fn remove(&mut self, favorite_id: FavoriteId) {
        for section in &mut self.sections {
            section.rows.retain(|row| row.id != favorite_id);
        }
        self.sections.retain(|section| !section.rows.is_empty());
    }

    pub(super) fn remove_unrecognized(
        &mut self,
        removal_locator: &UnrecognizedFavoriteRemovalLocator,
    ) {
        self.unrecognized
            .retain(|row| row.removal_locator != *removal_locator);
    }

    pub(super) fn removing_ids(&self) -> Vec<FavoriteId> {
        self.sections
            .iter()
            .flat_map(|section| &section.rows)
            .filter_map(|row| match row.lifecycle {
                FavoriteRowLifecycle::Active => None,
                FavoriteRowLifecycle::Removing { .. } => Some(row.id),
            })
            .collect()
    }
}

pub(super) enum FavoriteRowLookup<'a> {
    Found(&'a FavoriteRowView),
    Missing,
}

pub(super) enum FavoriteRowLookupMut<'a> {
    Found(&'a mut FavoriteRowView),
    Missing,
}

#[derive(Clone, Debug)]
pub(super) struct FavoriteModeSection {
    pub(super) mode: AttractMode,
    pub(super) rows: Vec<FavoriteRowView>,
}

#[derive(Clone, Debug)]
pub(super) struct FavoriteRowView {
    pub(super) id:        FavoriteId,
    pub(super) settings:  AttractSettings,
    pub(super) saved:     String,
    pub(super) lifecycle: FavoriteRowLifecycle,
}

impl From<&Favorite> for FavoriteRowView {
    fn from(favorite: &Favorite) -> Self {
        Self {
            id:        favorite.id,
            settings:  favorite.settings,
            saved:     format_timestamp(favorite),
            lifecycle: FavoriteRowLifecycle::Active,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FavoriteRowLifecycle {
    Active,
    Removing { since: Instant },
}

/// Display-ready rows a newer or misspelled file left unrecognized.
#[derive(Clone, Debug)]
pub(crate) struct UnrecognizedFavoritesView {
    pub(super) rows: Vec<UnrecognizedFavoriteView>,
}

#[derive(Clone, Debug)]
pub(super) struct UnrecognizedFavoriteView {
    pub(super) removal_locator: UnrecognizedFavoriteRemovalLocator,
    pub(super) key:             String,
    pub(super) spelling:        String,
    pub(super) lifecycle:       FavoriteRowLifecycle,
}

impl UnrecognizedFavoriteView {
    fn new(
        value: &UnrecognizedFavoriteValue,
        removal_locator: UnrecognizedFavoriteRemovalLocator,
    ) -> Self {
        Self {
            removal_locator,
            key: value.key.clone(),
            spelling: value.spelling.clone(),
            lifecycle: FavoriteRowLifecycle::Active,
        }
    }
}

fn format_timestamp(favorite: &Favorite) -> String {
    if favorite.saved.year() == Local::now().year() {
        favorite.saved.format("%d %b %H:%M:%S").to_string()
    } else {
        favorite.saved.format("%d %b %Y %H:%M:%S").to_string()
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::favorites;

    #[test]
    fn timestamps_keep_seconds_and_add_the_year_only_when_needed() {
        let current_year = Local::now().year();
        let old_year = current_year - 1;
        let rows = favorites::parse_rows_for_overlay_test(&format!(
            r#"
[[favorite]]
id = "01a03f64-9c14-7b41-8a02-1de4c7c9b336"
saved = "{current_year}-01-02T03:04:05-05:00"
mode = "moving_band"
direction = "left"
width = 10
speed = 32
tail_speed = 72
fraying = "leading"

[[favorite]]
id = "01a03f65-9c14-7b41-8a02-1de4c7c9b337"
saved = "{old_year}-01-02T03:04:05-05:00"
mode = "moving_band"
direction = "right"
width = 12
speed = 40
tail_speed = 96
fraying = "both"
"#
        ))
        .expect("timestamp fixture should parse");
        let view = FavoriteRowsView::from(&rows);
        let saved = view.sections[0]
            .rows
            .iter()
            .map(|row| row.saved.as_str())
            .collect::<Vec<_>>();

        assert!(saved.contains(&"02 Jan 03:04:05"));
        assert!(saved.contains(&format!("02 Jan {old_year} 03:04:05").as_str()));
    }
}
