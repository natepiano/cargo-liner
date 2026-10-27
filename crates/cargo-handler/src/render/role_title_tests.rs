//! Tile title role suffixes use the same footer palette as tree role words.

#![expect(
    clippy::expect_used,
    reason = "fixture tiles and role suffixes must exist"
)]
#![expect(clippy::panic, reason = "every hue fixture must have its own tile")]

use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Arc;

use agent_cell::TreeColumn;
use ratatui::buffer::Cell;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use tui_pane::Appearance;
use tui_pane::CellPoint;

use super::*;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdge;
use crate::census::HierarchyEdges;
use crate::census::HierarchyRole;
use crate::census::MachineState;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::hierarchy::ReviewerState;
use crate::census::scan::ScanStage;
use crate::footer_colors;
use crate::footer_colors::FooterColor;
use crate::footer_colors::FooterRole;
use crate::footer_colors::FooterTint;
use crate::footer_colors::tests;
use crate::sort::SortDirection;
use crate::sort::SummaryColumn;
use crate::sort::TreeSort;
use crate::summary;
use crate::theme;
use crate::theme::Role;
use crate::theme::SessionRowInk;

#[test]
fn tile_title_suffixes_use_showrunner_and_unit_role_colors() {
    let rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([
        {"agent":"claude","name":"production","pid":10,"started":10,"session_id":"boss","directory":"/fixture","children":[]},
        {"agent":"claude","name":"director","pid":20,"started":20,"session_id":"unit","directory":"/fixture","children":[]},
        {"agent":"claude","name":"worker","pid":30,"started":30,"session_id":"worker","directory":"/fixture","children":[]}
    ])).expect("agent rows");
    let edges = HierarchyEdges(vec![
        HierarchyEdge {
            manager:  "boss".into(),
            member:   "unit".into(),
            role:     HierarchyRole::Unit,
            reviewer: ReviewerState::NotPool,
        },
        HierarchyEdge {
            manager:  "unit".into(),
            member:   "worker".into(),
            role:     HierarchyRole::Impl,
            reviewer: ReviewerState::NotPool,
        },
    ]);
    let colors =
        FooterColors::parse("role.showrunner=bright-magenta\nrole.unit-director=bright-blue");
    for (name, word, role) in [
        ("production", "showrunner", FooterRole::Showrunner),
        ("director", "unit", FooterRole::UnitDirector),
    ] {
        let visible = rows
            .iter()
            .filter(|row| name == "production" || row.name != "production")
            .cloned()
            .collect();
        let state = MachineState::Answered(visible, edges.clone().into());
        let cells = Cells::new(
            vec![Machine {
                name:  "fixture",
                state: &state,
            }],
            100,
            &colors,
            &HashMap::new(),
            SummarySort::default(),
        );
        let tile = cells
            .tiles
            .iter()
            .find(|tile| matches!(tile,AgentTile::Agent(entry) if entry.row.name == name))
            .expect("role tile");
        let line = cells.group_title(&tile.key()).expect("tile title");
        let text = line.to_string();
        let start = summary::cell_width(&text[..text.find(word).expect("title role suffix")]);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 1));
        Paragraph::new(line).render(buffer.area, &mut buffer);
        let expected = colors
            .role(role)
            .style(Style::default())
            .fg
            .expect("role ink");
        for offset in 0..summary::cell_width(word) {
            assert_eq!(buffer[(start + offset, 0)].fg, expected, "{name}: {word}");
        }
    }
}

/// Full grid composition supplies the grounds; the test never paints one itself.
fn role_grid(colors: &FooterColors, boss_visible: bool) -> Buffer {
    account_grid(colors, boss_visible, &["claude1"])
}

fn account_grid(colors: &FooterColors, boss_visible: bool, labels: &[&str]) -> Buffer {
    const DRAWN_AT: u64 = 1_792_166_400;
    let read_at =
        chrono::DateTime::from_timestamp(i64::try_from(DRAWN_AT).expect("fixture time"), 0)
            .expect("fixture date")
            .to_rfc3339();
    let rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([
        {"agent":"claude","name":"production","pid":10,"started":10,"session_id":"boss","directory":"/fixture","children":[]},
        {"agent":"claude","name":"director","pid":20,"started":20,"session_id":"unit","directory":"/fixture","children":[]},
        {"agent":"claude","name":"worker","pid":30,"started":30,"session_id":"worker","directory":"/fixture","children":[]},
        {"agent":"claude","name":"unclaimed","pid":40,"started":40,"session_id":"free","directory":"/fixture","children":[]}
    ])).expect("grid rows");
    let edges = HierarchyEdges(vec![
        HierarchyEdge {
            manager:  "boss".into(),
            member:   "unit".into(),
            role:     HierarchyRole::Unit,
            reviewer: ReviewerState::NotPool,
        },
        HierarchyEdge {
            manager:  "unit".into(),
            member:   "worker".into(),
            role:     if boss_visible {
                HierarchyRole::Showrunner
            } else {
                HierarchyRole::Impl
            },
            reviewer: ReviewerState::NotPool,
        },
    ]);
    let state = MachineState::Answered(
        rows.into_iter()
            .filter(|row| boss_visible || row.name != "production")
            .collect(),
        edges.into(),
    );
    let accounts = AccountsTable::Ready(AccountsReading::parse(&serde_json::to_vec(&serde_json::json!({
        "accounts":labels.iter().map(|label| serde_json::json!({"label":label,"tool":"claude","roles":["showrunners","units"],"weekly_left_percent":50,"resets_at":null,"credits":2,"use_credits":true,"read_at":read_at})).collect::<Vec<_>>(),"problems":[]
    })).expect("account JSON")).expect("accounts"));
    let sorts = HashMap::new();
    let cells = Cells::new(
        vec![Machine {
            name:  "fixture",
            state: &state,
        }],
        DRAWN_AT,
        colors,
        &sorts,
        SummarySort::default(),
    )
    .with_accounts(&accounts);
    let mut app = App::new_for_test().expect("test app");
    let area = Rect::new(0, 0, 104, 34);
    let mut buffer = Buffer::empty(area);
    tui_pane::draw_tile_grid(
        &mut buffer,
        &mut app.tiles,
        area,
        app.loaded_config.config.tiles.growth(),
        TileGridContents::Shown,
        &cells,
    );
    app.tiles.settle_for_test();
    tui_pane::draw_tile_grid(
        &mut buffer,
        &mut app.tiles,
        area,
        app.loaded_config.config.tiles.growth(),
        TileGridContents::Shown,
        &cells,
    );
    buffer
}

