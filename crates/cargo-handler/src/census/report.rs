//! `cargo-handler census --json` prints the unit directors and workers the
//! tiles show. It reads the same local scan, staff file, accounts read and
//! remote probes as the TUI, once, and takes each status and activity from
//! the tile cells.

use std::collections::BTreeMap;
use std::io;
use std::io::Write;
use std::panic;
use std::process::ExitCode;
use std::thread;

use serde::Serialize;

use super::AgentPlacement;
use super::AgentRow;
use super::Census;
use super::CensusUpdate;
use super::HierarchySnapshot;
use super::MachineState;
use super::Observed;
use super::PassFact;
use super::PassRole;
use super::RemoteHierarchyAnswers;
use super::SourceHealth;
use super::StaffListing;
use super::WorkerFacts;
use super::accounts::AccountsTable;
use super::accounts::ReportedSessionState;
use super::accounts::SessionStates;
use super::activity::DetachedRunFact;
use super::activity::DetachedRunState;
use super::herdr::PaneState;
use super::hierarchy::StaffRunPointer;
use super::local_machine_name;
use super::probe;
use super::remote::RemoteRunner;
use super::remote::SshRunner;
use super::scan::ScanStage;
use super::unix_now;
use crate::agent_cell;
use crate::agent_cell::SessionCells;
use crate::config::CargoHandler;
use crate::config::Config;
use crate::config::LoadedConfig;
use crate::constants::BINARY_NAME;
use crate::constants::CENSUS_COMMAND;
use crate::constants::CENSUS_REASON_SEPARATOR;
use crate::constants::CENSUS_SCHEMA;
use crate::constants::CENSUS_UNANSWERED_REASON;
use crate::startup_log;

/// The `census --json` document. A field rename or removal increments `schema`.
#[derive(Debug, Serialize)]
struct CensusReport {
    /// The document version.
    schema:    u32,
    /// This machine first, then each configured remote in configuration order.
    machines:  Vec<MachineReport>,
    /// Each unit director a tile shows, in staff id order.
    directors: Vec<DirectorReport>,
    /// Each worker of a unit's current run that a tile shows, in staff id order.
    workers:   Vec<WorkerReport>,
}

/// One machine and whether its answer arrived.
#[derive(Debug, Serialize)]
struct MachineReport {
    /// This machine's short host name, or a remote's ssh name.
    name:      String,
    /// Whether the machine answered with its rows.
    reachable: bool,
    /// Why the machine gave no rows, or why its staff or accounts read failed; else null.
    error:     Option<String>,
}

/// One unit director.
#[derive(Debug, Serialize)]
struct DirectorReport {
    /// The staff row id.
    staff_id:            String,
    /// The tile name.
    name:                String,
    /// The machine whose tile shows the session.
    machine:             String,
    /// The staff run pointer: the current run directory, or null with no run.
    current_run_id:      Option<String>,
    /// The tile status word, or null with no accounts entry.
    status:              Option<&'static str>,
    /// The tile activity text.
    activity:            String,
    /// When the current activity started, in epoch seconds, or null when unknown.
    activity_started_at: Option<u64>,
    /// Why a fact above is unknown, or null when every fact is known.
    unknown:             Option<UnknownReport>,
}

/// One worker of a unit's current run.
#[derive(Debug, Serialize)]
struct WorkerReport {
    /// The staff row id.
    staff_id:            String,
    /// The tile name.
    name:                String,
    /// The staff id of the worker's unit director.
    parent_staff_id:     String,
    /// The machine whose tile shows the session.
    machine:             String,
    /// The tile status word, or null with no accounts entry.
    status:              Option<&'static str>,
    /// The tile activity text.
    activity:            String,
    /// When the current activity started, in epoch seconds, or null when unknown.
    activity_started_at: Option<u64>,
    /// The open pass, or null with no open pass or an unknown pass.
    pass:                Option<PassReport>,
    /// The worker's pane.
    pane:                PaneReport,
    /// Each detached run of the unit's current run, or null when the runs cannot be read.
    runs:                Option<Vec<RunReport>>,
    /// Why a fact above is unknown, or null when every fact is known.
    unknown:             Option<UnknownReport>,
}

/// An open pass.
#[derive(Debug, Serialize)]
struct PassReport {
    /// `impl`, `test`, `fix` or `review`.
    role:  PassRole,
    /// The phase instance id.
    phase: String,
}

/// The pane lifecycle, separate from the status word.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum PaneReport {
    Open,
    Stopped,
    Closed,
    Unknown,
}

/// One detached run.
#[derive(Debug, Serialize)]
struct RunReport {
    /// The run label.
    label:      String,
    /// When the run started, in epoch seconds, or null when unknown.
    started_at: Option<u64>,
    /// `running`, `finished_queued` or `missing`.
    state:      DetachedRunState,
    /// Why the start is unknown, or null.
    unknown:    Option<UnknownReport>,
}

