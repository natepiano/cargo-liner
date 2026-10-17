//! Footer configuration names and the terminal colors users see.

#![expect(
    clippy::expect_used,
    reason = "fixture colors and temporary files must be valid"
)]

use std::fs::File;
use std::fs::FileTimes;
use std::time::Duration;
use std::time::UNIX_EPOCH;

use ratatui::style::Color;

use super::constants::BLUE_LUMINANCE_WEIGHT;
use super::constants::CONTRAST_LUMINANCE_OFFSET;
use super::constants::GREEN_LUMINANCE_WEIGHT;
use super::constants::MINIMUM_CONTRAST;
use super::constants::RED_LUMINANCE_WEIGHT;
use super::constants::SRGB_ENCODED_OFFSET;
use super::constants::SRGB_ENCODED_SCALE;
use super::constants::SRGB_LINEAR_DIVISOR;
use super::constants::SRGB_LINEAR_THRESHOLD;
use super::constants::SRGB_POWER;
use super::*;
use crate::theme;

/// Independent WCAG oracle: ANSI values follow the documented xterm approximation.
pub(crate) fn contrast_ratio(foreground: Color, background: Color) -> f64 {
    fn rgb(color: Color) -> (u8, u8, u8) {
        let index = match color {
            Color::Black | Color::Reset => 0,
            Color::Red => 1,
            Color::Green => 2,
            Color::Yellow => 3,
            Color::Blue => 4,
            Color::Magenta => 5,
            Color::Cyan => 6,
            Color::Gray => 7,
            Color::DarkGray => 8,
            Color::LightRed => 9,
            Color::LightGreen => 10,
            Color::LightYellow => 11,
            Color::LightBlue => 12,
            Color::LightMagenta => 13,
            Color::LightCyan => 14,
            Color::White => 15,
            Color::Indexed(index) => index,
            Color::Rgb(red, green, blue) => return (red, green, blue),
        };
        let ansi = [
            (0, 0, 0),
            (205, 0, 0),
            (0, 205, 0),
            (205, 205, 0),
            (0, 0, 238),
            (205, 0, 205),
            (0, 205, 205),
            (229, 229, 229),
            (127, 127, 127),
            (255, 0, 0),
            (0, 255, 0),
            (255, 255, 0),
            (92, 92, 255),
            (255, 0, 255),
            (0, 255, 255),
            (255, 255, 255),
        ];
        if let Some(channels) = ansi.get(usize::from(index)) {
            return *channels;
        }
        if index >= 232 {
            let gray = 8 + 10 * (index - 232);
            return (gray, gray, gray);
        }
        let levels = [0, 95, 135, 175, 215, 255];
        let offset = index - 16;
        (
            levels[usize::from(offset / 36)],
            levels[usize::from((offset % 36) / 6)],
            levels[usize::from(offset % 6)],
        )
    }
    fn luminance(color: Color) -> f64 {
        let (red, green, blue) = rgb(color);
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= SRGB_LINEAR_THRESHOLD {
                value / SRGB_LINEAR_DIVISOR
            } else {
                ((value + SRGB_ENCODED_OFFSET) / SRGB_ENCODED_SCALE).powf(SRGB_POWER)
            }
        };
        RED_LUMINANCE_WEIGHT.mul_add(
            linear(red),
            GREEN_LUMINANCE_WEIGHT.mul_add(linear(green), BLUE_LUMINANCE_WEIGHT * linear(blue)),
        )
    }
    assert_ne!(
        foreground,
        Color::Reset,
        "text must have a measurable foreground"
    );
    assert_ne!(
        background,
        Color::Reset,
        "the theme must supply a background"
    );
    let foreground = luminance(foreground);
    let background = luminance(background);
    (foreground.max(background) + CONTRAST_LUMINANCE_OFFSET)
        / (foreground.min(background) + CONTRAST_LUMINANCE_OFFSET)
}

