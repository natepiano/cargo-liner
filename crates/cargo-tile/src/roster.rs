//! What the display holds between scans: the groups running now, and the
//! ones that have finished but have not yet faded.
//!
//! A scan is a snapshot of what is running, so on its own it cannot say
//! that something *stopped* -- a finished command is simply missing from
//! the next one. The roster keeps the previous answer beside the new one
//! and stamps whatever went away, which is what gives a finished row the
//! grey spell it fades through before the display lets go of it.
//!
//! Every entry is keyed by pid, so a row and its tile survive a scan
//! unchanged rather than being rebuilt four times a second.

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;

use crate::processes::Ancestor;
use crate::processes::CargoGroup;
use crate::processes::CargoProcess;
use crate::theme;

/// One table row, once it has stopped when that happened, and how far
/// its text has since been carried toward the ground it is drawn on.
pub(crate) struct TrackedRow {
    /// The invocation as the last scan that carried it described it.
    pub(crate) process: CargoProcess,
    /// When the process left the scan, or `None` while it runs.
    ended:              Option<Instant>,
    /// How far through the fade the row stands, on the alpha scale
    /// [`blend_color`](tui_pane::blend_color) reads: zero while it
    /// runs, [`u8::MAX`] once it has reached the ground and is about to
    /// be let go of.
    faded:              u8,
    /// The family this invocation heads, where it has cargo running
    /// under it, as an index into the palette. `None` on a row nothing
    /// points at.
    family:             Option<usize>,
    /// The family of the cargo this invocation is running under, which
    /// is the index that cargo's own row carries. `None` where nothing
    /// above it started it.
    parent_family:      Option<usize>,
}

impl From<CargoProcess> for TrackedRow {
    /// A row for a process the current scan carries.
    fn from(process: CargoProcess) -> Self {
        Self {
            process,
            ended: None,
            faded: 0,
            // Stamped by the roster once the whole scan is in: which
            // colours are free is a question about every group at
            // once, not about this row.
            family: None,
            parent_family: None,
        }
    }
}

impl TrackedRow {
    /// Whether the process behind this row has stopped.
    pub(crate) const fn is_ended(&self) -> bool { self.ended.is_some() }

    /// How far the row's text has been carried toward the ground.
    pub(crate) const fn faded(&self) -> u8 { self.faded }

    /// The family this invocation heads, where it heads one.
    pub(crate) const fn family(&self) -> Option<usize> { self.family }

    /// The family of the cargo this invocation runs under.
    pub(crate) const fn parent_family(&self) -> Option<usize> { self.parent_family }

    /// Take the scan's account of a process that is still running.
    fn refresh(&mut self, process: CargoProcess) {
        self.process = process;
        self.ended = None;
        self.faded = 0;
    }

    /// Stamp the row as finished, leaving an earlier stamp alone so the
    /// fade runs from when it actually stopped.
    fn finish(&mut self, now: Instant) { self.ended = self.ended.or(Some(now)); }

    /// Move the fade on, reporting whether it went anywhere.
    ///
    /// A row that has not stopped has nothing to move: the whole travel
    /// is measured from the stamp [`finish`](Self::finish) left.
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
        let Some(ended) = self.ended else {
            return 0;
        };
        let elapsed = now.duration_since(ended);
        if elapsed >= fade {
            return u8::MAX;
        }
        // Both fit a u32: `fade` is clamped to `MAX_FADE_SECONDS`, and
        // `elapsed` is shorter still to have reached here.
        let elapsed = u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX);
        let whole = u32::try_from(fade.as_millis()).unwrap_or(u32::MAX);
        let travelled = elapsed.saturating_mul(u32::from(u8::MAX)) / whole.max(1);
        u8::try_from(travelled).unwrap_or(u8::MAX)
    }

    /// Whether the row has been finished for longer than `fade`.
    fn is_expired(&self, now: Instant, fade: Duration) -> bool {
        self.ended
            .is_some_and(|ended| now.duration_since(ended) >= fade)
    }
}

