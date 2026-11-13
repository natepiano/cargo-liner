//! Row columns and details use the same source values.

use census::RoleFallback;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
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
use crate::census::RunLocation;
use crate::census::VerifyStep;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::accounts::Reading;
use crate::details::DetailsOverlay;
use crate::theme;

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
    rows[2].model = serde_json::from_value(serde_json::json!("claude-opus-4-6"))?;
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
        ("timer-name", vec![("name", "timer-name")]),
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
            details_test_positions(&buffer, name)
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
        let point = details_test_positions(buffer, section)
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
    let mut drawn_agent = Vec::new();
    let mut drawn_now = Vec::new();
    for y in buffer.area.y..buffer.area.bottom() {
        let content: String = (left..right).map(|x| buffer[(x, y)].symbol()).collect();
        let Some((label, rest)) = content.split_once(':') else {
            continue;
        };
        let label = label.trim();
        if ![
            "account",
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
        if !value.is_empty() {
            values.push(x);
            value_ends.push(x + unicode_width::UnicodeWidthStr::width(value));
        }
        if y > headings[0].y && y < headings[1].y {
            drawn_agent.push(label.to_owned());
        }
        if y > headings[3].y {
            drawn_now.push(label.to_owned());
        }
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
    assert_eq!(drawn_now, ["activity", "status", "age"]);
    assert_agent_order_and_hidden_labels(buffer, &text, &drawn_agent)
}

/// Agent properties keep their order, effort draws, and retired labels stay hidden.
fn assert_agent_order_and_hidden_labels(
    buffer: &Buffer,
    text: &[String],
    drawn_agent: &[String],
) -> TestResult {
    let agent_order = ["account", "model", "effort", "type"];
    let positions: Vec<_> = drawn_agent
        .iter()
        .map(|property| {
            agent_order
                .iter()
                .position(|expected| expected == property)
                .ok_or("agent property")
        })
        .collect::<Result<_, _>>()?;
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{drawn_agent:?}"
    );
    let effort = details_test_locate(buffer, "effort:")?;
    assert!(text[usize::from(effort.y)].contains('—'));
    assert!(
        !text
            .iter()
            .any(|line| line.contains("family:") || line.contains("state:"))
    );
    assert!(!text.iter().any(|line| line.contains("program:")));
    Ok(())
}

