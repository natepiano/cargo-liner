//! Native scan inputs and frame publication stamps for the reader CPU oracle.
//!
//! This module is compiled only in tests. The CPU reader calls `start()` to set
//! `READER`; other readers and supplied-counter tests have no recorder or output changes.

use std::fmt::Write as _;
use std::fs::File;
use std::io;
use std::io::Write;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Instant;

use sysinfo::Pid;

use super::invocation_cpu_accounting::InvocationCpuContributions;
use super::invocation_cpu_accounting::Measurement;
use super::process_identity::InvocationId;
use crate::constants::READER_CPU_OBSERVATION_FILE;

/// The accounting scan used by one published set of invocation rows.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum CpuScanPublication {
    /// A fixture or a reader without CPU observation supplied these rows.
    #[default]
    Unobserved,
    /// The recorder has native inputs for this scan.
    Observed(u64),
}

/// One reader owns the file, scan sequence, and accepted scan for its frames.
struct CpuReaderObservation {
    file:                 File,
    origin:               Instant,
    scan:                 u64,
    accepted_publication: CpuScanPublication,
    inputs:               String,
}

static READER: OnceLock<Mutex<CpuReaderObservation>> = OnceLock::new();

/// Install observation before the reader starts its scanner thread.
pub(crate) fn start() -> io::Result<()> {
    let observation = CpuReaderObservation {
        file:                 File::create(READER_CPU_OBSERVATION_FILE)?,
        origin:               Instant::now(),
        scan:                 0,
        accepted_publication: CpuScanPublication::Unobserved,
        inputs:               String::new(),
    };
    assert!(READER.set(Mutex::new(observation)).is_ok());
    Ok(())
}

/// Write all native inputs and accounting results from one scan as one record.
pub(super) fn record(now: Instant, inputs: impl FnOnce() -> String) -> CpuScanPublication {
    READER
        .get()
        .map_or(CpuScanPublication::Unobserved, |reader| {
            let mut reader = reader
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            reader.scan += 1;
            let scan = reader.scan;
            let at = now.saturating_duration_since(reader.origin).as_nanos();
            let inputs = inputs();
            let work = std::mem::take(&mut reader.inputs);
            let result = writeln!(reader.file, "scan\t{scan}\t{at}\n{work}{inputs}end\t{scan}");
            drop(reader);
            assert!(result.is_ok(), "write CPU scan observation: {result:?}");
            CpuScanPublication::Observed(scan)
        })
}

/// Write `InvocationCpuContributions` before accounting consumes its counters.
pub(super) fn work(
    owner: Pid,
    invocation: &InvocationId,
    work: &InvocationCpuContributions,
    evidence: Measurement<f32>,
) {
    if let Some(reader) = READER.get() {
        let mut inputs = String::new();
        let owner = owner.as_u32();
        let result = writeln!(inputs, "owner\t{owner}\t{evidence:?}\t{invocation:?}");
        assert!(result.is_ok());
        for (identity, counter) in &work.tree {
            let result = writeln!(
                inputs,
                "counter\t{owner}\ttree\t{identity:?}\t{}",
                counter.as_secs_f64()
            );
            assert!(result.is_ok());
        }
        for (root, counters) in &work.detached {
            for (identity, counter) in counters {
                let result = writeln!(
                    inputs,
                    "counter\t{owner}\t{root:?}\t{identity:?}\t{}",
                    counter.as_secs_f64()
                );
                assert!(result.is_ok());
            }
        }
        #[cfg(target_os = "linux")]
        for (identity, counter) in &work.nested {
            let result = writeln!(
                inputs,
                "nested\t{owner}\t{identity:?}\t{}",
                counter.as_secs_f64()
            );
            assert!(result.is_ok());
        }
        reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .inputs
            .push_str(&inputs);
    }
}

/// Only the scan taken by the terminal loop may supply the next frame's stamp.
pub(crate) fn accept(publication: CpuScanPublication) {
    if let Some(reader) = READER.get() {
        reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accepted_publication = publication;
    }
}

/// Write the OSC 777 `cargo-tile-cpu` stamp to stdout for the accepted
/// `CpuScanPublication::Observed` scan before the backend writes its frame.
pub(crate) fn publish_frame() {
    if let Some(reader) = READER.get() {
        let reader = reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let publication = reader.accepted_publication;
        let at = reader.origin.elapsed().as_nanos();
        drop(reader);
        if let CpuScanPublication::Observed(scan) = publication {
            let result = write!(
                io::stdout().lock(),
                "\x1b]777;cargo-tile-cpu;{scan};{at}\x07"
            );
            assert!(result.is_ok(), "write CPU frame stamp: {result:?}");
        }
    }
}
