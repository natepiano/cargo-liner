//! What the display holds between scans: the groups running now, and the
//! ones that have finished but have not yet faded.
//!
//! The census publishes live, missing and explicitly ended invocations. The roster
//! projects those states into rows and retains departure receipts for the census.
//!
//! Every entry is keyed by invocation identity, so a row and its tile survive a scan
//! unchanged rather than being rebuilt four times a second.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use tui_pane::CellDeparture;
use tui_pane::CellLifecycleTiming;
use tui_pane::EndedCellDeparture;
use tui_pane::PaintedRowSpan;
use tui_pane::Theme;

use crate::census::Ancestor;
use crate::census::CargoGroup;
use crate::census::CargoProcess;
use crate::census::InvocationId;
use crate::census::Measurement;
use crate::census::RunStart;
use crate::census::VisibleParent;
use crate::census::scan;
use crate::census::scan::AdmissibleInvocation;
use crate::census::scan::AncestryWalk;
use crate::census::scan::LiveInvocation;
use crate::census::scan::PublishedInvocation;
use crate::census::scan::PublishedInvocations;
use crate::render::SessionColumnWidths;
use crate::theme;

/// Display lifetime is copied from census publication, never inferred from absence.
type RowLifetime = PublishedInvocation;

/// Whether an invocation heads a family whose children share its colour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FamilyHead {
    /// No retained child family is assigned to this invocation.
    NoChildren,
    /// The invocation heads the family with this palette index.
    Heads(usize),
}

/// The family colour a row uses to point to its visible cargo parent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParentFamily {
    /// The row has no visible parent with an assigned family colour.
    NoFamily,
    /// The row belongs to the family with this palette index.
    Member(usize),
}

/// One table row, once it has stopped when that happened, and how far
/// its text has since been carried toward the ground it is drawn on.
#[derive(Clone)]
pub(crate) struct TrackedRow {
    /// The invocation as the last scan that carried it described it.
    pub(crate) process: CargoProcess,
    /// Whether the row still runs or is fading from a recorded finish.
    lifetime:           RowLifetime,
    /// How far through the fade the row stands, on the alpha scale
    /// [`blend_color`](tui_pane::blend_color) reads: zero while it
    /// runs, [`u8::MAX`] once it has reached the ground and is about to
    /// be let go of.
    faded:              u8,
    /// The family this invocation heads, where it has cargo running
    /// under it, as an index into the palette.
    family:             FamilyHead,
    /// The family of the cargo this invocation is running under, which
    /// is the index that cargo's own row carries.
    parent_family:      ParentFamily,
}

#[cfg(test)]
impl From<CargoProcess> for TrackedRow {
    /// A row for a process the current scan carries.
    fn from(process: CargoProcess) -> Self {
        Self {
            process,
            lifetime: RowLifetime::Live(scan::InvocationPresentation::default()),
            faded: 0,
            // Stamped by the roster once the whole scan is in: which
            // colours are free is a question about every group at
            // once, not about this row.
            family: FamilyHead::NoChildren,
            parent_family: ParentFamily::NoFamily,
        }
    }
}

impl TrackedRow {
    /// A display row requires the census state of that same invocation.
    const fn published(process: CargoProcess, lifetime: RowLifetime) -> Self {
        Self {
            process,
            lifetime,
            faded: 0,
            family: FamilyHead::NoChildren,
            parent_family: ParentFamily::NoFamily,
        }
    }

    /// Whether the process behind this row has stopped.
    pub(crate) const fn is_ended(&self) -> bool { matches!(&self.lifetime, RowLifetime::Ended(_)) }

    /// How far the row's text has been carried toward the ground.
    pub(crate) const fn faded(&self) -> u8 { self.faded }

    /// The family this invocation heads, where it heads one.
    pub(crate) const fn family(&self) -> FamilyHead { self.family }

    /// The family of the cargo this invocation runs under.
    pub(crate) const fn parent_family(&self) -> ParentFamily { self.parent_family }

    /// Accept only a census row proven live, and never revive a finished row.
    fn refresh_live(&mut self, live: LiveInvocation<'_>) {
        if self.is_ended() {
            return;
        }
        self.process = live.row.clone();
        self.lifetime = live.published();
        self.faded = 0;
    }

    /// Explicit completion for legacy row fixtures.
    #[cfg(test)]
    fn finish(&mut self, now: Instant) {
        if !self.is_ended() {
            self.lifetime =
                RowLifetime::Ended(scan::EndedInvocation::fixture(self.process.clone(), now));
        }
    }

    /// The sole authority for a first painted cell.
    pub(crate) fn admissible(&self) -> Result<AdmissibleInvocation, ()> {
        self.lifetime.admissible()
    }

    pub(crate) fn presented(&self, span: PaintedRowSpan<InvocationId>) {
        match &self.lifetime {
            RowLifetime::Live(_) => {
                if let Ok(admissible) = self.admissible() {
                    admissible.painted(span);
                }
            },
            RowLifetime::Ended(end) => end.painted(span),
            RowLifetime::AwaitingCargo(presentation) | RowLifetime::Missing(presentation) => {
                presentation.painted(span);
            },
            RowLifetime::Joined { .. } => {},
        }
    }

    fn withdraw(&self) {
        match &self.lifetime {
            RowLifetime::Live(presentation)
            | RowLifetime::AwaitingCargo(presentation)
            | RowLifetime::Missing(presentation) => {
                presentation.withdraw();
            },
            RowLifetime::Ended(end) => end.withdraw(),
            RowLifetime::Joined { .. } => {},
        }
    }

    /// Move the fade on, reporting whether it went anywhere.
    ///
    /// A row that has not stopped has nothing to move: the whole travel
    /// is measured from the published end timestamp.
    fn advance(&mut self, now: Instant, fade: Duration) -> bool {
        let faded = self.faded_at(now, fade);
        let moved = faded != self.faded;
        self.faded = faded;
        moved
    }

    /// Where the fade stands at `now`.
    ///
    /// The whole travel once `fade` has run out, which is also what a
    /// `fade` of nothing gives on the poll that stamps the row -- there
    /// is no travel to make, and the row goes on that same poll.
    fn faded_at(&self, now: Instant, fade: Duration) -> u8 {
        let RowLifetime::Ended(ended) = &self.lifetime else {
            return 0;
        };
        let elapsed = now.saturating_duration_since(ended.at());
        if elapsed >= fade {
            return u8::MAX;
        }
        tui_pane::CellLifecycleTiming::new(fade)
            .fade_out(elapsed)
            .amount()
    }

    /// Whether the row has been finished for longer than `fade`.
    fn is_expired(&self, now: Instant, fade: Duration) -> bool {
        matches!(&self.lifetime, RowLifetime::Ended(ended) if now.saturating_duration_since(ended.at()) >= fade)
    }
}

/// One command's rows: the summary row and the invocations under it.
#[derive(Clone)]
pub(crate) struct TrackedGroup {
    /// The lead invocation's identity, independent of its displayed pid.
    pub(crate) id:   InvocationId,
    /// The row the summary carries.
    pub(crate) lead: TrackedRow,
    /// The invocations running under the lead, newest first.
    rest:            Vec<TrackedRow>,
    /// What stands above the command, outermost first. Refreshed with
    /// the rest of the group: a chain is read fresh every scan, and a
    /// process in it that exits is one the next scan no longer carries.
    ancestry:        AncestryWalk<Ancestor>,
}

#[cfg(test)]
impl From<CargoGroup> for TrackedGroup {
    /// Track a group the current scan has just turned up.
    fn from(group: CargoGroup) -> Self {
        Self {
            id:       group.id(),
            lead:     TrackedRow::from(group.lead),
            rest:     group.rest.into_iter().map(TrackedRow::from).collect(),
            ancestry: group.ancestry,
        }
    }
}

