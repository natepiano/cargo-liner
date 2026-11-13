//! Row columns and details use the same source values.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
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
fn row_columns_put_account_first_and_separate_role_activity_and_timer() -> TestResult {
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
        ["account", "session", "role", "activity", "status"]
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
            assert_selected_details(&mut app, &mut renderer, view, expected)?;
        }
    }
    assert_child_details_use_own_values()?;
    Ok(())
}

fn assert_selected_details(
    app: &mut App,
    renderer: &mut Renderer,
    view: TileView,
    expected: &str,
) -> TestResult {
    let buffer = renderer.paint(app)?;
    let director = word_click_tests::positions(&buffer, "director-name")
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
        match view {
            TileView::Cells => [
                "account",
                "name",
                "role",
                "activity",
                "status",
                "directory",
                "pid",
                "branch",
                "program",
                "desktop",
                "state",
                "age"
            ],
            TileView::Summary => [
                "pid",
                "program",
                "name",
                "status",
                "desktop",
                "directory",
                "branch",
                "account",
                "role",
                "activity",
                "state",
                "age"
            ],
        }
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
        "program: claude",
        "account: claude1",
        "directory: /fixture/two  spaces",
        "branch: feature/two  spaces",
        "desktop: two  spaces",
        "state: waiting",
        &format!("age: {expected}"),
    ] {
        assert!(
            drawn.iter().any(|line| line.contains(property)),
            "{property}"
        );
    }
    tui_pane::dispatch_key(app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let closed = renderer.paint(app)?;
    assert_eq!(
        word_click_tests::positions(&closed, "age:"),
        [] as [ratatui::prelude::Position; 0]
    );
    Ok(())
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
        assert!(
            drawn
                .iter()
                .any(|line| line.contains(&format!("program: {program}")))
        );
        if name == "timer-name" {
            assert!(drawn.iter().any(|line| line.contains("pid: 11")));
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
