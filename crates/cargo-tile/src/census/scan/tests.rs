//! Shared scan fixtures and the metadata exec child.

use std::cell::Cell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use sysinfo::Pid;

#[cfg(any(target_os = "linux", target_os = "macos", test))]
use crate::birth_stamp;
use crate::birth_stamp::LifetimeEvidence;
#[cfg(any(target_os = "linux", test))]
use crate::birth_stamp::ProcessLifetime;
use crate::census::command_text::CommandText;
use crate::census::command_text::ScannerHome;
use crate::census::direct_capture::DirectAssociation;
use crate::census::direct_capture::SelectedProof;
use crate::census::invocation_cpu_accounting;
use crate::census::invocation_cpu_accounting::CompileMemoryShares;
use crate::census::invocation_cpu_accounting::CpuBaseline;
use crate::census::invocation_cpu_accounting::InvocationMeasurements;
use crate::census::invocation_cpu_accounting::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::census::process_identity::CaptureMembership;
use crate::census::process_identity::InvocationId;
use crate::census::process_identity::ProcessIdentity;
use crate::census::process_identity::VisibleParent;
use crate::constants::CARGO_DISPLAY_NAME;
use crate::constants::PID_SEPARATOR;
use crate::constants::RUN_LOG_PREFIX;
use crate::constants::RUN_LOG_SUFFIX;
use crate::constants::SCCACHE_BINARY;
use crate::progress::capture::Capture;
use crate::progress::capture_read::CaptureLookup;
use crate::progress::capture_roots::CaptureRoots;
use crate::registration::RegistrationCandidate;
use crate::registration::WorkingDirectoryIdentity;
use crate::settings::AssociationSelection;
use crate::settings::CaptureAssociation;
thread_local! {
    pub(super) static CENSUS_PREPARATIONS: Cell<usize> = const { Cell::new(0) };
    pub(super) static TOPOLOGY_PREPARATIONS: Cell<usize> = const { Cell::new(0) };
    pub(super) static CPU_ANCESTRY_WALKS: Cell<usize> = const { Cell::new(0) };
    pub(super) static NATIVE_DISCOVERY_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        pub(super) static SYSINFO_BASELINE_READS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Clone, Copy)]
pub(crate) enum FixtureExecution {
    OriginOnly,
    Working,
}

pub(crate) struct ShellRun {
    argv: [Vec<OsString>; 4],
}

impl ShellRun {
    pub(crate) fn new() -> Self {
        Self {
            argv: [
                ["/bin/sh", "/toolchain/bin/cargo", "build"]
                    .map(OsString::from)
                    .to_vec(),
                [
                    "script",
                    "-q",
                    "-c",
                    "'/toolchain/bin/cargo-tile-real' 'build' ",
                ]
                .map(OsString::from)
                .to_vec(),
                ["/toolchain/bin/cargo-tile-real", "build"]
                    .map(OsString::from)
                    .to_vec(),
                ["rustc", "--crate-name", "fixture"]
                    .map(OsString::from)
                    .to_vec(),
            ],
        }
    }

    pub(crate) fn records(&self, execution: FixtureExecution) -> ProcessObservations<'_> {
        ProcessObservations::new(
            [
                (100, 1, "sh", 10, 1),
                (200, 100, "script", 11, 2),
                (300, 200, "cargo", 12, 4),
                (400, 300, "rustc", 13, 128),
            ]
            .into_iter()
            .zip(&self.argv)
            .take(match execution {
                FixtureExecution::OriginOnly => 1,
                FixtureExecution::Working => 4,
            })
            .map(|((pid, parent, name, started, mib), argv)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.parent = ProcessField::Observed(Pid::from_u32(parent));
                record.name = ProcessField::Observed(OsStr::new(name));
                record.started = started;
                record.elapsed = 20 - started;
                record.memory = mib * 1024 * 1024;
                record.cpu = Measurement::Reading(if pid == 100 { 61.0 } else { 0.0 });
                record
            }),
        )
    }

    pub(crate) fn working_groups(&self, capture: &Capture, now: Instant) -> Vec<CargoGroup> {
        let mut sequence = CensusSequence::default();
        let mut groups = Vec::new();
        for sample in 0..13 {
            let mut records = self.records(FixtureExecution::Working);
            for (&pid, record) in &mut records.records {
                let work = match pid.as_u32() {
                    300 => 250,
                    400 => 750,
                    _ => 0,
                };
                record.accumulated = 100 + sample * work;
                record
                    .native_cpu
                    .set(Measurement::Reading(Duration::from_millis(
                        record.accumulated,
                    )));
            }
            groups = sequence.sample_counters_with_capture(
                &mut records,
                now + Duration::from_secs(sample),
                capture,
                &[],
            );
        }
        groups
    }

    pub(crate) fn capture(root: &Path) -> Capture {
        write_versioned_capture(root, 100, "continuity", "/work", "/writer", "build", "");
        verified_capture(root)
    }
}

