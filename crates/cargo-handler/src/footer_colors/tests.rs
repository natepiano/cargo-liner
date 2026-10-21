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

/// Proposed role assignments with the shared account assignments unchanged.
const ROLE_ACCOUNT_FIXTURE: &str = include_str!("fixtures/statusline.conf");
/// A distance above 10 reads as a different color at a glance; 12 keeps that margin.
const MINIMUM_LABEL_DISTANCE: f64 = 12.0;

/// D65 CIELAB coordinates used only by the independent perceptual oracle.
struct PerceptualColor {
    lightness:   f64,
    red_green:   f64,
    yellow_blue: f64,
}

impl PerceptualColor {
    fn from_terminal(color: Color) -> Self {
        let (red, green, blue) = measured_channels(color);
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= SRGB_LINEAR_THRESHOLD {
                value / SRGB_LINEAR_DIVISOR
            } else {
                ((value + SRGB_ENCODED_OFFSET) / SRGB_ENCODED_SCALE).powf(SRGB_POWER)
            }
        };
        let (red, green, blue) = (linear(red), linear(green), linear(blue));
        // The sRGB-to-XYZ matrix and D65 reference white define this conversion.
        let x = 0.180_437_5_f64.mul_add(blue, 0.357_576_1_f64.mul_add(green, 0.412_456_4 * red))
            / 0.95047;
        let y = 0.072_175_0_f64.mul_add(blue, 0.715_152_2_f64.mul_add(green, 0.212_672_9 * red));
        let z = 0.950_304_1_f64.mul_add(blue, 0.119_192_0_f64.mul_add(green, 0.019_333_9 * red))
            / 1.08883;
        let transform = |value: f64| {
            let delta = 6.0_f64 / 29.0;
            if value > delta.powi(3) {
                value.cbrt()
            } else {
                value / (3.0 * delta.powi(2)) + 4.0 / 29.0
            }
        };
        Self {
            lightness:   116.0f64.mul_add(transform(y), -16.0),
            red_green:   500.0 * (transform(x) - transform(y)),
            yellow_blue: 200.0 * (transform(y) - transform(z)),
        }
    }

    /// CIEDE2000 with unit lightness, chroma and hue weighting factors.
    fn distance(&self, other: &Self) -> f64 {
        let mean_chroma = f64::midpoint(
            self.red_green.hypot(self.yellow_blue),
            other.red_green.hypot(other.yellow_blue),
        );
        let chroma_weight =
            |chroma: f64| (chroma.powi(7) / (chroma.powi(7) + 25.0_f64.powi(7))).sqrt();
        let adjustment = (1.0 - chroma_weight(mean_chroma)) / 2.0;
        let first_red = (1.0 + adjustment) * self.red_green;
        let second_red = (1.0 + adjustment) * other.red_green;
        let first_chroma = first_red.hypot(self.yellow_blue);
        let second_chroma = second_red.hypot(other.yellow_blue);
        let hue = |red: f64, yellow: f64| yellow.atan2(red).to_degrees().rem_euclid(360.0);
        let first_hue = hue(first_red, self.yellow_blue);
        let second_hue = hue(second_red, other.yellow_blue);
        let hue_delta = if first_chroma * second_chroma == 0.0 {
            0.0
        } else if (second_hue - first_hue).abs() <= 180.0 {
            second_hue - first_hue
        } else if second_hue <= first_hue {
            second_hue - first_hue + 360.0
        } else {
            second_hue - first_hue - 360.0
        };
        let mean_hue = if first_chroma * second_chroma == 0.0 {
            first_hue + second_hue
        } else if (first_hue - second_hue).abs() <= 180.0 {
            f64::midpoint(first_hue, second_hue)
        } else if first_hue + second_hue < 360.0 {
            (first_hue + second_hue + 360.0) / 2.0
        } else {
            (first_hue + second_hue - 360.0) / 2.0
        };
        let cosine = |degrees: f64| degrees.to_radians().cos();
        let mean_lightness = f64::midpoint(self.lightness, other.lightness);
        let mean_chroma = f64::midpoint(first_chroma, second_chroma);
        let hue_scale = 0.20f64.mul_add(
            -cosine(4.0f64.mul_add(mean_hue, -63.0)),
            0.32f64.mul_add(
                cosine(3.0f64.mul_add(mean_hue, 6.0)),
                0.24f64.mul_add(
                    cosine(2.0 * mean_hue),
                    0.17f64.mul_add(-cosine(mean_hue - 30.0), 1.0),
                ),
            ),
        );
        let lightness_delta = (other.lightness - self.lightness)
            / (1.0
                + 0.015 * (mean_lightness - 50.0).powi(2)
                    / (mean_lightness - 50.0)
                        .mul_add(mean_lightness - 50.0, 20.0)
                        .sqrt());
        let chroma_delta = (second_chroma - first_chroma) / 0.045f64.mul_add(mean_chroma, 1.0);
        let hue_delta =
            2.0 * (first_chroma * second_chroma).sqrt() * (hue_delta / 2.0).to_radians().sin()
                / (0.015 * mean_chroma).mul_add(hue_scale, 1.0);
        let rotation = -2.0
            * chroma_weight(mean_chroma)
            * (60.0 * (-((mean_hue - 275.0) / 25.0).powi(2)).exp())
                .to_radians()
                .sin();
        (rotation * chroma_delta)
            .mul_add(
                hue_delta,
                hue_delta.mul_add(
                    hue_delta,
                    chroma_delta.mul_add(chroma_delta, lightness_delta.powi(2)),
                ),
            )
            .sqrt()
    }
}