/// One command's rows: the summary row and the invocations under it.
pub(crate) struct TrackedGroup {
    /// The group's identity, the lead's pid, stable while it runs.
    pub(crate) id:   u32,
    /// The row the summary carries.
    pub(crate) lead: TrackedRow,
    /// The invocations running under the lead, newest first.
    rest:            Vec<TrackedRow>,
    /// What stands above the command, outermost first. Refreshed with
    /// the rest of the group: a chain is read fresh every scan, and a
    /// process in it that exits is one the next scan no longer carries.
    ancestry:        Vec<Ancestor>,
}

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
    /// Every row the group's tile draws, the lead first.
    pub(crate) fn rows(&self) -> impl Iterator<Item = &TrackedRow> {
        std::iter::once(&self.lead).chain(self.rest.iter())
    }

    /// The same rows, for the pass that stamps each one's family.
    fn rows_mut(&mut self) -> impl Iterator<Item = &mut TrackedRow> {
        std::iter::once(&mut self.lead).chain(self.rest.iter_mut())
    }

    /// What stands above the command, outermost first.
    pub(crate) fn ancestry(&self) -> &[Ancestor] { &self.ancestry }

    /// Whether the command itself belongs at the foot of its cell's
    /// ancestry chain rather than in the table.
    ///
    /// True for exactly the commands `commands.hidden_when_idle`
    /// names. Those are drivers: they compile nothing, they are open
    /// all day, and their cell exists only because something is
    /// running under them. A row for the driver in that table says the
    /// same thing on every scan and takes a row from the invocations
    /// the cell was opened for -- while as the last step of the chain
    /// it says what the rest of the chain says, which is where the
    /// work came from.
    pub(crate) fn leads_as_ancestor(&self, hidden_when_idle: &[String]) -> bool {
        self.lead
            .process
            .command
            .is_hidden_when_idle(hidden_when_idle)
    }

    /// Fold in the scan's account of a group that is still running.
    ///
    /// Rows the scan no longer carries are stamped rather than dropped,
    /// and stay where they sat: a sub-command finishing should fade out
    /// of the place it occupied, not pull the rows under it upward.
    fn refresh(&mut self, group: CargoGroup, now: Instant) {
        self.lead.refresh(group.lead);
        self.ancestry = group.ancestry;
        let mut arriving = group.rest;
        for tracked in &mut self.rest {
            match arriving
                .iter()
                .position(|process| process.pid == tracked.process.pid)
            {
                Some(index) => tracked.refresh(arriving.remove(index)),
                None => tracked.finish(now),
            }
        }
        self.rest.extend(arriving.into_iter().map(TrackedRow::from));
    }

    /// Stamp the whole group as finished.
    fn finish(&mut self, now: Instant) {
        self.lead.finish(now);
        for row in &mut self.rest {
            row.finish(now);
        }
    }

    /// Drop the rows that have finished fading and move the rest of the
    /// group's fade on, reporting whether anything the display draws
    /// changed.
    fn advance(&mut self, now: Instant, fade: Duration) -> bool {
        let before = self.rest.len();
        self.rest.retain(|row| !row.is_expired(now, fade));
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

    /// Whether the grid gives this command a cell of its own.
    ///
    /// Every command does, except one whose subcommand
    /// `commands.hidden_when_idle` names while nothing is running under
    /// it. Those are the commands that stay open rather than finishing,
    /// and while one is only sitting there its cell would hold a single
    /// row with no reading, no compiler and no duration worth reading --
    /// a cell no build is getting, saying nothing the summary's one line
    /// for it does not already say. That summary line stays either way:
    /// this decides the cell alone.
    ///
    /// A row under it that has stopped but not yet faded still counts,
    /// so the cell goes out through the same fade as any other rather
    /// than vanishing the instant its work ends.
    fn deserves_a_cell(&self, hidden_when_idle: &[String]) -> bool {
        !self.rest.is_empty()
            || !self
                .lead
                .process
                .command
                .is_hidden_when_idle(hidden_when_idle)
    }
}

/// Every group the display knows about, running or fading.
#[derive(Default)]
pub(crate) struct Roster {
    /// Groups in scan order -- newest command first -- with finished
    /// ones held in place until they expire.
    groups:   Vec<TrackedGroup>,
    /// The scan last folded in, kept so an identical one can be
    /// recognised and skipped. With nothing building, the display has no
    /// reason to repaint four times a second.
    last:     Vec<CargoGroup>,
    /// The palette index each family holds, by the pid heading it.
    ///
    /// Kept here rather than worked out per cell because the colour has
    /// to be the same in the summary and in the command's own cell, and
    /// because which colours are free is a question about every group
    /// at once. An index is held for as long as the family is on screen,
    /// fading rows included, so a colour never moves under the reader.
    families: HashMap<u32, usize>,
}

