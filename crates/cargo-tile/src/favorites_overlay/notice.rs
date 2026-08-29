//! The favorites modal's notice line and the messages composed for it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use tui_pane::error_color;
use tui_pane::warning_color;

use super::parameter_column;
use crate::favorites;
use crate::favorites::AttractSettings;
use crate::favorites::FavoritesMutation;
use crate::favorites::FavoritesMutationError;
use crate::favorites::FavoritesRetryInstruction;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum FavoritesOverlayNotice {
    #[default]
    NoNotice,
    DeletionRefused {
        message: String,
    },
    DeletionConfirmation {
        message: String,
    },
    FavoriteAdjusted {
        message: String,
    },
}

pub(super) fn favorites_heading(saved_count: usize) -> String {
    format!(" Favorites -- {saved_count} saved -- ● matches the current parameters ")
}

pub(super) fn deletion_refusal_message(
    retry: &FavoritesRetryInstruction,
    error: &FavoritesMutationError,
) -> String {
    if matches!(error, FavoritesMutationError::UnrecognizedFavoriteChanged) {
        return "The favorites file changed after this row was loaded; nothing was deleted. Close \
                and reopen favorites, then try again."
            .to_string();
    }
    favorites::favorite_refusal_message(FavoritesMutation::Delete, retry, error)
}

pub(super) fn render_notice(frame: &mut Frame<'_>, notice: &FavoritesOverlayNotice, area: Rect) {
    let (message, color) = match notice {
        FavoritesOverlayNotice::NoNotice => return,
        FavoritesOverlayNotice::DeletionRefused { message } => (message, error_color()),
        FavoritesOverlayNotice::DeletionConfirmation { message }
        | FavoritesOverlayNotice::FavoriteAdjusted { message } => (message, warning_color()),
    };
    frame.render_widget(
        Paragraph::new(message.as_str())
            .style(Style::default().fg(color))
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub(super) fn favorite_adjustment_message(
    requested: AttractSettings,
    effective: AttractSettings,
) -> String {
    let mut fields = Vec::new();
    match (requested, effective) {
        (AttractSettings::MovingBand(requested), AttractSettings::MovingBand(effective)) => {
            record_adjustment(
                &mut fields,
                "direction",
                parameter_column::direction_name(requested.direction),
                parameter_column::direction_name(effective.direction),
            );
            record_numeric_adjustment(&mut fields, "width", &requested.width, &effective.width);
            record_numeric_adjustment(&mut fields, "speed", &requested.speed, &effective.speed);
            record_numeric_adjustment(
                &mut fields,
                "tail_speed",
                &requested.tail_speed,
                &effective.tail_speed,
            );
            record_adjustment(
                &mut fields,
                "fraying",
                parameter_column::fraying_name(requested.fraying),
                parameter_column::fraying_name(effective.fraying),
            );
        },
        (AttractSettings::MovingText(requested), AttractSettings::MovingText(effective)) => {
            record_adjustment(
                &mut fields,
                "direction",
                parameter_column::direction_name(requested.direction),
                parameter_column::direction_name(effective.direction),
            );
            record_numeric_adjustment(&mut fields, "speed", &requested.speed, &effective.speed);
            record_numeric_adjustment(&mut fields, "spread", &requested.spread, &effective.spread);
            record_adjustment(
                &mut fields,
                "drift",
                parameter_column::drift_name(requested.drift),
                parameter_column::drift_name(effective.drift),
            );
            record_adjustment(
                &mut fields,
                "fill",
                parameter_column::text_fill_name(requested.fill),
                parameter_column::text_fill_name(effective.fill),
            );
        },
        (AttractSettings::Pixelate(requested), AttractSettings::Pixelate(effective)) => {
            record_adjustment(
                &mut fields,
                "direction",
                parameter_column::direction_name(requested.direction),
                parameter_column::direction_name(effective.direction),
            );
            record_numeric_adjustment(&mut fields, "speed", &requested.speed, &effective.speed);
            record_numeric_adjustment(
                &mut fields,
                "wave_percent",
                &requested.wave_percent,
                &effective.wave_percent,
            );
            record_numeric_adjustment(
                &mut fields,
                "block_columns",
                &requested.block_columns,
                &effective.block_columns,
            );
            record_adjustment(
                &mut fields,
                "resolve",
                parameter_column::pixel_resolve_name(requested.resolve),
                parameter_column::pixel_resolve_name(effective.resolve),
            );
            record_adjustment(
                &mut fields,
                "fill",
                parameter_column::pixel_fill_name(requested.fill),
                parameter_column::pixel_fill_name(effective.fill),
            );
        },
        (
            AttractSettings::MovingBand(_)
            | AttractSettings::MovingText(_)
            | AttractSettings::Pixelate(_),
            AttractSettings::MovingBand(_)
            | AttractSettings::MovingText(_)
            | AttractSettings::Pixelate(_),
        ) => fields.push("mode changed unexpectedly".to_string()),
    }
    format!("Adjusted favorite for this terminal: {}", fields.join(", "))
}

fn record_adjustment(fields: &mut Vec<String>, name: &str, requested: &str, effective: &str) {
    if requested != effective {
        fields.push(format!("{name} {requested} -> {effective}"));
    }
}

fn record_numeric_adjustment<T: std::fmt::Display + Eq>(
    fields: &mut Vec<String>,
    name: &str,
    requested: &T,
    effective: &T,
) {
    if requested != effective {
        fields.push(format!("{name} {requested} -> {effective}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_explains_the_current_parameters_mark() {
        assert_eq!(
            favorites_heading(2),
            " Favorites -- 2 saved -- ● matches the current parameters "
        );
    }
}
