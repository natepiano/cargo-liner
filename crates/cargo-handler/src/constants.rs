//! Constants for `cargo-handler`.

use std::time::Duration;

// configuration
/// Directory under the OS config root holding `config.toml`,
/// `keymap.toml`, `favorites.toml` and `themes/`.
pub(crate) const CONFIG_DIRNAME: &str = "cargo-handler";
/// Id of the built-in dark variant, and the `appearance.dark_theme`
/// default. Defined in [`crate::theme`], not in `tui_pane`: theme
/// content belongs to the app.
pub(crate) const DEFAULT_DARK_THEME: &str = "Default Dark";
/// Id of the built-in high-contrast dark variant.
pub(crate) const DEFAULT_HC_DARK_THEME: &str = "High Contrast Dark";
/// Id of the built-in high-contrast light variant.
pub(crate) const DEFAULT_HC_LIGHT_THEME: &str = "High Contrast Light";
/// Id of the built-in light variant, and the `appearance.light_theme`
/// default.
pub(crate) const DEFAULT_LIGHT_THEME: &str = "Default Light";
/// What a test build uses in place of the OS config directory. It sits
/// under the filesystem root, where a test has no permission to create
/// it, so a test that saves a setting fails instead of writing a file.
#[cfg(test)]
pub(crate) const TEST_CONFIG_ROOT: &str = "/<config>";

// lifecycle
/// The binary's own name: what the command line calls itself in help
/// and in anything it reports going wrong, and the fallback executable
/// name when the running binary's path cannot be resolved for a
/// restart. Distinct from [`APP_NAME`], which is padded for the status
/// line.
pub(crate) const BINARY_NAME: &str = "cargo-handler";
/// The one line `--help` opens with. A placeholder until the tool's
/// purpose is written up.
pub(crate) const CLI_ABOUT: &str = "A terminal UI cargo tool";
/// The word cargo knows this tool by, which is the binary's name with
/// cargo's own prefix taken off. Cargo runs `cargo handler ...` by
/// finding `cargo-handler` on the path and handing it this word ahead
/// of every other argument, so the command line drops it before
/// parsing.
pub(crate) const SUBCOMMAND_NAME: &str = "handler";

// settings overlay
/// Section heading the settings overlay puts the grid's rows under.
pub(crate) const TILES_SETTINGS_SECTION: &str = "Tiles";
/// Section heading the settings overlay puts the remote machine list
/// under.
pub(crate) const MACHINES_SETTINGS_SECTION: &str = "Machines";
/// Label of the settings row holding `machines.remote`.
pub(crate) const REMOTE_MACHINES_LABEL: &str = "remote";

// status line and overlays
/// Section heading the keymap overlay gives this app's globals scope.
pub(crate) const APP_GLOBALS_SECTION: &str = "App Shortcuts";
/// Label leading the status line's version note. The spaces around it
/// are its padding -- the framework adds none.
pub(crate) const APP_NAME: &str = " cargo-handler ";
/// Version shown beside [`APP_NAME`], read from the manifest at compile
/// time so a running instance always says which build it is.
pub(crate) const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// What the status line says while the attract screen is being shown
/// because it was asked for: the grid is still there, it is being
/// drawn over.
pub(crate) const ATTRACT_NOTE_LABEL: &str = "attract";
/// Comment block the keymap editor writes above the generated tables.
pub(crate) const KEYMAP_TOML_HEADER: &str = "\
# cargo-handler keymap configuration\n\
# Edit bindings below. Format: action = \"key\" or \"modifier-key\"\n\
# Modifiers: ctrl, alt, shift.  Examples: \"ctrl-k\", \"shift-tab\", \"q\"\n\
# Chord steps are space-separated, e.g. \"g g\".\n\n";
/// Rows the status line occupies along the bottom of the terminal.
pub(crate) const STATUS_LINE_HEIGHT: u16 = 1;

// tiles
/// The summary cell's title, set into its top border. The leading space
/// holds the word off the corner glyph the title is set against.
pub(crate) const SUMMARY_CELL_TITLE: &str = " summary";
/// Leads an agent cell's title, holding the agent's name off the corner
/// glyph the way the space leading [`SUMMARY_CELL_TITLE`] does.
pub(crate) const AGENT_CELL_TITLE_LEAD: &str = " ";

