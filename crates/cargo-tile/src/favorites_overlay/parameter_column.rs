//! Parameter columns for a favorite's saved settings and the spellings of their values.

use tui_pane::BandDirection;
use tui_pane::BandFraying;
use tui_pane::PixelFill;
use tui_pane::PixelResolve;
use tui_pane::TextDrift;
use tui_pane::TextFill;

use crate::attract::AttractMode;
use crate::favorites::AttractSettings;

#[derive(Clone, Copy, Debug)]
pub(super) struct ParameterColumnDescriptor {
    pub(super) heading:      &'static str,
    value_renderer:          fn(AttractSettings) -> String,
    pub(super) action_names: &'static [&'static str],
    pub(super) separator:    &'static str,
}

impl ParameterColumnDescriptor {
    pub(super) fn render_value(self, settings: AttractSettings) -> String {
        (self.value_renderer)(settings)
    }
}

const BAND_COLUMNS: [ParameterColumnDescriptor; 5] = [
    ParameterColumnDescriptor {
        heading:        "Direction",
        value_renderer: render_band_direction,
        action_names:   &["travel_left", "travel_up", "travel_down", "travel_right"],
        separator:      "",
    },
    ParameterColumnDescriptor {
        heading:        "Width",
        value_renderer: render_band_width,
        action_names:   &["thinner", "wider"],
        separator:      "/",
    },
    ParameterColumnDescriptor {
        heading:        "Speed",
        value_renderer: render_band_speed,
        action_names:   &["slower", "faster"],
        separator:      "/",
    },
    ParameterColumnDescriptor {
        heading:        "Tail",
        value_renderer: render_band_tail_speed,
        action_names:   &["tail_slower", "tail_faster"],
        separator:      "/",
    },
    ParameterColumnDescriptor {
        heading:        "Fraying",
        value_renderer: render_band_fraying,
        action_names:   &["cycle_fraying"],
        separator:      "",
    },
];
#[cfg(test)]
pub(super) const BAND_COLUMNS_FOR_TEST: [ParameterColumnDescriptor; 5] = BAND_COLUMNS;

const TEXT_COLUMNS: [ParameterColumnDescriptor; 5] = [
    ParameterColumnDescriptor {
        heading:        "Direction",
        value_renderer: render_text_direction,
        action_names:   &["travel_left", "travel_up", "travel_down", "travel_right"],
        separator:      "",
    },
    ParameterColumnDescriptor {
        heading:        "Speed",
        value_renderer: render_text_speed,
        action_names:   &["slower", "faster"],
        separator:      "/",
    },
    ParameterColumnDescriptor {
        heading:        "Spread",
        value_renderer: render_text_spread,
        action_names:   &["spread_narrower", "spread_wider"],
        separator:      "/",
    },
    ParameterColumnDescriptor {
        heading:        "Drift",
        value_renderer: render_text_drift,
        action_names:   &["cycle_drift"],
        separator:      "",
    },
    ParameterColumnDescriptor {
        heading:        "Fill",
        value_renderer: render_text_fill,
        action_names:   &["cycle_fill"],
        separator:      "",
    },
];

const PIXEL_COLUMNS: [ParameterColumnDescriptor; 6] = [
    ParameterColumnDescriptor {
        heading:        "Direction",
        value_renderer: render_pixel_direction,
        action_names:   &["sweep_left", "sweep_up", "sweep_down", "sweep_right"],
        separator:      "",
    },
    ParameterColumnDescriptor {
        heading:        "Speed",
        value_renderer: render_pixel_speed,
        action_names:   &["slower", "faster"],
        separator:      "/",
    },
    ParameterColumnDescriptor {
        heading:        "Wave",
        value_renderer: render_pixel_wave_percent,
        action_names:   &["wave_narrower", "wave_wider"],
        separator:      "/",
    },
    ParameterColumnDescriptor {
        heading:        "Block",
        value_renderer: render_pixel_block_columns,
        action_names:   &["sharper", "coarser"],
        separator:      "/",
    },
    ParameterColumnDescriptor {
        heading:        "Resolve",
        value_renderer: render_pixel_resolve,
        action_names:   &["cycle_resolve"],
        separator:      "",
    },
    ParameterColumnDescriptor {
        heading:        "Fill",
        value_renderer: render_pixel_fill,
        action_names:   &["cycle_fill"],
        separator:      "",
    },
];

