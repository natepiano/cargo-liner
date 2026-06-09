//! Domain strings for the diagnostics module's perf log.

// perf log
pub(super) const DEFAULT_PERF_LOG_FILTER: &str = "info";
pub(super) const PERF_LOG_ENV: &str = "CARGO_PORT_LOG";
/// Tracing target used for cargo-port performance diagnostics.
pub const PERF_LOG_TARGET: &str = "cargo_port::perf";
