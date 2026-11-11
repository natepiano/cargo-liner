//! Disjoint invocation resource use and group totals.

#[cfg(test)]
#[path = "resource_use_tests.rs"]
mod resource_use_tests;

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::ops::Add;

use sysinfo::Pid;

use crate::census::invocation_cpu_accounting::InvocationMeasurements;
use crate::census::invocation_cpu_accounting::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::constants::COMPILER_PROCESS_NAMES;

/// Compiler absence is an observed idle state; an unknown observation is separate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CompilerObservation {
    /// The scanner cannot establish which compilers are running.
    Unknown,
    /// The scanner observed no compiler running for this invocation.
    None,
    /// The scanner observed this driver and count.
    Running(Compiler),
}

/// Disjoint invocation work: its process and descendants assigned to no other invocation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InvocationResourceUse {
    /// Core percentage remains numeric until a table formats its final contributors.
    pub(crate) cpu:      Measurement<f32>,
    /// Resident bytes include only this invocation's attributed processes.
    pub(crate) memory:   Measurement<u64>,
    /// Compiler counts retain unavailable observations and driver priority.
    pub(crate) compiler: CompilerObservation,
}

impl InvocationResourceUse {
    /// Missing evidence prevents publishing a partial contribution.
    pub(super) const fn unavailable(reason: MeasurementAbsence) -> Self {
        Self {
            cpu:      Measurement::Unavailable(reason),
            memory:   Measurement::Unavailable(reason),
            compiler: CompilerObservation::Unknown,
        }
    }

    /// Numeric contributors are formatted only after the table selects its rows.
    pub(crate) fn sum(contributions: impl IntoIterator<Item = Self>) -> Self {
        let mut total = Self {
            cpu:      Measurement::Reading(0.0),
            memory:   Measurement::Reading(0),
            compiler: CompilerObservation::None,
        };
        for contribution in contributions {
            total.cpu = total.cpu + contribution.cpu;
            total.memory = total.memory + contribution.memory;
            total.compiler = sum_compilers([total.compiler, contribution.compiler]);
        }
        total
    }

    /// Another invocation of the same pid can supply a missing observation.
    fn retain_readings(&mut self, other: Self) {
        if matches!(self.cpu, Measurement::Unavailable(_))
            && matches!(other.cpu, Measurement::Reading(_))
        {
            self.cpu = other.cpu;
        }
        if matches!(self.memory, Measurement::Unavailable(_))
            && matches!(other.memory, Measurement::Reading(_))
        {
            self.memory = other.memory;
        }
        if self.compiler == CompilerObservation::Unknown
            && other.compiler != CompilerObservation::Unknown
        {
            self.compiler = other.compiler;
        }
    }

    /// Each attributed bucket is disjoint from every nested cargo invocation.
    pub(super) fn observed(attributed: &InvocationMeasurements, pid: Pid) -> Self {
        Self {
            cpu:      aggregate(&attributed.cpu, std::iter::once(pid)),
            memory:   aggregate(&attributed.memory, std::iter::once(pid)),
            compiler: attributed
                .compilers
                .get(&pid)
                .cloned()
                .unwrap_or(CompilerObservation::Unknown),
        }
    }
}

/// Each pid contributes once, using any reading supplied by its invocations.
pub(crate) fn sum_resource_use_once_per_pid(
    contributions: impl IntoIterator<Item = (Pid, InvocationResourceUse)>,
) -> InvocationResourceUse {
    let mut indices: HashMap<Pid, usize> = HashMap::new();
    let mut resources: Vec<InvocationResourceUse> = Vec::new();
    for (pid, contribution) in contributions {
        match indices.entry(pid) {
            Entry::Occupied(index) => {
                resources[*index.get()].retain_readings(contribution);
            },
            Entry::Vacant(index) => {
                index.insert(resources.len());
                resources.push(contribution);
            },
        }
    }
    InvocationResourceUse::sum(resources)
}

/// The compiler driver an invocation is running, and how many at once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Compiler {
    /// Driver name, one of [`COMPILER_PROCESS_NAMES`].
    pub(crate) name:  &'static str,
    /// How many of it are running under this cargo invocation.
    pub(crate) count: usize,
}

/// One compiler tally across a whole group: the highest-priority driver
/// any member is running, totalled over all of them.
#[cfg(test)]
fn aggregate_compilers(
    counts: &HashMap<Pid, CompilerObservation>,
    members: impl Iterator<Item = Pid>,
) -> CompilerObservation {
    sum_compilers(members.map(|pid| {
        counts
            .get(&pid)
            .cloned()
            .unwrap_or(CompilerObservation::Unknown)
    }))
}

/// Keep the highest-priority observed driver without publishing a partial count.
fn sum_compilers(counts: impl IntoIterator<Item = CompilerObservation>) -> CompilerObservation {
    let mut running = Vec::new();
    for observation in counts {
        match observation {
            CompilerObservation::Unknown => return CompilerObservation::Unknown,
            CompilerObservation::None => {},
            CompilerObservation::Running(compiler) => running.push(compiler),
        }
    }
    let Some(name) = COMPILER_PROCESS_NAMES
        .iter()
        .find(|driver| running.iter().any(|compiler| compiler.name == **driver))
    else {
        return CompilerObservation::None;
    };
    CompilerObservation::Running(Compiler {
        name,
        count: running
            .iter()
            .filter(|compiler| compiler.name == *name)
            .map(|compiler| compiler.count)
            .sum(),
    })
}

/// One total across a whole group; an unavailable or missing member
/// prevents publishing a partial total as a measured value.
pub(crate) fn aggregate<T: Copy + Default + Add<Output = T>>(
    measurements: &HashMap<Pid, Measurement<T>>,
    members: impl Iterator<Item = Pid>,
) -> Measurement<T> {
    let mut seen = HashSet::new();
    members.filter(|pid| seen.insert(*pid)).fold(
        Measurement::Reading(T::default()),
        |total, pid| {
            total
                + measurements
                    .get(&pid)
                    .copied()
                    .unwrap_or(Measurement::Unavailable(MeasurementAbsence::Unproven))
        },
    )
}

/// A CPU share as the whole-number percent the table carries.
///
/// Rounded rather than truncated, and never below nought: a quarter of a
/// second of sampling has no meaningful resolution under one percent,
/// and a column of decimals costs width the command line wants.
pub(crate) fn cpu_label(cpu: f32) -> String {
    let percent = cpu.max(0.0);
    format!("{percent:.0}%")
}