#[test]
fn role_words_keep_readable_coverage_on_the_ground_composed_by_the_tile_grid() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let colors = FooterColors::parse(include_str!("../footer_colors/fixtures/statusline.conf"));
    let mut failures = Vec::new();
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins() {
        let name = variant.id.as_str();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            let mut saw_tree_role = false;
            for boss_visible in [true, false] {
                let buffer = role_grid(&colors, boss_visible);
                let mut surfaces = std::collections::HashSet::new();
                for y in 0..buffer.area.height {
                    let line: String = (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    for word in ["showrunner", "unit"] {
                        for (index, _) in line.match_indices(word) {
                            let x = summary::cell_width(&line[..index]);
                            let cell = &buffer[(x, y)];
                            let ratio = tests::contrast_ratio(cell.fg, cell.bg);
                            let covered = tests::covered_role_contrast(cell.fg, cell.bg);
                            eprintln!(
                                "{name} tint={tint} boss={boss_visible} {word} ({x},{y}): fg={:?} bg={:?} contrast={ratio:.6} covered={covered:.6}",
                                cell.fg, cell.bg
                            );
                            let surface = if line.contains("claude1") {
                                "accounts"
                            } else if line.starts_with('├') || line.starts_with('┌') {
                                "title"
                            } else {
                                "tree"
                            };
                            surfaces.insert(surface);
                            saw_tree_role |= surface == "tree";
                            for offset in 0..summary::cell_width(word) {
                                let drawn = &buffer[(x + offset, y)];
                                let ratio = tests::contrast_ratio(drawn.fg, drawn.bg);
                                let covered = tests::covered_role_contrast(drawn.fg, drawn.bg);
                                if ratio < 4.5 || covered < 4.5 {
                                    failures.push(format!("{name} tint={tint} {surface} {word} ({x},{y}) {drawn:?} contrast={ratio:.6} covered={covered:.6}"));
                                    break;
                                }
                            }
                        }
                    }
                }
                for surface in ["title", "accounts"] {
                    assert!(
                        surfaces.contains(surface),
                        "{surface} roles must draw: {buffer:?}"
                    );
                }
            }
            assert!(
                saw_tree_role,
                "{name} tint={tint} must draw management tree roles"
            );
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(
        failures.is_empty(),
        "drawn role contrast failures: {failures:#?}"
    );
}

#[test]
fn dark_title_accent_stays_distinct_and_readable_in_the_composed_grid() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let colors = FooterColors::parse(include_str!("../footer_colors/fixtures/statusline.conf"));
    let mut failures = Vec::new();
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins()
        .into_iter()
        .filter(|variant| variant.id.as_str().contains("Dark"))
    {
        let name = variant.id.as_str();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            let buffer = role_grid(&colors, true);
            let mut accent_cells = 0;
            for y in 0..buffer.area.height {
                let line: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                for title in ["summary", "unassigned"] {
                    let Some(index) = line.find(title) else {
                        continue;
                    };

                    let x = summary::cell_width(&line[..index]);
                    let cell = &buffer[(x, y)];
                    accent_cells += 1;
                    let ratio = tests::covered_role_contrast(cell.fg, cell.bg);
                    eprintln!(
                        "{name} tint={tint} title: fg={:?} bg={:?} contrast={ratio:.6}",
                        cell.fg, cell.bg
                    );
                    if ratio < 4.5 {
                        failures.push(format!("{name} tint={tint} title contrast={ratio:.6}"));
                    }
                    let entries = [
                        ("showrunner", colors.role(FooterRole::Showrunner)),
                        ("unit", colors.role(FooterRole::UnitDirector)),
                        ("astra-designer", FooterTint::Footer(FooterColor::Pink)),
                        (
                            "astra-reviewer",
                            FooterTint::Footer(FooterColor::BrightGreen),
                        ),
                        ("claude1", colors.account("claude1")),
                        ("claude2", colors.account("claude2")),
                        ("claude3", colors.account("claude3")),
                        ("claude4", colors.account("claude4")),
                        ("codex1", colors.account("codex1")),
                        ("codex2", colors.account("codex2")),
                    ];
                    for (label, ink) in entries {
                        let other = ink.style(Style::default()).fg.expect("fixture ink");
                        let distance = tests::perceptual_distance(cell.fg, other);
                        eprintln!("{name} tint={tint} title / {label}: CIEDE2000={distance:.6}");
                        if distance < 12.0 {
                            failures.push(format!(
                                "{name} tint={tint} title/{label} CIEDE2000={distance:.6}"
                            ));
                        }
                    }
                }
            }
            assert_eq!(accent_cells, 2, "summary and unassigned titles must draw");
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(
        failures.is_empty(),
        "drawn title accent failures: {failures:#?}"
    );
}

#[test]
fn status_line_labels_meet_contrast_on_the_drawn_bar_ground() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let app = App::new_for_test().expect("status app");
    let mut failures = Vec::new();
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins() {
        let name = variant.id.as_str();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(104, 1))
                .expect("status terminal");
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    draw_status_line(frame, &app, &app.keymap, area);
                })
                .expect("status draw");
            let buffer = terminal.backend().buffer();
            let line: String = (0..buffer.area.width)
                .map(|x| buffer[(x, 0)].symbol())
                .collect();
            let index = line.find(APP_NAME).expect("application status label");
            let start = summary::cell_width(&line[..index]);
            for offset in 0..summary::cell_width(APP_NAME) {
                let cell = &buffer[(start + offset, 0)];
                let ratio = tests::contrast_ratio(cell.fg, cell.bg);
                if offset == 0 {
                    eprintln!(
                        "{name} tint={tint} status label: fg={:?} bg={:?} contrast={ratio:.6}",
                        cell.fg, cell.bg
                    );
                }
                if ratio < 4.5 {
                    failures.push(format!("{name} tint={tint} {cell:?} contrast={ratio:.6}"));
                    break;
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(
        failures.is_empty(),
        "drawn status label contrast failures: {failures:#?}"
    );
}

#[test]
fn account_words_keep_readable_coverage_in_the_composed_grid() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let colors = FooterColors::parse(include_str!("../footer_colors/fixtures/statusline.conf"));
    let labels = [
        "claude1", "claude2", "claude3", "claude4", "codex1", "codex2",
    ];
    let mut failures = Vec::new();
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins() {
        let name = variant.id.as_str();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            let buffer = account_grid(&colors, true, &labels);
            for label in labels {
                let mut seen = false;
                for y in 0..buffer.area.height {
                    let line: String = (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    for (index, _) in line.match_indices(label) {
                        seen = true;
                        let x = summary::cell_width(&line[..index]);
                        let cell = &buffer[(x, y)];
                        let covered = tests::covered_role_contrast(cell.fg, cell.bg);
                        eprintln!(
                            "{name} tint={tint} {label}: fg={:?} bg={:?} covered={covered:.6}",
                            cell.fg, cell.bg
                        );
                        if covered < 4.5 {
                            failures
                                .push(format!("{name} tint={tint} {label} {cell:?}: {covered:.6}"));
                        }
                    }
                }
                assert!(seen, "account must draw: {label} {buffer:?}");
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(failures.is_empty(), "drawn account coverage: {failures:#?}");
}

/// Read the application label from the real status-line renderer.
fn drawn_status_label() -> Cell {
    let app = App::new_for_test().expect("status fixture");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(104, 1))
        .expect("status terminal");
    terminal
        .draw(|frame| {
            let area = frame.area();
            draw_status_line(frame, &app, &app.keymap, area);
        })
        .expect("status line");
    let status = terminal.backend().buffer();
    let line: String = (0..status.area.width)
        .map(|x| status[(x, 0)].symbol())
        .collect();
    let index = line.find(APP_NAME).expect("status application label");
    status[(summary::cell_width(&line[..index]), 0)].clone()
}

#[test]
fn light_title_accent_stays_distinct_from_session_role_and_account_inks() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let colors = FooterColors::parse(include_str!("../footer_colors/fixtures/statusline.conf"));
    let mut failures = Vec::new();
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins()
        .into_iter()
        .filter(|variant| variant.id.as_str().contains("Light"))
    {
        let name = variant.id.as_str();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            let buffer = role_grid(&colors, true);
            let mut surfaces = Vec::new();
            for y in 0..buffer.area.height {
                let line: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                for title in ["summary", "unassigned"] {
                    if let Some(index) = line.find(title) {
                        surfaces.push((
                            title,
                            buffer[(summary::cell_width(&line[..index]), y)].clone(),
                        ));
                    }
                }
            }
            assert_eq!(surfaces.len(), 2, "both chrome titles must draw");
            // With tint enabled shared borders are inactive: the interior tint shows focus.
            if !tint {
                let corner = &buffer[(0, 0)];
                assert_eq!(corner.symbol(), "┌", "focused summary border must draw");
                surfaces.push(("active border", corner.clone()));
            }
            surfaces.push(("status label", drawn_status_label()));
            for (surface, cell) in surfaces {
                let ratio = tests::contrast_ratio(cell.fg, cell.bg);
                eprintln!(
                    "{name} tint={tint} chrome {surface}: fg={:?} bg={:?} contrast={ratio:.6}",
                    cell.fg, cell.bg
                );
                if ratio < 4.5 {
                    failures.push(format!("{name} tint={tint} {surface} contrast={ratio}"));
                }
                let mut inks = vec![
                    (
                        "showrunner".to_owned(),
                        colors
                            .role(FooterRole::Showrunner)
                            .style(Style::default())
                            .fg
                            .expect("role"),
                    ),
                    (
                        "unit".to_owned(),
                        colors
                            .role(FooterRole::UnitDirector)
                            .style(Style::default())
                            .fg
                            .expect("role"),
                    ),
                ];
                for label in [
                    "claude1", "claude2", "claude3", "claude4", "codex1", "codex2",
                ] {
                    inks.push((
                        label.to_owned(),
                        colors
                            .account(label)
                            .style(Style::default())
                            .fg
                            .expect("account"),
                    ));
                }
                for ordinal in 0..5 {
                    let hue = theme::SessionHue::of_agent(ordinal);
                    inks.push((
                        format!("session {hue:?}"),
                        Role::Session(hue).style().fg.expect("session"),
                    ));
                }
                for (label, ink) in inks {
                    let distance = tests::perceptual_distance(cell.fg, ink);
                    eprintln!("{name} tint={tint} {surface}/{label} dE={distance:.6}");
                    if distance < 12.0 {
                        failures.push(format!(
                            "{name} tint={tint} {surface}/{label} dE={distance:.6}"
                        ));
                    }
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(failures.is_empty(), "light chrome: {failures:#?}");
}

#[test]
fn new_showrunner_and_unassigned_tiles_sort_names_and_first_click_reverses_them() {
    let mut app = App::new_for_test().expect("app");
    let rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([
        {"agent":"claude","name":"boss","pid":10,"started":10,"session_id":"boss","directory":"/fixture","children":[]},
        {"agent":"claude","name":"zeta-unit","pid":20,"started":20,"session_id":"zeta","directory":"/fixture","children":[]},
        {"agent":"claude","name":"alpha-unit","pid":30,"started":30,"session_id":"alpha","directory":"/fixture","children":[]},
        {"agent":"claude","name":"zeta-alone","pid":40,"started":40,"session_id":"zeta-alone","directory":"/fixture","children":[]},
        {"agent":"claude","name":"alpha-alone","pid":50,"started":50,"session_id":"alpha-alone","directory":"/fixture","children":[]}
    ])).expect("sort sessions");
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges(vec![
            HierarchyEdge {
                manager:  "boss".into(),
                member:   "zeta".into(),
                role:     HierarchyRole::Unit,
                reviewer: ReviewerState::NotPool,
            },
            HierarchyEdge {
                manager:  "boss".into(),
                member:   "alpha".into(),
                role:     HierarchyRole::Unit,
                reviewer: ReviewerState::NotPool,
            },
        ])
        .into(),
        stage: ScanStage::Completed,
    });
    let area = Rect::new(0, 0, 104, 16);
    let colors = FooterColors::default();
    let cells = Cells::new(
        app.census.machines(&[]),
        100,
        &colors,
        &app.tree_sorts,
        SummarySort::default(),
    );
    let keys: Vec<_> = cells.tiles.iter().map(AgentTile::key).collect();
    for key in keys {
        let cells = Cells::new(
            app.census.machines(&[]),
            100,
            &colors,
            &app.tree_sorts,
            SummarySort::default(),
        );
        let mut before = Buffer::empty(area);
        cells.draw(
            &mut before,
            &TileContent::Group(key.clone()),
            area,
            Color::Reset,
        );
        let (alpha, zeta) = if matches!(cells.tile(&key), Some(AgentTile::Unassigned(_))) {
            ("alpha-alone", "zeta-alone")
        } else {
            ("alpha-unit", "zeta-unit")
        };
        let position = |buffer: &Buffer, word: &str| {
            (0..buffer.area.height)
                .find_map(|y| {
                    let line: String = (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    line.find(word)
                        .map(|index| (summary::cell_width(&line[..index]), y))
                })
                .expect("drawn sort target")
        };
        assert!(
            position(&before, alpha).1 < position(&before, zeta).1,
            "default {key:?}: {before:?}"
        );
        let (x, y) = position(&before, "session");
        cell_clicked(
            &mut app,
            CellClick {
                index: 0,
                content: TileContent::Group(key.clone()),
                area,
                point: CellPoint {
                    column: x,
                    row:    y,
                },
            },
        );
        let cells = Cells::new(
            app.census.machines(&[]),
            100,
            &colors,
            &app.tree_sorts,
            SummarySort::default(),
        );
        let mut after = Buffer::empty(area);
        cells.draw(&mut after, &TileContent::Group(key), area, Color::Reset);
        assert!(
            position(&after, zeta).1 < position(&after, alpha).1,
            "first click reverses: {after:?}"
        );
    }
}

/// Fixture names are ASCII; search cell slices without rebuilding every buffer row.
fn session_ink_cell<'a>(buffer: &'a Buffer, word: &str) -> &'a Cell {
    buffer
        .content
        .chunks_exact(usize::from(buffer.area.width))
        .find_map(|row| {
            row.windows(word.len()).find(|cells| {
                cells
                    .iter()
                    .zip(word.bytes())
                    .all(|(cell, byte)| cell.symbol().as_bytes() == [byte])
            })
        })
        .map(|cells| &cells[0])
        .expect("session name draws")
}

