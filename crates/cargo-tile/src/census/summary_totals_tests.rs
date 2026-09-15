//! Command and summary rows charge each contributing process once.

use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::time::Duration;
use std::time::Instant;

use sysinfo::Pid;
use uuid::Uuid;

#[cfg(target_os = "linux")]
use super::CargoGroup;
use super::CargoProcess;
use super::CompilerObservation;
use super::InvocationId;
use super::Measurement;
use super::invocation_cpu_accounting::MeasurementAbsence;
use super::process_identity::ProcessIdentity;
use super::scan::CensusSequence;
use super::scan::ProcessField;
use super::scan::ProcessObservation;
use super::scan::ProcessObservations;
use crate::render;
use crate::roster::Roster;
use crate::roster::TrackedGroup;

/// Independent argv, ancestry, and CPU observations for one invocation family.
struct SummaryTree {
    argv: [Vec<OsString>; 6],
}

impl SummaryTree {
    fn new() -> Self {
        Self {
            argv: ["port", "build", "test", "check", "clippy", "bench"]
                .map(|command| vec!["cargo".into(), command.into()]),
        }
    }

    const fn pids() -> [u32; 6] { [100, 101, 102, 103, 104, 105] }

    fn observations(&self, parents: &[(u32, u32)], omitted: &[u32]) -> ProcessObservations<'_> {
        ProcessObservations::new(Self::pids().into_iter().zip(&self.argv).map(|(pid, argv)| {
            let mut record = ProcessObservation::cargo(pid, argv);
            record.parent = parents
                .iter()
                .find(|(child, _)| *child == pid)
                .map(|(_, parent)| Pid::from_u32(*parent))
                .into();
            if omitted.contains(&pid) {
                record.argv = ProcessField::Unavailable;
            }
            record
        }))
    }

    fn roster(&self, shares: &[(u32, Measurement<f32>)]) -> Roster {
        let [driver, first, nested, deep, second, sibling_nested] = Self::pids();
        // The direct children are promoted; nested and deep stay in their command view.
        let parents = [
            (first, driver),
            (nested, first),
            (deep, nested),
            (second, driver),
            (sibling_nested, second),
        ];
        let groups =
            CensusSequence::default().sample_cpu(&self.observations(&parents, &[]), shares);
        assert_eq!(groups.len(), 1, "the fixture is one invocation tree");
        assert_eq!(groups[0].lead.pid, driver);
        assert_eq!(groups[0].rest.len(), self.argv.len() - 1);
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        roster
    }

    fn readings() -> [(u32, Measurement<f32>); 6] {
        let cpu = [128.0, 1.0, 2.0, 4.0, 16.0, 32.0];
        std::array::from_fn(|index| (Self::pids()[index], Measurement::Reading(cpu[index])))
    }

    #[cfg(target_os = "linux")]
    fn roster_with_idle_parent(&self, parent: Measurement<Duration>) -> Roster {
        let [driver, first, nested, deep, second, sibling_nested] = Self::pids();
        let parents = [
            (first, driver),
            (nested, first),
            (deep, nested),
            (second, driver),
            (sibling_nested, second),
        ];
        // Thirteen observations model a Cargo parent that accrues no CPU ticks of its own.
        // Native cumulative counters include each busy nested Cargo once.
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        let samples: Vec<_> = (0..13u64)
            .map(|scan| {
                let records = Self::pids().into_iter().zip(&self.argv).map(|(pid, argv)| {
                    let mut record = ProcessObservation::cargo(pid, argv);
                    record.parent = parents
                        .iter()
                        .find(|(child, _)| *child == pid)
                        .map(|(_, parent)| Pid::from_u32(*parent))
                        .into();
                    record.accumulated = if pid == nested {
                        scan * 1000
                    } else if pid == sibling_nested {
                        scan * 500
                    } else {
                        0
                    };
                    record.native_cpu.set(if pid == first {
                        parent
                    } else {
                        Measurement::Reading(Duration::from_millis(record.accumulated))
                    });
                    record
                });
                let groups = sequence.sample_counters(
                    &mut ProcessObservations::new(records),
                    now + Duration::from_secs(scan),
                );
                Self::assert_idle_parent_sample(&groups, parent, scan);
                groups
            })
            .collect();
        let groups = samples.last().expect("thirteen observations").clone();
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        roster
    }

    #[cfg(target_os = "linux")]
    fn assert_idle_parent_sample(groups: &[CargoGroup], parent: Measurement<Duration>, scan: u64) {
        let [driver, first, nested, deep, second, sibling_nested] = Self::pids();
        assert_eq!(groups.len(), 1, "sample {scan}");
        assert_eq!(groups[0].lead.pid, driver, "sample {scan}");
        assert_eq!(groups[0].rest.len(), 5, "sample {scan}");
        for (pid, rate) in [
            (nested, "100%"),
            (sibling_nested, "50%"),
            (deep, "0%"),
            (second, "0%"),
        ] {
            let row = groups[0]
                .rest
                .iter()
                .find(|row| row.pid == pid)
                .expect("every synthetic child remains present");
            let expected = if scan == 0 {
                Measurement::Unavailable(MeasurementAbsence::FirstObservation)
            } else {
                Measurement::Reading(rate.to_owned())
            };
            assert_eq!(row.cpu, expected, "sample {scan}, pid {pid}");
        }
        let parent_cpu = match parent {
            Measurement::Unavailable(reason) => Measurement::Unavailable(reason),
            Measurement::Reading(_) if scan == 0 => {
                Measurement::Unavailable(MeasurementAbsence::FirstObservation)
            },
            Measurement::Reading(_) => Measurement::Reading("0%"),
        };
        let group_cpu = match parent {
            _ if scan == 0 => Measurement::Unavailable(MeasurementAbsence::FirstObservation),
            Measurement::Unavailable(reason) => Measurement::Unavailable(reason),
            Measurement::Reading(_) => Measurement::Reading("150%"),
        };
        let first_total = match parent_cpu {
            Measurement::Unavailable(reason) => Measurement::Unavailable(reason),
            Measurement::Reading(_) => Measurement::Reading("100%"),
        };
        let second_total = if scan == 0 {
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        } else {
            Measurement::Reading("50%")
        };
        let mut roster = Roster::new();
        roster.observe(groups.to_vec(), Instant::now());
        let rows = command_cpu(&roster);
        assert!(
            rows.contains(&(first, parent_cpu.map(str::to_owned))),
            "sample {scan}: {rows:?}"
        );
        assert!(
            rows.contains(&(driver, group_cpu.map(str::to_owned))),
            "sample {scan}: {rows:?}"
        );
        assert_cpu_rows(
            render::summary_cpu_for_test(&roster, &[]),
            &[(driver, group_cpu)],
        );
        Self::assert_summary(&roster, first_total, second_total);
    }

    fn roster_with_registration(
        &self,
        registration: &CargoProcess,
        omitted_process_pids: &[u32],
    ) -> Roster {
        let [driver, first, nested, deep, second, sibling_nested] = Self::pids();
        let parents = [
            (first, driver),
            (nested, first),
            (deep, nested),
            (second, driver),
            (sibling_nested, second),
        ];
        let mut sequence = CensusSequence::default();
        sequence.registration_rows = vec![registration.clone()];
        let groups = sequence.sample_cpu(
            &self.observations(&parents, omitted_process_pids),
            &Self::readings(),
        );
        assert_eq!(
            groups.len(),
            1,
            "both row sources share one invocation tree"
        );
        assert_eq!(groups[0].lead.pid, driver);
        assert_eq!(
            groups[0].rest.len(),
            self.argv.len() - omitted_process_pids.len(),
            "the injected invocation must survive group assembly"
        );
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        roster
    }

    fn assert_summary(roster: &Roster, first: Measurement<&str>, second: Measurement<&str>) {
        let before = command_cpu(roster);
        let [_, first_pid, _, _, second_pid, _] = Self::pids();
        assert_cpu_rows(
            render::summary_cpu_for_test(roster, &["port".to_owned()]),
            &[(first_pid, first), (second_pid, second)],
        );
        assert_eq!(
            command_cpu(roster),
            before,
            "selecting summary totals must preserve command-row measurements"
        );
    }
}