// agent cells
/// The column labels of an agent cell's table, in column order.
pub(crate) const CHILD_HEADERS: [&str; 4] = ["pid", "kind", "name", "age"];
/// Index of the `pid` column in [`CHILD_HEADERS`].
pub(crate) const CHILD_PID_COLUMN: usize = 0;
/// Index of the `kind` column in [`CHILD_HEADERS`].
pub(crate) const CHILD_KIND_COLUMN: usize = 1;
/// Index of the `name` column in [`CHILD_HEADERS`], the one column cut
/// to whatever width the fitted columns leave.
pub(crate) const CHILD_NAME_COLUMN: usize = 2;
/// Index of the `age` column in [`CHILD_HEADERS`].
pub(crate) const CHILD_AGE_COLUMN: usize = 3;
/// Cells the `kind` column is indented by for each level a row sits
/// below the agent.
pub(crate) const CHILD_KIND_INDENT: usize = 2;
/// Rows the header above an agent cell's table takes: the line naming
/// the agent's pid, program, status, age and machine, then its
/// directory.
pub(crate) const AGENT_HEADER_HEIGHT: u16 = 2;
/// Rows the line naming the agent that launched a session takes.
pub(crate) const LAUNCHER_LINE_HEIGHT: u16 = 1;
/// Blank rows between an agent cell's header and its table.
pub(crate) const AGENT_HEADER_GAP_HEIGHT: u16 = 1;
/// Rows the note standing in for an empty table takes.
pub(crate) const NOTHING_RUNNING_HEIGHT: u16 = 1;
/// What an agent cell says in place of its table when the agent is
/// running nothing.
pub(crate) const NOTHING_RUNNING_NOTE: &str = "nothing running";
/// Leads the agent's pid in an agent cell's header, and names a
/// launcher known by its pid alone.
pub(crate) const PID_LABEL: &str = "pid";
/// Leads the name of the agent that opened a session in its cell.
pub(crate) const LAUNCHED_BY_LABEL: &str = "launched by";
/// The `kind` of a command an agent's shell tool is running.
pub(crate) const SHELL_KIND: &str = "shell";
/// The `kind` of a subagent.
pub(crate) const SUBAGENT_KIND: &str = "subagent";
/// The `kind` of an agent another agent opened in a tmux session.
pub(crate) const SESSION_KIND: &str = "session";
/// The `kind` of a thread a Codex app server runs.
pub(crate) const THREAD_KIND: &str = "thread";

// summary table
/// The summary's column labels, in column order.
pub(crate) const SUMMARY_HEADERS: [&str; 6] =
    ["pid", "agent", "name", "status", "age", "directory"];
