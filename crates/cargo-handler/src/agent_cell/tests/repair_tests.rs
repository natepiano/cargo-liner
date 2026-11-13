//! Shared tree layout and specialist role ink remain consistent across tiles.

use super::*;
use crate::census::ShellTimerObservation;
use crate::census::accounts::EMPTY_SESSION_STATES;
use crate::census::transcript::SubagentRole;
use crate::footer_colors::FooterRole;
use crate::sort::SortDirection;

#[test]
fn mixed_heading_lengths_keep_one_column_set_and_positions() {
    let mut long = agent(10, "long", HOUR, None);
    long.directory = "/fixture/".repeat(24);
    long.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(11),
        "cargo test",
        MINUTE,
    ));
    let mut short = agent(20, "short", HOUR, None);
    short.directory = "/fixture".into();
    short.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(21),
        "cargo build",
        MINUTE,
    ));
    let cells = [entry(&long, None), entry(&short, None)];
    let buffers: Vec<_> = cells
        .iter()
        .map(|cell| {
            let rect = area(160, 20);
            let mut buffer = Buffer::empty(rect);
            draw(
                &mut buffer,
                rect,
                cell,
                &cells,
                NOW,
                AccountColumn::Shown,
                &FooterColors::default(),
            );
            buffer
        })
        .collect();
    for label in [
        "session",
        "type",
        "role",
        "status",
        "account",
        "branch",
        "directory",
        "age",
    ] {
        let positions: Vec<_> = buffers
            .iter()
            .map(|buffer| find(buffer, label).map(|(x, _)| x))
            .collect();
        assert_eq!(
            positions[0], positions[1],
            "{label}: both tiles use the same columns"
        );
    }
    assert!(find(&buffers[0], "cargo test").is_some());
    assert!(find(&buffers[1], "cargo build").is_some());
}

#[test]
fn specialist_management_roles_use_footer_ink_and_other_roles_stay_plain() {
    let colors = FooterColors::parse("role.showrunner=purple\nrole.unit-director=yellow");
    for (kind, role) in [
        (SubagentRole::Showrunner, Some(FooterRole::Showrunner)),
        (SubagentRole::Agent, None),
        (SubagentRole::Capture, None),
    ] {
        let mut root = agent(10, "owner", HOUR, None);
        let mut work = child(
            0,
            ChildKind::Subagent(SubagentRole::Unrecorded),
            None,
            "specialist job",
            MINUTE,
        );
        work.kind = ChildKind::Subagent(kind.clone());
        root.children.push(work);
        let cell = entry(&root, None);
        for width in [80, 220] {
            let rect = area(width, 20);
            let mut buffer = Buffer::empty(rect);
            draw(
                &mut buffer,
                rect,
                &cell,
                std::slice::from_ref(&cell),
                NOW,
                AccountColumn::Shown,
                &colors,
            );
            let x = find(&buffer, "activity").expect("activity header").0;
            let y = find(&buffer, "specialist job").expect("specialist row").1;
            let expected = role
                .map_or(FooterTint::Plain, |role| colors.role(role))
                .style(Style::default().fg(text_default()))
                .fg
                .expect("role foreground");
            for offset in 0..summary::cell_width(kind.text()) {
                assert_eq!(buffer[(x + offset, y)].fg, expected, "{kind:?} at {width}");
            }
        }
    }
}

#[test]
fn unassigned_sessions_without_a_role_draw_no_role_column() {
    let state = MachineState::Answered(
        vec![agent(10, "postmaster", HOUR, None)],
        HierarchyEdges::default().into(),
    );
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    let pool = tiles
        .iter()
        .find_map(|tile| match tile {
            AgentTile::Unassigned(pool) => Some(pool),
            AgentTile::Agent(_) => None,
        })
        .expect("unassigned pool");
    let rect = area(160, 10);
    let mut buffer = Buffer::empty(rect);
    draw_unassigned(
        &mut buffer,
        rect,
        pool,
        NOW,
        AccountColumn::Shown,
        &FooterColors::default(),
    );
    assert!(
        find(&buffer, "postmaster").is_some(),
        "the unassigned row stays visible"
    );
    assert!(
        find(&buffer, "role").is_some(),
        "an unlabeled row retains an empty role cell"
    );
    assert!(
        find(&buffer, "type").is_none(),
        "a session has no work type"
    );
}

