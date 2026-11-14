//! Matched inks retain their hue and readability on every selection ground.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ratatui::buffer::Cell;
use ratatui::layout::Position;
use tui_pane::ExpansionAction;
use tui_pane::TerminalApp;
use tui_pane::TileView;

use super::selection_tests;
use super::word_click_tests;
use super::word_click_tests::Renderer;
use super::word_click_tests::TestResult;
use super::*;
use crate::app::expansion;
use crate::app::expansion::HandlerExpansionAction;
use crate::config::WordHighlightEmphasis;
use crate::footer_colors;
use crate::theme;
use crate::theme::ResetThemeFixture;
use crate::theme::SessionHue;
use crate::theme::WordMatchLimit;
use crate::theme::WordMatchSurface;
use crate::word_highlight::HighlightWord;
use crate::word_highlight::PaintedWords;
use crate::word_highlight::WordHighlight;
use crate::word_highlight::WordHit;

const MINIMUM_CONTRAST: f64 = 4.5;
const WAITING_HUE_TOLERANCE: f64 = 0.25;
// The `palette_application` sessions set `started` to 100; at `BEFORE_AGE_STEP`, their age is four
// minutes.
const BEFORE_AGE_STEP: u64 = 399;
const DEFAULT_DARK_FOCUSED_SELECTION: Color = Color::Rgb(86, 86, 86);
const DEFAULT_DARK_FOCUSED_MATCH_CAP: Color = Color::Rgb(101, 101, 101);
// The CIEDE2000 distance retains a floor of three for every theme.
const MINIMUM_VISIBLE_GROUND_DIFFERENCE: f64 = 3.0;
const LOW_CAPACITY_FOCUSED_SELECTION: Color = Color::Rgb(112, 112, 112);
const LOW_CAPACITY_UNFOCUSED_SELECTION: Color = Color::Rgb(113, 113, 113);
const DARK_CUSTOM_FOCUS_GROUND: Color = Color::Rgb(100, 100, 100);
const LIGHT_CUSTOM_FOCUS_GROUND: Color = Color::Rgb(180, 180, 180);
// This width keeps every fixture word visible without unused columns.
const WIDTH: u16 = 100;
const HEIGHT: u16 = 70;
const FOOTER: &str = "role.showrunner=purple\nrole.unit-director=yellow\naccount.claude1=brown\naccount.claude2=magenta\naccount.claude3=blue\naccount.claude4=orange\naccount.codex1=pink\naccount.codex2=teal\n";
const ACCOUNTS: [&str; 6] = [
    "claude1", "claude2", "claude3", "claude4", "codex1", "codex2",
];