#[path = "../native_capture_tests.rs"]
mod native_capture_tests;
#[path = "../scan_cpu_scenario_tests.rs"]
mod scan_cpu_scenario_tests;

use std::fs;
use std::process::Child;

use sysinfo::Users;
use tempfile::TempDir;
use tempfile::tempdir;
use tui_pane::TABLE_CELL;
use tui_pane::TileGrowth;

use super::InvocationExecution;
use super::OriginStart;
use super::*;
use crate::app::App;
use crate::birth_stamp::IdentityEvidence;
use crate::birth_stamp::KernelObservation;
use crate::birth_stamp::Observation;
use crate::census::cpu_observation::CpuScanPublication;
use crate::constants::CAPTURE_LIVE_RUNS_DIR;
use crate::constants::COMPILER_COLUMN;
use crate::constants::CPU_COLUMN;
use crate::constants::MANAGED_COLUMN;
use crate::constants::MEMORY_COLUMN;
use crate::constants::TABLE_HEADERS;
use crate::progress::capture::CaptureRootIndex;
use crate::progress::capture_read::CaptureRead;
use crate::progress::capture_read::RunState;
use crate::progress::capture_roots::AccountName;
use crate::registration::Registration;
use crate::render;
use crate::root_scan;
use crate::root_scan::EffectiveUser;
use crate::root_scan::RootOwner;
use crate::roster::FamilyHead;
use crate::settings;
use crate::tiles::TileContent;
use crate::tiles::TileDemands;

/// Reap the metadata fixture even if an assertion fails before its exec transition.
pub(super) struct MetadataProcess {
    /// The test owns stdin, lifetime, and cleanup of the sampled process.
    pub(super) child: Child,
}

impl Drop for MetadataProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Owned text supports borrowed records without a process or leaked allocation.
pub(super) struct CargoFixture {
    pub(super) pid:       u32,
    pub(super) argv:      Vec<OsString>,
    pub(super) directory: PathBuf,
}

impl CargoFixture {
    pub(super) fn observation(&self) -> ProcessObservation<'_> {
        let mut record = ProcessObservation::cargo(self.pid, &self.argv);
        record.directory = ProcessField::Observed(&self.directory);
        record
    }
}

pub(super) fn cargo_process(command: &str) -> CargoFixture {
    CargoFixture {
        pid:       if command == "test" { 101 } else { 100 },
        argv:      vec![OsString::from(CARGO_DISPLAY_NAME), OsString::from(command)],
        directory: std::env::current_dir().expect("test directory"),
    }
}

/// Measurements are supplied explicitly by tests that need process evidence.
pub(super) fn no_measurements() -> InvocationMeasurements {
    InvocationMeasurements {
        cpu_scan_publication: CpuScanPublication::Unobserved,
        memory_shares:        CompileMemoryShares::default(),
        compilers:            HashMap::new(),
        cpu:                  HashMap::new(),
        memory:               HashMap::new(),
    }
}

pub(super) use foreign_capture::assert_foreign_owned_duplicate;

mod foreign_capture {
    use super::*;

