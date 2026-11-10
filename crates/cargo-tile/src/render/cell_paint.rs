//! Replay a cell painting with its command visibility proof.

use ratatui::buffer::Cell;

use super::Buffer;
use super::CellMeasurementKey;
use super::Color;
use super::InvocationId;
use super::Rc;
use super::Rect;
use super::RowInk;
use super::Text;
use super::TrackedGroup;
use super::TrackedRow;
use super::VisibleCommandText;
use super::VisibleRoster;
use crate::app::GridView;
use crate::probe;
use crate::roster::ParentFamily;

/// Rows read or written by the painter, including continuation-marker searches.
#[derive(Default)]
pub(super) struct CellPaintReach {
    pub(super) rows:      u16,
    pub(super) commands:  Vec<PlacedCommandText>,
    pub(super) header:    CellColumnHeader,
    pub(super) resources: Vec<super::RowResourceReading>,
}

/// The table painter identifies its column header in source coordinates.
#[derive(Clone, Copy, Default)]
pub(super) enum CellColumnHeader {
    #[default]
    Absent,
    Row(u16),
}

/// The table owns each command fragment's position and shared prepared text.
pub(super) struct PlacedCommandText {
    pub(super) invocation: InvocationId,
    pub(super) pid:        u32,
    pub(super) area:       Rect,
    pub(super) text:       Rc<Text<'static>>,
}

impl PlacedCommandText {
    pub(super) fn show(&self, buffer: &Buffer, row: &TrackedRow) {
        tui_pane::PaneFrame::with_visible_text(buffer, self.area, &self.text, || {
            row.show_command_text(VisibleCommandText(()));
        });
    }

    pub(super) fn show_in_roster(&self, buffer: &Buffer, roster: &VisibleRoster) {
        if let Some(row) = roster
            .groups()
            .iter()
            .flat_map(TrackedGroup::rows)
            .find(|row| row.process.invocation_id == self.invocation && row.process.pid == self.pid)
        {
            self.show(buffer, row);
        }
    }
}

/// A cell painting belongs to one source snapshot, geometry, policy and destination.
#[derive(Eq, PartialEq)]
pub(super) struct CellPaintInputs {
    pub(super) measurement: CellMeasurementKey,
    pub(super) view:        GridView,
    pub(super) inner:       Rect,
    pub(super) buffer_area: Rect,
    pub(super) ground:      Color,
}

/// Completion changes the row's display and resource ownership.
#[derive(Debug, Eq, PartialEq)]
enum PaintedCompletion {
    Live,
    Finished,
}

impl PaintedCompletion {
    const fn of(row: &TrackedRow) -> Self {
        if row.is_ended() {
            Self::Finished
        } else {
            Self::Live
        }
    }
}

/// Frozen row inputs prevent a new roster revision from changing a retained painting.
struct PaintedRowSource {
    snapshot:           Rc<super::InvocationSnapshot>,
    completion:         PaintedCompletion,
    family:             super::FamilyHead,
    parent_family:      ParentFamily,
    progress_ownership: super::InvocationProgressOwnership,
}

impl PaintedRowSource {
    fn read(row: &TrackedRow) -> Self {
        Self {
            snapshot:           row.snapshot(),
            completion:         PaintedCompletion::of(row),
            family:             row.family(),
            parent_family:      row.parent_family(),
            progress_ownership: super::progress_ownership(row),
        }
    }

    fn matches(&self, row: &TrackedRow) -> bool {
        let before = &self.snapshot.process;
        let after = &row.process;
        let matches = self.completion == PaintedCompletion::of(row)
            && self.family == row.family()
            && self.parent_family == row.parent_family()
            && self.progress_ownership == super::progress_ownership(row)
            && (Rc::ptr_eq(&self.snapshot, &row.snapshot())
                || (before.invocation_id == after.invocation_id
                    && before.pid == after.pid
                    && before.path == after.path
                    && before.directory_identity == after.directory_identity
                    && before.parent == after.parent
                    && before.provenance == after.provenance
                    && before.state == after.state
                    && before.nested == after.nested
                    && before.command == after.command
                    && super::managed_text(before) == super::managed_text(after)
                    && (before.origin_start == after.origin_start
                        || (before.origin_start.same_duration(after.origin_start)
                            && before.start() == after.start()))));
        if !matches {
            probe::source_rejection(|| self.describe_change(row));
        }
        matches
    }
    fn describe_change(&self, row: &TrackedRow) -> String {
        let before = &self.snapshot.process;
        let after = &row.process;
        macro_rules! displayed {
            ($field:ident, $after:expr) => {
                if self.$field != $after {
                    return format!(
                        "row {} {}: {:?} -> {:?}",
                        before.pid,
                        stringify!($field),
                        self.$field,
                        $after
                    );
                }
            };
        }
        displayed!(completion, PaintedCompletion::of(row));
        displayed!(family, row.family());
        displayed!(parent_family, row.parent_family());
        displayed!(progress_ownership, super::progress_ownership(row));
        macro_rules! changed {
            ($field:ident) => {
                if before.$field != after.$field {
                    return format!(
                        "row {} {}: {:?} -> {:?}",
                        before.pid,
                        stringify!($field),
                        before.$field,
                        after.$field
                    );
                }
            };
        }
        changed!(invocation_id);
        changed!(pid);
        changed!(path);
        changed!(directory_identity);
        changed!(parent);
        changed!(provenance);
        changed!(state);
        changed!(nested);
        changed!(command);
        let managed_before = super::managed_text(before);
        let managed_after = super::managed_text(after);
        if managed_before != managed_after {
            return format!("managed: {managed_before:?} -> {managed_after:?}");
        }
        if before.start() != after.start() {
            return format!("start: {:?} -> {:?}", before.start(), after.start());
        }
        if before.duration() != after.duration() {
            return format!(
                "duration: {:?} -> {:?}",
                before.duration(),
                after.duration()
            );
        }
        format!(
            "row {} duration: {:?} -> {:?}",
            before.pid,
            before.duration(),
            after.duration()
        )
    }
}

