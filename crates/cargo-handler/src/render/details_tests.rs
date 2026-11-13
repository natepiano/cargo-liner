//! Row columns and details use the same source values.

use census::RoleFallback;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::layout::Position;
use tui_pane::ClipboardBackend;
use tui_pane::ClipboardError;
use tui_pane::CopySelectionResult;
use tui_pane::ScrollDirection;
use tui_pane::TerminalApp;
use tui_pane::TileView;

use super::word_click_tests::Renderer;
use super::word_click_tests::TestResult;
use super::word_click_tests::{self};
use super::*;
use crate::census::Activity;
use crate::census::AgentActivity;
use crate::census::CensusUpdate;
use crate::census::ChildRow;
use crate::census::HierarchyRole;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::accounts::Reading;
use crate::details::DetailsOverlay;

fn application(now: u64, age: u64) -> TestResult<App> {
    let mut app = word_click_tests::application()?;
    app.census.apply(CensusUpdate::Local {
        rows: serde_json::from_value(serde_json::json!([
            {"agent":"claude","pid":10,"session_id":"director","name":"director-name",
             "started":now-age,"directory":"/fixture","account":{"home":"~/.claude1","label":"claude1"},
             "activity":{"active":"waiting"},"children":[
                {"depth":0,"kind":"shell","pid":11,"name":"timer-name","started":now-60,
                 "timer":{"started":now-60,"deadline":now+588}}
             ]},
            {"agent":"claude","pid":20,"session_id":"unit","name":"unit-name",
             "started":now-120,"directory":"/fixture","children":[]},
            {"agent":"claude","pid":30,"session_id":"worker","name":"worker-name",
             "started":now-60,"directory":"/fixture","children":[]}
        ]))?,
        hierarchy: serde_json::from_value::<HierarchyEdges>(serde_json::json!([
            {"manager":"director","member":"unit","role":"unit"},
            {"manager":"director","member":"worker","role":"impl"}
        ]))?.into(),
        stage: ScanStage::Completed,
    });
    app.accounts.table = AccountsTable::Ready(AccountsReading::parse(br#"{
        "accounts":[{"label":"claude1","roles":["showrunners"],"weekly_left_percent":50,
        "use_credits":false,"role_counts":[{"busy":1,"idle":1,"paused":1,"exited":1,"total":4}],
        "sessions":[{"session":"director","state":"busy"},{"session":"unit","state":"paused"},
        {"session":"worker","state":"idle"}]},
        {"label":"claude2","roles":[],"use_credits":false,"sessions":[{"session":"second-account","state":"exited"}]}],"problems":[]
    }"#).map_err(|error| error.to_string())?);
    Ok(app)
}

#[test]
fn details_sections_align_values_and_keep_effort() -> TestResult {
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    let machine = app.census.machines(&[])[0];
    let mut rows = machine.state.rows().to_vec();
    rows.push(serde_json::from_value(serde_json::json!({
        "agent":"claude", "pid":40, "name":"unassigned-name", "session_id":"unassigned",
        "started":now-60, "directory":"/fixture", "children":[]
    }))?);
    rows.push(serde_json::from_value(serde_json::json!({
        "agent":"claude", "pid":50, "name":"review-name", "session_id":"review",
        "role_fallback":"review", "started":now-60, "directory":"/fixture", "children":[]
    }))?);
    rows[2].name = "worker-with-a-very-long-session-name-that-stays-complete".into();
    let mut edges = HierarchyEdges(machine.state.edges().to_vec());
    edges.0[1].manager = "unit".into();
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: edges.into(),
        stage: ScanStage::Completed,
    });
    let mut renderer = Renderer::new(&mut app, 160, 70)?;
    for (name, hierarchy) in [
        (
            "worker-with-a-very-long-session-name-that-stays-complete",
            vec![
                ("showrunner", "director-name"),
                ("unit", "unit-name"),
                (
                    "worker",
                    "worker-with-a-very-long-session-name-that-stays-complete",
                ),
                ("role", "worker"),
            ],
        ),
        (
            "director-name",
            vec![("showrunner", "director-name"), ("role", "showrunner")],
        ),
        ("unassigned-name", vec![("name", "unassigned-name")]),
        (
            "review-name",
            vec![("name", "review-name"), ("role", "worker")],
        ),
        (
            "fix · repair the seat",
            vec![("worker", "fix · repair the seat"), ("role", "worker")],
        ),
    ] {
        if name == "fix · repair the seat" {
            let machine = app.census.machines(&[])[0];
            let mut rows = machine.state.rows().to_vec();
            rows[2].name = name.into();
            rows[2].role_fallback = RoleFallback::Worker(HierarchyRole::Fix);
            rows[2].launched_by = Some(10);
            let mut edges = HierarchyEdges(machine.state.edges().to_vec());
            edges.0.retain(|edge| edge.member != "worker");
            app.census.apply(CensusUpdate::Local {
                rows,
                hierarchy: edges.into(),
                stage: ScanStage::Completed,
            });
        }
        tui_pane::dispatch_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE),
        );
        let buffer = renderer.paint(&mut app)?;
        app.click(
            word_click_tests::positions(&buffer, name)
                .into_iter()
                .last()
                .ok_or_else(|| {
                    format!("missing row {name}: {}", drawn_details(&buffer).join("\n"))
                })?,
        );
        if name.starts_with("worker-") {
            let mut narrow = Renderer::new(&mut app, 60, 70)?;
            let cut = narrow.paint(&mut app)?;
            let text = drawn_details(&cut).join("\n");
            assert!(text.contains("worker-w…"), "{text}");
            assert!(!text.contains(name));
        }
        tui_pane::dispatch_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        );
        let buffer = renderer.paint(&mut app)?;
        assert_details_sections(&buffer, &hierarchy)?;
        tui_pane::dispatch_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
    Ok(())
}