    pub(in crate::census::scan) fn assert_foreign_owned_duplicate(
        records: &ProcessObservations<'_>,
        birth: &str,
        expected_pids: &[u32],
    ) {
        let parent = tempdir().expect("shared capture parent");
        let roots = CaptureRoots::from_parent(parent.path());
        let EffectiveUser::Known(uid) = root_scan::effective_user() else {
            panic!("fixture owner");
        };
        let users = Users::new_with_refreshed_list();
        let foreign_uid = (0..=u32::MAX)
            .rev()
            .find(|candidate| {
                *candidate != uid && users.iter().all(|user| **user.id() != *candidate)
            })
            .expect("unassigned uid");
        let own = parent.path().join(uid.to_string());
        let foreign = parent.path().join(foreign_uid.to_string());
        write_versioned_capture(
            &own,
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "Blocking waiting for file lock on build directory\n",
        );
        write_versioned_capture(
            &foreign,
            10,
            "generation",
            "/unused-directory",
            "/writer",
            "test",
            "PASS [0.010s] (7/13) unused\n",
        );
        let publication = own.join("state/pids/10.generation");
        if birth.is_empty() {
            let bytes = String::from_utf8(directory_record_bytes("/writer"))
                .expect("fixture UTF-8")
                .replace("\0boot\x00100\0", "\0boot\0\0");
            fs::write(&publication, bytes).expect("unverifiable selected proof");
        }
        let IdentityEvidence::Available(stamp) = directory_record("/writer").identity().clone()
        else {
            panic!("fixture birth");
        };
        let mut capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
        });
        assert_eq!(capture.root_status.len(), 2);
        let rejected = &capture.root_status[1];
        assert_eq!(rejected.owner, RootOwner::Uid(uid));
        assert_eq!(rejected.root.uid, foreign_uid);
        assert!(matches!(
            rejected.state,
            crate::progress::capture_roots::RootReadStatus::ForeignOwned { .. }
        ));
        assert_eq!(rejected.confirmed, 0);
        assert_eq!(rejected.associations, [] as [CaptureAssociation; 0]);
        let mut census = Census::take(records.records.values());
        let groups = CensusSequence::assemble(&mut census, records, &capture, &[]);
        assert_eq!(
            groups
                .iter()
                .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
                .map(|row| row.pid)
                .collect::<Vec<_>>(),
            expected_pids
        );
        if birth.is_empty() {
            assert_eq!(groups, [] as [CargoGroup; 0]);
            assert_eq!(capture.confirmed(), []);
        } else {
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].rest, [] as [CargoProcess; 0]);
            let row = &groups[0].lead;
            assert_eq!(row.command, CommandText::of("cargo", &["build"]));
            assert_eq!(row.path, "~/project");
            assert_eq!(
                row.state,
                CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
            );
            let RowProvenance::Direct(context) = &row.provenance else {
                panic!("selected row keeps its accepted-root provenance");
            };
            assert_eq!(context.root, CaptureRootIndex(0));
            assert_eq!(context.account.uid, uid);
            assert_eq!(context.account.name, capture.root_status[0].account);
            if records.records.is_empty() {
                assert_registration_display(&groups, &capture);
            } else {
                assert_single_captured_group_display(&groups, &capture);
            }
        }
        census.associate_status(&mut capture, &groups);
        assert_eq!(capture.root_status[0].associations.len(), 1);
        assert!(matches!(&capture.root_status[0].associations[0].selection,
            AssociationSelection::Selected { key, unused, .. }
                if key.pid == 10 && key.root == CaptureRootIndex(0) && unused.is_empty()));
        assert!(publication.exists());
        assert!(own.join("run-generation-10.log").exists());
        assert!(foreign.join("state/pids/10.generation").exists());
        assert_ignored_duplicate_settings(&capture, &foreign, uid, foreign_uid);
    }
}

