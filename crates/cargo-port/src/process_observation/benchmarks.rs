use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use sysinfo::Pid;
use sysinfo::ProcessRefreshKind;

use super::FullProcessDiscoveryOutcome;
use super::ProcessObserver;
use super::ProcessRefreshExecutionBackendSelection;
use super::ProcessRefreshHostSource;
use super::identity::ObservedProcessIdentity;
use super::identity::PlatformProcessObservation;
use super::identity::ProcessCreationOrderEvidence;
use super::identity::ProcessIdentity;
use super::snapshot::ProcessFieldObservation;
use super::snapshot::ProcessFieldSample;
use super::snapshot::ProcessFieldSourceObservation;
use super::snapshot::ProcessRefreshInput;
use super::snapshot::ReportedParent;

const EVENT_LOOP_PROCESS_REFRESH_BUDGET: Duration = Duration::from_millis(15);
const FIXTURE_PROCESS_START_TIME_OFFSET: u64 = 10_000;
const LARGE_FIXTURE_PROCESS_COUNT: usize = 5_000;
const PROCESS_COUNTS: [usize; 2] = [SMALL_FIXTURE_PROCESS_COUNT, LARGE_FIXTURE_PROCESS_COUNT];
const RECORDED_FIXTURE_BACKED_OBSERVER_TIMINGS: [FixtureBackedObserverTimingSamples; 2] = [
    FixtureBackedObserverTimingSamples {
        process_count:      SMALL_FIXTURE_PROCESS_COUNT,
        full_refreshes:     [
            Duration::from_nanos(11_680_042),
            Duration::from_nanos(11_561_833),
            Duration::from_nanos(11_895_250),
            Duration::from_nanos(12_608_833),
            Duration::from_nanos(12_573_542),
        ],
        targeted_refreshes: [
            Duration::from_nanos(14_238_708),
            Duration::from_nanos(14_384_334),
            Duration::from_nanos(13_982_750),
            Duration::from_nanos(14_089_791),
            Duration::from_nanos(16_098_416),
        ],
    },
    FixtureBackedObserverTimingSamples {
        process_count:      LARGE_FIXTURE_PROCESS_COUNT,
        full_refreshes:     [
            Duration::from_nanos(66_850_584),
            Duration::from_nanos(65_165_875),
            Duration::from_nanos(62_502_625),
            Duration::from_nanos(68_914_917),
            Duration::from_nanos(65_353_250),
        ],
        targeted_refreshes: [
            Duration::from_nanos(82_820_375),
            Duration::from_nanos(105_235_375),
            Duration::from_nanos(104_515_917),
            Duration::from_nanos(98_919_208),
            Duration::from_nanos(101_998_167),
        ],
    },
];
const RECORDED_TIMING_DATE: &str = "2026-08-02";
const REPEATED_SAMPLE_COUNT: usize = 5;
const SMALL_FIXTURE_PROCESS_COUNT: usize = 1_000;

struct DeterministicFixtureProcessRefreshHost {
    identities: BTreeSet<ProcessIdentity>,
    pids:       Vec<Pid>,
}

impl DeterministicFixtureProcessRefreshHost {
    fn new(process_count: usize) -> Self {
        let identities: BTreeSet<_> = (1..=process_count)
            .map(|index| {
                let pid = u32::try_from(index).expect("fixture process count fits in u32");
                ProcessIdentity::for_test(pid, u64::from(pid) + FIXTURE_PROCESS_START_TIME_OFFSET)
            })
            .collect();
        let pids = identities
            .iter()
            .map(|process_identity| Pid::from_u32(process_identity.pid()))
            .collect();
        Self { identities, pids }
    }

    fn measure_full_refresh(&self, process_observer: &mut ProcessObserver) -> Duration {
        let started = Instant::now();
        let process_observation_snapshot = process_observer
            .refresh_with_host_source(&ProcessRefreshInput::FullSystemSnapshot, self);
        black_box(process_observation_snapshot);
        started.elapsed()
    }

    fn measure_targeted_refresh(&self, process_observer: &mut ProcessObserver) -> Duration {
        let started = Instant::now();
        let process_refresh_input =
            ProcessRefreshInput::TargetedIdentities(self.identities.clone());
        let process_observation_snapshot =
            process_observer.refresh_with_host_source(&process_refresh_input, self);
        black_box(process_observation_snapshot);
        started.elapsed()
    }
}

impl ProcessRefreshHostSource for DeterministicFixtureProcessRefreshHost {
    fn full_process_discovery(&self) -> FullProcessDiscoveryOutcome {
        FullProcessDiscoveryOutcome::Updated(self.pids.clone())
    }