fn assert_details_sections(buffer: &Buffer, hierarchy: &[(&str, &str)]) -> TestResult {
    let text = drawn_details(buffer);
    let mut headings = Vec::new();
    for section in ["agent", "hierarchy", "location", "now"] {
        let point = word_click_tests::positions(buffer, section)
            .into_iter()
            .find(|point| {
                buffer[(point.x, point.y)].fg == tui_pane::title_color()
                    && buffer[(point.x.saturating_sub(1), point.y)].symbol() == " "
                    && buffer[(point.x.saturating_sub(2), point.y)].symbol() == "│"
            })
            .ok_or_else(|| format!("missing section {section}: {text:?}"))?;
        headings.push(point);
    }
    assert!(headings.windows(2).all(|pair| pair[0].y < pair[1].y));
    let left = headings[0].x;
    assert!(headings.iter().all(|heading| heading.x == left));
    let right = (left..buffer.area.right())
        .find(|&x| buffer[(x, headings[0].y)].symbol() == "│")
        .ok_or("right border")?;
    let mut value_ends = Vec::new();
    let mut values = Vec::new();
    let mut drawn_hierarchy = Vec::new();
    for y in buffer.area.y..buffer.area.bottom() {
        let content: String = (left..right).map(|x| buffer[(x, y)].symbol()).collect();
        let Some((label, rest)) = content.split_once(':') else {
            continue;
        };
        let label = label.trim();
        if ![
            "account",
            "family",
            "model",
            "effort",
            "type",
            "showrunner",
            "unit",
            "worker",
            "name",
            "role",
            "directory",
            "branch",
            "desktop",
            "pid",
            "activity",
            "status",
            "state",
            "age",
        ]
        .contains(&label)
        {
            continue;
        }
        let label_x = usize::from(left) + content.find(label).ok_or("label position")?;
        assert_eq!(u16::try_from(label_x)?, left);
        assert_eq!(buffer[(left - 1, y)].symbol(), " ");
        let value = rest.trim().trim_end_matches('│').trim_end();
        let x = usize::from(left) + content.find(':').ok_or("colon")? + 1 + rest.len()
            - rest.trim_start().len();
        values.push(x);
        value_ends.push(x + unicode_width::UnicodeWidthStr::width(value));
        if y > headings[1].y && y < headings[2].y {
            drawn_hierarchy.push((label.to_owned(), value.to_owned()));
        }
    }
    assert!(values.windows(2).all(|pair| pair[0] == pair[1]), "{text:?}");
    assert_eq!(
        value_ends.into_iter().max().ok_or("value end")?,
        usize::from(right - 1)
    );
    assert_eq!(
        drawn_hierarchy,
        hierarchy
            .iter()
            .map(|(label, value)| (label.to_string(), value.to_string()))
            .collect::<Vec<_>>()
    );
    let effort = word_click_tests::locate(buffer, "effort:")?;
    assert!(text[usize::from(effort.y)].contains('—'));
    word_click_tests::locate(buffer, "family:")?;
    assert!(!text.iter().any(|line| line.contains("program:")));
    Ok(())
}