impl TrackedGroup {
    /// Adapt only an explicit census end to the cell's departure evidence.
    pub(crate) fn departure_evidence(&self, fade: Duration) -> Option<EndedCellDeparture> {
        match &self.lead.lifetime {
            RowLifetime::Ended(end) => Some(end.departure(fade)),
            RowLifetime::Live(_)
            | RowLifetime::AwaitingCargo(_)
            | RowLifetime::Missing(_)
            | RowLifetime::Joined { .. } => None,
        }
    }

    /// Every row the group's tile draws, the lead first.
    pub(crate) fn rows(&self) -> impl Iterator<Item = &TrackedRow> {
        std::iter::once(&self.lead).chain(self.rest.iter())
    }

    /// The same rows, for the pass that stamps each one's family.
    fn rows_mut(&mut self) -> impl Iterator<Item = &mut TrackedRow> {
        std::iter::once(&mut self.lead).chain(self.rest.iter_mut())
    }

    /// Whole-cell departure exists only for a published end value.
    pub(crate) fn departure(&self, fade: Duration) -> CellDeparture {
        self.ended(fade)
            .map_or(CellDeparture::Live, |end| CellDeparture::Fading {
                started: end.0,
                timing:  end.1,
            })
    }

    /// Borrow the explicit end's clock for the drawing adapter.
    const fn ended(&self, fade: Duration) -> Option<(Instant, CellLifecycleTiming)> {
        match &self.lead.lifetime {
            RowLifetime::Live(_)
            | RowLifetime::AwaitingCargo(_)
            | RowLifetime::Missing(_)
            | RowLifetime::Joined { .. } => None,
            RowLifetime::Ended(end) => Some((end.at(), tui_pane::CellLifecycleTiming::new(fade))),
        }
    }

    /// What stands above the command, outermost first.
    pub(crate) const fn ancestry(&self) -> &AncestryWalk<Ancestor> { &self.ancestry }

    /// Refreshing a group requires a live invocation value from the census.
    fn refresh(&mut self, live: LiveInvocation<'_>, ancestry: &AncestryWalk<Ancestor>) {
        self.lead.refresh_live(live);
        self.ancestry = ancestry.clone();
    }

    /// Census membership is authoritative; a finished lead keeps its frozen list.
    fn apply_publication(&mut self, group: &CargoGroup, published: &PublishedInvocations) {
        if !self.lead.is_ended() {
            if let Ok(live) = published.live(&group.lead) {
                self.refresh(live, &group.ancestry);
            }
            if let Some(state) = published.state(&group.lead.invocation_id) {
                self.lead.lifetime = state.clone();
            }
        }
        let previous = std::mem::take(&mut self.rest);
        for mut row in previous {
            let Some(process) = group
                .rest
                .iter()
                .find(|process| process.invocation_id == row.process.invocation_id)
            else {
                continue;
            };
            let Some(state) = published.state(&process.invocation_id) else {
                continue;
            };
            if !row.is_ended() {
                if let Ok(live) = published.live(process) {
                    row.refresh_live(live);
                }
                row.lifetime = state.clone();
            }
            self.rest.push(row);
        }
        for process in &group.rest {
            if !self
                .rest
                .iter()
                .any(|row| row.process.invocation_id == process.invocation_id)
                && let Some(state) = published.state(&process.invocation_id)
            {
                self.rest
                    .push(TrackedRow::published(process.clone(), state.clone()));
            }
        }
    }

    /// Drop the rows that have finished fading and move the rest of the
    /// group's fade on, reporting whether anything the display draws
    /// changed.
    fn advance(&mut self, now: Instant, fade: Duration) -> bool {
        let before = self.rest.len();
        self.rest.retain(|row| {
            if row.is_expired(now, fade) {
                if let RowLifetime::Ended(end) = &row.lifetime {
                    end.departed();
                }
                false
            } else {
                true
            }
        });
        let mut changed = self.rest.len() != before;
        changed |= self.lead.advance(now, fade);
        for row in &mut self.rest {
            changed |= row.advance(now, fade);
        }
        changed
    }

    /// Whether the group itself has finished fading and its tile should
    /// close.
    fn is_expired(&self, now: Instant, fade: Duration) -> bool { self.lead.is_expired(now, fade) }

    /// Retain unlisted invocations and recompute readings after hidden subtrees leave.
    fn without_hidden_descendants(&self, hidden: &[String]) -> Self {
        if hidden.is_empty() {
            return self.clone();
        }
        let parents: HashMap<_, _> = self
            .rows()
            .filter_map(|row| match &row.process.parent {
                VisibleParent::Invocation { id, .. } => {
                    Some((row.process.invocation_id.clone(), id.clone()))
                },
                VisibleParent::Ancestor(_) | VisibleParent::None => None,
            })
            .collect();
        let mut removed: HashSet<_> = self
            .rows()
            .filter(|row| row.process.command.is_hidden(hidden))
            .map(|row| row.process.invocation_id.clone())
            .collect();
        loop {
            let before = removed.len();
            for (child, parent) in &parents {
                if removed.contains(parent) {
                    removed.insert(child.clone());
                }
            }
            if removed.len() == before {
                break;
            }
        }
        let mut visible = self.clone();
        visible
            .rest
            .retain(|row| !removed.contains(&row.process.invocation_id));
        if !removed.is_empty() {
            let members: Vec<_> = visible
                .rows()
                .filter(|row| !row.is_ended())
                .map(|row| &row.process)
                .collect();
            let totals = scan::subtree_resource_totals(&members);
            let totals: HashMap<_, _> = members
                .iter()
                .zip(totals)
                .map(|(row, total)| (row.invocation_id.clone(), total))
                .collect();
            let mut hidden_descendants: HashMap<InvocationId, Vec<&TrackedRow>> = HashMap::new();
            for child in self
                .rows()
                .filter(|row| removed.contains(&row.process.invocation_id))
            {
                let mut current = &child.process.invocation_id;
                let mut visited = HashSet::from([current]);
                while let Some(parent) = parents.get(current) {
                    if !visited.insert(parent) {
                        break;
                    }
                    hidden_descendants
                        .entry(parent.clone())
                        .or_default()
                        .push(child);
                    current = parent;
                }
            }
            for row in visible.rows_mut() {
                if let Some(total) = totals.get(&row.process.invocation_id) {
                    row.process.subtree_resource_use = total.clone();
                }
                if let Some(descendants) = hidden_descendants.get(&row.process.invocation_id) {
                    let removed_runs = descendants
                        .iter()
                        .filter(|child| row.is_ended() || !child.is_ended())
                        .count();
                    row.process.managed = row
                        .process
                        .managed
                        .map(|count| count.saturating_sub(removed_runs));
                }
            }
        }
        visible
    }

    /// Earliest visible invocation start, with pid breaking ties.
    fn began(&self) -> (RunStart, u32) {
        self.rows()
            .map(|row| (row.process.started, row.process.pid))
            .min()
            .unwrap_or((self.lead.process.started, self.lead.process.pid))
    }
}

/// Family assignments use the palette computed for the complete active theme.
#[derive(Clone, Default)]
enum FamilyPalette {
    #[default]
    Unmeasured,
    Measured {
        theme: Arc<Theme>,
        count: usize,
    },
}

impl FamilyPalette {
    fn count(&mut self) -> usize {
        let active = tui_pane::theme();
        if let Self::Measured { theme, count } = self
            && Arc::ptr_eq(theme, &active)
        {
            return *count;
        }
        let count = theme::family_color_count().max(1);
        *self = Self::Measured {
            theme: active,
            count,
        };
        count
    }
}