fn assert_ignored_duplicate_settings(
    capture: &Capture,
    foreign: &Path,
    uid: u32,
    foreign_uid: u32,
) {
    let mut app = crate::app::App::new_for_test().expect("settings app");
    app.root_status.clone_from(&capture.root_status);
    let settings = settings::rows(&app)
        .rows()
        .iter()
        .map(|row| row.value.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let owner = match &capture.root_status[0].account {
        AccountName::Resolved(name) => name.clone(),
        AccountName::Unavailable => uid.to_string(),
    };
    let ignored = format!(
        "{}: owned by {owner}, not by {foreign_uid} — ignored",
        foreign.canonicalize().expect("foreign path").display()
    );
    assert!(settings.contains(&ignored), "{settings}");
    assert!(!settings.contains("unused directory"), "{settings}");
    assert!(!settings.contains("/unused-directory"), "{settings}");
    // The own root's publication is the selection, and the ignored
    // duplicate is not listed as a proof that went unused.
    assert!(
        capture.root_status[0]
            .associations
            .iter()
            .any(|association| matches!(
                &association.selection,
                AssociationSelection::Selected { key, unused, .. }
                    if key.pid == 10 && unused.is_empty()
            )),
        "{:?}",
        capture.root_status[0].associations
    );
    assert!(!settings.contains("went unused"), "{settings}");
}

pub(super) fn assert_no_registration_rows(capture: &Capture) {
    assert_eq!(
        CensusSequence::assemble(
            &mut census_of(&[]),
            &ProcessObservations::default(),
            capture,
            &[],
        ),
        [] as [CargoGroup; 0]
    );
}

pub(super) fn account_settings_text(app: &App) -> String {
    settings::rows(app)
        .rows()
        .iter()
        .find(|row| row.label == settings::account_label(&app.root_status[0]))
        .expect("scanned account reaches Settings")
        .value
        .clone()
}

/// A scanned registration renders one qualified row with unavailable process measurements.
pub(super) fn assert_registration_display(groups: &[CargoGroup], capture: &Capture) -> String {
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].rest, [] as [CargoProcess; 0]);
    let row = &groups[0].lead;
    assert_eq!(row.pid, 10);
    assert_eq!(row.command, CommandText::of("cargo", &["build"]));
    assert_eq!(row.path, "~/project");
    assert_eq!(
        row.resource_use.cpu,
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    assert_eq!(
        row.resource_use.memory,
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    assert_eq!(
        row.subtree_resource_use.memory,
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    assert_eq!(row.resource_use.compiler, CompilerObservation::Unknown);
    assert_eq!(
        row.managed,
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    let lines = assert_single_captured_group_display(groups, capture);
    let text = lines.join("\n");
    let header = lines
        .iter()
        .find(|line| line.contains(TABLE_HEADERS[CPU_COLUMN]))
        .expect("table header");
    let rendered = lines
        .iter()
        .find(|line| line.contains("cargo build"))
        .expect("displayed registration row");
    assert_eq!(
        rendered
            .split_whitespace()
            .filter(|word| *word == "10")
            .count(),
        1,
        "{text}"
    );
    for column in [CPU_COLUMN, MEMORY_COLUMN, COMPILER_COLUMN, MANAGED_COLUMN] {
        assert_eq!(
            crate::census::measurement_cell(header, rendered, TABLE_HEADERS[column], &text),
            "",
            "{text}"
        );
    }
    text
}

fn assert_single_captured_group_display(groups: &[CargoGroup], capture: &Capture) -> Vec<String> {
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].rest, [] as [CargoProcess; 0]);
    let lines = render_group_lines(&groups[0]);
    let text = lines.join("\n");
    let account = match &capture.root_status[0].account {
        AccountName::Resolved(name) => name.clone(),
        AccountName::Unavailable => capture.root_status[0].root.uid.to_string(),
    };
    assert_eq!(
        text.matches(&format!("[{account}] ~/project")).count(),
        1,
        "{text}"
    );
    assert_eq!(text.matches("~/project").count(), 1, "{text}");
    assert_eq!(text.matches("cargo build").count(), 1, "{text}");
    lines
}

fn render_group_lines(group: &CargoGroup) -> Vec<String> {
    let mut roster = crate::roster::Roster::new();
    roster.observe(vec![group.clone()], Instant::now());
    assert_eq!(roster.groups().len(), 1);
    assert_eq!(roster.groups()[0].rows().count(), 1 + group.rest.len());
    assert_eq!(roster.tiled_ids(&[]), [group.id()]);
    let area = ratatui::layout::Rect::new(0, 0, 160, 24);
    render::cell_lines_for_test(&roster, &TileContent::Group(group.id()), area, 4)
}

/// Completed samples preserve one row and one heading through both source switches.
pub(super) fn assert_single_source_transitions(scans: &[Vec<CargoGroup>], pids: &[u32]) {
    let mut roster = crate::roster::Roster::new();
    let identity = scans[0][0].id();
    for (index, (scan, &pid)) in scans.iter().zip(pids).enumerate() {
        assert_eq!(scan.len(), 1);
        assert_eq!(scan[0].rest, [] as [CargoProcess; 0]);
        assert_eq!(scan[0].id(), identity);
        assert_eq!(scan[0].lead.pid, pid);
        assert_eq!(
            scan[0].lead.state,
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
        );
        if index == 1 {
            assert_eq!(scan[0].lead.resource_use.cpu, Measurement::Reading(42.0));
            assert_eq!(scan[0].lead.managed, Measurement::Reading(0));
            assert_eq!(
                scan[0].lead.resource_use.compiler,
                CompilerObservation::None
            );
        } else {
            assert_eq!(
                scan[0].lead.resource_use.cpu,
                Measurement::Unavailable(MeasurementAbsence::Unproven)
            );
            assert_eq!(
                scan[0].lead.managed,
                Measurement::Unavailable(MeasurementAbsence::Unproven)
            );
            assert_eq!(
                scan[0].lead.resource_use.compiler,
                CompilerObservation::Unknown
            );
        }
        roster.observe(scan.clone(), Instant::now());
        assert_eq!(roster.groups().len(), 1);
        assert_eq!(roster.groups()[0].rows().count(), 1);
        let area = ratatui::layout::Rect::new(0, 0, 160, 12);
        let text =
            render::cell_lines_for_test(&roster, &TileContent::Group(identity.clone()), area, 4)
                .join("\n");
        assert_eq!(text.matches("~/project").count(), 1, "{text}");
        assert_eq!(text.matches("cargo build").count(), 1, "{text}");
        assert_eq!(text.matches("blocked").count(), 1, "{text}");
        assert_eq!(
            text.split_whitespace()
                .filter(|word| *word == pid.to_string())
                .count(),
            1,
            "{text}"
        );
    }
}

/// Source transitions retain family assignments and the focused tile together.
pub(super) fn assert_family_source_transitions(scans: &[Vec<CargoGroup>]) {
    let mut roster = crate::roster::Roster::new();
    let mut grid = crate::tiles::TileGrid::new();
    let growth = TileGrowth {
        initial_rows:  1,
        fill:          tui_pane::TileFill::default(),
        widen_summary: false,
    };
    grid.set_layout(ratatui::layout::Rect::new(0, 0, 120, 40), growth);
    let mut expected_family = FamilyHead::NoChildren;
    for (index, scan) in scans.iter().enumerate() {
        roster.observe(scan.clone(), Instant::now());
        let visible = roster.visible(&[]);
        assert_eq!(visible.groups().len(), 1);
        let tracked = &visible.groups()[0];
        assert_eq!(tracked.rows().count(), 2);
        assert!(tracked.rows().all(|row| !row.is_ended()));
        if index == 0 {
            expected_family = tracked.lead.family();
        }
        assert_eq!(tracked.lead.family(), expected_family);
        assert!(matches!(
            expected_family,
            crate::roster::FamilyHead::Heads(_)
        ));
        let demands = TileDemands {
            summary:       3,
            summary_width: 0,
            groups:        vec![crate::tiles::TileDemand {
                id:   tracked.id.clone(),
                rows: 3,
            }],
        };
        grid.sync(&demands, growth);
        if index == 0 {
            grid.focus_cell(TABLE_CELL + 1);
        }
        assert!(
            grid.placements(ratatui::layout::Rect::new(0, 0, 120, 40), growth)
                .iter()
                .any(|placement| placement.content
                    == crate::tiles::TileContent::Group(tracked.id.clone())
                    && placement.frame.is_focused())
        );
    }
}

#[test]
fn metadata_exec_child() {
    let Some(directory) = std::env::var_os("CARGO_TILE_METADATA_EXEC_DIRECTORY") else {
        return;
    };
    println!("metadata ready");
    std::io::stdout().flush().expect("readiness");
    let mut release = String::new();
    std::io::stdin()
        .read_line(&mut release)
        .expect("exec release");
    let error = std::process::Command::new("sleep")
        .arg("60")
        .current_dir(directory)
        .exec();
    panic!("exec sleep: {error}");
}

/// A confirmed record supplies a real proof without using host process allocation.
pub(super) fn verified_capture(root: &Path) -> Capture {
    let IdentityEvidence::Available(stamp) = directory_record("/writer").identity().clone() else {
        return Capture::default();
    };
    Capture::take_from(root, |pid| {
        KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
    })
}

pub(super) fn assert_nested_selected_rows(command: &str, excluded: &[String]) {
    let root = tempdir().expect("capture root");
    write_versioned_capture(
        root.path(),
        10,
        "enclosing",
        "/writer/project",
        "/writer",
        command,
        "Blocking waiting for file lock on build directory\n",
    );
    write_versioned_capture(
        root.path(),
        20,
        "ended",
        "/writer/project",
        "/writer",
        "build",
        "ended\n",
    );
    let enclosing = root.path().join("state/pids/10.enclosing");
    let log = root.path().join("run-enclosing-10.log");
    let mut writer = fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .expect("live writer descriptor");
    let IdentityEvidence::Available(stamp) = directory_record("/writer").identity().clone() else {
        panic!("fixture birth");
    };
    let capture = Capture::take_from(root.path(), |pid| {
        KernelObservation::for_test(
            pid,
            if pid == 20 {
                Observation::Ended
            } else {
                Observation::Present(stamp.clone())
            },
        )
    });
    assert!(!root.path().join("state/pids/20.ended").exists());
    assert!(!root.path().join("run-ended-20.log").exists());
    assert!(enclosing.exists() && log.exists());
    let outer_argv = if command == "build" {
        vec!["cargo".into(), command.into()]
    } else {
        vec!["/writer/application".into()]
    };
    let check_argv = ["cargo".into(), "check".into(), "probe-nested-check".into()];
    let test_argv = ["cargo".into(), "test".into(), "probe-nested-test".into()];
    let ancestor_argv = [
        "cargo".into(),
        "test".into(),
        "probe-common-ancestor".into(),
    ];
    let mut ancestor = ProcessObservation::cargo(9, &ancestor_argv);
    ancestor.directory = ProcessField::Observed(Path::new("/writer/ancestor-directory"));
    let shim_argv = ["cargo".into(), command.into()];
    let mut shim = ProcessObservation::cargo(10, &shim_argv);
    shim.parent = ProcessField::Observed(Pid::from_u32(9));
    shim.directory = ProcessField::Observed(Path::new("/writer/project"));
    let mut outer = ProcessObservation::cargo(11, &outer_argv);
    outer.parent = ProcessField::Observed(Pid::from_u32(10));
    outer.directory = ProcessField::Observed(Path::new("/writer/project"));
    if command == "run" {
        outer.name = ProcessField::Observed(OsStr::new("application"));
    }
    let nested = [(12, &check_argv), (13, &test_argv)].map(|(pid, argv)| {
        let mut record = ProcessObservation::cargo(pid, argv);
        record.parent = ProcessField::Observed(Pid::from_u32(11));
        record.directory = ProcessField::Observed(Path::new("/writer/nested-directory"));
        record
    });
    let records =
        ProcessObservations::new([ancestor, shim, outer, nested[0].clone(), nested[1].clone()]);
    let mut sequence = CensusSequence::default();
    let groups = sequence.sample_with_capture(
        &records,
        &capture,
        excluded,
        ScannerHome::Known(Path::new("/writer")),
    );
    assert_nested_rows(&groups, command, excluded);
    writeln!(writer, "writer remains captured after reader scan").expect("continued capture write");
    assert!(
        fs::read_to_string(&log)
            .expect("retained log")
            .contains("writer remains captured after reader scan")
    );
    assert!(enclosing.exists());
    assert_eq!(verified_capture(root.path()).confirmed().len(), 1);
}

fn assert_nested_rows(groups: &[CargoGroup], command: &str, excluded: &[String]) {
    assert_eq!(groups.len(), 1, "nested invocations share one group");
    let group = &groups[0];
    assert_eq!(group.lead.pid, 9);
    assert_eq!(
        group.lead.command,
        CommandText::of("cargo", &["test", "probe-common-ancestor"])
    );
    let rows: Vec<_> = group.rest.iter().collect();
    assert_eq!(rows.len(), if excluded.is_empty() { 3 } else { 2 });
    for (pid, command, marker) in [
        (12, "check", "probe-nested-check"),
        (13, "test", "probe-nested-test"),
    ] {
        let matching: Vec<_> = rows.iter().filter(|row| row.pid == pid).collect();
        assert_eq!(matching.len(), 1);
        let row = matching[0];
        assert_eq!(row.command, CommandText::of("cargo", &[command, marker]));
        assert_eq!(row.path, "~/nested-directory");
        assert_eq!(
            row.state,
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
        );
        assert!(matches!(
            row.capture_membership,
            CaptureMembership::Enclosing(_)
        ));
    }
    assert_ne!(
        rows.iter()
            .find(|row| row.pid == 12)
            .expect("check")
            .invocation_id,
        rows.iter()
            .find(|row| row.pid == 13)
            .expect("test")
            .invocation_id
    );
    if excluded.is_empty() {
        assert_eq!(
            rows.iter()
                .filter(|row| row.command == CommandText::of("cargo", &[command]))
                .count(),
            1
        );
    } else {
        assert!(
            rows.iter()
                .all(|row| row.command != CommandText::of("cargo", &[command]))
        );
    }
    let lines = render_group_lines(group);
    let text = lines.join("\n");
    for row in std::iter::once(&group.lead).chain(&group.rest) {
        let command = format!("{} {}", row.command.program, row.command.line());
        let matching: Vec<_> = lines
            .iter()
            .filter(|line| line.contains(&command))
            .collect();
        assert_eq!(matching.len(), 1, "{text}");
        assert_eq!(
            matching[0]
                .split_whitespace()
                .filter(|word| *word == row.pid.to_string())
                .count(),
            1,
            "{text}"
        );
    }
    assert_eq!(text.matches("probe-common-ancestor").count(), 1, "{text}");
    assert_eq!(text.matches("probe-nested-check").count(), 1, "{text}");
    assert_eq!(text.matches("probe-nested-test").count(), 1, "{text}");
    assert_eq!(text.matches("blocked").count(), rows.len(), "{text}");
    if !excluded.is_empty() {
        assert!(!text.contains(&format!("cargo {command}")), "{text}");
    }
}

pub(super) fn assert_accepted_rewrite_rows(registered: &[&str], executed: &[&str]) {
    let (root, records_argv) = rewrite_capture(registered, executed);
    let mut record = ProcessObservation::cargo(11, &records_argv);
    record.parent = ProcessField::Observed(Pid::from_u32(10));
    record.directory = ProcessField::Observed(Path::new("/writer/project"));
    let records = ProcessObservations::new([record]);
    let mut capture = verified_capture(root.path());
    let mut census = Census::take(records.records.values());
    let groups = CensusSequence::assemble(&mut census, &records, &capture, &[]);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].rest, [] as [CargoProcess; 0]);
    let row = &groups[0].lead;
    assert_eq!(row.pid, 10);
    assert_eq!(row.command, CommandText::of("cargo", registered));
    assert_eq!(
        row.state,
        CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
    );
    assert!(matches!(
        census.direct_capture(&capture, Pid::from_u32(11)),
        DirectAssociation::Direct(_)
    ));
    census.associate_status(&mut capture, &groups);
    let mut app = crate::app::App::new_for_test().expect("settings app");
    app.root_status.clone_from(&capture.root_status);
    assert!(
        capture.root_status[0]
            .associations
            .iter()
            .any(|association| {
                association.pid == 11
                    && matches!(&association.selection,
                    AssociationSelection::Selected { key, proof: SelectedProof::Confirmed, .. }
                        if key.pid == 10)
            }),
        "{:?}",
        capture.root_status[0].associations
    );
    assert!(root.path().join("state/pids/10.generation").exists());
    assert!(root.path().join("run-generation-10.log").exists());
}

