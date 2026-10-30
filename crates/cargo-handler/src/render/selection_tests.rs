//! Selection is checked through input events and painted rows.

#![expect(clippy::expect_used, reason = "rendered fixture rows must exist")]
#![expect(clippy::panic, reason = "a missing fixture row must fail the test")]

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use tui_pane::PaneFocusState;
use tui_pane::ScrollDirection;
use tui_pane::TerminalApp;

use super::*;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::ChildKind;
use crate::census::HierarchyEdges;
use crate::census::ShellTimerObservation;
use crate::census::Timer;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::transcript::SubagentRole;
use crate::interaction::SelectionSurface;

fn session(pid: u32, name: &str) -> AgentRow {
    serde_json::from_value(serde_json::json!({
        "agent":"claude", "pid":pid, "name":name, "started":100,
        "session_id":format!("session-{pid}"), "directory":"/fixture",
        "status":"idle", "children":[]
    }))
    .expect("session fixture")
}

fn apply_rows(app: &mut App, rows: Vec<AgentRow>) {
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges::default().into(),
        stage: ScanStage::Completed,
    });
}

fn application(rows: Vec<AgentRow>) -> App {
    let mut app = App::new_for_test().expect("test app");
    app.loaded_config.config.tiles.initial_rows =
        serde_json::from_value(serde_json::json!(8)).expect("one screen column");
    app.loaded_config.config.tiles.widen_summary = false;
    apply_rows(&mut app, rows);
    app
}

fn paint(app: &mut App) -> Buffer { paint_at(app, census::unix_now()) }

fn paint_at(app: &mut App, now: u64) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(180, 48)).expect("test terminal");
    let keymap = app.keymap();
    terminal
        .draw(|frame| draw_at_for_test(frame, app, &keymap, now))
        .expect("first frame");
    app.tiles.settle_for_test();
    terminal
        .draw(|frame| draw_at_for_test(frame, app, &keymap, now))
        .expect("settled frame");
    terminal.backend().buffer().clone()
}