fn field(buffer: &Buffer, name: &str, column: &str) -> TestResult<String> {
    let row = word_click_tests::positions(buffer, name)
        .into_iter()
        .last()
        .ok_or_else(|| format!("missing {name}"))?;
    let header = word_click_tests::positions(buffer, column)
        .into_iter()
        .filter(|point| point.y < row.y && buffer[(point.x, point.y)].fg == tui_pane::label_color())
        .max_by_key(|point| point.y)
        .ok_or_else(|| format!("missing {column} header above {name}"))?;
    let end = (header.x..buffer.area.right())
        .find(|&x| {
            x > header.x
                && buffer[(x, header.y)].symbol() != " "
                && buffer[(x - 1, header.y)].symbol() == " "
                && !matches!(buffer[(x, header.y)].symbol(), "▲" | "▼")
        })
        .unwrap_or_else(|| buffer.area.right());
    Ok((header.x..end)
        .map(|x| buffer[(x, row.y)].symbol())
        .collect::<String>()
        .trim()
        .to_owned())
}

#[test]
fn row_columns_put_session_first_and_separate_role_activity_and_timer() -> TestResult {
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    let mut renderer = Renderer::with_fixed_time(&mut app, 200, 70, now)?;
    let buffer = renderer.paint(&mut app)?;
    let unit = word_click_tests::locate(&buffer, "unit-name")?;
    let header = (0..unit.y)
        .rev()
        .find(|&y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains("activity")
        })
        .ok_or("missing activity header")?;
    let text = (0..buffer.area.width)
        .map(|x| buffer[(x, header)].symbol())
        .collect::<String>();
    let columns: Vec<_> = text
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .collect();
    assert_eq!(
        &columns[..5],
        ["session", "account", "role", "activity", "status"]
    );
    assert!(!columns.contains(&"age"));
    for (name, role, activity, status) in [
        ("worker-name", "worker", "impl", "idle"),
        ("unit-name", "unit", "", "paused"),
        ("timer-name", "", "timer 9:48", "busy"),
    ] {
        assert_eq!(field(&buffer, name, "role")?, role, "{name}");
        assert_eq!(field(&buffer, name, "activity")?, activity, "{name}");
        assert_eq!(field(&buffer, name, "status")?, status, "{name}");
    }
    Ok(())
}

#[test]
fn row_status_uses_the_accounts_source_and_leaves_missing_states_empty() -> TestResult {
    for (state, conflict) in [
        ("busy", false),
        ("idle", false),
        ("paused", false),
        ("exited", false),
        ("busy", true),
    ] {
        let now = census::unix_now();
        let mut app = application(now, 86400)?;
        app.accounts.table = AccountsTable::Ready(AccountsReading::parse(&serde_json::to_vec(
            &serde_json::json!({"accounts":[{"label":"claude1","roles":["workers"],
                "weekly_left_percent":50,"use_credits":false,"role_counts":[{
                    "busy":u64::from(state=="busy"),"idle":u64::from(state=="idle"),
                    "paused":u64::from(state=="paused"),"exited":u64::from(state=="exited"),"total":1
                }],"sessions":[{"session":"worker","state":state}]},
                {"label":"claude2","roles":[],"use_credits":false,"sessions":if conflict {vec![
                    serde_json::json!({"session":"director","state":"busy"}),
                    serde_json::json!({"session":"worker","state":"idle"}),
                    serde_json::json!({"session":"worker","state":"busy"})
                ]} else {vec![serde_json::json!({"session":"director","state":"busy"})]} }],"problems":[]})
        )?).map_err(|error| error.to_string())?);
        let mut renderer = Renderer::new(&mut app, 200, 70)?;
        let buffer = renderer.paint(&mut app)?;
        let actual = field(&buffer, "worker-name", "status")?;
        assert_eq!(actual, if conflict { "" } else { state });
        assert_eq!(field(&buffer, "unit-name", "status")?, "");
        assert_eq!(field(&buffer, "director-name", "status")?, "busy");
        let AccountsTable::Ready(reading) = &app.accounts.table else {
            return Err("missing reading".into());
        };
        let Reading::Known(counts) = &reading.accounts[0].roles[0].counts else {
            return Err("missing counts".into());
        };
        let count = match state {
            "busy" => counts.busy,
            "idle" => counts.idle,
            "paused" => counts.paused,
            "exited" => counts.exited,
            _ => return Err("unexpected status".into()),
        };
        assert_eq!(count, counts.total);
    }
    assert_unrecorded_statuses()?;
    Ok(())
}