fn command_cpu(roster: &Roster) -> Vec<(u32, Measurement<String>)> {
    roster
        .groups()
        .iter()
        .flat_map(TrackedGroup::rows)
        .map(|row| (row.process.pid, row.process.cpu.clone()))
        .collect()
}

fn unproven_row(roster: &Roster, pid: u32) -> CargoProcess {
    let mut process = roster
        .groups()
        .iter()
        .flat_map(TrackedGroup::rows)
        .find(|row| row.process.pid == pid)
        .expect("the process row supplies display fields for the injected row")
        .process
        .clone();
    // This identity is injected after process_rows is collected; it carries no proof.
    process.invocation_id = InvocationId::Process(ProcessIdentity::Unavailable {
        pid,
        observation: Uuid::now_v7(),
    });
    process.cpu = Measurement::Unavailable(MeasurementAbsence::Unproven);
    process.subtree_cpu = Measurement::Unavailable(MeasurementAbsence::Unproven);
    process.compiler = CompilerObservation::Unknown;
    process.managed = Measurement::Unavailable(MeasurementAbsence::Unproven);
    process
}

fn assert_cpu_rows(
    mut actual: Vec<(u32, Measurement<String>)>,
    expected: &[(u32, Measurement<&str>)],
) {
    let mut expected: Vec<_> = expected
        .iter()
        .map(|&(pid, cpu)| (pid, cpu.map(str::to_owned)))
        .collect();
    actual.sort_by_key(|(pid, _)| *pid);
    expected.sort_by_key(|(pid, _)| *pid);
    assert_eq!(actual, expected);
}