/// Index of the `pid` column in [`SUMMARY_HEADERS`].
pub(crate) const PID_COLUMN: usize = 0;
/// Index of the `agent` column in [`SUMMARY_HEADERS`].
pub(crate) const AGENT_COLUMN: usize = 1;
/// Index of the `name` column in [`SUMMARY_HEADERS`].
pub(crate) const NAME_COLUMN: usize = 2;
/// Index of the `status` column in [`SUMMARY_HEADERS`].
pub(crate) const STATUS_COLUMN: usize = 3;
/// Index of the `age` column in [`SUMMARY_HEADERS`].
pub(crate) const AGE_COLUMN: usize = 4;
/// Index of the `directory` column in [`SUMMARY_HEADERS`], the one
/// column that takes whatever the fitted columns leave.
pub(crate) const DIRECTORY_COLUMN: usize = 5;
/// Cells the `name` column grows to at most. A longer name is cut to
/// fit and ends in [`TRUNCATION_MARK`].
pub(crate) const NAME_COLUMN_MAX: u16 = 36;
/// Ends a name cut to fit its column.
pub(crate) const TRUNCATION_MARK: char = '…';
/// Rows the column-label row under a machine's heading occupies. Every
/// machine that lists an agent has one; a machine that lists none has
/// none.
pub(crate) const TABLE_HEADER_HEIGHT: u16 = 1;
/// Rows the machine heading above each machine's rows occupies.
pub(crate) const GROUP_HEADER_HEIGHT: u16 = 1;
/// Blank rows between one machine's rows and the next machine's
/// heading.
pub(crate) const GROUP_GAP_HEIGHT: u16 = 1;
/// Blank cells between table columns.
pub(crate) const TABLE_COLUMN_SPACING: u16 = 2;
/// Sets a machine heading off from what is said about it.
pub(crate) const HEADING_SEPARATOR: &str = " · ";
/// What a machine heading says before the machine's first answer.
pub(crate) const SCANNING_NOTE: &str = "scanning";
/// What a machine heading says when the machine has no agents.
pub(crate) const NO_AGENTS_NOTE: &str = "no agents";
/// Counts one agent in a machine heading.
pub(crate) const AGENT_SINGULAR: &str = "agent";
/// Counts any other number of agents in a machine heading.
pub(crate) const AGENT_PLURAL: &str = "agents";
/// Shown in a cell that has no value, such as the status of an agent
/// that reports none.
pub(crate) const MISSING_VALUE: &str = "—";
/// The units an age is written in, largest first: seconds in the unit,
/// then its suffix.
pub(crate) const AGE_UNITS: [(u64, &str); 4] = [(86_400, "d"), (3_600, "h"), (60, "m"), (1, "s")];
/// The age written for no time at all.
pub(crate) const ZERO_AGE: &str = "0s";
/// Below this, an age's leading value is a single digit and the next
/// unit down is written after it.
pub(crate) const AGE_DETAIL_BELOW: u64 = 10;

// theme roles
/// `[variants.roles]` key for the `claude` agent label.
pub(crate) const CLAUDE_ROLE: &str = "claude";
/// `[variants.roles]` key for the `codex` agent label.
pub(crate) const CODEX_ROLE: &str = "codex";
/// `[variants.roles]` key for a `busy` status.
pub(crate) const BUSY_ROLE: &str = "busy";
/// `[variants.roles]` key for a `shell` status.
pub(crate) const SHELL_ROLE: &str = "shell";
/// `[variants.roles]` key for an `idle` status, a status the summary
/// does not know, and a missing one.
pub(crate) const IDLE_ROLE: &str = "idle";
/// `[variants.roles]` key for why a remote machine gave no answer.
pub(crate) const UNREACHABLE_ROLE: &str = "unreachable";