fn assert_unrecorded_statuses() -> TestResult {
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    let mut rows = app.census.machines(&[])[0].state.rows().to_vec();
    let hierarchy = HierarchyEdges(app.census.machines(&[])[0].state.edges().to_vec());
    rows[0].children.extend(serde_json::from_value::<Vec<ChildRow>>(serde_json::json!([
        {"depth":0,"kind":"subagent","pid":null,"name":"subagent-name","started":now-60,"activity":{"active":"thinking","compacting":true}},
        {"depth":0,"kind":"thread","pid":101,"name":"live-thread","started":now-60},
        {"depth":0,"kind":"thread","pid":null,"name":"gone-thread","started":now-60}
    ]))?);
    rows.extend(serde_json::from_value::<Vec<AgentRow>>(serde_json::json!([
        {"agent":"codex","pid":110,"name":"unrecorded-pane","started":now-60,"directory":"/fixture","children":[]},
        {"agent":"codex","pid":120,"name":"proven-pane","codex_session":{"resume_argument":"missing-status"},"started":now-60,"directory":"/fixture","children":[]}
    ]))?);
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: hierarchy.into(),
        stage: ScanStage::Completed,
    });
    let mut renderer = Renderer::new(&mut app, 150, 55)?;
    let buffer = renderer.paint(&mut app)?;
    for (name, expected) in [
        ("subagent-name", "busy"),
        ("live-thread", "idle"),
        ("gone-thread", "exited"),
        ("unrecorded-pane", "idle"),
        ("proven-pane", ""),
    ] {
        assert_eq!(field(&buffer, name, "status")?, expected, "{name}");
    }
    assert_eq!(field(&buffer, "subagent-name", "activity")?, "compacting");
    Ok(())
}

#[test]
fn top_level_agents_hide_age_and_keep_oldest_first() -> TestResult {
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    app.loaded_config.config.tiles.view = TileView::Summary;
    word_click_tests::apply_rows(
        &mut app,
        serde_json::from_value(serde_json::json!([
            {"agent":"claude","pid":40,"name":"young-name","started":now-60,"directory":"/fixture",
             "children":[{"kind":"shell","pid":41,"depth":0,"name":"young-work","started":now-30}]},
            {"agent":"claude","pid":50,"name":"old-name","started":now-86400,"directory":"/fixture",
             "children":[{"kind":"shell","pid":51,"depth":0,"name":"old-work","started":now-30}]}
        ]))?,
    );
    let mut renderer = Renderer::new(&mut app, 120, 35)?;
    let buffer = renderer.paint(&mut app)?;
    let old = word_click_tests::locate(&buffer, "old-name")?;
    let young = word_click_tests::locate(&buffer, "young-name")?;
    assert!(old.y < young.y);
    for y in 0..old.y {
        let text = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>();
        assert!(!text.split_whitespace().any(|word| word == "age"));
    }
    Ok(())
}