/// Each cell retains the source details that its painter reads.
#[derive(PartialEq)]
enum CellSourceDetails {
    Empty,
    Group(super::AncestryWalk<super::Ancestor>),
    Summary(Vec<(&'static str, String)>),
}

/// A painting retains its row inputs and the frame's column decisions.
pub(super) struct CellPaintSource {
    details:   CellSourceDetails,
    rows:      Vec<PaintedRowSource>,
    grid:      super::FrameTableColumnGrid,
    resources: Vec<PaintedResourceReading>,
}

/// Selected resource readings either keep or change their displayed values.
#[derive(Eq, PartialEq)]
enum ResourceDisplayMatch {
    Unchanged,
    Changed,
}

/// Resource text follows aggregation over the rows the painter selected.
#[derive(Debug, PartialEq)]
struct PaintedResourceReading {
    invocation: InvocationId,
    pid:        u32,
    cpu:        String,
    memory:     String,
    compiler:   super::CompilerObservation,
}

impl From<&super::RowResourceReading> for PaintedResourceReading {
    fn from(reading: &super::RowResourceReading) -> Self {
        Self {
            invocation: reading.invocation.clone(),
            pid:        reading.pid,
            cpu:        super::measurement_text(
                reading.resources.cpu,
                super::census::scan::cpu_label,
            ),
            memory:     super::measurement_text(reading.resources.memory, super::memory_label),
            compiler:   reading.resources.compiler.clone(),
        }
    }
}

impl CellPaintSource {
    fn rows<'a>(roster: &'a VisibleRoster, content: &super::TileContent) -> Vec<&'a TrackedRow> {
        match content {
            super::TileContent::Summary => roster
                .groups()
                .iter()
                .flat_map(TrackedGroup::rows)
                .collect(),
            super::TileContent::Group(id) => roster
                .groups()
                .iter()
                .find(|group| &group.id == id)
                .map_or_else(Vec::new, |group| group.rows().collect()),
            super::TileContent::Empty(_) => Vec::new(),
        }
    }

    fn details(roster: &VisibleRoster, content: &super::TileContent) -> CellSourceDetails {
        match content {
            super::TileContent::Summary => CellSourceDetails::Summary(
                super::summary_memory_rows(roster)
                    .map(|(label, bytes)| (label, super::memory_label(bytes)))
                    .collect(),
            ),
            super::TileContent::Group(id) => roster
                .groups()
                .iter()
                .find(|group| &group.id == id)
                .map_or(CellSourceDetails::Empty, |group| {
                    CellSourceDetails::Group(group.ancestry().clone())
                }),
            super::TileContent::Empty(_) => CellSourceDetails::Empty,
        }
    }

