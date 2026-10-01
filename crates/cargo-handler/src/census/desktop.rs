//! Which KDE virtual desktop each agent's terminal window is on.
//!
//! `KWin` lists every window with the pid of the process that owns it,
//! its caption and the desktops it is on. An agent's terminal is the
//! nearest process at or above the agent that owns a window. A terminal
//! that owns several windows, such as a single-instance Ghostty, is
//! narrowed by caption: Claude Code titles its window with the session's
//! name, behind a status glyph, and Codex with the thread's name and the
//! directory's.
//!
//! An agent a tmux server holds has no terminal above it. The pane it
//! runs in names its session, and its window is the one showing a tmux
//! client of that session: the nearest process at or above the client
//! that owns a window. A session no client shows is detached, and has
//! no window.

use std::collections::HashMap;
use std::iter;
use std::path::Path;
use std::ptr;

use serde::Deserialize;

use super::AgentRow;
use super::classify;
use super::classify::ProcessEntry;
use crate::constants::ALL_DESKTOPS_LABEL;
use crate::constants::CODEX_CAPTION_SEPARATOR;
use crate::constants::DESKTOP_SEPARATOR;
use crate::constants::TMUX_PROGRAM;

/// One window as `KWin` lists it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(super) struct WindowEntry {
    /// The pid of the process that owns the window.
    pub(super) pid:          u32,
    /// The window's title.
    pub(super) caption:      String,
    /// The names of the virtual desktops the window is on, in `KWin`'s
    /// order.
    pub(super) desktops:     Vec<String>,
    /// Whether the window is on every virtual desktop.
    pub(super) all_desktops: bool,
}

/// The default tmux server's panes and clients, each with the session
/// it belongs to.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct TmuxLayout {
    /// The session each pane belongs to, by the pid of the pane's first
    /// process.
    panes:   HashMap<u32, String>,
    /// Each client's pid, with the name of the session it shows.
    clients: Vec<(u32, String)>,
}

impl TmuxLayout {
    /// The layout read from what `list-panes` and `list-clients` print,
    /// one `<pid> <session>` to the line. A line that does not read that
    /// way is skipped.
    pub(super) fn parse(panes: &str, clients: &str) -> Self {
        Self {
            panes:   panes.lines().filter_map(pid_and_session).collect(),
            clients: clients.lines().filter_map(pid_and_session).collect(),
        }
    }
}

/// One `<pid> <session>` line: the pid, then the session's name, which
/// may itself hold spaces.
fn pid_and_session(line: &str) -> Option<(u32, String)> {
    let (pid, session) = line.split_once(' ')?;
    Some((pid.parse().ok()?, session.to_string()))
}

/// The windows `KWin` answered with, read from the JSON
/// [`WINDOW_LIST_EXPRESSION`](crate::constants::WINDOW_LIST_EXPRESSION)
/// evaluates to; none where it does not parse.
#[cfg(any(target_os = "linux", test))]
pub(super) fn parse_windows(answer: &str) -> Option<Vec<WindowEntry>> {
    serde_json::from_str(answer).ok()
}

/// The program to ask the tmux server holding one of `rows` for its
/// panes and clients: the absolute path the server was started from,
/// or else [`TMUX_PROGRAM`] looked up on `PATH`. None when tmux holds
/// none of `rows`, and then tmux is not asked.
pub(super) fn tmux_program(processes: &[ProcessEntry], rows: &[AgentRow]) -> Option<String> {
    let table = classify::pid_table(processes);
    let server = rows
        .iter()
        .filter_map(|row| table.get(&row.pid))
        .find_map(|process| {
            classify::ancestors(&table, process).find(|ancestor| ancestor.is_tmux_server())
        })?;
    let program = server
        .arguments
        .first()
        .filter(|program| Path::new(program).is_absolute())
        .map_or(TMUX_PROGRAM, String::as_str);
    Some(program.to_string())
}

