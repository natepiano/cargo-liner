//! The handler rejects incomplete session palettes before registering a theme.

#![expect(clippy::expect_used, reason = "theme fixtures must parse and load")]

use ratatui::style::Color;
use toml::Table;
use toml::Value;
use tui_pane::StyleSpec;
use tui_pane::Theme;
use tui_pane::ThemeId;
use tui_pane::ThemeRegistry;
use tui_pane::ThemeSource;

use super::SessionHue;
use super::files;
use crate::constants::DEFAULT_DARK_THEME;
use crate::constants::RESET_LATEST_ROLE;
use crate::constants::RESET_MIDDLE_ROLE;
use crate::constants::RESET_SOONEST_ROLE;

const WAITING_OVERRIDE_RGB: (u8, u8, u8) = (201, 190, 20);

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn theme_document() -> Table {
    toml::from_str(include_str!("../../themes/default_dark.toml")).expect("built-in document")
}

fn assert_rejected_file(mut document: Table) {
    const REJECTED: &str = "Rejected session fixture";
    document["variants"][0]["name"] = Value::String(REJECTED.to_owned());
    let dir = tempfile::tempdir().expect("isolated themes directory");
    let path = dir.path().join("invalid.toml");
    std::fs::write(
        &path,
        toml::to_string(&document).expect("serialize fixture"),
    )
    .expect("write invalid theme");
    let registry = ThemeRegistry::from_dir_with_parser(
        ThemeSource::Directory(dir.path()),
        super::builtins(),
        files::parse_family,
    );
    assert_eq!(
        registry.status().failed_files.len(),
        1,
        "invalid family is reported"
    );
    assert_eq!(registry.status().failed_files[0].0, path);
    assert!(
        registry.find(&ThemeId::new(REJECTED)).is_none(),
        "invalid variant is not registered"
    );
    assert!(
        registry.status().overridden.is_empty(),
        "rejected family cannot replace a builtin"
    );
}

#[test]
fn theme_session_palette_rejects_four_entries() {
    let mut document = theme_document();
    document["variants"][0]["session_hues"]
        .as_array_mut()
        .expect("session list")
        .truncate(4);
    assert_rejected_file(document);
}

#[test]
fn theme_session_palette_rejects_six_entries() {
    let mut document = theme_document();
    let entries = document["variants"][0]["session_hues"]
        .as_array_mut()
        .expect("session list");
    entries.push(entries[0].clone());
    assert_rejected_file(document);
}

#[test]
fn theme_session_palette_requires_the_list_in_each_variant() {
    let mut document = theme_document();
    document["variants"][0]
        .as_table_mut()
        .expect("variant")
        .remove("session_hues")
        .expect("session palette exists");
    assert_rejected_file(document);
}

#[test]
fn theme_session_palette_override_keeps_entry_order_and_inherits_status_inks() {
    let mut document = theme_document();
    let entries = document["variants"][0]["session_hues"]
        .as_array_mut()
        .expect("session list");
    entries.rotate_left(2);
    let expected: Vec<_> = entries
        .iter()
        .map(|entry| {
            let channel = |key| {
                u8::try_from(entry[key].as_integer().expect("RGB channel"))
                    .expect("eight-bit channel")
            };
            Color::Rgb(channel("r"), channel("g"), channel("b"))
        })
        .collect();
    document["variants"][0]["roles"] = Value::Table(toml::Table::new());
    let dir = tempfile::tempdir().expect("isolated themes directory");
    std::fs::write(
        dir.path().join("custom.toml"),
        toml::to_string(&document).expect("serialize fixture"),
    )
    .expect("write custom theme");
    let registry = ThemeRegistry::from_dir_with_parser(
        ThemeSource::Directory(dir.path()),
        super::builtins(),
        files::parse_family,
    );
    assert_eq!(
        registry.status().failed_files,
        [] as [(std::path::PathBuf, tui_pane::ThemeLoadError); 0]
    );
    let id = ThemeId::new(super::DEFAULT_DARK_THEME);
    assert_eq!(registry.status().overridden, std::slice::from_ref(&id));
    let loaded = &registry.find(&id).expect("override registered").theme;
    for (hue, expected) in SessionHue::ALL.into_iter().zip(expected) {
        assert_eq!(loaded.roles[hue.key()].color, expected, "{hue:?}");
    }
    assert_eq!(
        loaded.roles[super::CLAUDE_ROLE],
        super::default_dark().roles[super::CLAUDE_ROLE],
        "omitting status roles preserves built-in status colors"
    );
}

/// The three reset roles, in rank order, with the colors an override file gives them.
const RESET_OVERRIDES: [(&str, (u8, u8, u8)); 3] = [
    (RESET_SOONEST_ROLE, (0, 180, 90)),
    (RESET_MIDDLE_ROLE, (220, 180, 40)),
    (RESET_LATEST_ROLE, (60, 140, 250)),
];

/// The drawn color of each reset step: soonest, middle and latest.
fn reset_steps(theme: Theme) -> Vec<Color> {
    let previous = tui_pane::theme();
    tui_pane::set_active_theme(std::sync::Arc::new(theme));
    let steps = super::ResetRank::positions(RESET_OVERRIDES.len())
        .map(super::ResetRank::color)
        .collect();
    tui_pane::set_active_theme(previous);
    steps
}