    const fn grid<'a>(
        grids: &'a super::FrameColumnGrids,
        content: &super::TileContent,
    ) -> &'a super::FrameTableColumnGrid {
        match content {
            super::TileContent::Group(_) => &grids.commands.0,
            super::TileContent::Summary | super::TileContent::Empty(_) => &grids.summary.0,
        }
    }

    pub(super) fn read(
        roster: &VisibleRoster,
        content: &super::TileContent,
        grids: &super::FrameColumnGrids,
    ) -> Self {
        let rows = Self::rows(roster, content);
        let grid = Self::grid(grids, content);
        Self {
            details:   Self::details(roster, content),
            rows:      rows.iter().map(|row| PaintedRowSource::read(row)).collect(),
            grid:      grid.clone(),
            resources: Vec::new(),
        }
    }

    pub(super) fn has_finished_rows(&self) -> bool {
        self.rows
            .iter()
            .any(|row| row.completion == PaintedCompletion::Finished)
    }

    pub(super) fn record_resources(&mut self, reach: &CellPaintReach) {
        self.resources = reach
            .resources
            .iter()
            .map(PaintedResourceReading::from)
            .collect();
    }

    fn resources_match(
        &self,
        rows: &[&TrackedRow],
        grid: &super::FrameTableColumnGrid,
    ) -> ResourceDisplayMatch {
        let selected: Vec<_> = self
            .resources
            .iter()
            .filter_map(|before| {
                rows.iter().copied().find(|row| {
                    before.invocation == row.process.invocation_id && before.pid == row.process.pid
                })
            })
            .collect();
        if selected.len() != self.resources.len() {
            probe::source_rejection(|| "resource row identity changed".to_owned());
            return ResourceDisplayMatch::Changed;
        }
        let readings = super::row_resource_readings(&selected, &grid.contributions);
        for (before, reading) in self.resources.iter().zip(&readings) {
            let after = PaintedResourceReading::from(reading);
            if *before != after {
                probe::source_rejection(|| format!("resources: {before:?} -> {after:?}"));
                return ResourceDisplayMatch::Changed;
            }
        }
        ResourceDisplayMatch::Unchanged
    }

    pub(super) fn matches(
        &self,
        roster: &VisibleRoster,
        content: &super::TileContent,
        grids: &super::FrameColumnGrids,
    ) -> bool {
        let grid = Self::grid(grids, content);
        if !self.layout_matches(grid) || self.details != Self::details(roster, content) {
            return false;
        }
        let rows = Self::rows(roster, content);
        self.rows.len() == rows.len()
            && self
                .rows
                .iter()
                .zip(&rows)
                .all(|(before, row)| before.matches(row))
            && self.resources_match(&rows, grid) == ResourceDisplayMatch::Unchanged
    }

    fn layout_matches(&self, grid: &super::FrameTableColumnGrid) -> bool {
        let before = &self.grid;
        if !std::sync::Arc::ptr_eq(&before.theme, &grid.theme) {
            probe::source_rejection(|| "layout theme changed".to_owned());
            return false;
        }
        macro_rules! compare {
            ($field:ident, values) => {
                if before.$field != grid.$field {
                    probe::source_rejection(|| {
                        format!(
                            "layout {}: {:?} -> {:?}",
                            stringify!($field),
                            before.$field,
                            grid.$field
                        )
                    });
                    return false;
                }
            };
        }
        compare!(slots, values);
        compare!(separation, values);
        compare!(parent_content, values);
        if before.parent_links.hidden != grid.parent_links.hidden {
            probe::source_rejection(|| {
                format!(
                    "hidden parents: {:?} -> {:?}",
                    before.parent_links.hidden, grid.parent_links.hidden
                )
            });
            return false;
        }
        compare!(row_progress, values);
        compare!(intrinsic_widths, values);
        compare!(value_widths, values);
        compare!(command_title_width, values);
        compare!(longest_command_width, values);
        compare!(longest_subcommand_width, values);
        compare!(continuation_clearance, values);
        compare!(table_budgets, values);
        compare!(empty_tail_columns, values);
        compare!(state_column, values);
        compare!(kind, values);
        compare!(view, values);
        true
    }
}

/// A row identity selects the ink that its retained foreground uses.
#[derive(Clone)]
pub(super) struct InkRowIdentity {
    invocation: InvocationId,
    pid:        u32,
}

impl InkRowIdentity {
    fn of(row: &TrackedRow) -> Self {
        Self {
            invocation: row.process.invocation_id.clone(),
            pid:        row.process.pid,
        }
    }

    fn matches(&self, row: &TrackedRow) -> bool {
        self.invocation == row.process.invocation_id && self.pid == row.process.pid
    }
}

/// Shared headings use the minimum fade of their owning rows.
#[derive(Clone)]
pub(super) enum InkSource {
    Row(InkRowIdentity),
    OrdinaryRow(InkRowIdentity),
    Heading(Vec<InkRowIdentity>),
}

impl InkSource {
    pub(super) fn row(row: &TrackedRow) -> Self { Self::Row(InkRowIdentity::of(row)) }

    pub(super) fn ordinary(row: &TrackedRow) -> Self { Self::OrdinaryRow(InkRowIdentity::of(row)) }

    pub(super) fn heading(rows: &[&TrackedRow]) -> Self {
        Self::Heading(rows.iter().map(|row| InkRowIdentity::of(row)).collect())
    }

    fn ink(&self, rows: &[&TrackedRow]) -> super::RowInk {
        match self {
            Self::Row(identity) => rows
                .iter()
                .find(|row| identity.matches(row))
                .map_or(RowInk::Ordinary(0), |row| row.ink()),
            Self::OrdinaryRow(identity) => RowInk::Ordinary(
                rows.iter()
                    .find(|row| identity.matches(row))
                    .map_or(0, |row| row.faded()),
            ),
            Self::Heading(identities) => RowInk::Ordinary(
                identities
                    .iter()
                    .filter_map(|identity| rows.iter().find(|row| identity.matches(row)))
                    .map(|row| row.faded())
                    .min()
                    .unwrap_or_default(),
            ),
        }
    }
}

/// Each foreground keeps its source color before blending.
struct InkPixel {
    index: usize,
    role:  usize,
}

struct RetainedInkRole {
    source:    Color,
    displayed: Color,
}

/// A layer recolors foregrounds without repeating text layout or painting.
struct RetainedInkLayer {
    source: InkSource,
    pixels: Vec<InkPixel>,
    roles:  Vec<RetainedInkRole>,
}