/// Set the desktop of each of `rows` from the window matched to it
/// among `windows`, reading `tmux` for the rows a tmux server holds.
/// A row with no match, or held by tmux when `tmux` is none, gets no
/// desktop.
pub(super) fn attach_desktops(
    rows: &mut [AgentRow],
    processes: &[ProcessEntry],
    windows: &[WindowEntry],
    tmux: Option<&TmuxLayout>,
) {
    let table = classify::pid_table(processes);
    let mut owned: HashMap<u32, Vec<&WindowEntry>> = HashMap::new();
    for window in windows {
        owned.entry(window.pid).or_default().push(window);
    }
    for row in rows.iter_mut() {
        row.desktop = row_window(row, &table, &owned, tmux).and_then(desktop_label);
    }
}

/// The window `row`'s agent is in, found through its terminal or, when
/// a tmux server holds it, through the clients showing its session.
fn row_window<'w>(
    row: &AgentRow,
    table: &HashMap<u32, &ProcessEntry>,
    owned: &HashMap<u32, Vec<&'w WindowEntry>>,
    tmux: Option<&TmuxLayout>,
) -> Option<&'w WindowEntry> {
    let process = table.get(&row.pid)?;
    let lineage: Vec<&ProcessEntry> = iter::once(*process)
        .chain(classify::ancestors(table, process))
        .collect();
    let Some(server) = lineage.iter().position(|process| process.is_tmux_server()) else {
        let windows = lineage.iter().find_map(|process| owned.get(&process.pid))?;
        return choose(windows, &[row.name.as_str()]);
    };
    let tmux = tmux?;
    let session = lineage[..server]
        .iter()
        .find_map(|process| tmux.panes.get(&process.pid))?;
    let mut candidates: Vec<&WindowEntry> = Vec::new();
    for (client, _) in tmux.clients.iter().filter(|(_, shown)| shown == session) {
        for window in terminal_windows(*client, table, owned) {
            if !candidates.iter().any(|known| ptr::eq(*known, *window)) {
                candidates.push(*window);
            }
        }
    }
    choose(&candidates, &[row.name.as_str(), session.as_str()])
}

/// The windows of the terminal `pid` runs in: those of the nearest
/// process at or above it that owns any.
fn terminal_windows<'a, 'w>(
    pid: u32,
    table: &HashMap<u32, &ProcessEntry>,
    owned: &'a HashMap<u32, Vec<&'w WindowEntry>>,
) -> &'a [&'w WindowEntry] {
    table
        .get(&pid)
        .and_then(|process| {
            iter::once(*process)
                .chain(classify::ancestors(table, process))
                .find_map(|process| owned.get(&process.pid))
        })
        .map_or(&[], Vec::as_slice)
}

/// The one window among `candidates`: the only candidate, or else the
/// only one whose caption names one of `names`.
fn choose<'w>(candidates: &[&'w WindowEntry], names: &[&str]) -> Option<&'w WindowEntry> {
    if let [only] = candidates {
        return Some(*only);
    }
    let mut captioned = candidates.iter().filter(|window| {
        names
            .iter()
            .any(|name| caption_names(&window.caption, name))
    });
    let first = captioned.next()?;
    captioned.next().is_none().then_some(*first)
}

/// Whether `caption`, past any status glyph, is `name`, or is `name`
/// followed by [`CODEX_CAPTION_SEPARATOR`] and a directory, as Codex
/// titles its window.
fn caption_names(caption: &str, name: &str) -> bool {
    let caption = past_glyph(caption);
    let name = past_glyph(name);
    caption == name
        || caption
            .rsplit_once(CODEX_CAPTION_SEPARATOR)
            .is_some_and(|(thread, _)| thread == name)
}

/// `text` from its first letter or digit on, which drops the status
/// glyph Claude Code writes ahead of a session's name in its caption.
fn past_glyph(text: &str) -> &str {
    text.trim_start_matches(|character: char| !character.is_alphanumeric())
        .trim_end()
}

