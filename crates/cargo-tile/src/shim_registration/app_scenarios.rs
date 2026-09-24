//! Reader scenarios driven through an in-process `App` and a test terminal backend.

#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use sysinfo::Pid;
use tempfile::TempDir;
use tui_pane::GlobalAction;
use tui_pane::NavAction;
use tui_pane::Navigation;
use tui_pane::SettingStep;
use tui_pane::SettingsNavigation;
use tui_pane::TILE_ROWS_CONTENT_LABEL;
use tui_pane::ToastDuration;
use tui_pane::ToastVisualDeadline;

use crate::app::App;
use crate::birth_stamp::BirthStamp;
use crate::birth_stamp::IdentityEvidence;
use crate::birth_stamp::KernelObservation;
use crate::birth_stamp::Observation;
use crate::capture;
use crate::census::CargoProcess;
use crate::census::CompilerObservation;
use crate::census::Measurement;
use crate::census::RowProvenance;
use crate::census::VisibleParent;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::census::scan::CensusSequence;
use crate::census::scan::ProcessField;
use crate::census::scan::ProcessObservation;
use crate::census::scan::ProcessObservations;
use crate::constants::LOCK_WAIT_MARKER;
use crate::constants::NOTICE_TOAST_VISIBLE;
use crate::constants::SUPPORTED_REGISTRATION_VERSION;
use crate::constants::UNAVAILABLE_MEASUREMENT;
use crate::hook;
use crate::hook::NewerShim;
use crate::hook::Startup;
use crate::progress::capture::Capture;
use crate::progress::capture_roots::AccountCaptureDirectory;
use crate::progress::capture_roots::AccountName;
use crate::progress::capture_roots::CaptureCleanup;
use crate::progress::capture_roots::CaptureRoot;
use crate::progress::capture_roots::RootReadStatus;
use crate::render;
use crate::root_scan::RootOwner;
use crate::settings;
use crate::tiles::TileContent;

#[test]
fn settings_scroll_reaches_every_account_and_later_settings() {
    let mut app = App::new_for_test().expect("isolated settings app");
    app.root_status = (1000..1024)
        .map(|uid| AccountCaptureDirectory {
            root:         CaptureRoot {
                path: format!("/capture/{uid}").into(),
                uid,
                cleanup: CaptureCleanup::AccountNextRun,
            },
            owner:        RootOwner::Uid(uid),
            account:      AccountName::Unavailable,
            state:        RootReadStatus::Readable,
            confirmed:    0,
            diagnostics:  Vec::new(),
            associations: Vec::new(),
        })
        .collect();
    let directories: Vec<_> = app
        .root_status
        .iter()
        .map(|status| status.root.path.display().to_string())
        .collect();
    let keymap = Rc::clone(&app.keymap);
    keymap.dispatch_framework_global(GlobalAction::OpenSettings, &mut app);
    let mut terminal = Terminal::new(TestBackend::new(240, 10)).expect("settings terminal");
    let initial = draw(&mut terminal, &mut app);
    assert!(selected_line(&initial).contains("mode"), "{initial}");
    assert!(
        directories.iter().all(|path| !initial.contains(path)),
        "{initial}"
    );

    let mut pending = directories.clone();
    for _ in 0..settings::rows(&app).rows().len() {
        navigate(&mut app, NavAction::Down);
        let rendered = draw(&mut terminal, &mut app);
        pending.retain(|path| !selected_line(&rendered).contains(path));
        if pending.is_empty() {
            break;
        }
    }
    assert!(pending.is_empty(), "accounts not selectable: {pending:?}");
    let rendered = draw(&mut terminal, &mut app);
    let account = directories
        .iter()
        .find(|path| selected_line(&rendered).contains(*path))
        .expect("last selected account");
    let before = toml::to_string(&app.loaded_config.config).expect("serialize config");
    // Enter dispatches this same step; account rows must remain read-only.
    settings::cycle(&mut app, SettingStep::Next);
    navigate(&mut app, NavAction::Right);
    navigate(&mut app, NavAction::Left);
    assert_eq!(
        toml::to_string(&app.loaded_config.config).expect("serialize config"),
        before
    );
    assert!(selected_line(&draw(&mut terminal, &mut app)).contains(account));

    terminal.backend_mut().resize(240, 9);
    assert!(selected_line(&draw(&mut terminal, &mut app)).contains(account));
    let mut pending = vec!["excluded", "hidden when idle", "config", "themes", "keymap"];
    for _ in 0..settings::rows(&app).rows().len() {
        navigate(&mut app, NavAction::Down);
        let rendered = draw(&mut terminal, &mut app);
        let selected = selected_line(&rendered)
            .split('▶')
            .nth(1)
            .expect("selection cursor")
            .trim_start();
        pending.retain(|label| !selected.starts_with(label));
        if pending.is_empty() {
            break;
        }
    }
    assert!(
        pending.is_empty(),
        "later settings not selectable: {pending:?}"
    );
}