/// A capture records only the foregrounds that a paint operation writes.
#[derive(Default)]
pub(super) struct RetainedInk {
    layers: Vec<RetainedInkLayer>,
}

enum InkRecording {
    Disabled,
    Recording { marker: Color, ink: RetainedInk },
}

thread_local! {
    static INK_RECORDING: std::cell::RefCell<InkRecording> = const { std::cell::RefCell::new(InkRecording::Disabled) };
}

/// The capture ends before its foregrounds enter the retained patch.
pub(super) struct InkCapture;

impl InkCapture {
    pub(super) fn without_ink() -> Self {
        INK_RECORDING.set(InkRecording::Disabled);
        Self
    }

    pub(super) fn start() -> Self {
        INK_RECORDING.set(InkRecording::Recording {
            marker: foreground_marker(),
            ink:    RetainedInk::default(),
        });
        Self
    }

    pub(super) fn finish(self) -> RetainedInk {
        let recorded = match INK_RECORDING.replace(InkRecording::Disabled) {
            InkRecording::Recording { ink, .. } => ink,
            InkRecording::Disabled => RetainedInk::default(),
        };
        drop(self);
        recorded
    }
}

impl Drop for InkCapture {
    fn drop(&mut self) { INK_RECORDING.set(InkRecording::Disabled); }
}

pub(super) fn records_ink() -> bool {
    INK_RECORDING.with_borrow(|state| matches!(state, InkRecording::Recording { .. }))
}

/// Prepare source colors only while a retained paint records its foregrounds.
pub(super) fn ink_ground(ground: Color) -> Color {
    if records_ink() { Color::Reset } else { ground }
}

/// Select a marker that no source foreground in this painter uses.
fn foreground_marker() -> Color {
    let mut colors = vec![
        super::label_color(),
        super::text_default(),
        super::secondary_text_color(),
        super::accent_color(),
        super::success_color(),
        super::warning_color(),
    ];
    colors.extend((0..super::theme::family_color_count()).map(super::theme::family_color));
    (0..=u8::MAX)
        .map(Color::Indexed)
        .find(|color| !colors.contains(color))
        .unwrap_or(Color::Reset)
}

/// Record exact foreground coverage while the operation draws its source colors.
pub(super) fn paint_ink<T>(
    buffer: &mut Buffer,
    area: Rect,
    source: InkSource,
    ink: super::RowInk,
    ground: Color,
    draw: impl FnOnce(&mut Buffer) -> T,
) -> T {
    if !records_ink() {
        return draw(buffer);
    }
    let area = area.intersection(buffer.area);
    let marker = INK_RECORDING.with_borrow(|state| match state {
        InkRecording::Recording { marker, .. } => *marker,
        InkRecording::Disabled => Color::Reset,
    });
    let mut before = Vec::with_capacity(usize::from(area.width) * usize::from(area.height));
    for y in area.top()..area.bottom() {
        let start = buffer.index_of(area.x, y);
        for cell in &mut buffer.content[start..start + usize::from(area.width)] {
            before.push(cell.fg);
            cell.fg = marker;
        }
    }
    let result = draw(buffer);
    let mut pixels = Vec::new();
    let mut roles: Vec<RetainedInkRole> = Vec::new();
    let mut previous = before.into_iter();
    for y in area.top()..area.bottom() {
        let start = buffer.index_of(area.x, y);
        for (offset, cell) in buffer.content[start..start + usize::from(area.width)]
            .iter_mut()
            .enumerate()
        {
            let index = start + offset;
            let original = previous.next().unwrap_or(Color::Reset);
            if cell.fg == marker {
                cell.fg = original;
            } else {
                let color = cell.fg;
                let role = roles
                    .iter()
                    .position(|role| role.source == color)
                    .unwrap_or_else(|| {
                        roles.push(RetainedInkRole {
                            source:    color,
                            displayed: ink.color(color, ground),
                        });
                        roles.len() - 1
                    });
                pixels.push(InkPixel { index, role });
                cell.fg = roles[role].displayed;
            }
        }
    }
    INK_RECORDING.with_borrow_mut(|state| {
        if let InkRecording::Recording { ink, .. } = state {
            ink.layers.push(RetainedInkLayer {
                source,
                pixels,
                roles,
            });
        }
    });
    result
}

/// Each patch position can supply the last foreground writer once.
#[derive(Clone, Copy)]
enum PatchForeground {
    Unchanged,
    Available(usize),
    Claimed,
}

impl RetainedInk {
    pub(super) fn bind(&mut self, patch: &CellPaintPatch) {
        if self.layers.is_empty() {
            return;
        }
        let (Some(first), Some(last)) = (patch.0.first(), patch.0.last()) else {
            self.layers.clear();
            return;
        };
        let origin = first.index;
        let mut positions = vec![PatchForeground::Unchanged; last.index - origin + 1];
        for (position, change) in patch.0.iter().enumerate() {
            positions[change.index - origin] = PatchForeground::Available(position);
        }
        for layer in self.layers.iter_mut().rev() {
            layer.pixels.retain_mut(|pixel| {
                let Some(entry) = pixel
                    .index
                    .checked_sub(origin)
                    .and_then(|index| positions.get_mut(index))
                else {
                    return false;
                };
                match *entry {
                    PatchForeground::Available(position) => {
                        pixel.index = position;
                        *entry = PatchForeground::Claimed;
                        true
                    },
                    PatchForeground::Unchanged | PatchForeground::Claimed => false,
                }
            });
        }
    }
}

