//! Fixed pane geometry for the rows readout and trailing measurement cells.

use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::census::Ancestor;
use crate::census::CargoGroup;
use crate::census::CargoProcess;
use crate::census::CompilerObservation;
use crate::census::InvocationId;
use crate::census::Measurement;
use crate::census::RowProvenance;
use crate::census::RunStart;
use crate::census::VisibleParent;
use crate::census::command_text::CommandText;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::census::process_identity::CaptureMembership;
use crate::constants::TILE_ROWS_CONTENT_LABEL;
use crate::constants::UNAVAILABLE_MEASUREMENT;
use crate::progress::capture_read::CaptureLookup;
use crate::registration::WorkingDirectoryIdentity;
use crate::render;
use crate::roster::Roster;
use crate::tiles::TileContent;

/// The failing PTY frame has 298 columns and ten interior rows.
const COLLISION_INTERIOR: Rect = Rect::new(0, 0, 298, 10);
const PARENT_PID: u32 = 4100;
const CHILD_PID: u32 = 4200;
const CHILD_MARKER: &str = "readout-child";

/// Six ancestry lines, their gap, table labels, path, parent, and child ask for eleven rows.
fn collision_roster() -> Roster {
    let mut lead = invocation(PARENT_PID, "readout-parent");
    lead.managed = Measurement::Reading(1);
    let mut child = invocation(CHILD_PID, CHILD_MARKER);
    child.parent = VisibleParent::Invocation {
        id:  lead.invocation_id.clone(),
        pid: PARENT_PID,
    };
    child.cpu = Measurement::Unavailable(MeasurementAbsence::Unproven);
    child.subtree_cpu = Measurement::Unavailable(MeasurementAbsence::Unproven);
    child.compiler = CompilerObservation::Unknown;
    child.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);
    let ancestry = (0..6)
        .map(|level| Ancestor {
            pid:            5000 + level,
            command:        format!("ancestor-{level}"),
            passes_through: false,
        })
        .collect();
    let mut roster = Roster::new();
    roster.observe(
        vec![CargoGroup {
            lead,
            rest: vec![child],
            ancestry,
        }],
        Instant::now(),
    );
    roster
}

fn invocation(pid: u32, marker: &str) -> CargoProcess {
    CargoProcess {
        invocation_id: InvocationId::for_test(pid),
        capture_membership: CaptureMembership::Outside,
        provenance: RowProvenance::Uncaptured,
        path: "/fixture/readout".to_owned(),
        directory_identity: WorkingDirectoryIdentity::Absolute("/fixture/readout".into()),
        pid,
        parent: VisibleParent::None,
        start: "11:04".to_owned(),
        started: RunStart::Known(0),
        duration: "00:18".to_owned(),
        cpu: Measurement::Reading("12%".to_owned()),
        subtree_cpu: Measurement::Reading("12%".to_owned()),
        compiler: CompilerObservation::None,
        state: CaptureLookup::Unregistered,
        managed: Measurement::Reading(0),
        nested: false,
        command: CommandText::of("cargo", &["build", marker]),
    }
}

#[test]
fn rows_readout_preserves_all_child_measurements_at_collision_geometry() {
    assert_child_measurements(COLLISION_INTERIOR);
}

#[test]
fn rows_readout_preserves_all_child_measurements_when_content_fits() {
    for height in [11, 12] {
        assert_child_measurements(Rect {
            height,
            ..COLLISION_INTERIOR
        });
    }
}

fn assert_child_measurements(inner: Rect) {
    let roster = collision_roster();
    let mut buffer = Buffer::empty(inner);
    render::draw_cell_for_test(
        &mut buffer,
        &roster,
        &TileContent::Group(InvocationId::for_test(PARENT_PID)),
        inner,
        11,
    );
    let lines: Vec<String> = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect();
    let child_lines: Vec<_> = lines
        .iter()
        .filter(|line| line.contains(CHILD_MARKER))
        .collect();
    assert_eq!(child_lines.len(), 1, "{lines:#?}");
    assert_eq!(
        child_lines[0].matches(UNAVAILABLE_MEASUREMENT).count(),
        3,
        "the rows readout must leave CPU, compiler, and runs cells intact: {lines:#?}"
    );
    assert!(
        lines
            .last()
            .is_some_and(|line| line.contains(TILE_ROWS_CONTENT_LABEL)),
        "the readout remains visible at the foot of the same pane: {lines:#?}"
    );
    assert!(
        !child_lines[0].contains(TILE_ROWS_CONTENT_LABEL),
        "{lines:#?}"
    );
}