#[derive(Clone, Copy, Debug)]
enum MatchGround {
    Base,
    SelectedFocused,
    SelectedUnfocused,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContrastWords {
    Sessions,
    Accounts,
    Table,
}

impl ContrastWords {
    fn of(word: &str) -> Self {
        if word.starts_with("palette-") {
            Self::Sessions
        } else if ACCOUNTS.contains(&word) {
            Self::Accounts
        } else {
            Self::Table
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum PaneTint {
    Enabled,
    Disabled,
}

impl PaneTint {
    const fn enabled(self) -> bool { matches!(self, Self::Enabled) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectionDistanceLimit {
    BoundReached,
    ReadabilityCapped,
}

/// `UnmatchedFrame` keeps the unmarked background for one `MatchGround`.
struct UnmatchedFrame {
    plain_match: Color,
    buffer:      Buffer,
    words:       PaintedWords,
    ground:      MatchGround,
    tile_ground: Color,
}

fn rgb(color: Color) -> TestResult<[f64; 3]> {
    let Color::Rgb(red, green, blue) = tui_pane::blend_color(color, color, 0) else {
        return Err(format!("unresolved drawn color {color:?}").into());
    };
    Ok([red, green, blue].map(f64::from))
}

fn luminance(channels: [f64; 3]) -> f64 {
    let linear = channels.map(|channel| {
        let value = channel / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126f64.mul_add(linear[0], 0.7152f64.mul_add(linear[1], 0.0722 * linear[2]))
}

fn glyph_contrast(ink: Color, ground: Color) -> TestResult<f64> {
    let ink = rgb(ink)?;
    let ground = rgb(ground)?;
    let covered =
        std::array::from_fn(|index| f64::mul_add(ground[index], 47.0, ink[index] * 208.0) / 255.0);
    let ground_luminance = luminance(ground);
    Ok([covered, covered.map(f64::round)]
        .into_iter()
        .map(|channels| {
            let ink_luminance = luminance(channels);
            (ink_luminance.max(ground_luminance) + 0.05)
                / (ink_luminance.min(ground_luminance) + 0.05)
        })
        .fold(f64::INFINITY, f64::min))
}

fn palette_application(path: &Path) -> TestResult<App> {
    std::fs::write(path, FOOTER)?;
    let mut app = word_click_tests::application()?;
    app.footer_colors = crate::footer_colors::FooterColorFile::fixture(path.to_owned());
    app.footer_colors.refresh();
    let mut rows = Vec::new();
    for (index, account) in ACCOUNTS.iter().enumerate() {
        let pid = 100 + u32::try_from(index)? * 10;
        let role = if index == 0 { "showrunner" } else { "reviewer" };
        rows.push(serde_json::from_value(serde_json::json!({
            "agent":if account.starts_with("codex") {"codex"} else {"claude"},
            "pid":pid,"session_id":format!("palette-{index}"),"name":format!("palette-owner-{index}"),
            "started":100,"directory":"/fixture","activity":{"active":"thinking"},
            "account":{"home":format!("~/.{account}"),"label":account},
            "children":[
                {"kind":"shell","pid":pid+1,"depth":0,"name":format!("palette-work-{index}"),"started":101},
                {"kind":"subagent","depth":0,"name":format!("palette-review-{index}"),"subagent_role":{"Specialist":role},"started":102}
            ]
        }))?);
    }
    word_click_tests::apply_rows(&mut app, rows);
    Ok(app)
}

fn targets(app: &App, buffer: &Buffer) -> Vec<(String, Position)> {
    let painted = crate::word_highlight::PaintedWords::capture(buffer, app.tiles.pointer_regions());
    let mut inks = Vec::new();
    let mut targets = Vec::new();
    for point in buffer.area.positions() {
        let cell = &buffer[(point.x, point.y)];
        if cell.symbol().chars().any(char::is_alphanumeric)
            && !inks.contains(&cell.fg)
            && let WordHit::Word(word) = painted.hit(point)
        {
            inks.push(cell.fg);
            targets.push((word.text().to_owned(), point));
        }
    }
    targets
}

/// `InkHsl` measures the hue and lightness of one drawn RGB ink.
struct InkHsl {
    hue:       f64,
    chroma:    f64,
    lightness: f64,
}

impl InkHsl {
    fn from_drawn(color: Color) -> TestResult<Self> {
        let [red, green, blue] = rgb(color)?.map(|channel| channel / 255.0);
        let maximum = red.max(green).max(blue);
        let minimum = red.min(green).min(blue);
        let chroma = maximum - minimum;
        let sector = if chroma <= f64::EPSILON {
            0.0
        } else if red >= green && red >= blue {
            ((green - blue) / chroma).rem_euclid(6.0)
        } else if green >= blue {
            (blue - red) / chroma + 2.0
        } else {
            (red - green) / chroma + 4.0
        };
        Ok(Self {
            hue: sector * 60.0,
            chroma,
            lightness: f64::midpoint(maximum, minimum),
        })
    }
}

fn hue_distance(first: f64, second: f64) -> f64 {
    let distance = (first - second).rem_euclid(360.0);
    distance.min(360.0 - distance)
}

/// `InkHsl::hue` permits only the hue error within the neighboring one-byte RGB cube.
fn assert_lift_keeps_hue(original: Color, lifted: Color) -> TestResult {
    let source = InkHsl::from_drawn(original)?;
    let result = InkHsl::from_drawn(lifted)?;
    let Color::Rgb(red, green, blue) = tui_pane::blend_color(lifted, lifted, 0) else {
        return Err("the lifted ink must resolve to RGB".into());
    };
    if source.chroma <= f64::EPSILON {
        assert!(
            result.chroma <= 1.0 / 255.0,
            "an achromatic ink stays achromatic"
        );
        return Ok(());
    }
    let mut hue_tolerance = 0.0_f64;
    for red in [red.saturating_sub(1), red.saturating_add(1)] {
        for green in [green.saturating_sub(1), green.saturating_add(1)] {
            for blue in [blue.saturating_sub(1), blue.saturating_add(1)] {
                let neighbor = InkHsl::from_drawn(Color::Rgb(red, green, blue))?;
                hue_tolerance = hue_tolerance.max(hue_distance(neighbor.hue, result.hue));
            }
        }
    }
    let error = hue_distance(source.hue, result.hue);
    assert!(
        error <= hue_tolerance,
        "lifted ink keeps its HSL hue within one RGB byte: {original:?} -> {lifted:?}; hue error={error}, hue tolerance={hue_tolerance}"
    );
    Ok(())
}

fn assert_matched_ink(before: &Cell, after: &Cell, word: &str, point: Position) -> TestResult {
    let match_readability_bound = MINIMUM_CONTRAST.min(glyph_contrast(before.fg, before.bg)?);
    let original_on_match = glyph_contrast(before.fg, after.bg)?;
    if original_on_match >= match_readability_bound {
        assert_eq!(
            after.fg, before.fg,
            "{word}: sufficient ink stays exact at {point:?}"
        );
    } else {
        assert_ne!(
            after.fg, before.fg,
            "{word}: insufficient ink lifts at {point:?}"
        );
        assert_lift_keeps_hue(before.fg, after.fg)?;
    }
    Ok(())
}

/// Resolve each matching token once before inspecting its glyph cells.
fn matching_word_cells(
    painted: &PaintedWords,
    buffer: &Buffer,
    word: &str,
) -> TestResult<Vec<Position>> {
    let width = u16::try_from(unicode_width::UnicodeWidthStr::width(word))?;
    let mut cells = Vec::new();
    for start in word_click_tests::positions(buffer, word) {
        if matches!(painted.hit(start), WordHit::Word(token) if token.text() == word) {
            let end = start.x.saturating_add(width).min(buffer.area.right());
            cells.extend((start.x..end).map(|x| Position::new(x, start.y)));
        }
    }
    Ok(cells)
}

fn measure_matches(
    app: &mut App,
    renderer: &mut Renderer,
    word: &str,
    reference: &UnmatchedFrame,
    failures: &mut Vec<String>,
) -> TestResult {
    app.word_highlight.clear();
    let before = &reference.buffer;
    let ground = reference.ground;
    let WordHit::Word(token) = HighlightWord::from_token(word) else {
        return Err(format!("drawn token {word} must be nonempty").into());
    };
    app.word_highlight = WordHighlight::Word(token);
    let background = reference.plain_match;
    // Draw the first match through App. Reuse its unchanged text for later words.
    let (after, painted) = if app.painted_words.is_uncaptured() {
        let after = renderer.paint(app)?;
        (after, &app.painted_words)
    } else {
        let mut after = before.clone();
        reference.words.apply_with_inks(
            &app.word_highlight,
            &mut after,
            word_click_tests::match_background()?,
            app.loaded_config.config.word_highlight.emphasis,
            &mut app.match_inks,
        );
        (after, &reference.words)
    };
    let mut count = 0;
    let mut minimum = f64::INFINITY;
    let mut selected_grounds = Vec::new();
    for point in matching_word_cells(painted, &after, word)? {
        let cell = &after[(point.x, point.y)];
        if cell.symbol().chars().any(char::is_alphanumeric) && word.contains(cell.symbol()) {
            let unmatched = &before[(point.x, point.y)];
            if unmatched.bg == tui_pane::active_focus_color()
                || unmatched.bg == tui_pane::remembered_focus_color()
            {
                let grounds = (unmatched.bg, cell.bg);
                if !selected_grounds.contains(&grounds) {
                    selected_grounds.push(grounds);
                }
            } else {
                assert_eq!(
                    cell.bg, background,
                    "{word}: an unselected match uses the plain match ground"
                );
            }
            count += 1;
            assert_matched_ink(&before[(point.x, point.y)], cell, word, point)?;
            let ratio = glyph_contrast(cell.fg, cell.bg)?;
            minimum = minimum.min(ratio);
            let unmatched = &before[(point.x, point.y)];
            let unmatched_ratio = glyph_contrast(unmatched.fg, unmatched.bg)?;
            let match_readability_bound = MINIMUM_CONTRAST.min(unmatched_ratio);
            if ratio < match_readability_bound {
                failures.push(format!("the match never makes text less readable than today: {ground:?} {word} at {point:?}: ink={:?} match_background={:?} unmatched_background={:?} ratio_on_match={ratio:.9} ratio_on_unmatched_ground={unmatched_ratio:.9} match_readability_bound={match_readability_bound:.9}",cell.fg,cell.bg,unmatched.bg));
            }
        }
    }
    assert!(count > 0, "{word} must have a drawn match on {ground:?}");
    eprintln!("WORD_CONTRAST {ground:?} word={word} minimum={minimum:.9}");
    for (selection, selected_match) in selected_grounds {
        if app.loaded_config.config.word_highlight.emphasis == WordHighlightEmphasis::Subtle {
            assert_selected_match_distinct(
                selection,
                background,
                selected_match,
                reference.tile_ground,
            );
        } else {
            for original in [selection, background] {
                assert!(
                    footer_colors::perceptual_distance(selected_match, original)
                        >= MINIMUM_VISIBLE_GROUND_DIFFERENCE
                );
            }
        }
    }
    Ok(())
}

fn check_theme(
    name: &str,
    tint: PaneTint,
    ground: MatchGround,
    words: ContrastWords,
) -> TestResult {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let builtin = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == name)
        .ok_or("built-in theme")?;
    tui_pane::set_active_theme(Arc::new(builtin.theme));
    tui_pane::set_transparent_background(false);
    let result = measure_theme(tint, ground, words);
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    let failures = result?;
    assert!(failures.is_empty(), "{name}: {failures:#?}");
    Ok(())
}

fn measure_theme(
    tint: PaneTint,
    ground: MatchGround,
    subset: ContrastWords,
) -> TestResult<Vec<String>> {
    let directory = tempfile::tempdir()?;
    let mut failures = Vec::new();
    tui_pane::set_focused_pane_tint(tint.enabled());
    let mut app = palette_application(&directory.path().join("statusline.conf"))?;
    let mut renderer = word_click_tests::Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let initial = renderer.paint(&mut app)?;
    let selected = word_click_tests::locate(&initial, "palette-review-0")?;
    match ground {
        MatchGround::Base => {},
        MatchGround::SelectedFocused => app.click(selected),
        MatchGround::SelectedUnfocused => {
            app.click(selected);
            renderer.paint(&mut app)?;
            app.click(word_click_tests::locate(&initial, "palette-work-5")?);
        },
    }
    app.word_highlight.clear();
    let baseline = if matches!(ground, MatchGround::Base) {
        initial
    } else {
        renderer.paint(&mut app)?
    };
    let baseline = if app.tiles.has_pending_motion_for_test() {
        app.tiles.settle_for_test();
        renderer.paint(&mut app)?
    } else {
        baseline
    };
    match ground {
        MatchGround::Base => {
            assert_ne!(
                baseline[(selected.x, selected.y)].bg,
                tui_pane::active_focus_color()
            );
            assert_ne!(
                baseline[(selected.x, selected.y)].bg,
                tui_pane::remembered_focus_color()
            );
        },
        MatchGround::SelectedFocused => assert_eq!(
            baseline[(selected.x, selected.y)].bg,
            tui_pane::active_focus_color()
        ),
        MatchGround::SelectedUnfocused => assert_eq!(
            baseline[(selected.x, selected.y)].bg,
            tui_pane::remembered_focus_color()
        ),
    }
    let mut words = targets(&app, &baseline);
    assert!(
        words.len() >= 8,
        "the drawn fixture must include session, account, role, and status inks"
    );
    for text in ["palette-review-0", "claude1", "showrunner"] {
        if !words.iter().any(|(word, _)| word == text) {
            words.push((text.to_owned(), word_click_tests::locate(&baseline, text)?));
        }
    }
    // `measure_matches` checks every occurrence, so each word needs one frame.
    words.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    words.dedup_by(|left, right| left.0 == right.0);
    // `WordMatchGrounds` uses the theme ground for tinted and untinted frames.
    let tile_ground = tui_pane::theme().text.bg_focus.color;
    let words_in_frame = PaintedWords::capture(&baseline, app.tiles.pointer_regions());
    let mut reference = UnmatchedFrame {
        plain_match: word_click_tests::match_background()?,
        buffer: baseline,
        words: words_in_frame,
        ground,
        tile_ground,
    };
    for emphasis in [
        WordHighlightEmphasis::Subtle,
        WordHighlightEmphasis::Clear,
        WordHighlightEmphasis::Strong,
    ] {
        app.loaded_config.config.word_highlight.emphasis = emphasis;
        reference.plain_match =
            theme::WordMatchGrounds::new(word_click_tests::match_background()?, emphasis)
                .background(WordMatchSurface::Plain);
        for (word, _) in words
            .iter()
            .filter(|(word, _)| ContrastWords::of(word) == subset)
        {
            measure_matches(&mut app, &mut renderer, word, &reference, &mut failures)?;
        }
    }
    Ok(failures)
}

/// `MatchedInkFrames` retains the same drawn layout before and after one word is set.
struct MatchedInkFrames {
    before:     Buffer,
    after:      Buffer,
    background: Color,
}

fn single_word_frames(name: &str, word: &str) -> TestResult<MatchedInkFrames> {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let builtin = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == name)
        .ok_or("built-in theme")?;
    tui_pane::set_active_theme(Arc::new(builtin.theme));
    tui_pane::set_transparent_background(false);
    tui_pane::set_focused_pane_tint(false);
    let result = (|| {
        let directory = tempfile::tempdir()?;
        let mut app = palette_application(&directory.path().join("statusline.conf"))?;
        let mut renderer = Renderer::with_fixed_time(&mut app, WIDTH, HEIGHT, BEFORE_AGE_STEP)?;
        let before = renderer.paint(&mut app)?;
        app.started = app
            .started
            .checked_sub(Duration::from_secs(1))
            .ok_or("the fixture start must support a one-second uptime step")?;
        let WordHit::Word(token) = HighlightWord::from_token(word) else {
            return Err("the fixture word must be nonempty".into());
        };
        app.word_highlight = WordHighlight::Word(token);
        let after = renderer.paint(&mut app)?;
        Ok(MatchedInkFrames {
            before,
            after,
            background: word_click_tests::match_background()?,
        })
    })();
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    result
}

/// A one-byte step toward the original ink must lose the required contrast.
fn assert_nearest_byte_step(
    original: Color,
    lifted: Color,
    ground: Color,
    bound: f64,
) -> TestResult {
    let Color::Rgb(red, green, blue) = tui_pane::blend_color(lifted, lifted, 0) else {
        return Err("the lifted ink must resolve to RGB".into());
    };
    let source = rgb(original)?;
    let mut channels = [red, green, blue];
    for (index, channel) in channels.iter_mut().enumerate() {
        if f64::from(*channel) > source[index] {
            *channel = channel.saturating_sub(1);
        } else if f64::from(*channel) < source[index] {
            *channel = channel.saturating_add(1);
        }
    }
    let [red, green, blue] = channels;
    let nearer = Color::Rgb(red, green, blue);
    let ratio = glyph_contrast(nearer, ground)?;
    assert!(
        ratio < bound,
        "the nearer RGB byte step must fail: {original:?} -> {lifted:?}; nearer={nearer:?}, ratio={ratio}, bound={bound}"
    );
    let source_lightness = InkHsl::from_drawn(original)?.lightness;
    let lifted_lightness = InkHsl::from_drawn(lifted)?.lightness;
    let ground_lightness = InkHsl::from_drawn(ground)?.lightness;
    assert!(
        (ground_lightness < 0.5 && lifted_lightness > source_lightness)
            || (ground_lightness >= 0.5 && lifted_lightness < source_lightness),
        "the ink lightness must move away from the match ground"
    );
    Ok(())
}

fn check_named_lift(name: &str, word: &str) -> TestResult {
    let frames = single_word_frames(name, word)?;
    let point = word_click_tests::locate(&frames.before, word)?;
    let before = &frames.before[(point.x, point.y)];
    let after = &frames.after[(point.x, point.y)];
    let match_readability_bound = MINIMUM_CONTRAST.min(glyph_contrast(before.fg, before.bg)?);
    assert!(
        glyph_contrast(before.fg, frames.background)? < match_readability_bound,
        "{name} {word} must exercise an ink lift"
    );
    assert_eq!(after.bg, frames.background);
    assert_matched_ink(before, after, word, point)?;
    assert_nearest_byte_step(before.fg, after.fg, after.bg, match_readability_bound)?;
    let ratio_on_match = glyph_contrast(after.fg, after.bg)?;
    assert!(
        ratio_on_match >= match_readability_bound,
        "the match never makes text less readable than today: {name} {word}; ratio_on_match={ratio_on_match}, match_readability_bound={match_readability_bound}"
    );
    eprintln!(
        "NAMED_WORD_CONTRAST {name} {word}: {ratio_on_match:.9} bound={match_readability_bound:.9}"
    );
    Ok(())
}

fn check_unmatched_ink(name: &str) -> TestResult {
    let word = "claude1";
    let frames = single_word_frames(name, word)?;
    let word_width = u16::try_from(word.len())?;
    let word_cells: Vec<_> = word_click_tests::positions(&frames.before, word)
        .into_iter()
        .map(|point| Rect::new(point.x, point.y, word_width, 1))
        .collect();
    let mut matched = 0;
    let mut unmatched = 0;
    for point in frames.after.area.positions() {
        let after = &frames.after[(point.x, point.y)];
        let before = &frames.before[(point.x, point.y)];
        if word_cells.iter().any(|cells| cells.contains(point)) {
            matched += 1;
        } else if after.symbol().chars().any(char::is_alphanumeric) {
            unmatched += 1;
            assert_eq!(
                after.fg, before.fg,
                "{name}: unmatched ink stays exact at {point:?}"
            );
            assert_eq!(after.symbol(), before.symbol());
        }
    }
    assert!(matched > 0, "the word must mark drawn cells");
    assert!(unmatched > 0, "the frame must retain unmatched text");
    Ok(())
}

#[test]
fn magenta_matched_ink_lifts_along_its_hue() -> TestResult {
    check_named_lift("Default Dark", "claude2")
}

#[test]
fn pink_matched_ink_lifts_along_its_hue() -> TestResult {
    check_named_lift("Default Light", "codex1")
}

#[test]
fn default_light_header_matched_ink_lifts_along_its_hue() -> TestResult {
    check_named_lift("Default Light", "account")
}

#[test]
fn unmatched_text_keeps_its_ink_in_default_dark() -> TestResult {
    check_unmatched_ink("Default Dark")
}

#[test]
fn unmatched_text_keeps_its_ink_in_default_light() -> TestResult {
    check_unmatched_ink("Default Light")
}

#[test]
fn unmatched_text_keeps_its_ink_in_high_contrast_dark() -> TestResult {
    check_unmatched_ink("High Contrast Dark")
}

#[test]
fn unmatched_text_keeps_its_ink_in_high_contrast_light() -> TestResult {
    check_unmatched_ink("High Contrast Light")
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_session_words() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_account_words() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_selected_focused() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_selected_focused_session_words() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_selected_focused_account_words() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_selected_unfocused_table_words() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_selected_unfocused_session_words() -> TestResult
{
    check_theme(
        "Default Dark",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_selected_unfocused_account_words() -> TestResult
{
    check_theme(
        "Default Dark",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_tinted_base() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_tinted_base_session_words() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_tinted_base_account_words() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_tinted_selected_focused() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_tinted_selected_focused_session_words()
-> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_tinted_selected_focused_account_words()
-> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_tinted_selected_unfocused() -> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_tinted_selected_unfocused_session_words()
-> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_dark_tinted_selected_unfocused_account_words()
-> TestResult {
    check_theme(
        "Default Dark",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light() -> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_session_words() -> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_account_words() -> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_selected_focused() -> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_selected_focused_session_words() -> TestResult
{
    check_theme(
        "Default Light",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_selected_focused_account_words() -> TestResult
{
    check_theme(
        "Default Light",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_selected_unfocused() -> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_selected_unfocused_session_words() -> TestResult
{
    check_theme(
        "Default Light",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_selected_unfocused_account_words() -> TestResult
{
    check_theme(
        "Default Light",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_tinted_base() -> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_tinted_base_session_words() -> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_tinted_base_account_words() -> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_tinted_selected_focused() -> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_tinted_selected_focused_session_words()
-> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_tinted_selected_focused_account_words()
-> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_tinted_selected_unfocused() -> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_tinted_selected_unfocused_session_words()
-> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_default_light_tinted_selected_unfocused_account_words()
-> TestResult {
    check_theme(
        "Default Light",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark() -> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_session_words() -> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_account_words() -> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_selected_focused() -> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_selected_focused_session_words()
-> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_selected_focused_account_words()
-> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_selected_unfocused() -> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_selected_unfocused_session_words()
-> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_selected_unfocused_account_words()
-> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_tinted_base() -> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_tinted_base_session_words() -> TestResult
{
    check_theme(
        "High Contrast Dark",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_tinted_base_account_words() -> TestResult
{
    check_theme(
        "High Contrast Dark",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_tinted_selected_focused() -> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_tinted_selected_focused_session_words()
-> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_tinted_selected_focused_account_words()
-> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_tinted_selected_unfocused() -> TestResult
{
    check_theme(
        "High Contrast Dark",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_tinted_selected_unfocused_session_words()
-> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_dark_tinted_selected_unfocused_account_words()
-> TestResult {
    check_theme(
        "High Contrast Dark",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light() -> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_session_words() -> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_account_words() -> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Disabled,
        MatchGround::Base,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_selected_focused() -> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_selected_focused_session_words()
-> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_selected_focused_account_words()
-> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Disabled,
        MatchGround::SelectedFocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_selected_unfocused() -> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_selected_unfocused_session_words()
-> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_selected_unfocused_account_words()
-> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Disabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_tinted_base() -> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_tinted_base_session_words() -> TestResult
{
    check_theme(
        "High Contrast Light",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_tinted_base_account_words() -> TestResult
{
    check_theme(
        "High Contrast Light",
        PaneTint::Enabled,
        MatchGround::Base,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_tinted_selected_focused() -> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_tinted_selected_focused_session_words()
-> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_tinted_selected_focused_account_words()
-> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Enabled,
        MatchGround::SelectedFocused,
        ContrastWords::Accounts,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_tinted_selected_unfocused() -> TestResult
{
    check_theme(
        "High Contrast Light",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Table,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_tinted_selected_unfocused_session_words()
-> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Sessions,
    )
}

#[test]
fn matched_inks_keep_glyph_contrast_in_high_contrast_light_tinted_selected_unfocused_account_words()
-> TestResult {
    check_theme(
        "High Contrast Light",
        PaneTint::Enabled,
        MatchGround::SelectedUnfocused,
        ContrastWords::Accounts,
    )
}

fn selection_distance_limit(
    selection: Color,
    plain_match: Color,
    selected_match: Color,
    tile_ground: Color,
) -> SelectionDistanceLimit {
    let bound = footer_colors::perceptual_distance(plain_match, tile_ground)
        .max(MINIMUM_VISIBLE_GROUND_DIFFERENCE);
    if footer_colors::perceptual_distance(selected_match, selection) >= bound
        && footer_colors::perceptual_distance(selected_match, plain_match) >= bound
    {
        SelectionDistanceLimit::BoundReached
    } else {
        SelectionDistanceLimit::ReadabilityCapped
    }
}

fn assert_selected_match_distinct(
    selection: Color,
    plain_match: Color,
    selected_match: Color,
    tile_ground: Color,
) {
    let selected_match_visibility_bound =
        footer_colors::perceptual_distance(plain_match, tile_ground)
            .max(MINIMUM_VISIBLE_GROUND_DIFFERENCE);
    let required_distance =
        match selection_distance_limit(selection, plain_match, selected_match, tile_ground) {
            SelectionDistanceLimit::BoundReached => selected_match_visibility_bound,
            SelectionDistanceLimit::ReadabilityCapped => MINIMUM_VISIBLE_GROUND_DIFFERENCE,
        };
    if matches!(
        selection_distance_limit(selection, plain_match, selected_match, tile_ground),
        SelectionDistanceLimit::ReadabilityCapped
    ) {
        assert_eq!(
            selection, DEFAULT_DARK_FOCUSED_SELECTION,
            "focused Default Dark is the only capped selection"
        );
        assert_eq!(
            selected_match, DEFAULT_DARK_FOCUSED_MATCH_CAP,
            "focused Default Dark uses the measured readability cap"
        );
    }
    for (kind, original) in [("selection", selection), ("plain match", plain_match)] {
        let delta_e = footer_colors::perceptual_distance(selected_match, original);
        assert!(
            delta_e >= required_distance,
            "selected-match ground must remain visibly distinct from {kind}: original={original:?}, selected_match={selected_match:?}, tile_ground={tile_ground:?}, CIEDE2000={delta_e}, required_distance={required_distance}, selected_match_visibility_bound={selected_match_visibility_bound}"
        );
    }
}

fn assert_smallest_selection_step(
    selection: Color,
    plain_match: Color,
    drawn_match: Color,
    tile_ground: Color,
) -> TestResult {
    let grounds = theme::WordMatchGrounds::new(plain_match, WordHighlightEmphasis::Subtle);
    let surface = grounds.surface(selection);
    let step = grounds.step(surface);
    let selected_match_visibility_bound =
        footer_colors::perceptual_distance(plain_match, tile_ground)
            .max(MINIMUM_VISIBLE_GROUND_DIFFERENCE);
    assert_eq!(
        grounds.bound(),
        selected_match_visibility_bound,
        "the ground search uses the bound from the drawn unselected tile"
    );
    assert_eq!(
        drawn_match,
        grounds.selection_step(surface, step),
        "the drawn selected match uses the selected lightness step"
    );
    let preceding_step = step
        .checked_sub(1)
        .ok_or("a visible selected match must move from the selection ground")?;
    let preceding = grounds.selection_step(surface, preceding_step);
    let selection_distance = footer_colors::perceptual_distance(preceding, selection);
    let match_distance = footer_colors::perceptual_distance(preceding, plain_match);
    let distance_limit = selection_distance_limit(selection, plain_match, drawn_match, tile_ground);
    let expected_limit = match distance_limit {
        SelectionDistanceLimit::BoundReached => WordMatchLimit::DistanceMet,
        SelectionDistanceLimit::ReadabilityCapped => WordMatchLimit::ReadabilityCapped,
    };
    assert_eq!(
        grounds.limit(surface),
        expected_limit,
        "the search result agrees with the measured distance limit"
    );
    if matches!(distance_limit, SelectionDistanceLimit::BoundReached) {
        assert!(
            selection_distance < selected_match_visibility_bound
                || match_distance < selected_match_visibility_bound,
            "one lightness step less must fail the bound: preceding={preceding:?}, selection_distance={selection_distance}, match_distance={match_distance}, selected_match_visibility_bound={selected_match_visibility_bound}"
        );
    }
    let original_lightness = InkHsl::from_drawn(selection)?.lightness;
    let match_lightness = InkHsl::from_drawn(drawn_match)?.lightness;
    let tile_lightness = InkHsl::from_drawn(tile_ground)?.lightness;
    if tile_lightness >= 0.5 {
        assert!(
            match_lightness < original_lightness,
            "a light theme moves the selected match toward black"
        );
    } else {
        assert!(
            match_lightness > original_lightness,
            "a dark theme moves the selected match toward white"
        );
    }
    eprintln!(
        "WORD_SELECTED_GROUND limit={distance_limit:?} surface={surface:?} tile={tile_ground:?} selection={selection:?} plain={plain_match:?} drawn={drawn_match:?} bound={selected_match_visibility_bound:.9} distance_selection={:.9} distance_plain={:.9} step={step} preceding_distance_selection={selection_distance:.9} preceding_distance_plain={match_distance:.9}",
        footer_colors::perceptual_distance(drawn_match, selection),
        footer_colors::perceptual_distance(drawn_match, plain_match)
    );
    Ok(())
}

fn check_selected_match_ground(name: &str) -> TestResult {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let builtin = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == name)
        .ok_or("built-in theme")?;
    tui_pane::set_active_theme(Arc::new(builtin.theme));
    tui_pane::set_transparent_background(false);
    tui_pane::set_focused_pane_tint(false);
    let result = measure_selected_match_grounds();
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    result
}

/// Read each strength from a painted frame on all three row grounds.
fn measure_emphasized_frames() -> TestResult {
    let mut references = Vec::new();
    for tint in [false, true] {
        tui_pane::set_focused_pane_tint(tint);
        let mut app = word_click_tests::application()?;
        let mut renderer = Renderer::new(&mut app, 80, 35)?;
        let layout = renderer.paint(&mut app)?;
        let selected = word_click_tests::locate(&layout, "first-work")?;
        let away = word_click_tests::locate(&layout, "second-work")?;
        for surface in [
            MatchGround::Base,
            MatchGround::SelectedFocused,
            MatchGround::SelectedUnfocused,
        ] {
            app.click(away);
            if !matches!(surface, MatchGround::Base) {
                app.click(selected);
            }
            if matches!(surface, MatchGround::SelectedUnfocused) {
                app.click(away);
            }
            app.word_highlight.clear();
            let baseline = renderer.paint(&mut app)?;
            let words = PaintedWords::capture(&baseline, app.tiles.pointer_regions());
            references.push((baseline, words, selected, surface, tint));
        }
    }
    let WordHit::Word(word) = HighlightWord::from_token("first-work") else {
        return Err("the fixture word is empty".into());
    };
    let highlight = WordHighlight::Word(word);
    let mut cache = crate::word_highlight::MatchInkCache::default();
    let background = word_click_tests::match_background()?;
    let mut previous = Vec::new();
    for emphasis in [
        WordHighlightEmphasis::Subtle,
        WordHighlightEmphasis::Clear,
        WordHighlightEmphasis::Strong,
    ] {
        let plain =
            theme::WordMatchGrounds::new(background, emphasis).background(WordMatchSurface::Plain);
        for (index, (baseline, words, point, surface, tint)) in references.iter().enumerate() {
            let original = &baseline[*point];
            let mut frame = baseline.clone();
            words.apply_with_inks(&highlight, &mut frame, background, emphasis, &mut cache);
            let matched = &frame[*point];
            let distance = footer_colors::perceptual_distance(matched.bg, original.bg);
            assert!(
                distance >= MINIMUM_VISIBLE_GROUND_DIFFERENCE,
                "{emphasis:?} {surface:?} tint={tint}: distance={distance}"
            );
            assert_matched_ink(original, matched, "first-work", *point)?;
            let bound = MINIMUM_CONTRAST.min(glyph_contrast(original.fg, original.bg)?);
            assert!(
                glyph_contrast(matched.fg, matched.bg)? >= bound,
                "{emphasis:?} {surface:?}"
            );
            let expected = if emphasis == WordHighlightEmphasis::Strong {
                original.modifier | ratatui::style::Modifier::BOLD
            } else {
                original.modifier
            };
            assert_eq!(matched.modifier, expected);
            if !matches!(surface, MatchGround::Base) {
                assert!(
                    footer_colors::perceptual_distance(matched.bg, plain)
                        >= MINIMUM_VISIBLE_GROUND_DIFFERENCE
                );
            }
            match emphasis {
                WordHighlightEmphasis::Subtle => previous.push(matched.bg),
                WordHighlightEmphasis::Clear => {
                    if matches!(surface, MatchGround::Base) {
                        let subtle =
                            footer_colors::perceptual_distance(previous[index], original.bg);
                        assert!(
                            distance > subtle,
                            "clear={distance}, subtle={subtle}, tint={tint}"
                        );
                    }
                    previous[index] = matched.bg;
                },
                WordHighlightEmphasis::Strong => assert_eq!(matched.bg, previous[index]),
            }
        }
    }
    Ok(())
}

fn measure_selected_match_grounds() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut app = palette_application(&directory.path().join("statusline.conf"))?;
    let mut renderer = word_click_tests::Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let layout = renderer.paint(&mut app)?;
    let word = "palette-review-0";
    let selected = word_click_tests::locate(&layout, word)?;
    let away = word_click_tests::locate(&layout, "palette-work-5")?;
    let plain_match = word_click_tests::match_background()?;
    for focused in [true, false] {
        app.click(selected);
        renderer.paint(&mut app)?;
        if !focused {
            app.click(away);
        }
        app.word_highlight.clear();
        let before = renderer.paint(&mut app)?;
        let tile_point = word_click_tests::locate(&before, "palette-work-0")?;
        let tile_ground = before[(tile_point.x, tile_point.y)].bg;
        let selection = if focused {
            tui_pane::active_focus_color()
        } else {
            tui_pane::remembered_focus_color()
        };
        assert_eq!(before[(selected.x, selected.y)].bg, selection);
        let WordHit::Word(token) = HighlightWord::from_token(word) else {
            return Err("the selected name must contain a word".into());
        };
        app.word_highlight = WordHighlight::Word(token);
        let after = renderer.paint(&mut app)?;
        assert_selected_match_distinct(
            selection,
            plain_match,
            after[(selected.x, selected.y)].bg,
            tile_ground,
        );
        assert_smallest_selection_step(
            selection,
            plain_match,
            after[(selected.x, selected.y)].bg,
            tile_ground,
        )?;
        let drawn_match = after[(selected.x, selected.y)].bg;
        let limit = selection_distance_limit(selection, plain_match, drawn_match, tile_ground);
        let grounds = theme::WordMatchGrounds::new(plain_match, WordHighlightEmphasis::Subtle);
        let surface = grounds.surface(selection);
        let next_step = grounds
            .step(surface)
            .checked_add(1)
            .ok_or("the selected ground must have a next lightness step")?;
        let next_ground = grounds.selection_step(surface, next_step);
        let mut next_step_failures = 0;
        let mut count = 0;
        for x in selected.x..selected.x + u16::try_from(word.len())? {
            let point = Position::new(x, selected.y);
            let old = &before[(x, selected.y)];
            let new = &after[(x, selected.y)];
            assert_matched_ink(old, new, word, point)?;
            let match_readability_bound = MINIMUM_CONTRAST.min(glyph_contrast(old.fg, old.bg)?);
            let ratio_on_match = glyph_contrast(new.fg, new.bg)?;
            assert!(
                ratio_on_match >= match_readability_bound,
                "the match never makes text less readable than today: focused={focused}, ink={:?}, ground={:?}, ratio_on_match={ratio_on_match}, match_readability_bound={match_readability_bound}",
                new.fg,
                new.bg
            );
            if matches!(limit, SelectionDistanceLimit::ReadabilityCapped) {
                let next_ink =
                    crate::word_highlight::MatchInk::resolve(old.fg, old.bg, next_ground)
                        .foreground();
                let next_ratio = glyph_contrast(next_ink, next_ground)?;
                let next_capacity = glyph_contrast(Color::White, next_ground)?
                    .max(glyph_contrast(Color::Black, next_ground)?);
                if next_ratio < match_readability_bound && next_capacity < match_readability_bound {
                    next_step_failures += 1;
                }
                eprintln!(
                    "WORD_CAP_NEXT ink={:?} ground={next_ground:?} lifted={next_ink:?} bound={match_readability_bound:.9} ratio={next_ratio:.9} capacity={next_capacity:.9}",
                    old.fg
                );
            }
            count += 1;
        }
        assert_eq!(
            count,
            word.len(),
            "each selected name cell receives the match ground"
        );
        if matches!(limit, SelectionDistanceLimit::ReadabilityCapped) {
            assert!(
                next_step_failures > 0,
                "one lightness step beyond the cap must fail an ink bound on the selected name"
            );
        }
    }
    Ok(())
}

#[test]
fn selected_matches_remain_visible_in_default_dark() -> TestResult {
    check_selected_match_ground("Default Dark")
}

#[test]
fn selected_matches_remain_visible_in_default_light() -> TestResult {
    check_selected_match_ground("Default Light")
}

#[test]
fn selected_matches_remain_visible_in_high_contrast_dark() -> TestResult {
    check_selected_match_ground("High Contrast Dark")
}

#[test]
fn selected_matches_remain_visible_in_high_contrast_light() -> TestResult {
    check_selected_match_ground("High Contrast Light")
}

#[derive(Clone, Copy)]
enum CustomMatchTheme {
    LowCapacitySelection,
    LightWithDarkFocusGround,
    DarkWithLightFocusGround,
}

impl CustomMatchTheme {
    const fn builtin(self) -> &'static str {
        match self {
            Self::LowCapacitySelection | Self::DarkWithLightFocusGround => "Default Dark",
            Self::LightWithDarkFocusGround => "Default Light",
        }
    }
}

struct DrawnSelectionMatch {
    selection:      Color,
    selected_match: Color,
    plain_match:    Color,
}

fn custom_selection_frames(case: CustomMatchTheme) -> TestResult<Vec<DrawnSelectionMatch>> {
    let previous = tui_pane::theme();
    let previous_registry = tui_pane::registry();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let mut custom = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == case.builtin())
        .ok_or("custom theme base")?;
    match case {
        CustomMatchTheme::LowCapacitySelection => {
            custom.theme.focus.active.color = LOW_CAPACITY_FOCUSED_SELECTION;
            custom.theme.focus.remembered.color = LOW_CAPACITY_UNFOCUSED_SELECTION;
        },
        CustomMatchTheme::LightWithDarkFocusGround => {
            custom.theme.text.bg_focus.color = DARK_CUSTOM_FOCUS_GROUND;
        },
        CustomMatchTheme::DarkWithLightFocusGround => {
            custom.theme.text.bg_focus.color = LIGHT_CUSTOM_FOCUS_GROUND;
        },
    }
    let mut registry = (*previous_registry).clone();
    registry.register(custom.clone());
    tui_pane::replace_registry(registry);
    tui_pane::set_active_theme(Arc::new(custom.theme));
    tui_pane::set_transparent_background(false);
    tui_pane::set_focused_pane_tint(false);
    let result = (|| {
        let mut app = word_click_tests::application()?;
        let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
        let layout = renderer.paint(&mut app)?;
        let word = "first-work";
        let selected = word_click_tests::locate(&layout, word)?;
        let away = word_click_tests::locate(&layout, "second-work")?;
        let mut frames = Vec::new();
        for ground in [MatchGround::SelectedFocused, MatchGround::SelectedUnfocused] {
            app.click(selected);
            renderer.paint(&mut app)?;
            if matches!(ground, MatchGround::SelectedUnfocused) {
                app.click(away);
            }
            app.word_highlight.clear();
            let before = renderer.paint(&mut app)?;
            let WordHit::Word(token) = HighlightWord::from_token(word) else {
                return Err("the selected name must contain a word".into());
            };
            app.word_highlight = WordHighlight::Word(token);
            let after = renderer.paint(&mut app)?;
            frames.push(DrawnSelectionMatch {
                selection:      before[(selected.x, selected.y)].bg,
                selected_match: after[(selected.x, selected.y)].bg,
                plain_match:    word_click_tests::match_background()?,
            });
        }
        Ok(frames)
    })();
    tui_pane::replace_registry((*previous_registry).clone());
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    result
}

#[test]
fn low_capacity_selection_grounds_keep_the_match_distance_floor() -> TestResult {
    for DrawnSelectionMatch {
        selection,
        selected_match,
        plain_match,
    } in custom_selection_frames(CustomMatchTheme::LowCapacitySelection)?
    {
        let capacity =
            glyph_contrast(Color::White, selection)?.max(glyph_contrast(Color::Black, selection)?);
        assert!(
            capacity < MINIMUM_CONTRAST,
            "the custom selection lacks ink capacity"
        );
        for original in [selection, plain_match] {
            let distance = footer_colors::perceptual_distance(selected_match, original);
            assert!(
                distance >= MINIMUM_VISIBLE_GROUND_DIFFERENCE,
                "the selected match keeps the distance floor: selection={selection:?}, match={selected_match:?}, original={original:?}, distance={distance}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_light_appearance_darkens_matches_despite_a_dark_focus_ground() -> TestResult {
    for DrawnSelectionMatch {
        selection,
        selected_match,
        ..
    } in custom_selection_frames(CustomMatchTheme::LightWithDarkFocusGround)?
    {
        assert!(
            InkHsl::from_drawn(selected_match)?.lightness
                < InkHsl::from_drawn(selection)?.lightness,
            "the Light appearance darkens selected matches regardless of the focus ground"
        );
    }
    Ok(())
}

#[test]
fn a_dark_appearance_lightens_matches_despite_a_light_focus_ground() -> TestResult {
    for DrawnSelectionMatch {
        selection,
        selected_match,
        ..
    } in custom_selection_frames(CustomMatchTheme::DarkWithLightFocusGround)?
    {
        assert!(
            InkHsl::from_drawn(selected_match)?.lightness
                > InkHsl::from_drawn(selection)?.lightness,
            "the Dark appearance lightens selected matches regardless of the focus ground"
        );
    }
    Ok(())
}

fn reset_step_application() -> TestResult<App> {
    use crate::census::accounts::AccountsReading;
    use crate::census::accounts::AccountsTable;

    let mut app = App::new_for_test()?;
    app.loaded_config.config.tiles.view = TileView::Summary;
    let now = chrono::Utc::now();
    let accounts: Vec<_> = (0..6)
        .map(|rank| {
            serde_json::json!({
                "label":format!("reset-step-{rank}"),
                "roles":[], "use_credits":false,
                "weekly_left_percent":50,
                "resets_at":(now + chrono::Duration::minutes(10 + rank * 10)).to_rfc3339(),
                "read_ok":true,
                "read_at":now.to_rfc3339()
            })
        })
        .collect();
    app.accounts.table = AccountsTable::Ready(
        AccountsReading::parse(&serde_json::to_vec(
            &serde_json::json!({"accounts":accounts,"problems":[]}),
        )?)
        .map_err(|error| error.to_string())?,
    );
    Ok(app)
}

fn reset_step_cell(buffer: &Buffer, rank: usize) -> TestResult<Position> {
    let heading = word_click_tests::locate(buffer, "resets")?;
    let account = word_click_tests::locate(buffer, &format!("reset-step-{rank}"))?;
    let point = Position::new(heading.x + 5, account.y);
    assert!(
        buffer[(point.x, point.y)]
            .symbol()
            .chars()
            .any(|glyph| glyph.is_ascii_digit())
    );
    Ok(point)
}

fn assert_reset_ink_separation(ink: Color, description: &str) -> TestResult {
    assert_ne!(
        ink,
        tui_pane::error_color(),
        "{description} cannot use error red"
    );
    for hue in SessionHue::ALL {
        assert_ne!(
            ink,
            Role::Session(hue).style().fg.ok_or("session ink")?,
            "{description} differs from every session ink"
        );
    }
    Ok(())
}

fn check_reset_step_match(
    app: &mut App,
    renderer: &mut Renderer,
    baseline: &Buffer,
    point: Position,
) -> TestResult {
    let WordHit::Word(word) =
        crate::word_highlight::PaintedWords::word_at(baseline, app.tiles.pointer_regions(), point)
    else {
        return Err("the drawn reset must provide a word".into());
    };
    let before = &baseline[(point.x, point.y)];
    assert!(
        glyph_contrast(before.fg, before.bg)? >= MINIMUM_CONTRAST,
        "the drawn reset step meets the text bound: {before:?}"
    );
    assert_reset_ink_separation(before.fg, "a reset step")?;
    let text = word.text().to_owned();
    app.word_highlight = WordHighlight::Word(word);
    let matched = renderer.paint(app)?;
    let after = &matched[(point.x, point.y)];
    assert_ne!(
        after.bg, before.bg,
        "the reset word receives a match ground"
    );
    assert_reset_ink_separation(after.fg, "a matched reset")?;
    assert_matched_ink(before, after, &text, point)?;
    let bound = MINIMUM_CONTRAST.min(glyph_contrast(before.fg, before.bg)?);
    assert!(
        glyph_contrast(after.fg, after.bg)? >= bound,
        "the match never makes reset text less readable: {before:?} -> {after:?}, bound={bound}"
    );
    app.word_highlight.clear();
    Ok(())
}

fn measure_reset_steps() -> TestResult {
    for tint in [PaneTint::Disabled, PaneTint::Enabled] {
        tui_pane::set_focused_pane_tint(tint.enabled());
        let mut app = reset_step_application()?;
        let mut renderer = Renderer::new(&mut app, 100, 22)?;
        let initial = renderer.paint(&mut app)?;
        app.click(word_click_tests::locate(&initial, "reset-step-0")?);
        app.word_highlight.clear();
        let colors: Vec<_> = (0..6)
            .map(|rank| {
                let point = reset_step_cell(&initial, rank)?;
                Ok(initial[(point.x, point.y)].fg)
            })
            .collect::<TestResult<_>>()?;
        let mut steps = colors.clone();
        steps.dedup();
        assert!(
            (3..=4).contains(&steps.len()),
            "the drawn reset ramp has three or four steps: {colors:?}"
        );
        for rank in [0, 2, 5] {
            let point = reset_step_cell(&initial, rank)?;
            app.click(word_click_tests::locate(&initial, "reset-step-1")?);
            app.word_highlight.clear();
            let baseline = renderer.paint(&mut app)?;
            assert_eq!(
                baseline[(point.x, point.y)].bg,
                tui_pane::pane_background(true),
                "the plain reset measurement uses the plain focused pane ground"
            );
            check_reset_step_match(&mut app, &mut renderer, &baseline, point)?;
            app.click(word_click_tests::locate(
                &initial,
                &format!("reset-step-{rank}"),
            )?);
            app.word_highlight.clear();
            let selected = renderer.paint(&mut app)?;
            assert_eq!(
                selected[(point.x, point.y)].bg,
                tui_pane::active_focus_color()
            );
            check_reset_step_match(&mut app, &mut renderer, &selected, point)?;
        }
    }
    Ok(())
}

fn check_reset_steps(name: &str) -> TestResult {
    let variant = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == name)
        .ok_or("reset theme")?;
    let fixture = ResetThemeFixture::with_theme(variant.theme);
    tui_pane::set_transparent_background(false);
    let result = measure_reset_steps();
    drop(fixture);
    result
}

#[test]
fn reset_steps_keep_readable_hues_on_plain_and_selected_matches_in_default_dark() -> TestResult {
    check_reset_steps("Default Dark")
}

#[test]
fn reset_steps_keep_readable_hues_on_plain_and_selected_matches_in_default_light() -> TestResult {
    check_reset_steps("Default Light")
}

#[test]
fn reset_steps_keep_readable_hues_on_plain_and_selected_matches_in_high_contrast_dark() -> TestResult
{
    check_reset_steps("High Contrast Dark")
}

#[test]
fn reset_steps_keep_readable_hues_on_plain_and_selected_matches_in_high_contrast_light()
-> TestResult {
    check_reset_steps("High Contrast Light")
}

#[derive(Clone, Copy)]
enum AstraWordRole {
    Designer,
    Reviewer,
}

impl AstraWordRole {
    const fn words(self) -> (&'static str, &'static str) {
        match self {
            Self::Designer => ("designer-session", "designer"),
            Self::Reviewer => ("reviewer-session", "reviewer"),
        }
    }
}

fn check_astra_word_matches(name: &str, role: AstraWordRole) -> TestResult {
    let builtin = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == name)
        .ok_or("theme")?;
    let theme_reset = ResetThemeFixture::with_theme(builtin.theme);
    tui_pane::set_transparent_background(false);
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("footer.conf");
    std::fs::write(
        &path,
        include_str!("../footer_colors/fixtures/statusline_astra.conf"),
    )?;
    let mut failures = Vec::new();
    for tint in [false, true] {
        tui_pane::set_focused_pane_tint(tint);
        let mut app = selection_tests::astra_application();
        app.loaded_config.config.word_highlight.emphasis = WordHighlightEmphasis::Subtle;
        app.footer_colors = crate::footer_colors::FooterColorFile::fixture(path.clone());
        app.footer_colors.refresh();
        let mut renderer = Renderer::new(&mut app, 180, 60)?;
        let initial = renderer.paint(&mut app)?;
        {
            let (label, word) = role.words();
            let at = word_click_tests::locate(&initial, label)?;
            let role_word = word_click_tests::locate(&initial, &format!("astra {word}"))?;
            for ground in [
                MatchGround::Base,
                MatchGround::SelectedFocused,
                MatchGround::SelectedUnfocused,
            ] {
                app.word_highlight.clear();
                app.click(word_click_tests::locate(&initial, "astra-focus-away")?);
                if !matches!(ground, MatchGround::Base) {
                    app.click(at);
                }
                if matches!(ground, MatchGround::SelectedUnfocused) {
                    app.click(word_click_tests::locate(&initial, "build-for-50")?);
                }
                let before = renderer.paint(&mut app)?;
                let x = role_word.x + 6;
                let selection = before[(x, role_word.y)].bg;
                match ground {
                    MatchGround::Base => {
                        assert_ne!(selection, tui_pane::active_focus_color());
                        assert_ne!(selection, tui_pane::remembered_focus_color());
                    },
                    MatchGround::SelectedFocused => {
                        assert_eq!(selection, tui_pane::active_focus_color());
                    },
                    MatchGround::SelectedUnfocused => {
                        assert_eq!(selection, tui_pane::remembered_focus_color());
                    },
                }
                let WordHit::Word(token) = HighlightWord::from_token(word) else {
                    return Err("Astra role token".into());
                };
                app.word_highlight = WordHighlight::Word(token);
                let buffer = renderer.paint(&mut app)?;
                let cell = &buffer[(x, role_word.y)];
                let plain_match = word_click_tests::match_background()?;
                let grounds =
                    theme::WordMatchGrounds::new(plain_match, WordHighlightEmphasis::Subtle);
                let expected = match ground {
                    MatchGround::Base => plain_match,
                    _ => grounds.background(grounds.surface(selection)),
                };
                assert_eq!(
                    cell.bg, expected,
                    "{name} {word} {ground:?}: the match must paint its ground"
                );
                assert_ne!(
                    cell.bg, selection,
                    "the match must change the previous ground"
                );
                let ratio = glyph_contrast(cell.fg, cell.bg)?;
                if ratio < 4.5 {
                    failures.push(format!(
                        "{name} {word} tint={tint} {ground:?} ratio={ratio:.9}"
                    ));
                }
                assert_ne!(cell.symbol(), " ", "matched role glyph");
            }
        }
    }
    drop(theme_reset);
    assert!(failures.is_empty(), "Astra match contrast: {failures:?}");
    Ok(())
}

#[test]
fn astra_word_matches_keep_coverage_in_default_dark() -> TestResult {
    check_astra_word_matches("Default Dark", AstraWordRole::Designer)
}
#[test]
fn astra_word_matches_keep_coverage_in_default_light() -> TestResult {
    check_astra_word_matches("Default Light", AstraWordRole::Designer)
}
#[test]
fn astra_word_matches_keep_coverage_in_high_contrast_dark() -> TestResult {
    check_astra_word_matches("High Contrast Dark", AstraWordRole::Designer)
}
#[test]
fn astra_word_matches_keep_coverage_in_high_contrast_light() -> TestResult {
    check_astra_word_matches("High Contrast Light", AstraWordRole::Designer)
}

#[test]
fn astra_reviewer_word_matches_keep_coverage_in_default_dark() -> TestResult {
    check_astra_word_matches("Default Dark", AstraWordRole::Reviewer)
}

#[test]
fn astra_reviewer_word_matches_keep_coverage_in_default_light() -> TestResult {
    check_astra_word_matches("Default Light", AstraWordRole::Reviewer)
}

#[test]
fn astra_reviewer_word_matches_keep_coverage_in_high_contrast_dark() -> TestResult {
    check_astra_word_matches("High Contrast Dark", AstraWordRole::Reviewer)
}

#[test]
fn astra_reviewer_word_matches_keep_coverage_in_high_contrast_light() -> TestResult {
    check_astra_word_matches("High Contrast Light", AstraWordRole::Reviewer)
}

fn check_emphasized_theme(name: &str) -> TestResult {
    let variant = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == name)
        .ok_or("the emphasis theme")?;
    let reset = ResetThemeFixture::with_theme(variant.theme);
    tui_pane::set_transparent_background(false);
    let result = measure_emphasized_frames();
    drop(reset);
    result
}

#[test]
fn emphasis_grounds_and_modifiers_in_default_dark() -> TestResult {
    check_emphasized_theme("Default Dark")
}

#[test]
fn emphasis_grounds_and_modifiers_in_default_light() -> TestResult {
    check_emphasized_theme("Default Light")
}

#[test]
fn emphasis_grounds_and_modifiers_in_high_contrast_dark() -> TestResult {
    check_emphasized_theme("High Contrast Dark")
}

#[test]
fn emphasis_grounds_and_modifiers_in_high_contrast_light() -> TestResult {
    check_emphasized_theme("High Contrast Light")
}

fn waiting_activity_application() -> TestResult<App> {
    use crate::census::accounts::AccountsReading;
    use crate::census::accounts::AccountsTable;

    let mut app = word_click_tests::application()?;
    let mut owner = word_click_tests::session(10, "waiting-owner", "")?;
    owner.children = vec![serde_json::from_value(serde_json::json!({
        "kind":"thread", "depth":0, "name":"waiting-worker", "started":101,
        "thread_id":"waiting-thread", "activity":{"active":"waiting"}
    }))?];
    word_click_tests::apply_rows(
        &mut app,
        vec![
            owner,
            word_click_tests::session(20, "busy-control", "")?,
            word_click_tests::session(30, "idle-control", "")?,
        ],
    );
    app.accounts.table = AccountsTable::Ready(
        AccountsReading::parse(
            br#"{
        "accounts":[{"label":"claude1","roles":[],"use_credits":false,"read_ok":true,"sessions":[
            {"session":"word-session-10","state":"busy"},
            {"session":"waiting-thread","state":"busy"},
            {"session":"word-session-20","state":"busy"},
            {"session":"word-session-30","state":"idle"}
        ]}],"problems":[]
    }"#,
        )
        .map_err(|error| error.to_string())?,
    );
    expansion::dispatch(
        &mut app,
        HandlerExpansionAction::Shared(ExpansionAction::ExpandAll),
    );
    Ok(app)
}

fn assert_waiting_word_ink(cell: &Cell, other_inks: &[Color]) -> TestResult {
    let ratio = glyph_contrast(cell.fg, cell.bg)?;
    assert!(
        ratio >= MINIMUM_CONTRAST,
        "waiting: {cell:?}, covered contrast={ratio}"
    );
    for ink in other_inks {
        assert_ne!(cell.fg, *ink, "waiting keeps a distinct ink: {cell:?}");
    }
    Ok(())
}

fn fixture_word(buffer: &Buffer, label: &str, word: &str) -> TestResult<Position> {
    let row = word_click_tests::locate(buffer, label)?;
    word_click_tests::positions(buffer, word)
        .into_iter()
        .find(|point| point.y == row.y && point.x > row.x)
        .ok_or_else(|| format!("missing {word} for {label}").into())
}

#[test]
fn summary_status_of_a_waiting_session_draws_busy_in_the_busy_ink() -> TestResult {
    let reset = ResetThemeFixture::install();
    tui_pane::set_transparent_background(false);
    let mut app = waiting_activity_application()?;
    app.loaded_config.config.tiles.view = TileView::Summary;
    let mut renderer = Renderer::new(&mut app, 100, 20)?;
    let buffer = renderer.paint(&mut app)?;
    let point = fixture_word(&buffer, "waiting-owner", "busy")?;
    let expected = Role::Busy.style().fg.ok_or("busy ink")?;
    for x in point.x..point.x + 4 {
        assert_eq!(buffer[(x, point.y)].fg, expected);
    }
    let label = word_click_tests::locate(&buffer, "waiting-owner")?;
    assert!(
        word_click_tests::positions(&buffer, "waiting")
            .iter()
            .all(|waiting| waiting.y != point.y || waiting.x == label.x),
        "only the session name holds the word waiting"
    );
    drop(reset);
    Ok(())
}

fn check_waiting_activity_matches(name: &str) -> TestResult {
    let builtin = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == name)
        .ok_or("theme")?;
    let reset = ResetThemeFixture::with_theme(builtin.theme);
    tui_pane::set_transparent_background(false);
    for tint in [false, true] {
        tui_pane::set_focused_pane_tint(tint);
        let mut app = waiting_activity_application()?;
        let mut renderer = Renderer::new(&mut app, 100, 35)?;
        let initial = renderer.paint(&mut app)?;
        let worker = word_click_tests::locate(&initial, "waiting-worker")?;
        let away = word_click_tests::locate(&initial, "busy-control")?;
        let waiting = fixture_word(&initial, "waiting-worker", "waiting")?;
        for ground in [
            MatchGround::Base,
            MatchGround::SelectedFocused,
            MatchGround::SelectedUnfocused,
        ] {
            app.click(away);
            if !matches!(ground, MatchGround::Base) {
                app.click(worker);
            }
            if matches!(ground, MatchGround::SelectedUnfocused) {
                app.click(away);
            }
            app.word_highlight.clear();
            let baseline = renderer.paint(&mut app)?;
            let before = &baseline[waiting];
            match ground {
                MatchGround::Base => {
                    assert_ne!(before.bg, tui_pane::active_focus_color());
                    assert_ne!(before.bg, tui_pane::remembered_focus_color());
                },
                MatchGround::SelectedFocused => {
                    assert_eq!(before.bg, tui_pane::active_focus_color());
                },
                MatchGround::SelectedUnfocused => {
                    assert_eq!(before.bg, tui_pane::remembered_focus_color());
                },
            }
            let other_inks = [
                baseline[fixture_word(&baseline, "busy-control", "busy")?].fg,
                baseline[fixture_word(&baseline, "idle-control", "idle")?].fg,
                Role::Unreachable.style().fg.ok_or("unreachable ink")?,
            ];
            assert_waiting_word_ink(before, &other_inks)?;
            let [red, green, blue] = rgb(before.fg)?;
            assert!(
                red > blue
                    && green > blue
                    && (red - green).abs() <= red.max(green) * WAITING_HUE_TOLERANCE,
                "unmatched waiting retains its yellow hue: {before:?}"
            );
            let painted = PaintedWords::capture(&baseline, app.tiles.pointer_regions());
            let WordHit::Word(token) = HighlightWord::from_token("waiting") else {
                return Err("waiting token".into());
            };
            for emphasis in [
                WordHighlightEmphasis::Subtle,
                WordHighlightEmphasis::Clear,
                WordHighlightEmphasis::Strong,
            ] {
                let mut matched = baseline.clone();
                painted.apply_with_inks(
                    &WordHighlight::Word(token.clone()),
                    &mut matched,
                    word_click_tests::match_background()?,
                    emphasis,
                    &mut app.match_inks,
                );
                for x in waiting.x..waiting.x + 7 {
                    let after = &matched[(x, waiting.y)];
                    assert_ne!(after.bg, before.bg, "{name} {ground:?} {emphasis:?}");
                    assert_waiting_word_ink(after, &other_inks)?;
                    assert_matched_ink(before, after, "waiting", Position::new(x, waiting.y))?;
                }
            }
        }
    }
    drop(reset);
    Ok(())
}

#[test]
fn waiting_activity_matches_keep_yellow_contrast_in_default_dark() -> TestResult {
    check_waiting_activity_matches("Default Dark")
}

#[test]
fn waiting_activity_matches_keep_yellow_contrast_in_default_light() -> TestResult {
    check_waiting_activity_matches("Default Light")
}

#[test]
fn waiting_activity_matches_keep_yellow_contrast_in_high_contrast_dark() -> TestResult {
    check_waiting_activity_matches("High Contrast Dark")
}

#[test]
fn waiting_activity_matches_keep_yellow_contrast_in_high_contrast_light() -> TestResult {
    check_waiting_activity_matches("High Contrast Light")
}