/// Independent glyph oracle with literal 208/255 stem coverage.
fn session_glyph_contrast(foreground: Color, ground: Color) -> f64 {
    let rgb = |color| {
        let Color::Rgb(r, g, b) = tui_pane::blend_color(color, color, 0) else {
            panic!("explicit drawn color")
        };
        [r, g, b].map(f64::from)
    };
    let fg = rgb(foreground);
    let bg = rgb(ground);
    // Measure fractional and nearest-byte covered channels independently.
    let mixed =
        std::array::from_fn::<_, 3, _>(|i| f64::mul_add(bg[i], 47.0, fg[i] * 208.0) / 255.0);
    let luminance = |channels: [f64; 3]| {
        let [r, g, b] = channels.map(|c| {
            let v = c / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        });
        0.0722f64.mul_add(b, 0.7152f64.mul_add(g, 0.2126 * r))
    };
    let b = luminance(bg);
    [mixed, mixed.map(f64::round)]
        .map(|channels| {
            let a = luminance(channels);
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        })
        .into_iter()
        .fold(f64::INFINITY, f64::min)
}

#[derive(Clone, Copy)]
enum SessionResetFloor {
    DarkNotMeasured,
    HeldLight(f64),
}

/// Rounded down from the current final inks, with both pane-tint states measured.
#[derive(Clone, Copy)]
struct SessionPaletteFloors {
    consecutive_hue_gap: f64,
    session_distance:    f64,
    row_fixed_distance:  f64,
    reset_distance:      SessionResetFloor,
}