pub(super) fn assert_rejected_rewrite_rows(registered: &[&str], executed: &[&str]) {
    let (root, records_argv) = rewrite_capture(registered, executed);
    let mut record = ProcessObservation::cargo(11, &records_argv);
    record.parent = ProcessField::Observed(Pid::from_u32(10));
    record.directory = ProcessField::Observed(Path::new("/writer/project"));
    let records = ProcessObservations::new([record]);
    let capture = verified_capture(root.path());
    let groups = CensusSequence::default().sample_capture(&records, &capture);
    let rows: Vec<_> = groups
        .iter()
        .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
        .collect();
    assert_eq!(rows.len(), 2);
    for (pid, arguments) in [(10, registered), (11, executed)] {
        let matching: Vec<_> = rows.iter().filter(|row| row.pid == pid).collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].command, CommandText::of("cargo", arguments));
    }
    assert_ne!(rows[0].invocation_id, rows[1].invocation_id);
    assert!(root.path().join("state/pids/10.generation").exists());
    assert!(root.path().join("run-generation-10.log").exists());
}

fn rewrite_capture(registered: &[&str], executed: &[&str]) -> (TempDir, Vec<OsString>) {
    let root = tempdir().expect("capture root");
    write_versioned_capture(
        root.path(),
        10,
        "generation",
        "/writer/project",
        "/writer",
        "check",
        "Blocking waiting for file lock on build directory\n",
    );
    let bytes = directory_record_bytes("/writer");
    let fields = bytes.split(|byte| *byte == 0).take(7).collect::<Vec<_>>();
    let mut bytes = fields.join(&0);
    bytes.push(0);
    bytes.extend_from_slice(registered.len().to_string().as_bytes());
    bytes.push(0);
    for argument in registered {
        bytes.extend_from_slice(argument.as_bytes());
        bytes.push(0);
    }
    fs::write(root.path().join("state/pids/10.generation"), bytes).expect("registered arguments");
    (
        root,
        std::iter::once("cargo")
            .chain(executed.iter().copied())
            .map(OsString::from)
            .collect(),
    )
}