#[test]
fn promoted_totals_count_each_descendant_once_and_exclude_driver_and_sibling() {
    let tree = SummaryTree::new();
    let roster = tree.roster(&SummaryTree::readings());

    SummaryTree::assert_summary(
        &roster,
        Measurement::Reading("7%"),
        Measurement::Reading("48%"),
    );
}

#[cfg(target_os = "linux")]
#[test]
fn zero_own_ticks_across_thirteen_samples_keep_promoted_descendant_cpu_numeric() {
    let tree = SummaryTree::new();
    let roster = tree.roster_with_idle_parent(Measurement::Reading(Duration::ZERO));
    let [driver, first, nested, deep, second, sibling_nested] = SummaryTree::pids();
    assert_cpu_rows(
        command_cpu(&roster),
        &[
            (driver, Measurement::Reading("150%")),
            (first, Measurement::Reading("0%")),
            (nested, Measurement::Reading("100%")),
            (deep, Measurement::Reading("0%")),
            (second, Measurement::Reading("0%")),
            (sibling_nested, Measurement::Reading("50%")),
        ],
    );
    SummaryTree::assert_summary(
        &roster,
        Measurement::Reading("100%"),
        Measurement::Reading("50%"),
    );
}

#[cfg(target_os = "linux")]
#[test]
fn unreadable_parent_counter_keeps_its_promoted_total_unavailable_despite_busy_leaf() {
    let tree = SummaryTree::new();
    let unavailable = Measurement::Unavailable(MeasurementAbsence::ReadFailed);
    let roster = tree.roster_with_idle_parent(unavailable);
    SummaryTree::assert_summary(
        &roster,
        Measurement::Unavailable(MeasurementAbsence::ReadFailed),
        Measurement::Reading("50%"),
    );
    let [_, first, nested, ..] = SummaryTree::pids();
    let rows = command_cpu(&roster);
    assert!(rows.contains(&(
        first,
        Measurement::Unavailable(MeasurementAbsence::ReadFailed)
    )));
    assert!(rows.contains(&(nested, Measurement::Reading("100%".to_owned()))));
}

