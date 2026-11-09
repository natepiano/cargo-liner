use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

use super::duration_label;
use super::forwards_json_capture_arguments;
#[cfg(any(target_os = "linux", test))]
use super::is_transparent;
use super::registration_directory;
use super::subtree_resource_totals;
use crate::census::command_text::ScannerHome;
use crate::census::invocation_cpu_accounting::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::census::process_identity::InvocationId;
use crate::census::process_identity::VisibleParent;
#[cfg(test)]
use crate::census::scan::resource_use::Compiler;
use crate::census::scan::resource_use::CompilerObservation;
use crate::census::scan::resource_use::InvocationResourceUse;
use crate::census::scan::tests;
use crate::constants::SCCACHE_BINARY;

#[test]
fn registration_directory_shortens_only_the_same_writer_and_scanner_home() {
    let record = tests::directory_record("/writer");
    assert_eq!(record.directory(), Path::new("/writer/project"));
    assert_eq!(
        registration_directory(&record, ScannerHome::Known(Path::new("/writer"))),
        "~/project"
    );
    assert_eq!(
        registration_directory(&record, ScannerHome::Known(Path::new("/other"))),
        "/writer/project"
    );
    assert_eq!(
        registration_directory(&record, ScannerHome::Unavailable),
        "/writer/project"
    );
    assert_eq!(
        registration_directory(
            &tests::directory_record(""),
            ScannerHome::Known(Path::new("/writer"))
        ),
        "/writer/project"
    );
}

#[test]
fn a_custom_writer_home_does_not_use_the_scanners_tilde_prefix() {
    assert_eq!(
        registration_directory(
            &tests::directory_record("/custom"),
            ScannerHome::Known(Path::new("/writer"))
        ),
        "/writer/project"
    );
}

/// A shell or login process passed a command through rather than
/// starting it, and is marked so the cell can decide whether to
/// draw it.
#[test]
fn a_shell_and_a_login_are_marked_as_passing_through() {
    for name in ["zsh", "bash", "sh", "login"] {
        assert!(is_transparent(OsStr::new(name)), "{name}");
    }
}

#[test]
fn nu_is_a_transparent_launcher() {
    assert!(is_transparent(OsStr::new("nu")));
    assert!(is_transparent(OsStr::new("/nix/store/shell/bin/nu")));
}

#[test]
fn what_started_a_command_is_never_marked() {
    for name in ["zed", "iTerm2", "node", "cargo-mend"] {
        assert!(!is_transparent(OsStr::new(name)), "{name}");
    }
}

/// The list as it reaches [`CommandText::is_hidden`] once
/// the config has turned it into owned strings.

#[test]
fn duration_stays_minutes_and_seconds_under_an_hour() {
    assert_eq!(duration_label(57), "00:57");
    assert_eq!(duration_label(3599), "59:59");
}

#[test]
fn duration_widens_to_hours_once_pathological() {
    assert_eq!(duration_label(3600), "01:00:00");
    assert_eq!(duration_label(45_296), "12:34:56");
}

#[test]
fn a_shared_pid_contributes_once_to_its_parent_subtree() {
    let mut parent = tests::directory_row();
    parent.resource_use = InvocationResourceUse {
        cpu:      Measurement::Reading(10.0),
        memory:   Measurement::Reading(100),
        compiler: CompilerObservation::None,
    };
    let mut unproven = tests::directory_row();
    unproven.invocation_id = InvocationId::for_test(21);
    unproven.pid = 20;
    unproven.parent = VisibleParent::Invocation {
        id:  parent.invocation_id.clone(),
        pid: parent.pid,
    };
    unproven.resource_use = InvocationResourceUse::unavailable(MeasurementAbsence::Unproven);
    let mut known = unproven.clone();
    known.invocation_id = InvocationId::for_test(20);
    known.resource_use = InvocationResourceUse {
        cpu:      Measurement::Reading(20.0),
        memory:   Measurement::Reading(200),
        compiler: CompilerObservation::Running(Compiler {
            name:  SCCACHE_BINARY,
            count: 2,
        }),
    };
    let mut also_known = known.clone();
    also_known.invocation_id = InvocationId::for_test(22);
    for children in [
        [&unproven, &known],
        [&known, &unproven],
        [&known, &also_known],
    ] {
        let totals = subtree_resource_totals(&[&parent, children[0], children[1]]);
        assert_eq!(totals[0].cpu, Measurement::Reading(30.0));
        assert_eq!(totals[0].memory, Measurement::Reading(300));
        assert_eq!(totals[0].compiler, known.resource_use.compiler);
    }
}

#[test]
fn json_capture_forwarding_removes_all_quiet_flags_only_before_the_separator() {
    for format in [
        vec!["--message-format=json"],
        vec!["--message-format", "json-diagnostic-rendered-ansi"],
    ] {
        let registered: Vec<OsString> = ["check", "--quiet", "-q", "--quiet"]
            .into_iter()
            .chain(format.iter().copied())
            .chain(["--", "--quiet", "-q"])
            .map(OsString::from)
            .chain([OsString::from_vec(b"package\xff".to_vec())])
            .collect();
        let rewritten: Vec<OsString> = std::iter::once("check")
            .chain(format.iter().copied())
            .chain(["--", "--quiet", "-q"])
            .map(OsString::from)
            .chain([OsString::from_vec(b"package\xff".to_vec())])
            .collect();
        assert!(forwards_json_capture_arguments(&rewritten, &registered));
        let mut partial = rewritten.clone();
        partial.insert(1, OsString::from("-q"));
        assert!(!forwards_json_capture_arguments(&partial, &registered));
        let mut changed = rewritten.clone();
        changed[0] = OsString::from("build");
        assert!(!forwards_json_capture_arguments(&changed, &registered));
        let mut changed = rewritten.clone();
        changed.pop();
        changed.push(OsString::from("package"));
        assert!(!forwards_json_capture_arguments(&changed, &registered));
        let mut changed = rewritten;
        changed.remove(changed.len() - 2);
        assert!(!forwards_json_capture_arguments(&changed, &registered));
    }
}

#[test]
fn json_capture_forwarding_rejects_non_json_and_post_separator_formats() {
    for registered in [
        vec!["check", "--quiet"],
        vec!["check", "--quiet", "--message-format=human"],
        vec!["check", "--quiet", "--", "--message-format=json"],
        vec!["check", "--quiet", "--message-format", "--", "json"],
    ] {
        let registered: Vec<_> = registered.into_iter().map(OsString::from).collect();
        let mut rewritten = registered.clone();
        rewritten.remove(1);
        assert!(!forwards_json_capture_arguments(&rewritten, &registered));
    }
}