/// Bound failed worker handshakes without imposing a startup speed threshold.
pub(super) const WORKER_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// A capture directory holding one live run's log per entry.
pub(super) fn capture_root(runs: &[(u32, &str)]) -> TempDir {
    let root = tempdir().expect("temp dir must be created");
    let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
    fs::create_dir_all(&markers).expect("marker dir must be created");
    for (pid, output) in runs {
        let name = format!("{RUN_LOG_PREFIX}20260824-084300{PID_SEPARATOR}{pid}{RUN_LOG_SUFFIX}");
        fs::write(root.path().join(name), output).expect("run log must be written");
        fs::write(
            markers.join(pid.to_string()),
            "/writer/project\tcargo build",
        )
        .expect("live marker must be written");
    }
    root
}

/// A census that knows nothing but who each process's parent is,
/// which is all the capture walk reads.
pub(super) fn census_of(parents: &[(u32, u32)]) -> Census {
    Census {
        executing:                HashMap::new(),
        identities:               parents
            .iter()
            .flat_map(|&pair| <[u32; 2]>::from(pair))
            .map(|pid| (Pid::from_u32(pid), InvocationId::for_test(pid)))
            .collect(),
        capture_boundaries:       HashSet::new(),
        capture_wrappers:         HashSet::new(),
        capture_mismatches:       HashSet::new(),
        lifetimes:                HashMap::new(),
        eligibility:              HashMap::new(),
        registration_eligibility: HashMap::new(),
        parents:                  parents
            .iter()
            .map(|&(child, parent)| (Pid::from_u32(child), Pid::from_u32(parent)))
            .collect(),
        topology:                 std::cell::OnceCell::new(),
        cargo:                    Vec::new(),
        rowless_cargo:            Vec::new(),
        dispatched_runs:          HashMap::new(),
        compilers:                Vec::new(),
        cpu:                      HashMap::new(),
        memory:                   HashMap::new(),
        accumulated:              HashMap::new(),
        owners:                   HashMap::new(),
        registration_rows:        Vec::new(),
    }
}