/// An exact destination snapshot guards replay of the painter's writes.
pub(super) struct PaintedCellContents {
    pub(super) header:   CellColumnHeader,
    pub(super) inputs:   CellPaintInputs,
    pub(super) source:   CellPaintSource,
    pub(super) before:   CellPaintSurface,
    pub(super) changes:  CellPaintPatch,
    pub(super) commands: Vec<PlacedCommandText>,
    pub(super) ink:      RetainedInk,
}

impl PaintedCellContents {
    pub(super) fn recolor(&mut self, roster: &VisibleRoster, content: &super::TileContent) {
        let rows = match content {
            super::TileContent::Group(id) => roster
                .groups()
                .iter()
                .find(|group| &group.id == id)
                .map(|group| group.for_painting()),
            super::TileContent::Summary | super::TileContent::Empty(_) => None,
        };
        let rows = rows.as_ref().map_or_else(
            || CellPaintSource::rows(roster, content),
            |group| group.rows().collect(),
        );
        for layer in &mut self.ink.layers {
            let ink = layer.source.ink(&rows);
            let mut changed = false;
            for role in &mut layer.roles {
                let next = ink.color(role.source, self.inputs.ground);
                changed |= next != role.displayed;
                role.displayed = next;
            }
            if changed {
                for pixel in &layer.pixels {
                    self.changes.0[pixel.index].cell.fg = layer.roles[pixel.role].displayed;
                }
            }
        }
    }
}

/// Uniform destinations retain one exact cell; mixed destinations retain their full image.
pub(super) enum CellPaintSurface {
    Uniform { area: Rect, cell: Cell },
    Mixed(Buffer),
}

impl CellPaintSurface {
    pub(super) fn read(buffer: &Buffer, inner: Rect) -> Self {
        let area = inner.intersection(buffer.area);
        let width = usize::from(area.width);
        if !area.is_empty() {
            let cell = &buffer[(area.x, area.y)];
            if Self::uniform_matches(area, cell, buffer) {
                return Self::Uniform {
                    area,
                    cell: cell.clone(),
                };
            }
        }
        let mut content = Vec::with_capacity(width * usize::from(area.height));
        if width > 0 {
            for y in area.top()..area.bottom() {
                let start = buffer.index_of(area.x, y);
                content.extend_from_slice(&buffer.content[start..start + width]);
            }
        }
        probe::paint_work(|work| work.copied += content.len());
        Self::Mixed(Buffer { area, content })
    }

    pub(super) fn with_reach(self, inner: Rect, reach: &CellPaintReach) -> Self {
        let area = Rect {
            height: reach.rows.min(inner.height),
            ..inner
        }
        .intersection(self.area());
        match self {
            Self::Uniform { cell, .. } => Self::Uniform { area, cell },
            Self::Mixed(mut before) => {
                before
                    .content
                    .truncate(usize::from(area.width) * usize::from(area.height));
                before.area = area;
                Self::Mixed(before)
            },
        }
    }

    const fn area(&self) -> Rect {
        match self {
            Self::Uniform { area, .. } => *area,
            Self::Mixed(buffer) => buffer.area,
        }
    }

    fn cell(&self, index: usize) -> &Cell {
        match self {
            Self::Uniform { cell, .. } => cell,
            Self::Mixed(buffer) => &buffer.content[index],
        }
    }

    fn uniform_matches(area: Rect, cell: &Cell, buffer: &Buffer) -> bool {
        let width = usize::from(area.width);
        let mut scanned = 0;
        let matches = (area.top()..area.bottom()).all(|y| {
            let start = buffer.index_of(area.x, y);
            buffer.content[start..start + width].iter().all(|other| {
                scanned += 1;
                other == cell
            })
        });
        probe::paint_work(|work| work.scanned += scanned);
        matches
    }

    pub(super) fn matches(&self, buffer: &Buffer) -> bool {
        let area = self.area();
        let width = usize::from(area.width);
        if width == 0 {
            return true;
        }
        match self {
            Self::Uniform { cell, .. } => Self::uniform_matches(area, cell, buffer),
            Self::Mixed(before) => before
                .content
                .chunks(width)
                .zip(area.top()..area.bottom())
                .all(|(row, y)| {
                    let start = buffer.index_of(area.x, y);
                    let mut scanned = 0;
                    let matches =
                        row.iter()
                            .zip(&buffer.content[start..start + width])
                            .all(|(a, b)| {
                                scanned += 1;
                                a == b
                            });
                    probe::paint_work(|work| work.scanned += scanned);
                    matches
                }),
        }
    }
}

/// Writes made by one painting; untouched destination cells need no copy or storage.
pub(super) struct CellPaintPatch(Vec<PaintedCellChange>);