#[test]
fn older_tiles_measure_their_own_age_labels_beside_younger_tiles() {
    let mut old = agent(10, "old-owner", 30 * 24 * HOUR, None);
    old.children = vec![
        child(
            0,
            ChildKind::Thread,
            None,
            "week-old",
            7 * 24 * HOUR + 21 * HOUR,
        ),
        child(0, ChildKind::Thread, None, "twenty-days", 20 * 24 * HOUR),
    ];
    let mut young = agent(20, "young-owner", HOUR, None);
    // A different visible column set keeps this tile in its own sizing group.
    young.account = serde_json::from_value(serde_json::json!({
        "home": "~/.claude1", "label": "claude1"
    }))
    .expect("recorded account");
    young.children = vec![
        child(0, ChildKind::Thread, None, "minutes-old", 2 * MINUTE + 49),
        child(
            0,
            ChildKind::Thread,
            None,
            "hours-old",
            5 * HOUR + 16 * MINUTE,
        ),
    ];
    for child in &mut young.children {
        child.account = young.account.clone();
    }
    let mut cells = [entry(&old, None), entry(&young, None)];
    for cell in &mut cells {
        cell.sort = TreeSort {
            column:    TreeColumn::Session,
            direction: SortDirection::Ascending,
        };
    }
    for (index, labels, expected) in [
        (
            0,
            ["week-old", "twenty-days"],
            ["7 days, 21 hours, 0 minutes", "20 days, 0 hours, 0 minutes"],
        ),
        (
            1,
            ["minutes-old", "hours-old"],
            ["2 minutes", "5 hours, 16 minutes"],
        ),
    ] {
        let rect = area(220, 20);
        let mut buffer = Buffer::empty(rect);
        draw(
            &mut buffer,
            rect,
            &cells[index],
            &cells,
            NOW,
            AccountColumn::Shown,
            &FooterColors::default(),
        );
        assert!(find(&buffer, "age").is_none());
        for (label, expected) in labels.into_iter().zip(expected) {
            assert_eq!(detail_value(&cells[index], label, "age", NOW), expected);
            assert!(find(&buffer, label).is_some());
        }
    }
}