fn locate(buffer: &Buffer, word: &str) -> Position {
    for y in buffer.area.y..buffer.area.bottom() {
        let line: String = (buffer.area.x..buffer.area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(index) = line.find(word) {
            return Position::new(
                buffer.area.x + u16::try_from(line[..index].chars().count()).expect("column"),
                y,
            );
        }
    }
    panic!("missing drawn row {word}");
}

fn assert_selected(buffer: &Buffer, label: &str) {
    let point = locate(buffer, label);
    assert_eq!(
        buffer[(point.x, point.y)].bg,
        tui_pane::selection_style(PaneFocusState::Active)
            .bg
            .expect("selected background"),
        "selected row {label}"
    );
}

fn press(app: &mut App, code: KeyCode) {
    tui_pane::dispatch_key(app, KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn clicking_a_tile_row_selects_it_and_vertical_keys_stop_at_the_edges() {
    let mut app = application(vec![
        session(10, "alpha"),
        session(20, "beta"),
        session(30, "gamma"),
    ]);
    let before = paint(&mut app);
    app.click(locate(&before, "beta"));
    assert_selected(&paint(&mut app), "beta");
    for (key, expected) in [
        (KeyCode::Up, "alpha"),
        (KeyCode::Up, "alpha"),
        (KeyCode::Down, "beta"),
        (KeyCode::End, "gamma"),
        (KeyCode::Down, "gamma"),
        (KeyCode::Home, "alpha"),
        (KeyCode::PageDown, "gamma"),
        (KeyCode::PageUp, "alpha"),
    ] {
        press(&mut app, key);
        assert_selected(&paint(&mut app), expected);
    }
}

#[test]
fn horizontal_keys_and_tabs_move_tile_focus_without_moving_the_row() {
    let mut app = application(vec![
        with_shell(session(10, "left-owner")),
        with_shell(session(20, "right-top-owner")),
        with_shell(session(30, "right-bottom-owner")),
    ]);
    app.loaded_config.config.tiles.initial_rows =
        serde_json::from_value(serde_json::json!(1)).expect("one screen row");
    let before = paint(&mut app);
    let left = locate(&before, "build-for-10");
    let right_top = locate(&before, "build-for-20");
    let right = locate(&before, "build-for-30");
    assert_eq!(right_top.x, right.x, "both right rows share a column");
    assert!(right_top.y < right.y, "the right column has two tiles");
    assert!(left.x < right.x, "the right row is beside the left row");
    app.click(locate(&before, "build-for-10"));
    assert_selected(&paint(&mut app), "build-for-10");
    for (key, expected) in [
        (KeyCode::Right, "build-for-30"),
        (KeyCode::Left, "build-for-10"),
        (KeyCode::Tab, "build-for-20"),
        (KeyCode::BackTab, "build-for-10"),
    ] {
        let modifiers = if key == KeyCode::BackTab {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        tui_pane::dispatch_key(&mut app, KeyEvent::new(key, modifiers));
        let drawn = paint(&mut app);
        let point = locate(&drawn, expected);
        assert_eq!(
            drawn[(point.x, point.y)].bg,
            tui_pane::active_focus_color(),
            "focus after {key:?} must select {expected}; grid focus={:?}",
            app.tiles.focused_cell()
        );
    }
}

#[test]
fn selection_follows_a_renamed_session_and_uses_neighbors_after_removal() {
    let mut app = application(vec![
        session(10, "alpha"),
        session(20, "beta"),
        session(30, "gamma"),
    ]);
    let before = paint(&mut app);
    app.click(locate(&before, "beta"));
    assert_selected(&paint(&mut app), "beta");
    apply_rows(
        &mut app,
        vec![
            session(10, "alpha"),
            session(20, "renamed-beta"),
            session(30, "gamma"),
        ],
    );
    assert_selected(&paint(&mut app), "renamed-beta");
    apply_rows(&mut app, vec![session(10, "alpha"), session(30, "gamma")]);
    assert_selected(&paint(&mut app), "gamma");
    apply_rows(&mut app, vec![session(10, "alpha")]);
    assert_selected(&paint(&mut app), "alpha");
    apply_rows(&mut app, Vec::new());
    let empty = paint(&mut app);
    let drawn: String = empty
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    for removed in ["alpha", "beta", "gamma", "renamed-beta"] {
        assert!(!drawn.contains(removed), "removed row {removed} is absent");
    }
    let surface = SelectionSurface::Tile(TileKey::Unassigned {
        machine: "natedev".into(),
    });
    assert_eq!(
        app.selections.surface(&surface).map_or(
            crate::interaction::TileSelection::NoRows,
            crate::interaction::SurfaceSelection::selection,
        ),
        crate::interaction::TileSelection::NoRows,
        "empty pool has no selected row"
    );
}

#[test]
fn a_reused_pid_does_not_take_selection_from_the_surviving_neighbor() {
    let mut app = application(vec![
        session(10, "alpha"),
        session(20, "beta"),
        session(30, "gamma"),
    ]);
    let before = paint(&mut app);
    app.click(locate(&before, "beta"));
    let mut replacement = session(20, "aardvark-new-session");
    replacement.session_id = String::from("different-session").into();
    replacement.started = 200;
    replacement.process_started = 200;
    apply_rows(
        &mut app,
        vec![session(10, "alpha"), replacement, session(30, "gamma")],
    );
    assert_selected(&paint(&mut app), "alpha");
}

#[test]
fn duplicate_descriptions_keep_the_exact_session_selected_after_reordering() {
    let mut first = session(10, "same-description");
    first.branch = Some("first-identity".into());
    let mut second = session(20, "same-description");
    second.branch = Some("second-identity".into());
    let mut app = application(vec![first.clone(), second.clone()]);
    let before = paint(&mut app);
    app.click(locate(&before, "second-identity"));
    assert_selected(&paint(&mut app), "second-identity");
    first.name = "z-last".into();
    second.name = "a-first".into();
    apply_rows(&mut app, vec![second, first]);
    assert_selected(&paint(&mut app), "second-identity");
}

fn with_shell(mut row: AgentRow) -> AgentRow {
    row.children.push(
        serde_json::from_value(serde_json::json!({
            "kind":"shell", "depth":0, "pid":9000+row.pid,
            "name":format!("build-for-{}",row.pid), "started":100,
        }))
        .expect("shell fixture"),
    );
    row
}

#[test]
fn summary_rows_take_clicks_keys_and_neighbor_fallback_after_session_removal() {
    let mut app = application(vec![
        with_shell(session(10, "alpha-summary")),
        with_shell(session(20, "beta-summary")),
        with_shell(session(30, "gamma-summary")),
    ]);
    let before = paint(&mut app);
    app.click(locate(&before, "beta-summary"));
    assert_selected(&paint(&mut app), "beta-summary");
    press(&mut app, KeyCode::Down);
    assert_selected(&paint(&mut app), "gamma-summary");
    apply_rows(
        &mut app,
        vec![
            with_shell(session(10, "alpha-summary")),
            with_shell(session(20, "beta-summary")),
        ],
    );
    assert_selected(&paint(&mut app), "beta-summary");
    press(&mut app, KeyCode::Up);
    assert_selected(&paint(&mut app), "alpha-summary");
}

#[test]
fn a_summary_machine_group_can_take_selection_before_its_first_session() {
    let mut app = application(vec![
        with_shell(session(10, "alpha-summary")),
        with_shell(session(20, "beta-summary")),
    ]);
    let before = paint(&mut app);
    app.click(locate(&before, "natedev"));
    assert_selected(&paint(&mut app), "natedev");
    press(&mut app, KeyCode::Down);
    assert_selected(&paint(&mut app), "alpha-summary");
    press(&mut app, KeyCode::Up);
    assert_selected(&paint(&mut app), "natedev");
}

#[test]
fn selection_follows_a_session_between_the_unassigned_pool_and_summary() {
    let moving = session(10, "moving-session");
    let mut app = application(vec![moving.clone(), session(20, "neighbor")]);
    let before = paint(&mut app);
    app.click(locate(&before, "moving-session"));
    assert_selected(&paint(&mut app), "moving-session");
    apply_rows(
        &mut app,
        vec![with_shell(moving.clone()), session(20, "neighbor")],
    );
    assert_selected(&paint(&mut app), "moving-session");
    apply_rows(&mut app, vec![moving, session(20, "neighbor")]);
    assert_selected(&paint(&mut app), "moving-session");
}

#[test]
fn the_wheel_scrolls_the_tile_under_the_pointer_and_leaves_the_focused_tile_alone() {
    let owner = with_shell(session(10, "focused-owner"));
    let pool: Vec<_> = (20..60)
        .map(|pid| session(pid, &format!("pool-{pid:02}")))
        .collect();
    let mut rows = vec![owner];
    rows.extend(pool);
    let mut app = application(rows);
    let before = paint(&mut app);
    app.click(locate(&before, "build-for-10"));
    let selected = paint(&mut app);
    assert_selected(&selected, "build-for-10");
    let pointer = locate(&selected, "pool-20");
    // The pointer remains over the pool while keyboard focus stays on the work tile.
    for _ in 0..40 {
        app.scroll(pointer, ScrollDirection::Down);
    }
    let after = paint(&mut app);
    assert_selected(&after, "build-for-10");
    let first_pool = |buffer: &Buffer| {
        (20..60)
            .find(|pid| {
                (0..buffer.area.height).any(|y| {
                    let text: String = (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    text.contains(&format!("pool-{pid:02}"))
                })
            })
            .expect("a pool row is drawn")
    };
    assert!(
        first_pool(&after) > first_pool(&selected),
        "the pointed pool scrolls to later rows"
    );
}

fn tree_header(buffer: &Buffer) -> String {
    (buffer.area.y..buffer.area.bottom())
        .find_map(|y| {
            let line: String = (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            (line.contains("session") && line.contains("role") && !line.contains("pid"))
                .then_some(line)
        })
        .expect("tree header")
}

fn assert_age_at_header_edge(buffer: &Buffer, label: &str, header: &str, expected: &str) -> u16 {
    let age = header.find("age ▼").expect("age sort header");
    let end = buffer.area.x + summary::cell_width(&header[..age]) + summary::cell_width("age ▼");
    let y = locate(buffer, label).y;
    let through_age: String = (buffer.area.x..end)
        .map(|x| buffer[(x, y)].symbol())
        .collect();
    let (_, actual) = through_age
        .rsplit_once(char::is_whitespace)
        .expect("age follows another column");
    assert_eq!(actual, expected, "{label}: age ends at the header edge");
    end
}

#[test]
fn scrolling_measures_painted_roles_but_keeps_complete_group_age_slots() {
    let now = census::unix_now();
    let mut owner = session(10, "scroll-owner");
    owner.children = (0..70)
        .map(|index| {
            let age = if index == 0 {
                6 * 24 * 3600 + 17 * 3600
            } else {
                (70 - index) * 60
            };
            serde_json::from_value(serde_json::json!({
            "kind":"subagent", "subagent_role":crate::census::transcript::SubagentRole::from_record(
                if index==0 {"showrunner"} else {"review"}, ""),
            "depth":0, "name":format!("task-{index:02}"), "started":now-age,
        })).expect("scroll work")
        })
        .collect();
    let mut app = application(vec![owner]);
    let before = paint_at(&mut app, now);
    app.click(locate(&before, "task-00"));
    let before = paint_at(&mut app, now);
    assert_selected(&before, "task-00");
    let before_header = tree_header(&before);
    assert!(before_header.contains("age"));
    let role_x = before_header.find("role").expect("role header");
    let status_x = before_header.find("status").expect("status header");
    assert_eq!(status_x - role_x, 12, "showrunner plus two-cell gap");
    press(&mut app, KeyCode::End);
    let after = paint_at(&mut app, now);
    assert_selected(&after, "task-69");
    let after_header = tree_header(&after);
    let role_x = after_header.find("role").expect("retained role header");
    let status_x = after_header.find("status").expect("status header");
    assert_eq!(status_x - role_x, 8, "review plus two-cell gap");
    for word in ["status", "age"] {
        assert_eq!(before_header.find(word), after_header.find(word), "{word}");
    }
    assert_age_at_header_edge(&before, "task-00", &before_header, "6:17:00");
    assert_eq!(
        assert_age_at_header_edge(&before, "task-01", &before_header, "1:09"),
        assert_age_at_header_edge(&after, "task-69", &after_header, "1"),
        "the offscreen age remains part of the complete group width"
    );
}

#[test]
fn grouped_scrolled_tiles_keep_role_columns_and_complete_age_slots() {
    let now = census::unix_now();
    let owners = [10, 20].map(|pid| {
        let mut owner = with_shell(session(pid, &format!("group-owner-{pid}")));
        owner.children = (0..40)
            .map(|index| {
                let role = if index == 0 {
                    crate::census::transcript::SubagentRole::from_record("review", "")
                } else {
                    SubagentRole::Unrecorded
                };
                let age = if pid == 10 && index == 0 {
                    6 * 24 * 3600 + 17 * 3600
                } else {
                    (40 - index) * 60
                };
                serde_json::from_value(serde_json::json!({
                    "kind":"subagent", "depth":0,
                    "name":format!("group-{pid}-task-{index:02}"),
                    "started":now-age, "subagent_role":role
                }))
                .expect("grouped scroll child")
            })
            .collect();
        owner
    });
    let mut app = application(owners.into());
    let before = paint_at(&mut app, now);
    let header_for = |buffer: &Buffer, label: &str| {
        let point = locate(buffer, label);
        (0..point.y)
            .rev()
            .find_map(|y| {
                let text: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                (text.contains("session")
                    && text.contains("role")
                    && text.contains("age")
                    && !text.contains("pid"))
                .then_some(text)
            })
            .expect("retained tree header")
    };
    let before_headers = [10, 20].map(|pid| header_for(&before, &format!("group-{pid}-task-01")));
    assert_age_at_header_edge(&before, "group-10-task-00", &before_headers[0], "6:17:00");
    let before_edges = [0, 1].map(|index| {
        let pid = [10, 20][index];
        assert_age_at_header_edge(
            &before,
            &format!("group-{pid}-task-01"),
            &before_headers[index],
            "39",
        )
    });
    for pid in [10, 20] {
        let current = paint_at(&mut app, now);
        app.click(locate(&current, &format!("group-{pid}-task-00")));
        press(&mut app, KeyCode::End);
        assert_selected(&paint_at(&mut app, now), &format!("group-{pid}-task-39"));
    }
    let after = paint_at(&mut app, now);
    let after_headers = [10, 20].map(|pid| header_for(&after, &format!("group-{pid}-task-39")));
    assert_eq!(before_edges[0], before_edges[1], "shared age right edge");
    for (index, pid) in [10, 20].into_iter().enumerate() {
        let label = format!("group-{pid}-task-39");
        let after_header = &after_headers[index];
        for word in ["role", "status", "age"] {
            let column = before_headers[index]
                .find(word)
                .expect("complete-row column");
            assert_eq!(before_headers[0].find(word), Some(column), "shared {word}");
            assert_eq!(
                after_headers[0].find(word),
                after_header.find(word),
                "shared scrolled {word}"
            );
            assert!(after_header.contains(word), "retained {word}");
            if word != "role" {
                assert_eq!(after_header.find(word), Some(column), "scrolled {word}");
            }
        }
        assert_eq!(
            assert_age_at_header_edge(&after, &label, after_header, "1"),
            before_edges[index]
        );
        let drawn: String = after
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(!drawn.contains(&format!("group-{pid}-task-00")));
    }
}

fn seat_owner(thread: &str, launcher_pid: u32, started: u64, job: &str) -> AgentRow {
    let mut owner = session(10, "seat-owner");
    owner.children = serde_json::from_value(serde_json::json!([
        {"kind":"shell","depth":0,"pid":11,"name":"earlier-build","started":100},
        {"kind":"shell","depth":0,"pid":launcher_pid,"name":"hidden-delivery","started":started,
         "dispatch":{"worker":{"seat":"fixture-impl7","short_seat":"impl7","thread_id":thread,"job":job}}},
        {"kind":"thread","depth":0,"name":"raw-thread-name","thread_id":thread,"started":started},
        {"kind":"shell","depth":0,"pid":12,"name":"later-build","started":300}
    ])).expect("seat and delivery rows");
    owner
}

#[test]
fn a_selected_seat_survives_new_dispatch_text_and_a_relaunched_thread() {
    let mut app = application(vec![seat_owner("thread-one", 701, 200, "first task")]);
    let before = paint(&mut app);
    app.click(locate(&before, "impl7 · first task"));
    assert_selected(&paint(&mut app), "impl7 · first task");
    apply_rows(
        &mut app,
        vec![seat_owner("thread-one", 702, 200, "next task")],
    );
    assert_selected(&paint(&mut app), "impl7 · next task");
    apply_rows(
        &mut app,
        vec![seat_owner("thread-two", 801, 400, "relaunched task")],
    );
    let relaunched = paint(&mut app);
    assert_selected(&relaunched, "impl7 · relaunched task");
    assert!(
        locate(&relaunched, "later-build").y < locate(&relaunched, "impl7 · relaunched task").y,
        "relaunch moves the seat beyond its old index"
    );
}

#[test]
fn recorded_subagent_ids_keep_selection_when_duplicate_descriptions_change_order() {
    let mut owner = session(10, "subagent-owner");
    owner.children = serde_json::from_value(serde_json::json!([
        {"kind":"subagent","depth":0,"name":"same-task","started":100,
         "work_id":{"Recorded":"first-work"},"activity":{"active":"thinking"}},
        {"kind":"subagent","depth":0,"name":"same-task","started":200,
         "work_id":{"Recorded":"second-work"},"activity":{"active":"waiting"}}
    ]))
    .expect("identified subagents");
    let mut app = application(vec![owner.clone()]);
    let before = paint(&mut app);
    app.click(locate(&before, "waiting"));
    assert_selected(&paint(&mut app), "waiting");
    owner.children[0].started = 300;
    owner.children[0].name = "renamed-first-work".into();
    owner.children[1].name = "renamed-selected-work".into();
    apply_rows(&mut app, vec![owner]);
    assert_selected(&paint(&mut app), "renamed-selected-work");
}

#[test]
fn summary_account_rows_take_clicks_and_key_moves_between_accounts() {
    let mut app = application(vec![with_shell(session(10, "summary-owner"))]);
    app.accounts.table = AccountsTable::Ready(AccountsReading::parse(br#"{
        "accounts":[
            {"label":"fixture-first","tool":"claude","roles":[],"weekly_left_percent":50,"use_credits":false},
            {"label":"fixture-second","tool":"codex","roles":[],"weekly_left_percent":75,"use_credits":false}
        ],"problems":[]
    }"#).expect("account rows"));
    let before = paint(&mut app);
    app.click(locate(&before, "fixture-first"));
    assert_selected(&paint(&mut app), "fixture-first");
    press(&mut app, KeyCode::Down);
    assert_selected(&paint(&mut app), "fixture-second");
    press(&mut app, KeyCode::Up);
    assert_selected(&paint(&mut app), "fixture-first");
}

#[test]
fn shared_compression_keeps_every_painted_row_click_on_its_exact_row() {
    let mut short = session(10, "short-place-owner");
    short.children = (0..3)
        .map(|index| {
            serde_json::from_value(serde_json::json!({
                "kind":"shell", "depth":0, "pid":100+index,
                "name":format!("compressed-target-{index}"), "started":100+index
            }))
            .expect("clickable work row")
        })
        .collect();
    let mut long = short.clone();
    long.pid = 20;
    long.session_id = String::from("long-place-session").into();
    long.name = "long-place-owner".into();
    long.directory = format!("/{}", "long-directory-component/".repeat(12));
    for (index, child) in long.children.iter_mut().enumerate() {
        child.name = format!("long-place-work-{index}");
        child.pid = Some(200 + u32::try_from(index).expect("child index"));
    }
    let mut alone = application(vec![short.clone()]);
    let full = paint(&mut alone);
    let full_row = locate(&full, "compressed-target-0");
    let mut app = application(vec![short, long]);
    let grouped = paint(&mut app);
    let grouped_row = locate(&grouped, "compressed-target-0");
    assert!(
        grouped_row.y < full_row.y,
        "shared long heading compresses the otherwise full first tile"
    );
    for index in 0..3 {
        let label = format!("compressed-target-{index}");
        let current = paint(&mut app);
        app.click(locate(&current, &label));
        assert_selected(&paint(&mut app), &label);
    }
}

#[test]
fn a_background_summary_session_joining_the_pool_keeps_its_focused_selection() {
    let first = session(10, "focused-pool-session");
    let second = session(20, "background-summary-session");
    let mut app = application(vec![first.clone(), with_shell(second.clone())]);
    let before = paint(&mut app);
    app.click(locate(&before, "background-summary-session"));
    assert_selected(&paint(&mut app), "background-summary-session");
    let current = paint(&mut app);
    app.click(locate(&current, "focused-pool-session"));
    assert_selected(&paint(&mut app), "focused-pool-session");
    apply_rows(&mut app, vec![first, second]);
    assert_selected(&paint(&mut app), "focused-pool-session");
}

#[test]
fn removing_the_focused_row_paints_its_successor_active_on_the_first_frame() {
    let mut app = application(vec![
        session(10, "first-session"),
        session(20, "removed-session"),
        session(30, "successor-session"),
    ]);
    let before = paint(&mut app);
    app.click(locate(&before, "removed-session"));
    assert_selected(&paint(&mut app), "removed-session");
    apply_rows(
        &mut app,
        vec![
            session(10, "first-session"),
            session(30, "successor-session"),
        ],
    );
    let mut terminal = Terminal::new(TestBackend::new(180, 48)).expect("first update frame");
    let keymap = app.keymap();
    terminal
        .draw(|frame| draw(frame, &mut app, &keymap))
        .expect("one frame after removal");
    assert_selected(terminal.backend().buffer(), "successor-session");
}

#[test]
fn a_pidless_shell_keeps_selection_when_its_timer_changes_and_a_neighbor_leaves() {
    let mut owner = session(10, "legacy-shell-owner");
    owner.children = serde_json::from_value(serde_json::json!([
        {"kind":"shell","depth":0,"pid":11,"name":"earlier-neighbor","started":100},
        {"kind":"shell","depth":0,"name":"selected-pidless-shell","started":200,
         "timer":{"started":200,"deadline":census::unix_now()+600}},
        {"kind":"shell","depth":0,"pid":12,"name":"later-neighbor","started":300}
    ]))
    .expect("legacy shell timer fixture");
    let mut app = application(vec![owner.clone()]);
    let before = paint(&mut app);
    app.click(locate(&before, "selected-pidless-shell"));
    assert_selected(&paint(&mut app), "selected-pidless-shell");
    owner.children.remove(0);
    owner.children[0].kind = ChildKind::Shell(ShellTimerObservation::ObservedCountdown(Timer {
        started:  200,
        deadline: census::unix_now() + 900,
    }));
    apply_rows(&mut app, vec![owner]);
    assert_selected(&paint(&mut app), "selected-pidless-shell");
}