#[test]
fn footer_contrast_every_named_ink_is_readable_and_keeps_sufficient_ansi_colors() {
    let previous = tui_pane::theme();
    for variant in theme::builtins() {
        let background = variant.theme.text.bg_focus.color;
        tui_pane::set_active_theme(std::sync::Arc::new(variant.theme));
        for name in [
            "black",
            "red",
            "green",
            "yellow",
            "blue",
            "magenta",
            "cyan",
            "white",
            "bright-black",
            "bright-red",
            "bright-green",
            "bright-yellow",
            "bright-blue",
            "bright-magenta",
            "bright-cyan",
            "bright-white",
            "orange",
            "purple",
            "pink",
            "teal",
            "brown",
        ] {
            let colors =
                FooterColors::parse(&format!("account.fixture={name}\nrole.worker={name}"));
            let original = Color::from(
                name.parse::<FooterColor>()
                    .expect("documented footer color"),
            );
            let account = colors
                .account("fixture")
                .style(Style::default())
                .fg
                .expect("account ink");
            let role = colors
                .role(FooterRole::Worker)
                .style(Style::default())
                .fg
                .expect("role ink");
            for foreground in [account, role] {
                let ratio = contrast_ratio(foreground, background);
                assert!(
                    ratio >= MINIMUM_CONTRAST,
                    "{} {name}: {foreground:?} on {background:?} is {ratio}:1",
                    variant.id.as_str()
                );
                if contrast_ratio(original, background) >= MINIMUM_CONTRAST {
                    assert_eq!(
                        foreground,
                        original,
                        "{} {name} already meets contrast",
                        variant.id.as_str()
                    );
                } else {
                    assert!(
                        matches!(foreground, Color::Rgb(..)),
                        "{name} needs an explicit corrected RGB"
                    );
                }
            }
            let muted = colors
                .account("fixture")
                .subdued_style(Style::default().add_modifier(ratatui::style::Modifier::DIM));
            assert!(!muted.add_modifier.contains(ratatui::style::Modifier::DIM));
            let ratio = contrast_ratio(muted.fg.expect("subdued ink"), background);
            assert!(
                ratio >= MINIMUM_CONTRAST,
                "{} subdued {name}: {ratio}:1",
                variant.id.as_str()
            );
        }
    }
    tui_pane::set_active_theme(previous);
}

#[test]
fn footer_contrast_uses_theme_ground_through_transparency_focus_and_theme_changes() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let colors = FooterColors::parse("account.fixture=blue");
    let mut corrected = Vec::new();
    for variant in theme::builtins() {
        let background = variant.theme.text.bg_focus.color;
        tui_pane::set_active_theme(std::sync::Arc::new(variant.theme));
        let expected = colors
            .account("fixture")
            .style(Style::default())
            .fg
            .expect("blue ink");
        corrected.push(expected);
        for transparent in [false, true] {
            tui_pane::set_transparent_background(transparent);
            for focused_tint in [false, true] {
                tui_pane::set_focused_pane_tint(focused_tint);
                let actual = colors
                    .account("fixture")
                    .style(Style::default())
                    .fg
                    .expect("blue ink");
                assert_eq!(
                    actual, expected,
                    "pane decoration does not replace the theme ground"
                );
                assert!(contrast_ratio(actual, background) >= MINIMUM_CONTRAST);
            }
        }
    }
    assert_ne!(
        corrected[0], corrected[1],
        "blue must adapt between dark and light themes"
    );
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
}

#[test]
fn every_footer_color_name_matches_its_terminal_code() {
    for (name, expected) in [
        ("black", Color::Black),
        ("red", Color::Red),
        ("green", Color::Green),
        ("yellow", Color::Yellow),
        ("blue", Color::Blue),
        ("magenta", Color::Magenta),
        ("cyan", Color::Cyan),
        ("white", Color::Gray),
        ("bright-black", Color::DarkGray),
        ("bright-red", Color::LightRed),
        ("bright-green", Color::LightGreen),
        ("bright-yellow", Color::LightYellow),
        ("bright-blue", Color::LightBlue),
        ("bright-magenta", Color::LightMagenta),
        ("bright-cyan", Color::LightCyan),
        ("bright-white", Color::White),
        ("orange", Color::Indexed(208)),
        ("purple", Color::Indexed(141)),
        ("pink", Color::Indexed(211)),
        ("teal", Color::Indexed(30)),
        ("brown", Color::Indexed(137)),
    ] {
        let parsed = name.parse::<FooterColor>().expect("a footer color name");
        assert_eq!(Color::from(parsed), expected, "{name}");
    }
}

#[test]
fn footer_color_names_are_exact() {
    for name in [
        "",
        "Red",
        "bright_blue",
        "gray",
        " blue",
        "blue ",
        "38;5;208",
    ] {
        assert!(name.parse::<FooterColor>().is_err(), "{name:?}");
    }
}

#[test]
fn footer_configuration_accepts_comments_blanks_and_assignment_spacing() {
    let colors = FooterColors::parse(
        "\n # colors shared with the footer\n\
         role.showrunner = bright-magenta # manager\n\
         role.unit-director=bright-blue\n\
         role.worker = bright-cyan\n\
         role.unassigned=purple\n\
         account.claude1 = cyan # account\n\
         account.codex1=orange\n\n",
    );
    for (role, name) in [
        (FooterRole::Showrunner, "bright-magenta"),
        (FooterRole::UnitDirector, "bright-blue"),
        (FooterRole::Worker, "bright-cyan"),
        (FooterRole::Unassigned, "purple"),
    ] {
        assert_eq!(
            colors.role(role),
            FooterTint::Footer(name.parse().expect("fixture color")),
            "{role:?}"
        );
    }
    for (account, name) in [("claude1", "cyan"), ("codex1", "orange")] {
        assert_eq!(
            colors.account(account),
            FooterTint::Footer(name.parse().expect("fixture color")),
            "{account}"
        );
    }
    assert_eq!(colors.account("claude2"), FooterTint::Plain);
}

