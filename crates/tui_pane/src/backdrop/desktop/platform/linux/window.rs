//! Discovering terminal windows and reading their `KWin` geometry.

use std::collections::HashMap;
use std::env;
use std::env::VarError;
use std::process::Command;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::Instant;

use zbus::blocking::Proxy;
use zbus::zvariant::OwnedValue;

use super::DesktopReadFailure;
use super::SessionBus;
use super::constants::DESKTOP_RETRY_INTERVAL;
use super::constants::KDO_TOOL_ALL_MATCHES_ARGUMENT;
use super::constants::KDO_TOOL_ALL_WINDOWS_ARGUMENT;
use super::constants::KDO_TOOL_ALL_WINDOWS_PATTERN;
use super::constants::KDO_TOOL_COMMAND;
use super::constants::KDO_TOOL_ID_ARGUMENT;
use super::constants::KDO_TOOL_SEARCH_ARGUMENT;
use super::constants::KWIN_INTERFACE;
use super::constants::KWIN_PATH;
use super::constants::KWIN_SERVICE;
use super::display;
use super::display::Output;
use super::display::OutputSelection;
use super::read_desktop_command;
use super::session_connection;
use crate::backdrop::constants::POSITION_TOLERANCE;
use crate::backdrop::constants::TERM_PROGRAM_ENV;
use crate::backdrop::desktop::CaptureWindowTarget;
use crate::backdrop::desktop::Frame;
use crate::backdrop::desktop::Metrics;
use crate::backdrop::desktop::TerminalWindowSearchOutcome;
use crate::backdrop::desktop::TitledWindow;
use crate::backdrop::desktop::WindowTitle;
use crate::backdrop::desktop::candidate;
use crate::backdrop::desktop::candidate::TerminalWindowCandidate;
use crate::backdrop::desktop::candidate::TerminalWindowCandidates;
use crate::backdrop::desktop::candidate::TerminalWindowOwner;

/// Discovery and handle assignments retained for the process lifetime.
static TERMINAL_WINDOWS: LazyLock<Mutex<TerminalWindowInventory>> =
    LazyLock::new(|| Mutex::new(TerminalWindowInventory::default()));
/// Prevent overlapping temporary `KWin` scripts from colliding inside `kdotool`.
static KDO_TOOL_ACCESS: Mutex<()> = Mutex::new(());

/// One terminal window returned by `KWin`.
pub(super) struct ListedWindow {
    /// Process-local numeric handle used by the cross-platform monitor.
    pub(super) handle: u32,
    /// Current frame in `KWin`'s logical coordinates.
    pub(super) frame:  Frame,
    /// Current window title.
    title:             WindowTitle,
    /// Whether the window belongs to the terminal named by the environment.
    terminal_class:    TerminalClass,
}

impl TerminalWindowCandidate for ListedWindow {
    fn owner(&self) -> TerminalWindowOwner { TerminalWindowOwner::Application { pid: 0 } }

    fn frontmost(&self) -> bool { false }
}

/// `KWin` facts used internally before a UUID receives a numeric handle.
struct WindowInfo {
    /// Current frame in logical coordinates.
    frame:          Frame,
    /// Current window title.
    title:          WindowTitle,
    /// `KWin`'s stable identifier for this window.
    uuid:           String,
    /// Terminal-class membership is independent of a temporary marker title.
    terminal_class: TerminalClass,
}

/// Bidirectional conversion between `KWin` UUIDs and the public numeric window-id shape.
#[derive(Default)]
struct WindowRegistry {
    /// Numeric handles indexed by `KWin` UUID.
    by_uuid:   HashMap<String, u32>,
    /// `KWin` UUIDs indexed by numeric handle.
    by_handle: HashMap<u32, String>,
}

impl WindowRegistry {
    /// Return the existing handle for `uuid` or allocate the next one.
    fn register(&mut self, uuid: String) -> Option<u32> {
        if let Some(handle) = self.by_uuid.get(&uuid) {
            return Some(*handle);
        }
        let handle = u32::try_from(self.by_uuid.len().checked_add(1)?).ok()?;
        self.by_uuid.insert(uuid.clone(), handle);
        self.by_handle.insert(handle, uuid);
        Some(handle)
    }

    /// Resolve a process-local handle back to `KWin`'s UUID.
    fn uuid(&self, handle: u32) -> Option<String> { self.by_handle.get(&handle).cloned() }
}

/// Why terminal-window discovery could not complete.
#[derive(Clone, Debug, Eq, PartialEq)]
enum WindowSearchFailure {
    /// Another worker panicked while owning the temporary-script lock.
    AccessPoisoned,
    /// The bounded command failed or expired.
    Command(DesktopReadFailure),
}

/// Whether the window class names this process's terminal emulator.
#[derive(Clone, Copy)]
enum TerminalClass {
    /// The class contains the terminal program name, ignoring ASCII case.
    Matching,
    /// A marker may identify this window even when its class is unknown or different.
    Other,
}

/// Terminal membership retained for a UUID until the window stops answering.
#[derive(Clone, Copy)]
enum TerminalClassification {
    /// Discovery found this UUID, but no window query has classified it yet.
    Unclassified,
    /// The window's resource class has been observed during its lifetime.
    Known(TerminalClass),
}

