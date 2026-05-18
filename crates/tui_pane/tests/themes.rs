//! Roundtrip tests for the starter theme templates.
//!
//! Each `tui_pane/themes/*.toml` template is documentation: a copy a
//! user can drop into `~/.config/cargo-port/themes/` as a starting
//! point. These tests parse the template and assert it produces the
//! exact same [`Theme`] as the corresponding Rust constructor, so the
//! docs can't silently drift from the code.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]

use tui_pane::Appearance;
use tui_pane::ThemeFamily;
use tui_pane::default_dark;
use tui_pane::default_light;

const DARK_TEMPLATE: &str = include_str!("../themes/default_dark.toml");
const LIGHT_TEMPLATE: &str = include_str!("../themes/default_light.toml");

#[test]
fn dark_template_matches_constructor() {
    let family: ThemeFamily =
        toml::from_str(DARK_TEMPLATE).expect("default_dark.toml should parse");
    assert_eq!(family.schema, 1);
    assert_eq!(family.variants.len(), 1);
    let variant = family
        .variants
        .into_iter()
        .next()
        .expect("default_dark.toml should have one variant");
    assert_eq!(variant.name, "Default Dark");
    assert_eq!(variant.appearance, Appearance::Dark);
    assert_eq!(variant.into_theme(), default_dark());
}

#[test]
fn light_template_matches_constructor() {
    let family: ThemeFamily =
        toml::from_str(LIGHT_TEMPLATE).expect("default_light.toml should parse");
    assert_eq!(family.schema, 1);
    assert_eq!(family.variants.len(), 1);
    let variant = family
        .variants
        .into_iter()
        .next()
        .expect("default_light.toml should have one variant");
    assert_eq!(variant.name, "Default Light");
    assert_eq!(variant.appearance, Appearance::Light);
    assert_eq!(variant.into_theme(), default_light());
}
