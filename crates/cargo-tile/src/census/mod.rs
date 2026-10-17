//! Discovery and measurement of cargo invocation groups.

pub(crate) mod command_text;
pub(crate) mod direct_capture;
pub(crate) mod invocation_cpu_accounting;
#[cfg(test)]
mod memory_tests;
pub(crate) mod process_identity;
pub(crate) mod scan;
#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod summary_totals_tests;

pub(crate) use command_text::command_name;
pub(crate) use direct_capture::DirectAssociation;
pub(crate) use direct_capture::SelectedProof;
pub(crate) use invocation_cpu_accounting::CensusCadence;
pub(crate) use invocation_cpu_accounting::Measurement;
pub(crate) use process_identity::InvocationId;
pub(crate) use process_identity::VisibleParent;
pub(crate) use scan::Ancestor;
pub(crate) use scan::CargoGroup;
pub(crate) use scan::CargoProcess;
pub(crate) use scan::CensusScope;
pub(crate) use scan::CompilerObservation;
pub(crate) use scan::ExcludedCommands;
pub(crate) use scan::ProcessOwner;
pub(crate) use scan::RowProvenance;
pub(crate) use scan::RunStart;
pub(crate) use scan::spawn_with_resolver;
pub(crate) use scan::sum_resource_use_once_per_pid;

/// Read a rendered cell using its title's alignment and the row's inner right edge.
#[cfg(test)]
pub(crate) fn measurement_cell(header: &str, row: &str, column: &str, rendered: &str) -> String {
    let Some(start) = header.find(column) else {
        assert!(
            matches!(column, "compiler" | "runs"),
            "missing {column} title: {rendered}"
        );
        return String::new();
    };
    let row: Vec<_> = row.chars().collect();
    let title_start = header[..start].chars().count();
    let cell_start = |title: &str, start: usize| {
        if title != crate::constants::TABLE_HEADERS[crate::constants::STATE_COLUMN] {
            return start;
        }
        let end = start + title.chars().count();
        let mut start = end;
        while start > 0 && row.get(start - 1).is_some_and(|ch| !ch.is_whitespace()) {
            start -= 1;
        }
        if start == end {
            end - title.chars().count()
        } else {
            start
        }
    };
    let start = cell_start(column, title_start);
    let right = row.iter().rposition(|&ch| ch == '│').unwrap_or(row.len());
    let end = crate::constants::TABLE_HEADERS
        .iter()
        .filter_map(|title| {
            header
                .find(title)
                .map(|index| (*title, header[..index].chars().count()))
        })
        .filter(|&(_, index)| index > title_start)
        .min_by_key(|&(_, index)| index)
        .map_or(right, |(title, index)| cell_start(title, index));
    let end = if column == crate::constants::TABLE_HEADERS[crate::constants::STATE_COLUMN] {
        title_start + column.chars().count()
    } else {
        end
    };
    row.iter()
        .skip(start)
        .take(end - start)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::measurement_cell;

    #[test]
    fn a_measurement_cell_includes_a_reading_wider_than_its_title() {
        let header = "cpu mem command compiler  runs";
        let row = "        cargo   sccache×6  1";
        assert_eq!(measurement_cell(header, row, "compiler", row), "sccache×6");
        assert_eq!(measurement_cell(header, row, "runs", row), "1");
    }

    #[test]
    fn a_right_aligned_state_does_not_enter_the_memory_cell() {
        let header = "│ cpu mem       state command │";
        for (row, memory, state) in [
            ("│             blocked cargo   │", "", "blocked"),
            ("│     0.5G    blocked cargo   │", "0.5G", "blocked"),
            ("│     0.5G            cargo   │", "0.5G", ""),
        ] {
            assert_eq!(measurement_cell(header, row, "mem", row), memory);
            assert_eq!(measurement_cell(header, row, "state", row), state);
        }
    }
}