/// What the desktop column says for `window`: the names of its
/// desktops, [`ALL_DESKTOPS_LABEL`] when it is on every one, and none
/// when `KWin` names none.
fn desktop_label(window: &WindowEntry) -> Option<String> {
    if window.all_desktops {
        return Some(ALL_DESKTOPS_LABEL.to_string());
    }
    (!window.desktops.is_empty()).then(|| window.desktops.join(DESKTOP_SEPARATOR))
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::census::Agent;
    use crate::census::ServiceTier;

    /// The single-instance terminal that owns several windows.
    const GHOSTTY: u32 = 44_122;
    /// The tmux server holding the launched sessions.
    const SERVER: u32 = 3_261_729;

    /// A process with no command line read.
    fn process(pid: u32, parent: u32, name: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent: Some(parent),
            name: name.to_string(),
            arguments: Vec::new(),
            started: 0,
            directory: None,
            claude_pid: None,
        }
    }

    /// A window on the desktops named in `desktops`.
    fn window(pid: u32, caption: &str, desktops: &[&str]) -> WindowEntry {
        WindowEntry {
            pid,
            caption: caption.to_string(),
            desktops: desktops
                .iter()
                .map(|desktop| (*desktop).to_string())
                .collect(),
            all_desktops: false,
        }
    }

    /// A Claude Code row with no desktop yet.
    fn row(pid: u32, name: &str) -> AgentRow {
        AgentRow {
            agent: Agent::Claude,
            service_tier: ServiceTier::Unrecorded,
            name: name.to_string(),
            status: None,
            started: 0,
            pid,
            desktop: None,
            directory: "~".to_string(),
            branch: None,
            launched_by: None,
            children: Vec::new(),
        }
    }

    /// The processes of a desktop session: a terminal of its own window,
    /// a single-instance terminal with shells in several windows, and a
    /// tmux server with a client of one of its sessions in a terminal of
    /// its own.
    fn processes() -> Vec<ProcessEntry> {
        vec![
            process(45_448, 1, ".ghostty-wrappe"),
            process(45_460, 45_448, "zsh"),
            process(1_579_022, 45_460, "claude"),
            process(GHOSTTY, 1, ".ghostty-wrappe"),
            process(428_000, GHOSTTY, "zsh"),
            process(428_044, 428_000, "claude"),
            process(2_429_900, GHOSTTY, "zsh"),
            process(2_429_982, 2_429_900, "claude"),
            process(4_014_317, GHOSTTY, "zsh"),
            process(4_039_085, 4_014_317, "codex"),
            process(SERVER, 1, "tmux: server"),
            process(3_266_307, SERVER, "zsh"),
            process(3_266_367, 3_266_307, "claude"),
            process(3_331_350, SERVER, "zsh"),
            process(3_331_942, 3_331_350, "claude"),
            process(3_265_040, 1, ".ghostty-wrappe"),
            process(2_452_655, 3_265_040, "tmux"),
        ]
    }

    /// Every window of [`processes`].
    fn windows() -> Vec<WindowEntry> {
        vec![
            window(45_448, "✳ boss of bosses", &["boss"]),
            window(GHOSTTY, "◐ enh/handler", &["cargo handler"]),
            window(GHOSTTY, "✳ integration tests", &["bevy 0.20.0-rc.1"]),
            window(GHOSTTY, "~/rust/handler", &["cargo handler"]),
            window(GHOSTTY, "codex test | handler", &["daily reading"]),
            window(3_265_040, "tool-based-ui-trunk", &["berth_fix"]),
        ]
    }

    /// The tmux server of [`processes`]: two sessions, one shown by a
    /// client and one detached.
    fn tmux() -> TmuxLayout {
        TmuxLayout::parse(
            "3266307 tool-based-ui-trunk\n3331350 tool-based-ui-geometry-material\n",
            "2452655 tool-based-ui-trunk\n",
        )
    }

    /// The desktop each of `rows` is given against [`processes`],
    /// [`windows`] and [`tmux`].
    fn desktops(mut rows: Vec<AgentRow>) -> Vec<Option<String>> {
        attach_desktops(&mut rows, &processes(), &windows(), Some(&tmux()));
        rows.into_iter().map(|row| row.desktop).collect()
    }

    /// A terminal that owns one window gives that window's desktop,
    /// whatever its caption says.
    #[test]
    fn a_terminal_with_one_window_gives_its_desktop_by_pid() {
        assert_eq!(
            desktops(vec![row(1_579_022, "another name")]),
            [Some("boss".to_string())]
        );
    }

    /// A terminal that owns several windows gives the desktop of the one
    /// whose caption, past its status glyph, is the session's name.
    #[test]
    fn several_windows_are_told_apart_by_caption() {
        assert_eq!(
            desktops(vec![
                row(428_044, "enh/handler"),
                row(2_429_982, "integration tests"),
            ]),
            [
                Some("cargo handler".to_string()),
                Some("bevy 0.20.0-rc.1".to_string()),
            ]
        );
    }

    /// A Codex window is told apart by the thread's name ahead of the
    /// directory's in its caption.
    #[test]
    fn a_codex_window_is_told_apart_by_its_thread_name() {
        assert_eq!(
            desktops(vec![row(4_039_085, "codex test")]),
            [Some("daily reading".to_string())]
        );
    }

    /// A terminal that owns several windows, none captioned with the
    /// session's name, gives no desktop.
    #[test]
    fn several_windows_with_no_caption_match_give_no_desktop() {
        assert_eq!(desktops(vec![row(428_044, "renamed session")]), [None]);
    }

    /// A session tmux holds is found through its pane's session, the
    /// client showing that session, and the client's terminal window.
    #[test]
    fn a_tmux_session_is_found_through_the_client_showing_it() {
        assert_eq!(
            desktops(vec![row(3_266_367, "tool-based-ui-trunk")]),
            [Some("berth_fix".to_string())]
        );
    }

    /// A session tmux holds that no client shows is detached, and has no
    /// desktop.
    #[test]
    fn a_detached_tmux_session_gives_no_desktop() {
        assert_eq!(
            desktops(vec![row(3_331_942, "tool-based-ui-geometry-material")]),
            [None]
        );
    }

    /// A session tmux holds gives no desktop when tmux was not read.
    #[test]
    fn a_tmux_session_gives_no_desktop_without_the_tmux_layout() {
        let mut rows = vec![row(3_266_367, "tool-based-ui-trunk")];
        attach_desktops(&mut rows, &processes(), &windows(), None);
        assert_eq!(rows[0].desktop, None);
    }

    /// A window on every virtual desktop is labelled as being on all of
    /// them, and a window on two lists both.
    #[test]
    fn a_window_on_every_desktop_says_so() {
        let mut everywhere = window(45_448, "✳ boss of bosses", &[]);
        everywhere.all_desktops = true;
        assert_eq!(
            desktop_label(&everywhere),
            Some(ALL_DESKTOPS_LABEL.to_string())
        );
        assert_eq!(
            desktop_label(&window(45_448, "✳ boss of bosses", &["boss", "berth_fix"])),
            Some("boss, berth_fix".to_string())
        );
    }

    /// A row whose pid is not in this machine's process table, as a row
    /// a remote machine sent is not, gives no desktop.
    #[test]
    fn a_row_from_another_machine_gives_no_desktop() {
        assert_eq!(desktops(vec![row(80_020, "boss of bosses")]), [None]);
    }

    /// The tmux program is the absolute path the server was started
    /// from, and `tmux` on `PATH` when it was started by a bare name; it
    /// is not asked for at all when tmux holds no row.
    #[test]
    fn the_tmux_program_is_the_servers_own_path() {
        let rows = [row(3_266_367, "tool-based-ui-trunk")];
        let mut table = processes();
        let server = table
            .iter_mut()
            .find(|process| process.pid == SERVER)
            .expect("the server is listed");
        server.arguments = vec!["/nix/store/tmux-3.6a/bin/tmux".to_string()];
        assert_eq!(
            tmux_program(&table, &rows).as_deref(),
            Some("/nix/store/tmux-3.6a/bin/tmux")
        );
        assert_eq!(
            tmux_program(&processes(), &rows).as_deref(),
            Some(TMUX_PROGRAM)
        );
        assert_eq!(
            tmux_program(&processes(), &[row(428_044, "enh/handler")]),
            None
        );
    }

    /// `KWin`'s JSON answer reads as one entry per window.
    #[test]
    fn the_kwin_answer_reads_as_windows() {
        let answer = r#"[{"pid":45448,"caption":"✳ boss of bosses","desktops":["boss"],"all_desktops":false}]"#;
        assert_eq!(
            parse_windows(answer),
            Some(vec![window(45_448, "✳ boss of bosses", &["boss"])])
        );
        assert_eq!(parse_windows("not json"), None);
    }
}