#[test]
fn command_rows_keep_their_own_attributions_and_the_lead_keeps_its_group_total() {
    let tree = SummaryTree::new();
    let roster = tree.roster(&SummaryTree::readings());
    let [driver, first, nested, deep, second, sibling_nested] = SummaryTree::pids();

    assert_cpu_rows(
        command_cpu(&roster),
        &[
            (driver, Measurement::Reading("183%")),
            (first, Measurement::Reading("1%")),
            (nested, Measurement::Reading("2%")),
            (deep, Measurement::Reading("4%")),
            (second, Measurement::Reading("16%")),
            (sibling_nested, Measurement::Reading("32%")),
        ],
    );
    assert_cpu_rows(
        render::summary_cpu_for_test(&roster, &[]),
        &[(driver, Measurement::Reading("183%"))],
    );
    SummaryTree::assert_summary(
        &roster,
        Measurement::Reading("7%"),
        Measurement::Reading("48%"),
    );
}

#[test]
fn an_unavailable_contribution_affects_only_its_promoted_subtree() {
    let tree = SummaryTree::new();
    let [driver, first, nested, deep, second, sibling_nested] = SummaryTree::pids();
    for reason in [
        MeasurementAbsence::FirstObservation,
        MeasurementAbsence::ReadFailed,
        MeasurementAbsence::Unproven,
    ] {
        // Cover both depths and both siblings, as well as each promoted row's own bucket.
        for unavailable in [first, nested, deep, second, sibling_nested] {
            let shares = SummaryTree::readings().map(|(pid, cpu)| {
                (
                    pid,
                    if pid == unavailable {
                        Measurement::Unavailable(reason)
                    } else {
                        cpu
                    },
                )
            });
            let roster = tree.roster(&shares);
            let first_total = if [first, nested, deep].contains(&unavailable) {
                Measurement::Unavailable(reason)
            } else {
                Measurement::Reading("7%")
            };
            let second_total = if [second, sibling_nested].contains(&unavailable) {
                Measurement::Unavailable(reason)
            } else {
                Measurement::Reading("48%")
            };
            SummaryTree::assert_summary(&roster, first_total, second_total);

            let expected = [
                (driver, Measurement::Unavailable(reason)),
                (first, Measurement::Reading("1%")),
                (nested, Measurement::Reading("2%")),
                (deep, Measurement::Reading("4%")),
                (second, Measurement::Reading("16%")),
                (sibling_nested, Measurement::Reading("32%")),
            ]
            .map(|(pid, cpu)| {
                (
                    pid,
                    if pid == unavailable {
                        Measurement::Unavailable(reason)
                    } else {
                        cpu
                    },
                )
            });
            assert_cpu_rows(command_cpu(&roster), &expected);
        }
    }
}

#[test]
fn an_unavailable_hidden_driver_does_not_replace_known_promoted_totals() {
    let tree = SummaryTree::new();
    let [driver, ..] = SummaryTree::pids();
    for reason in [
        MeasurementAbsence::FirstObservation,
        MeasurementAbsence::ReadFailed,
        MeasurementAbsence::Unproven,
    ] {
        let shares = SummaryTree::readings().map(|(pid, cpu)| {
            (
                pid,
                if pid == driver {
                    Measurement::Unavailable(reason)
                } else {
                    cpu
                },
            )
        });
        let roster = tree.roster(&shares);
        SummaryTree::assert_summary(
            &roster,
            Measurement::Reading("7%"),
            Measurement::Reading("48%"),
        );
        assert_cpu_rows(
            render::summary_cpu_for_test(&roster, &[]),
            &[(driver, Measurement::Unavailable(reason))],
        );
    }
}

