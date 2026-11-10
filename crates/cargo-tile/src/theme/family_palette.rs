//! Family colors cached for the active immutable theme.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use ratatui::style::Color;
use tui_pane::Theme;

use super::palette_clear_of;
use super::reserved_colors;

/// Family colors remain valid only for the complete immutable theme.
#[derive(Default)]
enum FamilyPalette {
    #[default]
    Unprepared,
    Prepared {
        theme:  Arc<Theme>,
        colors: Rc<[Color]>,
    },
}

thread_local! {
    static FAMILY_PALETTE: RefCell<FamilyPalette> = const { RefCell::new(FamilyPalette::Unprepared) };
}

/// Reuse the family colors until any theme input changes.
pub(super) fn colors() -> impl Iterator<Item = Color> {
    let theme = tui_pane::theme();
    let colors = FAMILY_PALETTE.with_borrow_mut(|palette| {
        if let FamilyPalette::Prepared {
            theme: previous,
            colors,
        } = palette
            && Arc::ptr_eq(previous, &theme)
        {
            return Rc::clone(colors);
        }
        let reserved = reserved_colors(&theme);
        let colors: Rc<[Color]> = palette_clear_of(&theme, &reserved).collect();
        *palette = FamilyPalette::Prepared {
            theme,
            colors: Rc::clone(&colors),
        };
        colors
    });
    (0..colors.len()).map(move |index| colors[index])
}