/// The last terminal-window discovery attempt and its retry clock.
#[derive(Default)]
enum TerminalWindowSearch {
    /// No terminal class search has run in this process.
    #[default]
    NotSearched,
    /// Successful discovery, with vanished UUIDs removed as soon as they are observed.
    Found {
        /// Held UUIDs, including title candidates when the terminal class cannot identify its
        /// window.
        uuids:       Vec<String>,
        /// Completion of the last successful search, including an empty result.
        searched_at: Instant,
    },
    /// A failed attempt, retained until another search is permitted.
    Unavailable {
        /// The reason discovery failed.
        failure:  WindowSearchFailure,
        /// Completion of the failed attempt.
        tried_at: Instant,
    },
}

/// A current answer from the held session bus for one UUID.
enum WindowQuery {
    /// The window still supplies geometry and a title.
    Answered(WindowInfo),
    /// The UUID or its session bus no longer answers.
    Unavailable,
}

/// Why a caller reads the terminal inventory.
#[derive(Clone, Copy)]
enum WindowInventoryUse {
    /// An answering pinned window needs no terminal discovery.
    Capture(CaptureWindowTarget),
    /// Identification reads current facts for held UUIDs.
    Identification,
    /// Marker lookup and title restoration include windows outside the terminal class.
    MarkerIdentification,
}

/// Search and geometry operations, replaceable together without processes or D-Bus in tests.
trait TerminalWindowSource {
    /// Discover UUIDs for terminal selection and marker identification with one bounded search.
    fn search(&mut self) -> Result<Vec<String>, WindowSearchFailure>;

    /// Read current facts for a UUID without discovering any windows.
    fn query(&mut self, uuid: &str) -> WindowQuery;
}

/// Process-lifetime terminal UUID discovery and stable numeric handle assignments.
#[derive(Default)]
struct TerminalWindowInventory {
    /// The latest search and its earliest permitted retry.
    search:          TerminalWindowSearch,
    /// Handles survive inventory refreshes so pinned windows remain resolvable.
    registry:        WindowRegistry,
    /// Classifications survive discovery retries; marker queries refresh their answers.
    classifications: HashMap<String, TerminalClassification>,
}

impl TerminalWindowInventory {
    /// Read current windows, searching only at startup or when a due refresh is needed.
    fn read(
        &mut self,
        usage: WindowInventoryUse,
        now: impl Fn() -> Instant,
        source: &mut impl TerminalWindowSource,
    ) -> Vec<ListedWindow> {
        if let WindowInventoryUse::Capture(CaptureWindowTarget::PreferWindow { window_id }) = usage
            && let WindowQuery::Answered(window) = self.registered_window(window_id, source)
        {
            return vec![ListedWindow {
                handle:         window_id,
                frame:          window.frame,
                title:          window.title,
                terminal_class: window.terminal_class,
            }];
        }
        let windows = self.held_windows(source, usage, Vec::new());
        let observed_at = now();
        match &self.search {
            TerminalWindowSearch::NotSearched => {},
            TerminalWindowSearch::Found { searched_at, .. } => {
                if observed_at.saturating_duration_since(*searched_at) < DESKTOP_RETRY_INTERVAL
                    || (!windows.is_empty()
                        && !matches!(
                            usage,
                            WindowInventoryUse::Capture(
                                CaptureWindowTarget::TerminalWindowHeuristic
                            )
                        ))
                {
                    return windows;
                }
            },
            TerminalWindowSearch::Unavailable { failure, tried_at } => {
                if observed_at.saturating_duration_since(*tried_at) < DESKTOP_RETRY_INTERVAL {
                    tracing::debug!(?failure, "terminal window search awaits retry");
                    return windows;
                }
            },
        }
        self.search = match source.search() {
            Ok(uuids) => TerminalWindowSearch::Found {
                uuids,
                searched_at: now(),
            },
            Err(failure) => TerminalWindowSearch::Unavailable {
                failure,
                tried_at: now(),
            },
        };
        self.held_windows(source, usage, windows)
    }

    /// Query eligible UUIDs and prune failures, reusing this read's answers across discovery.
    fn held_windows(
        &mut self,
        source: &mut impl TerminalWindowSource,
        usage: WindowInventoryUse,
        mut windows: Vec<ListedWindow>,
    ) -> Vec<ListedWindow> {
        let TerminalWindowSearch::Found { uuids, .. } = &mut self.search else {
            return Vec::new();
        };
        windows.retain(|window| {
            self.registry
                .by_handle
                .get(&window.handle)
                .is_some_and(|uuid| uuids.contains(uuid))
        });
        uuids.retain(|uuid| {
            let classification = self
                .classifications
                .entry(uuid.clone())
                .or_insert(TerminalClassification::Unclassified);
            if matches!(
                classification,
                TerminalClassification::Known(TerminalClass::Other)
            ) && !matches!(usage, WindowInventoryUse::MarkerIdentification)
            {
                return true;
            }
            if self
                .registry
                .by_uuid
                .get(uuid)
                .is_some_and(|handle| windows.iter().any(|window| window.handle == *handle))
            {
                return true;
            }
            let WindowQuery::Answered(window) = source.query(uuid) else {
                self.classifications.remove(uuid);
                return false;
            };
            *classification = TerminalClassification::Known(window.terminal_class);
            if let Some(handle) = self.registry.register(window.uuid) {
                windows.push(ListedWindow {
                    handle,
                    frame: window.frame,
                    title: window.title,
                    terminal_class: window.terminal_class,
                });
            }
            true
        });
        windows.retain(|window| {
            matches!(usage, WindowInventoryUse::MarkerIdentification)
                || matches!(window.terminal_class, TerminalClass::Matching)
        });
        windows
    }