impl Roster {
    /// An empty roster.
    pub(crate) fn new() -> Self {
        Self {
            groups:   Vec::new(),
            last:     Vec::new(),
            families: HashMap::new(),
        }
    }

    /// The groups to display, newest first.
    pub(crate) fn groups(&self) -> &[TrackedGroup] { &self.groups }

    /// The identity of every group that gets a cell, in order -- what
    /// [`crate::tiles::TileGrid::sync`] assigns cells from.
    ///
    /// Narrower than [`groups`](Self::groups), which the summary reads:
    /// a command held back by `commands.hidden_when_idle` keeps its
    /// summary line and is left out here.
    ///
    /// Identity only. How tall each of these cells wants to be is
    /// [`crate::render`]'s answer, because a command line wraps and
    /// only the table layout knows how far.
    pub(crate) fn tiled_ids(&self, hidden_when_idle: &[String]) -> Vec<u32> {
        self.groups
            .iter()
            .filter(|group| group.deserves_a_cell(hidden_when_idle))
            .map(|group| group.id)
            .collect()
    }

    /// Fold one scan in, stamping whatever it no longer carries.
    ///
    /// A group the scan still carries keeps its place: the display is
    /// ordered by when a command started, and re-sorting on every scan
    /// would shuffle tiles under the reader for no reason.
    pub(crate) fn observe(&mut self, scan: Vec<CargoGroup>, now: Instant) -> bool {
        if scan == self.last {
            return false;
        }
        self.last.clone_from(&scan);
        let arriving: Vec<u32> = scan.iter().map(CargoGroup::id).collect();
        for group in scan {
            match self
                .groups
                .iter_mut()
                .find(|tracked| tracked.id == group.id())
            {
                Some(tracked) => tracked.refresh(group, now),
                None => self.groups.push(TrackedGroup::from(group)),
            }
        }
        for tracked in &mut self.groups {
            if !arriving.contains(&tracked.id) {
                tracked.finish(now);
            }
        }
        self.assign_families();
        true
    }

