//! Tile title role suffixes use the same footer palette as tree role words.

#![expect(
    clippy::expect_used,
    reason = "fixture tiles and role suffixes must exist"
)]
#![expect(clippy::panic, reason = "every hue fixture must have its own tile")]

use std::sync::Arc;

use ratatui::buffer::Cell;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
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
use crate::footer_colors::FooterColor;
use crate::footer_colors::FooterRole;
use crate::footer_colors::FooterTint;
use crate::footer_colors::tests;
use crate::summary;
use crate::theme;
use crate::theme::Role;

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
    // Quantize covered glyph channels to the terminal's eight-bit RGB before luminance.
    let mixed = std::array::from_fn::<_, 3, _>(|i| {
        (f64::mul_add(bg[i], 47.0, fg[i] * 208.0) / 255.0).floor()
    });
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
    let a = luminance(mixed);
    let b = luminance(bg);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[derive(Clone, Copy, Debug)]
enum SessionNeighbor {
    StatusOrAccount,
    Role,
}

impl SessionNeighbor {
    const fn minimum_distance(self) -> f64 {
        match self {
            Self::StatusOrAccount => 12.0,
            Self::Role => 10.0,
        }
    }
}

fn session_comparison_palette(
    colors: &FooterColors,
    fixture: &str,
) -> Vec<(SessionNeighbor, String, Color)> {
    let mut palette = Vec::new();
    for line in fixture.lines() {
        let Some((key, _)) = line.split_once('=') else {
            continue;
        };
        if let Some(label) = key.strip_prefix("account.") {
            palette.push((
                SessionNeighbor::StatusOrAccount,
                label.to_owned(),
                colors
                    .account(label)
                    .style(Style::default())
                    .fg
                    .expect("account ink"),
            ));
        }
    }
    for role in [
        HierarchyRole::Showrunner,
        HierarchyRole::Unit,
        HierarchyRole::Impl,
        HierarchyRole::Test,
        HierarchyRole::Review,
        HierarchyRole::Fix,
    ] {
        palette.push((
            SessionNeighbor::Role,
            format!("{role:?}"),
            colors
                .hierarchy_role(role)
                .style(Style::default())
                .fg
                .unwrap_or_else(tui_pane::text_default),
        ));
    }
    for role in [
        Role::Claude,
        Role::Codex,
        Role::Busy,
        Role::Shell,
        Role::Idle,
        Role::Unreachable,
    ] {
        palette.push((
            SessionNeighbor::StatusOrAccount,
            format!("status {role:?}"),
            role.style().fg.expect("status ink"),
        ));
    }
    palette
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
    let sorts = HashMap::new();
    let mut failures = Vec::new();
    let variant = theme::builtins()
        .into_iter()
        .find(|v| v.id.as_str() == variant_name)
        .expect("builtin");
    tui_pane::set_active_theme(Arc::new(variant.theme));
    tui_pane::set_transparent_background(false);
    for tint in [false, true] {
        tui_pane::set_focused_pane_tint(tint);
        let palette = session_comparison_palette(&colors, fixture);
        let cells = Cells::new(
            vec![Machine {
                name:  "fixture",
                state: &state,
            }],
            100,
            &colors,
            &sorts,
            SummarySort::default(),
        );
        // Distances depend on the inks, not the surfaces; measure each pair once per tint.
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
                let mut title = Buffer::empty(area);
                title.set_style(area, Style::default().bg(ground));
                Paragraph::new(cells.group_title(&tile.key()).expect("title"))
                    .render(area, &mut title);
                let mut tree = Buffer::empty(area);
                tree.set_style(area, Style::default().bg(ground));
                cells.draw(&mut tree, &TileContent::Group(tile.key()), area, ground);
                let ordinal = session.strip_prefix("session-hue-").expect("ordinal");
                for (buffer, word, surface) in [
                    (&summary, session.as_str(), "summary"),
                    (&title, session.as_str(), "title"),
                    (&tree, &format!("thread-hue-{ordinal}"), "tree"),
                ] {
                    let cell = session_ink_cell(buffer, word);
                    let ratio = session_glyph_contrast(cell.fg, cell.bg);
                    if ratio < 4.5 {
                        failures.push(format!("{variant_name} tint={tint} focused={focused} {surface}/{word} {cell:?} glyph={ratio:.6}"));
                    }
                    session_inks.entry(ordinal.to_owned()).or_insert(cell.fg);
                    if measured.insert((ordinal.to_owned(), cell.fg)) {
                        for (neighbor, label, ink) in &palette {
                            let distance = tests::perceptual_distance(cell.fg, *ink);
                            if distance < neighbor.minimum_distance() {
                                failures.push(format!("{variant_name} tint={tint} {word}/{label} {:?}/{ink:?} dE={distance:.6}",cell.fg));
                            }
                        }
                    }
                }
            }
        }
        let session_inks: Vec<_> = session_inks.into_iter().collect();
        assert_eq!(session_inks.len(), 5, "every session entry must draw");
        for (index, (first, first_ink)) in session_inks.iter().enumerate() {
            for (second, second_ink) in &session_inks[index + 1..] {
                let distance = tests::perceptual_distance(*first_ink, *second_ink);
                if distance < 12.0 {
                    failures.push(format!("{variant_name} tint={tint} session {first}/{second} {first_ink:?}/{second_ink:?} dE={distance:.6}"));
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(failures.is_empty(), "session palette: {failures:#?}");
}

#[test]
fn session_name_inks_stay_apart_from_all_palettes_in_default_dark() {
    check_session_palette("Default Dark");
}
#[test]
fn session_name_inks_stay_apart_from_all_palettes_in_default_light() {
    check_session_palette("Default Light");
}
#[test]
fn session_name_inks_stay_apart_from_all_palettes_in_high_contrast_dark() {
    check_session_palette("High Contrast Dark");
}
#[test]
fn session_name_inks_stay_apart_from_all_palettes_in_high_contrast_light() {
    check_session_palette("High Contrast Light");
}

/// Independent sRGB -> Oklab conversion; chroma is hypot(a, b).
fn session_oklch_chroma(color: Color) -> f64 {
    let Color::Rgb(red, green, blue) = color else {
        panic!("session palette must expose explicit sRGB: {color:?}");
    };
    let [red, green, blue] = [red, green, blue].map(|channel| {
        let encoded = f64::from(channel) / 255.0;
        if encoded <= 0.040_45 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        }
    });
    let long_response = 0.051_445_992_9f64
        .mul_add(
            blue,
            0.536_332_536_3f64.mul_add(green, 0.412_221_470_8 * red),
        )
        .cbrt();
    let medium_response = 0.107_396_956_6f64
        .mul_add(
            blue,
            0.680_699_545_1f64.mul_add(green, 0.211_903_498_2 * red),
        )
        .cbrt();
    let short_response = 0.629_978_700_5f64
        .mul_add(
            blue,
            0.281_718_837_6f64.mul_add(green, 0.088_302_461_9 * red),
        )
        .cbrt();
    let a = 0.450_593_709_9f64.mul_add(
        short_response,
        2.428_592_205f64.mul_add(-medium_response, 1.977_998_495_1 * long_response),
    );
    let b = 0.808_675_766f64.mul_add(
        -short_response,
        0.782_771_766_2f64.mul_add(medium_response, 0.025_904_037_1 * long_response),
    );
    a.hypot(b)
}

#[test]
fn every_theme_session_entry_retains_saturated_oklch_chroma() {
    let previous = tui_pane::theme();
    let mut failures = Vec::new();
    for variant in theme::builtins() {
        let name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for ordinal in 0..5 {
            let hue = theme::SessionHue::of_agent(ordinal);
            let ink = Role::Session(hue).style().fg.expect("session foreground");
            let chroma = session_oklch_chroma(ink);
            if chroma < 0.10 {
                failures.push(format!("{name} {hue:?} {ink:?} OKLCH C={chroma:.9}"));
            }
        }
    }
    tui_pane::set_active_theme(previous);
    assert!(
        failures.is_empty(),
        "saturated session palette: {failures:#?}"
    );
}
