//! Keep command measurements and text for unchanged source inputs.

use super::Arc;
use super::COMMAND_COLUMN;
use super::CellTableGap;
use super::CellTableLayout;
use super::Color;
use super::CommandText;
use super::CommandView;
use super::DirectoryHeading;
use super::DirectorySeparation;
use super::HashMap;
use super::HashSet;
use super::Index;
use super::IndexMut;
use super::InvocationId;
use super::InvocationSnapshot;
use super::OnceCell;
use super::ParentColumnContent;
use super::Rc;
use super::RowInk;
use super::TABLE_CONTINUATION_CLEARANCE;
use super::TABLE_HEADERS;
use super::TableKind;
use super::TableRowEnd;
use super::TableSeparators;
use super::Text;
use super::Theme;
use super::TrackedGroup;
use super::TrackedRow;
use super::VisibleRoster;
use super::cell_width;
use super::displayed_command;
use super::displayed_subcommand_name;
#[cfg(test)]
use super::tests;
use super::wrap_command_cell;

/// Solved columns for one unchanged source snapshot, including frame spacing decisions.
#[derive(Clone)]
pub(super) struct CachedTableLayout {
    pub(super) width:          u16,
    pub(super) height:         u16,
    pub(super) slots:          Vec<usize>,
    pub(super) row_end:        TableRowEnd,
    pub(super) frame_row_end:  TableRowEnd,
    pub(super) rows:           Vec<(InvocationId, u32)>,
    pub(super) layout:         CellTableLayout,
    pub(super) parents:        Vec<InvocationId>,
    pub(super) parent_content: ParentColumnContent,
    pub(super) gap:            CellTableGap,
}

/// Wrapped rows indexed by pid before checking full invocation identity and layout.
#[derive(Default)]
pub(super) struct PreparedProcessRows {
    pub(super) by_pid: HashMap<u32, Vec<CachedProcessRow>>,
}

/// The row's parent label follows the table's ancestry decision.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ParentVisibility {
    Shown,
    Hidden,
}

/// Styled row cells prepared once for one solved layout.
#[derive(Clone)]
pub(super) struct CachedProcessRow {
    pub(super) process:           Rc<InvocationSnapshot>,
    pub(super) faded:             RowInk,
    pub(super) cells:             PreparedRowCells,
    pub(super) lines:             u16,
    pub(super) column_widths:     Vec<u16>,
    pub(super) columns:           Vec<usize>,
    pub(super) ground:            Color,
    pub(super) parent_visibility: ParentVisibility,
    pub(super) kind:              TableKind,
    pub(super) command_form:      CommandForm,
}

