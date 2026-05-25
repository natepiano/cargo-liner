//! Framework-provided observability primitives.
//!
//! [`cpu`] exposes a sysinfo-backed CPU/GPU sampler plus severity
//! buckets keyed to the framework's theme colors. [`perf_log`] installs
//! a tracing subscriber that rotates a single perf log file and
//! exposes the slow-threshold constants and a `u128`→`u64` helper used
//! by callers when emitting tracing fields.

mod constants;
mod cpu;
mod perf_log;

pub use cpu::CpuBreakdown;
pub use cpu::CpuCoreUsage;
pub use cpu::CpuPoller;
pub use cpu::CpuSeverity;
pub use cpu::CpuUsage;
pub use cpu::blank_bar_color;
pub use cpu::filled_cells;
pub use cpu::severity;
pub use perf_log::SLOW_BG_BATCH_MS;
pub use perf_log::SLOW_FRAME_MS;
pub use perf_log::SLOW_INPUT_EVENT_MS;
pub use perf_log::init;
pub use perf_log::ms;
