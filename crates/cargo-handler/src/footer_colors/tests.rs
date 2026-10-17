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

use super::*;

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
