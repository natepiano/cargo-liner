//! Tile title role suffixes use the same footer palette as tree role words.

#![expect(
    clippy::expect_used,
    reason = "fixture tiles and role suffixes must exist"
)]

use std::sync::Arc;

use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use super::*;
use crate::census::AgentRow;
use crate::census::HierarchyEdge;
use crate::census::HierarchyEdges;
use crate::census::HierarchyRole;
use crate::census::MachineState;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::hierarchy::ReviewerState;
use crate::footer_colors::FooterColor;
use crate::footer_colors::FooterRole;
use crate::footer_colors::FooterTint;
use crate::footer_colors::tests;
use crate::theme;

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
        "accounts":[{"label":"claude1","tool":"claude","roles":["showrunners","units"],"weekly_left_percent":50,"resets_at":null,"credits":2,"use_credits":true,"read_at":read_at}],"problems":[]
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
                    let ratio = tests::contrast_ratio(cell.fg, cell.bg);
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