struct PaintedCellChange {
    index: usize,
    cell:  Cell,
}

impl CellPaintPatch {
    pub(super) fn between(before: &CellPaintSurface, buffer: &Buffer, ink: &RetainedInk) -> Self {
        let area = before.area();
        let width = usize::from(area.width);
        let origin = if area.is_empty() {
            0
        } else {
            buffer.index_of(area.x, area.y)
        };
        let span =
            usize::from(area.height.saturating_sub(1)) * usize::from(buffer.area.width) + width;
        let mut foregrounds = if ink.layers.is_empty() {
            Vec::new()
        } else {
            vec![false; span]
        };
        for pixel in ink.layers.iter().flat_map(|layer| &layer.pixels) {
            if let Some(written) = pixel
                .index
                .checked_sub(origin)
                .and_then(|index| foregrounds.get_mut(index))
            {
                *written = true;
            }
        }
        let mut changes = Vec::with_capacity(width * usize::from(area.height));
        if width > 0 {
            for (row, y) in (area.top()..area.bottom()).enumerate() {
                let start = buffer.index_of(area.x, y);
                for (offset, cell) in buffer.content[start..start + width].iter().enumerate() {
                    if before.cell(row * width + offset) != cell
                        || foregrounds
                            .get(start + offset - origin)
                            .copied()
                            .unwrap_or(false)
                    {
                        changes.push(PaintedCellChange {
                            index: start + offset,
                            cell:  cell.clone(),
                        });
                    }
                }
            }
        }
        probe::paint_work(|work| {
            work.scanned += width * usize::from(area.height);
            work.copied += changes.len();
        });
        Self(changes)
    }

    pub(super) fn apply(&self, buffer: &mut Buffer) {
        probe::paint_work(|work| work.copied += self.0.len());
        for change in &self.0 {
            buffer.content[change.index].clone_from(&change.cell);
        }
    }
}

#[cfg(test)]
mod cell_paint_tests {
    use std::time::Duration;
    use std::time::Instant;

    use crate::census::CargoGroup;
    use crate::census::Measurement;
    use crate::census::invocation_cpu_accounting::CompileMemoryShares;
    use crate::census::process_identity::CaptureMembership;
    use crate::census::scan::OriginStart;
    use crate::render;
    use crate::render::Buffer;
    use crate::render::CaptureLookup;
    use crate::render::CaptureRead;
    use crate::render::Cells;
    use crate::render::Color;
    use crate::render::CommandView;
    use crate::render::FreeDiskSpace;
    use crate::render::Phase;
    use crate::render::Progress;
    use crate::render::Rect;
    use crate::render::RunState;
    use crate::render::SccacheStats;
    use crate::render::TileCells;
    use crate::render::TileContent;
    use crate::render::VisibleParent;
    use crate::render::frame_preparation::PreparedTables;
    use crate::render::tests;
    use crate::render::tests::CELL_CONTENT_PAINTS;