const DEFAULT_DARK_SESSION_FLOORS: SessionPaletteFloors = SessionPaletteFloors {
    consecutive_hue_gap: 14.9,
    session_distance:    14.9,
    row_fixed_distance:  12.2,
    reset_distance:      SessionResetFloor::DarkNotMeasured,
};
const HIGH_CONTRAST_DARK_SESSION_FLOORS: SessionPaletteFloors = SessionPaletteFloors {
    consecutive_hue_gap: 14.5,
    session_distance:    16.0,
    row_fixed_distance:  14.8,
    reset_distance:      SessionResetFloor::DarkNotMeasured,
};
// Hold the achieved reset distances; "Session colours stay apart" owns the full 20.
const DEFAULT_LIGHT_SESSION_FLOORS: SessionPaletteFloors = SessionPaletteFloors {
    consecutive_hue_gap: 60.0,
    session_distance:    12.0,
    row_fixed_distance:  15.0,
    reset_distance:      SessionResetFloor::HeldLight(14.7),
};
const HIGH_CONTRAST_LIGHT_SESSION_FLOORS: SessionPaletteFloors = SessionPaletteFloors {
    reset_distance: SessionResetFloor::HeldLight(15.3),
    ..DEFAULT_LIGHT_SESSION_FLOORS
};
// Both dark minima round down to 4.5; the required contrast floor stays 4.5:1.
const MINIMUM_SESSION_GLYPH_CONTRAST: f64 = 4.5;
const NEAR_HUE_GAP_DEGREES: f64 = 45.0;
const RESET_RAMP_POSITION_COUNT: usize = 256;