#[test]
fn startup_newer_toast_expires_and_settings_retain_recovery() {
    assert_startup(Startup {
        kept_newer: vec![newer_shim()],
        ..Startup::default()
    });
}

#[test]
fn startup_newer_failure_keeps_notices_separate_after_expiry() {
    assert_startup(Startup {
        kept_newer: vec![newer_shim()],
        failed: vec![("z-broken".into(), "installation lock is a directory".into())],
        ..Startup::default()
    });
}

#[test]
fn startup_truncated_refusal_persists_until_dismissed_and_stays_in_settings() {
    assert_startup(Startup {
        failed: vec![("a-newer".into(), "incomplete shim version line".into())],
        ..Startup::default()
    });
}

fn newer_shim() -> NewerShim {
    NewerShim {
        toolchain: "a-newer".into(),
        installed: SUPPORTED_REGISTRATION_VERSION + 1,
        supported: SUPPORTED_REGISTRATION_VERSION,
    }
}

fn assert_startup(startup: Startup) {
    let mut expected = Vec::new();
    for shim in &startup.kept_newer {
        expected.push(("Newer capture shim kept", format!(
            "{}: newer shim v{} kept; this reader is older and supports v{}; upgrade and restart the reader",
            shim.toolchain, shim.installed, shim.supported,
        )));
    }
    for (name, error) in &startup.failed {
        expected.push(("Capture shim", format!("{name}: not installed: {error}")));
    }
    let mut app = App::new_for_test().expect("isolated startup app");
    app.loaded_config.config.capture.auto_install = true;
    app.framework
        .toasts
        .settings_mut()
        .animation
        .entrance_duration =
        ToastDuration::try_from_secs("entrance_duration", 0.0).expect("instant toast entrance");
    hook::with_startup_for_test(startup, || capture::stand_up(&mut app));
    let now = Instant::now();
    let entrance = app
        .framework
        .toasts
        .settings()
        .animation
        .entrance_duration
        .get();
    app.framework.toasts.prune(now + entrance);
    let views = app.framework.toasts.active_views(now + entrance);
    assert_eq!(views.len(), expected.len());
    for (view, (title, body)) in views.iter().zip(&expected) {
        assert_eq!(view.title(), *title);
        assert_eq!(view.body(), body);
    }
    let mut terminal = Terminal::new(TestBackend::new(240, 60)).expect("startup terminal");
    let rendered = draw(&mut terminal, &mut app);
    for (title, body) in &expected {
        assert!(popup_text(&rendered, title).contains(body), "{rendered}");
    }
    assert_settings_notices(&mut terminal, &mut app, &expected);

    let visible =
        NOTICE_TOAST_VISIBLE.max(app.framework.toasts.settings().status_toast_visible.get());
    let expiry = now + entrance + visible;
    app.framework.toasts.prune(expiry);
    let mut exited = expiry;
    for _ in 0..terminal.backend().buffer().area.height {
        match app.framework.toasts.next_visual_change_deadline(exited) {
            ToastVisualDeadline::At(next) => {
                exited = next;
                app.framework.toasts.prune(exited);
            },
            ToastVisualDeadline::NoVisualChangeScheduled => break,
        }
    }
    let remaining = app.framework.toasts.active_views(exited);
    let failures: Vec<_> = expected
        .iter()
        .filter(|(title, _)| *title == "Capture shim")
        .collect();
    assert_eq!(remaining.len(), failures.len());
    for (view, (title, body)) in remaining.iter().zip(failures) {
        assert_eq!(view.title(), *title);
        assert_eq!(view.body(), body);
    }
    let rendered = draw(&mut terminal, &mut app);
    assert!(!rendered.contains("Newer capture shim kept"), "{rendered}");
    for _ in &remaining {
        assert!(app.framework.toasts.dismiss_focused());
    }

    for _ in 0..2 {
        assert_settings_notices(&mut terminal, &mut app, &expected);
    }
}