#[test]
fn unrelated_keys_and_invalid_colors_leave_cells_plain() {
    let colors = FooterColors::parse(
        "role.showrunner=ultraviolet\n\
         role.worker=\n\
         role.session=red\n\
         level.high=red\n\
         tokens.high=orange\n\
         left.low=purple\n\
         claude1=cyan\n\
         account.claude2=not-a-color\n\
         account.split=red=blue\n\
         role.unit-director\n\
         # account.commented=green\n",
    );
    for role in [
        FooterRole::Showrunner,
        FooterRole::UnitDirector,
        FooterRole::Worker,
        FooterRole::Unassigned,
    ] {
        assert_eq!(colors.role(role), FooterTint::Plain, "{role:?}");
    }
    for account in ["claude1", "claude2", "split", "commented", "high", "low"] {
        assert_eq!(colors.account(account), FooterTint::Plain, "{account}");
    }
}

#[test]
fn missing_unreadable_and_removed_footer_files_leave_entries_plain() {
    let directory = tempfile::tempdir().expect("a temporary footer directory");
    let path = directory.path().join("statusline.conf");
    let mut file = FooterColorFile {
        source: FooterColorSource::File(path.clone()),
        ..FooterColorFile::default()
    };
    file.refresh();
    assert_eq!(file.colors().role(FooterRole::Worker), FooterTint::Plain);
    assert_eq!(file.colors().account("claude1"), FooterTint::Plain);

    std::fs::write(&path, "role.worker=orange\naccount.claude1=cyan")
        .expect("write a fixture footer");
    assert!(file.refresh(), "a newly available file changes the palette");
    assert_eq!(
        file.colors().role(FooterRole::Worker),
        FooterTint::Footer(FooterColor::Orange)
    );
    assert_eq!(
        file.colors().account("claude1"),
        FooterTint::Footer(FooterColor::Cyan)
    );

    std::fs::remove_file(&path).expect("remove the fixture footer");
    std::fs::create_dir(&path).expect("a directory cannot be read as a footer file");
    assert!(
        file.refresh(),
        "an unreadable source clears its previous colors"
    );
    assert_eq!(file.colors().role(FooterRole::Worker), FooterTint::Plain);
    assert_eq!(file.colors().account("claude1"), FooterTint::Plain);

    std::fs::remove_dir(&path).expect("remove the unreadable fixture source");
    std::fs::write(&path, "role.worker=orange\naccount.claude1=cyan")
        .expect("restore the fixture footer");
    assert!(file.refresh(), "a readable source restores its colors");
    std::fs::remove_file(&path).expect("remove the fixture footer");
    assert!(file.refresh(), "removing the file clears its colors");
    assert_eq!(file.colors().role(FooterRole::Worker), FooterTint::Plain);
    assert_eq!(file.colors().account("claude1"), FooterTint::Plain);
}

#[test]
fn footer_file_rereads_only_after_the_modification_time_changes() {
    let directory = tempfile::tempdir().expect("a temporary footer directory");
    let path = directory.path().join("statusline.conf");
    std::fs::write(&path, "account.claude1=red").expect("write the first palette");
    let first_revision = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    File::options()
        .write(true)
        .open(&path)
        .expect("open the fixture footer")
        .set_times(FileTimes::new().set_modified(first_revision))
        .expect("set the fixture revision");
    let mut file = FooterColorFile {
        source: FooterColorSource::File(path.clone()),
        ..FooterColorFile::default()
    };
    assert!(file.refresh());
    assert_eq!(
        file.colors().account("claude1"),
        FooterTint::Footer(FooterColor::Red)
    );

    std::fs::write(&path, "account.claude1=blue").expect("replace the palette");
    let fixture = File::options()
        .write(true)
        .open(&path)
        .expect("open the fixture footer");
    fixture
        .set_times(FileTimes::new().set_modified(first_revision))
        .expect("keep the same revision");
    assert!(
        !file.refresh(),
        "an unchanged revision keeps the parsed palette"
    );
    assert_eq!(
        file.colors().account("claude1"),
        FooterTint::Footer(FooterColor::Red)
    );

    fixture
        .set_times(FileTimes::new().set_modified(first_revision + Duration::from_secs(10)))
        .expect("advance the fixture revision");
    assert!(file.refresh(), "a changed revision reloads the palette");
    assert_eq!(
        file.colors().account("claude1"),
        FooterTint::Footer(FooterColor::Blue)
    );
}