#[test]
fn row_details_key_shows_properties_and_long_age_then_escape_closes() -> TestResult {
    for view in [TileView::Cells, TileView::Summary] {
        let now = census::unix_now();
        let mut app = application(now, 86400)?;
        let machine = app.census.machines(&[])[0];
        let mut rows = machine.state.rows().to_vec();
        let hierarchy = HierarchyEdges(machine.state.edges().to_vec());
        rows[0].directory = "/fixture/two  spaces".into();
        rows[0].branch = Some("feature/two  spaces".into());
        rows[0].desktop = Some("two  spaces".into());
        app.loaded_config.config.tiles.view = view;
        app.census.apply(CensusUpdate::Local {
            rows:      rows.clone(),
            hierarchy: hierarchy.clone().into(),
            stage:     ScanStage::Completed,
        });
        let mut renderer = Renderer::new(&mut app, 120, 35)?;
        let buffer = renderer.paint(&mut app)?;
        app.click(
            word_click_tests::positions(&buffer, "director-name")
                .into_iter()
                .last()
                .ok_or("missing selected row")?,
        );
        tui_pane::dispatch_key(&mut app, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        for (age, expected) in [
            (
                32 * 86400 + 14 * 3600 + 44 * 60,
                "32 days, 14 hours, 44 minutes",
            ),
            (14 * 3600 + 44 * 60, "14 hours, 44 minutes"),
            (44 * 60, "44 minutes"),
            (30, "0 minutes"),
        ] {
            rows[0].started = now - age;
            app.census.apply(CensusUpdate::Local {
                rows:      rows.clone(),
                hierarchy: hierarchy.clone().into(),
                stage:     ScanStage::Completed,
            });
            assert_selected_details(&mut app, &mut renderer, expected)?;
        }
    }
    assert_child_details_use_own_values()?;
    assert_wrapped_details_navigation()?;
    Ok(())
}

fn assert_selected_details(app: &mut App, renderer: &mut Renderer, expected: &str) -> TestResult {
    let buffer = renderer.paint(app)?;
    assert_eq!(
        word_click_tests::positions(&buffer, "waiting"),
        [] as [ratatui::prelude::Position; 0]
    );
    assert_eq!(field(&buffer, "director-name", "status")?, "busy");
    tui_pane::dispatch_key(app, KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    let DetailsOverlay::Open { details, .. } = &app.details_overlay else {
        return Err("closed details".into());
    };
    let properties: Vec<_> = details
        .properties
        .iter()
        .map(|(property, _)| property.label())
        .collect();
    assert_eq!(
        properties,
        [
            "account",
            "family",
            "effort",
            "showrunner",
            "role",
            "directory",
            "branch",
            "desktop",
            "pid",
            "status",
            "state",
            "age"
        ]
    );
    let details = renderer.paint(app)?;
    let drawn = drawn_details(&details);
    for pair in properties.windows(2) {
        let before = drawn
            .iter()
            .position(|line| line.contains(&format!("{}:", pair[0])))
            .ok_or("missing first drawn property")?;
        let after = drawn
            .iter()
            .position(|line| line.contains(&format!("{}:", pair[1])))
            .ok_or("missing second drawn property")?;
        assert!(before < after, "{} precedes {}", pair[0], pair[1]);
    }
    for property in [
        "pid: 10",
        "family: claude",
        "effort: —",
        "showrunner: director-name",
        "account: claude1",
        "directory: /fixture/two  spaces",
        "branch: feature/two  spaces",
        "desktop: two  spaces",
        "state: waiting",
        &format!("age: {expected}"),
    ] {
        let (label, value) = property.split_once(": ").ok_or("property pair")?;
        assert_eq!(drawn_property(&drawn, label)?, value, "{label}");
    }
    tui_pane::dispatch_key(app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let closed = renderer.paint(app)?;
    assert_eq!(
        word_click_tests::positions(&closed, "age:"),
        [] as [ratatui::prelude::Position; 0]
    );
    Ok(())
}

fn drawn_property(lines: &[String], label: &str) -> TestResult<String> {
    let marker = format!("{label}:");
    let rest = lines
        .iter()
        .find_map(|line| line.split_once(&marker).map(|(_, value)| value))
        .ok_or_else(|| format!("missing {marker}"))?;
    Ok(rest.split('│').next().ok_or("value")?.trim().to_owned())
}

fn drawn_details(buffer: &Buffer) -> Vec<String> {
    buffer
        .content
        .chunks_exact(usize::from(buffer.area.width))
        .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect())
        .collect()
}

fn assert_child_details_use_own_values() -> TestResult {
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    let machine = app.census.machines(&[])[0];
    let mut rows = machine.state.rows().to_vec();
    let mut hierarchy = HierarchyEdges(machine.state.edges().to_vec());
    rows[0].branch = Some("parent-branch".into());
    rows[0].desktop = Some("parent-desktop".into());
    rows[0]
        .children
        .push(serde_json::from_value(serde_json::json!({
            "kind":"thread", "thread_id":"child-thread", "pid":null,
            "name":"nested-thread", "depth":0, "started":now-60
        }))?);
    hierarchy.0.push(serde_json::from_value(serde_json::json!({
        "manager":"director", "member":"child-thread", "role":"impl"
    }))?);
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: hierarchy.into(),
        stage: ScanStage::Completed,
    });
    let mut renderer = Renderer::new(&mut app, 120, 35)?;
    for (name, program) in [("timer-name", "claude"), ("nested-thread", "codex")] {
        let buffer = renderer.paint(&mut app)?;
        app.click(word_click_tests::locate(&buffer, name)?);
        tui_pane::dispatch_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        );
        let buffer = renderer.paint(&mut app)?;
        let drawn = drawn_details(&buffer);
        assert_eq!(drawn_property(&drawn, "family")?, program);
        if name == "timer-name" {
            assert_eq!(drawn_property(&drawn, "pid")?, "11");
        }
        for label in ["directory:", "branch:", "desktop:"] {
            assert!(
                !drawn.iter().any(|line| line.contains(label)),
                "{name}: {label}"
            );
        }
        tui_pane::dispatch_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
    Ok(())
}

