//! Compare `pre_filter` sites with the pinned Python ledger.

#[path = "sites/parity.rs"]
mod parity;
mod support;

use parity::run_sites_parity;
use support::TestResult;

const ORACLE_SOURCE: &str = include_str!("sites/prefilter_oracle.py");

#[test]
fn prefilter_sites_match_the_python_ledger() -> TestResult<()> {
    compare_with_oracle("listed_sites_cases")?;
    compare_with_oracle("malformed_manifest_cases")
}

#[test]
fn invalid_prefilter_pattern_matches_the_python_refusal() -> TestResult<()> {
    compare_with_oracle("invalid_pattern_cases")
}

#[test]
fn failing_generator_refuses_the_prefilter_request() -> TestResult<()> {
    compare_with_oracle("invalid_generator_cases")
}

fn compare_with_oracle(entrypoint: &str) -> TestResult<()> {
    let oracle = format!("{ORACLE_SOURCE}\nrun({entrypoint})\n");
    run_sites_parity(oracle.as_bytes(), "cargo-style-prefilter-sites-")
}