    #[test]
    fn roster_revisions_keep_only_unchanged_cell_paintings() {
        for (summary, changed_command) in [(false, false), (true, false), (false, true)] {
            let mut app = tests::argument_toggle_app();
            let stats = SccacheStats::default();
            let mut tables = PreparedTables::default();
            let content = if summary {
                TileContent::Summary
            } else {
                TileContent::Group(app.roster.visible(&[]).groups()[0].id.clone())
            };
            let view = CommandView::long();
            let prepared =
                tables.for_frame(&mut app.roster, &[], view, 80, FreeDiskSpace::Unavailable);
            let cells = Cells::from_prepared(prepared, &stats);
            let area = Rect::new(0, 0, 160, 80);
            let original = Buffer::empty(area);
            let mut first = original.clone();
            cells.draw(&mut first, &content, area, Color::Black);
            if changed_command {
                let visible = app.roster.visible(&[]);
                let before = &visible.groups()[0];
                let mut process = before.lead.process.clone();
                process.command =
                    super::super::CommandText::of("cargo", &["test", "--all-targets"]);
                let mut group = tests::cargo_group(process);
                group.ancestry = before.ancestry().clone();
                app.roster.observe(vec![group], Instant::now());
            } else {
                app.roster.observe_memory_shares(CompileMemoryShares {
                    shared_compiler_cache: super::super::BYTES_PER_GIBIBYTE,
                    unattributed:          0,
                });
            }
            let prepared =
                tables.for_frame(&mut app.roster, &[], view, 80, FreeDiskSpace::Unavailable);
            let cells = Cells::from_prepared(prepared, &stats);
            let mut actual = original.clone();
            CELL_CONTENT_PAINTS.with(|count| count.set(0));
            cells.draw(&mut actual, &content, area, Color::Black);
            CELL_CONTENT_PAINTS.with(|count| {
                assert_eq!(
                    count.get(),
                    usize::from(summary || changed_command),
                    "only changed cell sources need painting"
                );
            });
            let mut fresh = original.clone();
            render::frame_draw_contents(
                &mut fresh,
                &cells.roster,
                &content,
                area,
                Color::Black,
                &cells.grids,
            );
            assert_eq!(
                actual, fresh,
                "retained paintings match the current visible source"
            );
        }
        for ground in [Color::Reset, Color::Black] {
            assert_brighten_revision(ground);
        }
        for change in [
            DisplayRevision::Subsecond,
            DisplayRevision::Second,
            DisplayRevision::CpuBelowBoundary,
            DisplayRevision::CpuAcrossBoundary,
            DisplayRevision::MemoryBelowBoundary,
            DisplayRevision::MemoryAcrossBoundary,
            DisplayRevision::HiddenCpuAcrossBoundary,
            DisplayRevision::HiddenCpuSameTotal,
            DisplayRevision::ProgressOwnership,
        ] {
            assert_display_revision(change);
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum DisplayRevision {
        Subsecond,
        Second,
        CpuBelowBoundary,
        CpuAcrossBoundary,
        MemoryBelowBoundary,
        MemoryAcrossBoundary,
        HiddenCpuAcrossBoundary,
        HiddenCpuSameTotal,
        ProgressOwnership,
    }

    fn display_groups(change: DisplayRevision) -> (CargoGroup, CargoGroup, usize, u16) {
        let mut lead = tests::invocation(4100, &["build"]);
        lead.origin_start = OriginStart::Process {
            epoch:   1_790_000_000,
            elapsed: Duration::from_millis(18_100),
        };
        lead.resource_use.cpu = Measurement::Reading(12.1);
        lead.resource_use.memory = Measurement::Reading(super::super::BYTES_PER_GIBIBYTE);
        if matches!(change, DisplayRevision::ProgressOwnership) {
            lead.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Working {
                phase:    Phase::Building,
                progress: Progress {
                    done:  5,
                    total: 10,
                },
            }));
        }
        let mut before = tests::cargo_group(lead);
        let hidden = matches!(
            change,
            DisplayRevision::HiddenCpuAcrossBoundary | DisplayRevision::HiddenCpuSameTotal
        );
        if hidden {
            before.lead.resource_use.cpu = Measurement::Reading(0.26);
            let mut child = tests::invocation(4101, &["check"]);
            child.parent = VisibleParent::Invocation {
                id:  before.lead.invocation_id.clone(),
                pid: before.lead.pid,
            };
            child.resource_use.cpu = Measurement::Reading(0.26);
            before.rest.push(child);
        }
        let mut after = before.clone();
        match change {
            DisplayRevision::Subsecond => {
                after.lead.origin_start = OriginStart::Process {
                    epoch:   1_790_000_000,
                    elapsed: Duration::from_millis(18_900),
                }
            },
            DisplayRevision::Second => {
                after.lead.origin_start = OriginStart::Process {
                    epoch:   1_790_000_000,
                    elapsed: Duration::from_millis(19_100),
                }
            },
            DisplayRevision::CpuBelowBoundary => {
                after.lead.resource_use.cpu = Measurement::Reading(12.4);
            },
            DisplayRevision::CpuAcrossBoundary => {
                after.lead.resource_use.cpu = Measurement::Reading(12.6);
            },
            DisplayRevision::MemoryBelowBoundary => {
                after.lead.resource_use.memory =
                    Measurement::Reading(super::super::BYTES_PER_GIBIBYTE * 104 / 100);
            },
            DisplayRevision::MemoryAcrossBoundary => {
                after.lead.resource_use.memory =
                    Measurement::Reading(super::super::BYTES_PER_GIBIBYTE * 106 / 100);
            },
            DisplayRevision::HiddenCpuAcrossBoundary => {
                after.rest[0].resource_use.cpu = Measurement::Reading(0.20);
            },
            DisplayRevision::ProgressOwnership => {
                after.lead.capture_membership = CaptureMembership::EnclosingUnconfirmed;
            },
            DisplayRevision::HiddenCpuSameTotal => {
                after.lead.resource_use.cpu = Measurement::Reading(0.20);
                after.rest[0].resource_use.cpu = Measurement::Reading(0.32);
            },
        }
        let paints = usize::from(matches!(
            change,
            DisplayRevision::Second
                | DisplayRevision::CpuAcrossBoundary
                | DisplayRevision::MemoryAcrossBoundary
                | DisplayRevision::HiddenCpuAcrossBoundary
                | DisplayRevision::ProgressOwnership
        ));
        (before, after, paints, if hidden { 3 } else { 80 })
    }