pub(super) const fn column_descriptors(mode: AttractMode) -> &'static [ParameterColumnDescriptor] {
    match mode {
        AttractMode::MovingBand => &BAND_COLUMNS,
        AttractMode::MovingText => &TEXT_COLUMNS,
        AttractMode::Pixelate => &PIXEL_COLUMNS,
    }
}

fn render_band_direction(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::MovingBand(settings) => direction_name(settings.direction).to_string(),
        AttractSettings::MovingText(_) | AttractSettings::Pixelate(_) => {
            parameter_value_mode_mismatch(AttractMode::MovingBand, settings.mode())
        },
    }
}

fn render_band_width(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::MovingBand(settings) => settings.width.to_string(),
        AttractSettings::MovingText(_) | AttractSettings::Pixelate(_) => {
            parameter_value_mode_mismatch(AttractMode::MovingBand, settings.mode())
        },
    }
}

fn render_band_speed(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::MovingBand(settings) => settings.speed.to_string(),
        AttractSettings::MovingText(_) | AttractSettings::Pixelate(_) => {
            parameter_value_mode_mismatch(AttractMode::MovingBand, settings.mode())
        },
    }
}

fn render_band_tail_speed(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::MovingBand(settings) => settings.tail_speed.to_string(),
        AttractSettings::MovingText(_) | AttractSettings::Pixelate(_) => {
            parameter_value_mode_mismatch(AttractMode::MovingBand, settings.mode())
        },
    }
}

fn render_band_fraying(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::MovingBand(settings) => fraying_name(settings.fraying).to_string(),
        AttractSettings::MovingText(_) | AttractSettings::Pixelate(_) => {
            parameter_value_mode_mismatch(AttractMode::MovingBand, settings.mode())
        },
    }
}

fn render_text_direction(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::MovingText(settings) => direction_name(settings.direction).to_string(),
        AttractSettings::MovingBand(_) | AttractSettings::Pixelate(_) => {
            parameter_value_mode_mismatch(AttractMode::MovingText, settings.mode())
        },
    }
}

fn render_text_speed(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::MovingText(settings) => settings.speed.to_string(),
        AttractSettings::MovingBand(_) | AttractSettings::Pixelate(_) => {
            parameter_value_mode_mismatch(AttractMode::MovingText, settings.mode())
        },
    }
}

fn render_text_spread(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::MovingText(settings) => settings.spread.to_string(),
        AttractSettings::MovingBand(_) | AttractSettings::Pixelate(_) => {
            parameter_value_mode_mismatch(AttractMode::MovingText, settings.mode())
        },
    }
}

fn render_text_drift(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::MovingText(settings) => drift_name(settings.drift).to_string(),
        AttractSettings::MovingBand(_) | AttractSettings::Pixelate(_) => {
            parameter_value_mode_mismatch(AttractMode::MovingText, settings.mode())
        },
    }
}

fn render_text_fill(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::MovingText(settings) => text_fill_name(settings.fill).to_string(),
        AttractSettings::MovingBand(_) | AttractSettings::Pixelate(_) => {
            parameter_value_mode_mismatch(AttractMode::MovingText, settings.mode())
        },
    }
}

fn render_pixel_direction(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::Pixelate(settings) => direction_name(settings.direction).to_string(),
        AttractSettings::MovingBand(_) | AttractSettings::MovingText(_) => {
            parameter_value_mode_mismatch(AttractMode::Pixelate, settings.mode())
        },
    }
}

fn render_pixel_speed(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::Pixelate(settings) => settings.speed.to_string(),
        AttractSettings::MovingBand(_) | AttractSettings::MovingText(_) => {
            parameter_value_mode_mismatch(AttractMode::Pixelate, settings.mode())
        },
    }
}