/// The reasons the census cannot read some facts.
#[derive(Debug, Serialize)]
struct UnknownReport {
    /// Each reason, joined with `; `.
    reason: String,
}

impl UnknownReport {
    fn of<'a>(reasons: impl IntoIterator<Item = Option<&'a str>>) -> Option<Self> {
        let reasons: Vec<_> = reasons.into_iter().flatten().collect();
        (!reasons.is_empty()).then(|| Self {
            reason: reasons.join(CENSUS_REASON_SEPARATOR),
        })
    }
}

/// The reason a fact is unknown.
const fn reason<T>(fact: &Observed<T>) -> Option<&str> {
    match fact {
        Observed::Known(_) => None,
        Observed::Unknown(reason) => Some(reason.0.as_str()),
    }
}

/// A source that failed or was not read.
const fn source_problem(fact: &Observed<SourceHealth>) -> Option<&str> {
    match fact {
        Observed::Known(SourceHealth::Unreadable(reason)) | Observed::Unknown(reason) => {
            Some(reason.0.as_str())
        },
        Observed::Known(SourceHealth::Loaded | SourceHealth::Absent) => None,
    }
}

const fn known<T: Copy>(fact: &Observed<T>) -> Option<T> {
    match fact {
        Observed::Known(value) => Some(*value),
        Observed::Unknown(_) => None,
    }
}

/// A listed director or worker: its tile cells and the facts of the machine that shows it.
struct Listed<'a> {
    listing: &'a StaffListing,
    cell:    &'a SessionCells,
    facts:   &'a WorkerFacts,
}

impl CensusReport {
    fn build(census: &Census, remotes: &[String], states: &SessionStates, now: u64) -> Self {
        let machines = census.machines(remotes);
        let placement = AgentPlacement::from_machines(&machines, states);
        let cells = agent_cell::session_cells(&placement, now);
        let mut listed = BTreeMap::new();
        for machine in &machines {
            let MachineState::Answered(_, hierarchy) = machine.state else {
                continue;
            };
            let Observed::Known(workers) = &hierarchy.facts.workers else {
                continue;
            };
            for (session, facts) in workers {
                // Only the machine whose tile shows the session has its pane and runs.
                let Some(cell) = cells
                    .get(session.as_str())
                    .filter(|cell| cell.machine == machine.name)
                else {
                    continue;
                };
                let Observed::Known(
                    listing @ (StaffListing::Director { id } | StaffListing::Worker { id, .. }),
                ) = &facts.listing
                else {
                    continue;
                };
                listed.insert(
                    id.as_str(),
                    Listed {
                        listing,
                        cell,
                        facts,
                    },
                );
            }
        }
        let mut directors = Vec::new();
        let mut workers = Vec::new();
        for entry in listed.into_values() {
            match entry.listing {
                StaffListing::Director { id } => {
                    directors.push(DirectorReport::new(&entry, id));
                },
                StaffListing::Worker { id, parent } => {
                    workers.push(WorkerReport::new(&entry, id, parent));
                },
                StaffListing::Other => {},
            }
        }
        Self {
            schema: CENSUS_SCHEMA,
            machines: machines
                .iter()
                .map(|machine| MachineReport::new(machine.name, machine.state))
                .collect(),
            directors,
            workers,
        }
    }
}

impl MachineReport {
    fn new(name: &str, state: &MachineState) -> Self {
        let (reachable, error) = match state {
            MachineState::Answered(_, hierarchy) => {
                let problems: Vec<_> = [
                    source_problem(&hierarchy.facts.staff),
                    source_problem(&hierarchy.facts.accounts),
                ]
                .into_iter()
                .flatten()
                .collect();
                (
                    true,
                    (!problems.is_empty()).then(|| problems.join(CENSUS_REASON_SEPARATOR)),
                )
            },
            MachineState::Failed(reason) => (false, Some(reason.clone())),
            MachineState::Scanning => (false, Some(CENSUS_UNANSWERED_REASON.to_owned())),
        };
        Self {
            name: name.to_owned(),
            reachable,
            error,
        }
    }
}

/// The status word a tile status cell draws, or null when it draws none.
const fn status(cell: &SessionCells) -> Option<&'static str> {
    match cell.status {
        ReportedSessionState::Reported(state) => Some(state.text()),
        ReportedSessionState::Unavailable => None,
    }
}

