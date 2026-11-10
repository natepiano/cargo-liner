use std::collections::HashMap;

use sysinfo::Pid;

#[cfg(test)]
use super::Compiler;
use super::CompilerObservation;
use super::InvocationResourceUse;
#[cfg(test)]
use super::aggregate;
use super::aggregate_compilers;
use super::cpu_label;
use super::sum_resource_use_once_per_pid;
use crate::census::invocation_cpu_accounting::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::constants::SCCACHE_BINARY;

#[test]
fn resource_use_sums_numeric_contributions_before_rounding() {
    let total = InvocationResourceUse::sum([
        InvocationResourceUse {
            cpu:      Measurement::Reading(0.375),
            memory:   Measurement::Reading(100),
            compiler: CompilerObservation::Running(Compiler {
                name:  SCCACHE_BINARY,
                count: 2,
            }),
        },
        InvocationResourceUse {
            cpu:      Measurement::Reading(0.375),
            memory:   Measurement::Reading(200),
            compiler: CompilerObservation::Running(Compiler {
                name:  SCCACHE_BINARY,
                count: 3,
            }),
        },
    ]);
    assert_eq!(total.cpu, Measurement::Reading(0.75));
    assert_eq!(
        total.cpu.map(cpu_label),
        Measurement::Reading("1%".to_owned())
    );
    assert_eq!(total.memory, Measurement::Reading(300));
    assert_eq!(
        total.compiler,
        CompilerObservation::Running(Compiler {
            name:  SCCACHE_BINARY,
            count: 5,
        })
    );
}