/// Load `document` from a theme directory and return the variant that `id` names.
fn loaded_theme(document: &Table, id: &ThemeId) -> Theme {
    let dir = tempfile::tempdir().expect("theme directory");
    std::fs::write(
        dir.path().join("resets.toml"),
        toml::to_string(document).expect("theme text"),
    )
    .expect("write theme");
    let registry = ThemeRegistry::from_dir_with_parser(
        ThemeSource::Directory(dir.path()),
        super::builtins(),
        files::parse_family,
    );
    assert!(
        registry.status().failed_files.is_empty(),
        "{:?}",
        registry.status()
    );
    registry.find(id).expect("file theme").theme.clone()
}

fn check_reset_file(template: &str, explicit: bool) {
    let mut document: Table = toml::from_str(template).expect("theme template");
    let name = document["variants"][0]["name"]
        .as_str()
        .expect("variant name")
        .to_owned();
    let id = ThemeId::new(name);
    let builtin = super::builtins()
        .into_iter()
        .find(|variant| variant.id == id)
        .expect("matching built-in")
        .theme;
    let roles = document["variants"][0]["roles"]
        .as_table_mut()
        .expect("roles table");
    let expected: Vec<_> = RESET_OVERRIDES
        .into_iter()
        .map(|(key, (red, green, blue))| {
            if explicit {
                roles.insert(
                    key.to_owned(),
                    Value::Table(
                        toml::from_str(&format!("r = {red}\ng = {green}\nb = {blue}"))
                            .expect("reset RGB"),
                    ),
                );
                Color::Rgb(red, green, blue)
            } else {
                roles.remove(key);
                builtin.roles[key].color
            }
        })
        .collect();
    assert_eq!(
        reset_steps(loaded_theme(&document, &id)),
        expected,
        "reset ranks use the loaded roles, explicit={explicit}"
    );
}

#[test]
fn every_built_in_theme_file_sets_the_three_reset_roles() -> TestResult {
    for template in [
        include_str!("../../themes/default_dark.toml"),
        include_str!("../../themes/default_light.toml"),
        include_str!("../../themes/high_contrast.toml"),
        include_str!("../../themes/starter.toml"),
    ] {
        let document: Table = toml::from_str(template)?;
        for variant in document["variants"].as_array().ok_or("variants")? {
            let roles = variant["roles"].as_table().ok_or("roles")?;
            for (key, _) in RESET_OVERRIDES {
                assert!(roles.contains_key(key), "{} sets {key}", variant["name"]);
            }
        }
    }
    Ok(())
}

#[test]
fn a_new_custom_theme_without_reset_roles_takes_the_default_reset_colors() {
    const CUSTOM: &str = "Custom Without Resets";
    let mut document = theme_document();
    document["variants"][0]["name"] = Value::String(CUSTOM.to_owned());
    let roles = document["variants"][0]["roles"]
        .as_table_mut()
        .expect("roles table");
    for (key, _) in RESET_OVERRIDES {
        roles.remove(key);
    }
    let defaults = super::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == DEFAULT_DARK_THEME)
        .expect("default dark")
        .theme;
    assert_eq!(
        reset_steps(loaded_theme(&document, &ThemeId::new(CUSTOM))),
        reset_steps(defaults)
    );
}

#[test]
fn theme_file_dark_reset_roles_override_the_three_reset_roles() {
    check_reset_file(include_str!("../../themes/default_dark.toml"), true);
}

#[test]
fn theme_file_light_reset_roles_override_the_three_reset_roles() {
    check_reset_file(include_str!("../../themes/default_light.toml"), true);
}

#[test]
fn theme_file_dark_reset_roles_inherit_the_three_reset_roles() {
    check_reset_file(include_str!("../../themes/default_dark.toml"), false);
}

#[test]
fn theme_file_light_reset_roles_inherit_the_three_reset_roles() {
    check_reset_file(include_str!("../../themes/default_light.toml"), false);
}

#[test]
fn waiting_custom_themes_inherit_or_override_the_builtin_ink() -> TestResult {
    for template in [
        include_str!("../../themes/default_dark.toml"),
        include_str!("../../themes/default_light.toml"),
        include_str!("../../themes/high_contrast.toml"),
    ] {
        let document: Table = toml::from_str(template)?;
        let variants = document["variants"].as_array().ok_or("variants")?;
        for variant in variants {
            for explicit in [false, true] {
                let mut custom = document.clone();
                custom["variants"] = Value::Array(vec![variant.clone()]);
                let name = custom["variants"][0]["name"]
                    .as_str()
                    .ok_or("variant name")?
                    .to_owned();
                let builtin = super::builtins()
                    .into_iter()
                    .find(|builtin| builtin.id.as_str() == name)
                    .ok_or("built-in")?;
                let roles = custom["variants"][0]["roles"]
                    .as_table_mut()
                    .ok_or("roles")?;
                let expected = if explicit {
                    let (red, green, blue) = WAITING_OVERRIDE_RGB;
                    let color = Color::Rgb(red, green, blue);
                    roles.insert(
                        "waiting".into(),
                        Value::Table(toml::from_str(&format!(
                            "r = {red}\ng = {green}\nb = {blue}"
                        ))?),
                    );
                    StyleSpec::from_color(color)
                } else {
                    roles.remove("waiting");
                    builtin.theme.roles["waiting"]
                };
                let directory = tempfile::tempdir()?;
                std::fs::write(
                    directory.path().join("custom.toml"),
                    toml::to_string(&custom)?,
                )?;
                let registry = ThemeRegistry::from_dir_with_parser(
                    ThemeSource::Directory(directory.path()),
                    super::builtins(),
                    files::parse_family,
                );
                let loaded = registry.find(&ThemeId::new(name)).ok_or("loaded theme")?;
                assert_eq!(loaded.theme.roles["waiting"], expected);
            }
        }
    }
    Ok(())
}
