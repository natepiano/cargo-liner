//! Resident-memory attribution and row aggregation scenarios.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::time::Duration;
use std::time::Instant;

use sysinfo::Pid;

use super::CargoGroup;
use super::CensusCadence;
use super::InvocationId;
use super::Measurement;
use super::invocation_cpu_accounting::InvocationCpuAccounting;
use super::invocation_cpu_accounting::MeasurementAbsence;
use super::invocation_cpu_accounting::MemoryReport;
use super::scan::CensusSequence;
use super::scan::ProcessField;
use super::scan::ProcessObservation;
use super::scan::ProcessObservations;

fn cargo(
    pid: u32,
    argv: &[OsString],
    parent: ProcessField<Pid>,
    memory: u64,
) -> ProcessObservation<'_> {
    let mut process = ProcessObservation::cargo(pid, argv);
    process.parent = parent;
    process.memory = memory;
    process
}

fn non_cargo<'scan>(
    pid: u32,
    name: &'static OsStr,
    argv: &'scan [OsString],
    parent: ProcessField<Pid>,
    memory: u64,
) -> ProcessObservation<'scan> {
    let mut process = cargo(pid, argv, parent, memory);
    process.name = ProcessField::Observed(name);
    process
}

fn parent(pid: u32) -> ProcessField<Pid> { ProcessField::Observed(Pid::from_u32(pid)) }

fn assert_row_memory(
    group: &CargoGroup,
    pid: u32,
    expected_memory: Measurement<u64>,
    expected_subtree_memory: Measurement<u64>,
) {
    assert!(
        std::iter::once(&group.lead)
            .chain(&group.rest)
            .any(|process| {
                process.pid == pid
                    && process.memory == expected_memory
                    && process.subtree_memory == expected_subtree_memory
            }),
        "pid {pid} must carry {expected_memory:?} and subtree {expected_subtree_memory:?}"
    );
}

#[test]
fn command_memory_includes_its_non_cargo_children() {
    let command = vec!["cargo".into(), "build".into()];
    let compiler = vec!["rustc".into(), "--crate-name".into(), "app".into()];
    let records = ProcessObservations::new([
        cargo(100, &command, ProcessField::Unavailable, 4_096),
        non_cargo(101, OsStr::new("rustc"), &compiler, parent(100), 8_192),
    ]);

    let groups = CensusSequence::default().sample(&records);

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.memory, Measurement::Reading(12_288));
    assert_eq!(groups[0].lead.subtree_memory, Measurement::Reading(12_288));
}

#[test]
fn group_rows_keep_buckets_while_subtrees_include_nested_invocations() {
    let driver = vec!["cargo".into(), "port".into()];
    let child = vec!["cargo".into(), "build".into()];
    let nested = vec!["cargo".into(), "test".into()];
    let records = ProcessObservations::new([
        cargo(100, &driver, ProcessField::Unavailable, 100),
        cargo(101, &child, parent(100), 200),
        cargo(102, &nested, parent(101), 300),
    ]);

    let groups = CensusSequence::default().sample(&records);

    assert_eq!(groups.len(), 1);
    let group = &groups[0];
    assert_eq!(group.lead.pid, 100);
    assert_eq!(group.lead.memory, Measurement::Reading(600));
    assert_eq!(group.lead.subtree_memory, Measurement::Reading(600));
    assert_row_memory(
        group,
        101,
        Measurement::Reading(200),
        Measurement::Reading(500),
    );
    assert_row_memory(
        group,
        102,
        Measurement::Reading(300),
        Measurement::Reading(300),
    );
}

#[test]
fn zero_own_memory_makes_the_invocation_and_group_total_unavailable() {
    let driver = vec!["cargo".into(), "port".into()];
    let child = vec!["cargo".into(), "build".into()];
    let records = ProcessObservations::new([
        cargo(100, &driver, ProcessField::Unavailable, 1_000),
        cargo(101, &child, parent(100), 0),
    ]);

    let groups = CensusSequence::default().sample(&records);

    let unavailable = Measurement::Unavailable(MeasurementAbsence::ReadFailed);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].lead.memory, unavailable);
    assert_row_memory(&groups[0], 101, unavailable, unavailable);
}

#[test]
fn detached_compiler_descendants_are_credited_to_the_named_owner() {
    let owner_argv = vec!["cargo".into(), "build".into()];
    let compiler = vec!["sccache".into()];
    let worker_argv = vec!["rustc".into(), "--crate-name".into(), "app".into()];
    let records = ProcessObservations::new([
        cargo(100, &owner_argv, ProcessField::Unavailable, 10),
        non_cargo(
            200,
            OsStr::new("sccache"),
            &compiler,
            ProcessField::Unavailable,
            20,
        ),
        non_cargo(201, OsStr::new("rustc"), &worker_argv, parent(200), 30),
    ]);

    let owner = Pid::from_u32(100);
    let detached = HashMap::from([(Pid::from_u32(200), owner)]);

    let credited = CensusSequence::attribute_memory(&records, &detached);
    let uncredited = CensusSequence::attribute_memory(&records, &HashMap::new());

    assert_eq!(credited, HashMap::from([(owner, Measurement::Reading(60))]));
    assert_eq!(
        uncredited,
        HashMap::from([(owner, Measurement::Reading(10))])
    );
}