    /// Resolve a known handle and immediately discard its UUID when it stops answering.
    fn registered_window(
        &mut self,
        handle: u32,
        source: &mut impl TerminalWindowSource,
    ) -> WindowQuery {
        let Some(uuid) = self.registry.uuid(handle) else {
            return WindowQuery::Unavailable;
        };
        let window = source.query(&uuid);
        match &window {
            WindowQuery::Answered(window) => {
                self.classifications
                    .insert(uuid, TerminalClassification::Known(window.terminal_class));
            },
            WindowQuery::Unavailable => {
                self.classifications.remove(&uuid);
                if let TerminalWindowSearch::Found { uuids, .. } = &mut self.search {
                    uuids.retain(|held| held != &uuid);
                }
            },
        }
        window
    }
}

/// The live terminal search and held session-bus window reader.
struct KWin;

impl TerminalWindowSource for KWin {
    fn search(&mut self) -> Result<Vec<String>, WindowSearchFailure> { search_uuids() }

    fn query(&mut self, uuid: &str) -> WindowQuery {
        query_window(uuid).map_or(WindowQuery::Unavailable, WindowQuery::Answered)
    }
}

/// The same inventory serves captures and all identification passes.
fn inventory_windows(usage: WindowInventoryUse) -> Vec<ListedWindow> {
    TERMINAL_WINDOWS.lock().map_or_else(
        |_| Vec::new(),
        |mut inventory| inventory.read(usage, Instant::now, &mut KWin),
    )
}

/// Windows needed by one capture, with discovery independent of the capture cadence.
pub(super) fn for_capture(target: CaptureWindowTarget) -> Vec<ListedWindow> {
    inventory_windows(WindowInventoryUse::Capture(target))
}

/// Classify the already terminal-filtered windows for shared selection diagnostics.
pub(super) fn candidates(windows: &[ListedWindow]) -> TerminalWindowCandidates<'_, ListedWindow> {
    candidate::terminal_window_candidates(windows, |_| false, |_| true)
}

/// The terminal window whose frame most nearly contains the reported text area.
pub(super) fn closest_size_match<'a>(
    windows: &[&'a ListedWindow],
    outputs: &[Output],
    metrics: Metrics,
) -> Option<&'a ListedWindow> {
    let score = |window: &ListedWindow| {
        let scale = match display::under(outputs, window.frame) {
            OutputSelection::Containing(output) | OutputSelection::Nearest(output) => output.scale,
            OutputSelection::NoActiveOutputs => 1.0,
        };
        let text = (
            f64::from(metrics.text_area.0) / scale,
            f64::from(metrics.text_area.1) / scale,
        );
        mismatch(window.frame.size.0, text.0) + mismatch(window.frame.size.1, text.1)
    };
    windows
        .iter()
        .min_by(|left, right| score(left).total_cmp(&score(right)))
        .copied()
}

/// How far one frame axis is from containing the corresponding text axis.
fn mismatch(frame: f64, text: f64) -> f64 {
    let difference = frame - text;
    if difference < 0.0 {
        -difference * 2.0
    } else {
        difference
    }
}

/// Current frame for a previously registered window handle.
pub(super) fn frame(handle: u32) -> Option<Frame> {
    let query = TERMINAL_WINDOWS
        .lock()
        .ok()?
        .registered_window(handle, &mut KWin);
    match query {
        WindowQuery::Answered(window) => Some(window.frame),
        WindowQuery::Unavailable => None,
    }
}

/// Titles of every current terminal window.
pub(super) fn titles() -> Vec<TitledWindow> {
    inventory_windows(WindowInventoryUse::MarkerIdentification)
        .into_iter()
        .map(|window| TitledWindow {
            window_id: window.handle,
            title:     window.title,
        })
        .collect()
}

/// The terminal window currently wearing `marker`.
pub(super) fn titled(marker: &str) -> TerminalWindowSearchOutcome {
    titled_in(
        inventory_windows(WindowInventoryUse::MarkerIdentification),
        marker,
    )
}

/// Match the marker against current titles from the held inventory.
fn titled_in(windows: Vec<ListedWindow>, marker: &str) -> TerminalWindowSearchOutcome {
    windows
        .into_iter()
        .find(|window| match &window.title {
            WindowTitle::Reported(title) => title.contains(marker),
            WindowTitle::Withheld => false,
        })
        .map_or(TerminalWindowSearchOutcome::NotFound, |window| {
            TerminalWindowSearchOutcome::Found {
                window_id: window.handle,
            }
        })
}

/// The terminal window nearest the reported origin.
pub(super) fn at(origin: (f64, f64)) -> TerminalWindowSearchOutcome {
    inventory_windows(WindowInventoryUse::Identification)
        .into_iter()
        .map(|window| {
            let distance =
                (window.frame.origin.0 - origin.0).abs() + (window.frame.origin.1 - origin.1).abs();
            (window.handle, distance)
        })
        .filter(|(_, distance)| *distance <= POSITION_TOLERANCE)
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map_or(TerminalWindowSearchOutcome::NotFound, |(window_id, _)| {
            TerminalWindowSearchOutcome::Found { window_id }
        })
}