fn field(buffer: &Buffer, name: &str, column: &str) -> TestResult<String> {
    let row = details_test_positions(buffer, name)
        .into_iter()
        .last()
        .ok_or_else(|| format!("missing {name}"))?;
    let header = details_test_positions(buffer, column)
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
    let unit = details_test_locate(&buffer, "unit-name")?;
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
    let old = details_test_locate(&buffer, "old-name")?;
    let young = details_test_locate(&buffer, "young-name")?;
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
            details_test_positions(&buffer, "director-name")
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
    let director = details_test_positions(&buffer, "director-name")
        .into_iter()
        .last()
        .ok_or("missing director row")?;
    let row: String = (buffer.area.x..buffer.area.right())
        .map(|x| buffer[(x, director.y)].symbol())
        .collect();
    assert!(!row.contains("waiting"), "director row: {row}");
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
            "effort",
            "showrunner",
            "role",
            "directory",
            "branch",
            "desktop",
            "pid",
            "activity",
            "status",
            "age"
        ]
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 35))?;
    let details = paint_details(app, &mut terminal)?;
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
        "effort: —",
        "showrunner: director-name",
        "account: claude1",
        "directory: /fixture/two  spaces",
        "branch: feature/two  spaces",
        "desktop: two  spaces",
        "activity: waiting",
        &format!("age: {expected}"),
    ] {
        let (label, value) = property.split_once(": ").ok_or("property pair")?;
        assert_eq!(drawn_property(&drawn, label)?, value, "{label}");
    }
    tui_pane::dispatch_key(app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let closed = renderer.paint(app)?;
    assert_eq!(
        details_test_positions(&closed, "age:"),
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
    for name in ["timer-name", "nested-thread"] {
        let buffer = renderer.paint(&mut app)?;
        app.click(details_test_locate(&buffer, name)?);
        tui_pane::dispatch_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        );
        let buffer = renderer.paint(&mut app)?;
        let drawn = drawn_details(&buffer);
        assert!(
            !drawn
                .iter()
                .any(|line| line.contains("family:") || line.contains("state:"))
        );
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
        details_test_positions(&buffer, "director-name")
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
        (KeyCode::Down, "—", "effort:"),
        (KeyCode::Down, "director-name", "showrunner:"),
        (KeyCode::Up, "—", "effort:"),
    ] {
        tui_pane::dispatch_key(&mut app, KeyEvent::new(key, KeyModifiers::NONE));
        let buffer = renderer.paint(&mut app)?;
        let point = details_test_locate(&buffer, label)?;
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
        details_test_positions(&buffer, "director-name")
            .into_iter()
            .last()
            .ok_or("director")?,
    );
    tui_pane::dispatch_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12))?;
    for _ in 0..4 {
        tui_pane::dispatch_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        paint_details(&mut app, &mut terminal)?;
    }
    let buffer = paint_details(&mut app, &mut terminal)?;
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
                match paint_details(&mut app, &mut terminal) {
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
                match paint_details(&mut app, &mut terminal) {
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
        details_test_positions(&buffer, "director-name")
            .into_iter()
            .last()
            .ok_or("director")?,
    );
    tui_pane::dispatch_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    let mut renderer = Renderer::new(&mut app, 40, 12)?;
    for _ in 0..4 {
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

#[test]
fn details_activity_joins_job_and_live_work_without_repeated_words() -> TestResult {
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    let mut renderer = Renderer::new(&mut app, 160, 42)?;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 32))?;
    for (session, activity, expected, status) in [
        (
            "worker-name",
            serde_json::json!({"active":{"verify":{"step":"clippy","location":"local"}}}),
            "impl · clippy",
            "idle",
        ),
        (
            "unit-name",
            serde_json::json!({"active":"thinking"}),
            "thinking",
            "paused",
        ),
        ("unit-name", serde_json::json!("idle"), "—", "paused"),
        ("unit-name", serde_json::json!("unreported"), "—", "paused"),
        ("worker-name", serde_json::json!("idle"), "impl", "idle"),
        (
            "worker-name",
            serde_json::json!({"active":"compacting"}),
            "compacting",
            "idle",
        ),
    ] {
        let machine = app.census.machines(&[])[0];
        let mut rows = machine.state.rows().to_vec();
        let hierarchy = HierarchyEdges(machine.state.edges().to_vec());
        let row = rows
            .iter_mut()
            .find(|row| row.name == session)
            .ok_or("activity row")?;
        row.activity = serde_json::from_value(activity)?;
        app.census.apply(CensusUpdate::Local {
            rows,
            hierarchy: hierarchy.into(),
            stage: ScanStage::Completed,
        });
        let tile = renderer.paint(&mut app)?;
        app.click(details_test_locate(&tile, session)?);
        tui_pane::dispatch_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        );
        terminal.draw(|frame| app.details_overlay.render(frame, app.framework.color_mode))?;
        let drawn = drawn_details(terminal.backend().buffer());
        assert_eq!(drawn_property(&drawn, "activity")?, expected, "{session}");
        assert_eq!(drawn_property(&drawn, "status")?, status, "{session}");
        assert!(
            !drawn
                .iter()
                .any(|line| line.contains("family:") || line.contains("state:"))
        );
        app.details_overlay = DetailsOverlay::Closed;
    }
    Ok(())
}

#[test]
fn details_values_keep_default_dark_tile_inks() -> TestResult {
    check_details_tile_inks("Default Dark")
}

#[test]
fn details_values_keep_default_light_tile_inks() -> TestResult {
    check_details_tile_inks("Default Light")
}

fn check_details_tile_inks(theme_name: &str) -> TestResult {
    let previous = tui_pane::theme();
    let builtin = theme::builtins()
        .into_iter()
        .find(|builtin| builtin.id.as_str() == theme_name)
        .ok_or("theme")?;
    tui_pane::set_active_theme(std::sync::Arc::new(builtin.theme));
    let result = compare_details_tile_inks();
    tui_pane::set_active_theme(previous);
    result
}