#[test]
fn a_missing_nested_attribution_is_unavailable_instead_of_zero() {
    let tree = SummaryTree::new();
    let [driver, _, _, deep, _, _] = SummaryTree::pids();
    let shares: Vec<_> = SummaryTree::readings()
        .into_iter()
        .filter(|&(pid, _)| pid != deep)
        .collect();
    let roster = tree.roster(&shares);

    let lead = &roster.groups()[0].lead.process;
    assert_eq!(
        lead.cpu,
        Measurement::Unavailable(MeasurementAbsence::Unproven),
        "the command lead cannot replace a missing member contribution with zero"
    );
    assert_cpu_rows(
        render::summary_cpu_for_test(&roster, &[]),
        &[(
            driver,
            Measurement::Unavailable(MeasurementAbsence::Unproven),
        )],
    );
    SummaryTree::assert_summary(
        &roster,
        Measurement::Unavailable(MeasurementAbsence::Unproven),
        Measurement::Reading("48%"),
    );
}

#[test]
fn a_non_process_contribution_with_a_cpu_bucket_keeps_leads_unavailable() {
    let tree = SummaryTree::new();
    let baseline = tree.roster(&SummaryTree::readings());
    let [driver, first, nested, deep, second, sibling_nested] = SummaryTree::pids();
    let registration = unproven_row(&baseline, deep);
    let roster = tree.roster_with_registration(&registration, &[deep]);

    SummaryTree::assert_summary(
        &baseline,
        Measurement::Reading("7%"),
        Measurement::Reading("48%"),
    );
    let injected = roster
        .groups()
        .iter()
        .flat_map(TrackedGroup::rows)
        .find(|row| row.process.invocation_id == registration.invocation_id)
        .expect("the unproven contribution remains in the assembled subtree");
    assert_eq!(injected.process.cpu, registration.cpu);
    assert_eq!(injected.process.compiler, registration.compiler);
    assert_cpu_rows(
        command_cpu(&roster),
        &[
            (
                driver,
                Measurement::Unavailable(MeasurementAbsence::Unproven),
            ),
            (first, Measurement::Reading("1%")),
            (nested, Measurement::Reading("2%")),
            (deep, Measurement::Unavailable(MeasurementAbsence::Unproven)),
            (second, Measurement::Reading("16%")),
            (sibling_nested, Measurement::Reading("32%")),
        ],
    );
    assert_cpu_rows(
        render::summary_cpu_for_test(&roster, &[]),
        &[(
            driver,
            Measurement::Unavailable(MeasurementAbsence::Unproven),
        )],
    );
    SummaryTree::assert_summary(
        &roster,
        Measurement::Unavailable(MeasurementAbsence::Unproven),
        Measurement::Reading("48%"),
    );
}

#[test]
fn a_shared_pid_under_distinct_invocations_cannot_publish_a_double_charged_total() {
    let tree = SummaryTree::new();
    let baseline = tree.roster(&SummaryTree::readings());
    let [driver, first, nested, deep, second, sibling_nested] = SummaryTree::pids();
    let registration = unproven_row(&baseline, deep);
    let roster = tree.roster_with_registration(&registration, &[]);
    let shared: Vec<_> = roster
        .groups()
        .iter()
        .flat_map(TrackedGroup::rows)
        .filter(|row| row.process.pid == deep)
        .collect();

    assert_eq!(
        shared.len(),
        2,
        "both invocation identities must survive assembly"
    );
    assert_ne!(
        shared[0].process.invocation_id,
        shared[1].process.invocation_id
    );
    assert_eq!(shared[0].process.parent, shared[1].process.parent);
    let injected = shared
        .iter()
        .find(|row| row.process.invocation_id == registration.invocation_id)
        .expect("the unproven row retains its distinct invocation identity");
    assert_eq!(injected.process.cpu, registration.cpu);
    assert_eq!(injected.process.compiler, registration.compiler);
    let process_rows = roster
        .groups()
        .iter()
        .flat_map(TrackedGroup::rows)
        .filter(|row| row.process.invocation_id != registration.invocation_id)
        .map(|row| (row.process.pid, row.process.cpu.clone()))
        .collect();
    // The second invocation identity must not charge the same process twice.
    assert_cpu_rows(
        process_rows,
        &[
            (driver, Measurement::Reading("183%")),
            (first, Measurement::Reading("1%")),
            (nested, Measurement::Reading("2%")),
            (deep, Measurement::Reading("4%")),
            (second, Measurement::Reading("16%")),
            (sibling_nested, Measurement::Reading("32%")),
        ],
    );
    assert_cpu_rows(
        render::summary_cpu_for_test(&roster, &[]),
        &[(driver, Measurement::Reading("183%"))],
    );
    SummaryTree::assert_summary(
        &roster,
        Measurement::Unavailable(MeasurementAbsence::Unproven),
        Measurement::Reading("48%"),
    );
}