/// Discover UUIDs once for both terminal-class selection and class-independent title markers.
/// Titles and classes are read over the held bus; a missing terminal name cannot hide its marker.
fn search_uuids() -> Result<Vec<String>, WindowSearchFailure> {
    let _access = KDO_TOOL_ACCESS
        .lock()
        .map_err(|_| WindowSearchFailure::AccessPoisoned)?;
    let bytes = read_desktop_command(Command::new(KDO_TOOL_COMMAND).args(search_arguments()))
        .map_err(WindowSearchFailure::Command)?;
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// One class-independent search emits every matching UUID for the shared inventory.
const fn search_arguments() -> [&'static str; 5] {
    [
        KDO_TOOL_SEARCH_ARGUMENT,
        KDO_TOOL_ALL_WINDOWS_ARGUMENT,
        KDO_TOOL_ALL_WINDOWS_PATTERN,
        KDO_TOOL_ID_ARGUMENT,
        KDO_TOOL_ALL_MATCHES_ARGUMENT,
    ]
}

/// Read `KWin`'s current facts for one UUID.
fn query_window(uuid: &str) -> Option<WindowInfo> {
    let connection = match session_connection() {
        SessionBus::Connected(connection) => connection,
        SessionBus::Unavailable(failure) => {
            tracing::debug!(%failure, "desktop query has no session bus");
            return None;
        },
    };
    let proxy = Proxy::new(&connection, KWIN_SERVICE, KWIN_PATH, KWIN_INTERFACE).ok()?;
    let properties: HashMap<String, OwnedValue> = proxy.call("getWindowInfo", &uuid).ok()?;
    if properties.is_empty() {
        return None;
    }
    Some(WindowInfo {
        frame:          Frame {
            origin: (
                property_f64(&properties, "x")?,
                property_f64(&properties, "y")?,
            ),
            size:   (
                property_f64(&properties, "width")?,
                property_f64(&properties, "height")?,
            ),
        },
        title:          property_text(&properties, "caption")
            .map_or(WindowTitle::Withheld, WindowTitle::Reported),
        uuid:           property_text(&properties, "uuid").unwrap_or_else(|| uuid.to_owned()),
        terminal_class: terminal_class(
            env::var(TERM_PROGRAM_ENV).as_deref(),
            property_text(&properties, "resourceClass").as_deref(),
        ),
    })
}

/// Interpret the terminal program name at the environment and D-Bus property boundaries.
fn terminal_class(program: Result<&str, &VarError>, class: Option<&str>) -> TerminalClass {
    match (program, class) {
        (Ok(program), Some(class))
            if !program.is_empty()
                && class
                    .to_ascii_lowercase()
                    .contains(&program.to_ascii_lowercase()) =>
        {
            TerminalClass::Matching
        },
        _ => TerminalClass::Other,
    }
}

/// Read a string property from `KWin`'s variant map.
fn property_text(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    <&str>::try_from(properties.get(key)?)
        .ok()
        .map(str::to_owned)
}

/// Read a floating-point geometry property from `KWin`'s variant map.
fn property_f64(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<f64> {
    let value = properties.get(key)?;
    f64::try_from(value)
        .ok()
        .or_else(|| i32::try_from(value).ok().map(f64::from))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::env::VarError;
    use std::time::Duration;

    use super::*;
    use crate::backdrop::desktop::platform::linux::constants::TOPOLOGY_READ_DEADLINE;

    /// A terminal server with countable searches and independently changing live windows.
    struct ScriptedTerminalWindows {
        searches:          usize,
        discovery:         Result<Vec<String>, WindowSearchFailure>,
        answering:         Vec<String>,
        queries:           Vec<String>,
        caption:           String,
        terminal_class:    TerminalClass,
        marker_candidates: Vec<String>,
    }

    impl ScriptedTerminalWindows {
        fn available() -> Self {
            Self {
                searches:          0,
                discovery:         Ok(vec!["{one}".to_owned(), "{two}".to_owned()]),
                answering:         vec!["{one}".to_owned(), "{two}".to_owned()],
                queries:           Vec::new(),
                caption:           "terminal".to_owned(),
                terminal_class:    TerminalClass::Matching,
                marker_candidates: Vec::new(),
            }
        }

        fn mixed_classes() -> Self {
            let marker_candidates: Vec<_> = ["{desktop}", "{browser}", "{editor}"]
                .into_iter()
                .map(str::to_owned)
                .collect();
            let mut answering = vec!["{one}".to_owned()];
            answering.extend(marker_candidates.clone());
            Self {
                discovery: Ok(answering.clone()),
                answering,
                marker_candidates,
                ..Self::available()
            }
        }
    }

    impl TerminalWindowSource for ScriptedTerminalWindows {
        fn search(&mut self) -> Result<Vec<String>, WindowSearchFailure> {
            self.searches += 1;
            self.discovery.clone()
        }

        fn query(&mut self, uuid: &str) -> WindowQuery {
            self.queries.push(uuid.to_owned());
            if self.answering.iter().any(|answer| answer == uuid) {
                WindowQuery::Answered(WindowInfo {
                    frame:          Frame {
                        origin: (0.0, 0.0),
                        size:   (100.0, 100.0),
                    },
                    title:          WindowTitle::Reported(self.caption.clone()),
                    uuid:           uuid.to_owned(),
                    terminal_class: if self.marker_candidates.iter().any(|marker| marker == uuid) {
                        TerminalClass::Other
                    } else {
                        self.terminal_class
                    },
                })
            } else {
                WindowQuery::Unavailable
            }
        }
    }

    /// A slow window query and command advance the same clock the inventory uses.
    struct DelayedTerminalWindows<'a> {
        clock:            &'a Cell<Instant>,
        next_query_delay: Duration,
        search_delay:     Duration,
        source:           ScriptedTerminalWindows,
    }

    impl TerminalWindowSource for DelayedTerminalWindows<'_> {
        fn search(&mut self) -> Result<Vec<String>, WindowSearchFailure> {
            self.clock.set(self.clock.get() + self.search_delay);
            self.source.search()
        }

        fn query(&mut self, uuid: &str) -> WindowQuery {
            self.clock
                .set(self.clock.get() + std::mem::take(&mut self.next_query_delay));
            self.source.query(uuid)
        }
    }

    #[test]
    fn discovery_arguments_search_all_window_classes_and_return_every_match() {
        assert_eq!(
            search_arguments(),
            ["search", "--name", ".*", "getwindowid", "%@"]
        );
    }

    #[test]
    fn steady_unpinned_capture_and_coordinate_reads_query_only_terminal_uuids() {
        let now = Instant::now();
        for usage in [
            WindowInventoryUse::Capture(CaptureWindowTarget::TerminalWindowHeuristic),
            WindowInventoryUse::Capture(CaptureWindowTarget::PreferWindow { window_id: 99 }),
            WindowInventoryUse::Identification,
        ] {
            let mut inventory = TerminalWindowInventory::default();
            let mut source = ScriptedTerminalWindows::mixed_classes();
            assert_eq!(inventory.read(usage, || now, &mut source).len(), 1);
            assert_eq!(source.queries, source.answering);
            for tick in 1..DESKTOP_RETRY_INTERVAL.as_secs() * 3 {
                source.queries.clear();
                let windows =
                    inventory.read(usage, || now + Duration::from_secs(tick), &mut source);
                assert!(matches!(windows.as_slice(), [window] if window.handle == 1));
                assert_eq!(source.queries, ["{one}"], "tick {tick}");
            }
        }
    }

    #[test]
    fn discovery_refresh_classifies_new_uuids_once_and_keeps_existing_classifications() {
        let now = Instant::now();
        let usage = WindowInventoryUse::Capture(CaptureWindowTarget::TerminalWindowHeuristic);
        let mut inventory = TerminalWindowInventory::default();
        let mut source = ScriptedTerminalWindows::mixed_classes();
        inventory.read(usage, || now, &mut source);
        source
            .answering
            .extend(["{new-other}".to_owned(), "{new-terminal}".to_owned()]);
        source.marker_candidates.push("{new-other}".to_owned());
        source.discovery = Ok(source.answering.clone());
        source.queries.clear();
        assert_eq!(
            inventory
                .read(usage, || now + DESKTOP_RETRY_INTERVAL, &mut source)
                .len(),
            2
        );
        assert_eq!(source.queries, ["{one}", "{new-other}", "{new-terminal}"]);
        for tick in 1..=DESKTOP_RETRY_INTERVAL.as_secs() {
            source.queries.clear();
            assert_eq!(
                inventory
                    .read(
                        usage,
                        || now + DESKTOP_RETRY_INTERVAL + Duration::from_secs(tick),
                        &mut source,
                    )
                    .len(),
                2
            );
            assert_eq!(source.queries, ["{one}", "{new-terminal}"]);
        }
        assert_eq!(source.searches, 3);
    }

    #[test]
    fn marker_identification_queries_every_held_uuid_and_refreshes_classification() {
        let now = Instant::now();
        let capture = WindowInventoryUse::Capture(CaptureWindowTarget::TerminalWindowHeuristic);
        let mut inventory = TerminalWindowInventory::default();
        let mut source = ScriptedTerminalWindows::mixed_classes();
        inventory.read(capture, || now, &mut source);
        for _ in 0..3 {
            source.queries.clear();
            assert_eq!(
                inventory
                    .read(
                        WindowInventoryUse::MarkerIdentification,
                        || now,
                        &mut source
                    )
                    .len(),
                4
            );
            assert_eq!(source.queries, source.answering);
        }
        // Differing scripted answers verify that marker observations replace held classifications.
        source.marker_candidates.retain(|uuid| uuid != "{browser}");
        source.marker_candidates.push("{one}".to_owned());
        inventory.read(
            WindowInventoryUse::MarkerIdentification,
            || now,
            &mut source,
        );
        source.queries.clear();
        let windows = inventory.read(capture, || now, &mut source);
        assert!(matches!(windows.as_slice(), [window] if window.handle == 3));
        assert_eq!(source.queries, ["{browser}"]);
        assert_eq!(source.searches, 1);
    }

    #[test]
    fn vanished_terminal_is_pruned_without_querying_held_other_classes() {
        let now = Instant::now();
        for usage in [
            WindowInventoryUse::Capture(CaptureWindowTarget::TerminalWindowHeuristic),
            WindowInventoryUse::Capture(CaptureWindowTarget::PreferWindow { window_id: 1 }),
            WindowInventoryUse::Identification,
        ] {
            let mut inventory = TerminalWindowInventory::default();
            let mut source = ScriptedTerminalWindows::mixed_classes();
            inventory.read(usage, || now, &mut source);
            source.answering.remove(0);
            source.discovery = Ok(source.answering.clone());
            source.queries.clear();
            assert!(inventory.read(usage, || now, &mut source).is_empty());
            assert_eq!(source.queries, ["{one}"]);
            assert!(
                matches!(&inventory.search, TerminalWindowSearch::Found { uuids, .. }
                if uuids == &source.marker_candidates)
            );
            source.queries.clear();
            for tick in 1..DESKTOP_RETRY_INTERVAL.as_secs() {
                assert!(
                    inventory
                        .read(
                            WindowInventoryUse::Capture(
                                CaptureWindowTarget::TerminalWindowHeuristic
                            ),
                            || now + Duration::from_secs(tick),
                            &mut source,
                        )
                        .is_empty()
                );
            }
            assert!(source.queries.is_empty());
            assert_eq!(source.searches, 1);
            source.queries.clear();
            assert!(
                inventory
                    .read(
                        WindowInventoryUse::Identification,
                        || now + DESKTOP_RETRY_INTERVAL,
                        &mut source,
                    )
                    .is_empty()
            );
            assert!(source.queries.is_empty());
            assert_eq!(source.searches, 2);
        }
    }

    #[test]
    fn failed_discovery_does_not_forget_other_classifications_before_retry() {
        let now = Instant::now();
        let usage = WindowInventoryUse::Capture(CaptureWindowTarget::TerminalWindowHeuristic);
        let mut inventory = TerminalWindowInventory::default();
        let mut source = ScriptedTerminalWindows::mixed_classes();
        inventory.read(usage, || now, &mut source);
        source.discovery = Err(WindowSearchFailure::Command(DesktopReadFailure::Expired));
        assert!(
            inventory
                .read(usage, || now + DESKTOP_RETRY_INTERVAL, &mut source)
                .is_empty()
        );
        source.discovery = Ok(source.answering.clone());
        source.queries.clear();
        assert_eq!(
            inventory
                .read(usage, || now + DESKTOP_RETRY_INTERVAL * 2, &mut source)
                .len(),
            1
        );
        assert_eq!(source.queries, ["{one}"]);
        assert_eq!(source.searches, 3);
    }

    #[test]
    fn slow_queries_and_searches_cannot_shorten_the_next_search_interval() {
        let now = Instant::now();
        let clock = Cell::new(now);
        let mut source = DelayedTerminalWindows {
            clock:            &clock,
            next_query_delay: Duration::ZERO,
            search_delay:     Duration::ZERO,
            source:           ScriptedTerminalWindows::available(),
        };
        let mut inventory = TerminalWindowInventory::default();
        let usage = WindowInventoryUse::Capture(CaptureWindowTarget::TerminalWindowHeuristic);
        inventory.read(usage, || clock.get(), &mut source);
        clock.set(now + DESKTOP_RETRY_INTERVAL);
        source.next_query_delay = DESKTOP_RETRY_INTERVAL;
        source.search_delay = TOPOLOGY_READ_DEADLINE;
        inventory.read(usage, || clock.get(), &mut source);
        let completed_at = clock.get();
        assert!(
            matches!(&inventory.search, TerminalWindowSearch::Found { searched_at, .. } if *searched_at == completed_at)
        );
        for tick in 0..DESKTOP_RETRY_INTERVAL.as_secs() {
            clock.set(completed_at + Duration::from_secs(tick));
            inventory.read(usage, || clock.get(), &mut source);
        }
        assert_eq!(source.source.searches, 2);
        clock.set(completed_at + DESKTOP_RETRY_INTERVAL);
        source.source.discovery = Err(WindowSearchFailure::Command(DesktopReadFailure::Expired));
        assert!(
            inventory
                .read(usage, || clock.get(), &mut source)
                .is_empty()
        );
        let failed_at = clock.get();
        assert!(
            matches!(&inventory.search, TerminalWindowSearch::Unavailable { tried_at, .. } if *tried_at == failed_at)
        );
        for tick in 0..DESKTOP_RETRY_INTERVAL.as_secs() {
            clock.set(failed_at + Duration::from_secs(tick));
            assert!(
                inventory
                    .read(usage, || clock.get(), &mut source)
                    .is_empty()
            );
        }
        assert_eq!(source.source.searches, 3);
        source.source.discovery = Ok(source.source.answering.clone());
        clock.set(failed_at + DESKTOP_RETRY_INTERVAL);
        assert_eq!(inventory.read(usage, || clock.get(), &mut source).len(), 2);
        assert_eq!(source.source.searches, 4);
    }

    #[test]
    fn held_marker_candidates_identify_and_pin_without_a_matching_terminal_program() {
        let now = Instant::now();
        let missing = VarError::NotPresent;
        for program in [Err(&missing), Ok(""), Ok("unrelated-terminal")] {
            let mut inventory = TerminalWindowInventory::default();
            let mut source = ScriptedTerminalWindows {
                terminal_class: terminal_class(program, Some("kitty")),
                ..ScriptedTerminalWindows::available()
            };
            assert!(
                inventory
                    .read(
                        WindowInventoryUse::Capture(CaptureWindowTarget::TerminalWindowHeuristic),
                        || now,
                        &mut source
                    )
                    .is_empty()
            );
            let previous = inventory.read(
                WindowInventoryUse::MarkerIdentification,
                || now,
                &mut source,
            );
            assert_eq!(previous.len(), 2);
            source.caption = "unique marker".to_owned();
            for _ in 0..60 {
                let windows = inventory.read(
                    WindowInventoryUse::MarkerIdentification,
                    || now,
                    &mut source,
                );
                assert_eq!(
                    titled_in(windows, &source.caption),
                    TerminalWindowSearchOutcome::Found { window_id: 1 }
                );
            }
            assert_eq!(source.searches, 1);
            assert_eq!(
                inventory
                    .read(
                        WindowInventoryUse::Capture(CaptureWindowTarget::PreferWindow {
                            window_id: 1,
                        }),
                        || now + DESKTOP_RETRY_INTERVAL,
                        &mut source
                    )
                    .len(),
                1
            );
            assert_eq!(source.searches, 1);
        }
        assert!(matches!(
            terminal_class(Ok("WezTerm"), Some("org.wezfurlong.wezterm")),
            TerminalClass::Matching
        ));
    }

    #[test]
    fn every_steady_capture_and_identification_read_reuses_the_first_search() {
        let now = Instant::now();
        for usage in [
            WindowInventoryUse::Capture(CaptureWindowTarget::TerminalWindowHeuristic),
            WindowInventoryUse::Capture(CaptureWindowTarget::PreferWindow { window_id: 1 }),
            WindowInventoryUse::Identification,
            WindowInventoryUse::MarkerIdentification,
        ] {
            let mut inventory = TerminalWindowInventory::default();
            let mut source = ScriptedTerminalWindows::available();
            assert_eq!(
                inventory
                    .read(WindowInventoryUse::Identification, || now, &mut source)
                    .len(),
                2
            );
            for tick in 0..DESKTOP_RETRY_INTERVAL.as_secs() {
                let windows =
                    inventory.read(usage, || now + Duration::from_secs(tick), &mut source);
                assert!(!windows.is_empty());
            }
            assert_eq!(source.searches, 1);
        }
    }

    #[test]
    fn answering_pin_and_identification_reads_never_refresh_a_nonempty_inventory() {
        let now = Instant::now();
        let mut inventory = TerminalWindowInventory::default();
        let mut source = ScriptedTerminalWindows::available();
        inventory.read(WindowInventoryUse::Identification, || now, &mut source);
        source.caption = "fresh identification marker".to_owned();
        for interval in 1..=60 {
            for usage in [
                WindowInventoryUse::Capture(CaptureWindowTarget::PreferWindow { window_id: 1 }),
                WindowInventoryUse::Identification,
                WindowInventoryUse::MarkerIdentification,
            ] {
                let windows = inventory.read(
                    usage,
                    || now + DESKTOP_RETRY_INTERVAL * interval,
                    &mut source,
                );
                assert!(!windows.is_empty());
                assert!(windows.iter().all(|window| matches!(&window.title,
                    WindowTitle::Reported(title) if title == &source.caption)));
            }
        }
        assert_eq!(source.searches, 1);
    }

    #[test]
    fn heuristic_refresh_adds_new_terminal_only_when_the_interval_expires() {
        let usage = WindowInventoryUse::Capture(CaptureWindowTarget::TerminalWindowHeuristic);
        let now = Instant::now();
        let mut inventory = TerminalWindowInventory::default();
        let mut source = ScriptedTerminalWindows::available();
        source.discovery = Ok(vec!["{one}".to_owned()]);
        assert_eq!(inventory.read(usage, || now, &mut source).len(), 1);
        source.discovery = Ok(source.answering.clone());
        assert_eq!(
            inventory
                .read(usage, || now + DESKTOP_RETRY_INTERVAL / 2, &mut source)
                .len(),
            1
        );
        assert_eq!(source.searches, 1);
        assert_eq!(
            inventory
                .read(usage, || now + DESKTOP_RETRY_INTERVAL, &mut source)
                .len(),
            2
        );
        assert_eq!(source.searches, 2);
        for _ in 0..60 {
            assert_eq!(
                inventory
                    .read(usage, || now + DESKTOP_RETRY_INTERVAL, &mut source)
                    .len(),
                2
            );
        }
        assert_eq!(source.searches, 2);
    }

    #[test]
    fn failed_empty_and_expired_searches_retry_only_after_the_interval() {
        let now = Instant::now();
        for discovery in [
            Ok(Vec::new()),
            Err(WindowSearchFailure::Command(DesktopReadFailure::Failed(
                "command failed".to_owned(),
            ))),
            Err(WindowSearchFailure::Command(DesktopReadFailure::Expired)),
        ] {
            let mut inventory = TerminalWindowInventory::default();
            let mut source = ScriptedTerminalWindows {
                discovery,
                ..ScriptedTerminalWindows::available()
            };
            assert!(
                inventory
                    .read(WindowInventoryUse::Identification, || now, &mut source)
                    .is_empty()
            );
            match &source.discovery {
                Ok(_) => assert!(
                    matches!(&inventory.search, TerminalWindowSearch::Found { uuids, .. } if uuids.is_empty())
                ),
                Err(expected) => assert!(
                    matches!(&inventory.search, TerminalWindowSearch::Unavailable { failure, .. } if failure == expected)
                ),
            }
            source.discovery = Ok(source.answering.clone());
            for tick in 0..DESKTOP_RETRY_INTERVAL.as_secs() {
                for usage in [
                    WindowInventoryUse::Capture(CaptureWindowTarget::TerminalWindowHeuristic),
                    WindowInventoryUse::Capture(CaptureWindowTarget::PreferWindow { window_id: 1 }),
                    WindowInventoryUse::Identification,
                    WindowInventoryUse::MarkerIdentification,
                ] {
                    assert!(
                        inventory
                            .read(usage, || now + Duration::from_secs(tick), &mut source)
                            .is_empty()
                    );
                }
            }
            assert_eq!(source.searches, 1);
            assert_eq!(
                inventory
                    .read(
                        WindowInventoryUse::Identification,
                        || now + DESKTOP_RETRY_INTERVAL,
                        &mut source
                    )
                    .len(),
                2
            );
            assert_eq!(source.searches, 2);
        }
    }

    #[test]
    fn vanished_pin_falls_back_without_search_and_is_removed_immediately() {
        let now = Instant::now();
        let mut inventory = TerminalWindowInventory::default();
        let mut source = ScriptedTerminalWindows::available();
        inventory.read(WindowInventoryUse::Identification, || now, &mut source);
        source.answering.remove(0);
        let windows = inventory.read(
            WindowInventoryUse::Capture(CaptureWindowTarget::PreferWindow { window_id: 1 }),
            || now + DESKTOP_RETRY_INTERVAL,
            &mut source,
        );
        assert!(matches!(windows.as_slice(), [window] if window.handle == 2));
        assert_eq!(source.searches, 1);
        assert!(
            matches!(&inventory.search, TerminalWindowSearch::Found { uuids, .. }
            if uuids == &["{two}".to_owned()])
        );
        source.queries.clear();
        inventory.read(
            WindowInventoryUse::Identification,
            || now + DESKTOP_RETRY_INTERVAL,
            &mut source,
        );
        assert_eq!(source.queries, vec!["{two}"]);
    }

    #[test]
    fn vanished_terminal_retries_even_while_other_marker_candidates_answer() {
        let now = Instant::now();
        let mut inventory = TerminalWindowInventory::default();
        let mut source = ScriptedTerminalWindows {
            marker_candidates: vec!["{two}".to_owned()],
            ..ScriptedTerminalWindows::available()
        };
        let usage = WindowInventoryUse::Capture(CaptureWindowTarget::PreferWindow { window_id: 1 });
        assert_eq!(inventory.read(usage, || now, &mut source).len(), 1);
        source.answering.remove(0);
        source.answering.push("{three}".to_owned());
        source.discovery = Ok(source.answering.clone());
        for tick in 0..DESKTOP_RETRY_INTERVAL.as_secs() {
            assert!(
                inventory
                    .read(usage, || now + Duration::from_secs(tick), &mut source)
                    .is_empty()
            );
        }
        assert_eq!(source.searches, 1);
        let windows = inventory.read(usage, || now + DESKTOP_RETRY_INTERVAL, &mut source);
        assert!(matches!(windows.as_slice(), [window] if window.handle == 3));
        assert_eq!(source.searches, 2);
    }

    #[test]
    fn losing_all_windows_or_the_bus_never_searches_more_than_once_per_interval() {
        let now = Instant::now();
        let mut inventory = TerminalWindowInventory::default();
        let mut source = ScriptedTerminalWindows::available();
        inventory.read(WindowInventoryUse::Identification, || now, &mut source);
        source.answering.clear();
        for tick in 0..DESKTOP_RETRY_INTERVAL.as_secs() * 3 {
            for usage in [
                WindowInventoryUse::Capture(CaptureWindowTarget::PreferWindow { window_id: 1 }),
                WindowInventoryUse::Capture(CaptureWindowTarget::TerminalWindowHeuristic),
                WindowInventoryUse::Identification,
                WindowInventoryUse::MarkerIdentification,
            ] {
                assert!(
                    inventory
                        .read(usage, || now + Duration::from_secs(tick), &mut source)
                        .is_empty()
                );
                assert_eq!(
                    source.searches,
                    usize::try_from(tick / DESKTOP_RETRY_INTERVAL.as_secs() + 1)
                        .unwrap_or_default()
                );
            }
            assert!(
                matches!(&inventory.search, TerminalWindowSearch::Found { uuids, .. } if uuids.is_empty())
            );
        }
        source.answering = vec!["{two}".to_owned()];
        assert_eq!(
            inventory
                .read(
                    WindowInventoryUse::Identification,
                    || now + DESKTOP_RETRY_INTERVAL * 3,
                    &mut source
                )
                .len(),
            1
        );
        assert_eq!(source.searches, 4);
    }

    #[test]
    fn a_short_frame_axis_is_penalized_more_than_extra_space() {
        assert!(mismatch(90.0, 100.0) > mismatch(110.0, 100.0));
    }

    #[test]
    fn registry_returns_the_same_handle_for_one_uuid() {
        let mut registry = WindowRegistry::default();
        let first = registry.register("{one}".to_owned());
        let second = registry.register("{one}".to_owned());
        assert_eq!(first, second);
    }
}