fn assert_settings_notices(
    terminal: &mut Terminal<TestBackend>,
    app: &mut App,
    expected: &[(&str, String)],
) {
    let keymap = Rc::clone(&app.keymap);
    keymap.dispatch_framework_global(GlobalAction::OpenSettings, app);
    let rendered = draw(terminal, app);
    assert!(rendered.contains("Notices:"), "{rendered}");
    let rows = settings::rows(app).rows().to_vec();
    let notices: Vec<_> = rows.iter().filter(|row| row.label == "capture").collect();
    assert_eq!(notices.len(), expected.len());
    for (row, (_, body)) in notices.iter().zip(expected) {
        assert_eq!(&row.value, body);
        assert!(
            popup_text(&rendered, "Settings").contains(body),
            "notice missing from settings: {rendered}"
        );
    }
    keymap.dispatch_framework_global(GlobalAction::Dismiss, app);
    assert!(!draw(terminal, app).contains("Notices:"));
}

fn popup_text(rendered: &str, title: &str) -> String {
    let lines: Vec<Vec<char>> = rendered
        .lines()
        .map(|line| line.chars().collect())
        .collect();
    let top = rendered
        .lines()
        .position(|line| line.contains(title))
        .expect("popup title");
    let left = lines[top]
        .iter()
        .position(|cell| *cell == '┌')
        .expect("popup left border");
    let right = lines[top]
        .iter()
        .rposition(|cell| *cell == '┐')
        .expect("popup right border");
    lines[top + 1..]
        .iter()
        .take_while(|line| line[left] != '└')
        .map(|line| line[left + 1..right].iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn navigate(app: &mut App, action: NavAction) {
    SettingsNavigation::<App>::dispatcher()(action, *app.framework.focused(), app);
}

fn selected_line(rendered: &str) -> &str {
    rendered
        .lines()
        .find(|line| line.contains('▶'))
        .expect("selected settings row")
}

fn draw(terminal: &mut Terminal<TestBackend>, app: &mut App) -> String {
    let keymap = Rc::clone(&app.keymap);
    terminal
        .draw(|frame| render::draw(frame, app, &keymap))
        .expect("draw app");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn mixed_v2_v3_captures_render_blocked_rows_without_rewriting_publications() {
    let entries = [
        ("cargo-tile-v3", 100, "current-command"),
        ("cargo-tile-v2", 200, "older-command"),
    ];
    let (root, capture, publications) = published_capture(&entries);
    let rows = [("cargo-tile-v3", 101, "current-command"), entries[1]];
    let argv = ["cargo", "build", "current-command"].map(OsString::from);
    let mut parent = ProcessObservation::cargo(100, &argv);
    parent.directory = ProcessField::Observed(Path::new("/writer/project"));
    parent.uid = ProcessField::Observed(capture.root_status[0].root.uid);
    let mut current = ProcessObservation::cargo(101, &argv);
    current.parent = ProcessField::Observed(Pid::from_u32(100));
    current.directory = ProcessField::Observed(Path::new("/writer/project"));
    current.uid = ProcessField::Observed(capture.root_status[0].root.uid);
    let records = ProcessObservations::new([parent.clone(), current.clone()]);
    let mut sequence = CensusSequence::default();
    let mut app = App::new_for_test().expect("mixed capture app");
    for _ in 0..2 {
        let groups = sequence.sample_capture(&records, &capture);
        assert_eq!(groups.len(), 2);
        assert!(
            groups
                .iter()
                .all(|group| matches!(group.lead.provenance, RowProvenance::Direct(_)))
        );
        assert_current_process(
            &groups
                .iter()
                .find(|group| group.lead.pid == 101)
                .expect("v3 cargo child discovered independently of registration PID")
                .lead,
        );
        let panes: Vec<_> = groups
            .iter()
            .map(|group| (group.lead.pid, TileContent::Group(group.id())))
            .collect();
        app.roster.observe(groups, Instant::now());
        app.root_status.clone_from(&capture.root_status);
        let rendered = draw_summary(&app, 8);
        assert_capture_rows(&rendered, &rows);
        let older = rendered
            .lines()
            .find(|line| line.contains("older-command"))
            .expect("v2 row");
        assert_eq!(
            older.matches(UNAVAILABLE_MEASUREMENT).count(),
            3,
            "{rendered}"
        );
        for (pid, content) in panes {
            let rendered = draw_content(&app, &content, 8);
            let entry = rows
                .iter()
                .find(|(_, expected, _)| *expected == pid)
                .expect("command entry");
            assert_capture_rows(&rendered, std::slice::from_ref(entry));
            if pid == 200 {
                let row = rendered
                    .lines()
                    .find(|line| line.contains("older-command"))
                    .expect("v2 command row");
                assert_eq!(
                    row.matches(UNAVAILABLE_MEASUREMENT).count(),
                    3,
                    "{rendered}"
                );
            }
        }
        let keymap = Rc::clone(&app.keymap);
        keymap.dispatch_framework_global(GlobalAction::OpenSettings, &mut app);
        let mut terminal =
            Terminal::new(TestBackend::new(240, 60)).expect("mixed settings terminal");
        let settings = draw(&mut terminal, &mut app).to_lowercase();
        assert!(!settings.contains("invalid registration"), "{settings}");
        assert!(!settings.contains("unsupported"), "{settings}");
        keymap.dispatch_framework_global(GlobalAction::Dismiss, &mut app);
        assert_publications(root.path(), &entries, &publications);
    }
    assert_mixed_command_pane(parent, current, &capture, &rows);
    assert_publications(root.path(), &entries, &publications);
}

fn assert_current_process(row: &CargoProcess) {
    assert_eq!(row.pid, 101);
    assert_eq!(row.managed, Measurement::Reading(0));
    assert_eq!(row.compiler, CompilerObservation::None);
    assert_eq!(
        row.cpu,
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
}

fn assert_mixed_command_pane(
    mut parent: ProcessObservation<'_>,
    current: ProcessObservation<'_>,
    capture: &Capture,
    rows: &[(&str, u32, &str)],
) {
    let driver_argv = ["cargo", "nextest", "run"].map(OsString::from);
    let carrier_argv = ["python3", "writer.py"].map(OsString::from);
    let driver = ProcessObservation::cargo(300, &driver_argv);
    parent.parent = ProcessField::Observed(Pid::from_u32(300));
    // The v2 carrier has ancestry but no cargo argv, so its registration supplies its row.
    let mut carrier = ProcessObservation::cargo(200, &carrier_argv);
    carrier.parent = ProcessField::Observed(Pid::from_u32(300));
    carrier.uid = parent.uid;
    let records = ProcessObservations::new([driver, parent, current, carrier]);
    let mut sequence = CensusSequence::default();
    let mut app = App::new_for_test().expect("mixed command app");
    for _ in 0..2 {
        let groups = sequence.sample_capture(&records, capture);
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group.lead.pid, 300);
        assert_eq!(group.rest.len(), 2);
        assert_current_process(
            group
                .rest
                .iter()
                .find(|row| row.pid == 101)
                .expect("v3 cargo child in shared family"),
        );
        for row in &group.rest {
            assert_eq!(
                row.parent,
                VisibleParent::Invocation {
                    id:  group.id(),
                    pid: 300,
                }
            );
            assert!(matches!(row.provenance, RowProvenance::Direct(_)));
        }
        let content = TileContent::Group(group.id());
        app.roster.observe(groups, Instant::now());
        let rendered = draw_content(&app, &content, 12);
        assert_capture_rows(&rendered, rows);
        let older = rendered
            .lines()
            .find(|line| line.contains("older-command"))
            .expect("v2 row in shared command pane");
        assert_eq!(
            older.matches(UNAVAILABLE_MEASUREMENT).count(),
            3,
            "{rendered}"
        );
        let current = rendered
            .lines()
            .find(|line| line.contains("current-command"))
            .expect("v3 process row in shared command pane");
        assert_eq!(
            current.matches(UNAVAILABLE_MEASUREMENT).count(),
            1,
            "{rendered}"
        );
    }
}

#[test]
fn summary_registration_row_keeps_measurements_above_the_footer() {
    let entries = [
        ("cargo-tile-v2", 200, "summary-fallback"),
        ("cargo-tile-v2", 300, "summary-sentinel"),
    ];
    let (root, capture, publications) = published_capture(&entries);
    let mut sequence = CensusSequence::default();
    let groups = sequence.sample_capture(&ProcessObservations::default(), &capture);
    assert_eq!(groups.len(), 2);
    let mut app = App::new_for_test().expect("summary app");
    app.roster.observe(groups, Instant::now());
    for height in [5, 6, 7] {
        let rendered = draw_summary(&app, height);
        assert_capture_rows(&rendered, &entries);
        let lines: Vec<_> = rendered.lines().collect();
        let fallback = lines
            .iter()
            .position(|line| line.contains("summary-fallback"))
            .expect("fallback row");
        let sentinel = lines
            .iter()
            .position(|line| line.contains("summary-sentinel"))
            .expect("following row");
        assert!(
            fallback < sentinel && sentinel < lines.len() - 1,
            "{rendered}"
        );
        assert_eq!(
            lines[fallback].matches(UNAVAILABLE_MEASUREMENT).count(),
            3,
            "{rendered}"
        );
        assert!(
            !lines[fallback].contains(TILE_ROWS_CONTENT_LABEL),
            "{rendered}"
        );
        assert!(
            lines
                .last()
                .expect("footer")
                .contains(TILE_ROWS_CONTENT_LABEL),
            "{rendered}"
        );
    }
    assert_publications(root.path(), &entries, &publications);
}

fn published_capture(entries: &[(&str, u32, &str)]) -> (TempDir, Capture, Vec<Vec<u8>>) {
    let root = tempfile::tempdir().expect("capture root");
    fs::create_dir_all(root.path().join("state/pids")).expect("registration directory");
    let publications = entries
        .iter()
        .map(|(version, pid, marker)| {
            let log = format!("run-generation-{pid}.log");
            let bytes = [
                *version,
                "generation",
                "boot",
                "100",
                &log,
                "/writer/project",
                "/writer",
                "2",
                "build",
                *marker,
                "",
            ]
            .join("\0")
            .into_bytes();
            fs::write(
                root.path().join(format!("state/pids/{pid}.generation")),
                &bytes,
            )
            .expect("registration");
            fs::write(root.path().join(log), format!("{LOCK_WAIT_MARKER}\n"))
                .expect("blocked output");
            bytes
        })
        .collect::<Vec<_>>();
    let stamp = match BirthStamp::from_fields("boot", "100") {
        IdentityEvidence::Available(stamp) => Ok(stamp),
        IdentityEvidence::Unavailable => Err("fixed valid birth stamp"),
    }
    .expect("fixture birth stamp");
    let mut capture = Capture::take_from(root.path(), |pid| {
        KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
    });
    assert_eq!(capture.confirmed().len(), entries.len());
    for status in &mut capture.root_status {
        assert!(status.diagnostics.is_empty(), "{:?}", status.diagnostics);
        status.account = AccountName::Resolved("reader-fixture".into());
    }
    (root, capture, publications)
}

fn assert_publications(root: &Path, entries: &[(&str, u32, &str)], publications: &[Vec<u8>]) {
    for ((_, pid, _), bytes) in entries.iter().zip(publications) {
        assert_eq!(
            fs::read(root.join(format!("state/pids/{pid}.generation")))
                .expect("retained registration"),
            *bytes
        );
        assert_eq!(
            fs::read_to_string(root.join(format!("run-generation-{pid}.log")))
                .expect("retained capture log"),
            format!("{LOCK_WAIT_MARKER}\n")
        );
    }
}

fn draw_summary(app: &App, height: u16) -> String {
    draw_content(app, &TileContent::Summary, height)
}

fn draw_content(app: &App, content: &TileContent, height: u16) -> String {
    let inner = Rect::new(0, 0, 298, height);
    let mut buffer = Buffer::empty(inner);
    render::draw_cell_for_test(&mut buffer, &app.roster, content, inner, 4);
    (0..height)
        .map(|y| {
            (0..inner.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_capture_rows(rendered: &str, entries: &[(&str, u32, &str)]) {
    assert_eq!(
        rendered.matches("[reader-fixture] /writer/project").count(),
        1,
        "{rendered}"
    );
    for (_, pid, marker) in entries {
        let rows: Vec<_> = rendered
            .lines()
            .filter(|line| line.contains(marker))
            .collect();
        assert_eq!(rows.len(), 1, "{rendered}");
        assert_eq!(
            rows[0].split_whitespace().next(),
            Some(pid.to_string().as_str()),
            "{rendered}"
        );
        assert!(
            rows[0].contains(&format!("cargo build {marker}")),
            "{rendered}"
        );
        assert!(rows[0].contains("blocked"), "{rendered}");
    }
}