fn ink_application() -> TestResult<App> {
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    let palette = tempfile::tempdir()?;
    let path = palette.path().join("statusline.conf");
    std::fs::write(
        &path,
        "account.claude1=brown\naccount.codex1=magenta\nrole.unit-director=bright-blue\n",
    )?;
    app.footer_colors = crate::footer_colors::FooterColorFile::fixture(path);
    app.footer_colors.refresh();
    let machine = app.census.machines(&[])[0];
    let mut rows = machine.state.rows().to_vec();
    let hierarchy = HierarchyEdges(machine.state.edges().to_vec());
    rows[1].account = rows[0].account.clone();
    rows[1].activity = AgentActivity::Active(Activity::Thinking);
    rows[2].activity = AgentActivity::Active(Activity::Verify {
        step:     VerifyStep::Clippy,
        location: RunLocation::Local,
    });
    rows[1].model = serde_json::from_value(serde_json::json!("claude-opus-4-6"))?;
    rows[1].effort = serde_json::from_value(serde_json::json!("high"))?;
    rows[1].branch = Some("ink-branch".into());
    rows[1].directory = "/ink-directory".into();
    rows[1].desktop = Some("ink-desktop".into());
    rows.push(serde_json::from_value(serde_json::json!({
        "agent":"codex", "pid":80, "name":"ink-session", "started":now-60,
        "account":{"home":"~/.codex1","label":"codex1"}, "service_tier":"fast",
        "directory":"/fixture", "children":[]
    }))?);
    rows.push(serde_json::from_value(serde_json::json!({
        "agent":"codex", "pid":90, "name":"ink-alternate", "started":now-60,
        "directory":"/fixture", "children":[]
    }))?);
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: hierarchy.into(),
        stage: ScanStage::Completed,
    });
    app.accounts.table = AccountsTable::Ready(
        AccountsReading::parse(
            br#"{
        "accounts":[{"label":"claude1","roles":[],"use_credits":false,
        "sessions":[{"session":"director","state":"busy"},
        {"session":"unit","state":"waiting"},{"session":"worker","state":"idle"}]}],"problems":[]
    }"#,
        )
        .map_err(|error| error.to_string())?,
    );
    Ok(app)
}

fn compare_details_tile_inks() -> TestResult {
    let mut app = ink_application()?;
    let mut renderer = Renderer::new(&mut app, 240, 42)?;
    let layout = renderer.paint(&mut app)?;
    for (name, fields) in [
        (
            "unit-name",
            vec![
                ("account", "claude1"),
                ("role", "unit"),
                ("status", "waiting"),
                ("model", "opus"),
                ("effort", "high"),
                ("branch", "ink-branch"),
                ("directory", "/ink-directory"),
            ],
        ),
        (
            "ink-session",
            vec![("name", "ink-session"), ("account", "codex1 fast")],
        ),
        ("worker-name", vec![("activity", "impl")]),
        ("timer-name", vec![("type", "shell")]),
    ] {
        let tiles = ink_tile_buffers(&mut app, &mut renderer, &layout, name)?;
        check_row_inks(&mut app, name, &fields, &tiles)?;
        app.details_overlay = DetailsOverlay::Closed;
    }
    Ok(())
}

struct InkTileBuffers {
    active:     Buffer,
    remembered: Buffer,
    plain:      Buffer,
}