/// Independent perceptual distance for rendered cell regressions.
pub(crate) fn perceptual_distance(first: Color, second: Color) -> f64 {
    PerceptualColor::from_terminal(first).distance(&PerceptualColor::from_terminal(second))
}

#[test]
fn footer_roles_and_accounts_stay_perceptually_distinct_from_column_headers() {
    // Published CIEDE2000 reference pairs check the independent oracle first.
    let reference = PerceptualColor {
        lightness:   50.0,
        red_green:   0.0,
        yellow_blue: -82.7485,
    };
    for (red_green, yellow_blue, expected) in
        [(2.6772, -79.7751, 2.0425), (3.1571, -77.2803, 2.8615)]
    {
        let sample = PerceptualColor {
            lightness: 50.0,
            red_green,
            yellow_blue,
        };
        assert!(
            (sample.distance(&reference) - expected).abs() < 0.0001,
            "perceptual oracle reference pair"
        );
    }
    let colors = FooterColors::parse(ROLE_ACCOUNT_FIXTURE);
    let previous = tui_pane::theme();
    let mut measurements = Vec::new();
    for variant in theme::builtins() {
        let name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        let header = PerceptualColor::from_terminal(tui_pane::label_color());
        let entries = [
            ("role.showrunner", colors.role(FooterRole::Showrunner)),
            ("role.unit-director", colors.role(FooterRole::UnitDirector)),
            ("account.claude1", colors.account("claude1")),
            ("account.claude2", colors.account("claude2")),
            ("account.claude3", colors.account("claude3")),
            ("account.claude4", colors.account("claude4")),
            ("account.codex1", colors.account("codex1")),
            ("account.codex2", colors.account("codex2")),
        ];
        for (entry, tint) in entries {
            let ink = tint
                .style(Style::default())
                .fg
                .expect("configured foreground");
            let distance = PerceptualColor::from_terminal(ink).distance(&header);
            measurements.push((name.clone(), entry, distance));
        }
    }
    tui_pane::set_active_theme(previous);
    for (name, entry, distance) in &measurements {
        eprintln!("{name} {entry}: CIEDE2000={distance:.3}");
    }
    assert!(
        measurements
            .iter()
            .all(|(_, _, distance)| *distance >= MINIMUM_LABEL_DISTANCE),
        "role and account colors must differ from headers by at least {MINIMUM_LABEL_DISTANCE}: {measurements:?}"
    );
}

/// Resolve a fixture-only future role without extending the production role enum.
fn fixture_role_tint(key: &str) -> FooterTint {
    let name = ROLE_ACCOUNT_FIXTURE
        .lines()
        .find_map(|line| {
            line.strip_prefix(key)
                .and_then(|tail| tail.strip_prefix('='))
        })
        .expect("fixture role assignment");
    FooterTint::Footer(name.parse().expect("fixture role color"))
}