/// Prepared text is shared until a table plan changes one cell's reading or ink.
#[derive(Clone)]
pub(super) struct PreparedRowCells(pub(super) [Rc<Text<'static>>; TABLE_HEADERS.len()]);

impl PreparedRowCells {
    pub(super) const fn new(cells: [Rc<Text<'static>>; TABLE_HEADERS.len()]) -> Self { Self(cells) }

    pub(super) fn iter(&self) -> impl Iterator<Item = &Text<'static>> {
        self.0.iter().map(Rc::as_ref)
    }
}

impl Index<usize> for PreparedRowCells {
    type Output = Text<'static>;

    fn index(&self, column: usize) -> &Self::Output { &self.0[column] }
}

impl IndexMut<usize> for PreparedRowCells {
    fn index_mut(&mut self, column: usize) -> &mut Self::Output {
        #[cfg(test)]
        if Rc::strong_count(&self.0[column]) > 1 {
            tests::PROCESS_ROW_CELL_CLONES.with(|count| count.set(count.get() + 1));
        }
        Rc::make_mut(&mut self.0[column])
    }
}

impl CachedProcessRow {
    pub(super) fn matches(
        &self,
        row: &TrackedRow,
        layout: &CellTableLayout,
        parent_visibility: ParentVisibility,
    ) -> bool {
        Rc::ptr_eq(&self.process, &row.snapshot())
            && self.faded == row.ink()
            && self.column_widths == layout.column_widths
            && self.columns == layout.columns
            && self.ground == layout.ground
            && self.parent_visibility == parent_visibility
            && self.kind == layout.kind
    }
}

/// Widths and wrapping share one command and view identity across resource updates.
#[derive(Default)]
pub(super) struct PreparedCommandCells {
    by_invocation: HashMap<(InvocationId, u32), UnchangedCommandInputs>,
}

/// `PreparedCommandCell` keeps `form` and its wrapped `text` in one preparation.
#[derive(Clone)]
pub(super) struct PreparedCommandCell {
    pub(super) form: CommandForm,
    pub(super) text: Rc<Text<'static>>,
}

/// `CommandForm` selects the full command or its named words for a summary row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CommandForm {
    Full,
    WithoutArguments,
}

impl CommandForm {
    const fn for_width(kind: TableKind, selected: u16, width: u16) -> Self {
        match kind {
            TableKind::Summary if selected > width => Self::WithoutArguments,
            TableKind::Summary | TableKind::Command => Self::Full,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MeasuredCommandWidths {
    pub(super) selected:   u16,
    pub(super) full:       u16,
    pub(super) subcommand: u16,
}

impl MeasuredCommandWidths {
    fn new(command: &CommandText, view: &CommandView) -> Self {
        Self {
            selected:   cell_width(&displayed_command(command, view)),
            full:       cell_width(&displayed_command(command, CommandView::long()))
                .saturating_add(TABLE_CONTINUATION_CLEARANCE),
            subcommand: cell_width(&displayed_subcommand_name(command).words)
                .saturating_add(TABLE_CONTINUATION_CLEARANCE),
        }
    }
}

/// Changing text or selection invalidates both measurements and painted words.
struct UnchangedCommandInputs {
    source:   Rc<InvocationSnapshot>,
    view:     CommandView,
    widths:   OnceCell<MeasuredCommandWidths>,
    painting: CommandPaintPreparation,
}

/// Ink changes invalidate wrapping while preserving resource-independent widths.
#[derive(Default)]
enum CommandPaintPreparation {
    #[default]
    Unprepared,
    Prepared(UnchangedCommandInk),
}

struct UnchangedCommandInk {
    ink:     CommandRowInk,
    theme:   Arc<Theme>,
    wrapped: Vec<WrappedCommandCell>,
}

#[derive(Eq, PartialEq)]
enum CommandRowInk {
    Running(RowInk),
    Finished(RowInk),
}

impl From<&TrackedRow> for CommandRowInk {
    fn from(row: &TrackedRow) -> Self {
        if row.is_ended() {
            Self::Finished(row.ink())
        } else {
            Self::Running(row.ink())
        }
    }
}

struct WrappedCommandCell {
    width:    u16,
    ground:   Color,
    kind:     TableKind,
    prepared: PreparedCommandCell,
}

impl PreparedCommandCells {
    pub(super) fn retain_visible(&mut self, roster: &VisibleRoster) {
        let visible: HashSet<_> = roster
            .groups()
            .iter()
            .flat_map(TrackedGroup::rows)
            .map(|row| (&row.process.invocation_id, row.process.pid))
            .collect();
        self.by_invocation
            .retain(|(id, pid), _| visible.contains(&(id, *pid)));
    }

    fn inputs(&mut self, row: &TrackedRow, view: &CommandView) -> &mut UnchangedCommandInputs {
        let source = self
            .by_invocation
            .entry((row.process.invocation_id.clone(), row.process.pid))
            .or_insert_with(|| UnchangedCommandInputs::new(row, view));
        if (!Rc::ptr_eq(&source.source, &row.snapshot())
            && (source.source.process.command != row.process.command
                || source.source.process.path != row.process.path))
            || source.view != *view
        {
            *source = UnchangedCommandInputs::new(row, view);
        } else {
            source.source = row.snapshot();
        }
        source
    }

    pub(super) fn widths(&mut self, row: &TrackedRow, view: &CommandView) -> MeasuredCommandWidths {
        self.inputs(row, view).measured_widths()
    }

    /// Read the cached summary command with its complete directory label.
    pub(super) fn for_summary_row(
        &mut self,
        row: &TrackedRow,
        layout: &CellTableLayout,
    ) -> Rc<Text<'static>> {
        self.for_row(row, layout).text
    }

    pub(super) fn for_row(
        &mut self,
        row: &TrackedRow,
        layout: &CellTableLayout,
    ) -> PreparedCommandCell {
        let source = self.inputs(row, &layout.view);
        let width = source.measured_widths().selected;
        let ink = CommandRowInk::from(row);
        if let CommandPaintPreparation::Prepared(painted) = &mut source.painting
            && painted.ink == ink
            && std::sync::Arc::ptr_eq(&painted.theme, &layout.theme)
        {
            return painted.for_row(row, layout, width);
        }
        let mut painting = UnchangedCommandInk {
            ink,
            theme: std::sync::Arc::clone(&layout.theme),
            wrapped: Vec::new(),
        };
        let text = painting.for_row(row, layout, width);
        source.painting = CommandPaintPreparation::Prepared(painting);
        text
    }
}

impl UnchangedCommandInputs {
    fn measured_widths(&self) -> MeasuredCommandWidths {
        *self
            .widths
            .get_or_init(|| MeasuredCommandWidths::new(&self.source.process.command, &self.view))
    }

    fn new(row: &TrackedRow, view: &CommandView) -> Self {
        Self {
            source:   row.snapshot(),
            view:     view.clone(),
            widths:   std::cell::OnceCell::new(),
            painting: CommandPaintPreparation::Unprepared,
        }
    }
}

impl UnchangedCommandInk {
    fn for_row(
        &mut self,
        row: &TrackedRow,
        layout: &CellTableLayout,
        selected: u16,
    ) -> PreparedCommandCell {
        let width = layout.column_width(COMMAND_COLUMN);
        if let Some(wrapped) = self.wrapped.iter().find(|wrapped| {
            wrapped.width == width && wrapped.ground == layout.ground && wrapped.kind == layout.kind
        }) {
            return wrapped.prepared.clone();
        }
        let form = CommandForm::for_width(layout.kind, selected, width);
        let command = wrap_command_cell(row, layout, width, form);
        let text = match layout.kind {
            TableKind::Command => command,
            TableKind::Summary => super::summary_command_cell(row, layout, width, command),
        };
        let prepared = PreparedCommandCell {
            form,
            text: Rc::new(text),
        };
        self.wrapped.push(WrappedCommandCell {
            width,
            ground: layout.ground,
            kind: layout.kind,
            prepared: prepared.clone(),
        });
        prepared
    }
}

#[cfg(test)]
mod prepared_command_cells_tests {
    use std::time::Instant;

    use crate::render;
    use crate::render::App;
    use crate::render::BYTES_PER_GIBIBYTE;
    use crate::render::Measurement;
    use crate::render::tests;
    use crate::render::tests::COMMAND_CELL_WRAPS;
    use crate::render::tests::PROCESS_COMMAND_FORMATS;

    #[test]
    fn resource_updates_keep_their_measured_command_widths()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut app = App::new_for_test()?;
        let mut group = tests::cargo_group(tests::invocation(
            100,
            &[
                "check",
                "--workspace",
                "--config",
                "env.NAME='界e\u{301} spaced value'",
            ],
        ));
        group.lead.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        group.lead.subtree_resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        app.roster.observe(vec![group.clone()], Instant::now());
        let before = tests::settled_frame(&mut app);
        assert!(
            tests::buffer_rows(&before)
                .join("\n")
                .contains(&render::memory_label(BYTES_PER_GIBIBYTE))
        );
        PROCESS_COMMAND_FORMATS.with(|count| count.set(0));
        group.lead.resource_use.memory = Measurement::Reading(2 * BYTES_PER_GIBIBYTE);
        group.lead.subtree_resource_use.memory = Measurement::Reading(2 * BYTES_PER_GIBIBYTE);
        app.roster.observe(vec![group], Instant::now());
        let after = tests::settled_frame(&mut app);
        assert!(
            tests::buffer_rows(&after)
                .join("\n")
                .contains(&render::memory_label(2 * BYTES_PER_GIBIBYTE))
        );
        assert_ne!(
            after, before,
            "the latest resource reading must reach the frame"
        );
        PROCESS_COMMAND_FORMATS.with(|count| {
            assert_eq!(
                count.get(),
                0,
                "resource updates cannot change command widths"
            );
        });
        Ok(())
    }

    #[test]
    fn resource_updates_keep_their_already_wrapped_command()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut app = App::new_for_test()?;
        let mut group =
            tests::cargo_group(tests::invocation(100, &["cargo", "check", "--workspace"]));
        group.lead.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        group.lead.subtree_resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        app.roster.observe(vec![group.clone()], Instant::now());
        let before = tests::settled_frame(&mut app);
        assert!(
            tests::buffer_rows(&before)
                .join("\n")
                .contains(&render::memory_label(BYTES_PER_GIBIBYTE))
        );
        COMMAND_CELL_WRAPS.with(|count| count.set(0));
        group.lead.resource_use.memory = Measurement::Reading(2 * BYTES_PER_GIBIBYTE);
        group.lead.subtree_resource_use.memory = Measurement::Reading(2 * BYTES_PER_GIBIBYTE);
        app.roster.observe(vec![group], Instant::now());
        let after = tests::settled_frame(&mut app);
        assert!(
            tests::buffer_rows(&after)
                .join("\n")
                .contains(&render::memory_label(2 * BYTES_PER_GIBIBYTE))
        );
        assert_ne!(
            after, before,
            "the current resource reading must still reach the frame"
        );
        COMMAND_CELL_WRAPS.with(|count| {
            assert_eq!(
                count.get(),
                0,
                "resource updates do not change command wrapping or ink"
            );
        });
        Ok(())
    }
}

/// Viewport plan shared by row-use probes and paint at one solved width.
#[derive(Clone)]
pub(super) struct CachedTablePlan {
    pub(super) rows:                Vec<Vec<(InvocationId, u32)>>,
    pub(super) headings:            Vec<DirectoryHeading>,
    pub(super) widths:              Vec<u16>,
    pub(super) columns:             Vec<usize>,
    pub(super) height:              u16,
    pub(super) separation:          DirectorySeparation,
    pub(super) visible:             Vec<usize>,
    pub(super) separators:          TableSeparators,
    pub(super) visible_group_count: usize,
    pub(super) drawn:               u16,
    pub(super) required:            u16,
    pub(super) labels:              [bool; TABLE_HEADERS.len()],
}