    fn process_identity_observation(&self, pid: u32) -> PlatformProcessObservation {
        let process_identity =
            ProcessIdentity::for_test(pid, u64::from(pid) + FIXTURE_PROCESS_START_TIME_OFFSET);
        PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(process_identity.clone()),
            ProcessCreationOrderEvidence::for_test_identity(&process_identity),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        )
    }

    fn repeated_process_field_observations(
        &self,
        pids: &[Pid],
        _: ProcessRefreshKind,
    ) -> BTreeMap<Pid, ProcessFieldSourceObservation> {
        pids.iter()
            .map(|pid| (*pid, repeated_field_samples(pid.as_u32())))
            .collect()
    }
}

#[derive(Clone, Debug)]
struct FixtureBackedObserverTimingSamples {
    process_count:      usize,
    full_refreshes:     [Duration; REPEATED_SAMPLE_COUNT],
    targeted_refreshes: [Duration; REPEATED_SAMPLE_COUNT],
}

impl FixtureBackedObserverTimingSamples {
    fn selected_backend(&self) -> ProcessRefreshExecutionBackendSelection {
        select_execution_backend(
            self.full_refreshes[0],
            self.full_refreshes[1..]
                .iter()
                .chain(&self.targeted_refreshes)
                .copied(),
        )
    }
}

fn repeated_field_samples(pid: u32) -> ProcessFieldSourceObservation {
    let process_field_sample = ProcessFieldSample::for_test(
        PathBuf::from(format!("/fixture/target/debug/process-{pid}")),
        vec![format!("process-{pid}").into()],
        PathBuf::from("/fixture"),
    );
    ProcessFieldSourceObservation::repeated_fresh_system_samples(
        process_field_sample.clone(),
        process_field_sample,
    )
}

fn measure_fixture(process_count: usize) -> FixtureBackedObserverTimingSamples {
    let process_refresh_host = DeterministicFixtureProcessRefreshHost::new(process_count);
    let mut full_process_observer = ProcessObserver::default();
    let mut targeted_process_observer = ProcessObserver::default();

    black_box(process_refresh_host.measure_full_refresh(&mut full_process_observer));
    black_box(process_refresh_host.measure_targeted_refresh(&mut targeted_process_observer));

    let full_refreshes = std::array::from_fn(|_| {
        process_refresh_host.measure_full_refresh(&mut full_process_observer)
    });
    let targeted_refreshes = std::array::from_fn(|_| {
        process_refresh_host.measure_targeted_refresh(&mut targeted_process_observer)
    });
    FixtureBackedObserverTimingSamples {
        process_count,
        full_refreshes,
        targeted_refreshes,
    }
}

fn select_execution_backend(
    first_sample: Duration,
    remaining_samples: impl IntoIterator<Item = Duration>,
) -> ProcessRefreshExecutionBackendSelection {
    if first_sample <= EVENT_LOOP_PROCESS_REFRESH_BUDGET
        && remaining_samples
            .into_iter()
            .all(|elapsed| elapsed <= EVENT_LOOP_PROCESS_REFRESH_BUDGET)
    {
        ProcessRefreshExecutionBackendSelection::Synchronous
    } else {
        ProcessRefreshExecutionBackendSelection::DedicatedWorker
    }
}

#[test]
fn report_fixture_backed_process_observer_refresh_timings() {
    let current_samples: Vec<_> = PROCESS_COUNTS.into_iter().map(measure_fixture).collect();
    eprintln!(
        "ProcessObserver timing with deterministic fixture-backed full discovery, identity, and \
         repeated field host calls; OS host-call latency is excluded. budget={EVENT_LOOP_PROCESS_REFRESH_BUDGET:?}, \
         recorded_on={RECORDED_TIMING_DATE}, recorded_samples={RECORDED_FIXTURE_BACKED_OBSERVER_TIMINGS:#?}, \
         current_samples={current_samples:#?}"
    );
}

#[test]
fn recorded_five_thousand_process_samples_select_the_dedicated_worker() {
    let recorded_samples = RECORDED_FIXTURE_BACKED_OBSERVER_TIMINGS
        .iter()
        .find(|samples| samples.process_count == LARGE_FIXTURE_PROCESS_COUNT)
        .expect("five-thousand-process recorded timing fixture is configured");

    assert_eq!(
        recorded_samples.selected_backend(),
        ProcessRefreshExecutionBackendSelection::DedicatedWorker
    );
}

#[test]
fn samples_at_or_below_fifteen_milliseconds_select_synchronous_execution() {
    let samples = [Duration::ZERO, EVENT_LOOP_PROCESS_REFRESH_BUDGET];

    assert_eq!(
        select_execution_backend(samples[0], samples[1..].iter().copied()),
        ProcessRefreshExecutionBackendSelection::Synchronous
    );
}

#[test]
fn any_sample_above_fifteen_milliseconds_selects_the_dedicated_worker() {
    let samples = [
        EVENT_LOOP_PROCESS_REFRESH_BUDGET,
        EVENT_LOOP_PROCESS_REFRESH_BUDGET + Duration::from_nanos(1),
    ];

    assert_eq!(
        select_execution_backend(samples[0], samples[1..].iter().copied()),
        ProcessRefreshExecutionBackendSelection::DedicatedWorker
    );
}