/// Every group the display knows about, running or fading.
#[derive(Clone)]
pub(crate) struct Roster {
    /// Groups in scan order -- newest command first -- with finished
    /// ones held in place until they expire.
    groups:         Vec<TrackedGroup>,
    /// The scan last folded in, kept so an identical one can be
    /// recognised and skipped. With nothing building, the display has no
    /// reason to repaint four times a second.
    /// Missing evidence is part of the answer, including its eventual resolution.
    publication:    PublishedInvocations,
    observed_at:    Option<Instant>,
    /// The palette index each family holds, by the invocation heading it.
    ///
    /// Kept here rather than worked out per cell because the colour has
    /// to be the same in the summary and in the command's own cell, and
    /// because which colours are free is a question about every group
    /// at once. An index is held while the family is on screen and the
    /// active palette contains it, fading rows included.
    families:       HashMap<InvocationId, usize>,
    family_palette: FamilyPalette,
    /// Metadata columns keep the widest machine or session reading.
    column_widths:  SessionColumnWidths,
}

impl Default for Roster {
    fn default() -> Self { Self::new() }
}

impl Roster {
    /// An empty roster.
    pub(crate) fn new() -> Self {
        Self {
            groups:         Vec::new(),
            publication:    PublishedInvocations::default(),
            observed_at:    None,
            families:       HashMap::new(),
            family_palette: FamilyPalette::Unmeasured,
            column_widths:  crate::render::SessionColumnWidths::machine(),
        }
    }

    /// An empty roster sized for a stated machine rather than the one running the test.
    #[cfg(test)]
    pub(crate) fn for_test(column_widths: SessionColumnWidths) -> Self {
        Self {
            column_widths,
            ..Self::new()
        }
    }

    /// The groups to display, newest first.
    #[cfg(test)]
    pub(crate) fn groups(&self) -> &[TrackedGroup] { &self.groups }

    /// Inject a later ancestry reading without changing the invocation's lifetime.
    #[cfg(test)]
    pub(crate) fn observe_ancestry_for_test(
        &mut self,
        id: &InvocationId,
        ancestry: AncestryWalk<Ancestor>,
    ) -> Result<(), &'static str> {
        let group = self
            .groups
            .iter_mut()
            .find(|group| &group.id == id)
            .ok_or("ancestry update requires a tracked command")?;
        group.ancestry = ancestry;
        Ok(())
    }

    /// Metadata widths retained across every frame in this session.
    #[cfg(test)]
    pub(crate) const fn column_widths(&self) -> SessionColumnWidths { self.column_widths }

    /// Visible identities for scanner fixtures that do not construct a frame.
    #[cfg(test)]
    pub(crate) fn tiled_ids(&self, hidden: &[String]) -> Vec<InvocationId> {
        self.clone().visible(hidden).tiled_ids()
    }

    /// Remove hidden subtrees before observing readings or building any display layout.
    pub(crate) fn visible(&mut self, hidden: &[String]) -> VisibleRoster {
        let mut groups = if hidden.is_empty() {
            self.groups.clone()
        } else {
            for group in &self.groups {
                if group.lead.process.command.is_hidden(hidden) {
                    for row in group.rows() {
                        row.withdraw();
                    }
                }
            }
            let groups: Vec<_> = self
                .groups
                .iter()
                .filter(|group| !group.lead.process.command.is_hidden(hidden))
                .map(|group| group.without_hidden_descendants(hidden))
                .collect();
            let visible_ids: HashSet<_> = groups
                .iter()
                .flat_map(TrackedGroup::rows)
                .map(|row| &row.process.invocation_id)
                .collect();
            for row in self.groups.iter().flat_map(TrackedGroup::rows) {
                if !visible_ids.contains(&row.process.invocation_id) {
                    row.withdraw();
                }
            }
            groups
        };
        self.assign_families(&mut groups);
        let mut column_widths = self.column_widths;
        for row in groups.iter().flat_map(TrackedGroup::rows) {
            column_widths.observe(&row.process);
        }
        self.column_widths = column_widths;
        VisibleRoster {
            groups,
            column_widths,
        }
    }

    /// Fold one scan in, stamping whatever it no longer carries.
    ///
    /// A group the scan still carries keeps its place: the display is
    /// ordered by when a command started, and re-sorting on every scan
    /// would shuffle tiles under the reader for no reason.
    #[cfg(test)]
    pub(crate) fn observe(&mut self, scan: Vec<CargoGroup>, now: Instant) -> bool {
        self.observe_discovery(scan, &[], now)
    }

    /// Legacy fixtures publish their explicit end observations through the census adapter.
    #[cfg(test)]
    pub(crate) fn observe_discovery(
        &mut self,
        groups: Vec<CargoGroup>,
        missing: &[InvocationId],
        now: Instant,
    ) -> bool {
        let previous: Vec<_> = self
            .groups
            .iter()
            .map(|group| CargoGroup {
                lead:     group.lead.process.clone(),
                rest:     group.rest.iter().map(|row| row.process.clone()).collect(),
                ancestry: group.ancestry.clone(),
            })
            .collect();
        let published = PublishedInvocations::fixture(groups, missing, &previous, now);
        self.observe_invocations(&published)
    }

    /// Reject delayed answers without letting publication age alter the end evidence.
    pub(crate) fn observe_published_scan(
        &mut self,
        published: &PublishedInvocations,
        observed_at: Instant,
    ) -> bool {
        if self
            .observed_at
            .is_some_and(|previous| observed_at < previous)
        {
            return false;
        }
        self.observed_at = Some(observed_at);
        self.observe_invocations(published)
    }

    /// Only published states can change a row's lifetime. Omission has no end meaning.
    pub(crate) fn observe_invocations(&mut self, published: &PublishedInvocations) -> bool {
        if &self.publication == published {
            return false;
        }
        let mut ordered = published.groups().iter().collect::<Vec<_>>();
        ordered.sort_by_key(|group| {
            self.groups
                .iter()
                .position(|tracked| tracked.id == group.id())
                .unwrap_or(self.groups.len())
        });
        let mut previous = std::mem::take(&mut self.groups);
        self.groups = ordered
            .into_iter()
            .filter_map(|group| {
                previous
                    .iter()
                    .position(|tracked| tracked.id == group.id())
                    .map_or_else(
                        || {
                            published
                                .state(&group.lead.invocation_id)
                                .map(|state| TrackedGroup {
                                    id:       group.id(),
                                    lead:     TrackedRow::published(
                                        group.lead.clone(),
                                        state.clone(),
                                    ),
                                    rest:     group
                                        .rest
                                        .iter()
                                        .filter_map(|row| {
                                            published.state(&row.invocation_id).map(|state| {
                                                TrackedRow::published(row.clone(), state.clone())
                                            })
                                        })
                                        .collect(),
                                    ancestry: group.ancestry.clone(),
                                })
                        },
                        |index| {
                            let mut tracked = previous.remove(index);
                            tracked.apply_publication(group, published);
                            Some(tracked)
                        },
                    )
            })
            .collect();
        self.publication.clone_from(published);
        true
    }

    /// Hand every family on screen a colour no other family on screen
    /// holds, and stamp it on the rows that draw it.
    ///
    /// A family is a cargo with cargo running under it. Its index is
    /// kept while it is on screen and inside the active palette:
    /// reassigning from scratch each scan would recolour every cell whenever a build finished,
    /// which is the most routine event here. A family that goes away
    /// gives its index back, so the palette is spent on what is
    /// actually running.
    ///
    /// An unavailable managed count keeps an established family, or
    /// confirms a new one when a retained child names the row as its
    /// parent. Without either, it establishes no family. A measured
    /// zero releases the index once no fading child points to it.
    ///
    /// More families at once than the palette holds is the one case
    /// this cannot serve, and it wraps rather than leaving the extra
    /// ones unmarked -- a repeated colour still pairs correctly far
    /// more often than none does.
    fn assign_families(&mut self, groups: &mut [TrackedGroup]) {
        let parents: HashSet<InvocationId> = groups
            .iter()
            .flat_map(TrackedGroup::rows)
            .filter_map(|row| match &row.process.parent {
                VisibleParent::Invocation { id, .. } => Some(id.clone()),
                VisibleParent::Ancestor(_) | VisibleParent::None => None,
            })
            .collect();
        let fading_parents: HashSet<_> = groups
            .iter()
            .flat_map(TrackedGroup::rows)
            .filter(|row| row.is_ended())
            .filter_map(|row| match &row.process.parent {
                VisibleParent::Invocation { id, .. } => Some(id.clone()),
                VisibleParent::Ancestor(_) | VisibleParent::None => None,
            })
            .collect();
        let heads: Vec<InvocationId> = groups
            .iter()
            .flat_map(TrackedGroup::rows)
            .filter(|row| match row.process.managed {
                Measurement::Reading(managed) => {
                    managed > 0 || fading_parents.contains(&row.process.invocation_id)
                },
                Measurement::Unavailable(_) => {
                    self.families.contains_key(&row.process.invocation_id)
                        || parents.contains(&row.process.invocation_id)
                },
            })
            .map(|row| row.process.invocation_id.clone())
            .collect();
        let mut families = std::mem::take(&mut self.families);
        // Asked of the theme rather than of the palette, because how
        // many colours are left depends on what the column headers are
        // drawn in and that is the reader's to change.
        let count = self.family_palette.count();
        families.retain(|id, index| heads.contains(id) && *index < count);
        for id in heads {
            if families.contains_key(&id) {
                continue;
            }
            let taken: HashSet<usize> = families.values().copied().collect();
            let free = (0..count).find(|index| !taken.contains(index));
            families.insert(id, free.unwrap_or(families.len() % count));
        }
        for group in groups {
            for row in group.rows_mut() {
                row.family = families
                    .get(&row.process.invocation_id)
                    .copied()
                    .map_or(FamilyHead::NoChildren, FamilyHead::Heads);
                row.parent_family = match &row.process.parent {
                    VisibleParent::Invocation { id, .. } => families
                        .get(id)
                        .copied()
                        .map_or(ParentFamily::NoFamily, ParentFamily::Member),
                    VisibleParent::Ancestor(_) | VisibleParent::None => ParentFamily::NoFamily,
                };
            }
        }
        self.families = families;
    }

    /// Move every fade on and drop whatever has finished one, reporting
    /// whether the display needs repainting as a result.
    ///
    /// A fade is a colour walking toward the ground rather than a
    /// single change of shade, so this reports a repaint on every step
    /// of it, not only on the row that has just been let go of.
    pub(crate) fn advance(&mut self, now: Instant, fade: Duration) -> bool {
        let before = self.groups.len();
        self.groups.retain(|group| {
            if group.is_expired(now, fade) {
                for row in group.rows() {
                    if let RowLifetime::Ended(end) = &row.lifetime {
                        end.departed();
                    }
                }
                false
            } else {
                true
            }
        });
        let mut changed = self.groups.len() != before;
        for group in &mut self.groups {
            changed |= group.advance(now, fade);
        }
        changed
    }
}