    fn assert_display_revision(change: DisplayRevision) {
        let (before, after, expected_paints, height) = display_groups(change);
        let mut roster = super::super::Roster::default();
        let now = Instant::now();
        roster.observe(vec![before], now);
        let stats = SccacheStats::default();
        let mut tables = PreparedTables::default();
        let content = TileContent::Group(roster.visible(&[]).groups()[0].id.clone());
        let view = CommandView::long();
        let area = Rect::new(0, 0, 160, height);
        let original = Buffer::empty(area);
        let cells = Cells::from_prepared(
            tables.for_frame(&mut roster, &[], view, 80, FreeDiskSpace::Unavailable),
            &stats,
        );
        let mut first = original.clone();
        cells.draw(&mut first, &content, area, Color::Black);
        roster.observe(vec![after], now);
        let cells = Cells::from_prepared(
            tables.for_frame(&mut roster, &[], view, 80, FreeDiskSpace::Unavailable),
            &stats,
        );
        let mut actual = original.clone();
        CELL_CONTENT_PAINTS.with(|count| count.set(0));
        cells.draw(&mut actual, &content, area, Color::Black);
        CELL_CONTENT_PAINTS.with(|count| assert_eq!(count.get(), expected_paints, "{change:?}"));
        let mut fresh = original;
        render::frame_draw_contents(
            &mut fresh,
            &cells.roster,
            &content,
            area,
            Color::Black,
            &cells.grids,
        );
        assert_eq!(
            actual, fresh,
            "{change:?}: retained and fresh paintings agree"
        );
        if expected_paints > 0 {
            assert_ne!(first, fresh, "{change:?}: the displayed value changes");
        } else {
            assert_eq!(first, fresh, "{change:?}: the displayed value stays equal");
        }
    }

    fn assert_brighten_revision(ground: Color) {
        let now = Instant::now();
        let mut roster = super::super::Roster::default();
        roster.observe(
            vec![tests::cargo_group(tests::invocation(4100, &["build"]))],
            now,
        );
        roster.observe(Vec::new(), now + Duration::from_millis(100));
        roster.advance(now + Duration::from_millis(2600), Duration::from_secs(3));
        let stats = SccacheStats::default();
        let mut tables = PreparedTables::default();
        let content = TileContent::Group(roster.visible(&[]).groups()[0].id.clone());
        let (canvas, area) = match ground {
            Color::Indexed(16) => (Rect::new(0, 0, 180, 90), Rect::new(3, 5, 160, 80)),
            _ => (Rect::new(0, 0, 160, 80), Rect::new(0, 0, 160, 80)),
        };
        let original = Buffer::empty(canvas);
        let cells = Cells::from_prepared(
            tables.for_frame(
                &mut roster,
                &[],
                CommandView::long(),
                80,
                FreeDiskSpace::Unavailable,
            ),
            &stats,
        );
        let mut first = original.clone();
        cells.draw(&mut first, &content, area, ground);
        let before_ink = roster.visible(&[]).groups()[0].lead.painted_ink();
        roster.advance(now + Duration::from_millis(2625), Duration::from_secs(3));
        assert_ne!(
            before_ink,
            roster.visible(&[]).groups()[0].lead.painted_ink()
        );
        let cells = Cells::from_prepared(
            tables.for_frame(
                &mut roster,
                &[],
                CommandView::long(),
                80,
                FreeDiskSpace::Unavailable,
            ),
            &stats,
        );
        let mut actual = original.clone();
        CELL_CONTENT_PAINTS.with(|count| count.set(0));
        cells.draw(&mut actual, &content, area, ground);
        CELL_CONTENT_PAINTS.with(|count| {
            assert_eq!(
                count.get(),
                0,
                "ink changes retain the painting: {ground:?}"
            );
        });
        let mut fresh = original;
        render::frame_draw_contents(
            &mut fresh,
            &cells.roster,
            &content,
            area,
            ground,
            &cells.grids,
        );
        assert_eq!(actual, fresh, "retained ink matches fresh ink: {ground:?}");
        if ground == Color::Reset {
            assert_eq!(first, fresh, "equal blended colors keep every cell");
        } else {
            assert_ne!(first, fresh, "different blended colors recolor the cells");
        }
    }

    #[test]
    fn untouched_tail_changes_reuse_fitting_cell_paint() {
        let app = tests::argument_toggle_app();
        let stats = SccacheStats::default();
        let cells = Cells::new(&app.roster, &[], CommandView::long(), &stats);
        let canvas = Rect::new(3, 5, 180, 32);
        for inner in [Rect::new(5, 7, 176, 28), Rect::new(5, 7, 176, 24)] {
            let ground = Color::Black;
            for content in [
                TileContent::Summary,
                TileContent::Group(cells.roster.groups()[0].id.clone()),
            ] {
                let original = Buffer::empty(canvas);
                let mut first = original.clone();
                cells.draw(&mut first, &content, inner, ground);
                let mut actual = original.clone();
                let tail = (inner.right() - 1, inner.bottom() - 1);
                actual[tail].set_symbol("界").set_fg(Color::Green);
                let mut fresh = actual.clone();
                render::frame_draw_contents(
                    &mut fresh,
                    &cells.roster,
                    &content,
                    inner,
                    ground,
                    &cells.grids,
                );
                assert_eq!(
                    first[tail], original[tail],
                    "the cell leaves its tail alone"
                );
                CELL_CONTENT_PAINTS.with(|count| count.set(0));
                cells.draw(&mut actual, &content, inner, ground);
                assert_eq!(actual, fresh, "tail changes preserve fresh painting");
                CELL_CONTENT_PAINTS.with(|count| {
                    assert_eq!(
                        count.get(),
                        0,
                        "untouched tail rows cannot change this painting"
                    );
                });
            }
        }
    }
}