fn render_pixel_wave_percent(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::Pixelate(settings) => settings.wave_percent.to_string(),
        AttractSettings::MovingBand(_) | AttractSettings::MovingText(_) => {
            parameter_value_mode_mismatch(AttractMode::Pixelate, settings.mode())
        },
    }
}

fn render_pixel_block_columns(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::Pixelate(settings) => settings.block_columns.to_string(),
        AttractSettings::MovingBand(_) | AttractSettings::MovingText(_) => {
            parameter_value_mode_mismatch(AttractMode::Pixelate, settings.mode())
        },
    }
}

fn render_pixel_resolve(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::Pixelate(settings) => pixel_resolve_name(settings.resolve).to_string(),
        AttractSettings::MovingBand(_) | AttractSettings::MovingText(_) => {
            parameter_value_mode_mismatch(AttractMode::Pixelate, settings.mode())
        },
    }
}

fn render_pixel_fill(settings: AttractSettings) -> String {
    match settings {
        AttractSettings::Pixelate(settings) => pixel_fill_name(settings.fill).to_string(),
        AttractSettings::MovingBand(_) | AttractSettings::MovingText(_) => {
            parameter_value_mode_mismatch(AttractMode::Pixelate, settings.mode())
        },
    }
}

fn parameter_value_mode_mismatch(expected: AttractMode, actual: AttractMode) -> String {
    format!(
        "{} value unavailable for {}",
        mode_label(expected),
        mode_label(actual)
    )
}

pub(super) const fn direction_name(direction: BandDirection) -> &'static str {
    match direction {
        BandDirection::Left => "left",
        BandDirection::Right => "right",
        BandDirection::Up => "up",
        BandDirection::Down => "down",
    }
}

pub(super) const fn fraying_name(fraying: BandFraying) -> &'static str {
    match fraying {
        BandFraying::Trailing => "trailing",
        BandFraying::Both => "both",
        BandFraying::Leading => "leading",
        BandFraying::Neither => "neither",
    }
}

pub(super) const fn drift_name(drift: TextDrift) -> &'static str {
    match drift {
        TextDrift::Together => "together",
        TextDrift::Apart => "apart",
    }
}

pub(super) const fn text_fill_name(fill: TextFill) -> &'static str {
    match fill {
        TextFill::Bars => "bars",
        TextFill::Glyphs => "glyphs",
    }
}

pub(super) const fn pixel_resolve_name(resolve: PixelResolve) -> &'static str {
    match resolve {
        PixelResolve::Blend => "blend",
        PixelResolve::Step => "step",
        PixelResolve::Scatter => "scatter",
    }
}

pub(super) const fn pixel_fill_name(fill: PixelFill) -> &'static str {
    match fill {
        PixelFill::Solid => "solid",
        PixelFill::Shades => "shades",
    }
}

pub(super) const fn mode_label(mode: AttractMode) -> &'static str {
    match mode {
        AttractMode::MovingBand => "Moving Band",
        AttractMode::MovingText => "Moving Text",
        AttractMode::Pixelate => "Pixelate",
    }
}

#[cfg(test)]
mod tests {
    use tui_pane::BandSettings;

    use super::*;

    #[test]
    fn reordering_descriptors_keeps_each_heading_with_its_value() {
        let settings = AttractSettings::MovingBand(BandSettings {
            direction:  BandDirection::Right,
            width:      17,
            speed:      23,
            tail_speed: 41,
            fraying:    BandFraying::Both,
        });
        let reordered = [
            BAND_COLUMNS[4],
            BAND_COLUMNS[1],
            BAND_COLUMNS[3],
            BAND_COLUMNS[0],
            BAND_COLUMNS[2],
        ];

        assert_eq!(
            reordered.map(|descriptor| (descriptor.heading, descriptor.render_value(settings))),
            [
                ("Fraying", "both".to_string()),
                ("Width", "17".to_string()),
                ("Tail", "41".to_string()),
                ("Direction", "right".to_string()),
                ("Speed", "23".to_string()),
            ]
        );
    }
}