fn session_palette_floors(appearance: Appearance, name: &str) -> SessionPaletteFloors {
    match appearance {
        Appearance::Light => match name {
            "Default Light" => DEFAULT_LIGHT_SESSION_FLOORS,
            "High Contrast Light" => HIGH_CONTRAST_LIGHT_SESSION_FLOORS,
            _ => panic!("unmeasured light session palette {name}"),
        },
        Appearance::Dark => match name {
            "Default Dark" => DEFAULT_DARK_SESSION_FLOORS,
            "High Contrast Dark" => HIGH_CONTRAST_DARK_SESSION_FLOORS,
            _ => panic!("unmeasured dark session palette {name}"),
        },
    }
}

/// The pane treatment whose production inks are being measured.
#[derive(Clone, Copy, Debug)]
enum SessionPaneGround {
    Base,
    Tinted,
}

impl SessionPaneGround {
    fn activate(self) { tui_pane::set_focused_pane_tint(matches!(self, Self::Tinted)); }
}

struct SessionPaletteCheck<'a> {
    theme_name:   &'a str,
    floors:       SessionPaletteFloors,
    failures:     Vec<String>,
    measurements: BTreeMap<String, Vec<(String, Color, String, Color)>>,
    neighbors:    String,
    contrast:     Vec<(String, Color, Color, f64)>,
}

