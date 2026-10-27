//! Account seed corrections keep teal distinct after drawing on each pane ground.

use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use super::tests;
use super::*;
use crate::theme;
use crate::theme::Role;
use crate::theme::SessionHue;

/// Read the ink produced by the same style path used for account and role words.
fn drawn_ink(style: Style, ground: Color) -> Color {
    let area = Rect::new(0, 0, 1, 1);
    let mut buffer = Buffer::empty(area);
    buffer.set_style(area, Style::default().bg(ground));
    Paragraph::new("x").style(style).render(area, &mut buffer);
    buffer[(0, 0)].fg
}

#[test]
fn shell_timer_ink_stays_distinct_from_every_account_after_coverage_solving() {
    let colors = FooterColors::parse(include_str!("fixtures/statusline.conf"));
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let mut measurements = Vec::new();
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins() {
        let name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            for focused in [false, true] {
                let ground = tui_pane::pane_background(focused);
                let shell = drawn_ink(Role::Shell.style(), ground);
                for account in [
                    "claude1", "claude2", "claude3", "claude4", "codex1", "codex2",
                ] {
                    let ink = drawn_ink(colors.account(account).style(Style::default()), ground);
                    measurements.push((
                        name.clone(),
                        tint,
                        focused,
                        account,
                        shell,
                        ink,
                        tests::covered_role_contrast(shell, ground),
                        tests::covered_role_contrast(ink, ground),
                        tests::perceptual_distance(shell, ink),
                    ));
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    for (name, tint, focused, account, shell, ink, shell_ratio, account_ratio, distance) in
        measurements
    {
        println!(
            "{name} tint={tint} focused={focused} shell={shell:?} {account}={ink:?} contrast={shell_ratio:.6}/{account_ratio:.6} dE={distance:.6}"
        );
        assert!(
            shell_ratio >= MINIMUM_CONTRAST,
            "shell covered contrast: {shell_ratio}"
        );
        assert!(
            account_ratio >= MINIMUM_CONTRAST,
            "{account} covered contrast: {account_ratio}"
        );
        assert!(
            distance >= 12.0,
            "{name} tint={tint} focused={focused} shell/{account}: {distance}"
        );
    }
}

#[test]
fn teal_account_ink_stays_distinct_from_other_accounts_roles_and_azure() {
    let colors = FooterColors::parse(include_str!("fixtures/statusline.conf"));
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let mut distances = Vec::new();
    let mut contrasts = Vec::new();
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins() {
        let name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            for focused in [false, true] {
                let ground = tui_pane::pane_background(focused);
                let ink = |tint: FooterTint| drawn_ink(tint.style(Style::default()), ground);
                let teal = ink(colors.account("codex2"));
                contrasts.push((
                    name.clone(),
                    tint,
                    focused,
                    tests::covered_role_contrast(teal, ground),
                ));
                let peers = [
                    ("claude1", ink(colors.account("claude1"))),
                    ("claude2", ink(colors.account("claude2"))),
                    ("claude3", ink(colors.account("claude3"))),
                    ("claude4", ink(colors.account("claude4"))),
                    ("codex1", ink(colors.account("codex1"))),
                    ("showrunner", ink(colors.role(FooterRole::Showrunner))),
                    ("unit-director", ink(colors.role(FooterRole::UnitDirector))),
                    ("astra-designer", ink(FooterTint::Footer(FooterColor::Pink))),
                    (
                        "astra-reviewer",
                        ink(FooterTint::Footer(FooterColor::BrightGreen)),
                    ),
                    (
                        "azure",
                        drawn_ink(Role::Session(SessionHue::Fifth).style(), ground),
                    ),
                ];
                for (peer, color) in peers {
                    distances.push((
                        name.clone(),
                        tint,
                        focused,
                        peer,
                        tests::perceptual_distance(teal, color),
                    ));
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    for (name, tint, focused, ratio) in contrasts {
        println!("{name} tint={tint} focused={focused} teal covered contrast={ratio:.6}");
        assert!(ratio >= MINIMUM_CONTRAST, "teal covered contrast: {ratio}");
    }
    for (name, tint, focused, peer, distance) in distances {
        println!("{name} tint={tint} focused={focused} teal/{peer} dE={distance:.6}");
        assert!(
            distance >= 12.0,
            "{name} tint={tint} focused={focused} teal/{peer}: {distance}"
        );
    }
}