/// Command groups whose hidden invocations and descendants have been removed.
///
/// Every display reader takes this view, including session column measurement.
pub(crate) struct VisibleRoster {
    /// Retained groups with only visible rows and their contributions.
    groups:        Vec<TrackedGroup>,
    /// Session widths measured after hidden rows leave.
    column_widths: SessionColumnWidths,
}

impl VisibleRoster {
    /// Visible groups, retaining their scan order and fading state.
    pub(crate) fn groups(&self) -> &[TrackedGroup] { &self.groups }

    /// Widths grown only by visible invocations.
    pub(crate) const fn column_widths(&self) -> SessionColumnWidths { self.column_widths }

    /// Visible command cells, oldest work first.
    pub(crate) fn tiled_ids(&self) -> Vec<InvocationId> {
        let mut groups: Vec<_> = self
            .groups
            .iter()
            .filter(|group| !group.lead.is_ended())
            .collect();
        groups.sort_by_key(|group| group.began());
        groups.into_iter().map(|group| group.id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;
    use sysinfo::Pid;

    use super::*;
    use crate::birth_stamp::LifetimeEvidence;
    use crate::birth_stamp::ProcessLifetime;
    use crate::census::CompilerObservation;
    use crate::census::ProcessOwner;
    use crate::census::RowProvenance;
    use crate::census::RunStart;
    use crate::census::command_text::CommandText;
    use crate::census::invocation_cpu_accounting::MeasurementAbsence;
    use crate::census::process_identity::CaptureMembership;
    use crate::census::process_identity::ProcessIdentities;
    use crate::census::process_identity::ProcessIdentity;
    use crate::census::scan::AncestryWalk;
    use crate::census::scan::InvocationResourceUse;
    use crate::constants::DEFAULT_FADE_SECONDS;
    use crate::constants::DEFAULT_HIDDEN_COMMANDS;
    use crate::constants::MIN_FINISHED_ROW_CONTRAST;
    use crate::constants::PORT_SUBCOMMAND_NAME;
    use crate::constants::TEST_INVOCATION_PID;
    use crate::constants::TEST_REPLACEMENT_LIFETIME;
    use crate::progress::capture_read::CaptureLookup;
    use crate::registration::WorkingDirectoryIdentity;

    #[test]
    fn a_forwarding_join_leaves_one_cell_for_the_process() {
        let mut roster = Roster::new();
        let now = Instant::now();
        let first = PublishedInvocations::fixture(vec![group(4100, &[])], &[], &[], now);
        roster.observe_invocations(&first);
        let joined = PublishedInvocations::fixture(vec![group(4200, &[4100])], &[], &[], now);
        roster.observe_invocations(&joined);
        assert_eq!(roster.groups().len(), 1);
        assert_eq!(roster.groups()[0].id, InvocationId::for_test(4200));
        assert_eq!(
            roster.groups()[0]
                .rows()
                .filter(|row| row.process.pid == 4100)
                .count(),
            1
        );
    }

    /// A process replacement starts its own row while the old row finishes fading.
    #[test]
    fn reused_pids_keep_separate_rows_and_family_colours() {
        let mut roster = Roster::new();
        let mut first = group(TEST_INVOCATION_PID, &[]);
        first.lead.managed = Measurement::Reading(1);
        let mut replacement = first.clone();
        replacement.lead.invocation_id = InvocationId::Process(ProcessIdentity::Known {
            pid:      TEST_INVOCATION_PID,
            lifetime: ProcessLifetime::for_test(TEST_REPLACEMENT_LIFETIME),
        });
        roster.observe(vec![first.clone()], start());
        let family = roster.visible(&[]).groups()[0].lead.family();

        roster.observe(vec![replacement.clone()], start());

        assert_eq!(roster.visible(&[]).groups().len(), 2);
        assert_eq!(roster.visible(&[]).groups()[0].id, first.id());
        assert_eq!(roster.visible(&[]).groups()[1].id, replacement.id());
        assert!(roster.visible(&[]).groups()[0].lead.is_ended());
        assert!(!roster.visible(&[]).groups()[1].lead.is_ended());
        assert_eq!(roster.visible(&[]).groups()[0].lead.family(), family);
        assert_ne!(roster.visible(&[]).groups()[1].lead.family(), family);
    }

    /// Reusing a child pid must finish the old row even when its lead survives.
    #[test]
    fn reused_child_pids_keep_separate_rows_within_one_group() {
        let mut roster = Roster::new();
        let first = family(TEST_INVOCATION_PID, &[TEST_INVOCATION_PID + 1]);
        let mut replacement = first.clone();
        replacement.rest[0].invocation_id = InvocationId::Process(ProcessIdentity::Known {
            pid:      replacement.rest[0].pid,
            lifetime: ProcessLifetime::for_test(TEST_REPLACEMENT_LIFETIME),
        });
        roster.observe(vec![first.clone()], start());
        roster.observe(vec![replacement.clone()], start());

        let visible = roster.visible(&[]);
        let retained = &visible.groups()[0];
        assert_eq!(retained.rest.len(), 2);
        assert_eq!(retained.rest[0].process, first.rest[0]);
        assert_eq!(retained.rest[1].process, replacement.rest[0]);
        assert!(retained.rest[0].is_ended());
        assert!(!retained.rest[1].is_ended());
        assert_eq!(
            retained.rest[0].parent_family(),
            retained.rest[1].parent_family()
        );
    }

    /// A fading child's link still proves a family when the next count is unavailable.
    #[test]
    fn an_unavailable_count_can_establish_a_family_from_a_retained_child() {
        let mut first = family(TEST_INVOCATION_PID, &[TEST_INVOCATION_PID + 1]);
        first.lead.managed = Measurement::Reading(0);
        let mut next = first.clone();
        next.rest.clear();
        next.lead.managed = Measurement::Unavailable(MeasurementAbsence::ReadFailed);
        let mut roster = Roster::new();
        roster.observe(vec![first], start());
        assert_eq!(
            roster.visible(&[]).groups()[0].lead.family(),
            FamilyHead::NoChildren
        );

        roster.observe(vec![next], start());

        let visible = roster.visible(&[]);
        let retained = &visible.groups()[0];
        assert!(retained.rest[0].is_ended());
        assert!(
            matches!((retained.lead.family(), retained.rest[0].parent_family()),
            (FamilyHead::Heads(head), ParentFamily::Member(member)) if head == member)
        );
    }

    /// Repeated unreadable births preserve one live row during continuous pid presence.
    #[test]
    fn unavailable_lifetimes_keep_a_continuously_present_row() {
        let pid = Pid::from_u32(TEST_INVOCATION_PID);
        let lifetimes = HashMap::from([(pid, LifetimeEvidence::Unavailable)]);
        let mut identities = ProcessIdentities::default();
        let mut first = group(TEST_INVOCATION_PID, &[]);
        first.lead.invocation_id = identities.observe(&lifetimes)[&pid].clone();
        first.lead.managed = Measurement::Reading(1);
        let mut roster = Roster::new();
        roster.observe(vec![first.clone()], start());
        let family = roster.visible(&[]).groups()[0].lead.family();

        let mut refreshed = first.clone();
        refreshed.lead.invocation_id = identities.observe(&lifetimes)[&pid].clone();
        refreshed.lead.managed = Measurement::Unavailable(MeasurementAbsence::ReadFailed);
        roster.observe(vec![refreshed.clone()], start());

        assert_eq!(roster.visible(&[]).groups().len(), 1);
        let visible = roster.visible(&[]);
        let retained = &visible.groups()[0];
        assert_eq!(retained.id, first.id());
        assert_eq!(retained.lead.process, refreshed.lead);
        assert_eq!(retained.lead.family(), family);
        assert!(!retained.lead.is_ended());
    }

    /// An absent scan ends unavailable continuity before that pid appears again.
    #[test]
    fn unavailable_lifetimes_never_merge_retained_rows() {
        let pid = Pid::from_u32(TEST_INVOCATION_PID);
        let lifetimes = HashMap::from([(pid, LifetimeEvidence::Unavailable)]);
        let mut identities = ProcessIdentities::default();
        let mut first = group(TEST_INVOCATION_PID, &[]);
        first.lead.invocation_id = identities.observe(&lifetimes)[&pid].clone();
        let mut roster = Roster::new();
        roster.observe(vec![first.clone()], start());
        assert!(identities.observe(&HashMap::new()).is_empty());
        identities.finish(&crate::census::process_identity::ProvenProcessEnd::for_test(pid));
        roster.observe(Vec::new(), start());
        assert!(roster.visible(&[]).groups()[0].lead.is_ended());

        let mut replacement = first.clone();
        replacement.lead.invocation_id = identities.observe(&lifetimes)[&pid].clone();
        roster.observe(vec![replacement.clone()], start());

        assert_eq!(roster.visible(&[]).groups().len(), 2);
        assert_ne!(first.id(), replacement.id());
        assert_eq!(roster.visible(&[]).groups()[0].lead.process, first.lead);
        assert_eq!(
            roster.visible(&[]).groups()[1].lead.process,
            replacement.lead
        );
        assert!(roster.visible(&[]).groups()[0].lead.is_ended());
        assert!(!roster.visible(&[]).groups()[1].lead.is_ended());
    }

    /// A scan can change the displayed process without replacing the invocation.
    #[test]
    fn displayed_pid_changes_preserve_the_invocation_row_and_family() {
        let mut first = group(TEST_INVOCATION_PID, &[]);
        first.lead.managed = Measurement::Reading(1);
        let mut refreshed = first.clone();
        refreshed.lead.pid += 1;
        let mut roster = Roster::new();
        roster.observe(vec![first], start());
        let family = roster.visible(&[]).groups()[0].lead.family();

        roster.observe(vec![refreshed.clone()], start());

        assert_eq!(roster.visible(&[]).groups().len(), 1);
        assert_eq!(roster.visible(&[]).groups()[0].lead.process, refreshed.lead);
        assert_eq!(roster.visible(&[]).groups()[0].lead.family(), family);
        assert!(!roster.visible(&[]).groups()[0].lead.is_ended());
    }

    /// The colour is a tie between a row and the rows under it, so a
    /// row's `parent` stamp has to be the very index its parent's own
    /// row carries. Anything else pairs the eye off against nothing.
    #[test]
    fn a_child_carries_the_index_its_parent_was_given() {
        let mut roster = Roster::new();

        roster.observe(vec![family(64432, &[4003, 4058])], Instant::now());

        let stamped = stamps(&mut roster);
        let lead = stamped
            .iter()
            .find(|(pid, ..)| *pid == 64432)
            .map(|(_, family, _)| *family);
        assert!(matches!(lead, Some(FamilyHead::Heads(_))), "{stamped:?}");
        for (pid, _, parent) in &stamped {
            if *pid == 64432 {
                continue;
            }
            assert!(
                matches!((lead, parent), (Some(FamilyHead::Heads(head)), ParentFamily::Member(member)) if head == *member),
                "{stamped:?}"
            );
        }
    }

    /// Two families on screen at once must not share a colour. A
    /// function of the pid alone cannot promise that -- the first two
    /// families the display was shown collided -- so the indices are
    /// handed out against what is already taken.
    #[test]
    fn two_families_are_never_given_one_colour() {
        let mut roster = Roster::new();

        roster.observe(
            vec![family(64432, &[4003]), family(70001, &[4058])],
            Instant::now(),
        );

        let heads: Vec<FamilyHead> = stamps(&mut roster)
            .into_iter()
            .filter(|(pid, ..)| *pid == 64432 || *pid == 70001)
            .map(|(_, family, _)| family)
            .collect();
        assert_eq!(heads.len(), 2);
        assert_ne!(heads[0], heads[1]);
    }

    /// A colour that moved when an unrelated command finished would be
    /// no use for pairing rows off, and a build finishing is the most
    /// routine thing on this screen.
    #[test]
    fn a_family_keeps_its_colour_when_another_one_ends() {
        let mut roster = Roster::new();
        roster.observe(
            vec![family(64432, &[4003]), family(70001, &[4058])],
            Instant::now(),
        );
        let before = stamps(&mut roster)
            .into_iter()
            .find(|(pid, ..)| *pid == 70001)
            .map(|(_, family, _)| family);

        roster.observe(vec![family(70001, &[4058])], Instant::now());

        let after = stamps(&mut roster)
            .into_iter()
            .find(|(pid, ..)| *pid == 70001)
            .map(|(_, family, _)| family);
        assert_eq!(before, after);
    }

    #[test]
    fn surviving_families_take_distinct_colours_when_the_theme_palette_shrinks() {
        let previous_theme = tui_pane::theme();
        let variants = theme::builtins::builtins();
        let dark = &variants[0];
        let light = &variants[1];
        assert_eq!(dark.id.as_str(), crate::constants::DEFAULT_DARK_THEME);
        assert_eq!(light.id.as_str(), crate::constants::DEFAULT_LIGHT_THEME);
        tui_pane::set_active_theme(std::sync::Arc::new(dark.theme.clone()));
        let now = start();
        let fade = Duration::from_secs(DEFAULT_FADE_SECONDS);
        let mut roster = Roster::new();
        let scan = vec![
            family(10, &[11]),
            family(20, &[21]),
            family(30, &[31]),
            family(40, &[41]),
        ];
        roster.observe(scan.clone(), now);
        roster.visible(&[]);
        let first_index = roster.families[&scan[0].id()];
        let survivors = vec![scan[0].clone(), scan[3].clone()];
        roster.observe(survivors.clone(), now);
        roster.advance(now + fade, fade);
        roster.visible(&[]);
        let colours_before = [
            theme::family_color(roster.families[&survivors[0].id()]),
            theme::family_color(roster.families[&survivors[1].id()]),
        ];
        let mut smaller_light = light.theme.clone();
        let first_colour = smaller_light.roles[crate::constants::FAMILY_BLUE_ROLE];
        for role in [
            crate::constants::FAMILY_CYAN_ROLE,
            crate::constants::FAMILY_ORANGE_ROLE,
            crate::constants::FAMILY_VIOLET_ROLE,
            crate::constants::FAMILY_YELLOW_ROLE,
            crate::constants::FAMILY_RED_ROLE,
        ] {
            smaller_light.roles.insert(role.to_owned(), first_colour);
        }
        tui_pane::set_active_theme(std::sync::Arc::new(smaller_light));
        roster.observe(survivors.clone(), now + fade);
        roster.visible(&[]);
        let colours_after = [
            theme::family_color(roster.families[&survivors[0].id()]),
            theme::family_color(roster.families[&survivors[1].id()]),
        ];
        let retained_index = roster.families[&survivors[0].id()];
        roster.observe(survivors.clone(), now + fade);
        roster.visible(&[]);
        let colours_next_scan = [
            theme::family_color(roster.families[&survivors[0].id()]),
            theme::family_color(roster.families[&survivors[1].id()]),
        ];
        tui_pane::set_active_theme(previous_theme);

        assert_ne!(colours_before[0], colours_before[1]);
        assert_ne!(colours_after[0], colours_after[1]);
        assert_eq!(retained_index, first_index);
        assert_eq!(colours_next_scan, colours_after);
    }

    /// A command with nothing running under it has nothing pointing at
    /// it, so a colour on its pid would be a mark meaning nothing.
    #[test]
    fn a_command_with_no_children_is_given_no_colour() {
        let mut roster = Roster::new();

        roster.observe(vec![group(64432, &[])], Instant::now());

        assert_eq!(
            stamps(&mut roster),
            [(64432, FamilyHead::NoChildren, ParentFamily::NoFamily)]
        );
    }

    /// An unknown count alone supplies no evidence that a family exists.
    #[test]
    fn an_unknown_managed_count_does_not_create_a_family_without_evidence() {
        let mut roster = Roster::new();
        let mut group = group(64432, &[]);
        group.lead.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);

        roster.observe(vec![group], start());

        assert_eq!(
            stamps(&mut roster),
            [(64432, FamilyHead::NoChildren, ParentFamily::NoFamily)]
        );
    }

    /// A child's parent link proves the family even when its size is unknown.
    #[test]
    fn an_unknown_managed_count_assigns_a_family_when_a_child_names_its_parent() {
        let mut roster = Roster::new();
        let mut group = family(64432, &[4003]);
        group.lead.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);

        roster.observe(vec![group], start());

        let lead = roster.visible(&[]).groups()[0].lead.family();
        assert!(matches!(lead, FamilyHead::Heads(_)));
        assert!(
            matches!((lead, roster.visible(&[]).groups()[0].rest[0].parent_family()), (FamilyHead::Heads(head), ParentFamily::Member(member)) if head == member)
        );
    }