impl SessionPaletteCheck<'_> {
    fn check_reset_shades(&mut self, pane_ground: SessionPaneGround, minimum_distance: f64) {
        let resets: Vec<_> = theme::ResetRank::positions(RESET_RAMP_POSITION_COUNT)
            .map(|rank| {
                footer_colors::readable_status_style(Style::default().fg(rank.color()))
                    .fg
                    .expect("final reset foreground")
            })
            .collect();
        let first = theme::SessionHue::of_agent(0);
        let sessions = std::iter::once(first).chain(
            (1..)
                .map(theme::SessionHue::of_agent)
                .take_while(|hue| *hue != first),
        );
        for (ordinal, session) in sessions.enumerate() {
            let session_ink = Role::Session(session).style().fg.expect("session ink");
            for (rank, reset_ink) in resets.iter().enumerate() {
                let gap = tests::hue_gap(session_ink, *reset_ink);
                let distance = tests::perceptual_distance(session_ink, *reset_ink);
                self.measurements
                    .entry("session/reset".into())
                    .or_default()
                    .push((
                        format!("pane_ground={pane_ground:?} session-{ordinal}"),
                        session_ink,
                        format!("reset-{rank:03}"),
                        *reset_ink,
                    ));
                if gap <= NEAR_HUE_GAP_DEGREES && distance < minimum_distance {
                    self.failures.push(format!(
                        "{} pane_ground={pane_ground:?} session-{ordinal}/reset-{rank:03} {session_ink:?}/{reset_ink:?} dE={distance:.9} floor={minimum_distance} hue gap={gap:.9}",
                        self.theme_name
                    ));
                }
            }
        }
    }

    fn check_ink(
        &mut self,
        session_ink: Color,
        word: &str,
        palette: &[(SessionRowInk, Color)],
        pane_ground: SessionPaneGround,
    ) {
        let theme_name = self.theme_name;
        let oklch = tests::oklch_ink(session_ink);
        writeln!(
            self.neighbors,
            "pane_ground={pane_ground:?} {word} {session_ink:?} OKLCH C={:.9} L={:.9}",
            oklch.chroma, oklch.lightness
        )
        .expect("write current gamut measurement");
        for (neighbor, ink) in palette {
            let distance = tests::perceptual_distance(session_ink, *ink);
            self.measurements
                .entry("session/fixed".into())
                .or_default()
                .push((
                    format!("pane_ground={pane_ground:?} {word}"),
                    session_ink,
                    format!("{neighbor:?}"),
                    *ink,
                ));
            if distance < self.floors.row_fixed_distance {
                self.failures.push(format!(
                    "{theme_name} pane_ground={pane_ground:?} {word}/{neighbor:?} {session_ink:?}/{ink:?} dE={distance:.6} floor={}", self.floors.row_fixed_distance
                ));
            }
        }
    }

    fn check_pairs(
        &mut self,
        session_inks: BTreeMap<String, Color>,
        pane_ground: SessionPaneGround,
    ) {
        let theme_name = self.theme_name;
        let session_inks: Vec<_> = session_inks.into_iter().collect();
        assert_eq!(session_inks.len(), 5, "every session entry must draw");
        for (index, (first, first_ink)) in session_inks.iter().enumerate() {
            let next = &session_inks[(index + 1) % session_inks.len()];
            let gap = tests::hue_gap(*first_ink, next.1);
            self.measurements
                .entry("session/consecutive".into())
                .or_default()
                .push((
                    format!("pane_ground={pane_ground:?} {first}"),
                    *first_ink,
                    next.0.clone(),
                    next.1,
                ));
            if gap < self.floors.consecutive_hue_gap {
                self.failures.push(format!(
                    "{theme_name} pane_ground={pane_ground:?} consecutive {first}/{:?} hue gap={gap:.6}",
                    next.0
                ));
            }
            for (second, second_ink) in &session_inks[index + 1..] {
                let distance = tests::perceptual_distance(*first_ink, *second_ink);
                let gap = tests::hue_gap(*first_ink, *second_ink);
                let floor = self.floors.session_distance;
                self.measurements
                    .entry("session/session".into())
                    .or_default()
                    .push((
                        format!("pane_ground={pane_ground:?} {first}"),
                        *first_ink,
                        second.clone(),
                        *second_ink,
                    ));
                if distance < floor {
                    self.failures.push(format!("{theme_name} pane_ground={pane_ground:?} session {first}/{second} {first_ink:?}/{second_ink:?} dE={distance:.6} floor={floor} hue={gap:.6}"));
                }
            }
        }
    }

    fn check_drawn(
        &mut self,
        cells: &Cells<'_>,
        palette: &[(SessionRowInk, Color)],
        pane_ground: SessionPaneGround,
    ) {
        let theme_name = self.theme_name;
        // Distances depend on the inks, not the surfaces; measure each pair once per pane_ground.
        let mut measured = std::collections::HashSet::new();
        let mut session_inks = std::collections::BTreeMap::new();
        for focused in [false, true] {
            let area = Rect::new(0, 0, 160, 16);
            let ground = tui_pane::pane_background(focused);
            let mut summary = Buffer::empty(area);
            summary.set_style(area, Style::default().bg(ground));
            cells.draw(&mut summary, &TileContent::Summary, area, ground);
            for tile in &cells.tiles {
                let AgentTile::Agent(entry) = tile else {
                    panic!("all hue fixtures have work");
                };
                let session = &entry.row.name;
                let mut heading_buffer = Buffer::empty(area);
                heading_buffer.set_style(area, Style::default().bg(ground));
                Paragraph::new(cells.group_title(&tile.key()).expect("title"))
                    .render(area, &mut heading_buffer);
                let mut tree = Buffer::empty(area);
                tree.set_style(area, Style::default().bg(ground));
                cells.draw(&mut tree, &TileContent::Group(tile.key()), area, ground);
                let ordinal = session.strip_prefix("session-hue-").expect("ordinal");
                for (buffer, word, surface) in [
                    (&summary, session.as_str(), "summary"),
                    (&heading_buffer, session.as_str(), "title"),
                    (&tree, &format!("thread-hue-{ordinal}"), "tree"),
                ] {
                    let cell = session_ink_cell(buffer, word);
                    let ratio = session_glyph_contrast(cell.fg, cell.bg);
                    self.contrast.push((
                        format!("pane_ground={pane_ground:?} focused={focused} {surface}/{word}"),
                        cell.fg,
                        cell.bg,
                        ratio,
                    ));
                    if ratio < MINIMUM_SESSION_GLYPH_CONTRAST {
                        self.failures.push(format!("{theme_name} pane_ground={pane_ground:?} focused={focused} {surface}/{word} {cell:?} glyph={ratio:.6}"));
                    }
                    session_inks.entry(ordinal.to_owned()).or_insert(cell.fg);
                    if measured.insert((ordinal.to_owned(), cell.fg)) {
                        self.check_ink(cell.fg, word, palette, pane_ground);
                    }
                }
            }
        }
        self.check_pairs(session_inks, pane_ground);
    }
}