#[test]
fn compacting_sessions_replace_each_role_job_with_compacting() -> TestResult {
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    let machine = app.census.machines(&[])[0];
    let mut rows = machine.state.rows().to_vec();
    let hierarchy = HierarchyEdges(machine.state.edges().to_vec());
    rows.extend(serde_json::from_value::<Vec<AgentRow>>(
        serde_json::json!([
            {"agent":"claude","pid":60,"session_id":"producer","name":"producer-name",
             "producer":true,"started":now-60,"directory":"/fixture","children":[]},
            {"agent":"claude","pid":70,"session_id":"unlabeled","name":"unlabeled-name",
             "started":now-60,"directory":"/fixture","children":[]}
        ]),
    )?);
    for row in &mut rows {
        row.activity = AgentActivity::Active(Activity::Compacting);
    }
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: hierarchy.into(),
        stage: ScanStage::Completed,
    });
    let mut renderer = Renderer::new(&mut app, 200, 70)?;
    let buffer = renderer.paint(&mut app)?;
    for (name, role) in [
        ("director-name", "showrunner"),
        ("unit-name", "unit"),
        ("worker-name", "worker"),
        ("producer-name", "producer"),
        ("unlabeled-name", ""),
    ] {
        assert_eq!(field(&buffer, name, "activity")?, "compacting", "{name}");
        assert_eq!(field(&buffer, name, "role")?, role, "{name}");
    }
    Ok(())
}

/// Record the clipboard boundary without writing to the test terminal.
#[derive(Default)]
struct CopiedValues(Vec<String>);

impl ClipboardBackend for CopiedValues {
    fn write_clipboard(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.0.push(text.to_owned());
        Ok(())
    }
}

#[test]
fn details_navigation_skips_headings_and_y_copies_only_the_value() -> TestResult {
    use tui_pane::Shortcuts;

    use crate::details::DetailsPane;
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    let mut renderer = Renderer::new(&mut app, 160, 60)?;
    let buffer = renderer.paint(&mut app)?;
    app.click(
        word_click_tests::positions(&buffer, "director-name")
            .into_iter()
            .last()
            .ok_or("director")?,
    );
    tui_pane::dispatch_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    let actions = DetailsPane::defaults().into_scope_map();
    let yank = actions
        .action_for(&tui_pane::KeyBind::from('y'))
        .ok_or("y binding")?;
    let mut clipboard = CopiedValues::default();
    for (key, value, label) in [
        (KeyCode::Down, "claude", "family:"),
        (KeyCode::Down, "—", "effort:"),
        (KeyCode::Down, "director-name", "showrunner:"),
        (KeyCode::Up, "—", "effort:"),
    ] {
        tui_pane::dispatch_key(&mut app, KeyEvent::new(key, KeyModifiers::NONE));
        let buffer = renderer.paint(&mut app)?;
        let point = word_click_tests::locate(&buffer, label)?;
        assert_eq!(
            buffer[(point.x, point.y)].bg,
            tui_pane::active_focus_color()
        );
        yank.apply(&mut app, &mut clipboard);
        assert_eq!(clipboard.0.last().ok_or("copied value")?, value);
    }
    let buffer = renderer.paint(&mut app)?;
    let text = drawn_details(&buffer).join("\n");
    assert!(text.contains("Copied value"), "{text}");
    assert_full_frame_copy_notice()?;
    Ok(())
}

