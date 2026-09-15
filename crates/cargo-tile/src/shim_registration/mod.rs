//! Exercise shim publications and the production reader with an explicit fixture parent.

#[cfg(test)]
mod app_scenarios;
#[cfg(test)]
mod reader_scenarios;
#[cfg(test)]
mod rows_readout;
#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod shared_capture;
#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod wire;