fn ink_tile_buffers(
    app: &mut App,
    renderer: &mut Renderer,
    layout: &Buffer,
    name: &str,
) -> TestResult<InkTileBuffers> {
    app.click(details_test_locate(layout, name)?);
    tui_pane::dispatch_key(app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let active = renderer.paint(app)?;
    let away = if name == "ink-session" {
        "unit-name"
    } else {
        "ink-alternate"
    };
    app.click(details_test_locate(&active, away)?);
    tui_pane::dispatch_key(app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let remembered = renderer.paint(app)?;
    let alternate = match name {
        "ink-session" => "ink-alternate",
        "worker-name" => "unit-name",
        _ => "worker-name",
    };
    app.click(details_test_locate(&remembered, alternate)?);
    tui_pane::dispatch_key(app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let plain = renderer.paint(app)?;
    app.click(details_test_locate(&plain, name)?);
    tui_pane::dispatch_key(app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    tui_pane::dispatch_key(app, KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    Ok(InkTileBuffers {
        active,
        remembered,
        plain,
    })
}

const DEFAULT_INK_PROPERTIES: [&str; 5] = ["showrunner", "unit", "desktop", "pid", "age"];

fn details_property_index(app: &App, property: &str) -> TestResult<usize> {
    let DetailsOverlay::Open { details, .. } = &app.details_overlay else {
        return Err("details overlay".into());
    };
    details
        .properties
        .iter()
        .position(|(key, _)| key.label() == property)
        .ok_or_else(|| format!("missing {property}").into())
}

fn select_details_property(app: &mut App, index: usize) {
    if let DetailsOverlay::Open { selected, .. } = &mut app.details_overlay {
        *selected = index;
    }
}

fn check_row_inks(
    app: &mut App,
    name: &str,
    fields: &[(&str, &str)],
    tiles: &InkTileBuffers,
) -> TestResult {
    let DetailsOverlay::Open { details, .. } = &app.details_overlay else {
        return Err("details overlay".into());
    };
    let plain_index = details
        .properties
        .iter()
        .position(|(key, _)| {
            !fields.iter().any(|(property, _)| *property == key.label())
                && (name != "unit-name" || !DEFAULT_INK_PROPERTIES.contains(&key.label()))
        })
        .ok_or("plain selection property")?;
    select_details_property(app, plain_index);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 32))?;
    terminal.draw(|frame| {
        app.details_overlay.render_for_focus(
            frame,
            app.framework.color_mode,
            PaneFocusState::Active,
        );
    })?;
    let plain_active = terminal.backend().buffer().clone();
    terminal.draw(|frame| {
        app.details_overlay.render_for_focus(
            frame,
            app.framework.color_mode,
            PaneFocusState::Remembered,
        );
    })?;
    let plain_remembered = terminal.backend().buffer().clone();
    for (property, word) in fields {
        let index = details_property_index(app, property)?;
        let label = details_test_locate(&plain_active, &format!("{property}:"))?;
        let point = details_test_positions(&plain_active, word)
            .into_iter()
            .find(|point| point.y == label.y && point.x > label.x)
            .ok_or("details value")?;
        let tile_point = tile_property_position(&tiles.plain, name, property, word)?;
        let value = drawn_property(&drawn_details(&plain_active), property)?;
        let width = u16::try_from(unicode_width::UnicodeWidthStr::width(value.as_str()))?;
        for (selected_value, focus) in [
            (false, PaneFocusState::Active),
            (true, PaneFocusState::Active),
            (false, PaneFocusState::Remembered),
            (true, PaneFocusState::Remembered),
        ] {
            let overlay = if selected_value {
                select_details_property(app, index);
                terminal.draw(|frame| {
                    app.details_overlay
                        .render_for_focus(frame, app.framework.color_mode, focus);
                })?;
                terminal.backend().buffer()
            } else if focus == PaneFocusState::Active {
                &plain_active
            } else {
                &plain_remembered
            };
            let tile = match (selected_value, focus) {
                (true, PaneFocusState::Remembered) => &tiles.remembered,
                (true, _) => &tiles.active,
                (false, _) => &tiles.plain,
            };
            for offset in 0..width {
                let tile_x = tile_point.x + if *property == "activity" { 0 } else { offset };
                let cell = &overlay[(point.x + offset, point.y)];
                assert_eq!(
                    cell.fg,
                    tile[(tile_x, tile_point.y)].fg,
                    "{name} {property} selected={selected_value} focus={focus:?} cell={offset}"
                );
                if selected_value {
                    assert_eq!(cell.bg, tile[(tile_x, tile_point.y)].bg);
                }
                assert!(
                    crate::footer_colors::antialiased_contrast(cell.fg, cell.bg) >= 4.5,
                    "{name} {property}: {:?} on {:?}",
                    cell.fg,
                    cell.bg
                );
            }
        }
    }
    if name == "unit-name" {
        check_default_inks(app, &mut terminal, &plain_active)?;
    }
    Ok(())
}

fn check_default_inks(
    app: &mut App,
    terminal: &mut Terminal<TestBackend>,
    plain: &Buffer,
) -> TestResult {
    for property in DEFAULT_INK_PROPERTIES {
        let index = details_property_index(app, property)?;
        let label = details_test_locate(plain, &format!("{property}:"))?;
        let text: String = (label.x..plain.area.right())
            .map(|x| plain[(x, label.y)].symbol())
            .collect();
        let colon = text.find(':').ok_or("default value colon")?;
        let rest = &text[colon + 1..];
        let start = u16::try_from(colon + 1 + rest.len() - rest.trim_start().len())?;
        let value = drawn_property(&drawn_details(plain), property)?;
        for selected_value in [false, true] {
            let buffer = if selected_value {
                select_details_property(app, index);
                terminal
                    .draw(|frame| app.details_overlay.render(frame, app.framework.color_mode))?;
                terminal.backend().buffer()
            } else {
                plain
            };
            for offset in 0..u16::try_from(unicode_width::UnicodeWidthStr::width(value.as_str()))? {
                assert_eq!(
                    buffer[(label.x + start + offset, label.y)].fg,
                    buffer[(label.x, label.y)].fg,
                    "default ink {property}"
                );
            }
        }
    }
    Ok(())
}

fn tile_property_position(
    buffer: &Buffer,
    name: &str,
    property: &str,
    word: &str,
) -> TestResult<Position> {
    let row = details_test_locate(buffer, name)?;
    let column = if property == "name" {
        "session"
    } else {
        property
    };
    let header = details_test_positions(buffer, column)
        .into_iter()
        .filter(|point| point.y < row.y && buffer[(point.x, point.y)].fg == tui_pane::label_color())
        .max_by_key(|point| point.y)
        .ok_or_else(|| format!("missing {column} header"))?;
    details_test_positions(buffer, word)
        .into_iter()
        .find(|point| point.y == row.y && point.x >= header.x)
        .ok_or_else(|| format!("tile value {name} {property} {word}").into())
}

/// The lookup words use narrow characters that occupy one cell each.
fn details_test_positions(buffer: &Buffer, text: &str) -> Vec<Position> {
    assert_ne!(text, "");
    let cells: Vec<String> = text.chars().map(String::from).collect();
    let mut positions = Vec::new();
    let Some(last) = buffer
        .area
        .right()
        .checked_sub(u16::try_from(cells.len()).unwrap_or(u16::MAX))
    else {
        return positions;
    };
    for y in buffer.area.y..buffer.area.bottom() {
        for x in buffer.area.x..=last {
            if cells
                .iter()
                .zip(x..)
                .all(|(cell, column)| buffer[(column, y)].symbol() == cell)
            {
                positions.push(Position::new(x, y));
            }
        }
    }
    positions
}

fn details_test_locate(buffer: &Buffer, text: &str) -> TestResult<Position> {
    details_test_positions(buffer, text)
        .into_iter()
        .next()
        .ok_or_else(|| format!("missing painted {text}").into())
}

#[test]
fn details_click_selects_the_value_and_preserves_non_value_clicks() -> TestResult {
    use tui_pane::Shortcuts;

    use crate::details::DetailsPane;

    let mut app = details_click_app()?;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 32))?;
    terminal.draw(|frame| app.details_overlay.render(frame, app.framework.color_mode))?;
    let buffer = terminal.backend().buffer();
    let target = details_test_locate(buffer, "effort:")?;
    let heading = details_test_locate(buffer, "hierarchy")?;
    let blank = Position::new(heading.x, heading.y - 1);
    app.click(target);
    let actions = DetailsPane::defaults().into_scope_map();
    let copy = actions
        .action_for(&tui_pane::KeyBind::from('y'))
        .ok_or("y binding")?;
    let mut clipboard = CopiedValues::default();
    copy.apply(&mut app, &mut clipboard);
    assert_eq!(clipboard.0, ["—"]);
    for point in [heading, blank, Position::new(0, 0)] {
        app.click(point);
        copy.apply(&mut app, &mut clipboard);
        assert_eq!(clipboard.0.last().ok_or("copied value")?, "—");
    }
    terminal.draw(|frame| app.details_overlay.render(frame, app.framework.color_mode))?;
    assert_eq!(
        terminal.backend().buffer()[(target.x, target.y)].bg,
        tui_pane::active_focus_color()
    );
    tui_pane::dispatch_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    app.scroll(target, ScrollDirection::Down);
    copy.apply(&mut app, &mut clipboard);
    assert_eq!(clipboard.0.last().ok_or("wheel copy")?, "—");
    Ok(())
}

fn details_click_app() -> TestResult<App> {
    let now = census::unix_now();
    let mut app = application(now, 86400)?;
    let mut renderer = Renderer::new(&mut app, 120, 32)?;
    let buffer = renderer.paint(&mut app)?;
    app.click(details_test_locate(&buffer, "director-name")?);
    tui_pane::dispatch_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    Ok(app)
}

fn paint_details(app: &mut App, terminal: &mut Terminal<TestBackend>) -> TestResult<Buffer> {
    terminal.draw(|frame| app.details_overlay.render(frame, app.framework.color_mode))?;
    Ok(terminal.backend().buffer().clone())
}