fn assert_wrapped_details_navigation() -> TestResult {
    use tui_pane::CopySelection;

    use crate::details::DetailsPane;
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    let machine = app.census.machines(&[])[0];
    let mut rows = machine.state.rows().to_vec();
    let directory = format!("/first-{}-last", "x".repeat(288));
    rows[0].directory = directory.clone();
    let hierarchy = HierarchyEdges(machine.state.edges().to_vec());
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: hierarchy.into(),
        stage: ScanStage::Completed,
    });
    let mut renderer = Renderer::new(&mut app, 160, 60)?;
    let buffer = renderer.paint(&mut app)?;
    app.click(
        word_click_tests::positions(&buffer, "director-name")
            .into_iter()
            .last()
            .ok_or("director")?,
    );
    tui_pane::dispatch_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    let mut renderer = Renderer::new(&mut app, 40, 12)?;
    for _ in 0..5 {
        tui_pane::dispatch_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        renderer.paint(&mut app)?;
    }
    let buffer = renderer.paint(&mut app)?;
    assert!(drawn_details(&buffer).join("\n").contains("/first-"));
    for wheel in [false, true] {
        let last = (0..30)
            .find_map(|_| {
                if wheel {
                    app.scroll(Position::new(1, 1), ScrollDirection::Down);
                } else {
                    tui_pane::dispatch_key(
                        &mut app,
                        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                    );
                }
                match renderer.paint(&mut app) {
                    Ok(buffer) => drawn_details(&buffer)
                        .join("\n")
                        .contains("-last")
                        .then_some(Ok(buffer)),
                    Err(error) => Some(Err(error)),
                }
            })
            .transpose()?;
        assert!(last.is_some(), "the end of the directory must be reachable");
        let CopySelectionResult::Payload(payload) = DetailsPane::copy_selection(&app) else {
            return Err("directory copy target".into());
        };
        assert_eq!(payload.text, directory);
        let first = (0..30)
            .find_map(|_| {
                tui_pane::dispatch_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
                match renderer.paint(&mut app) {
                    Ok(buffer) => drawn_details(&buffer)
                        .join("\n")
                        .contains("/first-")
                        .then_some(Ok(buffer)),
                    Err(error) => Some(Err(error)),
                }
            })
            .transpose()?;
        assert!(
            first.is_some(),
            "the start of the directory must be reachable"
        );
    }
    Ok(())
}

/// Exercise the failed clipboard boundary without writing to the terminal.
struct FailedClipboard;

impl ClipboardBackend for FailedClipboard {
    fn write_clipboard(&mut self, _: &str) -> Result<(), ClipboardError> {
        Err(ClipboardError::WriteFailed("clipboard write error".into()))
    }
}

fn assert_full_frame_copy_notice() -> TestResult {
    use crate::details::DetailsAction;
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    let machine = app.census.machines(&[])[0];
    let mut rows = machine.state.rows().to_vec();
    let directory = format!("/{}", "x".repeat(299));
    rows[0].directory = directory.clone();
    let hierarchy = HierarchyEdges(machine.state.edges().to_vec());
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: hierarchy.into(),
        stage: ScanStage::Completed,
    });
    let mut renderer = Renderer::new(&mut app, 160, 60)?;
    let buffer = renderer.paint(&mut app)?;
    app.click(
        word_click_tests::positions(&buffer, "director-name")
            .into_iter()
            .last()
            .ok_or("director")?,
    );
    tui_pane::dispatch_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    let mut renderer = Renderer::new(&mut app, 40, 12)?;
    for _ in 0..5 {
        DetailsAction::Down.apply(&mut app, &mut CopiedValues::default());
        renderer.paint(&mut app)?;
    }
    let mut clipboard = CopiedValues::default();
    DetailsAction::Copy.apply(&mut app, &mut clipboard);
    assert_eq!(clipboard.0, [directory]);
    let buffer = renderer.paint(&mut app)?;
    let text = drawn_details(&buffer).join("\n");
    assert!(text.contains("Copied value"), "{text}");
    DetailsAction::Copy.apply(&mut app, &mut FailedClipboard);
    let buffer = renderer.paint(&mut app)?;
    let text = drawn_details(&buffer).join("\n");
    assert!(text.contains("Copy failed"), "{text}");
    assert!(text.contains("clipboard write error"), "{text}");
    Ok(())
}
