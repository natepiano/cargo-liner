//! The handler rejects incomplete session palettes before registering a theme.

#![expect(clippy::expect_used, reason = "theme fixtures must parse and load")]

use ratatui::style::Color;
use toml::Table;
use toml::Value;
use tui_pane::ThemeId;
use tui_pane::ThemeRegistry;
use tui_pane::ThemeSource;

use super::SessionHue;
use super::files;

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
    let expected = if explicit {
        roles.insert(
            "reset_nearest".to_owned(),
            Value::Table(toml::from_str("r = 127\ng = 191\nb = 110").expect("nearest RGB")),
        );
        roles.insert(
            "reset_farthest".to_owned(),
            Value::Table(toml::from_str("r = 69\ng = 196\nb = 153").expect("farthest RGB")),
        );
        [Color::Rgb(127, 191, 110), Color::Rgb(69, 196, 153)]
    } else {
        roles.remove("reset_nearest");
        roles.remove("reset_farthest");
        [
            builtin.roles["reset_nearest"].color,
            builtin.roles["reset_farthest"].color,
        ]
    };
    let dir = tempfile::tempdir().expect("theme directory");
    std::fs::write(
        dir.path().join("resets.toml"),
        toml::to_string(&document).expect("theme text"),
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
    let previous = tui_pane::theme();
    tui_pane::set_active_theme(std::sync::Arc::new(
        registry.find(&id).expect("file override").theme.clone(),
    ));
    let actual: Vec<_> = super::ResetRank::positions(2)
        .map(super::ResetRank::color)
        .collect();
    tui_pane::set_active_theme(previous);
    assert_eq!(
        actual, expected,
        "reset ranks use loaded endpoints, explicit={explicit}"
    );
}

#[test]
fn theme_file_dark_reset_roles_override_both_rank_ends() {
    check_reset_file(include_str!("../../themes/default_dark.toml"), true);
}

#[test]
fn theme_file_light_reset_roles_override_both_rank_ends() {
    check_reset_file(include_str!("../../themes/default_light.toml"), true);
}

#[test]
fn theme_file_dark_reset_roles_inherit_both_rank_ends() {
    check_reset_file(include_str!("../../themes/default_dark.toml"), false);
}

#[test]
fn theme_file_light_reset_roles_inherit_both_rank_ends() {
    check_reset_file(include_str!("../../themes/default_light.toml"), false);
}