#[test]
fn shared_pid_readings_replace_only_their_missing_measurements() {
    let first = InvocationResourceUse {
        cpu:      Measurement::Reading(10.0),
        memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
        compiler: CompilerObservation::Unknown,
    };
    let second = InvocationResourceUse {
        cpu:      Measurement::Unavailable(MeasurementAbsence::ReadFailed),
        memory:   Measurement::Reading(200),
        compiler: CompilerObservation::Running(Compiler {
            name:  SCCACHE_BINARY,
            count: 2,
        }),
    };
    for readings in [
        [first.clone(), second.clone()],
        [second.clone(), first.clone()],
    ] {
        let total =
            sum_resource_use_once_per_pid(readings.map(|reading| (Pid::from_u32(10), reading)));
        assert_eq!(total.cpu, first.cpu);
        assert_eq!(total.memory, second.memory);
        assert_eq!(total.compiler, second.compiler);
    }
    let total =
        sum_resource_use_once_per_pid([(Pid::from_u32(10), first), (Pid::from_u32(20), second)]);
    assert_eq!(
        total.cpu,
        Measurement::Unavailable(MeasurementAbsence::ReadFailed)
    );
    assert_eq!(
        total.memory,
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    assert_eq!(total.compiler, CompilerObservation::Unknown);
}

#[test]
fn resource_use_sums_keep_each_unavailable_measurement() {
    let total = InvocationResourceUse::sum([
        InvocationResourceUse {
            cpu:      Measurement::Reading(10.0),
            memory:   Measurement::Reading(100),
            compiler: CompilerObservation::None,
        },
        InvocationResourceUse {
            cpu:      Measurement::Unavailable(MeasurementAbsence::FirstObservation),
            memory:   Measurement::Unavailable(MeasurementAbsence::ReadFailed),
            compiler: CompilerObservation::Unknown,
        },
    ]);
    assert_eq!(
        total.cpu,
        Measurement::Unavailable(MeasurementAbsence::FirstObservation)
    );
    assert_eq!(
        total.memory,
        Measurement::Unavailable(MeasurementAbsence::ReadFailed)
    );
    assert_eq!(total.compiler, CompilerObservation::Unknown);
}

/// A share the platform could only report as a rounding artefact
/// still has to read as idle rather than as a negative percent.
#[test]
fn a_share_below_nought_reads_as_nought() {
    assert_eq!(cpu_label(-0.4), "0%");
}

/// Every group member contributes explicitly, including measured zero.
#[test]
fn a_group_adds_up_the_shares_of_everything_under_it() {
    let shares = HashMap::from([
        (Pid::from(1), Measurement::Reading(90.4)),
        (Pid::from(2), Measurement::Reading(300.2)),
        (Pid::from(3), Measurement::Reading(0.0)),
    ]);
    let members = [1, 2, 3].into_iter().map(Pid::from);

    assert_eq!(
        aggregate(&shares, members).map(cpu_label).to_string(),
        "391%"
    );
}

#[test]
fn a_group_with_a_missing_share_is_unavailable() {
    assert_eq!(
        aggregate(
            &HashMap::<Pid, Measurement<f32>>::new(),
            std::iter::once(Pid::from(1)),
        ),
        Measurement::Unavailable(MeasurementAbsence::Unproven),
    );
}

#[test]
fn a_group_of_measured_zero_shares_reads_as_idle() {
    let pid = Pid::from(1);
    let shares = HashMap::from([(pid, Measurement::Reading(0.0))]);
    assert_eq!(
        aggregate(&shares, std::iter::once(pid))
            .map(cpu_label)
            .to_string(),
        "0%",
    );
}

#[test]
fn an_unknown_cpu_contributor_makes_the_group_total_unknown() {
    for reason in [
        MeasurementAbsence::FirstObservation,
        MeasurementAbsence::ReadFailed,
        MeasurementAbsence::Unproven,
    ] {
        let shares = HashMap::from([
            (Pid::from(1), Measurement::Reading(100.0)),
            (Pid::from(2), Measurement::Unavailable(reason)),
        ]);
        for members in [[Pid::from(1), Pid::from(2)], [Pid::from(2), Pid::from(1)]] {
            assert_eq!(
                aggregate(&shares, members.into_iter()),
                Measurement::Unavailable(reason)
            );
        }
    }
}

#[test]
fn compiler_totals_preserve_measured_absence_and_driver_priority() {
    let first = Pid::from(1);
    let second = Pid::from(2);
    let third = Pid::from(3);
    let mut counts = HashMap::from([
        (first, CompilerObservation::None),
        (second, CompilerObservation::None),
    ]);
    assert_eq!(
        aggregate_compilers(&counts, [first, second].into_iter()),
        CompilerObservation::None
    );
    counts.insert(
        first,
        CompilerObservation::Running(Compiler {
            name:  "rustc",
            count: 9,
        }),
    );
    counts.insert(
        second,
        CompilerObservation::Running(Compiler {
            name:  "sccache",
            count: 2,
        }),
    );
    counts.insert(
        third,
        CompilerObservation::Running(Compiler {
            name:  "sccache",
            count: 3,
        }),
    );
    assert_eq!(
        aggregate_compilers(&counts, [first, second, third].into_iter()),
        CompilerObservation::Running(Compiler {
            name:  "sccache",
            count: 5,
        })
    );
}

#[test]
fn unknown_compiler_contributors_invalidate_the_group_tally() {
    let first = Pid::from(1);
    let second = Pid::from(2);
    for unknown in [
        HashMap::new(),
        HashMap::from([(second, CompilerObservation::Unknown)]),
    ] {
        let mut counts = unknown;
        counts.insert(
            first,
            CompilerObservation::Running(Compiler {
                name:  "rustc",
                count: 2,
            }),
        );
        assert_eq!(
            aggregate_compilers(&counts, [first, second].into_iter()),
            CompilerObservation::Unknown
        );
    }
}

/// The scale is `top`'s, so a build across several cores reads past
/// 100% rather than being folded back into a share of the machine.
#[test]
fn a_cpu_share_reads_as_a_whole_number_of_percent() {
    assert_eq!(cpu_label(0.0), "0%");
    assert_eq!(cpu_label(12.4), "12%");
    assert_eq!(cpu_label(12.6), "13%");
    assert_eq!(cpu_label(783.2), "783%");
}