// census
/// The `agent` value of a Claude Code row, and the process name a
/// Claude Code process runs under.
pub(crate) const CLAUDE_AGENT: &str = "claude";
/// The `agent` value of a Codex row, and the process name a Codex
/// process runs under.
pub(crate) const CODEX_AGENT: &str = "codex";
/// The argument that marks a `codex` process as an app server: a
/// backend for a client, never an agent someone is typing into.
pub(crate) const CODEX_APP_SERVER_ARGUMENT: &str = "app-server";
/// Name of the macOS desktop app's process, and of the row that stands
/// for it. The app runs its agents through a `codex app-server` child
/// of its own.
pub(crate) const CODEX_DESKTOP_APP: &str = "ChatGPT";
/// Process names a tmux server runs under. On Linux the server renames
/// itself `tmux: server`; on macOS renaming a process does nothing, so
/// the server keeps the name `tmux`.
pub(crate) const TMUX_SERVER_NAMES: [&str; 2] = ["tmux: server", "tmux"];
/// The status a Claude Code session reports while it works.
pub(crate) const BUSY_STATUS: &str = "busy";
/// The status a Claude Code session reports while a shell command it
/// ran is in the foreground.
pub(crate) const SHELL_STATUS: &str = "shell";
/// Characters of a Claude session id a row shows when the session has
/// no name.
pub(crate) const SESSION_ID_PREFIX_LENGTH: usize = 8;
/// Claude Code's directory under the home directory.
pub(crate) const CLAUDE_DIRNAME: &str = ".claude";
/// The directory under [`CLAUDE_DIRNAME`] holding one `<pid>.json`
/// record per running Claude Code session.
pub(crate) const CLAUDE_SESSIONS_DIRNAME: &str = "sessions";
/// Extension of a session record; the directory holds other files too.
pub(crate) const SESSION_RECORD_EXTENSION: &str = "json";
/// The directory under [`CLAUDE_DIRNAME`] holding each session's
/// transcript, in a directory named for where the session started.
pub(crate) const PROJECTS_DIRNAME: &str = "projects";
/// Extension of a transcript, one JSON object per line.
pub(crate) const TRANSCRIPT_EXTENSION: &str = "jsonl";
/// The `type` of a transcript line the session wrote itself.
pub(crate) const ASSISTANT_LINE: &str = "assistant";
/// The `type` of a transcript line carrying the reader's turn or a tool
/// result.
pub(crate) const USER_LINE: &str = "user";
/// The name Claude Code gives its shell tool.
pub(crate) const BASH_TOOL: &str = "Bash";
/// Text every transcript line calling the shell tool holds. A line
/// without it is read for its time alone.
pub(crate) const BASH_TOOL_MARKER: &str = r#""name":"Bash""#;
/// The `type` of a message block that calls a tool.
pub(crate) const TOOL_USE_BLOCK: &str = "tool_use";
/// The `stop_reason` of an assistant message that ends its turn.
pub(crate) const END_TURN: &str = "end_turn";
/// The directory beside a session's transcript, named for the session,
/// under which its subagents' transcripts sit.
pub(crate) const SUBAGENTS_DIRNAME: &str = "subagents";
/// How the name of a subagent's files starts: `agent-<id>.jsonl` and
/// `agent-<id>.meta.json`.
pub(crate) const SUBAGENT_PREFIX: &str = "agent-";
/// How the name of the file saying what a subagent was asked to do ends.
pub(crate) const SUBAGENT_META_SUFFIX: &str = ".meta.json";
/// How long a subagent's transcript may go unwritten before the
/// subagent counts as stopped rather than working.
pub(crate) const SUBAGENT_QUIET_LIMIT: Duration = Duration::from_mins(30);
/// Bytes a bisection of a transcript narrows the start of a span to
/// before reading forward line by line.
pub(crate) const TRANSCRIPT_BISECT_GRAIN: u64 = 64 * 1024;
/// Bytes read forward from the start of a span before the read gives
/// up, however far the span runs.
pub(crate) const TRANSCRIPT_SPAN_READ_LIMIT: u64 = 32 * 1024 * 1024;
/// Bytes of a transcript's end read first when looking for its last
/// turn.
pub(crate) const TRANSCRIPT_TAIL_START: u64 = 64 * 1024;
/// Bytes of a transcript's end read at most when looking for its last
/// turn.
pub(crate) const TRANSCRIPT_TAIL_LIMIT: u64 = 16 * 1024 * 1024;
/// The flag Claude Code's shell tool runs its script under.
pub(crate) const SHELL_SCRIPT_FLAG: &str = "-c";
/// Text the script of every Claude Code shell tool call holds: it
/// starts by sourcing a snapshot of the reader's shell from here.
pub(crate) const SHELL_SNAPSHOT_MARKER: &str = "/shell-snapshots/snapshot-";
/// What opens the command inside a shell tool script, which runs it
/// quoted in single quotes.
pub(crate) const SHELL_EVAL_OPEN: &str = "eval '";
/// How a shell tool script writes one `'` inside the quoted command.
pub(crate) const SHELL_QUOTED_QUOTE: &str = r"'\''";
/// How far before a process started the transcript is read for the
/// call that started it.
pub(crate) const CALL_LOOKBACK: Duration = Duration::from_mins(10);
/// How far after a process started the call that started it may be
/// written: the process table counts a start in whole seconds, and a
/// transcript line is written as the call goes out.
pub(crate) const CALL_LOOKAHEAD: Duration = Duration::from_secs(5);
/// What the command of a shell call that opened a tmux session holds,
/// and the word naming that tmux command.
pub(crate) const TMUX_NEW_SESSION: &str = "new-session";
/// The flags of tmux's `new-session` that take an argument.
pub(crate) const TMUX_ARGUMENT_FLAGS: &str = "cefFnstxy";
/// The `new-session` flag naming the session it opens.
pub(crate) const TMUX_SESSION_NAME_FLAG: char = 's';
/// The word that ends a tmux command's options.
pub(crate) const TMUX_OPTIONS_END: &str = "--";
/// How old a process must be before a scan that found no call or
/// launcher for it stops looking. A younger one may not have had its
/// transcript line written yet.
pub(crate) const CALL_SEARCH_SETTLE: Duration = Duration::from_secs(30);
/// Levels of agents under agents an agent cell's tree follows at most,
/// so a process table that loops cannot hold the scan.
pub(crate) const TREE_DEPTH_LIMIT: u8 = 16;
/// Codex's directory under the home directory.
pub(crate) const CODEX_DIRNAME: &str = ".codex";
/// How the name of Codex's thread database starts. The whole name is
/// `state_<n>.sqlite`, `<n>` counting the versions of its layout, and
/// the highest `<n>` present is the one Codex writes.
pub(crate) const CODEX_STATE_PREFIX: &str = "state_";
/// Extension of Codex's thread database. Its journal files beside it
/// carry longer ones.
pub(crate) const CODEX_STATE_EXTENSION: &str = "sqlite";
/// How long a read of Codex's thread database waits on a writer before
/// giving up on the scan's names.
pub(crate) const CODEX_STATE_BUSY_TIMEOUT: Duration = Duration::from_millis(200);
/// The threads an interactive Codex created within a span of creation
/// times: `?1` is [`CODEX_TUI_ORIGINATOR`], `?2` and `?3` the span's
/// ends in unix milliseconds.
pub(crate) const CODEX_THREADS_QUERY: &str = "SELECT cwd, created_at_ms, name, first_user_message \
     FROM threads WHERE originator = ?1 AND created_at_ms BETWEEN ?2 AND ?3";
