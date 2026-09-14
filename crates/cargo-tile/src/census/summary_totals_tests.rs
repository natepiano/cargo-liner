//! Command and summary rows charge each contributing process once.

use std::io::BufRead;
use std::io::BufReader;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
#[cfg(target_os = "linux")]
use std::time::Duration;
use std::time::Instant;

use sysinfo::Pid;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;
use uuid::Uuid;

use crate::census::CargoProcess;
use crate::census::CompilerObservation;
use crate::census::InvocationId;
use crate::census::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::census::process_identity::ProcessIdentity;
#[cfg(target_os = "linux")]
use crate::census::scan::groups_with_cpu_counters_for_test;
use crate::census::scan::groups_with_cpu_for_test;
use crate::census::scan::groups_with_registration_rows_for_test;
use crate::render::summary_cpu_for_test;
use crate::roster::Roster;
use crate::roster::TrackedGroup;

/// Live argv and cwd observations, with ancestry and CPU supplied independently.
struct SummaryTree {
    children: Vec<Child>,
    system:   System,
}

impl SummaryTree {
    fn new() -> Self {
        let mut tree = Self {
            children: Vec::new(),
            system:   System::new(),
        };
        for command in ["port", "build", "test", "check", "clippy", "bench"] {
            tree.children.push(
                Command::new("sh")
                    .args(["-c", "printf 'ready\\n'; read -r release", "cargo", command])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .expect("start a cargo-shaped process"),
            );
            let stdout = tree
                .children
                .last_mut()
                .expect("the spawned process is owned by the tree")
                .stdout
                .take()
                .expect("process readiness pipe");
            let mut ready = String::new();
            BufReader::new(stdout)
                .read_line(&mut ready)
                .expect("read process readiness");
            assert_eq!(ready, "ready\n");
        }
        tree.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&tree.pids().map(Pid::from_u32)),
            false,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_cmd(UpdateKind::Always)
                .with_cwd(UpdateKind::Always),
        );
        assert_eq!(tree.system.processes().len(), tree.children.len());
        tree
    }

    fn pids(&self) -> [u32; 6] { std::array::from_fn(|index| self.children[index].id()) }

    fn roster(&self, shares: &[(u32, Measurement<f32>)]) -> Roster {
        let [driver, first, nested, deep, second, sibling_nested] = self.pids();
        // The direct children are promoted; nested and deep stay in their command view.
        let parents = [
            (first, driver),
            (nested, first),
            (deep, nested),
            (second, driver),
            (sibling_nested, second),
        ];
        let groups = groups_with_cpu_for_test(&self.system, &parents, shares);
        assert_eq!(groups.len(), 1, "the fixture is one invocation tree");
        assert_eq!(groups[0].lead.pid, driver);
        assert_eq!(groups[0].rest.len(), self.children.len() - 1);
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        roster
    }

    fn readings(&self) -> [(u32, Measurement<f32>); 6] {
        let cpu = [128.0, 1.0, 2.0, 4.0, 16.0, 32.0];
        std::array::from_fn(|index| (self.children[index].id(), Measurement::Reading(cpu[index])))
    }

    #[cfg(target_os = "linux")]
    fn roster_with_idle_parent(&self, parent: Measurement<Duration>) -> Roster {
        let [driver, first, nested, deep, second, sibling_nested] = self.pids();
        let parents = [
            (first, driver),
            (nested, first),
            (deep, nested),
            (second, driver),
            (sibling_nested, second),
        ];
        // Thirteen observations model a Cargo parent that accrues no CPU ticks of its own.
        // Native cumulative counters include each busy nested Cargo once.
        let samples: Vec<_> = (0..13u64)
            .map(|scan| {
                self.pids().map(|pid| {
                    let millis = if pid == nested {
                        scan * 1000
                    } else if pid == sibling_nested {
                        scan * 500
                    } else {
                        0
                    };
                    let read = if pid == first {
                        parent
                    } else {
                        Measurement::Reading(Duration::from_millis(millis))
                    };
                    (pid, millis, read)
                })
            })
            .collect();
        let samples: Vec<_> = samples.iter().map(<[_; 6]>::as_slice).collect();
        let groups = groups_with_cpu_counters_for_test(&self.system, &parents, &samples);
        assert_eq!(groups.len(), 1);
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        roster
    }

    fn roster_with_registration(
        &self,
        registration: &CargoProcess,
        omitted_process_pids: &[u32],
    ) -> Roster {
        let [driver, first, nested, deep, second, sibling_nested] = self.pids();
        let parents = [
            (first, driver),
            (nested, first),
            (deep, nested),
            (second, driver),
            (sibling_nested, second),
        ];
        let groups = groups_with_registration_rows_for_test(
            &self.system,
            &parents,
            &self.readings(),
            std::slice::from_ref(registration),
            omitted_process_pids,
        );
        assert_eq!(
            groups.len(),
            1,
            "both row sources share one invocation tree"
        );
        assert_eq!(groups[0].lead.pid, driver);
        assert_eq!(
            groups[0].rest.len(),
            self.children.len() - omitted_process_pids.len(),
            "the injected invocation must survive group assembly"
        );
        let mut roster = Roster::new();
        roster.observe(groups, Instant::now());
        roster
    }

    fn assert_summary(&self, roster: &Roster, first: Measurement<&str>, second: Measurement<&str>) {
        let before = command_cpu(roster);
        let [_, first_pid, _, _, second_pid, _] = self.pids();
        assert_cpu_rows(
            summary_cpu_for_test(roster, &["port".to_owned()]),
            &[(first_pid, first), (second_pid, second)],
        );
        assert_eq!(
            command_cpu(roster),
            before,
            "selecting summary totals must preserve command-row measurements"
        );
    }
}

