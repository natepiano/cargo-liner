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

use std::time::Duration;
use std::time::Instant;

use crate::processes::CargoGroup;
use crate::processes::CargoProcess;

/// One table row and, once it has stopped, when that happened.
pub(crate) struct TrackedRow {
    /// The invocation as the last scan that carried it described it.
    pub(crate) process: CargoProcess,
    /// When the process left the scan, or `None` while it runs.
    ended:              Option<Instant>,
}

impl From<CargoProcess> for TrackedRow {
    /// A row for a process the current scan carries.
    fn from(process: CargoProcess) -> Self {
        Self {
            process,
            ended: None,
        }
    }
}

impl TrackedRow {
    /// Whether the process behind this row has stopped.
    pub(crate) const fn is_ended(&self) -> bool { self.ended.is_some() }

    /// Take the scan's account of a process that is still running.
    fn refresh(&mut self, process: CargoProcess) {
        self.process = process;
        self.ended = None;
    }

    /// Stamp the row as finished, leaving an earlier stamp alone so the
    /// fade runs from when it actually stopped.
    fn finish(&mut self, now: Instant) { self.ended = self.ended.or(Some(now)); }

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
}

impl From<CargoGroup> for TrackedGroup {
    /// Track a group the current scan has just turned up.
    fn from(group: CargoGroup) -> Self {
        Self {
            id:   group.id(),
            lead: TrackedRow::from(group.lead),
            rest: group.rest.into_iter().map(TrackedRow::from).collect(),
        }
    }
}

impl TrackedGroup {
    /// Every row the group's tile draws, the lead first.
    pub(crate) fn rows(&self) -> impl Iterator<Item = &TrackedRow> {
        std::iter::once(&self.lead).chain(self.rest.iter())
    }

    /// Fold in the scan's account of a group that is still running.
    ///
    /// Rows the scan no longer carries are stamped rather than dropped,
    /// and stay where they sat: a sub-command finishing should fade out
    /// of the place it occupied, not pull the rows under it upward.
    fn refresh(&mut self, group: CargoGroup, now: Instant) {
        self.lead.refresh(group.lead);
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

    /// Drop the rows that have finished fading, reporting whether any
    /// went.
    fn expire(&mut self, now: Instant, fade: Duration) -> bool {
        let before = self.rest.len();
        self.rest.retain(|row| !row.is_expired(now, fade));
        self.rest.len() != before
    }

    /// Whether the group itself has finished fading and its tile should
    /// close.
    fn is_expired(&self, now: Instant, fade: Duration) -> bool { self.lead.is_expired(now, fade) }
}

/// Every group the display knows about, running or fading.
#[derive(Default)]
pub(crate) struct Roster {
    /// Groups in scan order -- newest command first -- with finished
    /// ones held in place until they expire.
    groups: Vec<TrackedGroup>,
    /// The scan last folded in, kept so an identical one can be
    /// recognised and skipped. With nothing building, the display has no
    /// reason to repaint four times a second.
    last:   Vec<CargoGroup>,
}

impl Roster {
    /// An empty roster.
    pub(crate) const fn new() -> Self {
        Self {
            groups: Vec::new(),
            last:   Vec::new(),
        }
    }

    /// The groups to display, newest first.
    pub(crate) fn groups(&self) -> &[TrackedGroup] { &self.groups }

    /// The identity of every group the display is holding, in order --
    /// what [`crate::tiles::TileGrid::sync`] assigns cells from.
    pub(crate) fn ids(&self) -> Vec<u32> { self.groups.iter().map(|group| group.id).collect() }

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
        true
    }

    /// Drop everything that has finished fading, reporting whether the
    /// display needs repainting as a result.
    pub(crate) fn expire(&mut self, now: Instant, fade: Duration) -> bool {
        let before = self.groups.len();
        self.groups.retain(|group| !group.is_expired(now, fade));
        let mut changed = self.groups.len() != before;
        for group in &mut self.groups {
            changed |= group.expire(now, fade);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processes::CommandText;

    /// A process row carrying nothing but the pid the tests key on.
    fn process(pid: u32) -> CargoProcess {
        CargoProcess {
            path: "~/rust/project".to_string(),
            pid,
            start: "10:00".to_string(),
            duration: "00:01".to_string(),
            compiler: None,
            progress: None,
            managed: 0,
            command: CommandText::of("cargo", &["build"]),
        }
    }

    /// A group led by `lead` with one invocation per entry of `rest`.
    fn group(lead: u32, rest: &[u32]) -> CargoGroup {
        CargoGroup {
            lead: process(lead),
            rest: rest.iter().copied().map(process).collect(),
        }
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

        assert!(!roster.expire(now, Duration::from_secs(3)));
        assert!(roster.expire(now + Duration::from_secs(3), Duration::from_secs(3)));
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
        roster.expire(now + Duration::from_secs(3), Duration::from_secs(3));

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

        assert!(roster.expire(now + Duration::from_secs(3), Duration::from_secs(3)));
    }
}