/// The `originator` Codex records on a thread the interactive `codex`
/// started, as against `codex exec`, an app server's client, or the
/// desktop app.
pub(crate) const CODEX_TUI_ORIGINATOR: &str = "codex-tui";
/// How long after an interactive Codex starts the thread it starts with
/// may be created. Codex creates it during startup, after any question
/// about trusting the directory.
pub(crate) const CODEX_THREAD_START_WINDOW: Duration = Duration::from_secs(60);
/// How long before an interactive Codex's recorded start its thread may
/// be created: the process table counts a start in whole seconds from a
/// boot time that is itself read in whole seconds.
pub(crate) const CODEX_THREAD_START_SLACK: Duration = Duration::from_secs(1);
/// Characters of a Codex thread's first prompt a row carries when the
/// thread has no name. The summary cuts it again to fit its column.
pub(crate) const CODEX_PROMPT_LABEL_MAX: usize = 80;
/// Any thread, whoever started it, by its id: `?1` is the id.
pub(crate) const CODEX_THREAD_BY_ID_QUERY: &str =
    "SELECT cwd, created_at_ms, name, first_user_message FROM threads WHERE id = ?1";
/// The directory under [`CODEX_DIRNAME`] holding the files Codex writes
/// each thread's conversation to, one directory per day.
pub(crate) const CODEX_SESSIONS_DIRNAME: &str = "sessions";
/// How the name of a thread's conversation file starts. The whole name
/// is `rollout-<time>-<thread id>.jsonl`.
pub(crate) const CODEX_ROLLOUT_PREFIX: &str = "rollout-";
/// Extension of a thread's conversation file.
pub(crate) const CODEX_ROLLOUT_EXTENSION: &str = "jsonl";
/// Characters in a thread id, the hyphenated UUID that ends the stem of
/// its conversation file.
pub(crate) const CODEX_THREAD_ID_LENGTH: usize = 36;
/// What a `codex app-server` row is called.
pub(crate) const CODEX_APP_SERVER_LABEL: &str = "app-server";
/// The environment variable Claude Code sets for every process it
/// starts, naming the Claude Code process. A Codex app server carries it
/// after it has been moved out from under the process that started it.
pub(crate) const CLAUDE_PID_VARIABLE: &str = "CLAUDE_PID";
/// Where each process's open files are listed on Linux, one link per
/// descriptor under `/proc/<pid>/fd`.
pub(crate) const PROC_DIRNAME: &str = "/proc";
/// The directory under a process's `/proc` entry holding its open
/// descriptors.
pub(crate) const PROC_FD_DIRNAME: &str = "fd";
/// Written in place of the home directory in a row's directory.
pub(crate) const HOME_ABBREVIATION: &str = "~";
/// This machine's heading when its host name cannot be read.
pub(crate) const LOCAL_MACHINE_FALLBACK: &str = "localhost";
/// How often the scheduler scans this machine.
pub(crate) const LOCAL_SCAN_INTERVAL: Duration = Duration::from_secs(2);
/// How often the scheduler asks each remote machine for its agents.
pub(crate) const REMOTE_PROBE_INTERVAL: Duration = Duration::from_secs(5);

