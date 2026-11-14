//! The grid draws staff producers in one `producers` tile that keeps its
//! sort, expansion and selection when the producers change.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use serde_json::json;
use tui_pane::ExpansionAction;
use tui_pane::TerminalApp;
use tui_pane::TileColumnOrder;
use tui_pane::TileView;

use super::PaintedSurfaceContent;
use super::details_tests;
use super::ranked_column_tests;
use super::word_click_tests::Renderer;
use crate::agent_cell::TreeColumn;
use crate::app::App;
use crate::app::expansion;
use crate::app::expansion::HandlerExpansionAction;
use crate::census::AgentRow;
use crate::constants::AGENT_CELL_TITLE_LEAD;
use crate::constants::PRODUCERS_CELL_TITLE;
use crate::sort::SortDirection;
use crate::tiles::TileKey;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
/// The clock `ranked_column_tests::paint` draws at.
const NOW: u64 = 1_794_096_000;
const WIDTH: u16 = 181;
const HEIGHT: u16 = 70;

/// A root session; `producer` says whether staff names it a producer.
fn session(pid: u32, name: &str, producer: bool, work: Option<&str>) -> TestResult<AgentRow> {
    let children: Vec<_> = work
        .into_iter()
        .map(|work| json!({"depth":0,"kind":"shell","pid":pid + 1,"name":work,"started":NOW-10}))
        .collect();
    Ok(serde_json::from_value(json!({
        "agent":"claude","pid":pid,"session_id":format!("session-{pid}"),"name":name,
        "producer":producer,"started":NOW-1000,"directory":"/fixture","children":children,
        "account":{"home":"~/.claude1","label":"claude1"}
    }))?)
}

fn rows(producers: &[(u32, &str)]) -> TestResult<Vec<AgentRow>> {
    let mut rows = vec![
        session(10, "director", false, Some("director-work"))?,
        session(20, "loner", false, None)?,
    ];
    for &(pid, name) in producers {
        rows.push(session(pid, name, true, Some(&format!("{name}-work")))?);
    }
    Ok(rows)
}

pub(super) fn application(producers: &[(u32, &str)]) -> TestResult<App> {
    let mut app = App::new_for_test()?;
    app.loaded_config.config.tiles.view = TileView::Cells;
    app.loaded_config.config.tiles.widen_summary = false;
    ranked_column_tests::publish(&mut app, rows(producers)?);
    expansion::dispatch(
        &mut app,
        HandlerExpansionAction::Shared(ExpansionAction::ExpandAll),
    );
    Ok(app)
}

fn producers_text(app: &App, buffer: &Buffer) -> TestResult<String> {
    let rect = ranked_column_tests::positions(app)
        .remove(&TileKey::Producers)
        .ok_or("the producers tile")?;
    Ok(ranked_column_tests::rect_text(buffer, rect))
}

pub(super) fn locate(buffer: &Buffer, rect: Rect, text: &str) -> TestResult<Position> {
    (rect.y..rect.bottom())
        .find_map(|y| {
            let line: String = (rect.x..rect.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            line.find(text).map(|at| {
                let x = u16::try_from(line[..at].chars().count()).unwrap_or(u16::MAX);
                Position::new(rect.x.saturating_add(x), y)
            })
        })
        .ok_or_else(|| format!("missing {text}").into())
}

fn row_order(text: &str, first: &str, second: &str) -> TestResult<bool> {
    Ok(text.find(first).ok_or("first row")? < text.find(second).ok_or("second row")?)
}

#[test]
fn the_producers_tile_is_titled_producers_and_holds_no_unassigned_session() -> TestResult {
    let mut app = application(&[(30, "morty"), (40, "brain")])?;
    let buffer = ranked_column_tests::paint(&mut app, WIDTH, HEIGHT)?;
    let frame = ranked_column_tests::rect_text(&buffer, buffer.area);
    assert!(
        frame.contains(&format!("{AGENT_CELL_TITLE_LEAD}{PRODUCERS_CELL_TITLE}")),
        "{frame}"
    );
    let producers = producers_text(&app, &buffer)?;
    for name in ["morty", "brain"] {
        assert!(producers.contains(name), "{producers}");
    }
    assert!(!producers.contains("loner"), "{producers}");
    let unassigned = ranked_column_tests::rect_text(
        &buffer,
        ranked_column_tests::positions(&app)
            .remove(&TileKey::Unassigned {
                machine: "natedev".to_owned(),
            })
            .ok_or("the unassigned tile")?,
    );
    assert!(unassigned.contains("loner"), "{unassigned}");
    assert!(!unassigned.contains("morty"), "{unassigned}");
    Ok(())
}

#[test]
fn the_producers_tile_keeps_sort_expansion_and_selection_when_producers_change() -> TestResult {
    let mut app = application(&[(30, "morty"), (40, "brain")])?;
    ranked_column_tests::paint(&mut app, WIDTH, HEIGHT)?;
    let surface = app
        .painted_surfaces
        .iter()
        .find_map(|surface| match &surface.content {
            PaintedSurfaceContent::Tree { key, spans, .. } if key == &TileKey::Producers => spans
                .iter()
                .find(|span| span.column == TreeColumn::Session)
                .map(|span| {
                    Position::new(
                        surface.area.x + span.columns.start,
                        surface.area.y + span.row,
                    )
                }),
            _ => None,
        })
        .ok_or("the producers session header")?;
    app.click(surface);
    let sort = *app
        .tree_sorts
        .get(&TileKey::Producers)
        .ok_or("the clicked sort")?;
    assert_eq!(sort.column, TreeColumn::Session);
    let buffer = ranked_column_tests::paint(&mut app, WIDTH, HEIGHT)?;
    let rect = ranked_column_tests::positions(&app)
        .remove(&TileKey::Producers)
        .ok_or("the producers tile")?;
    // A press on a row selects it and closes its work.
    app.click(locate(&buffer, rect, "morty")?);
    ranked_column_tests::publish(
        &mut app,
        rows(&[(30, "morty"), (40, "brain"), (50, "dwight")])?,
    );
    let buffer = ranked_column_tests::paint(&mut app, WIDTH, HEIGHT)?;
    let text = producers_text(&app, &buffer)?;
    assert_eq!(app.tree_sorts.get(&TileKey::Producers), Some(&sort));
    let ascending = sort.direction == SortDirection::Ascending;
    assert_eq!(row_order(&text, "brain", "dwight")?, ascending, "{text}");
    assert_eq!(row_order(&text, "dwight", "morty")?, ascending, "{text}");
    assert!(text.contains("dwight-work"), "{text}");
    assert!(!text.contains("morty-work"), "{text}");
    tui_pane::dispatch_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    // The details view draws through `App::draw`, which the fixed-clock paint leaves out.
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let drawn = details_tests::drawn_details(&renderer.paint(&mut app)?);
    assert_eq!(details_tests::drawn_property(&drawn, "name")?, "morty");
    assert_eq!(details_tests::drawn_property(&drawn, "role")?, "producer");
    Ok(())
}

#[test]
fn the_producers_tile_ranks_by_its_full_tree() -> TestResult {
    let app = application(&[(30, "morty"), (40, "brain")])?;
    let TileColumnOrder::LongestWithShortest(counts) = ranked_column_tests::ranks(&app) else {
        return Err("ranked columns required".into());
    };
    let producers = counts
        .iter()
        .find(|(key, _)| key == &TileKey::Producers)
        .ok_or("the producers rank")?;
    assert_eq!(producers.1, 4, "two producers and their work");
    Ok(())
}
