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
use tui_pane::BUILTIN_DARK_NAME;
use tui_pane::ThemeFamily;
use tui_pane::ThemeId;
use tui_pane::ThemeRegistry;
use tui_pane::ThemeVariant;
use tui_pane::default_dark;
use tui_pane::default_light;
use tui_pane::high_contrast_dark;
use tui_pane::high_contrast_light;

const DARK_TEMPLATE: &str = include_str!("../themes/default_dark.toml");
const LIGHT_TEMPLATE: &str = include_str!("../themes/default_light.toml");
const HC_TEMPLATE: &str = include_str!("../themes/high_contrast.toml");

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

#[test]
fn hc_template_matches_constructors() {
    let family: ThemeFamily = toml::from_str(HC_TEMPLATE).expect("high_contrast.toml should parse");
    assert_eq!(family.schema, 1);
    assert_eq!(family.variants.len(), 2);
    let mut iter = family.variants.into_iter();
    let dark = iter.next().expect("first variant");
    let light = iter.next().expect("second variant");
    assert_eq!(dark.name, "High Contrast Dark");
    assert_eq!(dark.appearance, Appearance::Dark);
    assert_eq!(dark.into_theme(), high_contrast_dark());
    assert_eq!(light.name, "High Contrast Light");
    assert_eq!(light.appearance, Appearance::Light);
    assert_eq!(light.into_theme(), high_contrast_light());
}

#[test]
fn registry_parses_template_variants_via_into_theme() {
    let family: ThemeFamily =
        toml::from_str(DARK_TEMPLATE).expect("default_dark.toml should parse");
    let mut registry = ThemeRegistry::empty();
    for variant_file in family.variants {
        let id = ThemeId::new(variant_file.name.clone());
        let appearance = variant_file.appearance;
        let theme = variant_file.into_theme();
        registry.register(ThemeVariant {
            id,
            appearance,
            theme,
        });
    }
    assert_eq!(registry.len(), 1);
    assert_eq!(
        registry
            .find(&ThemeId::new(BUILTIN_DARK_NAME))
            .expect("registered")
            .theme,
        default_dark()
    );
}
