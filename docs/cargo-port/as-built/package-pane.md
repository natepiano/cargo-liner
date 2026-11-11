# Package details and statistics

The Package pane presents the selected package, workspace, or checkout group.
Its labels, values, measurements, and pagination share the same width rules.

Paths below are relative to `crates/cargo-port/src/`.

## Data and state

`tui/panes/pane_data/package_data.rs` defines package detail data, `StructureCounts`, and `PackageDeletion`.
`PackageDeletion::{Present, Deleted}` controls deleted titles and disk presentation.
Absent target content does not represent deletion.

`DescriptionState` distinguishes pending, known, and known-empty descriptions.
Cargo and GitHub producers contribute to that state.
No description appears only after the relevant sources resolve empty.
A pending or failed fetch does not manufacture known-empty text.

`DiskMeasurement` in `project/info.rs` has `NotMeasured`, `TotalOnly(u64)`, and `Measured { total, target, other }`.
Zero is a valid measurement.
Unmeasured disk rows draw `…`.
An incomplete Projects sum draws `Σ …`.

## Field layout

`tui/panes/package/render.rs` owns `package_field_label_width`, `PackageStatisticRow`, and `fit_package_title`.
The fixed two-column threshold is `MIN_TWO_COLUMN_INNER_WIDTH`, which is 37 inner cells.
Smaller panes stack sections and keep each label with its value across pages.

One-row fields share a value column.
Values do not become right-aligned when they wrap.
A field uses one row when its label, gap, and value fit before the right margin.
Otherwise its whole label and whole count occupy separate rows.
Statistics use `PackageStatisticRow` and `inline_stat_height` for both measurement and drawing.

Path and Disk retain their specific label-width rules.
A cut Path aligns with the other one-row values.
Disk children share an indent and the disk value column.
The indent can shrink when that permits the whole child label and value on one row.
The wrap decision uses the child's own label, one gap, value, and margin.
It does not require the longest sibling label to fit first.

Structure reserves its label widths before crates.io data arrives.
Its release-value budget is eight cells.
Long versions and unreachable text cut within that budget without moving the divider.
The Tests section includes a Total row.
Structure labels identify kinds, so `binary` stays singular for every count.

## CI, lint, and titles

`GithubRunCount::{Loading, Known, Unavailable}` lives in `project/info.rs`.
The Package CI value shows `local N` and a second `gh` row when space permits.
The GitHub row draws a known count, `…` while loading, or `?` when unavailable.
A later failure preserves an already known count.
Known zero on both sources draws No CI runs.
The GitHub row aligns with local text after the status icon.

Lint omits an empty icon slot when the value needs those cells.
Empty placeholders use `dim_value_color()`.
`fit_package_title` preserves the pane label.
A title with a project name keeps the separator and a name letter when that form fits.
A cut title keeps a blank cell before the corner.
A whole label can occupy the available title width.

## Invariants and rationale

1. Keep labels whole.
2. Keep a right margin and a blank cell before the Structure divider.
3. Measure the same stacked rows that the renderer draws.
4. Keep zero measurements distinct from pending measurements.
5. Count every binary target in Structure.
6. Keep the divider steady while asynchronous data arrives.

These rules prevent page changes caused only by inconsistent width calculations.
See [Targets](targets-pane.md) for binary enumeration and launching.