#[test]
fn memory_reporting_holds_until_due_and_replaces_unavailable_values_immediately() {
    let invocation = InvocationId::for_test(100);
    let cargo = [invocation.clone()];
    let mut accounting = InvocationCpuAccounting::default();

    let first = HashMap::from([(invocation.clone(), Measurement::Reading(10))]);
    assert_eq!(
        accounting.report_memory(&first, &cargo, MemoryReport::Keep)[&invocation],
        Measurement::Reading(10)
    );

    let changed = HashMap::from([(invocation.clone(), Measurement::Reading(20))]);
    assert_eq!(
        accounting.report_memory(&changed, &cargo, MemoryReport::Keep)[&invocation],
        Measurement::Reading(10)
    );
    assert_eq!(
        accounting.report_memory(&changed, &cargo, MemoryReport::Replace)[&invocation],
        Measurement::Reading(20)
    );

    let unavailable = Measurement::Unavailable(MeasurementAbsence::ReadFailed);
    let failed = HashMap::from([(invocation.clone(), unavailable)]);
    assert_eq!(
        accounting.report_memory(&failed, &cargo, MemoryReport::Keep)[&invocation],
        unavailable
    );

    let recovered = HashMap::from([(invocation.clone(), Measurement::Reading(30))]);
    assert_eq!(
        accounting.report_memory(&recovered, &cargo, MemoryReport::Keep)[&invocation],
        Measurement::Reading(30)
    );
}

#[test]
fn memory_reporting_drops_departed_invocations_and_marks_missing_samples_unproven() {
    let departed = InvocationId::for_test(100);
    let unsampled = InvocationId::for_test(101);
    let mut accounting = InvocationCpuAccounting::default();
    let sampled = HashMap::from([(departed.clone(), Measurement::Reading(10))]);
    accounting.report_memory(
        &sampled,
        std::slice::from_ref(&departed),
        MemoryReport::Replace,
    );

    let reported = accounting.report_memory(
        &sampled,
        std::slice::from_ref(&unsampled),
        MemoryReport::Keep,
    );

    assert_eq!(
        reported,
        HashMap::from([(
            unsampled,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        )])
    );
}

#[test]
fn scanned_memory_changes_only_when_the_cpu_reading_falls_due() {
    let command = vec!["cargo".into(), "build".into()];
    let report = CensusCadence::default().report;
    let start = Instant::now();
    let mut sequence = CensusSequence::default();
    let mut scan = |memory: u64, elapsed: Duration| {
        let mut records =
            ProcessObservations::new([cargo(100, &command, ProcessField::Unavailable, memory)]);
        let groups = sequence.sample_counters(&mut records, start + elapsed);
        assert_eq!(groups.len(), 1);
        groups[0].lead.memory
    };

    assert_eq!(scan(100, Duration::ZERO), Measurement::Reading(100));
    assert_eq!(scan(200, report / 2), Measurement::Reading(100));
    assert_eq!(scan(300, report), Measurement::Reading(300));
}

#[test]
fn registration_only_row_has_no_reading_and_withholds_its_group_total() {
    let driver = vec!["cargo".into(), "port".into()];
    let child = vec!["cargo".into(), "build".into()];
    let unproven = Measurement::Unavailable(MeasurementAbsence::Unproven);
    let baseline = CensusSequence::default().sample(&ProcessObservations::new([
        cargo(100, &driver, ProcessField::Unavailable, 1_000),
        cargo(101, &child, parent(100), 500),
    ]));
    assert_eq!(baseline[0].lead.memory, Measurement::Reading(1_500));
    // What `registration_row` builds: verified metadata with no measured process.
    let mut registration = baseline[0].rest[0].clone();
    registration.invocation_id = InvocationId::for_test(201);
    registration.memory = unproven;
    registration.subtree_memory = unproven;
    let mut sequence = CensusSequence::default();
    sequence.registration_rows = vec![registration];

    let mut rowless_child = cargo(101, &child, parent(100), 500);
    rowless_child.argv = ProcessField::Unavailable;
    let records = ProcessObservations::new([
        cargo(100, &driver, ProcessField::Unavailable, 1_000),
        rowless_child,
    ]);

    let groups = sequence.sample(&records);

    assert_eq!(groups.len(), 1);
    let group = &groups[0];
    assert_eq!(group.lead.pid, 100);
    assert_eq!(group.lead.memory, unproven);
    assert_eq!(group.lead.subtree_memory, unproven);
    assert_row_memory(group, 101, unproven, unproven);
}
