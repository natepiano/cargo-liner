//! Shared tree layout and specialist role ink remain consistent across tiles.

use super::*;
use crate::census::transcript::SubagentRole;
use crate::footer_colors::FooterRole;
use crate::sort::SortDirection;

#[test]
fn mixed_heading_lengths_keep_one_column_set_and_positions() {
    let mut long = agent(10, "long", HOUR, None);
    long.directory = "/fixture/".repeat(24);
    long.children
        .push(child(0, ChildKind::Shell, Some(11), "cargo test", MINUTE));
    let mut short = agent(20, "short", HOUR, None);
    short.directory = "/fixture".into();
    short
        .children
        .push(child(0, ChildKind::Shell, Some(21), "cargo build", MINUTE));
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
        ("showrunner", Some(FooterRole::Showrunner)),
        ("unit-director", Some(FooterRole::UnitDirector)),
        ("capture", None),
    ] {
        let mut root = agent(10, "owner", HOUR, None);
        let mut work = child(
            0,
            ChildKind::Subagent(SubagentRole::Unrecorded),
            None,
            "specialist job",
            MINUTE,
        );
        work.kind = ChildKind::Subagent(SubagentRole::from_record(kind, ""));
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
            let x = find(&buffer, "role").expect("role header").0;
            let y = find(&buffer, "specialist job").expect("specialist row").1;
            let expected = role
                .map_or(FooterTint::Plain, |role| colors.role(role))
                .style(Style::default().fg(text_default()))
                .fg
                .expect("role foreground");
            for offset in 0..summary::cell_width(kind) {
                assert_eq!(buffer[(x + offset, y)].fg, expected, "{kind} at {width}");
            }
        }
    }
}

#[test]
fn unassigned_sessions_without_a_role_draw_an_empty_role_cell() {
    let state = MachineState::Answered(
        vec![agent(10, "postmaster", HOUR, None)],
        HierarchyEdges::default().into(),
    );
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
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
    let (x, _) = find(&buffer, "role").expect("role header");
    let (_, y) = find(&buffer, "postmaster").expect("unassigned row");
    for x in x..x + summary::cell_width("role") {
        assert_eq!(buffer[(x, y)].symbol(), " ", "a missing role stays blank");
    }
}

#[test]
fn older_tiles_keep_only_their_own_age_units_beside_younger_tiles() {
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
    let mut cells = [entry(&old, None), entry(&young, None)];
    for cell in &mut cells {
        cell.sort = TreeSort {
            column:    TreeColumn::Session,
            direction: SortDirection::Ascending,
        };
    }
    for (index, labels, expected) in [
        (0, ["week-old", "twenty-days"], [" 7d 21h", "20d    "]),
        (
            1,
            ["minutes-old", "hours-old"],
            ["    2m 49s", "5h 16m    "],
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
        let (header_x, _) = find(&buffer, "age").expect("age header");
        let end = header_x + 3;
        for (label, expected) in labels.into_iter().zip(expected) {
            let (_, y) = find(&buffer, label).expect("age row");
            let start = end - summary::cell_width(expected);
            let actual: String = (start..end).map(|x| buffer[(x, y)].symbol()).collect();
            assert_eq!(
                actual, expected,
                "{label}: header ends at the last used unit"
            );
            assert_eq!(buffer[(start - 1, y)].symbol(), " ", "age has a column gap");
        }
    }
}
