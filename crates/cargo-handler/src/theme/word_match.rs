//! Selected word grounds retain visible separation and readable ink.

use ratatui::style::Color;
use tui_pane::Appearance;

use crate::footer_colors;
use crate::footer_colors::constants::MINIMUM_CONTRAST;
use crate::word_highlight::MatchInk;

const MINIMUM_VISIBLE_DISTANCE: f64 = 3.0;

/// A word match retains the selected row's focus state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WordMatchSurface {
    Plain,
    FocusedSelection,
    UnfocusedSelection,
}

/// The selected step records which constraint ends the search.
enum SelectedWordMatchGround {
    DistanceMet(u8),
    ReadabilityCapped(u8),
    VisibilityFloor {
        step:      u8,
        direction: SelectionLightnessDirection,
    },
}

impl SelectedWordMatchGround {
    #[cfg(test)]
    const fn step(&self) -> u8 {
        match *self {
            Self::DistanceMet(step)
            | Self::ReadabilityCapped(step)
            | Self::VisibilityFloor { step, .. } => step,
        }
    }

    fn background(&self, selection: Color, direction: SelectionLightnessDirection) -> Color {
        match self {
            Self::VisibilityFloor { step, direction } => direction.at(selection, *step),
            Self::DistanceMet(step) | Self::ReadabilityCapped(step) => {
                direction.at(selection, *step)
            },
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WordMatchLimit {
    Plain,
    DistanceMet,
    ReadabilityCapped,
    VisibilityFloor,
}

/// The active variant's appearance determines the lightness direction.
#[derive(Clone, Copy)]
enum SelectionLightnessDirection {
    Lighter,
    Darker,
}

impl From<Appearance> for SelectionLightnessDirection {
    fn from(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Dark => Self::Lighter,
            Appearance::Light => Self::Darker,
        }
    }
}

impl SelectionLightnessDirection {
    fn at(self, selection: Color, step: u8) -> Color {
        let endpoint = match self {
            Self::Lighter => Color::White,
            Self::Darker => Color::Black,
        };
        tui_pane::blend_color(selection, endpoint, step)
    }
}

/// `WordMatchGrounds` separates selected matches from selection and plain match grounds.
pub(crate) struct WordMatchGrounds {
    plain:               Color,
    focused_selection:   Color,
    unfocused_selection: Color,
    focused_match:       SelectedWordMatchGround,
    unfocused_match:     SelectedWordMatchGround,
    direction:           SelectionLightnessDirection,
}

impl WordMatchGrounds {
    pub(crate) fn new(plain: Color) -> Self {
        let active = tui_pane::theme();
        let registry = tui_pane::registry();
        let appearance = registry
            .all()
            .find(|variant| variant.theme == *active)
            .map(|variant| variant.appearance)
            .or_else(|| {
                super::builtins()
                    .into_iter()
                    .find(|variant| variant.theme == *active)
                    .map(|variant| variant.appearance)
            })
            .unwrap_or(Appearance::Dark);
        Self::for_appearance(plain, appearance)
    }

    /// Use the variant's declared appearance, including custom mid-gray variants.
    pub(crate) fn for_appearance(plain: Color, appearance: Appearance) -> Self {
        let focused_selection = tui_pane::active_focus_color();
        let unfocused_selection = tui_pane::remembered_focus_color();
        let bound = visibility_bound(plain);
        let direction = SelectionLightnessDirection::from(appearance);
        Self {
            plain,
            focused_selection,
            unfocused_selection,
            focused_match: select_ground(focused_selection, plain, bound, direction),
            unfocused_match: select_ground(unfocused_selection, plain, bound, direction),
            direction,
        }
    }

    pub(crate) fn surface(&self, ground: Color) -> WordMatchSurface {
        if ground == self.focused_selection {
            WordMatchSurface::FocusedSelection
        } else if ground == self.unfocused_selection {
            WordMatchSurface::UnfocusedSelection
        } else {
            WordMatchSurface::Plain
        }
    }

    pub(crate) fn background(&self, surface: WordMatchSurface) -> Color {
        match surface {
            WordMatchSurface::Plain => self.plain,
            WordMatchSurface::FocusedSelection => self
                .focused_match
                .background(self.focused_selection, self.direction),
            WordMatchSurface::UnfocusedSelection => self
                .unfocused_match
                .background(self.unfocused_selection, self.direction),
        }
    }

    #[cfg(test)]
    pub(crate) fn bound(&self) -> f64 { visibility_bound(self.plain) }

    /// One step moves one part in 255 toward the appearance's lightness endpoint.
    #[cfg(test)]
    pub(crate) fn selection_step(&self, surface: WordMatchSurface, step: u8) -> Color {
        match surface {
            WordMatchSurface::Plain => self.plain,
            WordMatchSurface::FocusedSelection => self.direction.at(self.focused_selection, step),
            WordMatchSurface::UnfocusedSelection => {
                self.direction.at(self.unfocused_selection, step)
            },
        }
    }

    #[cfg(test)]
    pub(crate) const fn step(&self, surface: WordMatchSurface) -> u8 {
        match surface {
            WordMatchSurface::Plain => 0,
            WordMatchSurface::FocusedSelection => self.focused_match.step(),
            WordMatchSurface::UnfocusedSelection => self.unfocused_match.step(),
        }
    }

    #[cfg(test)]
    pub(crate) const fn limit(&self, surface: WordMatchSurface) -> WordMatchLimit {
        let selected = match surface {
            WordMatchSurface::Plain => return WordMatchLimit::Plain,
            WordMatchSurface::FocusedSelection => &self.focused_match,
            WordMatchSurface::UnfocusedSelection => &self.unfocused_match,
        };
        match selected {
            SelectedWordMatchGround::DistanceMet(_) => WordMatchLimit::DistanceMet,
            SelectedWordMatchGround::ReadabilityCapped(_) => WordMatchLimit::ReadabilityCapped,
            SelectedWordMatchGround::VisibilityFloor { .. } => WordMatchLimit::VisibilityFloor,
        }
    }
}

fn visibility_bound(plain: Color) -> f64 {
    footer_colors::perceptual_distance(plain, tui_pane::theme().text.bg_focus.color)
        .max(MINIMUM_VISIBLE_DISTANCE)
}

fn separation(background: Color, selection: Color, plain: Color) -> f64 {
    footer_colors::perceptual_distance(background, selection)
        .min(footer_colors::perceptual_distance(background, plain))
}

fn select_ground(
    selection: Color,
    plain: Color,
    bound: f64,
    direction: SelectionLightnessDirection,
) -> SelectedWordMatchGround {
    let mut previous_step = u8::MIN;
    for step in u8::MIN..=u8::MAX {
        let background = direction.at(selection, step);
        if MatchInk::capacity(background) < MINIMUM_CONTRAST {
            let previous = direction.at(selection, previous_step);
            if separation(previous, selection, plain) >= MINIMUM_VISIBLE_DISTANCE {
                return SelectedWordMatchGround::ReadabilityCapped(previous_step);
            }
            // Visibility has priority when the readability cap removes the match.
            return visible_floor(selection, plain, direction, step);
        }
        if separation(background, selection, plain) >= bound {
            return SelectedWordMatchGround::DistanceMet(step);
        }
        previous_step = step;
    }
    visible_floor(selection, plain, direction, u8::MIN)
}

fn visible_floor(
    selection: Color,
    plain: Color,
    direction: SelectionLightnessDirection,
    start: u8,
) -> SelectedWordMatchGround {
    for step in start..=u8::MAX {
        let background = direction.at(selection, step);
        if separation(background, selection, plain) >= MINIMUM_VISIBLE_DISTANCE {
            return SelectedWordMatchGround::VisibilityFloor { step, direction };
        }
    }
    // A selection at its appearance's endpoint needs the opposite direction.
    let direction = match direction {
        SelectionLightnessDirection::Lighter => SelectionLightnessDirection::Darker,
        SelectionLightnessDirection::Darker => SelectionLightnessDirection::Lighter,
    };
    for step in u8::MIN..=u8::MAX {
        let background = direction.at(selection, step);
        if separation(background, selection, plain) >= MINIMUM_VISIBLE_DISTANCE {
            return SelectedWordMatchGround::VisibilityFloor { step, direction };
        }
    }
    SelectedWordMatchGround::VisibilityFloor {
        step: u8::MAX,
        direction,
    }
}