/// Keep the same wire fields for parser and capture-annotation fixtures.
pub(super) fn directory_record_bytes(home: &str) -> Vec<u8> {
    [
        "cargo-tile-v2",
        "generation",
        "boot",
        "100",
        "run-generation-10.log",
        "/writer/project",
        home,
        "1",
        "build",
        "",
    ]
    .join("\0")
    .into_bytes()
}

/// A candidate record can provide text without becoming a verified row source.
pub(super) fn directory_record(home: &str) -> RegistrationCandidate {
    match Registration::parse(&directory_record_bytes(home)).expect("well-formed directory record")
    {
        Registration::Versioned(record) => Ok(record),
        Registration::Legacy(_) => Err("expected v2 record"),
    }
    .expect("fixture writes a versioned record")
}

/// A process-table row whose absolute cwd is independent of display shortening.
pub(super) fn directory_row() -> CargoProcess {
    CargoProcess {
        invocation_id:        InvocationId::for_test(10),
        execution:            InvocationExecution::Command,
        capture_membership:   CaptureMembership::Outside,
        provenance:           RowProvenance::Uncaptured(ProcessOwner::Unavailable),
        path:                 "~/project".to_owned(),
        directory_identity:   WorkingDirectoryIdentity::Absolute("/writer/project".into()),
        pid:                  10,
        parent:               VisibleParent::None,
        origin_start:         OriginStart::Process {
            epoch:   0,
            elapsed: Duration::from_secs(1),
        },
        resource_use:         InvocationResourceUse {
            cpu:      Measurement::Reading(0.0),
            memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
            compiler: CompilerObservation::None,
        },
        subtree_resource_use: InvocationResourceUse {
            cpu:      Measurement::Reading(0.0),
            memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
            compiler: CompilerObservation::None,
        },
        state:                CaptureLookup::Unregistered,
        managed:              Measurement::Reading(0),
        nested:               false,
        command:              CommandText::of("cargo", &["build"]),
    }
}