    /// Losing a count does not prove that an established family has ended.
    #[test]
    fn an_unknown_managed_count_keeps_an_established_family_without_a_child_link() {
        let mut roster = Roster::new();
        let mut group = group(64432, &[]);
        group.lead.managed = Measurement::Reading(1);
        roster.observe(vec![group.clone()], start());
        let before = roster.visible(&[]).groups()[0].lead.family();
        assert!(matches!(before, FamilyHead::Heads(_)));

        group.lead.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);
        assert!(roster.observe(vec![group.clone()], start()));

        assert_eq!(roster.visible(&[]).groups()[0].lead.family(), before);
        assert!(!roster.observe(vec![group], start()));
    }

    /// A measured zero ends the family retained during an unavailable scan.
    #[test]
    fn a_measured_zero_releases_a_family_after_an_unknown_managed_count() {
        let mut roster = Roster::new();
        let mut group = group(64432, &[]);
        group.lead.managed = Measurement::Reading(1);
        roster.observe(vec![group.clone()], start());
        assert!(matches!(
            roster.visible(&[]).groups()[0].lead.family(),
            FamilyHead::Heads(_)
        ));
        group.lead.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);
        roster.observe(vec![group.clone()], start());
        assert!(matches!(
            roster.visible(&[]).groups()[0].lead.family(),
            FamilyHead::Heads(_)
        ));

        group.lead.managed = Measurement::Reading(0);
        assert!(roster.observe(vec![group], start()));

        assert_eq!(
            stamps(&mut roster),
            [(64432, FamilyHead::NoChildren, ParentFamily::NoFamily)]
        );
        assert!(roster.families.is_empty());
    }

    #[test]
    fn changed_display_pid_and_unavailable_start_keep_rows_and_family_live() {
        let mut roster = Roster::new();
        let mut registration = family(10, &[12]);
        registration.lead.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);
        registration.lead.started = RunStart::Unavailable;
        roster.observe(vec![registration.clone()], start());
        let identity = roster.visible(&[]).groups()[0].id.clone();
        let family = roster.visible(&[]).groups()[0].lead.family();
        let mut process = registration.clone();
        process.lead.pid = 11;
        process.lead.started = RunStart::Known(100);
        process.lead.managed = Measurement::Reading(1);
        process.rest[0].parent = VisibleParent::Invocation {
            id:  identity.clone(),
            pid: 11,
        };
        for scan in [process, registration] {
            roster.observe(vec![scan], start());
            assert_eq!(roster.visible(&[]).groups().len(), 1);
            let visible = roster.visible(&[]);
            let tracked = &visible.groups()[0];
            assert_eq!(tracked.id, identity);
            assert_eq!(tracked.lead.family(), family);
            assert_eq!(tracked.rows().count(), 2);
            assert!(tracked.rows().all(|row| !row.is_ended()));
        }
    }

    /// A process row carrying nothing but the pid the tests key on.
    fn process(pid: u32) -> CargoProcess {
        CargoProcess {
            path: "~/rust/project".to_string(),
            directory_identity: WorkingDirectoryIdentity::Absolute(
                "/test-home/rust/project".into(),
            ),
            pid,
            invocation_id: InvocationId::for_test(pid),
            capture_membership: CaptureMembership::Outside,
            provenance: RowProvenance::Uncaptured(ProcessOwner::Unavailable),
            parent: VisibleParent::None,
            start: "10:00".to_string(),
            started: RunStart::Known(0),
            duration: "00:01".to_string(),
            resource_use: InvocationResourceUse {
                cpu:      Measurement::Reading(0.0),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::None,
            },
            subtree_resource_use: InvocationResourceUse {
                cpu:      Measurement::Reading(0.0),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::None,
            },
            state: CaptureLookup::Unregistered,
            managed: Measurement::Reading(0),
            nested: false,
            command: CommandText::of("cargo", &["build"]),
        }
    }

    /// A group led by `lead` with one invocation per entry of `rest`.
    fn group(lead: u32, rest: &[u32]) -> CargoGroup {
        CargoGroup {
            lead:     process(lead),
            rest:     rest.iter().copied().map(process).collect(),
            ancestry: AncestryWalk::Ended(Vec::new()),
        }
    }

    /// The same, with the lead owning every invocation under it -- what
    /// the census reports for a command actually driving cargo, and
    /// what makes the lead a family.
    fn family(lead: u32, rest: &[u32]) -> CargoGroup {
        let mut group = group(lead, rest);
        group.lead.managed = Measurement::Reading(rest.len());
        for row in &mut group.rest {
            row.parent = VisibleParent::Invocation {
                id:  group.lead.invocation_id.clone(),
                pid: lead,
            };
        }
        group
    }

    /// The palette index every row of `roster` came out with, as
    /// `(pid, own family, parent's family)`.
    fn stamps(roster: &mut Roster) -> Vec<(u32, FamilyHead, ParentFamily)> {
        roster
            .visible(&[])
            .groups()
            .iter()
            .flat_map(TrackedGroup::rows)
            .map(|row| (row.process.pid, row.family(), row.parent_family()))
            .collect()
    }

    /// The same, led by a command `commands.hidden` names.
    fn hidden_group(lead: u32, rest: &[u32]) -> CargoGroup {
        let mut group = group(lead, rest);
        group.lead.command = CommandText::of("cargo", &[PORT_SUBCOMMAND_NAME]);
        group
    }

    /// The list as it reaches the roster once the config has turned it
    /// into owned strings.
    fn hidden_commands() -> Vec<String> {
        DEFAULT_HIDDEN_COMMANDS
            .iter()
            .map(|subcommand| (*subcommand).to_string())
            .collect()
    }

    /// The same group with start times stamped on it, lead first --
    /// what the cells are ordered by.
    fn started(mut group: CargoGroup, lead: u64, rest: &[u64]) -> CargoGroup {
        group.lead.started = RunStart::Known(lead);
        for (row, start) in group.rest.iter_mut().zip(rest) {
            row.started = RunStart::Known(*start);
        }
        group
    }

    /// The instant every test starts from.
    fn start() -> Instant { Instant::now() }

    #[test]
    fn a_group_the_scan_stops_carrying_is_stamped_rather_than_dropped() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[])], now);
        roster.observe(Vec::new(), now);

        assert_eq!(roster.visible(&[]).groups().len(), 1);
        assert!(roster.visible(&[]).groups()[0].lead.is_ended());
    }

    #[test]
    fn an_ended_group_withdraws_while_its_last_rows_are_still_readable() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[])], now);
        assert_eq!(roster.visible(&[]).tiled_ids().len(), 1);

        roster.observe(Vec::new(), now);
        let visible = roster.visible(&[]);
        assert_eq!(visible.tiled_ids(), Vec::<InvocationId>::new());
        let lead = &visible.groups()[0].lead;
        assert!(lead.is_ended());
        assert_eq!(lead.faded(), 0);
        assert!(
            matches!(visible.groups()[0].departure(Duration::from_secs(3)),
            tui_pane::CellDeparture::Fading { started, timing }
            if started == now && timing.duration() == Duration::from_secs(3))
        );
    }

    #[test]
    fn a_stamped_group_goes_once_the_fade_has_run() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[])], now);
        roster.observe(Vec::new(), now);

        assert!(!roster.advance(now, Duration::from_secs(3)));
        assert!(roster.advance(now + Duration::from_secs(3), Duration::from_secs(3)));
        assert!(roster.visible(&[]).groups().is_empty());
    }

    /// A later live answer cannot revoke a published end for the same invocation.
    #[test]
    fn an_ended_group_cannot_be_revived_by_a_later_live_answer() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[])], now);
        roster.observe(Vec::new(), now);
        roster.observe(vec![group(10, &[])], now);

        assert!(roster.visible(&[]).groups()[0].lead.is_ended());
    }

    #[test]
    fn a_sub_command_that_finishes_fades_without_taking_the_group() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[11, 12])], now);
        roster.observe(vec![group(10, &[12])], now);

        let visible = roster.visible(&[]);
        let tracked = &visible.groups()[0];
        assert!(!tracked.lead.is_ended(), "the command is still running");
        let ended: Vec<u32> = tracked
            .rest
            .iter()
            .filter(|row| row.is_ended())
            .map(|row| row.process.pid)
            .collect();
        assert_eq!(ended, vec![11]);
    }

    #[test]
    fn a_finished_sub_command_is_dropped_once_it_has_faded() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[11, 12])], now);
        roster.observe(vec![group(10, &[12])], now);
        roster.advance(now + Duration::from_secs(3), Duration::from_secs(3));

        let remaining: Vec<u32> = roster.visible(&[]).groups()[0]
            .rest
            .iter()
            .map(|row| row.process.pid)
            .collect();
        assert_eq!(remaining, vec![12]);
    }

    /// A sub-command finishing must not pull the rows under it up the
    /// tile: it fades where it sat.
    #[test]
    fn a_finished_sub_command_keeps_its_place_while_it_fades() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[11, 12, 13])], now);
        roster.observe(vec![group(10, &[11, 13])], now);

        let order: Vec<u32> = roster.visible(&[]).groups()[0]
            .rest
            .iter()
            .map(|row| row.process.pid)
            .collect();
        assert_eq!(order, vec![11, 12, 13]);
    }

    #[test]
    fn a_new_sub_command_joins_the_group_it_runs_under() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[11])], now);
        roster.observe(vec![group(10, &[11, 12])], now);

        assert_eq!(roster.visible(&[]).groups()[0].rest.len(), 2);
        assert!(
            roster.visible(&[]).groups()[0]
                .rows()
                .all(|row| !row.is_ended())
        );
    }

    /// A scan identical to the last one must not repaint the display:
    /// with nothing building, that is every scan.
    #[test]
    fn an_unchanged_scan_reports_nothing_to_redraw() {
        let mut roster = Roster::new();
        let now = start();
        assert!(roster.observe(vec![group(10, &[11])], now));
        assert!(!roster.observe(vec![group(10, &[11])], now));
    }

    /// The fade runs from when the process stopped, not from the scan
    /// that noticed it was still gone.
    #[test]
    fn a_second_absent_scan_does_not_restart_the_fade() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[])], now);
        roster.observe(Vec::new(), now);
        roster.observe(Vec::new(), now + Duration::from_secs(2));

        assert!(roster.advance(now + Duration::from_secs(3), Duration::from_secs(3)));
    }

    /// The fade is a colour walking toward the ground, so the roster
    /// has to report a repaint on every step of it rather than only on
    /// the row it finally lets go of.
    #[test]
    fn every_step_of_the_fade_asks_for_a_repaint() {
        let mut roster = Roster::new();
        let now = start();
        let fade = Duration::from_secs(3);
        roster.observe(vec![group(10, &[])], now);
        roster.observe(Vec::new(), now);

        assert!(roster.advance(now + Duration::from_millis(500), fade));
        assert!(roster.advance(now + Duration::from_millis(1000), fade));
    }

    #[test]
    fn a_finished_row_stays_readable_through_the_first_half_of_its_fade() {
        let now = start();
        let fade = Duration::from_secs(DEFAULT_FADE_SECONDS);
        let mut row = TrackedRow::from(group(TEST_INVOCATION_PID, &[]).lead);
        row.finish(now);
        let mut steps: Vec<_> = (0..=fade.as_millis())
            .map(|elapsed| {
                let elapsed = Duration::from_millis(u64::try_from(elapsed).unwrap_or(u64::MAX));
                (elapsed, row.faded_at(now + elapsed, fade))
            })
            .collect();
        steps.dedup_by_key(|(_, alpha)| *alpha);
        let previous_theme = tui_pane::theme();
        let previous_transparency = tui_pane::transparent_background();
        let previous_tint = tui_pane::focused_pane_tint_enabled();
        tui_pane::set_focused_pane_tint(true);
        for variant in theme::builtins::builtins() {
            tui_pane::set_active_theme(std::sync::Arc::new(variant.theme.clone()));
            let mut inks = vec![
                tui_pane::text_default(),
                tui_pane::label_color(),
                tui_pane::success_color(),
                tui_pane::warning_color(),
                tui_pane::secondary_text_color(),
            ];
            inks.extend(variant.theme.roles.values().map(|style| style.color));
            for transparent in [false, true] {
                tui_pane::set_transparent_background(transparent);
                for focused in [false, true] {
                    let ground = tui_pane::pane_background(focused);
                    for ink in &inks {
                        for elapsed in [Duration::ZERO, fade / 2, fade] {
                            let drawn = tui_pane::blend_color(
                                *ink,
                                ground,
                                row.faded_at(now + elapsed, fade),
                            );
                            let contrast = fade_contrast(drawn, ground);
                            if elapsed <= fade / 2 {
                                assert!(
                                    contrast >= MIN_FINISHED_ROW_CONTRAST,
                                    "{} {ink:?} on {ground:?} at {elapsed:?}: {contrast:.3}:1, transparent={transparent}, focused={focused}",
                                    variant.id.as_str()
                                );
                            } else {
                                assert_eq!(drawn, tui_pane::blend_color(ground, ground, 0));
                            }
                        }
                        let mut previous_contrast = fade_contrast(*ink, ground);
                        for (elapsed, alpha) in &steps {
                            let contrast =
                                fade_contrast(tui_pane::blend_color(*ink, ground, *alpha), ground);
                            assert!(
                                contrast <= previous_contrast,
                                "{} {ink:?} on {ground:?} grows in contrast at alpha {alpha}: {previous_contrast:.6} -> {contrast:.6}",
                                variant.id.as_str()
                            );
                            if *elapsed <= fade / 2 {
                                assert!(
                                    contrast >= MIN_FINISHED_ROW_CONTRAST,
                                    "{} {ink:?} on {ground:?} at {elapsed:?}: {contrast:.3}:1",
                                    variant.id.as_str()
                                );
                            }
                            previous_contrast = contrast;
                        }
                    }
                }
            }
        }
        tui_pane::set_active_theme(previous_theme);
        tui_pane::set_transparent_background(previous_transparency);
        tui_pane::set_focused_pane_tint(previous_tint);
        let mut previous = 0;
        for elapsed in 0..=fade.as_millis() {
            let elapsed = Duration::from_millis(u64::try_from(elapsed).unwrap_or(u64::MAX));
            let alpha = row.faded_at(now + elapsed, fade);
            assert!(alpha >= previous);
            previous = alpha;
        }
        assert_eq!(previous, u8::MAX);
        assert!(row.is_expired(now + fade, fade));
    }

    fn fade_contrast(ink: Color, ground: Color) -> f64 {
        let ink = fade_luminance(ink);
        let ground = fade_luminance(ground);
        (ink.max(ground) + 0.05) / (ink.min(ground) + 0.05)
    }

    fn fade_luminance(color: Color) -> f64 {
        let color = tui_pane::blend_color(color, Color::Black, 0);
        let (red, green, blue) = match color {
            Color::Rgb(red, green, blue) => (red, green, blue),
            _ => (0, 0, 0),
        };
        assert_eq!(
            color,
            Color::Rgb(red, green, blue),
            "a built-in ink must have measurable channels"
        );
        let linear = |channel: u8| {
            let channel = f64::from(channel) / f64::from(u8::MAX);
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126f64.mul_add(
            linear(red),
            0.7152f64.mul_add(linear(green), 0.0722 * linear(blue)),
        )
    }

    /// The step the fade stands on is what the render layer carries the
    /// row's colour by, so it has to track the spell rather than the
    /// scans inside it.
    #[test]
    fn the_fade_travels_with_the_spell_it_was_given() {
        let mut roster = Roster::new();
        let now = start();
        let fade = Duration::from_secs(4);
        roster.observe(vec![group(10, &[])], now);
        roster.observe(Vec::new(), now);

        assert_eq!(
            roster.visible(&[]).groups()[0].lead.faded(),
            0,
            "stamped, not yet gone"
        );
        roster.advance(now + Duration::from_secs(1), fade);
        let quarter = roster.visible(&[]).groups()[0].lead.faded();
        assert!(quarter > 0 && quarter < u8::MAX / 4);
        roster.advance(now + Duration::from_secs(2), fade);
        let midpoint = roster.visible(&[]).groups()[0].lead.faded();
        assert!(midpoint > quarter && midpoint < u8::MAX / 2);
        let longer_fade = fade * 2;
        let visible = roster.visible(&[]);
        let row = &visible.groups()[0].lead;
        assert_eq!(row.faded_at(now + longer_fade / 4, longer_fade), quarter);
        assert_eq!(row.faded_at(now + longer_fade / 2, longer_fade), midpoint);
        assert_eq!(row.faded_at(now + fade, fade), u8::MAX);
    }

    /// A row still in the scan is drawn in its own colours, however
    /// long the display has held it.
    #[test]
    fn a_running_row_never_fades() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[])], now);
        roster.advance(now + Duration::from_secs(2), Duration::from_secs(3));

        assert_eq!(roster.visible(&[]).groups()[0].lead.faded(), 0);
    }

    /// A delayed live answer cannot rewind an ended invocation's fade.
    #[test]
    fn an_ended_group_keeps_its_fade_after_a_delayed_live_answer() {
        let mut roster = Roster::new();
        let now = start();
        let fade = Duration::from_secs(3);
        roster.observe(vec![group(10, &[])], now);
        roster.observe(Vec::new(), now);
        roster.advance(now + Duration::from_secs(2), fade);
        let faded = roster.visible(&[]).groups()[0].lead.faded();
        assert_ne!(faded, 0, "the fade had started");

        roster.observe(vec![group(10, &[])], now + Duration::from_secs(2));

        assert_eq!(roster.visible(&[]).groups()[0].lead.faded(), faded);
    }

    #[test]
    fn a_hidden_command_gets_no_cell_with_nothing_under_it() {
        let mut roster = Roster::new();
        roster.observe(vec![hidden_group(10, &[])], start());

        assert_eq!(
            roster.tiled_ids(&hidden_commands()),
            [] as [InvocationId; 0]
        );
        assert!(roster.visible(&hidden_commands()).groups().is_empty());
    }

    #[test]
    fn a_hidden_command_gets_no_cell_while_it_drives_something() {
        let mut roster = Roster::new();
        roster.observe(vec![hidden_group(10, &[11])], start());
        let visible = roster.visible(&hidden_commands());
        assert_eq!(visible.tiled_ids(), Vec::<InvocationId>::new());
        assert!(visible.groups().is_empty());
    }

    #[test]
    fn a_hidden_command_stays_hidden_while_its_children_fade() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![hidden_group(10, &[11])], now);
        roster.observe(vec![hidden_group(10, &[])], now);
        assert!(roster.visible(&hidden_commands()).groups().is_empty());
        roster.advance(now + Duration::from_secs(1), Duration::ZERO);
        assert!(roster.visible(&hidden_commands()).groups().is_empty());
    }

    #[test]
    fn an_unlisted_driver_is_ordered_by_its_own_start() {
        let mut roster = Roster::new();
        roster.observe(
            vec![
                started(hidden_group(10, &[11]), 0, &[200]),
                started(group(20, &[]), 100, &[]),
            ],
            start(),
        );
        assert_eq!(
            roster.visible(&[]).tiled_ids(),
            vec![InvocationId::for_test(10), InvocationId::for_test(20)]
        );
    }

    #[test]
    fn cells_are_ordered_by_start_rather_than_by_arrival() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![started(group(10, &[]), 200, &[])], now);
        roster.observe(
            vec![
                started(group(10, &[]), 200, &[]),
                started(group(20, &[]), 100, &[]),
            ],
            now,
        );

        assert_eq!(
            roster.tiled_ids(&hidden_commands()),
            vec![20, 10]
                .into_iter()
                .map(InvocationId::for_test)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_command_off_the_list_gets_a_cell_with_nothing_under_it() {
        let mut roster = Roster::new();
        roster.observe(vec![group(10, &[])], start());

        assert_eq!(
            roster.tiled_ids(&hidden_commands()),
            vec![10]
                .into_iter()
                .map(InvocationId::for_test)
                .collect::<Vec<_>>()
        );
    }
}