// remote probe
/// The hidden subcommand that prints this machine's agents as JSON,
/// which a remote cargo-handler runs over ssh.
pub(crate) const PROBE_COMMAND: &str = "probe";
/// Version of the probe's JSON. A machine answering with another
/// version is reported rather than read.
pub(crate) const PROBE_SCHEMA: u32 = 2;
/// The program the probe runs through.
pub(crate) const SSH_PROGRAM: &str = "ssh";
/// ssh options every probe passes: never prompt, and give up on a
/// connection that does not open in five seconds.
pub(crate) const SSH_BASE_OPTIONS: [&str; 4] = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"];
/// The directory under the home directory ssh keeps its files in.
pub(crate) const SSH_DIRNAME: &str = ".ssh";
/// The control socket's file name. ssh expands `%C` into a hash of the
/// connection, so each host gets a socket of its own.
pub(crate) const SSH_CONTROL_SOCKET: &str = "cargo-handler-%C";
/// The token ssh expands in [`SSH_CONTROL_SOCKET`].
pub(crate) const SSH_CONNECTION_HASH_TOKEN: &str = "%C";
/// Characters `%C` expands to: a hex SHA-1.
pub(crate) const SSH_CONNECTION_HASH_LENGTH: usize = 40;
/// Bytes a Unix socket path must stay under. A control path at or past
/// it would fail every probe, so the probe goes without one instead.
pub(crate) const SSH_CONTROL_PATH_LIMIT: usize = 100;
/// Seconds an idle shared connection stays open after its last probe.
pub(crate) const SSH_CONTROL_PERSIST: &str = "ControlPersist=60";
/// Opens a shared connection when none is open, and uses it when one
/// is.
pub(crate) const SSH_CONTROL_MASTER: &str = "ControlMaster=auto";
/// How long a probe runs before it is killed.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// How often a probe that has closed its output is checked for having
/// exited.
pub(crate) const PROBE_EXIT_CHECK: Duration = Duration::from_millis(20);
/// ssh's exit status when it could not reach the host.
pub(crate) const SSH_UNREACHABLE_STATUS: i32 = 255;
/// The shell's exit status for a command it could not find.
pub(crate) const COMMAND_NOT_FOUND_STATUS: i32 = 127;
/// Why a remote gave no answer: ssh could not reach it.
pub(crate) const UNREACHABLE_REASON: &str = "unreachable";
/// Why a remote gave no answer: it has no `cargo-handler` to run.
pub(crate) const NOT_INSTALLED_REASON: &str = "cargo-handler not installed";
/// Why a remote gave no answer: its probe ran past [`PROBE_TIMEOUT`].
pub(crate) const TIMED_OUT_REASON: &str = "timed out";
/// Why a remote gave no answer: its output was not the probe's JSON.
pub(crate) const UNREADABLE_PROBE_REASON: &str = "unreadable probe output";
/// Why a remote gave no answer: ssh itself could not be started.
pub(crate) const SSH_NOT_STARTED_REASON: &str = "ssh could not start";
/// Why a remote gave no answer: the probe was ended by a signal.
pub(crate) const PROBE_SIGNALLED_REASON: &str = "probe ended by a signal";