fn check_session_palette(variant_name: &str) {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let fixture = include_str!("../footer_colors/fixtures/statusline.conf");
    let colors = FooterColors::parse(fixture);
    let rows:Vec<AgentRow>=(0..5).map(|ordinal|serde_json::from_value(serde_json::json!({
        "agent":"claude","name":format!("session-hue-{ordinal}"),"pid":10+ordinal,"started":10,"directory":"/fixture",
        "children":[{"depth":0,"kind":"thread","name":format!("thread-hue-{ordinal}"),"started":20}]
    })).expect("session hue fixture")).collect();
    let state = MachineState::Answered(rows, HierarchyEdges::default().into());
    let variant = theme::builtins()
        .into_iter()
        .find(|v| v.id.as_str() == variant_name)
        .expect("builtin");
    let mut check = SessionPaletteCheck {
        theme_name:   variant_name,
        floors:       session_palette_floors(variant.appearance, variant_name),
        failures:     Vec::new(),
        measurements: std::collections::BTreeMap::new(),
        neighbors:    String::new(),
        contrast:     Vec::new(),
    };
    tui_pane::set_active_theme(Arc::new(variant.theme));
    tui_pane::set_transparent_background(false);
    for pane_ground in [SessionPaneGround::Base, SessionPaneGround::Tinted] {
        pane_ground.activate();
        let palette: Vec<_> = colors
            .session_row_inks()
            .map(|ink| (ink, ink.style().fg.expect("session row foreground")))
            .collect();
        writeln!(
            check.neighbors,
            "pane_ground={pane_ground:?} pane-active={:?} pane-inactive={:?}",
            tui_pane::pane_background(true),
            tui_pane::pane_background(false)
        )
        .expect("write pane measurements");
        for (neighbor, ink) in &palette {
            writeln!(
                check.neighbors,
                "pane_ground={pane_ground:?} {neighbor:?} {:?}",
                tests::terminal_rgb(*ink)
            )
            .expect("write row ink measurement");
        }
        let cells = Cells::new(
            vec![Machine {
                name:  "fixture",
                state: &state,
            }],
            100,
            &colors,
            &HashMap::new(),
            SummarySort::default(),
        );
        check.check_drawn(&cells, &palette, pane_ground);
        if let SessionResetFloor::HeldLight(minimum_distance) = check.floors.reset_distance {
            check.check_reset_shades(pane_ground, minimum_distance);
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    let mut report = format!("{variant_name} final production inks on the pane ground\n");
    for (class, pairs) in check.measurements {
        report.push_str(&tests::palette_pair_report(&class, &pairs));
    }
    let (surface, ink, ground, ratio) = check
        .contrast
        .iter()
        .min_by(|first, second| first.3.total_cmp(&second.3))
        .expect("every palette draws session glyphs");
    writeln!(
        report,
        "minimum glyph contrast={ratio:.9} {surface} {ink:?} on {ground:?}"
    )
    .expect("write contrast measurement");
    report.push_str(&check.neighbors);
    assert!(
        check.failures.is_empty(),
        "session palette: {:#?}; measurements: {report}",
        check.failures
    );
}

/// Holds 14.9° consecutive hue, session dE 14.9, row-fixed dE 12.2 and contrast 4.5.
/// The final 60°/session dE 20/row-fixed dE 15 rule is the open item "Dark session palette".
#[test]
fn session_name_inks_stay_apart_from_all_palettes_in_default_dark() {
    check_session_palette("Default Dark");
}
/// Holds session/reset dE 14.7; "Session colours stay apart" owns the full 20.
#[test]
fn session_name_inks_stay_apart_from_all_palettes_in_default_light() {
    check_session_palette("Default Light");
}
/// Holds 14.5° consecutive hue, session dE 16.0, row-fixed dE 14.8 and contrast 4.5.
/// The final 60°/session dE 20/row-fixed dE 15 rule is the open item "Dark session palette".
#[test]
fn session_name_inks_stay_apart_from_all_palettes_in_high_contrast_dark() {
    check_session_palette("High Contrast Dark");
}
/// Holds session/reset dE 15.3; "Session colours stay apart" owns the full 20.
#[test]
fn session_name_inks_stay_apart_from_all_palettes_in_high_contrast_light() {
    check_session_palette("High Contrast Light");
}

/// Holds API output as well as drawn rows: Default Dark floors are 14.9°/14.9/12.2;
/// High Contrast Dark floors are 14.5°/16.0/14.8; contrast stays 4.5 on both grounds.
/// The final 60°/session dE 20/row-fixed dE 15 rule is the open item "Dark session palette".
#[test]
fn dark_session_palette_api_keeps_current_floors_on_each_pane_ground() {
    let previous = tui_pane::theme();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let transparent = tui_pane::transparent_background();
    let colors = FooterColors::parse(include_str!("../footer_colors/fixtures/statusline.conf"));
    let mut failures = Vec::new();
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins() {
        if variant.appearance == Appearance::Light {
            continue;
        }
        let name = variant.id.as_str();
        let mut check = SessionPaletteCheck {
            theme_name:   name,
            floors:       session_palette_floors(variant.appearance, name),
            failures:     Vec::new(),
            measurements: BTreeMap::new(),
            neighbors:    String::new(),
            contrast:     Vec::new(),
        };
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for pane_ground in [SessionPaneGround::Base, SessionPaneGround::Tinted] {
            pane_ground.activate();
            let palette: Vec<_> = colors
                .session_row_inks()
                .map(|ink| (ink, ink.style().fg.expect("row ink")))
                .collect();
            let mut session_inks = BTreeMap::new();
            for ordinal in 0..5 {
                let hue = theme::SessionHue::of_agent(ordinal);
                let ink = Role::Session(hue).style().fg.expect("session foreground");
                let word = ordinal.to_string();
                check.check_ink(ink, &word, &palette, pane_ground);
                session_inks.insert(word, ink);
                for focused in [false, true] {
                    let ground = tui_pane::pane_background(focused);
                    let ratio = session_glyph_contrast(ink, ground);
                    if ratio < MINIMUM_SESSION_GLYPH_CONTRAST {
                        check.failures.push(format!(
                            "{name} pane_ground={pane_ground:?} focused={focused} {hue:?} {ink:?} on {ground:?} contrast={ratio:.9}"
                        ));
                    }
                }
            }
            check.check_pairs(session_inks, pane_ground);
        }
        failures.extend(check.failures);
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_focused_pane_tint(tinted);
    tui_pane::set_transparent_background(transparent);
    assert!(failures.is_empty(), "current dark palette: {failures:#?}");
}

#[test]
fn root_session_colours_ignore_subtree_sizes_and_match_titles_after_both_sorts() {
    let previous = tui_pane::theme();
    let state = roots_with_different_subtree_sizes();
    let colors = FooterColors::parse(include_str!("../footer_colors/fixtures/statusline.conf"));
    let mut failures = Vec::new();
    for variant in theme::builtins() {
        let theme_name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for direction in [SortDirection::Ascending, SortDirection::Descending] {
            let mut sorts = HashMap::new();
            let first = Cells::new(
                vec![Machine {
                    name:  "fixture",
                    state: &state,
                }],
                100,
                &colors,
                &sorts,
                SummarySort::default(),
            );
            let keys: Vec<_> = first.tiles.iter().map(AgentTile::key).collect();
            drop(first);
            for key in keys {
                sorts.insert(
                    key,
                    TreeSort {
                        column: TreeColumn::Session,
                        direction,
                    },
                );
            }
            let cells = Cells::new(
                vec![Machine {
                    name:  "fixture",
                    state: &state,
                }],
                100,
                &colors,
                &sorts,
                SummarySort {
                    column: SummaryColumn::Name,
                    direction,
                },
            );
            let area = Rect::new(0, 0, 200, 48);
            let ground = tui_pane::pane_background(false);
            let mut summary = Buffer::empty(area);
            summary.set_style(area, Style::default().bg(ground));
            cells.draw(&mut summary, &TileContent::Summary, area, ground);
            let mut distinct = std::collections::HashSet::new();
            for (ordinal, entry) in cells.agents.iter().enumerate() {
                let expected = Role::Session(theme::SessionHue::of_agent(ordinal))
                    .style()
                    .fg
                    .expect("ordinal ink");
                let summary_ink = session_ink_cell(&summary, &entry.row.name).fg;
                if summary_ink != expected {
                    failures.push(format!("{theme_name} {direction:?} {} root{ordinal} expected{expected:?} drawn{summary_ink:?}", entry.row.name));
                }
                distinct.insert(summary_ink);
                let tile = cells.tiles.iter().find(|tile| matches!(tile, AgentTile::Agent(candidate) if candidate.row.pid == entry.row.pid)).expect("root tile");
                let mut heading_buffer = Buffer::empty(area);
                Paragraph::new(cells.group_title(&tile.key()).expect("root title"))
                    .render(area, &mut heading_buffer);
                assert_eq!(
                    session_ink_cell(&heading_buffer, &entry.row.name).fg,
                    summary_ink,
                    "tile and summary agree after sorting"
                );
                let mut tree = Buffer::empty(area);
                cells.draw(&mut tree, &TileContent::Group(tile.key()), area, ground);
                assert_root_tree_sort(&tree, ordinal, direction);
            }
            if distinct.len() != 5 {
                failures.push(format!(
                    "{theme_name} {direction:?}: five roots use {} inks",
                    distinct.len()
                ));
            }
        }
    }
    tui_pane::set_active_theme(previous);
    assert!(failures.is_empty(), "independent root hues: {failures:#?}");
}

fn assert_root_tree_sort(tree: &Buffer, ordinal: usize, direction: SortDirection) {
    let last_child = ordinal;
    let first_name = format!("member-{ordinal}-0");
    let last_name = format!("member-{ordinal}-{last_child}");
    let first_cell = session_ink_cell(tree, &first_name);
    let last_cell = session_ink_cell(tree, &last_name);
    let first_row = tree
        .content
        .iter()
        .position(|cell| std::ptr::eq(cell, first_cell))
        .expect("first member")
        / usize::from(tree.area.width);
    let last_row = tree
        .content
        .iter()
        .position(|cell| std::ptr::eq(cell, last_cell))
        .expect("last member")
        / usize::from(tree.area.width);
    if last_child > 0 {
        match direction {
            SortDirection::Ascending => {
                assert!(first_row < last_row, "tree name sort ascending");
            },
            SortDirection::Descending => {
                assert!(last_row < first_row, "tree name sort descending");
            },
        }
    }
}

fn roots_with_different_subtree_sizes() -> MachineState {
    let mut rows = Vec::new();
    let mut edges = Vec::new();
    for root in 0..5 {
        let id = format!("root-{root}");
        rows.push(
            serde_json::from_value(serde_json::json!({
                "agent":"claude", "name":id, "pid":100+root*10, "started":100-root,
                "session_id":id, "directory":"/fixture", "children":[]
            }))
            .expect("root row"),
        );
        for child in 0..=root {
            let member = format!("member-{root}-{child}");
            rows.push(
                serde_json::from_value(serde_json::json!({
                    "agent":"claude", "name":member, "pid":101+root*10+child, "started":10+child,
                    "session_id":member, "directory":"/fixture", "children":[]
                }))
                .expect("member row"),
            );
            edges.push(HierarchyEdge {
                manager: id.clone(),
                member,
                role: HierarchyRole::Unit,
                reviewer: ReviewerState::NotPool,
            });
        }
    }
    MachineState::Answered(rows, HierarchyEdges(edges).into())
}

#[test]
fn light_session_inks_keep_saturated_chroma_on_the_final_output() {
    let previous = tui_pane::theme();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let transparent = tui_pane::transparent_background();
    let mut failures = Vec::new();
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins() {
        if variant.appearance != Appearance::Light {
            continue;
        }
        let name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            for ordinal in 0..5 {
                let hue = theme::SessionHue::of_agent(ordinal);
                let ink = Role::Session(hue).style().fg.expect("session ink");
                let chroma = tests::oklch_ink(ink).chroma;
                if chroma < 0.10 {
                    failures.push(format!(
                        "{name} tint={tint} {hue:?} {ink:?} chroma={chroma:.9} floor=0.10"
                    ));
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_focused_pane_tint(tinted);
    tui_pane::set_transparent_background(transparent);
    assert!(failures.is_empty(), "light session chroma: {failures:#?}");
}