/// Management colors cannot reuse the footer levels or the reserved designer ink.
fn assert_management_role_names(roles: [(&str, FooterTint); 2]) {
    for (role, tint) in roles {
        assert!(
            !matches!(
                tint,
                FooterTint::Management(
                    FooterColor::Red
                        | FooterColor::BrightRed
                        | FooterColor::Green
                        | FooterColor::BrightGreen
                        | FooterColor::Pink
                )
            ),
            "{role} must not reuse level or designer colors"
        );
    }
}

/// Roles must not reuse account, level, Astra or header colors after correction.
#[test]
fn footer_role_colors_stay_distinct_from_accounts_headers_and_other_roles() {
    let colors = FooterColors::parse(ROLE_ACCOUNT_FIXTURE);
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let mut measurements = Vec::new();
    let mut contrasts = Vec::new();
    let role_tints = [
        ("role.showrunner", colors.role(FooterRole::Showrunner)),
        ("role.unit-director", colors.role(FooterRole::UnitDirector)),
    ];
    assert_management_role_names(role_tints);
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins() {
        let theme_name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for focused_tint in [false, true] {
            tui_pane::set_focused_pane_tint(focused_tint);
            let appearance = format!("{theme_name} tint={focused_tint}");
            let ink = |tint: FooterTint| {
                tint.style(Style::default())
                    .fg
                    .expect("configured foreground")
            };
            let roles = role_tints.map(|(role, tint)| (role, ink(tint)));
            let accounts = [
                "claude1", "claude2", "claude3", "claude4", "codex1", "codex2",
            ]
            .map(|account| (account, ink(colors.account(account))));
            let astra = ["role.astra-designer", "role.astra-reviewer"]
                .map(|role| (role, ink(fixture_role_tint(role))));
            let header = tui_pane::label_color();
            let plain = FooterTint::Plain
                .style(Style::default().fg(tui_pane::text_default()))
                .fg
                .expect("plain row foreground");
            let distance = |first, second| {
                PerceptualColor::from_terminal(first)
                    .distance(&PerceptualColor::from_terminal(second))
            };
            for (index, (role, color)) in roles.iter().enumerate() {
                measurements.push((
                    appearance.clone(),
                    *role,
                    "header",
                    distance(*color, header),
                ));
                measurements.push((
                    appearance.clone(),
                    *role,
                    "plain row text",
                    distance(*color, plain),
                ));
                for (other, other_color) in accounts
                    .iter()
                    .chain(&astra)
                    .chain(roles.iter().skip(index + 1))
                {
                    measurements.push((
                        appearance.clone(),
                        *role,
                        *other,
                        distance(*color, *other_color),
                    ));
                }
                for focused in [false, true] {
                    contrasts.push((
                        appearance.clone(),
                        *role,
                        focused,
                        contrast_ratio(*color, tui_pane::pane_background(focused)),
                    ));
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    assert_role_measurements(&measurements, &contrasts);
}

/// Report every measured pair before checking the complete palette.
fn assert_role_measurements(
    measurements: &[(String, &str, &str, f64)],
    contrasts: &[(String, &str, bool, f64)],
) {
    for (theme, role, other, distance) in measurements {
        eprintln!("{theme} {role} / {other}: CIEDE2000={distance:.6}");
    }
    for (theme, role, focused, ratio) in contrasts {
        eprintln!("{theme} {role} focused={focused}: contrast={ratio:.6}");
    }
    let failures: Vec<_> = measurements
        .iter()
        .filter(|(_, _, _, distance)| *distance < MINIMUM_LABEL_DISTANCE)
        .collect();
    let unreadable: Vec<_> = contrasts
        .iter()
        .filter(|(_, _, _, ratio)| *ratio < MINIMUM_CONTRAST)
        .collect();
    assert!(
        failures.is_empty() && unreadable.is_empty(),
        "role distance failures: {failures:?}; pane contrast failures: {unreadable:?}"
    );
}

/// Independent xterm channel oracle, shared by WCAG and CIEDE2000 tests.
fn measured_channels(color: Color) -> (u8, u8, u8) {
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

/// Model the production stem coverage independently of its channel blending.
/// Arbitrary font fringes can have coverage approaching zero.
pub(crate) fn covered_role_contrast(foreground: Color, background: Color) -> f64 {
    let ground_share = u32::from(255 - ANTIALIASED_GLYPH_COVERAGE);
    let foreground = <[u8; 3]>::from(measured_channels(foreground));
    let ground = <[u8; 3]>::from(measured_channels(background));
    let mixed: [u8; 3] = std::array::from_fn(|index| {
        let channel = (u32::from(foreground[index]) * (255 - ground_share)
            + u32::from(ground[index]) * ground_share)
            / 255;
        u8::try_from(channel).expect("convex channel mixture fits a byte")
    });
    contrast_ratio(Color::Rgb(mixed[0], mixed[1], mixed[2]), background)
}

/// Independent WCAG oracle: ANSI values follow the documented xterm approximation.
pub(crate) fn contrast_ratio(foreground: Color, background: Color) -> f64 {
    fn luminance(color: Color) -> f64 {
        let (red, green, blue) = measured_channels(color);
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
            let colors = FooterColors::parse(&format!(
                "account.fixture={name}\nrole.unit-director={name}"
            ));
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
                .role(FooterRole::UnitDirector)
                .style(Style::default())
                .fg
                .expect("role ink");
            let account_source = match original {
                Color::Blue | Color::LightBlue => Color::Rgb(0, 100, 255),
                Color::Indexed(30) => Color::Rgb(0, 135, 160),
                color => color,
            };
            for (foreground, original) in [(account, account_source), (role, original)] {
                let ratio = contrast_ratio(foreground, background);
                assert!(
                    ratio >= MINIMUM_CONTRAST,
                    "{} {name}: {foreground:?} on {background:?} is {ratio}:1",
                    variant.id.as_str()
                );
                if [false, true].into_iter().all(|focused| {
                    let ground = tui_pane::pane_background(focused);
                    contrast_ratio(original, ground) >= MINIMUM_CONTRAST
                        && covered_role_contrast(original, ground) >= MINIMUM_CONTRAST
                }) {
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
                let base = Style::default().add_modifier(ratatui::style::Modifier::BOLD);
                let styled = colors.account("fixture").style(base);
                assert_eq!(styled.add_modifier, base.add_modifier);
                let actual = styled.fg.expect("blue ink");
                for focused in [false, true] {
                    let background = tui_pane::pane_background(focused);
                    assert!(
                        contrast_ratio(actual, background) >= MINIMUM_CONTRAST,
                        "{} transparent={transparent} tinted={focused_tint} focused={focused}: {actual:?} on {background:?}",
                        variant.id.as_str()
                    );
                }
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
    ] {
        assert_eq!(
            colors.role(role),
            FooterTint::Management(name.parse().expect("fixture color")),
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
    for role in [FooterRole::Showrunner, FooterRole::UnitDirector] {
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
    assert_eq!(
        file.colors().role(FooterRole::UnitDirector),
        FooterTint::Plain
    );
    assert_eq!(file.colors().account("claude1"), FooterTint::Plain);

    std::fs::write(&path, "role.unit-director=orange\naccount.claude1=cyan")
        .expect("write a fixture footer");
    assert!(file.refresh(), "a newly available file changes the palette");
    assert_eq!(
        file.colors().role(FooterRole::UnitDirector),
        FooterTint::Management(FooterColor::Orange)
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
    assert_eq!(
        file.colors().role(FooterRole::UnitDirector),
        FooterTint::Plain
    );
    assert_eq!(file.colors().account("claude1"), FooterTint::Plain);

    std::fs::remove_dir(&path).expect("remove the unreadable fixture source");
    std::fs::write(&path, "role.unit-director=orange\naccount.claude1=cyan")
        .expect("restore the fixture footer");
    assert!(file.refresh(), "a readable source restores its colors");
    std::fs::remove_file(&path).expect("remove the fixture footer");
    assert!(file.refresh(), "removing the file clears its colors");
    assert_eq!(
        file.colors().role(FooterRole::UnitDirector),
        FooterTint::Plain
    );
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