#[test]
fn an_unproven_group_lead_cannot_use_its_cpu_bucket_for_a_subtree_total() {
    let tree = SummaryTree::new();
    let baseline = tree.roster(&SummaryTree::readings());
    let [driver, first, nested, deep, second, sibling_nested] = SummaryTree::pids();
    let registration = unproven_row(&baseline, driver);
    let roster = tree.roster_with_registration(&registration, &[driver]);
    let lead = &roster.groups()[0].lead.process;

    assert_eq!(lead.invocation_id, registration.invocation_id);
    assert_eq!(lead.cpu, registration.cpu);
    assert_eq!(lead.compiler, registration.compiler);
    assert_cpu_rows(
        command_cpu(&roster),
        &[
            (
                driver,
                Measurement::Unavailable(MeasurementAbsence::Unproven),
            ),
            (first, Measurement::Reading("1%")),
            (nested, Measurement::Reading("2%")),
            (deep, Measurement::Reading("4%")),
            (second, Measurement::Reading("16%")),
            (sibling_nested, Measurement::Reading("32%")),
        ],
    );
    assert_cpu_rows(
        render::summary_cpu_for_test(&roster, &[]),
        &[(
            driver,
            Measurement::Unavailable(MeasurementAbsence::Unproven),
        )],
    );
    SummaryTree::assert_summary(
        &roster,
        Measurement::Reading("7%"),
        Measurement::Reading("48%"),
    );
    assert_eq!(
        lead.subtree_cpu,
        Measurement::Unavailable(MeasurementAbsence::Unproven),
        "an unproven lead cannot seed a numeric subtree total from its pid bucket"
    );
}

#[test]
fn measured_zero_remains_available_in_each_promoted_subtree() {
    let tree = SummaryTree::new();
    let [driver, ..] = SummaryTree::pids();
    for shares in [
        SummaryTree::readings().map(|(pid, cpu)| {
            (
                pid,
                if pid == driver {
                    cpu
                } else {
                    Measurement::Reading(0.0)
                },
            )
        }),
        SummaryTree::readings().map(|(pid, _)| (pid, Measurement::Reading(0.0))),
    ] {
        let roster = tree.roster(&shares);
        SummaryTree::assert_summary(
            &roster,
            Measurement::Reading("0%"),
            Measurement::Reading("0%"),
        );
    }
}

#[test]
fn subtree_totals_round_once_after_summing_raw_attributions() {
    let tree = SummaryTree::new();
    let cpu = [101.25, 11.25, 31.25, 43.25, 23.75, 59.5];
    let shares: Vec<_> = SummaryTree::pids()
        .into_iter()
        .zip(cpu.map(Measurement::Reading))
        .collect();
    let roster = tree.roster(&shares);
    let [driver, first, nested, deep, second, sibling_nested] = SummaryTree::pids();

    // 85.75 and 83.25 round to 86 and 83; summing the row labels yields 85 and 84.
    SummaryTree::assert_summary(
        &roster,
        Measurement::Reading("86%"),
        Measurement::Reading("83%"),
    );
    assert_cpu_rows(
        command_cpu(&roster),
        &[
            (driver, Measurement::Reading("270%")),
            (first, Measurement::Reading("11%")),
            (nested, Measurement::Reading("31%")),
            (deep, Measurement::Reading("43%")),
            (second, Measurement::Reading("24%")),
            (sibling_nested, Measurement::Reading("60%")),
        ],
    );
}