/// Exercise the annotation path when asserting the nearest ancestor's reading.
pub(super) fn captured_state(census: &Census, capture: &Capture, pid: u32) -> CaptureLookup {
    let mut row = directory_row();
    row.pid = pid;
    census.annotate_capture(&mut row, capture, ScannerHome::Unavailable);
    row.state
}

/// Resolve fixture paths through production code without scanning the user's root.
pub(super) fn resolved_test_roots(paths: &[&Path]) -> CaptureRoots { CaptureRoots::for_test(paths) }

/// Different roots may name different logs and commands for the same shim pid.
pub(super) fn write_versioned_capture(
    root: &Path,
    pid: u32,
    generation: &str,
    directory: &str,
    home: &str,
    command: &str,
    output: &str,
) {
    let markers = root.join(CAPTURE_LIVE_RUNS_DIR);
    fs::create_dir_all(&markers).expect("registration directory");
    let log = format!("run-{generation}-{pid}.log");
    let record = [
        "cargo-tile-v2",
        generation,
        "boot",
        "100",
        &log,
        directory,
        home,
        "1",
        command,
        "",
    ]
    .join("\0");
    fs::write(markers.join(format!("{pid}.{generation}")), record).expect("versioned registration");
    fs::write(root.join(log), output).expect("capture output");
}

/// Same process identity for CPU boundary fixtures; only the counter changes.
pub(super) fn cpu_baseline(accumulated: u64) -> CpuBaseline {
    CpuBaseline {
        lifetime: LifetimeEvidence::Available(birth_stamp::ProcessLifetime::for_test(1)),
        accumulated,
    }
}