    /// Hand every family on screen a colour no other family on screen
    /// holds, and stamp it on the rows that draw it.
    ///
    /// A family is a cargo with cargo running under it. Its index is
    /// kept for as long as it is on screen: reassigning from scratch
    /// each scan would recolour every cell whenever a build finished,
    /// which is the most routine event here. A family that goes away
    /// gives its index back, so the palette is spent on what is
    /// actually running.
    ///
    /// More families at once than the palette holds is the one case
    /// this cannot serve, and it wraps rather than leaving the extra
    /// ones unmarked -- a repeated colour still pairs correctly far
    /// more often than none does.
    fn assign_families(&mut self) {
        let heads: Vec<u32> = self
            .groups
            .iter()
            .flat_map(TrackedGroup::rows)
            .filter(|row| row.process.managed > 0)
            .map(|row| row.process.pid)
            .collect();
        let mut families = std::mem::take(&mut self.families);
        families.retain(|pid, _| heads.contains(pid));
        // Asked of the theme rather than of the palette, because how
        // many colours are left depends on what the column headers are
        // drawn in and that is the reader's to change.
        let count = theme::family_color_count().max(1);
        for pid in heads {
            if families.contains_key(&pid) {
                continue;
            }
            let taken: HashSet<usize> = families.values().copied().collect();
            let free = (0..count).find(|index| !taken.contains(index));
            families.insert(pid, free.unwrap_or(families.len() % count));
        }
        for group in &mut self.groups {
            for row in group.rows_mut() {
                row.family = families.get(&row.process.pid).copied();
                row.parent_family = row
                    .process
                    .parent
                    .and_then(|parent| families.get(&parent).copied());
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
        self.groups.retain(|group| !group.is_expired(now, fade));
        let mut changed = self.groups.len() != before;
        // A group that has finished fading gives its colour back here
        // rather than at the next scan, which with nothing running
        // would never come.
        if changed {
            self.assign_families();
        }
        for group in &mut self.groups {
            changed |= group.advance(now, fade);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::DEFAULT_HIDDEN_WHEN_IDLE;
    use crate::constants::SIBLING_SUBCOMMAND_NAME;
    use crate::processes::CommandText;

    /// The colour is a tie between a row and the rows under it, so a
    /// row's `parent` stamp has to be the very index its parent's own
    /// row carries. Anything else pairs the eye off against nothing.
    #[test]
    fn a_child_carries_the_index_its_parent_was_given() {
        let mut roster = Roster::new();

        roster.observe(vec![family(64432, &[4003, 4058])], Instant::now());

        let stamped = stamps(&roster);
        let lead = stamped
            .iter()
            .find(|(pid, ..)| *pid == 64432)
            .and_then(|(_, family, _)| *family);
        assert!(lead.is_some(), "{stamped:?}");
        for (pid, _, parent) in &stamped {
            if *pid == 64432 {
                continue;
            }
            assert_eq!(*parent, lead, "{stamped:?}");
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

        let heads: Vec<Option<usize>> = stamps(&roster)
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
        let before = stamps(&roster)
            .into_iter()
            .find(|(pid, ..)| *pid == 70001)
            .and_then(|(_, family, _)| family);

        roster.observe(vec![family(70001, &[4058])], Instant::now());

        let after = stamps(&roster)
            .into_iter()
            .find(|(pid, ..)| *pid == 70001)
            .and_then(|(_, family, _)| family);
        assert_eq!(before, after);
    }

    /// A command with nothing running under it has nothing pointing at
    /// it, so a colour on its pid would be a mark meaning nothing.
    #[test]
    fn a_command_with_no_children_is_given_no_colour() {
        let mut roster = Roster::new();

        roster.observe(vec![group(64432, &[])], Instant::now());

        assert_eq!(stamps(&roster), [(64432, None, None)]);
    }

    /// A process row carrying nothing but the pid the tests key on.
    fn process(pid: u32) -> CargoProcess {
        CargoProcess {
            path: "~/rust/project".to_string(),
            pid,
            parent: None,
            start: "10:00".to_string(),
            started: 0,
            duration: "00:01".to_string(),
            cpu: "0%".to_string(),
            compiler: None,
            state: None,
            managed: 0,
            nested: false,
            command: CommandText::of("cargo", &["build"]),
        }
    }

    /// A group led by `lead` with one invocation per entry of `rest`.
    fn group(lead: u32, rest: &[u32]) -> CargoGroup {
        CargoGroup {
            lead:     process(lead),
            rest:     rest.iter().copied().map(process).collect(),
            ancestry: Vec::new(),
        }
    }

    /// The same, with the lead owning every invocation under it -- what
    /// the census reports for a command actually driving cargo, and
    /// what makes the lead a family.
    fn family(lead: u32, rest: &[u32]) -> CargoGroup {
        let mut group = group(lead, rest);
        group.lead.managed = rest.len();
        for row in &mut group.rest {
            row.parent = Some(lead);
        }
        group
    }

    /// The palette index every row of `roster` came out with, as
    /// `(pid, own family, parent's family)`.
    fn stamps(roster: &Roster) -> Vec<(u32, Option<usize>, Option<usize>)> {
        roster
            .groups()
            .iter()
            .flat_map(TrackedGroup::rows)
            .map(|row| (row.process.pid, row.family(), row.parent_family()))
            .collect()
    }

    /// The same, led by a command `commands.hidden_when_idle` names.
    fn hidden_group(lead: u32, rest: &[u32]) -> CargoGroup {
        let mut group = group(lead, rest);
        group.lead.command = CommandText::of("cargo", &[SIBLING_SUBCOMMAND_NAME]);
        group
    }

    /// The list as it reaches the roster once the config has turned it
    /// into owned strings.
    fn hidden_when_idle() -> Vec<String> {
        DEFAULT_HIDDEN_WHEN_IDLE
            .iter()
            .map(|subcommand| (*subcommand).to_string())
            .collect()
    }

    /// The instant every test starts from.
    fn start() -> Instant { Instant::now() }

    #[test]
    fn a_group_the_scan_stops_carrying_is_stamped_rather_than_dropped() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[])], now);
        roster.observe(Vec::new(), now);

        assert_eq!(roster.groups().len(), 1);
        assert!(roster.groups()[0].lead.is_ended());
    }

    #[test]
    fn a_stamped_group_goes_once_the_fade_has_run() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[])], now);
        roster.observe(Vec::new(), now);

        assert!(!roster.advance(now, Duration::from_secs(3)));
        assert!(roster.advance(now + Duration::from_secs(3), Duration::from_secs(3)));
        assert!(roster.groups().is_empty());
    }

    /// A command that comes back with the same pid is the same command,
    /// so the stamp has to lift rather than leaving it greyed forever.
    #[test]
    fn a_group_that_returns_stops_being_ended() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[])], now);
        roster.observe(Vec::new(), now);
        roster.observe(vec![group(10, &[])], now);

        assert!(!roster.groups()[0].lead.is_ended());
    }

    #[test]
    fn a_sub_command_that_finishes_fades_without_taking_the_group() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[11, 12])], now);
        roster.observe(vec![group(10, &[12])], now);

        let tracked = &roster.groups()[0];
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

        let remaining: Vec<u32> = roster.groups()[0]
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

        let order: Vec<u32> = roster.groups()[0]
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

        assert_eq!(roster.groups()[0].rest.len(), 2);
        assert!(roster.groups()[0].rows().all(|row| !row.is_ended()));
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

        assert_eq!(roster.groups()[0].lead.faded(), 0, "stamped, not yet gone");
        roster.advance(now + Duration::from_secs(1), fade);
        assert_eq!(roster.groups()[0].lead.faded(), u8::MAX / 4);
        roster.advance(now + Duration::from_secs(2), fade);
        assert_eq!(roster.groups()[0].lead.faded(), u8::MAX / 2);
    }

    /// A row still in the scan is drawn in its own colours, however
    /// long the display has held it.
    #[test]
    fn a_running_row_never_fades() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![group(10, &[])], now);
        roster.advance(now + Duration::from_secs(2), Duration::from_secs(3));

        assert_eq!(roster.groups()[0].lead.faded(), 0);
    }

    /// A command that comes back with the same pid picks its colours
    /// back up rather than carrying on from where the fade left it.
    #[test]
    fn a_group_that_returns_loses_the_fade_it_had_made() {
        let mut roster = Roster::new();
        let now = start();
        let fade = Duration::from_secs(3);
        roster.observe(vec![group(10, &[])], now);
        roster.observe(Vec::new(), now);
        roster.advance(now + Duration::from_secs(2), fade);
        assert_ne!(roster.groups()[0].lead.faded(), 0, "the fade had started");

        roster.observe(vec![group(10, &[])], now + Duration::from_secs(2));

        assert_eq!(roster.groups()[0].lead.faded(), 0);
    }

    #[test]
    fn a_command_hidden_while_idle_gets_no_cell_with_nothing_under_it() {
        let mut roster = Roster::new();
        roster.observe(vec![hidden_group(10, &[])], start());

        assert!(roster.tiled_ids(&hidden_when_idle()).is_empty());
        // The summary is not what the list holds back: the command is
        // running, and one line saying so is the whole of what it has.
        assert_eq!(roster.groups().len(), 1);
    }

    #[test]
    fn a_command_hidden_while_idle_gets_a_cell_once_it_drives_something() {
        let mut roster = Roster::new();
        roster.observe(vec![hidden_group(10, &[11])], start());

        assert_eq!(roster.tiled_ids(&hidden_when_idle()), vec![10]);
    }

    #[test]
    fn its_cell_stays_while_the_invocation_under_it_fades() {
        let mut roster = Roster::new();
        let now = start();
        roster.observe(vec![hidden_group(10, &[11])], now);
        roster.observe(vec![hidden_group(10, &[])], now);

        // The invocation is stamped rather than gone, so the cell goes
        // out through the fade instead of vanishing under the reader.
        assert_eq!(roster.tiled_ids(&hidden_when_idle()), vec![10]);
        roster.advance(now + Duration::from_secs(1), Duration::ZERO);
        assert!(roster.tiled_ids(&hidden_when_idle()).is_empty());
    }

    #[test]
    fn a_command_off_the_list_gets_a_cell_with_nothing_under_it() {
        let mut roster = Roster::new();
        roster.observe(vec![group(10, &[])], start());

        assert_eq!(roster.tiled_ids(&hidden_when_idle()), vec![10]);
    }
}