impl DirectorReport {
    fn new(entry: &Listed<'_>, id: &str) -> Self {
        let Listed { cell, facts, .. } = entry;
        Self {
            staff_id:            id.to_owned(),
            name:                cell.name.clone(),
            machine:             cell.machine.clone(),
            current_run_id:      match &facts.current_run {
                Observed::Known(StaffRunPointer::Current(run)) => {
                    Some(run.path().display().to_string())
                },
                Observed::Known(StaffRunPointer::NoRun) | Observed::Unknown(_) => None,
            },
            status:              status(cell),
            activity:            cell.activity.clone(),
            activity_started_at: known(&facts.activity_started),
            unknown:             UnknownReport::of([
                reason(&facts.current_run),
                reason(&facts.activity_started),
            ]),
        }
    }
}

impl WorkerReport {
    fn new(entry: &Listed<'_>, id: &str, parent: &str) -> Self {
        let Listed { cell, facts, .. } = entry;
        Self {
            staff_id:            id.to_owned(),
            name:                cell.name.clone(),
            parent_staff_id:     parent.to_owned(),
            machine:             cell.machine.clone(),
            status:              status(cell),
            activity:            cell.activity.clone(),
            activity_started_at: known(&facts.activity_started),
            pass:                match &facts.pass {
                Observed::Known(PassFact::Open(assignment)) => Some(PassReport {
                    role:  assignment.role,
                    phase: assignment.phase.clone(),
                }),
                Observed::Known(PassFact::Closed) | Observed::Unknown(_) => None,
            },
            pane:                match facts.pane {
                Observed::Known(PaneState::Open) => PaneReport::Open,
                Observed::Known(PaneState::Stopped) => PaneReport::Stopped,
                Observed::Known(PaneState::Closed) => PaneReport::Closed,
                Observed::Unknown(_) => PaneReport::Unknown,
            },
            runs:                match &facts.runs {
                Observed::Known(runs) => Some(runs.iter().map(RunReport::new).collect()),
                Observed::Unknown(_) => None,
            },
            unknown:             UnknownReport::of([
                reason(&facts.current_run),
                reason(&facts.pane),
                reason(&facts.runs),
                reason(&facts.activity_started),
                reason(&facts.pass),
            ]),
        }
    }
}

impl RunReport {
    fn new(run: &DetachedRunFact) -> Self {
        Self {
            label:      run.label.clone(),
            started_at: known(&run.started),
            state:      run.state,
            unknown:    UnknownReport::of([reason(&run.started)]),
        }
    }
}

/// Scan once, print the census report on stdout, and exit.
pub(crate) fn print() -> ExitCode {
    startup_log::init();
    let loaded = LoadedConfig::load::<CargoHandler>();
    print_to(
        &loaded,
        &SshRunner::new(),
        probe::scan_local,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )
}

/// A bad configuration exits non-zero; a failed source is in the report.
fn print_to(
    loaded: &LoadedConfig,
    runner: &dyn RemoteRunner,
    scan: impl FnOnce(&Config) -> (Vec<AgentRow>, HierarchySnapshot),
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    if let Some(error) = &loaded.error {
        let _ = writeln!(stderr, "{BINARY_NAME}: {CENSUS_COMMAND}: {error}");
        return ExitCode::FAILURE;
    }
    let config = &loaded.config;
    let remotes = &config.machines.remote;
    let ((rows, hierarchy), accounts, outcomes) = thread::scope(|scope| {
        #[expect(
            clippy::needless_collect,
            reason = "every probe starts before the local scan, so the probes run at the same time"
        )]
        let probes: Vec<_> = remotes
            .iter()
            .map(|host| scope.spawn(move || runner.probe(host)))
            .collect();
        let accounts = scope.spawn(|| AccountsTable::read_once(&config.census.accounts_command));
        let local = scan(config);
        let accounts = accounts
            .join()
            .unwrap_or_else(|failure| panic::resume_unwind(failure));
        let outcomes: Vec<_> = probes
            .into_iter()
            .map(|probe| {
                probe
                    .join()
                    .unwrap_or_else(|failure| panic::resume_unwind(failure))
            })
            .collect();
        (local, accounts, outcomes)
    });
    let mut census = Census::new(local_machine_name());
    census.record_local_accounts(&accounts);
    census.apply(CensusUpdate::Local {
        rows,
        hierarchy,
        stage: ScanStage::Completed,
    });
    let mut answers = RemoteHierarchyAnswers::default();
    for (host, outcome) in remotes.iter().zip(outcomes) {
        census.apply(CensusUpdate::Remote {
            host:  host.clone(),
            state: answers.answer(host, outcome),
        });
    }
    let report = CensusReport::build(&census, remotes, accounts.session_states(), unix_now());
    let written = serde_json::to_string_pretty(&report)
        .map_err(io::Error::from)
        .and_then(|json| writeln!(stdout, "{json}"));
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(stderr, "{BINARY_NAME}: {CENSUS_COMMAND}: {error}");
            ExitCode::FAILURE
        },
    }
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