#[test]
fn narrow_tree_rows_keep_every_ancestor_guide() {
    let owner = AgentRow {
        children: (0..=GUIDE_TEST_DEPTH)
            .map(|depth| child(depth, ChildKind::Thread, None, "Dispatch-worker", MINUTE))
            .collect(),
        ..agent(10, "director", HOUR, None)
    };
    let cell = entry(&owner, None);
    for width in GUIDE_TEST_WIDTHS {
        let buffer = drawn_into(&cell, &[], width, usize::from(REPAIR_TILE_HEIGHT));
        let header_y = header_spans(&cell, buffer.area, NOW, AccountColumn::Hidden)
            .first()
            .expect("tree header")
            .row;
        for (index, line) in cell.lines.iter().enumerate() {
            let y = header_y + 1 + u16::try_from(index).expect("row index");
            let actual: String = (header_indented(buffer.area).x..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            assert!(
                actual.starts_with(&line.lead()),
                "width={width}, row={index}: {actual:?}"
            );
        }
    }
}

#[test]
fn sparse_type_columns_give_space_to_distinct_session_names() {
    let owner = population_owner(10, SPARSE_TYPED_ROWS);
    let cell = entry(&owner, None);
    for width in SPARSE_TEST_WIDTHS {
        let buffer = drawn_into(&cell, &[], width, usize::from(REPAIR_TILE_HEIGHT));
        let text = lines(&buffer).join("\n");
        assert!(find(&buffer, "report").is_some(), "width={width}: {text}");
        assert!(find(&buffer, "resou").is_some(), "width={width}: {text}");
        if width >= COMPLETE_NAME_WIDTH {
            assert!(
                find(&buffer, "resources").is_some(),
                "width={width}: {text}"
            );
        }
    }
}

const GUIDE_TEST_WIDTHS: [u16; 4] = [24, 32, 50, 120];
const SPARSE_TEST_WIDTHS: [u16; 4] = [36, 40, 50, 120];
const COMPLETE_NAME_WIDTH: u16 = 50;
const MEASUREMENT_WIDTH: u16 = 120;
const STACKED_TILE_WIDTH: u16 = 44;
const REPAIR_TILE_HEIGHT: u16 = 40;
const POPULATION_ROWS: usize = 19;
const SPARSE_TYPED_ROWS: usize = 2;
const GUIDE_TEST_DEPTH: u8 = 3;

fn population_owner(pid: u32, typed_rows: usize) -> AgentRow {
    let mut owner = agent(pid, "director", HOUR, None);
    owner.children = (0..POPULATION_ROWS)
        .map(|index| {
            child(
                0,
                if index < typed_rows {
                    ChildKind::Shell(ShellTimerObservation::Untimed)
                } else {
                    ChildKind::Thread
                },
                None,
                if index % 2 == 0 {
                    "report"
                } else {
                    "resources"
                },
                MINUTE,
            )
        })
        .collect();
    owner
}

#[test]
fn stacked_sparse_tiles_keep_name_space_beside_dense_tiles() {
    let sparse = population_owner(10, SPARSE_TYPED_ROWS);
    let dense = population_owner(20, POPULATION_ROWS - 1);
    let cells = [entry(&sparse, None), entry(&dense, None)];
    let measurements = cells.iter().enumerate().map(|(index, cell)| {
        let mut rect = area(STACKED_TILE_WIDTH, usize::from(REPAIR_TILE_HEIGHT));
        rect.y = u16::try_from(index).expect("tile index") * REPAIR_TILE_HEIGHT;
        TreeMeasurement::for_entry(cell, rect, NOW, AccountColumn::Hidden)
    });
    for (cell, metrics) in cells.iter().zip(shared_tree_metrics(measurements)) {
        *cell.sizing.borrow_mut() = TreeSizing::Shared(metrics);
    }
    let sparse_buffer = drawn_into(
        &cells[0],
        &cells,
        STACKED_TILE_WIDTH,
        usize::from(REPAIR_TILE_HEIGHT),
    );
    let dense_buffer = drawn_into(
        &cells[1],
        &cells,
        STACKED_TILE_WIDTH,
        usize::from(REPAIR_TILE_HEIGHT),
    );
    assert!(
        find(&sparse_buffer, "report").is_some(),
        "{:?}",
        lines(&sparse_buffer)
    );
    assert!(
        find(&sparse_buffer, "resources").is_some(),
        "{:?}",
        lines(&sparse_buffer)
    );
    assert!(
        find(&sparse_buffer, "type").is_none(),
        "the sparse tile gives Type space to names"
    );
    assert!(
        find(&dense_buffer, "type").is_none(),
        "the dense tile removes Type before cutting names"
    );
    assert!(find(&dense_buffer, "resources").is_some());
}

#[test]
fn minimum_tile_width_reserves_depth_three_guides_and_status() {
    let shallow = agent(10, "director", HOUR, None);
    let mut deep = shallow.clone();
    deep.children = (0..=GUIDE_TEST_DEPTH)
        .map(|depth| child(depth, ChildKind::Thread, None, "Dispatch-worker", MINUTE))
        .collect();
    let mut shallow = shallow;
    shallow.children = (0..=GUIDE_TEST_DEPTH)
        .map(|_| child(0, ChildKind::Thread, None, "Dispatch-worker", MINUTE))
        .collect();
    let shallow_cell = entry(&shallow, None);
    let deep_cell = entry(&deep, None);
    let measure = |cell: &AgentEntry<'_>| {
        TreeMeasurement::measure(
            &cell.lines,
            area(MEASUREMENT_WIDTH, usize::from(REPAIR_TILE_HEIGHT)),
            NOW,
            cell.sort,
            AccountColumn::Hidden,
            SessionHeaderLead::Flush,
            TreeHeadingSpace {
                offset: 0,
                width:  0,
                rows:   PresentationValues {
                    full:       0,
                    compressed: 0,
                },
            },
        )
    };
    let minimum = measure(&deep_cell).minimum_tile_width();
    assert!(
        minimum > measure(&shallow_cell).minimum_tile_width(),
        "deeper guides reserve more cells"
    );
    let buffer = drawn_into(&deep_cell, &[], minimum, usize::from(REPAIR_TILE_HEIGHT));
    let text = lines(&buffer).join("\n");
    assert!(text.contains(&deep_cell.lines[0].lead()), "{text}");
    assert!(find(&buffer, "status").is_some(), "{text}");
    assert_eq!(
        detail_value(&deep_cell, "Dispatch-worker", "state", NOW),
        "idle"
    );
}