impl Drop for SummaryTree {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
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
    let roster = tree.roster(&tree.readings());

    tree.assert_summary(
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
    let [driver, first, nested, deep, second, sibling_nested] = tree.pids();
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
    tree.assert_summary(
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
    tree.assert_summary(
        &roster,
        Measurement::Unavailable(MeasurementAbsence::ReadFailed),
        Measurement::Reading("50%"),
    );
    let [_, first, nested, ..] = tree.pids();
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
    let roster = tree.roster(&tree.readings());
    let [driver, first, nested, deep, second, sibling_nested] = tree.pids();

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
        summary_cpu_for_test(&roster, &[]),
        &[(driver, Measurement::Reading("183%"))],
    );
    tree.assert_summary(
        &roster,
        Measurement::Reading("7%"),
        Measurement::Reading("48%"),
    );
}

#[test]
fn an_unavailable_contribution_affects_only_its_promoted_subtree() {
    let tree = SummaryTree::new();
    let [driver, first, nested, deep, second, sibling_nested] = tree.pids();
    for reason in [
        MeasurementAbsence::FirstObservation,
        MeasurementAbsence::ReadFailed,
        MeasurementAbsence::Unproven,
    ] {
        // Cover both depths and both siblings, as well as each promoted row's own bucket.
        for unavailable in [first, nested, deep, second, sibling_nested] {
            let shares = tree.readings().map(|(pid, cpu)| {
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
            tree.assert_summary(&roster, first_total, second_total);

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
    let [driver, ..] = tree.pids();
    for reason in [
        MeasurementAbsence::FirstObservation,
        MeasurementAbsence::ReadFailed,
        MeasurementAbsence::Unproven,
    ] {
        let shares = tree.readings().map(|(pid, cpu)| {
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
        tree.assert_summary(
            &roster,
            Measurement::Reading("7%"),
            Measurement::Reading("48%"),
        );
        assert_cpu_rows(
            summary_cpu_for_test(&roster, &[]),
            &[(driver, Measurement::Unavailable(reason))],
        );
    }
}

#[test]
fn a_missing_nested_attribution_is_unavailable_instead_of_zero() {
    let tree = SummaryTree::new();
    let [driver, _, _, deep, _, _] = tree.pids();
    let shares: Vec<_> = tree
        .readings()
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
        summary_cpu_for_test(&roster, &[]),
        &[(
            driver,
            Measurement::Unavailable(MeasurementAbsence::Unproven),
        )],
    );
    tree.assert_summary(
        &roster,
        Measurement::Unavailable(MeasurementAbsence::Unproven),
        Measurement::Reading("48%"),
    );
}

#[test]
fn a_non_process_contribution_with_a_cpu_bucket_keeps_leads_unavailable() {
    let tree = SummaryTree::new();
    let baseline = tree.roster(&tree.readings());
    let [driver, first, nested, deep, second, sibling_nested] = tree.pids();
    let registration = unproven_row(&baseline, deep);
    let roster = tree.roster_with_registration(&registration, &[deep]);

    tree.assert_summary(
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
        summary_cpu_for_test(&roster, &[]),
        &[(
            driver,
            Measurement::Unavailable(MeasurementAbsence::Unproven),
        )],
    );
    tree.assert_summary(
        &roster,
        Measurement::Unavailable(MeasurementAbsence::Unproven),
        Measurement::Reading("48%"),
    );
}

#[test]
fn a_shared_pid_under_distinct_invocations_cannot_publish_a_double_charged_total() {
    let tree = SummaryTree::new();
    let baseline = tree.roster(&tree.readings());
    let [driver, first, nested, deep, second, sibling_nested] = tree.pids();
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
        summary_cpu_for_test(&roster, &[]),
        &[(driver, Measurement::Reading("183%"))],
    );
    tree.assert_summary(
        &roster,
        Measurement::Unavailable(MeasurementAbsence::Unproven),
        Measurement::Reading("48%"),
    );
}

#[test]
fn an_unproven_group_lead_cannot_use_its_cpu_bucket_for_a_subtree_total() {
    let tree = SummaryTree::new();
    let baseline = tree.roster(&tree.readings());
    let [driver, first, nested, deep, second, sibling_nested] = tree.pids();
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
        summary_cpu_for_test(&roster, &[]),
        &[(
            driver,
            Measurement::Unavailable(MeasurementAbsence::Unproven),
        )],
    );
    tree.assert_summary(
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
    let [driver, ..] = tree.pids();
    for shares in [
        tree.readings().map(|(pid, cpu)| {
            (
                pid,
                if pid == driver {
                    cpu
                } else {
                    Measurement::Reading(0.0)
                },
            )
        }),
        tree.readings()
            .map(|(pid, _)| (pid, Measurement::Reading(0.0))),
    ] {
        let roster = tree.roster(&shares);
        tree.assert_summary(
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
    let shares: Vec<_> = tree
        .pids()
        .into_iter()
        .zip(cpu.map(Measurement::Reading))
        .collect();
    let roster = tree.roster(&shares);
    let [driver, first, nested, deep, second, sibling_nested] = tree.pids();

    // 85.75 and 83.25 round to 86 and 83; summing the row labels yields 85 and 84.
    tree.assert_summary(
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
